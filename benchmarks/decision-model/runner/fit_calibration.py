#!/usr/bin/env python3
"""Fit D15 temperature-calibration artifacts from benchmark run data.

Offline tooling (PLANNING.md Rule 8, DECISIONS.md D15): raw winner
probabilities measured across the decision-model board are badly
calibrated (ECE 0.048-0.626), so exposing them as confidence is a lie.
This fitter fits one temperature per model from the suite's
(winner_probability, correct) observations and emits validated JSON
artifacts that `opencodifier-engine`'s `TemperatureCalibration` loads.

Method, and its one approximation. The runner records each item's winner
probability and correctness, not the full answer distribution, so the
fit is temperature scaling on the winner-vs-rest margin: an observation
(p, y) with y in {0, 1} is scored as

    q(p; T) = p^(1/T) / (p^(1/T) + (1-p)^(1/T))

and T minimizes the Bernoulli NLL -[y ln q + (1-y) ln(1-q)]. This is
exact temperature scaling when the non-winner mass is lumped into a
single competitor, which is the reading a policy gate uses (it reads
the top probability only). T is found by golden-section search on the
NLL, which is convex in log T; no third-party dependencies.

When the NLL falls monotonically all the way to its T->inf limit, the
best calibrated predictor is the constant 0.5 and no finite temperature
helps: the fit is reported as DEGENERATE and no artifact is emitted -
those scores are ordering evidence only, not confidence.

The engine keys temperatures by question class (`choice`); the suite is
all-Choice, so each artifact carries the global fit under "choice".
Per-suite-class fits are printed to stdout as analysis evidence (they
differ per class - that spread is what motivates per-task-class
artifacts once the IR carries class tags), but are not shipped as
engine keys the engine cannot address.

Artifacts are fitted and evaluated on the same 120 items (in-sample).
Held-out fitting needs a larger suite; the REPORT's threats-to-validity
section already carries that limitation (n=120, single seed) - do not
read the ece_after values as generalization claims.

Usage:
    python3 runner/fit_calibration.py --results-dir <runs dir> \
        --out-dir ../results/calibration
"""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path

# Board arm -> artifact file stem. The D16 tier scheme plus the two
# sub-LLM rungs; everything else on the board is analysis-only.
ARMS: dict[str, str] = {
    # The engine arm of record is the relational solver over the lexical
    # classifier (its live identity composes both ids); the bare lexical
    # stack it replaced no longer ships as the default.
    "engine__relational-v1.json": "relational-v1",
    "embed__gte-modernbert-onnx-fp32.json": "gte-modernbert-onnx-fp32",
    "llama__Qwen3.5-0.8B-q4_0.json": "qwen3.5-0.8b-q4_0",
    "llama__Qwen3.5-2B.json": "qwen3.5-2b-q4_k_m",
    "llama__Qwen3.5-4B-Q3_K_S.json": "qwen3.5-4b-q3_k_s",
    "llama__Qwen3.5-4B-UD-Q4_K_XL.json": "qwen3.5-4b-ud-q4_k_xl",
    "llama__MiMo-V2.6-Distill-Qwen-9B-Q3_K_S.json": "mimo-v2.6-9b-q3_k_s",
}


def observations(run: dict) -> list[tuple[float, int, str]]:
    """(winner_probability, correct, suite_class) per single-run item."""
    return [
        (float(item["prob"]), int(item["pred"] == item["answer"]), str(item["class"]))
        for item in run["single"]
    ]


def winner_calibrated(probability: float, temperature: float) -> float:
    """q(p; T): temperature scaling of the winner-vs-rest margin."""
    if probability <= 0.0:
        return 0.0
    if probability >= 1.0:
        return 1.0
    exponent = 1.0 / temperature
    winner = probability**exponent
    rest = (1.0 - probability) ** exponent
    return winner / (winner + rest)


def bernoulli_nll(observations: list[tuple[float, int]], temperature: float) -> float:
    """Mean -[y ln q + (1-y) ln(1-q)] at temperature T."""
    total = 0.0
    for probability, correct in observations:
        q = min(max(winner_calibrated(probability, temperature), 1e-12), 1.0 - 1e-12)
        total -= math.log(q if correct else 1.0 - q)
    return total / len(observations)


