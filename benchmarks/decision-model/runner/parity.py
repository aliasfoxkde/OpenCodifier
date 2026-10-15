#!/usr/bin/env python3
"""Reporting-parity metrics from frozen run JSONs (RESEARCH §15.6 item 8).

Board B rows historically carried accuracy + ECE + p50 only, which makes
two rungs with the same ECE look interchangeable when they separate on
every other parity axis. This script reads every frozen run JSON (never
rewriting one) and derives, per rung:

- ``brier_top1`` — Brier score of the winner probability against top-1
  correctness: mean over items of ``(p_top − 1{pred == answer})²``. This
  is the confidence Brier (the scored companion to the binned ECE), not a
  multiclass Brier — the frozen evidence stores the winner probability,
  not the full distribution, and a parity artifact must never pretend to
  more than its inputs carry.
- ``auroc_top1`` — AUROC of the winner probability as the score for the
  binary task "item answered correctly" (selective-classification
  discrimination: does higher confidence mean more often right?). Ties
  take midranks; ``null`` when every item shares one label (undefined).
- ``reliability`` — the 10-bin reliability curve over the winner
  probability, same equal-width binning (last bin inclusive of 1.0) as
  the ECE convention in run_engine.py; each bin carries n, mean p, acc.
- ``p95_ms`` / ``p99_ms`` — nearest-index percentiles over the per-item
  wall clock, the same convention as run_engine.py's p50/p95 (embed runs
  store no per-item wall clock and stay null there).
- ``tokens_per_decision`` — mean measured prompt tokens per item plus the
  one verdict token the parallel-decision readout emits; ``null`` where a
  rung records no per-item tokens (the zero-ML rungs consume no model
  tokens at all).
- ``sha256`` of each source file, so a parity row always points at the
  exact frozen bytes it was computed from.

Writes ``results/parity.json`` keyed by run file name plus
``results/RELIABILITY.md`` (the per-rung curves, human-readable). Both
are derived artifacts: regenerate with

  python3 runner/parity.py --runs-dir "$RUNS"
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

BINS = 10


def reliability_curve(pairs: list[tuple[float, int]]) -> list[dict]:
    """10 equal-width bins over the winner probability (ECE convention)."""
    out = []
    for i in range(BINS):
        lo, hi = i / BINS, (i + 1) / BINS
        sub = [(p, ok) for p, ok in pairs
               if lo <= p < hi or (i == BINS - 1 and p == hi)]
        if not sub:
            continue
        out.append({
            "lo": lo,
            "hi": hi,
            "n": len(sub),
            "mean_p": sum(p for p, _ in sub) / len(sub),
            "acc": sum(ok for _, ok in sub) / len(sub),
        })
    return out


def auroc(pairs: list[tuple[float, int]]) -> float | None:
    """AUROC of the probability as the score for correctness (midranks)."""
    n_pos = sum(ok for _, ok in pairs)
    n_neg = len(pairs) - n_pos
    if n_pos == 0 or n_neg == 0:
        return None
    ordered = sorted(pairs)  # by probability; list order breaks ties
    ranks: dict[int, float] = {}
    i = 0
    while i < len(ordered):
        j = i
        while j + 1 < len(ordered) and ordered[j + 1][0] == ordered[i][0]:
            j += 1
        mid = (i + j) / 2 + 1  # 1-based midrank for the tied block
        for k in range(i, j + 1):
            ranks[k] = mid
        i = j + 1
    pos_rank_sum = sum(ranks[k] for k, (_, ok) in enumerate(ordered) if ok)
    return (pos_rank_sum - n_pos * (n_pos + 1) / 2) / (n_pos * n_neg)


def nearest_index(values: list[float], q: float) -> float:
    return sorted(values)[int(len(values) * q)]


def parity_for(data: dict, sha256: str) -> dict:
    """Derived parity metrics for one run JSON (never mutates the input)."""
    rows = data.get("single") or []
    pairs = [(r["prob"], 1 if r["pred"] == r["answer"] else 0)
             for r in rows if "prob" in r and "pred" in r and "answer" in r]
    walls = [r["wall_ms"] for r in rows
             if isinstance(r.get("wall_ms"), (int, float))]
    # Cost-parity input: measured prompt tokens + the single verdict token
    # the parallel-decision readout emits.
    tokens = [r["context_tokens"] for r in rows
              if isinstance(r.get("context_tokens"), (int, float))]
    if not pairs:
        return {}
    lat = (data.get("metrics") or {}).get("latency") or {}
    out = {
        "n": len(pairs),
        "accuracy": sum(ok for _, ok in pairs) / len(pairs),
        "ece": (data.get("metrics") or {}).get("ece"),
        "brier_top1": sum((p - ok) ** 2 for p, ok in pairs) / len(pairs),
        "auroc_top1": auroc(pairs),
        "reliability": reliability_curve(pairs),
        "p50_ms": lat.get("p50_ms") or (nearest_index(walls, 0.50) if walls else None),
        "p95_ms": lat.get("p95_ms") or (nearest_index(walls, 0.95) if walls else None),
        "p99_ms": nearest_index(walls, 0.99) if walls else None,
        "tokens_per_decision":
            (sum(tokens) + len(tokens)) / len(tokens) if tokens else None,
        "sha256": sha256,
    }
    return {k: v for k, v in out.items() if v is not None}


def reliability_md(results: dict) -> str:
    lines = [
        "# Reliability curves — Board B rungs (generated)",
        "",
        "10 equal-width bins over the winner probability; per bin the mean",
        "probability and the top-1 accuracy of the items in it. Derived from",
        "the frozen run JSONs by `runner/parity.py`; the binning matches the",
        "ECE convention in `run_engine.py`. `n` is items in the bin.",
        "",
    ]
    for name in sorted(results):
        curve = (results[name].get("reliability") or [])
        if not curve:
            continue
        lines += [f"## {name}", "",
                  "| bin | n | mean p | acc |", "|---|---:|---:|---:|"]
        for b in curve:
            lines.append(
                f"| {b['lo']:.1f}–{b['hi']:.1f} | {b['n']} "
                f"| {b['mean_p']:.3f} | {b['acc']:.3f} |")
        lines.append("")
    return "\n".join(lines)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    here = Path(__file__).resolve().parent
    ap.add_argument("--runs-dir", type=Path,
                    default=Path("/nas/Temp/work/oc-model-eval/runs"))
    ap.add_argument("--out", type=Path,
                    default=here.parent / "results" / "parity.json")
    ap.add_argument("--reliability-md", type=Path,
                    default=here.parent / "results" / "RELIABILITY.md")
    args = ap.parse_args()
    if not args.runs_dir.is_dir():
        sys.stderr.write(f"runs dir not found: {args.runs_dir}\n")
        return 1

    results = {}
    for path in sorted(args.runs_dir.glob("*.json")):
        if path.name == "models.manifest.json":
            continue
        data = json.loads(path.read_text())
        # Chat-only screens have no decision arm — nothing to calibrate.
        if not (data.get("single") or []):
            continue
        parity = parity_for(data, hashlib.sha256(path.read_bytes()).hexdigest())
        if parity:
            results[path.name] = parity

    args.out.write_text(json.dumps(results, sort_keys=True, indent=1) + "\n")
    args.reliability_md.write_text(reliability_md(results))
    sys.stdout.write(
        f"{len(results)} rungs -> {args.out} (+ {args.reliability_md.name})\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
