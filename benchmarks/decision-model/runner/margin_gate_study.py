#!/usr/bin/env python3
"""Rank/margin gate study for the embedding rung (CALIBRATION.md finding 3).

The embedding rung's softmax scores are ordering-only: the winner-probability
fit is degenerate (NLL falls to the T -> infinity limit; CALIBRATION.md),
so no temperature calibrates them and a probability gate can never be
trusted on this rung. The §19 `min_margin` gate is the alternative: accept
only when top1 - top2 is wide enough, else escalate.

This script sweeps that gate **offline over measured per-item rows** — the
margins come from an `--margins` arm (run_embed.py re-run with per-item
margin + full probs recorded); nothing here is re-inferred. Per threshold:

- coverage: share of items the rung would accept (the rest escalate);
- accepted accuracy: accuracy on accepted items only (the number that has
  to justify not escalating);
- accepted ECE: calibration of the accepted subset (selection usually
  improves it, which the ladder inherits);
- accepted errors: absolute count — what the gate lets through.

The operating point of record is the smallest threshold whose accepted
accuracy reaches `--target` (default 0.85, the bar the engine rung clears
post-fusion): lower thresholds buy coverage at accuracy's expense.

Usage:
  python3 runner/margin_gate_study.py \
      --margins ../results/embed__gte-modernbert-onnx-fp32__margins.json \
      --out ../results/embed-margin-study.md
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from run_laya import ece  # noqa: E402


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--margins", required=True, help="run_embed.py margins JSON")
    ap.add_argument("--out", required=True, help="markdown report path")
    ap.add_argument("--target", type=float, default=0.85,
                    help="accepted-accuracy bar for the operating point")
    args = ap.parse_args()

    payload = json.loads(Path(args.margins).read_text())
    rows: list[dict] = payload["single"]
    for row in rows:
        row["correct"] = row["pred"] == row["answer"]

    def accepted_ece(accepted: list[dict]) -> float:
        """run_laya's ECE over the accepted subset's (prob, correct) pairs."""
        return ece([(row["prob"], row["correct"]) for row in accepted])

    # Adaptive grid: embedding margins span a narrow, data-dependent range
    # (this arm's max is ≈0.07), so a fixed 0..1 grid would be dead rows.
    margins = sorted(row["margin"] for row in rows)

    def quantile(fraction: float) -> float:
        position = fraction * (len(margins) - 1)
        low = int(position)
        high = min(low + 1, len(margins) - 1)
        weight = position - low
        return margins[low] * (1.0 - weight) + margins[high] * weight

    thresholds = sorted({round(quantile(fraction), 4) for fraction in
                         (i / 20 for i in range(21))})
    sweep = []
    for threshold in thresholds:
        accepted = [row for row in rows if row["margin"] >= threshold]
        if not accepted:
            continue
        accuracy = sum(row["correct"] for row in accepted) / len(accepted)
        sweep.append((
            threshold,
            len(accepted) / len(rows),
            accuracy,
            accepted_ece(accepted),
            sum(1 for row in accepted if not row["correct"]),
        ))

    operating = next(
        (entry for entry in sweep if entry[2] >= args.target),
        sweep[-1],
    )
    correct_margins = sorted(row["margin"] for row in rows if row["correct"])
    wrong_margins = sorted(row["margin"] for row in rows if not row["correct"])

    def auc() -> float:
        """P(correct item has a wider margin than a wrong one), ties 0.5."""
        if not wrong_margins:
            return 1.0
        wins = ties = 0
        for right in correct_margins:
            for wrong in wrong_margins:
                if right > wrong:
                    wins += 1
                elif right == wrong:
                    ties += 1
        pairs = len(correct_margins) * len(wrong_margins)
        return (wins + 0.5 * ties) / pairs if pairs else 0.0

    digest = hashlib.sha256(Path(args.margins).read_bytes()).hexdigest()[:12]
    lines = [
        "# Rank/margin gate study — embedding rung (CALIBRATION finding 3)",
        "",
        f"Source: `{Path(args.margins).name}` (sha256 {digest}, "
        f"{len(rows)} items, config {json.dumps(payload['config'])}). "
        "Every margin, probability and label is a harness measurement.",
        "",
        f"Separation: mean margin {sum(correct_margins) / len(correct_margins):.4f} on "
        f"correct vs {sum(wrong_margins) / len(wrong_margins):.4f} on wrong items; "
        f"P(correct margin wider) = {auc():.3f}.",
        "",
        "## Gate sweep (`min_margin`)",
        "",
        "| margin ≥ | coverage | accepted acc | accepted ECE | accepted errors |",
        "|---|---|---|---|---|",
    ]
    for threshold, coverage, accuracy, subset_ece, errors in sweep:
        marker = " ← operating point" if (threshold, coverage, accuracy, subset_ece, errors) == operating else ""
        lines.append(
            f"| {threshold:.4f} | {coverage:.3f} | {accuracy:.3f} | "
            f"{subset_ece:.3f} | {errors} |{marker}"
        )
    lines += [
        "",
        f"**Operating point: margin ≥ {operating[0]:.4f}** — "
        f"coverage {operating[1]:.3f}, accepted accuracy {operating[2]:.3f}, "
        f"accepted ECE {operating[3]:.3f}, {operating[4]} errors let through "
        f"of {sum(1 for row in rows if not row['correct'])} total.",
        "",
        "Ladder reading: the embedding rung accepts the top "
        f"{operating[1] * 100:.0f}% of items outright at ≥{operating[2] * 100:.0f}% "
        "accuracy and escalates the rest to the decision-model rung; its "
        "gate key is `min_margin`, never the (uncalibratable) probability. "
        "Two cautions: accepted-set ECE is computed on a small subset "
        "(n = "
        f"{round(operating[1] * len(rows))}) and swings wildly with "
        "selection — it is informational, not a gate input; and coverage "
        "at the operating point is modest, which is the honest price of "
        "an ordering-only scorer — the rung is a cheap pre-filter, not a "
        "replacement for the model rung.",
        "",
    ]
    Path(args.out).write_text("\n".join(lines))
    print(f"wrote {args.out}: operating point margin ≥ {operating[0]:.2f}, "
          f"coverage {operating[1]:.3f}, accepted acc {operating[2]:.3f}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
