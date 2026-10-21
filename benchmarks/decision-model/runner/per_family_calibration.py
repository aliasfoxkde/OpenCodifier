#!/usr/bin/env python3
"""Per-family calibration analysis for the E1-E calibration arc.

Groups JevBench results by family field, fits temperature per family and
globally, and reports which families benefit from per-family calibration.
The output feeds the #186 per-family ladder routing decision.

Usage:
    python3 runner/per_family_calibration.py \
        --results /nas/Temp/work/oc-model-eval/runs/jevbench/e1e-slot-231/results.jsonl \
        --out-dir results/calibration
"""

from __future__ import annotations

import argparse
import json
import math
import sys
from collections import defaultdict
from pathlib import Path


def out(msg: str = "") -> None:
    sys.stdout.write(msg + "\n")


def winner_calibrated(probability: float, temperature: float) -> float:
    if probability <= 0.0:
        return 0.0
    if probability >= 1.0:
        return 1.0
    exponent = 1.0 / temperature
    winner = probability**exponent
    rest = (1.0 - probability) ** exponent
    return winner / (winner + rest)


def bernoulli_nll(observations: list[tuple[float, int]], temperature: float) -> float:
    total = 0.0
    for probability, correct in observations:
        q = min(max(winner_calibrated(probability, temperature), 1e-12), 1.0 - 1e-12)
        total -= math.log(q if correct else 1.0 - q)
    return total / len(observations)


def fit_temperature(observations: list[tuple[float, int]]) -> tuple[float, bool]:
    """Golden-section search; returns (T, degenerate)."""
    lo, hi = math.log(0.05), math.log(200.0)
    ceiling = hi
    gratio = (math.sqrt(5.0) - 1.0) / 2.0
    left = hi - gratio * (hi - lo)
    right = lo + gratio * (hi - lo)
    f_left = bernoulli_nll(observations, math.exp(left))
    f_right = bernoulli_nll(observations, math.exp(right))
    for _ in range(200):
        if f_left < f_right:
            hi, right, f_right = right, left, f_left
            left = hi - gratio * (hi - lo)
            f_left = bernoulli_nll(observations, math.exp(left))
        else:
            lo, left, f_left = left, right, f_right
            right = lo + gratio * (hi - lo)
            f_right = bernoulli_nll(observations, math.exp(right))
    temperature = math.exp((lo + hi) / 2.0)
    return temperature, temperature >= math.exp(ceiling) * 0.99


def expected_calibration_error(
    observations: list[tuple[float, int]], temperature: float, bins: int = 10
) -> float:
    total = 0.0
    for index in range(bins):
        lo, hi = index / bins, (index + 1) / bins
        bucket = [
            (probability, correct)
            for probability, correct in observations
            if lo < probability <= hi or (index == 0 and probability == 0.0)
        ]
        if not bucket:
            continue
        accuracy = sum(correct for _, correct in bucket) / len(bucket)
        confidence = sum(winner_calibrated(p, temperature) for p, _ in bucket) / len(bucket)
        total += len(bucket) / len(observations) * abs(accuracy - confidence)
    return total


def load_results(path: Path) -> dict[str, list[tuple[float, int]]]:
    """Load results.jsonl, group by family field."""
    by_family: dict[str, list[tuple[float, int]]] = defaultdict(list)
    text = path.read_text(encoding="utf-8")
    for line in text.splitlines():
        line = line.strip()
        if not line:
            continue
        row = json.loads(line)
        if "correct" not in row:
            continue
        prob = float(row.get("prob", row.get("probability", 0.0)))
        correct = int(bool(row["correct"]))
        family = row.get("family", "unknown")
        by_family[family].append((prob, correct))
    return by_family


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Per-family calibration analysis: which families benefit from per-family T?"
    )
    parser.add_argument("--results", type=Path, required=True,
                        help="JevBench results.jsonl")
    parser.add_argument("--out-dir", type=Path, default=Path("results/calibration"),
                        help="Where to write the report JSON")
    args = parser.parse_args()

    by_family = load_results(args.results)
    all_obs: list[tuple[float, int]] = []
    for obs in by_family.values():
        all_obs.extend(obs)

    out(f"Loaded {len(all_obs)} observations across {len(by_family)} families")

    # Global T
    global_T, global_degenerate = fit_temperature(all_obs)
    global_ece = expected_calibration_error(all_obs, global_T)
    global_raw_ece = expected_calibration_error(all_obs, 1.0)

    out(f"\nGlobal: T={global_T:.4f}, ECE={global_ece:.6f} (raw={global_raw_ece:.6f})")

    # Per-family
    rows = []
    for family in sorted(by_family.keys()):
        obs = by_family[family]
        n = len(obs)
        family_T, fam_deg = fit_temperature(obs)
        fam_ece = expected_calibration_error(obs, family_T)
        global_T_on_fam = expected_calibration_error(obs, global_T)
        delta = fam_ece - global_T_on_fam  # negative = per-family is better
        rows.append({
            "family": family,
            "n": n,
            "T": round(family_T, 4),
            "degenerate": fam_deg,
            "ECE_per_family": round(fam_ece, 6),
            "ECE_global_T": round(global_T_on_fam, 6),
            "delta": round(delta, 6),
            "verdict": "per-family better" if delta < -0.01 else ("global better" if delta > 0.01 else "same"),
        })
        out(f"  {family:20s} n={n:3d}  T={family_T:6.3f}  "
            f"ECE_family={fam_ece:.3f}  ECE_global={global_T_on_fam:.3f}  "
            f"delta={delta:+.3f}  {'per-fam' if delta < -0.01 else 'same' if abs(delta) <= 0.01 else 'global'}")

    # Summary
    per_fam_better = sum(1 for r in rows if r["verdict"] == "per-family better")
    global_better = sum(1 for r in rows if r["verdict"] == "global better")
    out(f"\nSummary: {per_fam_better} families per-family-better, "
        f"{global_better} global-better, {len(rows) - per_fam_better - global_better} same")

    # Write report
    args.out_dir.mkdir(parents=True, exist_ok=True)
    report = {
        "global": {
            "T": round(global_T, 6),
            "degenerate": global_degenerate,
            "ECE": round(global_ece, 6),
            "raw_ECE": round(global_raw_ece, 6),
        },
        "families": rows,
        "summary": {
            "per_family_better": per_fam_better,
            "global_better": global_better,
            "same": len(rows) - per_fam_better - global_better,
        },
    }
    out_path = args.out_dir / "per-family-calibration.json"
    out_path.write_text(json.dumps(report, indent=2) + "\n")
    out(f"\nReport: {out_path}")


if __name__ == "__main__":
    main()
