#!/usr/bin/env python3
"""Per-item comparison of JevBench run dirs against a baseline arm.

The harness's determinism and A/B claims are per-item claims — tree mode
is deterministic, so two runs of one model that differ only in the wrapper
instruction (template A/B) or the serving binary (the d15 rebuild) must
agree on every task id or the delta is the finding. This tool makes that
check reproducible: it joins arms on `task_id`, reports agreement, the
accuracy delta, where the deltas landed (per family), and whether each
flipped item was fixed or broken. An arm that also recorded a
calibrated-probability channel (the d15 fork's additive distribution,
D15) additionally gets ECE and winner-Brier over the joined set.

Two determinism surfaces are checked per arm:
- internal: `results-replay.jsonl` (when present) must repeat
  `results.jsonl` prediction-for-prediction;
- external: the arm's predictions against the baseline's.

Labels never appear in JevBench rows (the harness records `correct`
only), so calibration is scored on the winner-probability channel: ECE
over (winner prob, correct) pairs, and Brier in winner form
`(p_winner - correct)^2`. Full multiclass Brier would need labels and is
deliberately not approximated here.

Usage:
  python3 runner/compare_arms.py --baseline <run_dir> \
      --arm <run_dir> [--arm <run_dir> ...] [--out report.md]
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from run_laya import ece  # noqa: E402


def load_rows(dir_path: Path, filename: str = "results.jsonl") -> dict[str, dict]:
    rows = {}
    for line in (dir_path / filename).read_text().splitlines():
        if not line.strip():
            continue
        r = json.loads(line)
        probs = r.get("probs") or {}
        pred = r.get("predicted")
        rows[r["task_id"]] = {
            "pred": pred,
            "correct": bool(r.get("correct")),
            "probs": {k: float(v) for k, v in probs.items()} if probs else None,
            "probs_source": r.get("probs_source"),
            "family": r.get("family"),
            "ms": (r.get("latency_s") or 0.0) * 1000.0,
        }
    return rows


def replay_ok(dir_path: Path, rows: dict[str, dict]) -> bool:
    """`results-replay.jsonl` must repeat `results.jsonl` predictions."""
    replay = load_rows(dir_path, "results-replay.jsonl")
    return set(replay) == set(rows) and all(
        replay[i]["pred"] == r["pred"] for i, r in rows.items()
    )


def winner_stats(rows: dict[str, dict]) -> tuple[float | None, float | None]:
    """ECE and winner-form Brier over the rows that carry probabilities."""
    pairs = []
    briers = []
    for r in rows.values():
        if not r["probs"] or r["pred"] not in r["probs"]:
            continue
        p = r["probs"][r["pred"]]
        pairs.append((p, r["correct"]))
        briers.append((p - float(r["correct"])) ** 2)
    if not pairs:
        return None, None
    return ece(pairs), sum(briers) / len(briers)


def percentile(values: list[float], q: float) -> float:
    s = sorted(values)
    return s[min(len(s) - 1, int(q * len(s)))] if s else 0.0


def compare(name: str, base: dict[str, dict], arm: dict[str, dict]) -> dict:
    ids = sorted(set(base) & set(arm))
    n = len(ids)
    agree = [i for i in ids if base[i]["pred"] == arm[i]["pred"]]
    fixed = [i for i in ids if base[i]["pred"] != arm[i]["pred"]
             and arm[i]["correct"] and not base[i]["correct"]]
    broken = [i for i in ids if base[i]["pred"] != arm[i]["pred"]
              and base[i]["correct"] and not arm[i]["correct"]]
    # Predictions differ ⇒ at most one side is correct, so every remaining
    # disagreement is both-wrong.
    swapped = n - len(agree) - len(fixed) - len(broken)
    return {
        "name": name,
        "joined": n,
        "agree_n": len(agree),
        "agreement": len(agree) / n if n else 0.0,
        "acc_base": sum(base[i]["correct"] for i in ids) / n,
        "acc_arm": sum(arm[i]["correct"] for i in ids) / n,
        "fixed": fixed,
        "broken": broken,
        "swapped": swapped,
        "families": {
            fam: (
                sum(1 for i in sub if base[i]["pred"] == arm[i]["pred"]) / len(sub),
                sum(base[i]["correct"] for i in sub) / len(sub),
                sum(arm[i]["correct"] for i in sub) / len(sub),
            )
            for fam in sorted({base[i]["family"] for i in ids})
            for sub in [[i for i in ids if base[i]["family"] == fam]]
        },
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--baseline", type=Path, required=True,
                    help="baseline JevBench run dir (e.g. fork_4b-v1)")
    ap.add_argument("--arm", type=Path, action="append", required=True,
                    help="arm run dir to compare; repeatable")
    ap.add_argument("--out", type=Path, default=None,
                    help="write the report here (default: stdout only)")
    args = ap.parse_args()

    base = load_rows(args.baseline)
    lines: list[str] = [
        f"# Arm comparison — per-item against `{args.baseline.name}`", "",
        f"Baseline: {len(base)} rows, acc "
        f"{sum(r['correct'] for r in base.values()) / len(base):.4f}"
        + (
            f", replay {'consistent' if replay_ok(args.baseline, base) else 'MISMATCH'}"
            if (args.baseline / "results-replay.jsonl").exists()
            else ", no replay file"
        ),
        "",
        "| arm | joined | agreement | acc base | acc arm | delta"
        " | fixed | broken | swapped |",
        "|---|---|---|---|---|---|---|---|---|",
    ]
    reports = []
    for arm_dir in args.arm:
        arm = load_rows(arm_dir)
        rep = compare(arm_dir.name, base, arm)
        rep["rows"] = arm
        rep["internal"] = (
            None if not (arm_dir / "results-replay.jsonl").exists()
            else replay_ok(arm_dir, arm)
        )
        rep["ece"], rep["brier"] = winner_stats(arm)
        rep["sources"] = sorted({r["probs_source"] for r in arm.values()
                                 if r["probs_source"]})
        lat = sorted(r["ms"] for r in arm.values())
        rep["lat_p50"] = percentile(lat, 0.5)
        rep["lat_mean"] = sum(lat) / len(lat) if lat else 0.0
        reports.append(rep)

        lines.append(
            f"| {rep['name']} | {rep['joined']} | {rep['agree_n']}/{rep['joined']}"
            f" ({rep['agreement']:.4f}) | {rep['acc_base']:.4f}"
            f" | {rep['acc_arm']:.4f} | {rep['acc_arm'] - rep['acc_base']:+.4f}"
            f" | {len(rep['fixed'])} | {len(rep['broken'])} | {rep['swapped']} |"
        )

    lines += ["", "## Detail", ""]
    for rep in reports:
        internal = ("no replay file" if rep["internal"] is None
                    else ("replay consistent" if rep["internal"]
                          else "REPLAY MISMATCH"))
        lines += [
            f"### {rep['name']}", "",
            f"- external agreement: {rep['agree_n']}/{rep['joined']}"
            f" = {rep['agreement']:.4f}"
            + (" — deterministic against the baseline"
               if rep["agree_n"] == rep["joined"] else ""),
            f"- internal replay: {internal}",
            f"- probs channel: {', '.join(rep['sources']) or '—'}",
        ]
        if rep["ece"] is not None:
            lines.append(f"- ECE {rep['ece']:.4f}, winner-Brier"
                         f" {rep['brier']:.4f} (winner-probability channel;"
                         " JevBench rows carry no labels)")
        lines.append(
            f"- latency context (load-confounded): p50"
            f" {rep['lat_p50'] / 1000:.2f} s, mean {rep['lat_mean'] / 1000:.2f} s"
        )
        lines.append("- per-family agreement / acc base → acc arm:")
        for fam, (agr, a0, a1) in rep["families"].items():
            lines.append(f"  - {fam}: {agr:.3f} ({a0:.3f} → {a1:.3f})")
        if rep["fixed"] or rep["broken"]:
            lines.append("- flipped items:")
            for i in sorted(rep["fixed"]):
                lines.append(f"  - {i}: wrong → **correct**")
            for i in sorted(rep["broken"]):
                lines.append(f"  - {i}: correct → **wrong**")
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
