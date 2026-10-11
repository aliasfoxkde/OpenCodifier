#!/usr/bin/env python3
"""VIVERE teacher-scoring campaign: merged-v4 records -> per-row teacher
distributions over the closed option set.

Two modes:

``emit``  walks merged-v4 exactly like decision_sft_prep (same skips,
          same state budget, same per-question rendering inputs) but
          emits gold-free scoring items: id, qtype, instructions,
          options (ids + descriptions), state text. The gold label is
          never read, so a teacher cannot leak it and the facts are
          honest teacher beliefs. A parity self-test rebuilds each
          qtype's first row through decision_sft_prep's segments_for
          and asserts the y=0 context pieces match this renderer.

``score`` loads base (+optional LoRA adapter), renders each item in the
          training-time shape (render_header + per-option ``->`` slot
          lines), finds per-slot verdict positions (e1_eval's
          slot_positions), and reads the per-slot yes/no logit
          difference, softmaxed over slots — the same readout the suite
          eval uses, so teacher facts and gate numbers share one
          mechanism. Shardable: --shard i --nshards n splits rows
          deterministically for multi-GPU runs.

Outputs (compute host, not committed; manifests carry sha256 pins):
    items.jsonl        {id, qtype, instructions, options, ids, state}
    teacherfacts.jsonl {id, qtype, n_options, probs, argmax_id, top,
                        margin, entropy} + manifest sidecar

Usage:
    python3 teacher_score_rows.py emit --corpus merged-v4.jsonl \
        --out items.jsonl
    python3 teacher_score_rows.py score --base models/Qwen3.5-2B \
        --adapter runs/arm/adapter-best --items items.jsonl \
        --out teacherfacts.jsonl --dtype fp16 --shard 0 --nshards 2

Downstream: facts feed corpus_gates.py (merged-line noise gate) and the
e1-softlabel-v1 build (teacher KL targets) — see
docs/planning/e1-08b-next-arms.md.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import decision_sft_prep as prep  # noqa: E402
from e1_eval import sha256_of, slot_positions  # noqa: E402


def say(msg: str) -> None:
    sys.stdout.write(msg + "\n")
    sys.stdout.flush()


def render_slots(options: list[str]) -> str:
    """Slot lines exactly as segments_for writes them: options_for's
    lines are already ``id: description`` (or bare levels); the context
    piece is the line verbatim plus `` ->``."""
    return "\n".join(f"{opt} ->" for opt in options)


def render_item(state: str, qtype: str, instructions: str,
                options: list[str]) -> str:
    """Scoring render: header from prep, then slots, verdicts absent —
    e1_eval's suite shape applied to corpus rows."""
    header = prep.render_header(qtype, instructions, options, state)
    return header + render_slots(options) + "\n"


def parity_self_test(rec: dict) -> None:
    """Assert this module's slot rendering matches segments_for's y=0
    context pieces for one row of each qtype the record carries."""
    state = prep.state_text(rec)
    req = rec.get("request") or {}
    seen: set[str] = set()
    for qname, q in (req.get("questions") or {}).items():
        qtype = q.get("type")
        if qtype in seen:
            continue
        options = prep.options_for(q)
        if options is None:
            continue
        ids = prep.option_ids(q)
        label = ids[0]  # dummy; only context pieces are compared
        segs = prep.segments_for(ids, options, label)
        # segments_for's y=0 run == slot lines + trailing newline; that
        # run is what the model reads option text from at train time.
        expected = "".join(s["t"] for s in segs if s["y"] == 0)
        got = render_slots(options) + "\n"
        if got != expected:
            raise SystemExit(
                f"parity self-test FAILED for qtype={qtype}:\n"
                f"got:      {got!r}\nexpected: {expected!r}")
        seen.add(qtype)


