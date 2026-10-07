#!/usr/bin/env python3
"""LoRA-train a decision head on verdict-slot SFT rows (E1, #88).

Consumes `decision_sft_prep.py` rows (segmented macjev-render-v1
renders) and teaches exactly the readout the serving path scores:
loss lands only on the y=1 verdict segments (" yes"/" no" at the
option slots); everything else — the state, the question, the option
lines, the `` ->`` markers — is masked context. EOS is appended and
supervised, so the model learns to stop after the last slot.

v2 (r3 protocol, TRAINING.md §9.7 Phase A) — what changed from the
r2 trainer and why:

- **Windowed loss** (A1): the r2 log printed the cumulative epoch
  mean at every step, which made the curve look plateaued while the
  recent dynamics were invisible. The log now carries the mean over
  the last ~100 micro-batches; the token-weighted cumulative mean
  survives only in the manifest.
- **Stratified val split** (A2): ~1 % of rows (capped) held out by
  question type (the corpus families), scored every --val-every
  steps as exact per-row token-mean losses — overall and per family,
  plus token accuracy and mean entropy over supervised positions
  (TRL/Axolotl log schema). Val selects; the pre-registered suite
  gate judges.
- **Checkpoint selection** (A3): the best-val adapter is saved to
  ``adapter-best/`` alongside the final ``adapter/``; the manifest
  records the best step and its val loss.
- **Loud-skip truncated rows** (A4): rows exceeding --max-len are
  dropped and counted (their verdict tokens would be truncated away
  — r2 silently trained nothing on 86 of them, and an all-masked
  micro-batch would have produced a NaN loss).
- **Token-weighted metrics** (A4): logged/val losses weight each row
  by its supervised-token count, so 2-slot and 6-slot rows compare
  fairly. Gradient handling is unchanged (mean CE / accum) — only
  the metrics are weighted.
- **Shuffle before limit** (A5): --limit-rows samples a seeded
  shuffle, not the file head (the corpus is family-grouped, so the
  r2 pilot path was family-skewed).
- **Feature cache** (A5): tokenized (ids, labels, families) cached
  to ``<data>/.cache/`` keyed by data sha + max_len; pilots skip
  re-tokenization. Cache invalidated wholesale when row counts do
  not match the current split.
- **Optimizer/library versions + peak memory** (A3): weight_decay/
  betas/eps, torch/transformers/peft versions, and
  ``torch.cuda.max_memory_allocated()`` land in the manifest — the
  r2 manifest could not reproduce its own optimizer.
- **Family weighting** (B5 knob): --family-weight ``noul=0.7``
  scales each micro-batch loss by the mean family weight of its
  rows; recorded in the manifest.
- **sdpa attention default** (A7, research-integrated): PyTorch SDPA
  dispatches to the memory-efficient CUTLASS backend below Ampere
  (V100) and flash on Ampere+; eager is available via --attn eager
  and used automatically if the backend refuses. FlashAttention-2
  itself is Ampere+ only — not a V100 option (research finding).
- **Gradient checkpointing opt-out** (A7, research-integrated):
  checkpointing costs ~20 % throughput and is an inherited default;
  --no-grad-checkpoint turns it off where peak memory allows (the
  manifest's peak-memory field is the evidence for the decision).
- **pad to multiple of 8** (research-integrated): fp16 tensor-core
  efficiency wants batch-seq dimensions that are multiples of 8.
- **Bounded profiler** (A7): --profile-steps N wraps the first N
  optimizer steps in ``torch.profiler`` and writes a chrome trace +
  summary table under ``out/profile/``.

torch/transformers/peft import lazily so the pure helpers (segment
encoding, label masking, batching, stratified split) unit-test on
machines without torch. Every knob is a CLI flag with the
pre-registered default; a run records its full knob set + data sha
in the output manifest.

Usage:
    python3 runner/e1_train.py \
        --base Qwen/Qwen3.5-2B --data ~/oc-model-eval/corpora/e1-sft-v1 \
        --out ~/oc-model-eval/runs/e1-a-qwen2b \
        [--epochs 1] [--lr 1e-4] [--lora-r 16] [--batch 4] [--accum 4] \
        [--max-len 2048] [--dtype fp16|bf16] [--limit-rows N] [--seed 42]
"""

