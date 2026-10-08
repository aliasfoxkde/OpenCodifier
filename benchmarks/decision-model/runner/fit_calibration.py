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

Exact fit path (d15 fork builds). Runs whose items carry `probs` —
the fork's full per-choice softmax — drop the approximation: with the
true label's index known, the categorical observation is the whole
vector, and reverse-softmax temperature scaling

    q_i(T) = p_i^(1/T) / sum_j p_j^(1/T)

is EXACT temperature scaling (p came from softmax, so p^(1/T)
renormalized is softmax(z/T) up to a constant that cancels). T is fit
by golden-section search over beta = 1/T, where the categorical NLL
-sum ln q_label is convex; the ECE gate keeps reading the winner
probability through the same winner-vs-rest formula as before, because
that is what a policy gate actually consumes. An arm uses the exact
path only when EVERY item carries `probs`; a partial board falls back
to the margin fit for that arm rather than mixing objectives.

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
    # with a run_jevbench.py arm dir in ARMS, also pass the dataset:
    python3 runner/fit_calibration.py --results-dir <runs dir> \
        --out-dir ../results/calibration \
        --tasks "$REF/datasets/public/easy.jsonl,$REF/datasets/public/hard.jsonl,$REF/datasets/public/original.jsonl"
"""

from __future__ import annotations

import argparse
import json
import math
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "suite"))
from generate_suite import build_suite  # noqa: E402

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
    "llama__jebadiah-4b-v2-Q8_0.json": "jebadiah-4b-v2-q8_0",
    "llama__jebadiah-9b-v2-Q8_0.json": "jebadiah-9b-v2-q8_0",
    # run_jevbench.py arm dir (results.jsonl rows + --tasks label join).
    "jevbench/fork_4b-d15-v1": "fork_4b-d15-v1",
    # Stock letters-lane operating points (run_stock.py passes-schema;
    # the loader below reads the identity pass). The official-weights
    # Q4_0 is the small-model operating point as of r15; the unsloth
    # UD-Q4_K_XL QAT arm is kept as its direct predecessor. The r16
    # arms bracket it: the E4B-QAT native-format rescue (partial) and
    # the matched-format non-QAT control (+45.8 pp QAT effect). The
    # r17 arms extend the matched-format A/B to a second family
    # (Qwen3.5, YoozLabs third-party QAT). clef-flash-q4_k_m is
    # deliberately absent: 115/120 invalid distributions leave 5
    # valid rows — a temperature fit on those would be noise, so no
    # artifact ships for the emission-broken backbone arm.
    "stock__e2bqat-official-q4_0-letters.json": "gemma-4-e2b-qat-q4_0",
    "stock__gemma-4-e2b-qat-letters.json": "gemma-4-e2b-qat-ud-q4_k_xl",
    "stock__e4bqat-official-q4_0-letters.json": "gemma-4-e4b-qat-q4_0",
    "stock__e2b-noqat-q4_0-letters.json": "gemma-4-e2b-noqat-q4_0",
    "stock__yoozlabs-qwen35-08b-qat-q4_0-letters.json": "qwen3.5-0.8b-qat-q4_0",
    "stock__yoozlabs-qwen35-4b-qat-q4_0-letters.json": "qwen3.5-4b-qat-q4_0",
    "stock__qwen35-4b-noqat-q4_0-letters.json": "qwen3.5-4b-noqat-q4_0",
}


def jevbench_items(arm_dir: Path, tasks: str) -> list[dict]:
    """Load a run_jevbench.py arm dir into the fitter's single-item schema.

    The d15 fork rows carry the full per-choice softmax (`probs`) but not
    the ground-truth label, so labels are joined by task id from the
    harness dataset — the same pinned jsonl set run_jevbench.py consumed.
    `expected` must be one of the row's `probs` keys for every valid row:
    that is what lets the exact d15 fit path engage; the winner-vs-rest
    fields (prob/pred/answer) stay the margin-fit observations either way.
    Score-class items carry integer ordinal levels while the fork's
    readout labels them as strings ("0".."3"), so the label is normalized
    into the probs key space — without this, every score row reads as
    wrong and both fit paths are fitted on corrupted labels.
    """
    by_id: dict[str, str] = {}
    for part in tasks.split(","):
        for line in Path(part.strip()).read_text().splitlines():
            if line.strip():
                row = json.loads(line)
                by_id[row["id"]] = row["expected"]
    items: list[dict] = []
    for line in (arm_dir / "results.jsonl").read_text().splitlines():
        if not line.strip():
            continue
        row = json.loads(line)
        if not (row.get("ok") and row.get("valid")):
            continue
        answer = by_id.get(row["task_id"])
        probs = row.get("probs")
        if answer is None or not isinstance(probs, dict) or not probs:
            continue
        answer = str(answer)
        items.append({
            "id": row["task_id"],
            "prob": max(float(v) for v in probs.values()),
            "pred": str(row.get("predicted")),
            "answer": answer,
            "class": row.get("family"),
            "probs": probs,
        })
    if not items:
        raise SystemExit(f"no valid joined items in {arm_dir}")
    return items


def observations(run: dict) -> list[tuple[float, int, str]]:
    """(winner_probability, correct, suite_class) per single-run item."""
    return [
        (float(item["prob"]), int(item["pred"] == item["answer"]), str(item["class"]))
        for item in run["single"]
    ]


def full_observations(run: dict) -> list[tuple[list[float], int]] | None:
    """Exact-fit observations: (prob vector, true-label index) per item.

    Returns None unless EVERY single-run item carries the d15 fork's
    `probs` (full per-choice softmax keyed by candidate id) and a known
    `answer` id inside it — a partially covered arm falls back to the
    winner-vs-rest margin fit rather than mixing objectives.
    """
    out: list[tuple[list[float], int]] = []
    for item in run["single"]:
        probs = item.get("probs")
        answer = item.get("answer")
        if not isinstance(probs, dict) or not probs or answer not in probs:
            return None
        vec = [max(float(probs[key]), 1e-12) for key in probs]
        total = sum(vec)
        out.append(([v / total for v in vec], list(probs).index(answer)))
    return out


def categorical_nll(
    dist_observations: list[tuple[list[float], int]], beta: float
) -> float:
    """Mean -ln q_true over the board at inverse temperature beta."""
    total = 0.0
    for vec, label in dist_observations:
        scaled = [v**beta for v in vec]
        z = sum(scaled)
        q = min(max(scaled[label] / z, 1e-12), 1.0 - 1e-12)
        total -= math.log(q)
    return total / len(dist_observations)


def fit_temperature_full(
    dist_observations: list[tuple[list[float], int]],
) -> tuple[float, bool]:
    """Golden-section search over beta = 1/T; the categorical NLL is
    convex in beta (log-sum-exp of linear functions), hence unimodal.

    Returns (T, degenerate). Degenerate is the beta -> 0 ceiling: the NLL
    falls monotonically toward the uniform limit (every finite temperature
    loses to the uninformative uniform distribution), so no finite
    temperature calibrates these scores.
    """
    lo, hi = 1.0 / 200.0, 20.0
    floor = lo
    if abs(
        categorical_nll(dist_observations, hi) - categorical_nll(dist_observations, lo)
    ) < 1e-9:
        # NLL flat across the whole bracket: the board carries no
        # temperature-sensitive signal (perfectly uniform scores are the
        # extreme case) — the same honest verdict as the margin fit's
        # T->inf ceiling. Without this, golden-section's tie-breaking
        # silently walks to the bracket edge and ships a fake T.
        return 1.0, True
    gratio = (math.sqrt(5.0) - 1.0) / 2.0
    left = hi - gratio * (hi - lo)
    right = lo + gratio * (hi - lo)
    f_left = categorical_nll(dist_observations, left)
    f_right = categorical_nll(dist_observations, right)
    for _ in range(200):
        if f_left < f_right:
            hi, right, f_right = right, left, f_left
            left = hi - gratio * (hi - lo)
            f_left = categorical_nll(dist_observations, left)
        else:
            lo, left, f_left = left, right, f_right
            right = lo + gratio * (hi - lo)
            f_right = categorical_nll(dist_observations, right)
    beta = (lo + hi) / 2.0
    return 1.0 / beta, beta <= floor * 1.01


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


def aurc(observations: list[tuple[float, int]], temperature: float) -> float:
    """Area under the risk-coverage curve (RESEARCH.md §15.6 item 1).

    Order items by calibrated confidence descending; the prefix risk at
    coverage k is the error rate of the top-k; AURC is the mean prefix
    risk over k = 1..n. Lower is better, and a perfect ranker reaches
    0.0. This is the selective-prediction metric a policy gate's
    abstention actually experiences - it is known to diverge from ECE
    (a fit can improve calibration while worsening ranking), so the
    ship gate reads both.

    Expected reading for this fitter: temperature scaling is monotone
    in the winner probability, so it reorders nothing and aurc_after ==
    aurc_before is the CORRECT result, not a dead metric - the values
    are the selective-prediction evidence of record, and the ship gate
    becomes load-bearing the moment a non-monotone scheme (per-class
    isotonic with flat regions, a verifier blend) writes artifacts.
    """
    scored = sorted(
        ((winner_calibrated(p, temperature), 1 - y) for p, y in observations),
        key=lambda pair: -pair[0],
    )
    running = 0.0
    total = 0.0
    for k, (_, error) in enumerate(scored, 1):
        running += error
        total += running / k
    return total / len(scored)


def accuracy_at_coverage(
    observations: list[tuple[float, int]], temperature: float, coverage: float
) -> float:
    """Acceptance accuracy when the top `coverage` fraction is accepted."""
    k = max(1, round(len(observations) * coverage))
    top = sorted(
        observations, key=lambda pair: -winner_calibrated(pair[0], temperature)
    )[:k]
    return sum(correct for _, correct in top) / k


def build_artifact(
    name: str, run: dict, run_path: Path,
    source_suite: str = "benchmarks/decision-model suite (seed 20260926, 120 items)",
) -> dict:
    """Assemble a `CalibrationArtifact`-shaped dict for one arm.

    `source_suite` names the observation set in the shipped provenance
    string — run-JSON arms come from the internal 120-item suite, while
    run_jevbench.py arm dirs are fitted on the external public split and
    must not inherit the internal description.
    """
    items = observations(run)
    pairs = [(probability, correct) for probability, correct, _ in items]
    dists = full_observations(run)
    if dists is not None:
        temperature, degenerate = fit_temperature_full(dists)
        method = "exact full-distribution reverse-softmax fit (d15), in-sample"
    else:
        temperature, degenerate = fit_temperature(pairs)
        method = "winner-vs-rest margin fit, in-sample"
    classes = sorted({item_class for _, _, item_class in items})
    per_class = {
        item_class: fit_temperature(
            [(p, y) for p, y, c in items if c == item_class]
        )[0]
        for item_class in classes
    }
    # Per-cardinality bucket fits (RESEARCH.md §7.3 delta 3): candidate
    # count is a prompt-shape variable that shifts distribution spread, so
    # one temperature may not serve all option counts. Suite cardinalities
    # are joined by item id from the seed-deterministic generator. Analysis
    # evidence only — the shipped artifact keys stay what the engine can
    # address, the same discipline as per_class.
    card_of = {it["id"]: len(it["candidates"]) for it in build_suite()["items"]}
    per_card: dict[str, dict] = {}
    if dists is not None:
        card_by_index = [
            card_of.get(item.get("id")) for item in run["single"]
        ]
        for k in sorted({c for c in card_by_index if c is not None}):
            sub = [obs for obs, c in zip(dists, card_by_index) if c == k]
            if len(sub) < 8:
                continue  # a bucket too small to fit honestly is not a bucket
            t, degen = fit_temperature_full(sub)
            per_card[str(k)] = {"T": t, "n": len(sub), "degenerate": degen}
    else:
        by_card: dict[int, list[tuple[float, int]]] = {}
        for it in run["single"]:
            k = card_of.get(it["id"])
            if k is not None:
                by_card.setdefault(k, []).append(
                    (float(it["prob"]), int(it["pred"] == it["answer"]))
                )
        for k in sorted(by_card):
            if len(by_card[k]) < 8:
                continue
            t, degen = fit_temperature(by_card[k])
            sub_ece = expected_calibration_error(by_card[k], t)
            per_card[str(k)] = {"T": t, "n": len(by_card[k]),
                                "degenerate": degen, "ece_after": sub_ece,
                                "ece_before": raw_ece(by_card[k])}
    ece_before = raw_ece(pairs)
    ece_after = expected_calibration_error(pairs, temperature)
    aurc_before = aurc(pairs, 1.0)
    aurc_after = aurc(pairs, temperature)
    coverage_accuracy = {
        str(int(fraction * 100)): accuracy_at_coverage(pairs, temperature, fraction)
        for fraction in (0.5, 0.8)
    }
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
                "aurc_before": round(aurc_before, 6),
                "aurc_after": round(aurc_after, 6),
                "accuracy_at_coverage": {
                    key: round(value, 6) for key, value in coverage_accuracy.items()
                },
                "source": (
                    f"{source_suite}, "
                    f"single-run arm {run_path.name}, model {source}; {method}"
                ),
            },
        },
        "ece_before": ece_before,
        "ece_after": ece_after,
        "aurc_before": aurc_before,
        "aurc_after": aurc_after,
        "temperature": temperature,
        "degenerate": degenerate,
        "per_class": per_class,
        "per_cardinality": per_card,
    }


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Fit D15 temperature-calibration artifacts from run data."
    )
    parser.add_argument("--results-dir", type=Path, required=True, help="directory of run JSONs")
    parser.add_argument("--out-dir", type=Path, required=True, help="artifact output directory")
    parser.add_argument("--tasks", type=str, default=None,
                        help="comma-separated harness dataset jsonl files "
                             "(needed for run_jevbench.py arm dirs)")
    args = parser.parse_args()

    args.out_dir.mkdir(parents=True, exist_ok=True)
    rows: list[tuple[str, float, float, float, float]] = []
    for run_name, artifact_name in ARMS.items():
        run_path = args.results_dir / run_name
        source_suite = "benchmarks/decision-model suite (seed 20260926, 120 items)"
        if run_path.is_dir():
            if not args.tasks:
                raise SystemExit(f"--tasks required for arm dir: {run_path}")
            run = {"single": jevbench_items(run_path, args.tasks)}
            source_suite = ("JevBench public split (231 items, fstandhartinger/"
                            "jevbench MIT, labels joined from the pinned dataset)")
        elif run_path.is_file():
            run = json.loads(run_path.read_text())
            if "single" not in run and "passes" in run:
                # run_stock.py lane: the identity pass ("0") is the
                # single-run of record — permutations and the determinism
                # replay are sibling keys, and rows without a prediction
                # are excluded exactly like metrics() excludes them.
                run = {
                    **run,
                    "single": [r for r in run["passes"]["0"] if r["pred"] is not None],
                }
        else:
            # Skip rather than abort: arms are fitted from whatever runs
            # are present, and a missing input must not take down the
            # refit of an arm that did land. The skip is printed, not
            # silent, and no artifact is touched for it.
            print(f"{artifact_name}: SKIPPED (missing run JSON/dir: {run_path})")
            continue
        fitted = build_artifact(artifact_name, run, run_path, source_suite)
        out_path = args.out_dir / f"{artifact_name}.json"
        # An artifact ships only if the fit does not worsen the headline
        # diagnostics. NLL (the fit objective) improves on any in-sample
        # 1-parameter fit of enough items; ECE is the check that the
        # rescaling is actually the right shape; AURC is the check that
        # selective prediction did not pay for it (RESEARCH.md §15.1:
        # the two diverge, so neither alone gates). A bimodal
        # proof/delegate stack (hard 1.0s plus an underconfident lexical
        # tail) is exactly where a single global temperature is the wrong
        # tool, and its artifact must not ship.
        ece_helps = fitted["ece_after"] <= fitted["ece_before"]
        # 1e-9: the same rounding slack the engine's load gate allows.
        aurc_helps = fitted["aurc_after"] <= fitted["aurc_before"] + 1e-9
        helps = ece_helps and aurc_helps
        if fitted["degenerate"] or not helps:
            out_path.unlink(missing_ok=True)
            if fitted["degenerate"]:
                # T -> inf wins: the constant 0.5 beats every finite
                # temperature. No artifact - an infinite temperature is
                # not a valid calibration (engine validation requires
                # T > 0 finite), and the honest reading is that this
                # rung's scores are ordering-only, not confidence.
                rows.append((artifact_name, float("inf"), fitted["ece_before"], float("nan"),
                             fitted["aurc_before"], float("nan")))
            else:
                rows.append((artifact_name, fitted["temperature"], fitted["ece_before"],
                             fitted["ece_after"], fitted["aurc_before"],
                             fitted["aurc_after"]))
        else:
            out_path.write_text(json.dumps(fitted["artifact"], indent=2) + "\n")
            rows.append((artifact_name, fitted["temperature"], fitted["ece_before"],
                         fitted["ece_after"], fitted["aurc_before"], fitted["aurc_after"]))
        per_class = ", ".join(
            f"{item_class}={value:.3f}" for item_class, value in fitted["per_class"].items()
        )
        per_card = ", ".join(
            f"k{key}={bucket['T']:.3f}(n={bucket['n']})"
            + ("[degenerate]" if bucket["degenerate"] else "")
            for key, bucket in fitted["per_cardinality"].items()
        )
        skip_reason = ""
        if not fitted["degenerate"] and not helps:
            skip_reason = ("  [SKIP: AURC worsens, no artifact]"
                           if ece_helps and not aurc_helps
                           else "  [SKIP: ECE worsens, no artifact]")
        print(f"{artifact_name}: T={fitted['temperature']:.3f} "
              f"ECE {fitted['ece_before']:.3f} -> {fitted['ece_after']:.3f} "
              f"AURC {fitted['aurc_before']:.3f} -> {fitted['aurc_after']:.3f} [{per_class}]"
              + (f" | per-cardinality: {per_card}" if per_card else "")
              + ("  [DEGENERATE: T->inf, no artifact]" if fitted["degenerate"] else "")
              + skip_reason)

    print()
    print("| artifact | T | ECE before | ECE after | AURC before | AURC after |")
    print("|---|---|---|---|---|---|")
    for name, temperature, before, after, aurc_before_v, aurc_after_v in rows:
        shown_t = "inf" if math.isinf(temperature) else f"{temperature:.3f}"
        shown_after = "-" if math.isnan(after) else f"{after:.3f}"
        shown_aurc_after = "-" if math.isnan(aurc_after_v) else f"{aurc_after_v:.3f}"
        print(f"| {name} | {shown_t} | {before:.3f} | {shown_after} | "
              f"{aurc_before_v:.3f} | {shown_aurc_after} |")


if __name__ == "__main__":
    main()
