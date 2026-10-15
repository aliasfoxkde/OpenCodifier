#!/usr/bin/env python3
"""Fusion-gate threshold refit for OOD distributions (task #114, fusion-v3).

The fusion-v2 gate (choice/boolean/score min_confidence 0.56) was fitted
in-domain; on the JevBench public split it keeps OOD engine answers at
0.434 accuracy while the same rung converts escalations at 0.833
(results/jevbench-fullsystem-fedora.md) — the +13.0 pp posture swap. This
script refits those thresholds **offline over measured per-item rows**:
the engine trace (engine-only arm on the shipped build) supplies each
item's winner probability — the gate feature (F29: margin is flat on
lexical confidences, winner-prob is the axis) — and the proofs-posture
trace supplies the rung counterfactual (same rung, same contract). Every
probability and correctness flag is a harness measurement; nothing here
is re-inferred.

Discipline (the point of the exercise):
- **Split-half**: thresholds are fitted on one deterministic half of the
  joined items (max-accuracy plateau midpoint, F29) and reported on the
  untouched half. The full-set number at the fitted threshold is the
  projection; the full-set optimum is reported only as the in-sample
  ceiling and must be disclosed as fitted-on-the-231 in any submission
  (Von precedent).
- **Abstentions gate as never-accept**: an engine abstain carries no
  probability, so the item escalates; a rung abstain scores incorrect
  (their runner's rule). A policy cannot beat the oracle by refusing.
- **Anchor reproduction**: the simulated as-shipped (t=0.56) and proofs
  (t=1.0) postures must land near their live-measured accuracies
  (0.5584 / 0.6883); a divergence is a build-vintage flag, not a tuning
  opportunity.

Usage:
  python3 runner/jevbench_gate_refit.py \
      --engine-dir ~/oc-model-eval/runs/jevbench/engine-only-fedora-v1 \
      --rung-dir ~/oc-model-eval/runs/jevbench/proofs-only-fedora-v1 \
      --tasks "$REF/datasets/public/easy.jsonl,$REF/datasets/public/hard.jsonl,$REF/datasets/public/original.jsonl" \
      --out-md ../results/jevbench-fusion-v3-gate-refit.md \
      --out-json ../results/jevbench-fusion-v3-gate-refit.json
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from run_laya import ece  # noqa: E402

GRID = [round(0.30 + 0.01 * i, 2) for i in range(71)]  # 0.30 .. 1.00
AS_SHIPPED = 0.56


def load_arm(dir_path: Path) -> dict[str, dict]:
    """Per-item rows from a run's results.jsonl.

    `prob` is the winner probability (None when the arm abstained or
    failed — the gate must treat that as never-accept); `correct` is the
    harness's own judgment (abstains score incorrect); `ms` is the
    measured wall latency.
    """
    rows: dict[str, dict] = {}
    for line in (dir_path / "results.jsonl").read_text().splitlines():
        if not line.strip():
            continue
        r = json.loads(line)
        ok = r.get("status") == "ok"
        probs = r.get("probs") or {}
        pred = r.get("predicted")
        rows[r["task_id"]] = {
            "id": r["task_id"],
            "ok": bool(ok),
            "correct": bool(r.get("correct")),
            "prob": float(probs[pred]) if ok and pred in probs else None,
            "kind": None,
            "ms": (r.get("latency_s") or 0.0) * 1000.0,
        }
    return rows


def load_kinds(tasks: str) -> dict[str, str]:
    kinds: dict[str, str] = {}
    for path in tasks.split(","):
        for line in Path(path).read_text().splitlines():
            if not line.strip():
                continue
            t = json.loads(line)
            kinds[t["id"]] = t["question"]["type"]
    return kinds


def simulate(items: list[dict], t: float) -> dict:
    """Route each item: engine iff its winner prob clears t, else rung."""
    rows = []
    for it in items:
        accept = it["engine"]["prob"] is not None and it["engine"]["prob"] >= t
        arm = it["engine"] if accept else it["rung"]
        rows.append({
            "id": it["id"], "kind": it["kind"], "rung": "engine" if accept else "llm",
            # Winner prob of whichever arm answered — the fused
            # distribution the ECE is computed over. Rung abstains carry
            # None and drop out of the ECE (they still count incorrect).
            "prob": arm["prob"],
            "correct": arm["correct"],
            # The engine always runs to produce the gate feature; an
            # escalation pays the rung's latency on top.
            "ms": it["engine"]["ms"] + (0.0 if accept else it["rung"]["ms"]),
        })
    n = len(rows)
    lat = sorted(r["ms"] for r in rows)
    by_rung: dict[str, int] = {}
    for r in rows:
        by_rung[r["rung"]] = by_rung.get(r["rung"], 0) + 1
    probs = [(r["prob"], r["correct"]) for r in rows if r["prob"] is not None]
    return {
        "t": t,
        "accuracy": sum(r["correct"] for r in rows) / n,
        "ece": ece(probs) if probs else float("nan"),
        "routing": {k: v / n for k, v in sorted(by_rung.items())},
        "mean_ms": sum(lat) / n,
        "p50_ms": lat[n // 2],
        "p95_ms": lat[int(0.95 * (n - 1))],
        "rows": rows,
    }


def plateau_midpoint(curve: list[tuple[float, float]]) -> float:
    """Midpoint of the max-accuracy threshold plateau (F29 discipline).

    The shipped optimum must sit inside a flat neighborhood, not on a
    spike; picking the plateau's middle is the stabler in-half choice.
    """
    best = max(acc for _, acc in curve)
    span = [t for t, acc in curve if acc >= best]
    return (min(span) + max(span)) / 2.0


def per_kind_band_table(items: list[dict]) -> list[str]:
    """Engine-vs-rung accuracy per kind and engine-confidence band.

    The question the sweep answers globally, asked per decision kind: is
    there a (kind, band) pocket where accepting the engine beats
    escalating? A band's engine accuracy below the rung's means the gate
    feature carries no usable OOD signal there.
    """
    bands = ((0.50, 0.60), (0.60, 0.70), (0.70, 0.80),
             (0.80, 0.90), (0.90, 0.99), (0.99, 1.01))
    out = ["", "## Per-kind pockets (engine confidence bands)", "",
           "Rung accuracy = the rung counterfactual on the same items "
           "(ok rows only). No pocket means the per-kind gate registry "
           "has nothing to register for this distribution.", "",
           "| kind | n | engine acc | rung acc | bands (engine acc) |",
           "|---|---|---|---|---|"]
    for kind in sorted({it["kind"] for it in items if it["kind"]}):
        sub = [it for it in items if it["kind"] == kind]
        n = len(sub)
        e_acc = sum(it["engine"]["correct"] for it in sub) / n
        ok_rows = [it for it in sub if it["rung"]["ok"]]
        r_acc = (sum(it["rung"]["correct"] for it in ok_rows) / len(ok_rows)
                 if ok_rows else float("nan"))
        cells = []
        for lo, hi in bands:
            s = [it for it in sub if it["engine"]["prob"] is not None
                 and lo <= it["engine"]["prob"] < hi]
            if s:
                acc = sum(it["engine"]["correct"] for it in s) / len(s)
                cells.append(f"[{lo:.2f},{hi:.2f}) {acc:.2f} (n={len(s)})")
        out.append(f"| {kind} | {n} | {e_acc:.3f} | {r_acc:.3f} | "
                   + "; ".join(cells) + " |")
    oracle = sum(1 for it in items
                 if it["engine"]["correct"] or it["rung"]["correct"]) / len(items)
    out += ["", f"Oracle (either arm correct): **{oracle:.4f}** — the "
            "routing ceiling on the joined set; the proofs posture "
            "extracts most of it because the rung, not the gate, is the "
            "asset OOD.", ""]
    return out


def boundary_finding(t_fit: float, acc_fit: float,
                     curve: list[tuple[float, float]]) -> str:
    """What the fitted threshold's position means, stated plainly."""
    if t_fit >= max(t for t, _ in curve):
        return ("**Boundary finding:** the fit half's accuracy plateau "
                "reaches the grid edge — the OOD-optimal gate on this "
                "distribution IS the proofs posture (accept exact proofs, "
                "escalate everything else). No intermediate winner-prob "
                "threshold recovers the fusion latency advantage without "
                "paying accuracy: the engine's confidence is not "
                "OOD-informative (its accepted items convert below the "
                "rung's rate at every threshold, every kind, every band). "
                "The lever this closes: threshold refit. The levers it "
                "opens: a better gate *feature* (rule/extractor provenance, "
                "not softmax confidence) or an OOD-robust engine.")
    return (f"Fitted threshold {t_fit:.2f} sits inside the curve — the "
            f"full set projects {acc_fit:.4f} at that operating point.")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--engine-dir", type=Path, required=True)
    ap.add_argument("--rung-dir", type=Path, required=True)
    ap.add_argument("--tasks", type=str, required=True)
    ap.add_argument("--out-md", type=Path, required=True)
    ap.add_argument("--out-json", type=Path, required=True)
    args = ap.parse_args()

    engine = load_arm(args.engine_dir)
    rung = load_arm(args.rung_dir)
    kinds = load_kinds(args.tasks)

    both = sorted(set(engine) & set(rung))
    only_engine = sorted(set(engine) - set(rung))
    items = []
    for i in both:
        items.append({"id": i, "kind": kinds.get(i),
                      "engine": engine[i], "rung": rung[i]})
    n = len(items)
    fit = [it for k, it in enumerate(items) if k % 2 == 0]
    hold = [it for k, it in enumerate(items) if k % 2 == 1]

    curve = [(t, simulate(items, t)["accuracy"]) for t in GRID]
    t_fit = plateau_midpoint([(t, simulate(fit, t)["accuracy"]) for t in GRID])
    sim_fit = simulate(items, t_fit)
    sim_full_best = max((simulate(items, t) for t in GRID),
                        key=lambda s: s["accuracy"])
    sim_shipped = simulate(items, AS_SHIPPED)
    sim_proofs = simulate(items, 1.0)

    # The proofs-posture trace keeps its engine-answered items in place
    # (their rows carry engine answers, not rung counterfactuals). They
    # sit at prob = 1.0, so under every swept threshold they remain
    # engine-answered and the sweep never consults those rows as rung
    # outcomes — the t=1.00 anchor reproducing the live 0.6883 exactly
    # is the check.
    keep_safe = all(engine[i]["prob"] is not None
                    and engine[i]["prob"] >= max(t_fit, AS_SHIPPED)
                    for i in only_engine)
    proofs_kept = [i for i in both
                   if engine[i]["prob"] is not None
                   and engine[i]["prob"] >= 1.0
                   and engine[i]["ok"]]

    lines = [
        "# Fusion-gate OOD refit — engine + E2B-QAT rung (post-hoc, task #114)",
        "",
        "Offline over measured per-item rows: engine trace "
        f"`{args.engine_dir.name}` (gate features, shipped build), rung "
        f"trace `{args.rung_dir.name}` (E2B-QAT counterfactual). Nothing "
        "re-inferred; abstains gate as never-accept.",
        "",
        f"Joined {n} items. The proofs posture's own engine-kept items "
        f"({len(proofs_kept)}) carry engine answers in the rung trace; "
        "they sit at prob = 1.0 and stay engine-answered under every "
        "swept threshold, so they never enter the sweep as rung "
        f"outcomes (checked: {keep_safe}).",
        "",
        "## Anchors (simulated vs live-measured)",
        "",
        "| posture | t | sim acc | live acc | routing | mean ms | p95 |",
        "|---|---|---|---|---|---|---|",
        f"| as-shipped fusion-v2 | {AS_SHIPPED:.2f} | "
        f"{sim_shipped['accuracy']:.4f} | 0.5584 | "
        + ", ".join(f"{k} {v:.0%}" for k, v in
                    sim_shipped["routing"].items()) +
        f" | {sim_shipped['mean_ms']:.0f} | {sim_shipped['p95_ms']:.0f} |",
        f"| proofs-only | 1.00 | {sim_proofs['accuracy']:.4f} | 0.6883 | "
        + ", ".join(f"{k} {v:.0%}" for k, v in sim_proofs["routing"].items()) +
        f" | {sim_proofs['mean_ms']:.0f} | {sim_proofs['p95_ms']:.0f} |",
        "",
        "## Split-half refit",
        "",
        f"Fitted on {len(fit)} items (even indices), held out "
        f"{len(hold)} (odd). Plateau-midpoint threshold on the fit half: "
        f"**t* = {t_fit:.2f}**.",
        "",
        "| view | t | acc | ECE | routing | mean ms | p50 | p95 |",
        "|---|---|---|---|---|---|---|---|",
        f"| fit-half @ t* | {t_fit:.2f} | "
        f"{simulate(fit, t_fit)['accuracy']:.4f} | "
        f"{simulate(fit, t_fit)['ece']:.3f} | — | — | — | — |",
        f"| **hold-half @ t*** | {t_fit:.2f} | "
        f"**{simulate(hold, t_fit)['accuracy']:.4f}** | "
        f"{simulate(hold, t_fit)['ece']:.3f} | — | — | — | — |",
        f"| full set @ t* | {t_fit:.2f} | {sim_fit['accuracy']:.4f} | "
        f"{sim_fit['ece']:.3f} | "
        + ", ".join(f"{k} {v:.0%}" for k, v in sim_fit["routing"].items()) +
        f" | {sim_fit['mean_ms']:.0f} | {sim_fit['p50_ms']:.0f} | "
        f"{sim_fit['p95_ms']:.0f} |",
        f"| full set in-sample max | {sim_full_best['t']:.2f} | "
        f"{sim_full_best['accuracy']:.4f} | {sim_full_best['ece']:.3f} | "
        + ", ".join(f"{k} {v:.0%}" for k, v in
                    sim_full_best["routing"].items()) +
        f" | {sim_full_best['mean_ms']:.0f} | "
        f"{sim_full_best['p50_ms']:.0f} | {sim_full_best['p95_ms']:.0f} |",
        "",
        boundary_finding(t_fit, sim_fit["accuracy"], curve),
        "",
        "## Threshold curve (full set)", "",
        "| t | acc |", "|---|---|",
    ]
    lines += [f"| {t:.2f} | {a:.4f} |" for t, a in curve]
    lines += per_kind_band_table(items)
    lines += [
        "",
        "Disclosure: every number on this page is fitted and/or evaluated "
        "on the 231 public items (Von precedent) — in-sample for the "
        "full-set rows, half-held-out for the split-half rows. The sealed "
        "half is untestable locally by construction.",
        "",
    ]
    args.out_md.write_text("\n".join(lines) + "\n")

    payload = {
        "refit_version": "opencodifier.jevbench-gate-refit/1",
        "engine_dir": args.engine_dir.name,
        "rung_dir": args.rung_dir.name,
        "joined": n,
        "only_engine": {"ids": only_engine, "projection_exact": keep_safe},
        "t_fit": round(t_fit, 2),
        "boundary_finding": t_fit >= GRID[-1],
        "grid": GRID,
        "curve": [{"t": t, "accuracy": a} for t, a in curve],
        "sim": {
            "as_shipped": {k: v for k, v in sim_shipped.items() if k != "rows"},
            "proofs": {k: v for k, v in sim_proofs.items() if k != "rows"},
            "fit_half": simulate(fit, t_fit) | {"rows": []},
            "hold_half": simulate(hold, t_fit) | {"rows": []},
            "full_at_t_fit": {k: v for k, v in sim_fit.items() if k != "rows"},
            "full_in_sample_max": {k: v for k, v in sim_full_best.items()
                                   if k != "rows"},
        },
    }
    args.out_json.write_text(json.dumps(payload, indent=1) + "\n")
    print(f"wrote {args.out_md}")
    print(f"wrote {args.out_json}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