from __future__ import annotations

import argparse
import hashlib
import json
import platform
import random
import sys
import time
from collections import Counter, deque
from pathlib import Path


def sha256_of(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def say(msg: str) -> None:
    """Console line + flush (runner convention: lines are also tailed
    from the .out file by the night chain)."""
    sys.stdout.write(msg + "\n")
    sys.stdout.flush()


def encode_row(segments: list[dict], encode_fn, eos_id: int,
               max_len: int) -> tuple[list[int], list[int]] | None:
    """One row -> (input_ids, labels), or None if the row exceeds
    max_len (loud-skipped by the caller: truncation would cut the
    verdict tokens and the row would train on nothing). Each segment
    is encoded separately (boundaries at ' ->'/verdict tokens are
    stable token edges); context ids are masked to -100, verdict ids
    supervise. EOS is appended and supervised."""
    ids: list[int] = []
    labels: list[int] = []
    for seg in segments:
        seg_ids = encode_fn(seg["t"])
        ids.extend(seg_ids)
        labels.extend(seg_ids if seg["y"] else [-100] * len(seg_ids))
    ids.append(eos_id)
    labels.append(eos_id)
    if len(ids) > max_len:
        return None
    return ids, labels


class PadCollate:
    """Right-pad input_ids/labels to the batch max (rounded up to a
    multiple of 8 — fp16 tensor-core tile width) with pad/eos-id and
    -100 respectively; attention mask marks real tokens."""

    def __init__(self, pad_id: int):
        self.pad_id = pad_id

    def __call__(self, batch: list[tuple[list[int], list[int]]]):
        raw_max = max(len(ids) for ids, _ in batch)
        max_len = ((raw_max + 7) // 8) * 8
        input_ids, labels, attn = [], [], []
        for ids, lab in batch:
            pad = max_len - len(ids)
            input_ids.append(ids + [self.pad_id] * pad)
            labels.append(lab + [-100] * pad)
            attn.append([1] * len(ids) + [0] * pad)
        return input_ids, labels, attn


def stratified_split(rows: list[dict], val_frac: float, val_cap: int,
                     seed: int) -> tuple[list[int], list[int]]:
    """Hold out a val slice stratified by question type (the corpus
    families); returns (train_indices, val_indices) into `rows`.
    Deterministic under `seed`. The suite is never a candidate — the
    prep tool already excluded it."""
    by_family: dict[str, list[int]] = {}
    for i, r in enumerate(rows):
        by_family.setdefault(str(r.get("qtype")), []).append(i)
    rng = random.Random(seed)
    val: list[int] = []
    for fam in sorted(by_family):
        idx = by_family[fam][:]
        rng.shuffle(idx)
        take = max(1, int(round(len(idx) * val_frac)))
        val.extend(idx[:take])
    if len(val) > val_cap:
        rng.shuffle(val)
        val = val[:val_cap]
    val_set = set(val)
    train = [i for i in range(len(rows)) if i not in val_set]
    return train, val


def parse_family_weights(spec: str) -> dict[str, float]:
    """Parse ``noul=0.7,score=1.0`` -> {"noul": 0.7, "score": 1.0}."""
    out: dict[str, float] = {}
    if not spec.strip():
        return out
    for part in spec.split(","):
        k, _, v = part.partition("=")
        out[k.strip()] = float(v)
    return out


def bucketed_order(lengths: list[int], seed: int, epoch: int,
                   bucket: int) -> list[int]:
    """Length-homogeneous batching (HF LengthGroupedSampler shape):
    sort by length inside shuffled mega-batches so batches pad less;
    bucket membership and within-bucket order reshuffle every epoch.
    Length correlates with question family, so this is OFF by default
    (--length-bucket enables it as the A7 experiment) — grouping with
    high length variance is a documented eval-loss degradation risk."""
    idx = list(range(len(lengths)))
    random.Random(seed * 1000 + epoch).shuffle(idx)
    order: list[int] = []
    for start in range(0, len(idx), bucket):
        chunk = idx[start:start + bucket]
        chunk.sort(key=lambda i: lengths[i])
        order.extend(chunk)
    random.Random(seed * 1000 + epoch + 1).shuffle(order)
    return order


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--base", required=True)
    ap.add_argument("--data", type=Path, required=True,
                    help="decision_sft_prep output dir")
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--epochs", type=float, default=1.0)
    ap.add_argument("--lr", type=float, default=1e-4)
    ap.add_argument("--lora-r", type=int, default=16)
    ap.add_argument("--lora-alpha", type=int, default=32)
    ap.add_argument("--lora-dropout", type=float, default=0.05)
    ap.add_argument("--batch", type=int, default=4)
    ap.add_argument("--accum", type=int, default=4)
    ap.add_argument("--max-len", type=int, default=2048)
    ap.add_argument("--dtype", choices=("fp16", "bf16"), default="fp16",
                    help="fp16 = Volta (T5500 V100); bf16 = Ampere+")
    ap.add_argument("--attn", choices=("sdpa", "eager"), default="sdpa",
                    help="sdpa = flash backend on Ampere+, "
                         "memory-efficient CUTLASS below (V100); the "
                         "trainer falls back to eager with a logged "
                         "warning if the backend refuses")
    ap.add_argument("--no-grad-checkpoint", action="store_true",
                    help="disable gradient checkpointing (faster, "
                         "more activation memory; decide from the "
                         "manifest's peak-memory field)")
    ap.add_argument("--warmup", type=float, default=0.03)
    ap.add_argument("--limit-rows", type=int, default=0,
                    help="pilot/smoke: seeded shuffle THEN take N")
    ap.add_argument("--val-frac", type=float, default=0.01,
                    help="stratified val slice, per question type")
    ap.add_argument("--val-cap", type=int, default=2000,
                    help="max val rows overall")
    ap.add_argument("--val-every", type=int, default=500,
                    help="optimizer steps between val evals")
    ap.add_argument("--family-weight", default="",
                    help="per-family loss weights, noul=0.7,score=1.0")
    ap.add_argument("--length-bucket", action="store_true",
                    help="length-homogeneous batching experiment")
    ap.add_argument("--profile-steps", type=int, default=0,
                    help="wrap the first N optimizer steps in "
                         "torch.profiler")
    ap.add_argument("--seed", type=int, default=42)
    args = ap.parse_args()

    try:
        import numpy as np
        import torch
        import torch.nn.functional as F
        import peft
        import transformers
        from peft import LoraConfig, get_peft_model
        from transformers import (AutoModelForCausalLM, AutoTokenizer,
                                  get_cosine_schedule_with_warmup)
    except ImportError as e:
        sys.stderr.write(f"torch stack unavailable on this host: {e}\n")
        return 1

    rows_path = args.data / "e1-sft-v1.rows.jsonl"
    rows = [json.loads(line) for line in rows_path.open(encoding="utf-8")
            if line.strip()]
    data_sha = sha256_of(rows_path)
    random.Random(args.seed).shuffle(rows)
    if args.limit_rows:
        rows = rows[:args.limit_rows]
    train_idx, val_idx = stratified_split(
        rows, args.val_frac, args.val_cap, args.seed)
    fam_weights = parse_family_weights(args.family_weight)
    fams_train = dict(Counter(str(rows[i].get("qtype"))
                              for i in train_idx))
    say(f"rows={len(rows)} train={len(train_idx)} val={len(val_idx)} "
        f"families_train={fams_train}")

    tok = AutoTokenizer.from_pretrained(args.base)
    if tok.pad_token_id is None:
        tok.pad_token = tok.eos_token
    dtype = torch.float16 if args.dtype == "fp16" else torch.bfloat16
    try:
        model = AutoModelForCausalLM.from_pretrained(
            args.base, dtype=dtype, attn_implementation=args.attn)
    except (ValueError, RuntimeError) as e:
        say(f"attn={args.attn} refused by backend ({e}); "
            f"falling back to eager")
        args.attn = "eager"
        model = AutoModelForCausalLM.from_pretrained(
            args.base, dtype=dtype, attn_implementation="eager")
    model.config.use_cache = False
    if not args.no_grad_checkpoint:
        model.gradient_checkpointing_enable()
        model.enable_input_require_grads()
    lconf = LoraConfig(
        r=args.lora_r, lora_alpha=args.lora_alpha,
        lora_dropout=args.lora_dropout,
        target_modules=["q_proj", "k_proj", "v_proj", "o_proj"],
        task_type="CAUSAL_LM")
    model = get_peft_model(model, lconf)
    if torch.cuda.is_available():
        model = model.to("cuda")  # from_pretrained leaves weights on CPU
    model.print_trainable_parameters()

    eos_id = tok.eos_token_id
    collate = PadCollate(tok.pad_token_id)

    # Supervised-position CE (train and val both): run the transformer
    # trunk, gather only supervised positions, project just those
    # through lm_head. Identical loss to passing labels (CE over
    # non-ignored tokens, mean-reduced) but the logits tensor shrinks
    # from B*S*V (~4.9 GiB at batch 4 x 2048 on this vocab) to
    # n_supervised*V (megabytes). Without this the first backward OOMs
    # a 16 GB card at max_len 2048 no matter the dtype/backend.
    base_model = model.get_base_model()
    trunk = getattr(base_model, "model", None)
    head = getattr(base_model, "lm_head", None)
    if trunk is None or head is None:
        say("cannot resolve trunk/lm_head for supervised-position CE")
        return 1

    def sup_logits(t_ids, t_att, t_lab):
        """(selected logits fp32, shifted targets, positions) for one
        batch. The trunk's last_hidden_state is post-final-norm, so
        lm_head(hidden) equals the model's own logits exactly."""
        hidden = trunk(input_ids=t_ids,
                       attention_mask=t_att).last_hidden_state
        shift = t_lab[:, 1:]
        mask = shift != -100
        pos = mask.nonzero(as_tuple=True)
        sel = head(hidden[:, :-1, :][pos[0], pos[1]]).float()
        return sel, shift, pos

    # Tokenize once, cache by (data sha, max_len, split knobs): pilots
    # re-running on the same corpus+config skip the encode pass (A5).
    # The split knobs MUST be in the key — the internal-consistency
    # count check cannot detect a cache written under a different
    # --limit-rows/--seed/--val-frac, and it would silently serve a
    # stale split (seen in smoke v2d: fresh split 395/5, cache 395/4).
    # Families travel with the feats so loud-skips cannot desync
    # row<->family.
    cache_dir = args.data / ".cache"
    split_tag = (f"l{args.limit_rows or 0}-s{args.seed}"
                 f"-vf{args.val_frac:g}-vc{args.val_cap}")
    cache_path = cache_dir / (f"e1-train-{data_sha[:12]}-"
                              f"ml{args.max_len}-{split_tag}.npz")

    def encode_all(split_rows: list[dict], tag: str):
        feats: list[tuple[list[int], list[int]]] = []
        fams: list[str] = []
        skipped = 0
        for row in split_rows:
            enc = encode_row(row["segments"], tok.encode, eos_id,
                             args.max_len)
            if enc is None:
                skipped += 1
                continue
            feats.append(enc)
            fams.append(str(row.get("qtype")))
        say(f"{tag}: feats={len(feats)} loud_skipped_over_maxlen="
            f"{skipped}")
        return feats, fams, skipped

    if cache_path.exists():
        z = np.load(cache_path, allow_pickle=False)
        counts = json.loads(bytes(z["counts"]).decode())
        offs = z["offsets"]
        ids_flat = z["ids"]
        lab_flat = z["labels"]
        fams_all = [str(f) for f in z["fams"]]
        n_feats = len(offs) - 1
        if n_feats == counts["train_n"] + counts["val_n"] \
                and n_feats > 0:
            feats = [(ids_flat[offs[i]:offs[i + 1]].tolist(),
                      lab_flat[offs[i]:offs[i + 1]].tolist())
                     for i in range(n_feats)]
            train_feats = feats[:counts["train_n"]]
            train_fams = fams_all[:counts["train_n"]]
            val_feats = feats[counts["train_n"]:]
            val_fams = fams_all[counts["train_n"]:]
            skipped_train = counts["skipped_train"]
            skipped_val = counts.get("skipped_val", 0)
            say(f"cache hit {cache_path.name} train={len(train_feats)} "
                f"val={len(val_feats)}")
        else:
            say(f"cache stale {cache_path.name} (row-count mismatch) "
                f"— re-encoding")
            cache_path.unlink()
            train_feats, train_fams, skipped_train = encode_all(
                [rows[i] for i in train_idx], "train")
            val_feats, val_fams, skipped_val = encode_all(
                [rows[i] for i in val_idx], "val")
    else:
        train_feats, train_fams, skipped_train = encode_all(
            [rows[i] for i in train_idx], "train")
        val_feats, val_fams, skipped_val = encode_all(
            [rows[i] for i in val_idx], "val")

    if not cache_path.exists():
        cache_dir.mkdir(parents=True, exist_ok=True)
        offs = [0]
        for ids, _ in train_feats + val_feats:
            offs.append(offs[-1] + len(ids))
        counts = {"train_n": len(train_feats), "val_n": len(val_feats),
                  "skipped_train": skipped_train,
                  "skipped_val": skipped_val}
        np.savez(cache_path,
                 ids=np.array([i for ids, _ in train_feats + val_feats
                               for i in ids], dtype=np.int32),
                 labels=np.array([i for _, ls in train_feats + val_feats
                                  for i in ls], dtype=np.int32),
                 offsets=np.array(offs, dtype=np.int64),
                 fams=np.array(train_fams + val_fams),
                 counts=np.frombuffer(json.dumps(counts).encode(),
                                      dtype=np.uint8))
        say(f"cache written {cache_path.name}")

    lengths = [len(ids) for ids, _ in train_feats]
    val_order = sorted(range(len(val_feats)),
                       key=lambda i: len(val_feats[i][0]))

    def make_order(epoch: int) -> list[int]:
        if args.length_bucket:
            return bucketed_order(lengths, args.seed, epoch,
                                  bucket=max(args.batch * args.accum * 32,
                                             256))
        idx = list(range(len(train_feats)))
        random.Random(args.seed + 7 + epoch).shuffle(idx)
        return idx

    steps_per_epoch = max(1, len(train_feats) // (args.batch * args.accum))
    total_steps = max(1, int(steps_per_epoch * args.epochs))
    opt = torch.optim.AdamW(
        [p for p in model.parameters() if p.requires_grad], lr=args.lr)
    sched = get_cosine_schedule_with_warmup(
        opt, int(total_steps * args.warmup), total_steps)
    # fp16 (the Volta/T5500 plan) needs loss scaling — unscaled fp16
    # gradients underflow to zero and the run trains nothing, silently.
    scaler = torch.amp.GradScaler("cuda", enabled=(args.dtype == "fp16"))

    args.out.mkdir(parents=True, exist_ok=True)
    log_path = args.out / "train-log.txt"

    def log(msg: str) -> None:
        with log_path.open("a", encoding="utf-8") as fh:
            fh.write(msg + "\n")
        say(msg)

    log(f"knobs: {json.dumps({**vars(args), 'data_sha': data_sha[:12]},
                              default=str)}")

    def run_val(step: int) -> float:
        """Exact per-row token-mean val losses in ONE forward per
        batch: cross-entropy gathered only at supervised positions
        (a full-vocab reshape at 2048 tokens would need tens of GB),
        attributed per row and per family. Also reports token
        accuracy and mean entropy over supervised positions. Returns
        the token-weighted overall loss; logs the per-family table."""
        model.eval()
        row_losses: list[tuple[int, int]] = []
        n_correct = 0
        n_sup = 0
        ent_sum = 0.0
        with torch.no_grad():
            for start in range(0, len(val_order), 8):
                batch = [val_feats[i] for i in
                         val_order[start:start + 8]]
                input_ids, labels, attn = collate(batch)
                t_ids = torch.tensor(input_ids, device=model.device)
                t_lab = torch.tensor(labels, device=model.device)
                t_att = torch.tensor(attn, device=model.device)
                sel, shift, pos = sup_logits(t_ids, t_att, t_lab)
                if pos[0].numel() == 0:
                    continue
                ce = F.cross_entropy(sel, shift[pos], reduction="none")
                n_correct += int((sel.argmax(-1)
                                  == shift[pos]).sum())
                n_sup += int(sel.shape[0])
                probs = torch.softmax(sel, dim=-1)
                ent_sum += float(
                    -(probs * probs.clamp_min(1e-12).log())
                    .sum(-1).sum())
                bs = t_lab.shape[0]
                row_sum = torch.zeros(bs, device=ce.device) \
                    .index_add_(0, pos[0], ce)
                row_cnt = torch.zeros(bs, device=ce.device) \
                    .index_add_(0, pos[0], torch.ones_like(ce))
                for j in range(bs):
                    if int(row_cnt[j]) > 0:
                        row_losses.append(
                            (float(row_sum[j]), int(row_cnt[j])))
        model.train()
        total = sum(s for s, _ in row_losses)
        toks = sum(c for _, c in row_losses)
        overall = total / max(1, toks)
        fam_sum: Counter = Counter()
        fam_tok: Counter = Counter()
        for (s, c), i in zip(row_losses, val_order):
            fam_sum[val_fams[i]] += s
            fam_tok[val_fams[i]] += c
        per_family = {f: round(fam_sum[f] / max(1, fam_tok[f]), 4)
                      for f in sorted(fam_sum)}
        log(f"val step={step} loss={overall:.4f} "
            f"tok_acc={n_correct / max(1, n_sup):.4f} "
            f"entropy={ent_sum / max(1, n_sup):.4f} "
            f"per_family={json.dumps(per_family)} "
            f"rows={len(row_losses)}")
        return overall

    best = {"step": -1, "val_loss": float("inf")}

    def maybe_checkpoint(step: int) -> None:
        val_loss = run_val(step)
        if val_loss < best["val_loss"]:
            best["step"] = step
            best["val_loss"] = val_loss
            model.save_pretrained(str(args.out / "adapter-best"))
            log(f"best -> adapter-best/ (step={step} "
                f"loss={val_loss:.4f})")

    window: deque[float] = deque(maxlen=100)
    cum_loss = 0.0
    cum_tokens = 0
    micro = 0
    step = 0
    last_gn = 0.0
    t0 = time.time()
    model.train()
    profiler = None
    profile_left = 0
    for epoch in range(int(args.epochs + 0.999)):
        if step >= total_steps:
            break
        order = make_order(epoch)
        for start in range(0, len(order), args.batch):
            sel_idx = order[start:start + args.batch]
            batch = [train_feats[i] for i in sel_idx]
            input_ids, labels, attn = collate(batch)
            t_ids = torch.tensor(input_ids, device=model.device)
            t_lab = torch.tensor(labels, device=model.device)
            t_att = torch.tensor(attn, device=model.device)
            sel, shift, pos = sup_logits(t_ids, t_att, t_lab)
            if pos[0].numel() == 0:
                continue
            raw = F.cross_entropy(sel, shift[pos], reduction="mean")
            fw = 1.0
            if fam_weights:
                fams = [train_fams[i] for i in sel_idx]
                fw = sum(fam_weights.get(f, 1.0) for f in fams) \
                    / max(1, len(fams))
            loss = raw * fw
            scaler.scale(loss / args.accum).backward()
            n_tok = sum(1 for _, ls in batch for l in ls if l != -100)
            loss_f = float(raw.detach())
            window.append(loss_f)
            cum_loss += loss_f * n_tok
            cum_tokens += n_tok
            micro += 1
            if micro % args.accum == 0:
                scaler.unscale_(opt)
                last_gn = float(torch.nn.utils.clip_grad_norm_(
                    [p for p in model.parameters() if p.requires_grad],
                    1.0))
                scaler.step(opt)
                scaler.update()
                sched.step()
                opt.zero_grad(set_to_none=True)
                step += 1
                if step % 20 == 0 or step == total_steps:
                    log(f"step={step}/{total_steps} "
                        f"win={sum(window) / max(1, len(window)):.4f} "
                        f"cum={cum_loss / max(1, cum_tokens):.4f} "
                        f"lr={sched.get_last_lr()[0]:.2e} "
                        f"gn={last_gn:.2f} "
                        f"tok/s={cum_tokens / (time.time() - t0):.0f}")
                if args.profile_steps > 0 and step == 1:
                    from torch.profiler import (ProfilerActivity,
                                                profile)
                    profiler = profile(
                        activities=[ProfilerActivity.CPU,
                                    ProfilerActivity.CUDA],
                        record_shapes=True)
                    profiler.__enter__()
                    profile_left = args.profile_steps
                if profiler is not None and profile_left > 0:
                    profile_left -= 1
                    if profile_left == 0:
                        profiler.__exit__(None, None, None)
                        prof_dir = args.out / "profile"
                        prof_dir.mkdir(exist_ok=True)
                        profiler.export_chrome_trace(
                            str(prof_dir / "trace.json"))
                        (prof_dir / "summary.txt").write_text(
                            profiler.key_averages().table(
                                sort_by="cuda_time_total",
                                row_limit=25))
                        log(f"profile -> {prof_dir}/ "
                            f"({args.profile_steps} steps)")
                        profiler = None
                if args.val_every > 0 and step % args.val_every == 0:
                    maybe_checkpoint(step)
                if step >= total_steps:
                    break
    if args.val_every > 0 and step % args.val_every != 0:
        maybe_checkpoint(step)

    model.save_pretrained(str(args.out / "adapter"))
    tok.save_pretrained(str(args.out / "adapter"))
    peak_mem = (int(torch.cuda.max_memory_allocated())
                if torch.cuda.is_available() else 0)
    manifest = {
        "manifest_version": "opencodifier.e1-train/2",
        "base": args.base,
        "data": {"path": str(rows_path), "sha256": data_sha,
                 "rows": len(rows), "train": len(train_feats),
                 "val": len(val_feats),
                 "loud_skipped_over_maxlen_train": skipped_train,
                 "loud_skipped_over_maxlen_val": skipped_val},
        "knobs": {"epochs": args.epochs, "lr": args.lr,
                  "lora_r": args.lora_r, "lora_alpha": args.lora_alpha,
                  "lora_dropout": args.lora_dropout,
                  "target_modules": ["q_proj", "k_proj", "v_proj",
                                     "o_proj"],
                  "batch": args.batch, "accum": args.accum,
                  "max_len": args.max_len, "dtype": args.dtype,
                  "attn": args.attn, "warmup": args.warmup,
                  "seed": args.seed, "val_frac": args.val_frac,
                  "val_cap": args.val_cap, "val_every": args.val_every,
                  "family_weight": fam_weights,
                  "length_bucket": args.length_bucket,
                  "grad_checkpoint": not args.no_grad_checkpoint},
        "optimizer": {"name": "AdamW",
                      "weight_decay": opt.defaults["weight_decay"],
                      "betas": list(opt.defaults["betas"]),
                      "eps": opt.defaults["eps"]},
        "versions": {"python": platform.python_version(),
                     "torch": torch.__version__,
                     "transformers": transformers.__version__,
                     "peft": peft.__version__},
        "steps": step,
        "final_loss_token_weighted": round(
            cum_loss / max(1, cum_tokens), 4),
        "tokens_per_second_wall": round(
            cum_tokens / max(1e-9, time.time() - t0)),
        "peak_memory_bytes": peak_mem,
        "loss_surface": "verdict tokens only (y=1 segments + EOS)",
        "best_checkpoint": {"step": best["step"],
                            "val_loss": round(best["val_loss"], 4)}
        if best["step"] >= 0 else None,
    }
    (args.out / "train-manifest.json").write_text(
        json.dumps(manifest, indent=1) + "\n")
    log(f"manifest -> {args.out / 'train-manifest.json'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