def fit_temperature(observations: list[tuple[float, int]]) -> tuple[float, bool]:
    """Golden-section search over log-temperature; NLL is convex in it.

    Returns (T, degenerate). A fit is degenerate when the optimum sits at
    the search ceiling: the NLL falls monotonically toward its T->inf
    limit ln(2), i.e. the uninformative 0.5 constant outperforms every
    finite temperature and the probabilities carry no usable confidence
    signal.
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
    """Standard 10-bin ECE over the calibrated winner probabilities."""
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


def build_artifact(name: str, run: dict, run_path: Path) -> dict:
    """Assemble a `CalibrationArtifact`-shaped dict for one arm."""
    items = observations(run)
    pairs = [(probability, correct) for probability, correct, _ in items]
    temperature, degenerate = fit_temperature(pairs)
    classes = sorted({item_class for _, _, item_class in items})
    per_class = {
        item_class: fit_temperature(
            [(p, y) for p, y, c in items if c == item_class]
        )[0]
        for item_class in classes
    }
    ece_before = raw_ece(pairs)
    ece_after = expected_calibration_error(pairs, temperature)
    model = run.get("model") or {}
    source = model.get("file") or model.get("name") or run_path.name
    return {
        "artifact": {
            "format_version": 1,
            "scheme": "temperature",
            "model_id": name,
            "calibration_version": 1,
            "default_temperature": round(temperature, 6),
            "temperatures": {"choice": round(temperature, 6)},
            "fit": {
                "items": len(pairs),
                "ece_before": round(ece_before, 6),
                "ece_after": round(ece_after, 6),
                "source": (
                    "benchmarks/decision-model suite (seed 20260926, 120 items, "
                    f"single-run arm {run_path.name}, model {source}); "
                    "winner-vs-rest margin fit, in-sample"
                ),
            },
        },
        "ece_before": ece_before,
        "ece_after": ece_after,
        "temperature": temperature,
        "degenerate": degenerate,
        "per_class": per_class,
    }


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Fit D15 temperature-calibration artifacts from run data."
    )
    parser.add_argument("--results-dir", type=Path, required=True, help="directory of run JSONs")
    parser.add_argument("--out-dir", type=Path, required=True, help="artifact output directory")
    args = parser.parse_args()

    args.out_dir.mkdir(parents=True, exist_ok=True)
    rows: list[tuple[str, float, float, float, float]] = []
    for run_name, artifact_name in ARMS.items():
        run_path = args.results_dir / run_name
        if not run_path.is_file():
            raise SystemExit(f"missing run JSON: {run_path}")
        run = json.loads(run_path.read_text())
        fitted = build_artifact(artifact_name, run, run_path)
        out_path = args.out_dir / f"{artifact_name}.json"
        # An artifact ships only if the fit does not worsen the headline
        # calibration diagnostic. NLL (the fit objective) improves on any
        # in-sample 1-parameter fit of enough items; ECE is the check that
        # the rescaling is actually the right shape. A bimodal
        # proof/delegate stack (hard 1.0s plus an underconfident lexical
        # tail) is exactly where a single global temperature is the wrong
        # tool, and its artifact must not ship.
        helps = fitted["ece_after"] <= fitted["ece_before"]
        if fitted["degenerate"] or not helps:
            out_path.unlink(missing_ok=True)
            if fitted["degenerate"]:
                # T -> inf wins: the constant 0.5 beats every finite
                # temperature. No artifact - an infinite temperature is
                # not a valid calibration (engine validation requires
                # T > 0 finite), and the honest reading is that this
                # rung's scores are ordering-only, not confidence.
                rows.append((artifact_name, float("inf"), fitted["ece_before"], float("nan")))
            else:
                rows.append((artifact_name, fitted["temperature"], fitted["ece_before"],
                             fitted["ece_after"]))
        else:
            out_path.write_text(json.dumps(fitted["artifact"], indent=2) + "\n")
            rows.append((artifact_name, fitted["temperature"], fitted["ece_before"], fitted["ece_after"]))
        per_class = ", ".join(
            f"{item_class}={value:.3f}" for item_class, value in fitted["per_class"].items()
        )
        print(f"{artifact_name}: T={fitted['temperature']:.3f} "
              f"ECE {fitted['ece_before']:.3f} -> {fitted['ece_after']:.3f} [{per_class}]"
              + ("  [DEGENERATE: T->inf, no artifact]" if fitted["degenerate"] else "")
              + ("  [SKIP: ECE worsens, no artifact]"
                 if not fitted["degenerate"] and not helps else ""))

    print()
    print("| artifact | T | ECE before | ECE after |")
    print("|---|---|---|---|")
    for name, temperature, before, after in rows:
        shown_t = "inf" if math.isinf(temperature) else f"{temperature:.3f}"
        shown_after = "-" if math.isnan(after) else f"{after:.3f}"
        print(f"| {name} | {shown_t} | {before:.3f} | {shown_after} |")


if __name__ == "__main__":
    main()
