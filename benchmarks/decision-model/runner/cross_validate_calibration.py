#!/usr/bin/env python3
"""Cross-slice calibration transfer test.

Fits temperature on one observation set (fit set) and evaluates it on
another (eval set). Reports whether the temperature transfers — the
central question for the E1-E calibration arc (ECE 0.2067 → JEV's 0.08).

Cross-slice calibration is a calibration DEFECT test, not a model test:
if the temperature fitted on surface A does not transfer to surface B,
the artifact must not be used on B without a separate fit. This is the
NCM Table 7 discipline applied to our ECE problem.

Usage:
    # Fit on E1-E holdout eval rows, evaluate on JevBench 231:
    python3 runner/cross_validate_calibration.py \
        --e1-dir /path/to/e1e-holdout-eval/ \
        --fit-provenance "E1-E holdout eval" \
        --eval-suite /nas/Temp/work/oc-model-eval/runs/jevbench/e1e-slot-231/results.jsonl \
        --eval-provenance "JevBench public 231" \
        --out-dir results/calibration

    # Fit on JevBench 231 (in-sample), report the transfer gap:
    python3 runner/cross_validate_calibration.py \
        --jevbench-dir /nas/Temp/work/oc-model-eval/runs/jevbench/fork_4b-d15-v1/ \
        --fit-provenance "JevBench public 231 (in-sample)" \
        --eval-suite /nas/Temp/work/oc-model-eval/runs/jevbench/fork_4b-d15-v1/results.jsonl \
        --eval-provenance "JevBench public 231 (same set)" \
        --out-dir results/calibration
"""

from __future__ import annotations

import argparse
import json
import math
import sys
from pathlib import Path


def out(msg: str = "") -> None:
    sys.stdout.write(msg + "\n")


# ---------------------------------------------------------------------------
# Core math (same as fit_calibration.py — standalone at CLI)


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
    """Golden-section search over log-temperature; NLL is convex in it.

    Returns (T, degenerate). Degenerate when optimum is at the ceiling —
    constant 0.5 outperforms every finite temperature.
    """
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


def raw_ece(observations: list[tuple[float, int]]) -> float:
    return expected_calibration_error(observations, temperature=1.0)


def load_results_jsonl(path: Path) -> list[tuple[float, int]]:
    rows = []
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
        rows.append((prob, correct))
    return rows


