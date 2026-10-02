#!/usr/bin/env python3
"""Post-hoc position-prior study over JevBench arms.

RESEARCH.md's laya study flagged candidate order as the untested
structural lever: a per-slot readout can plausibly prefer early
positions (primacy) regardless of content. The full permutation replay
needs a server slot; the cheap half needs nothing — every arm's
predictions can be indexed against its task's ordered candidate list
*right now*, asking:

- does the arm's winner land on early positions more often than the
  gold answers do (a position prior)?
- does accuracy degrade as the gold answer moves later (primacy
  cost)?

Both are computed against the dataset's own gold-position distribution,
so a skewed benchmark cannot masquerade as a skewed model. Everything
comes from already-measured run dirs; no inference is re-done.

Usage:
  python3 runner/position_bias.py --ref <jevbench-ref> \
      --arm <run_dir> [--arm <run_dir> ...] [--out report.md]
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import defaultdict
from pathlib import Path

SPLITS = ("easy.jsonl", "hard.jsonl", "original.jsonl")


def load_tasks(ref: Path) -> dict[str, dict]:
    tasks = {}
    for split in SPLITS:
        path = ref / "datasets" / "public" / split
        for line in path.read_text().splitlines():
            if not line.strip():
                continue
            t = json.loads(line)
            tasks[t["id"]] = {"labels": t["labels"], "expected": t["expected"]}
    return tasks


def load_arm(dir_path: Path) -> dict[str, dict]:
    rows = {}
    for line in (dir_path / "results.jsonl").read_text().splitlines():
        if not line.strip():
            continue
        r = json.loads(line)
        rows[r["task_id"]] = {"pred": r.get("predicted"),
                              "correct": bool(r.get("correct"))}
    return rows


def study(name: str, tasks: dict[str, dict], arm: dict[str, dict]) -> dict:
    """Aggregate pred/gold position counts, normalized per cardinality.

    Uniform-position expectation inside a k-choice item is 1/k, so each
    item contributes 1/k to every position of its cardinality; a share
    above that at position 0 is a prior, not noise.
    """
    pred_share: dict[int, dict[int, float]] = defaultdict(lambda: defaultdict(float))
    gold_share: dict[int, dict[int, float]] = defaultdict(lambda: defaultdict(float))
    acc_by_gold: dict[int, dict[int, list[int]]] = defaultdict(lambda: defaultdict(list))
    items = 0
    skipped = 0
    for tid, row in arm.items():
        t = tasks.get(tid)
        if not t:
            continue
        k = len(t["labels"])
        # score-type items carry integer expected levels over string labels.
        pred = None if row["pred"] is None else str(row["pred"])
        if pred is not None and pred in t["labels"]:
            p = t["labels"].index(pred)
            pred_share[k][p] += 1.0 / k
        else:
            skipped += 1
        g = t["labels"].index(str(t["expected"]))
        gold_share[k][g] += 1.0 / k
        acc_by_gold[k][g].append(int(row["correct"]))
        items += 1

    cards = sorted(set(pred_share) | set(gold_share))
    rows = []
    for k in cards:
        denom = sum(gold_share[k].values())
        if denom == 0:
            continue
        positions = sorted(set(pred_share[k]) | set(gold_share[k]))
        for pos in positions:
            rows.append({
                "k": k, "pos": pos,
                "pred": pred_share[k].get(pos, 0.0) / denom,
                "gold": gold_share[k].get(pos, 0.0) / denom,
                "acc": (sum(acc_by_gold[k].get(pos, [])) / len(acc_by_gold[k][pos])
                        if acc_by_gold[k].get(pos) else None),
                "n": len(acc_by_gold[k].get(pos, [])),
            })
    # Cardinality-weighted summary: total share each position captures
    # across all items (each cardinality's normalized distribution counts
    # once, so the totals sum to 1 across positions).
    tot_pred = defaultdict(float)
    tot_gold = defaultdict(float)
    for k in cards:
        denom = sum(gold_share[k].values())
        if denom == 0:
            continue
        for pos, v in gold_share[k].items():
            tot_gold[pos] += v / denom / len(cards)
            tot_pred[pos] += pred_share[k].get(pos, 0.0) / denom / len(cards)
    return {"name": name, "items": items, "skipped": skipped,
            "rows": rows, "tot_pred": dict(tot_pred), "tot_gold": dict(tot_gold)}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--ref", type=Path, required=True,
                    help="JevBench ref root (datasets/public/*.jsonl)")
    ap.add_argument("--arm", type=Path, action="append", required=True)
    ap.add_argument("--out", type=Path, default=None)
    args = ap.parse_args()

    tasks = load_tasks(args.ref)
    lines = [
        "# Position-prior study (post-hoc, no inference)", "",
        f"Tasks: {len(tasks)} public JevBench items. Shares are normalized"
        " within each cardinality (a k-choice item contributes 1/k to each"
        " position), then aggregated; `gold` is the benchmark's own"
        " answer-position distribution, so pred − gold is the arm's prior.", "",
    ]
    studies = [study(d.name, tasks, load_arm(d)) for d in args.arm]

    lines += ["## Aggregate position share (cardinality-weighted)", ""]
    lines += ["| arm | " + " | ".join(f"pos {p}" for p in range(8)) + " |",
              "|---" * 1 + "---|" + "---|" * 8]
    for s in studies:
        cells = []
        for p in range(8):
            g, pr = s["tot_gold"].get(p, 0.0), s["tot_pred"].get(p, 0.0)
            cells.append(f"{pr:.3f} vs {g:.3f}" if (g or pr) else "—")
        lines.append(f"| {s['name']} | " + " | ".join(cells) + " |")
    lines += ["", "Each cell: predicted share vs gold share at that position.", ""]

    for s in studies:
        lines += [f"## {s['name']}", "",
                  f"Items joined: {s['items']}; predictions outside the"
                  f" candidate list: {s['skipped']}.", "",
                  "Accuracy by gold position (primacy cost shows up as a",
                  "downward slope):", "",
                  "| k | pos | pred share | gold share | acc | n |",
                  "|---|---|---|---|---|---|"]
        for r in s["rows"]:
            acc = f"{r['acc']:.3f}" if r["acc"] is not None else "—"
            lines.append(f"| {r['k']} | {r['pos']} | {r['pred']:.3f} |"
                         f" {r['gold']:.3f} | {acc} | {r['n']} |")
        lines.append("")

    report = "\n".join(lines) + "\n"
    if args.out:
        args.out.write_text(report)
        print(f"wrote {args.out}")
    else:
        print(report)
    return 0


if __name__ == "__main__":
    sys.exit(main())