def cmd_emit(args: argparse.Namespace) -> int:
    counts: Counter = Counter()
    n_out = 0
    with args.corpus.open(encoding="utf-8") as src, \
            args.out.open("w", encoding="utf-8") as dst:
        for line in src:
            if not line.strip():
                continue
            rec = json.loads(line)
            rid = str(rec.get("record_id") or "")
            source = str(rec.get("source") or "")
            if source.startswith("suite"):
                counts["skipped: suite row"] += 1
                continue
            state = prep.state_text(rec)
            if not state:
                counts["skipped: empty state"] += 1
                continue
            if len(state) > args.max_state_chars:
                counts["skipped: state over budget"] += 1
                continue
            if n_out < 3 and args.parity:
                parity_self_test(rec)
                counts["parity self-test ok"] += 1
            req = rec.get("request") or {}
            for qname, q in (req.get("questions") or {}).items():
                qtype = q.get("type")
                options = prep.options_for(q)
                if options is None:
                    counts[f"skipped: unrenderable {qtype} criteria"] += 1
                    continue
                ids = prep.option_ids(q)
                instructions = str(q.get("instructions") or "").strip()
                if not instructions:
                    counts["skipped: empty instructions"] += 1
                    continue
                dst.write(json.dumps({
                    "id": f"{rid}::{qname}",
                    "record_id": rid,
                    "question": qname,
                    "qtype": qtype,
                    "n_options": len(ids),
                    "ids": ids,
                    "options": options,
                    "instructions": instructions,
                    "state": state,
                }, ensure_ascii=False) + "\n")
                counts[f"emitted: {qtype}"] += 1
                n_out += 1
                if args.limit and n_out >= args.limit:
                    break
            if args.limit and n_out >= args.limit:
                break

    manifest = {
        "manifest_version": "opencodifier.teacher-score-items/1",
        "readout": "macjev-render-v1 slots, yes/no verdict tokens, "
                   "gold-free (labels never read)",
        "corpus": {"path": args.corpus.name,
                   "sha256": sha256_of(args.corpus)},
        "max_state_chars": args.max_state_chars,
        "parity_self_test": bool(args.parity),
        "counts": dict(counts),
        "rows": n_out,
    }
    args.out.with_suffix(".manifest.json").write_text(
        json.dumps(manifest, indent=1) + "\n")
    say(json.dumps({"rows": n_out, "counts": dict(counts)}, indent=1))
    return 0


def fact_row(it: dict, scores: list[float]) -> dict:
    """Fact dict from per-slot yes/no logit diffs — softmax over slots,
    exactly the suite readout's math (one helper so the batch-1 and
    batched paths cannot drift)."""
    mx = max(scores)
    exps = [math.exp(s - mx) for s in scores]
    total = sum(exps)
    probs = [e / total for e in exps]
    k = max(range(len(scores)), key=lambda i: scores[i])
    ent = -sum(p * math.log(max(p, 1e-12)) for p in probs)
    return {
        "id": it["id"],
        "qtype": it["qtype"],
        "n_options": it["n_options"],
        "probs": [round(p, 6) for p in probs],
        "argmax_id": it["ids"][k],
        "top": round(probs[k], 6),
        "margin": (round(probs[k] - sorted(probs)[-2], 6)
                   if len(probs) > 1 else 1.0),
        "entropy": round(ent, 6),
    }


def render_len(it: dict) -> int:
    """Character-length proxy for the rendered item (batch bucketing
    only; never read by the readout)."""
    return (len(it["state"]) + len(it["instructions"])
            + sum(len(o) for o in it["options"]))


_W_TOK = None


def _prep_init(base_path: str) -> None:
    """Worker initializer: one tokenizer per prepare process."""
    global _W_TOK
    from transformers import AutoTokenizer
    _W_TOK = AutoTokenizer.from_pretrained(base_path)


def _prep_item(item: dict) -> tuple:
    """Render + locate slot positions + tokenize (CPU-only; runs in a
    worker process so it never serializes against the GPU)."""
    text = render_item(item["state"], item["qtype"],
                       item["instructions"], item["options"])

    def enc(s: str) -> list[int]:
        return _W_TOK.encode(s, add_special_tokens=False)

    positions = slot_positions(text, item["n_options"], enc)
    ids = enc(text) if positions is not None else None
    return positions, ids


