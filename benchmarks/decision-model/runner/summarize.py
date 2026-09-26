#!/usr/bin/env python3
"""Merge benchmark result files into one comparison table and manifest.

Reads every results/*.json produced by the runners and emits:

- results/summary.md — the human-readable comparison (accuracy per class,
  ECE, latency, determinism) across all arms and models
- models.manifest.json — the SHA-256 manifest of every model used (D14
  practice: artifacts are identified by content hash, never by trust)

Usage:
  python3 runner/summarize.py --results-dir results
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


def fmt_class_table(acc_by_class: dict) -> str:
    keys = ["metadata_match", "lexical_semantic", "relational_compositional"]
    return " / ".join(f"{acc_by_class.get(k, float('nan')):.2f}" for k in keys)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--results-dir", type=Path, default=Path(__file__).parent.parent / "results")
    args = ap.parse_args()

    results = []
    for p in sorted(args.results_dir.glob("*.json")):
        if p.name == "models.manifest.json":
            continue
        d = json.loads(p.read_text())
        if "arm" not in d or "metrics" not in d:
            continue
        results.append((p.name, d))

    if not results:
        print("no result files found", file=sys.stderr)
        return 1

    manifest = {}
    lines = [
        "# Decision-model benchmark — results",
        "",
        "Suite: `suite/suite.json` (120 items; metadata_match / lexical_semantic /",
        "relational_compositional, 40 each). Accuracy is top-1 candidate id.",
        "Columns: accuracy (meta/lex/rel), overall accuracy, ECE of the winner",
        "probability, single-decision latency, determinism.",
        "",
        "| run | acc (meta/lex/rel) | acc | ECE | p50 | determinism |",
        "|---|---|---|---|---|---|",
    ]
    for name, d in results:
        m = d["metrics"]
        acc = m["accuracy"]
        ece = m.get("ece")
        lat = m.get("latency") or {}
        p50 = lat.get("p50_ms")
        if p50 is None and "ms_per_item" in m:
            p50 = m["ms_per_item"]
        det = d.get("determinism", {})
        det_s = (
            "yes"
            if det.get("predictions_match")
            else f"NO (Δ{det.get('max_prob_delta', 0):.2g})"
        )
        ece_s = f"{ece:.3f}" if ece is not None else "—"
        p50_s = f"{p50:.1f}ms" if p50 is not None else "—"
        lines.append(
            f"| {name} | {fmt_class_table(m['accuracy_by_class'])} | {acc:.3f} "
            f"| {ece_s} | {p50_s} | {det_s} |"
        )
        model = d.get("model", {})
        if "sha256" in model:
            manifest[model["file"]] = model["sha256"]
        elif "sha256_model_onnx" in model:
            manifest[model["name"]] = model["sha256_model_onnx"]

    lines += ["", "## Notes", ""]
    for name, d in results:
        if d.get("chat"):
            cm = d["chat"]["metrics"]
            lines.append(
                f"- `{name}` chat (JSON-writing) baseline: acc {cm['accuracy']:.3f}, "
                f"p50 {cm['p50_ms']:.0f}ms — the token-by-token alternative the "
                "decision arm replaces."
            )
        if d.get("batched"):
            per = d["batched"].get("per_class", {})
            parts = [
                f"{cls}: {v.get('per_decision_ms'):.0f}ms" for cls, v in sorted(per.items())
                if v.get("per_decision_ms") is not None
            ]
            if parts:
                lines.append(f"- `{name}` bulk per-decision (batched contexts): {'; '.join(parts)}.")
        if d.get("arm") == "engine_builtin_lexical":
            lines.append(
                f"- `{name}` outcomes: {d['metrics'].get('outcomes')} — abstention/verify "
                "routing is part of the engine contract, not a failure."
            )
    lines.append("")

    out_md = args.results_dir / "summary.md"
    out_md.write_text("\n".join(lines))
    (args.results_dir / "models.manifest.json").write_text(
        json.dumps(manifest, sort_keys=True, indent=1) + "\n"
    )
    print(f"wrote {out_md} and models.manifest.json ({len(results)} runs)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