def load_e1_rows(path: Path) -> list[tuple[float, int]]:
    rows = []
    text = path.read_text(encoding="utf-8")
    for line in text.splitlines():
        line = line.strip()
        if not line:
            continue
        row = json.loads(line)
        if "correct" not in row:
            continue
        prob = float(row.get("p", row.get("prob", 0.0)))
        correct = int(bool(row["correct"]))
        rows.append((prob, correct))
    return rows


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Cross-slice calibration transfer test: fit on one surface, evaluate on another."
    )
    parser.add_argument("--fit-suite", type=Path, help="results.jsonl path for the fit set")
    parser.add_argument("--fit-provenance", type=str, default="unknown",
                        help="Human-readable description of the fit set")
    parser.add_argument("--eval-suite", type=Path, help="results.jsonl path for the eval set")
    parser.add_argument("--eval-provenance", type=str, default="unknown",
                        help="Human-readable description of the eval set")
    parser.add_argument("--e1-dir", type=Path,
                        help="e1_eval.py rows.jsonl for the fit set (overrides --fit-suite)")
    parser.add_argument("--jevbench-dir", type=Path,
                        help="jevbench arm dir with results.jsonl for both fit and eval (in-sample)")
    parser.add_argument("--out-dir", type=Path, default=Path("results/calibration"),
                        help="Where to write the transfer report JSON")
    parser.add_argument("--name", type=str, default="cross-slice",
                        help="Artifact name stem")
    args = parser.parse_args()

    # Load fit observations
    if args.e1_dir:
        fit_path = Path(args.e1_dir) / "rows.jsonl"
        fit_obs = load_e1_rows(fit_path)
        fit_provenance = args.fit_provenance
    elif args.jevbench_dir:
        fit_path = Path(args.jevbench_dir) / "results.jsonl"
        fit_obs = load_results_jsonl(fit_path)
        fit_provenance = args.fit_provenance
    elif args.fit_suite:
        fit_obs = load_results_jsonl(args.fit_suite)
        fit_provenance = args.fit_provenance
    else:
        raise SystemExit("--fit-suite, --e1-dir, or --jevbench-dir is required")

    # Load eval observations
    if args.jevbench_dir:
        eval_path = Path(args.jevbench_dir) / "results.jsonl"
        eval_obs = load_results_jsonl(eval_path)
        eval_provenance = args.eval_provenance
    elif args.eval_suite:
        eval_obs = load_results_jsonl(args.eval_suite)
        eval_provenance = args.eval_provenance
    else:
        raise SystemExit("--eval-suite or --jevbench-dir is required")

    out(f"Fit set:    {fit_provenance} ({len(fit_obs)} observations)")
    out(f"Eval set:   {eval_provenance} ({len(eval_obs)} observations)")

    # Baselines
    eval_raw_ece = raw_ece(eval_obs)
    fit_raw_ece = raw_ece(fit_obs)
    out(f"Eval raw ECE:          {eval_raw_ece:.6f}")
    out(f"Fit raw ECE:          {fit_raw_ece:.6f}")

    # Fit temperature on fit set
    temperature, degenerate = fit_temperature(fit_obs)
    fit_ece = expected_calibration_error(fit_obs, temperature)
    out(f"Fitted T:            {temperature:.4f}  (degenerate={degenerate})")
    out(f"Fit ECE after T:      {fit_ece:.6f}")

    # Transfer: apply fitted T to eval set
    eval_calibrated_ece = expected_calibration_error(eval_obs, temperature)
    out(f"Eval ECE with T:     {eval_calibrated_ece:.6f}")

    # Transfer gap
    gap = eval_calibrated_ece - eval_raw_ece
    if gap < -1e-6:
        delta_label = f"BETTER ({abs(gap)*100:+.2f} pp)"
    elif gap > 1e-6:
        delta_label = f"WORSE ({gap*100:+.2f} pp)"
    else:
        delta_label = "SAME"
    out(f"Transfer delta:       {gap:+.6f}  ({delta_label})")

    # In-sample comparison
    eval_T, eval_degenerate = fit_temperature(eval_obs)
    eval_T_ece = expected_calibration_error(eval_obs, eval_T)
    out(f"Eval in-sample T:    {eval_T:.4f}  (degenerate={eval_degenerate})")
    out(f"Eval ECE after T:     {eval_T_ece:.6f}")
    out(f"T delta (eval-fit):  {eval_T - temperature:+.4f}")

    # Decision
    if degenerate:
        verdict = ("DEGENERATE — no finite temperature helps on fit set; "
                   "scores are ordering-only")
    elif gap < -0.01:
        verdict = f"TRANSFER — temperature transfers ({abs(gap)*100:.2f} pp improvement)"
    elif gap > 0.01:
        verdict = f"NO TRANSFER — temperature fitted on fit set worsens eval ({gap*100:.2f} pp)"
    else:
        verdict = "NEUTRAL — temperature has negligible effect on eval set (|delta| < 1 pp)"

    out(f"\nVerdict: {verdict}")

    # Write report
    args.out_dir.mkdir(parents=True, exist_ok=True)
    report = {
        "fit_set": {"provenance": fit_provenance, "n": len(fit_obs), "raw_ece": round(fit_raw_ece, 6)},
        "eval_set": {
            "provenance": eval_provenance,
            "n": len(eval_obs),
            "raw_ece": round(eval_raw_ece, 6),
            "calibrated_ece_with_fit_T": round(eval_calibrated_ece, 6),
        },
        "temperature": round(temperature, 6),
        "degenerate": degenerate,
        "transfer_delta": round(gap, 6),
        "verdict": verdict,
        "eval_in_sample": {"T": round(eval_T, 6), "degenerate": eval_degenerate,
                           "ECE": round(eval_T_ece, 6)},
    }
    out_path = args.out_dir / f"{args.name}-transfer.json"
    out_path.write_text(json.dumps(report, indent=2) + "\n")
    out(f"Report: {out_path}")


if __name__ == "__main__":
    main()
