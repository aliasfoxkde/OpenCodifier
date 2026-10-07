#!/usr/bin/env python3
"""Slot-readout evaluation for E1 decision heads (oc-readout-v1).

The serving-path readout for a trained in-house decision model: render
the item in the same macjev layout the SFT data used (with the verdict
tokens ABSENT), teacher-force one forward pass, and read each option's
verdict logits at its `` ->`` slot position —
score_k = logit(" yes") - logit(" no") — then softmax over slots. No
generation, one decode per item. This is the arm that the D16 rows say
matters: a model trained on the wrong interface scores 0.217 through
this readout; a trained-native model is the reference architecture.

Suite items (choice: candidates with id+description, gold answer id)
map directly. The suite must never be a training source
(decision_sft_prep excludes suite rows); this tool only reads it.

Usage:
    python3 runner/e1_eval.py \
        --base ~/oc-model-eval/models/hf/Qwen3.5-0.8B \
        [--adapter ~/oc-model-eval/runs/e1-smoke-tiny/adapter] \
        --suite suite/suite.json \
        --out ~/oc-model-eval/results/e1-readout-smoke [--limit N]
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import sys
from collections import Counter
from pathlib import Path

SLOT_MARKER = " ->\n"


def sha256_of(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def render_choice(context: str, question: str,
                  candidates: list[dict]) -> tuple[str, list[str]]:
    """macjev render with verdict slots present and verdict tokens
    absent. Returns (render_text, option_ids) in slot order. Matches
    decision_sft_prep's choice rendering exactly."""
    opts = "\n".join(f"- {c['id']}: {c['description']}"
                     for c in candidates)
    header = (f"State:\n{context}\n\nQuestion [choice]: {question}\n"
              f"Options:\n{opts}\nJudge each option:\n")
    slots = "\n".join(f"{c['id']}: {c['description']} ->"
                      for c in candidates)
    return header + slots + "\n", [c["id"] for c in candidates]


