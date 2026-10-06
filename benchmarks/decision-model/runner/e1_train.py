#!/usr/bin/env python3
"""LoRA-train a decision head on verdict-slot SFT rows (E1, #88).

Consumes `decision_sft_prep.py` rows (segmented macjev-render-v1
renders) and teaches exactly the readout the serving path scores:
loss lands only on the y=1 verdict segments (" yes"/" no" at the
option slots); everything else — the state, the question, the option
lines, the `` ->`` markers — is masked context. EOS is appended and
supervised, so the model learns to stop after the last slot.

Knobs are pre-registered in docs/TRAINING.md (E1 cell): LoRA r16 /
alpha 32 / dropout 0.05 on q,k,v,o projections, lr 1e-4 cosine, one
epoch for the first pass, loss on verdict tokens only. The gate the
tuned model must clear: beat the untuned base AND the community
checkpoint (suite >= 0.68, JevBench >= 0.6494) through the readout
serving path.

torch/transformers/peft import lazily so the pure helpers (segment
encoding, label masking, batching) unit-test on machines without
torch. Every knob is a CLI flag with the pre-registered default; a
run records its full knob set + data sha in the output manifest.

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
import sys
from pathlib import Path


def sha256_of(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def encode_row(segments: list[dict], encode_fn, eos_id: int,
               max_len: int) -> tuple[list[int], list[int]]:
    """One row -> (input_ids, labels). Each segment is encoded
    separately (boundaries at ' ->'/verdict tokens are stable token
    edges); context ids are masked to -100, verdict ids supervise.
    EOS is appended and supervised. Rows longer than max_len after
    truncation keep their first max_len tokens with the mask intact —
    the prep tool's length census says this fires for ~0 rows."""
    ids: list[int] = []
    labels: list[int] = []
    for seg in segments:
        seg_ids = encode_fn(seg["t"])
        ids.extend(seg_ids)
        labels.extend(seg_ids if seg["y"] else [-100] * len(seg_ids))
    ids.append(eos_id)
    labels.append(eos_id)
    return ids[:max_len], labels[:max_len]


class PadCollate:
    """Right-pad input_ids/labels to the batch max with pad/eos-id and
    -100 respectively; attention mask marks real tokens."""

    def __init__(self, pad_id: int):
        self.pad_id = pad_id

    def __call__(self, batch: list[tuple[list[int], list[int]]]):
        max_len = max(len(ids) for ids, _ in batch)
        input_ids, labels, attn = [], [], []
        for ids, lab in batch:
            pad = max_len - len(ids)
            input_ids.append(ids + [self.pad_id] * pad)
            labels.append(lab + [-100] * pad)
            attn.append([1] * len(ids) + [0] * pad)
        return input_ids, labels, attn


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
    ap.add_argument("--warmup", type=float, default=0.03)
    ap.add_argument("--limit-rows", type=int, default=0,
                    help="smoke: only the first N rows")
    ap.add_argument("--seed", type=int, default=42)
    args = ap.parse_args()

    try:
        import torch
        from peft import LoraConfig, get_peft_model
        from transformers import (AutoModelForCausalLM, AutoTokenizer,
                                  get_cosine_schedule_with_warmup)
    except ImportError as e:
        print(f"torch stack unavailable on this host: {e}", file=sys.stderr)
        return 1

    rows_path = args.data / "e1-sft-v1.rows.jsonl"
    rows = [json.loads(line) for line in rows_path.open(encoding="utf-8")
            if line.strip()]
    if args.limit_rows:
        rows = rows[:args.limit_rows]
    print(f"rows={len(rows)}", flush=True)

    torch.manual_seed(args.seed)
    tok = AutoTokenizer.from_pretrained(args.base)
    if tok.pad_token_id is None:
        tok.pad_token = tok.eos_token
    dtype = torch.float16 if args.dtype == "fp16" else torch.bfloat16
    model = AutoModelForCausalLM.from_pretrained(
        args.base, torch_dtype=dtype, attn_implementation="eager")
    model.config.use_cache = False
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

    feats = []
    trunc = 0
    for row in rows:
        ids, labels = encode_row(row["segments"], tok.encode, eos_id,
                                 args.max_len)
        trunc += (len(ids) == args.max_len)
        feats.append((ids, labels))
    print(f"feats={len(feats)} truncated_at_max={trunc}", flush=True)

    steps_per_epoch = max(1, len(feats) // (args.batch * args.accum))
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
        print(msg, flush=True)

    model.train()
    order = list(range(len(feats)))
    step = 0
    micro = 0
    import random

    random.Random(args.seed).shuffle(order)
    for epoch in range(int(args.epochs + 0.999)):
        if step >= total_steps:
            break
        random.Random(args.seed + epoch).shuffle(order)
        epoch_loss = 0.0
        for start in range(0, len(order), args.batch):
            batch = [feats[i] for i in order[start:start + args.batch]]
            input_ids, labels, attn = collate(batch)
            t_ids = torch.tensor(input_ids, device=model.device)
            t_lab = torch.tensor(labels, device=model.device)
            t_att = torch.tensor(attn, device=model.device)
            loss = model(input_ids=t_ids, attention_mask=t_att,
                         labels=t_lab).loss
            scaler.scale(loss / args.accum).backward()
            epoch_loss += loss.item()
            micro += 1
            if micro % args.accum == 0:
                scaler.unscale_(opt)
                torch.nn.utils.clip_grad_norm_(
                    [p for p in model.parameters() if p.requires_grad], 1.0)
                scaler.step(opt)
                scaler.update()
                sched.step()
                opt.zero_grad(set_to_none=True)
                step += 1
                if step % 20 == 0 or step == total_steps:
                    log(f"step={step}/{total_steps} "
                        f"loss={epoch_loss / max(1, micro):.4f} "
                        f"lr={sched.get_last_lr()[0]:.2e}")
                if step >= total_steps:
                    break
        log(f"epoch={epoch} mean_loss={epoch_loss / max(1, micro):.4f}")

    model.save_pretrained(str(args.out / "adapter"))
    tok.save_pretrained(str(args.out / "adapter"))
    manifest = {
        "manifest_version": "opencodifier.e1-train/1",
        "base": args.base,
        "data": {"path": str(rows_path), "sha256": sha256_of(rows_path),
                 "rows": len(rows)},
        "knobs": {"epochs": args.epochs, "lr": args.lr,
                  "lora_r": args.lora_r, "lora_alpha": args.lora_alpha,
                  "lora_dropout": args.lora_dropout,
                  "target_modules": ["q_proj", "k_proj", "v_proj", "o_proj"],
                  "batch": args.batch, "accum": args.accum,
                  "max_len": args.max_len, "dtype": args.dtype,
                  "warmup": args.warmup, "seed": args.seed},
        "steps": step,
        "final_loss": round(epoch_loss / max(1, micro), 4),
        "truncated_at_max_len": trunc,
        "loss_surface": "verdict tokens only (y=1 segments + EOS)",
    }
    (args.out / "train-manifest.json").write_text(
        json.dumps(manifest, indent=1) + "\n")
    log(f"manifest -> {args.out / 'train-manifest.json'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
