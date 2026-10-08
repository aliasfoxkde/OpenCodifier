#!/usr/bin/env python3
"""Assemble the §9.7 Phase C pilot selection table from e1c run dirs.

For each pilot arm this reads three artifacts the chain already wrote —
``train-manifest.json`` (knobs, best checkpoint), the arm's training log
(per-family val-loss table at every val step; the manifest carries only
the overall best), and ``eval-{best,last}/summary.json`` (suite_holdout
accuracy / ECE / per-class accuracy) — and prints one markdown row per
arm plus the pre-registered §9.7 gate checks for the P2 arms:

* relational_compositional accuracy >= 0.60 (best-val adapter),
* ECE <= 0.15,
* relational val curve still descending (last val point < first).

Nothing is invented: an arm missing an artifact reports ``missing`` in
that column and its gate checks are skipped, not failed.
"""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

DEFAULT_ARMS = (
    "p0-lr1e4,p1-lr1e4,p2-lr1e4,p3-lr1e4,p2-lr5e5,p2-lr2e4"
)

VAL_LINE = re.compile(
    r"val step=(\d+) loss=([0-9.]+) .*per_family=(\{.*\})"
)

RELATIONAL_CLASS = "relational_compositional"
RELATIONAL_GATE_ACC = 0.60
ECE_GATE = 0.15


def parse_val_history(log_path: Path) -> list[dict]:
    """Extract every val line: step, overall loss, per-family losses."""
    history: list[dict] = []
    if not log_path.exists():
        return history
    for line in log_path.read_text(encoding="utf-8", errors="replace").splitlines():
        match = VAL_LINE.search(line)
        if match:
            history.append({
                "step": int(match.group(1)),
                "loss": float(match.group(2)),
                "per_family": json.loads(match.group(3)),
            })
    return history


def relational_curve(history: list[dict]) -> list[tuple[int, float]]:
    return [
        (point["step"], point["per_family"][RELATIONAL_CLASS])
        for point in history
        if RELATIONAL_CLASS in point["per_family"]
    ]


def read_summary(path: Path) -> dict | None:
    if not path.exists():
        return None
    return json.loads(path.read_text(encoding="utf-8"))


def arm_row(runs_root: Path, logs_root: Path, arm: str) -> dict:
    run_dir = runs_root / f"e1c-{arm}"
    manifest_path = run_dir / "train-manifest.json"
    row: dict = {"arm": arm}

    manifest = None
    if manifest_path.exists():
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    # e1-train/3 manifests carry the val curve themselves (A2 fix);
    # log scraping stays as the fallback for v2 manifests whose logs
    # survive.
    history = (manifest or {}).get("val_history") or parse_val_history(
        logs_root / f"e1c-{arm}-train.log")
    curve = relational_curve(history)

    if manifest:
        row["steps"] = manifest.get("steps")
        row["rows"] = (manifest.get("data") or {}).get("rows")
        row["lr"] = (manifest.get("knobs") or {}).get("lr")
        row["data_sha12"] = ((manifest.get("data") or {}).get("sha256")
                             or "")[:12]
        row["best_checkpoint"] = manifest.get("best_checkpoint")
        row["final_train_loss"] = manifest.get("final_loss_token_weighted")
    else:
        row["steps"] = row["lr"] = row["data_sha12"] = None
        row["best_checkpoint"] = row["final_train_loss"] = None

    row["val_points"] = len(history)
    row["relational_val_first_last"] = (
        [curve[0][1], curve[-1][1]] if len(curve) >= 2 else None)
    row["relational_val_descending"] = (
        bool(curve and len(curve) >= 2 and curve[-1][1] < curve[0][1]))
    per_family_final = history[-1]["per_family"] if history else None
    row["per_family_val_final"] = per_family_final

    for tag in ("best", "last"):
        summary = read_summary(run_dir / f"eval-{tag}" / "summary.json")
        if summary is None:
            row[f"acc_{tag}"] = row[f"ece_{tag}"] = None
            row[f"relational_acc_{tag}"] = None
        else:
            by_class = summary.get("accuracy_by_class") or {}
            rel = by_class.get(RELATIONAL_CLASS) or {}
            row[f"acc_{tag}"] = summary.get("accuracy")
            row[f"ece_{tag}"] = summary.get("ece")
            row[f"relational_acc_{tag}"] = rel.get("acc")
    return row


def fmt(value, digits: int = 4) -> str:
    if value is None:
        return "missing"
    if isinstance(value, float):
        return f"{value:.{digits}f}"
    return str(value)


def gate_lines(p2_rows: list[dict]) -> list[str]:
    """The pre-registered §9.7 Phase-D gate checks over the P2 arms."""
    lines: list[str] = []
    for row in p2_rows:
        acc = row.get("relational_acc_best")
        ece = row.get("ece_best")
        checks = []
        if acc is None:
            checks.append("relational acc: MISSING")
        else:
            checks.append(
                f"relational acc {acc:.4f} >= 0.60: "
                + ("PASS" if acc >= RELATIONAL_GATE_ACC else "FAIL"))
        if ece is None:
            checks.append("ECE: MISSING")
        else:
            checks.append(
                f"ECE {ece:.4f} <= 0.15: "
                + ("PASS" if ece <= ECE_GATE else "FAIL"))
        if row.get("relational_val_first_last") is None:
            checks.append("relational val curve: MISSING")
        else:
            checks.append(
                "relational val curve descending: "
                + ("PASS" if row.get("relational_val_descending")
                   else "FAIL"))
        lines.append(f"- **{row['arm']}**: " + "; ".join(checks))
    return lines


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runs-root", type=Path, default=Path.home()
                        / "oc-model-eval" / "runs")
    parser.add_argument("--logs-root", type=Path, default=Path.home()
                        / "oc-model-eval" / "logs")
    parser.add_argument("--arms", default=DEFAULT_ARMS,
                        help="comma list of e1c arm names")
    parser.add_argument("--out", type=Path, default=None,
                        help="optional JSON output path for the table")
    args = parser.parse_args()

    arms = [arm.strip() for arm in args.arms.split(",") if arm.strip()]
    rows = [arm_row(args.runs_root, args.logs_root, arm) for arm in arms]

    header = (
        "| arm | lr | rows | best ckpt (step/loss) | rel val first->last"
        " | acc best | ECE best | rel acc best | acc last |")
    split = "|---|---|---|---|---|---|---|---|---|"
    print(header)
    print(split)
    for row in rows:
        best = row.get("best_checkpoint") or {}
        rel_fl = row.get("relational_val_first_last")
        rel_str = ("missing" if rel_fl is None else
                   f"{rel_fl[0]:.4f}->{rel_fl[1]:.4f}")
        print(
            f"| {row['arm']} | {fmt(row.get('lr'))} | {fmt(row.get('rows'))} "
            f"| {fmt(best.get('step'))}/{fmt(best.get('val_loss'))} "
            f"| {rel_str} | {fmt(row.get('acc_best'))} "
            f"| {fmt(row.get('ece_best'))} "
            f"| {fmt(row.get('relational_acc_best'))} "
            f"| {fmt(row.get('acc_last'))} |")

    p2_rows = [row for row in rows if row["arm"].startswith("p2-")]
    if p2_rows:
        print("\n## §9.7 Phase-D gates (P2 arms, best-val adapter)\n")
        for line in gate_lines(p2_rows):
            print(line)

    if args.out:
        args.out.write_text(json.dumps(rows, indent=1) + "\n",
                            encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