def cmd_score(args: argparse.Namespace) -> int:
    try:
        import torch
        from peft import PeftModel
        from transformers import AutoModelForCausalLM, AutoTokenizer
    except ImportError as e:
        say(f"torch stack unavailable: {e}")
        return 1

    items = [json.loads(l) for l in args.items.open(encoding="utf-8")
             if l.strip()]
    if args.nshards > 1:
        items = [it for k, it in enumerate(items)
                 if k % args.nshards == args.shard]
    if args.limit:
        items = items[:args.limit]

    tok = AutoTokenizer.from_pretrained(args.base)
    dtype = torch.float16 if args.dtype == "fp16" else torch.bfloat16
    model = AutoModelForCausalLM.from_pretrained(
        args.base, dtype=dtype, attn_implementation=args.attn)
    if args.adapter:
        model = PeftModel.from_pretrained(model, args.adapter)
    model.eval()
    device = "cuda" if torch.cuda.is_available() else "cpu"
    model.to(device)

    yes_id = tok.encode(" yes", add_special_tokens=False)
    no_id = tok.encode(" no", add_special_tokens=False)
    if len(yes_id) != 1 or len(no_id) != 1:
        say(f"verdict tokens are not single tokens: yes={yes_id} "
            f"no={no_id}")
        return 1
    yes_id, no_id = yes_id[0], no_id[0]

    def encode_fn(s: str) -> list[int]:
        return tok.encode(s, add_special_tokens=False)

    # Incremental output + resume: facts append as scored, a restarted
    # shard skips ids already present. A long campaign must not lose
    # hours of scoring to a late OOM.
    args.out.parent.mkdir(parents=True, exist_ok=True)
    done_ids: set[str] = set()
    if args.out.exists():
        with args.out.open(encoding="utf-8") as fh:
            for line in fh:
                line = line.strip()
                if line:
                    done_ids.add(json.loads(line)["id"])
        if done_ids:
            say(f"resume: {len(done_ids)} rows already scored")
    dst = args.out.open("a", encoding="utf-8")
    state = {"n": len(done_ids)}

    def emit_row(row: dict) -> None:
        if row["id"] in done_ids:
            return
        done_ids.add(row["id"])
        dst.write(json.dumps(row, ensure_ascii=False) + "\n")
        state["n"] += 1

    errors: Counter = Counter()
    if args.batch <= 1:
        with torch.no_grad():
            for n, it in enumerate(items):
                if it["id"] in done_ids:
                    continue
                text = render_item(it["state"], it["qtype"],
                                   it["instructions"], it["options"])
                positions = slot_positions(text, it["n_options"], encode_fn)
                if positions is None:
                    errors["slot mismatch"] += 1
                    emit_row({"id": it["id"], "probs": None,
                              "error": "slot positions != options"})
                    continue
                ids = encode_fn(text)
                logits = model(torch.tensor([ids], device=device)).logits[0]
                scores = [float(logits[p, yes_id] - logits[p, no_id])
                          for p in positions]
                emit_row(fact_row(it, scores))
                if (n + 1) % 500 == 0:
                    say(f"{n + 1}/{len(items)}")
    else:
        # Length-bucketed right-padded batches; rows are appended in
        # bucket order (the downstream join is id-keyed, order-free).
        # Right padding keeps every real position's logits identical to
        # the unpadded forward (causal attention + tail-only pads).
        # The forward runs the backbone and applies lm_head ONLY at the
        # per-row slot positions — a full [batch, seqlen, vocab] logits
        # tensor is the OOM term (the corpus head carries ~7k-token
        # items) and is almost entirely discarded anyway. Batches are
        # capped by a padded-token budget instead.
        causal = model.get_base_model() if args.adapter else model
        backbone, lm_head = causal.model, causal.lm_head
        order = sorted(range(len(items)), key=lambda i: -render_len(items[i]))
        pad_id = tok.pad_token_id if tok.pad_token_id is not None \
            else (tok.eos_token_id or 0)
        token_budget = args.token_budget  # padded tokens per forward
        done = 0
        if args.workers > 1:
            import multiprocessing as mp
            from concurrent.futures import ProcessPoolExecutor
            # spawn: workers must not inherit the parent's CUDA context
            ctx = mp.get_context("spawn")
            pool = ProcessPoolExecutor(
                max_workers=args.workers, mp_context=ctx,
                initializer=_prep_init, initargs=(args.base,))
            say(f"prep workers: {args.workers}")
            prep_iter = pool.map(_prep_item, [items[i] for i in order],
                                 chunksize=64)
        else:
            _prep_init(args.base)
            prep_iter = map(_prep_item, (items[i] for i in order))
        stream = iter(prep_iter)
        buf: list = []
        idx = 0
        n_batches = 0

        def pull(n: int) -> None:
            nonlocal idx
            while len(buf) < n and idx < len(order):
                positions, ids = next(stream)
                if items[order[idx]]["id"] not in done_ids:
                    buf.append((items[order[idx]], positions, ids))
                idx += 1

        with torch.inference_mode():
            while buf or idx < len(order):
                if not buf:
                    pull(1)
                kept = []
                # first buffered item fixes the batch's padded length
                while buf:
                    it, positions, ids = buf[0]
                    if positions is None:
                        errors["slot mismatch"] += 1
                        emit_row({"id": it["id"], "probs": None,
                                  "error": "slot positions != options"})
                        buf.pop(0)
                        continue
                    maxlen = len(ids)
                    if (len(kept) + 1) * maxlen > token_budget \
                            or len(kept) >= args.batch:
                        break
                    kept.append(buf.pop(0))
                    pull(1)
                if not kept:
                    continue
                maxlen = max(len(ids) for _, _, ids in kept)
                input_ids = torch.full((len(kept), maxlen), pad_id,
                                       dtype=torch.long)
                attn = torch.zeros((len(kept), maxlen), dtype=torch.long)
                for r, (_, _, ids) in enumerate(kept):
                    input_ids[r, :len(ids)] = torch.tensor(ids)
                    attn[r, :len(ids)] = 1
                hidden = backbone(
                    input_ids=input_ids.to(device),
                    attention_mask=attn.to(device),
                ).last_hidden_state
                flat = [(r, p) for r, (_, positions, _)
                        in enumerate(kept) for p in positions]
                sel = hidden[torch.tensor([r for r, _ in flat],
                                          device=device),
                             torch.tensor([p for _, p in flat],
                                          device=device)]
                sel_logits = lm_head(sel)
                off = 0
                for r, (it, positions, _) in enumerate(kept):
                    k = len(positions)
                    scores = [float(sel_logits[off + j, yes_id]
                                    - sel_logits[off + j, no_id])
                              for j in range(k)]
                    off += k
                    emit_row(fact_row(it, scores))
                n_batches += 1
                if n_batches % 100 == 0:
                    dst.flush()
                    if torch.cuda.is_available():
                        torch.cuda.empty_cache()
                if done // 500 < idx // 500 or idx == len(items):
                    say(f"{idx}/{len(items)}")
                done = idx

    dst.flush()
    dst.close()
    adapter_sha = (sha256_of(Path(args.adapter) / "adapter_model.safetensors")
                   if args.adapter else None)
    manifest = {
        "manifest_version": "opencodifier.teacher-facts/1",
        "base": {"path": args.base},
        "adapter": {"path": args.adapter or None,
                    "sha256": adapter_sha},
        "items": {"path": str(args.items),
                  "sha256": sha256_of(args.items)},
        "shard": {"index": args.shard, "of": args.nshards},
        "dtype": args.dtype, "attn": args.attn,
        "batch": args.batch,
        "prep_workers": args.workers,
        "token_budget": args.token_budget,
        "batch_order": ("length-bucketed (id-keyed join, order-free)"
                        if args.batch > 1 else "items order"),
        "readout": "per-slot yes/no logit diff, softmax over slots "
                   "(e1_eval mechanism, corpus rows; batched path "
                   "selects lm_head at slot positions only)",
        "rows": state["n"],
        "errors": dict(errors),
    }
    args.out.with_suffix(".manifest.json").write_text(
        json.dumps(manifest, indent=1) + "\n")
    say(json.dumps({"rows": state["n"], "errors": dict(errors)}))
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    sub = ap.add_subparsers(dest="mode", required=True)

    e = sub.add_parser("emit", help="merged-v4 -> gold-free score items")
    e.add_argument("--corpus", type=Path, required=True)
    e.add_argument("--out", type=Path, required=True)
    e.add_argument("--limit", type=int, default=0)
    e.add_argument("--max-state-chars", type=int, default=24000)
    e.add_argument("--parity", type=int, default=1,
                   help="run the segments_for parity self-test on the "
                        "first records (1=on)")
    e.set_defaults(fn=cmd_emit)

    s = sub.add_parser("score", help="items -> teacher facts")
    s.add_argument("--base", required=True)
    s.add_argument("--adapter", default="")
    s.add_argument("--items", type=Path, required=True)
    s.add_argument("--out", type=Path, required=True)
    s.add_argument("--limit", type=int, default=0)
    s.add_argument("--dtype", choices=("fp16", "bf16"), default="fp16")
    s.add_argument("--attn", choices=("sdpa", "eager"), default="sdpa")
    s.add_argument("--shard", type=int, default=0)
    s.add_argument("--nshards", type=int, default=1)
    s.add_argument("--batch", type=int, default=1,
                   help="items per forward pass (1 = single-item "
                        "reference path, byte-identical to pre-batch "
                        "runs; >1 = length-bucketed right-padded "
                        "batches, lm_head at slot positions only)")
    s.add_argument("--workers", type=int, default=1,
                   help="prepare (render+tokenize) worker processes "
                        "for --batch > 1 (1 = serial, in-process)")
    s.add_argument("--token-budget", type=int, default=32768,
                   help="padded-token ceiling per forward pass in the "
                        "batched path (activation + FLA-state headroom; "
                        "16384 if the GPU is tight)")
    s.set_defaults(fn=cmd_score)

    args = ap.parse_args()
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main())
