#!/usr/bin/env python3
"""Offline ladder-gate refit on tree-shaped model-rung rows (F28).

fusion-v1's choice gate (min_margin 0.0183) was fitted on embedding-cosine
margins in the letters readout shape, but the shipped ladder applies it to
the lexical/BM25 rung serving the tree shape — a double mismatch. This
instrument joins two measured run JSONs and sweeps the gate exactly as
`ladder.rs` applies it:

  - relational/rule proofs (prob == 1.0): accept, never escalate
  - otherwise: accept the lexical answer iff prob >= min_confidence AND
    margin >= min_margin; else escalate to the model rung, whose answer
    the ladder accepts (fusion-v1 choice gates are 0.0)

Inputs:
  --lexical  engine lexical run WITH distributions (run_engine.py now
             stores `dist`/`margin`; re-capture if the run predates it)
  --tree     collect_tree.py output (tree rows under the engine rung's
             exact payload contract, full per-choice distribution)
  --engine-fusion  optional D26 record; when given, the sweep first
             reproduces its blended accuracy as a semantics check

Reports the reproduction, the Pareto frontier (accuracy vs escalations),
and the best operating point under the D26 budget (mean <= 1 s). Gate
semantics live here in exactly one place; the shipped ladder JSON must
cite this sweep.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path


def load_rows(path: Path) -> dict[str, dict]:
    data = json.loads(path.read_text())
    return {r["id"]: r for r in data["single"]}


def tree_pred(row: dict) -> tuple[str, float]:
    return row["pred"], float(row["prob"])


def simulate(
    items: list[dict],
    lex: dict[str, dict],
    tree: dict[str, dict],
    min_conf: float,
    min_margin: float,
) -> dict:
    correct = 0
    escalations = 0
    total_ms = 0.0
    for it in items:
        lid = it["id"]
        lrow = lex[lid]
        total_ms += lrow["wall_ms"]
        trow = tree[lid]
        tpred, tprob = tree_pred(trow)
        tms = trow["wall_ms"]
        if lrow["prob"] >= 1.0:  # relational/rule proof
            ok = lrow["pred"] == it["answer"]
        elif lrow["prob"] >= min_conf and lrow["margin"] >= min_margin:
            ok = lrow["pred"] == it["answer"]
        else:
            escalations += 1
            total_ms += tms
            ok = tpred == it["answer"]
        correct += ok
    n = len(items)
    return {
        "min_confidence": min_conf,
        "min_margin": min_margin,
        "accuracy": correct / n,
        "escalations": escalations,
        "mean_ms": total_ms / n,
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--lexical", type=Path, required=True)
    ap.add_argument("--tree", type=Path, required=True)
    ap.add_argument("--engine-fusion", type=Path, default=None)
    ap.add_argument("--budget-ms", type=float, default=1000.0)
    ap.add_argument("--target", type=float, default=0.85)
    args = ap.parse_args()

    suite = json.loads(
        (args.lexical.parent.parent / "suite" / "suite.json").read_text()
    ) if (args.lexical.parent.parent / "suite" / "suite.json").exists() else None
    lex = load_rows(args.lexical)
    tree = load_rows(args.tree)
    items = [
        {"id": i, "answer": lex[i]["answer"]} for i in sorted(lex) if i in tree
    ]
    if suite is not None:
        by_id = {i["id"]: i["answer"] for i in suite["items"]}
        items = [{"id": i["id"], "answer": by_id[i["id"]]} for i in items]

    print(f"joined items: {len(items)}")

    # Semantics check: reproduce fusion-v1 (min_conf 0.0, min_margin 0.0183).
    baseline = simulate(items, lex, tree, 0.0, 0.0183)
    print(
        f"fusion-v1 reproduction: acc={baseline['accuracy']:.3f} "
        f"esc={baseline['escalations']} mean={baseline['mean_ms']:.1f}ms"
    )
    if args.engine_fusion:
        rec = json.loads(args.engine_fusion.read_text())
        m = rec["metrics"]
        print(
            f"D26 record:            acc={m['accuracy']:.3f} "
            f"esc={m['outcomes'].get('verify', 0) + m['outcomes'].get('abstain', 0)}"
            f" mean={m['latency']['mean_ms']:.1f}ms"
        )

    conf_grid = [round(x, 4) for x in frange(0.0, 1.01, 0.05)]
    margin_grid = [round(x, 4) for x in frange(0.0, 0.51, 0.01)]
    results = [
        simulate(items, lex, tree, c, m)
        for c in conf_grid
        for m in margin_grid
    ]
    results.sort(key=lambda r: (-r["accuracy"], r["escalations"], r["mean_ms"]))

    budget = [r for r in results if r["mean_ms"] <= args.budget_ms]
    best = budget[0] if budget else results[0]
    at_target = [r for r in budget if r["accuracy"] >= args.target]
    print(f"\ntop operating points (mean <= {args.budget_ms:.0f} ms):")
    print("  min_conf  min_margin   acc    esc   mean_ms")
    for r in (at_target[:8] if at_target else budget[:8]):
        print(
            f"  {r['min_confidence']:.2f}      {r['min_margin']:.3f}"
            f"      {r['accuracy']:.3f}  {r['escalations']:3d}  {r['mean_ms']:7.1f}"
        )
    print(f"\nbest under budget: {json.dumps(best, sort_keys=True)}")
    frontier = pareto(results)
    print(f"pareto frontier ({len(frontier)} points, acc desc):")
    for r in frontier[:12]:
        print(
            f"  acc={r['accuracy']:.3f} esc={r['escalations']:3d} "
            f"mean={r['mean_ms']:7.1f}ms  (conf={r['min_confidence']:.2f}, "
            f"margin={r['min_margin']:.3f})"
        )
    return 0


def frange(start: float, stop: float, step: float) -> list[float]:
    out = []
    x = start
    while x < stop - 1e-9:
        out.append(x)
        x += step
    return out


def pareto(results: list[dict]) -> list[dict]:
    """Non-dominated by (accuracy up, escalations down, mean_ms down)."""
    out = []
    for r in results:
        dominated = any(
            o["accuracy"] >= r["accuracy"]
            and o["escalations"] <= r["escalations"]
            and o["mean_ms"] <= r["mean_ms"]
            and (
                o["accuracy"] > r["accuracy"]
                or o["escalations"] < r["escalations"]
                or o["mean_ms"] < r["mean_ms"]
            )
            for o in results
        )
        if not dominated:
            out.append(r)
    out.sort(key=lambda r: -r["accuracy"])
    return out


if __name__ == "__main__":
    raise SystemExit(main())