def slot_positions(text: str, n_expected: int,
                   encode_fn) -> list[int] | None:
    """Token indexes whose logits predict each option's verdict.

    The SFT rows end every slot context segment with `` ->``; at
    inference the render is the same text with verdicts absent, so the
    position predicting a slot's verdict is the LAST token of the
    prefix ending at that slot's `` ->``. Found by re-encoding the
    truncated text per marker occurrence — robust to tokenizer merges
    at slot boundaries. None if the markers do not line up with the
    expected option count."""
    positions: list[int] = []
    start = 0
    for _ in range(n_expected):
        j = text.find(SLOT_MARKER, start)
        if j < 0:
            return None
        positions.append(len(encode_fn(text[:j + len(SLOT_MARKER)])) - 1)
        start = j + len(SLOT_MARKER)
    return positions if len(positions) == n_expected else None


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--base", required=True)
    ap.add_argument("--adapter", default="")
    ap.add_argument("--suite", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument("--dtype", choices=("fp16", "bf16"), default="bf16")
    args = ap.parse_args()

    try:
        import torch
        from peft import PeftModel
        from transformers import AutoModelForCausalLM, AutoTokenizer
    except ImportError as e:
        print(f"torch stack unavailable: {e}", file=sys.stderr)
        return 1

    suite = json.load(args.suite.open(encoding="utf-8"))
    items = suite["items"]
    if args.limit:
        items = items[:args.limit]

    tok = AutoTokenizer.from_pretrained(args.base)
    dtype = torch.float16 if args.dtype == "fp16" else torch.bfloat16
    model = AutoModelForCausalLM.from_pretrained(
        args.base, dtype=dtype, attn_implementation="eager")
    if args.adapter:
        model = PeftModel.from_pretrained(model, args.adapter)
    model.eval()
    device = "cuda" if torch.cuda.is_available() else "cpu"
    model.to(device)

    yes_id = tok.encode(" yes", add_special_tokens=False)
    no_id = tok.encode(" no", add_special_tokens=False)
    if len(yes_id) != 1 or len(no_id) != 1:
        print(f"verdict tokens are not single tokens: yes={yes_id} "
              f"no={no_id}", file=sys.stderr)
        return 1
    yes_id, no_id = yes_id[0], no_id[0]

    def encode_fn(s: str) -> list[int]:
        return tok.encode(s, add_special_tokens=False)

    rows = []
    by_class: Counter = Counter()
    n_class: Counter = Counter()
    conf_sum: Counter = Counter()
    margin_sum: Counter = Counter()
    ent_sum: Counter = Counter()
    bin_ok: Counter = Counter()
    bin_n: Counter = Counter()
    bin_conf: Counter = Counter()
    with torch.no_grad():
        for it in items:
            text, opt_ids = render_choice(it["context"], it["question"],
                                          it["candidates"])
            positions = slot_positions(text, len(opt_ids), encode_fn)
            if positions is None:
                rows.append({"id": it["id"], "pred": None,
                             "error": "slot positions != options"})
                continue
            ids = encode_fn(text)
            logits = model(torch.tensor([ids], device=device)).logits[0]
            scores = [float(logits[p, yes_id] - logits[p, no_id])
                      for p in positions]
            mx = max(scores)
            exps = [math.exp(s - mx) for s in scores]
            total = sum(exps)
            probs = [e / total for e in exps]
            k = max(range(len(scores)), key=lambda i: scores[i])
            pred = opt_ids[k]
            ok = pred == it["answer"]
            # Post-hoc confidence anatomy (A6): margin between the top
            # two slot probabilities and Shannon entropy over the slot
            # distribution — the r2 post-mortem showed relational items
            # separate from lexical ones on these long before accuracy
            # does. ECE bins use the predicted-slot probability.
            srt = sorted(probs, reverse=True)
            margin = srt[0] - (srt[1] if len(srt) > 1 else 0.0)
            ent = -sum(p * math.log(p) for p in probs if p > 0)
            cls = it["class"]
            by_class[cls] += int(ok)
            n_class[cls] += 1
            conf_sum[cls] += probs[k]
            margin_sum[cls] += margin
            ent_sum[cls] += ent
            bin_i = min(9, int(probs[k] * 10))
            bin_ok[bin_i] += int(ok)
            bin_n[bin_i] += 1
            bin_conf[bin_i] += probs[k]
            rows.append({"id": it["id"], "class": cls,
                         "answer": it["answer"], "pred": pred,
                         "p": round(probs[k], 4),
                         "margin": round(margin, 4),
                         "entropy": round(ent, 4), "correct": ok,
                         "slot_scores": [round(s, 4) for s in scores]})

    n_ok = sum(1 for r in rows if "correct" in r)
    acc = sum(r["correct"] for r in rows if "correct" in r)
    # Expected Calibration Error over 10 predicted-probability bins:
    # sum over bins of (bin share) * |bin accuracy - bin mean confidence|.
    n_scored = sum(bin_n.values())
    ece = sum(bin_n[b] / n_scored
              * abs(bin_ok[b] / bin_n[b] - bin_conf[b] / bin_n[b])
              for b in bin_n) if n_scored else None
    args.out.mkdir(parents=True, exist_ok=True)
    with (args.out / "rows.jsonl").open("w", encoding="utf-8") as fh:
        for r in rows:
            fh.write(json.dumps(r, ensure_ascii=False) + "\n")
    summary = {
        "readout": "oc-readout-v1 (macjev layout, yes/no slot logits)",
        "base": args.base,
        "adapter": args.adapter or None,
        "suite": {"path": str(args.suite), "sha256": sha256_of(args.suite)},
        "items": len(rows),
        "scored": n_ok,
        "accuracy": round(acc / n_ok, 4) if n_ok else None,
        "ece": round(ece, 4) if ece is not None else None,
        "confidence_by_class": {
            c: {
                "mean_top_prob": round(conf_sum[c] / n_class[c], 4),
                "mean_margin": round(margin_sum[c] / n_class[c], 4),
                "mean_entropy": round(ent_sum[c] / n_class[c], 4),
            }
            for c in sorted(n_class)},
        "accuracy_by_class": {c: {"acc": round(by_class[c] / n_class[c], 4),
                                  "n": n_class[c]}
                              for c in sorted(n_class)},
    }
    (args.out / "summary.json").write_text(
        json.dumps(summary, indent=1) + "\n")
    print(json.dumps(summary, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
