#!/usr/bin/env python3
"""Post-hoc fusion study: what would a confidence-gated escalation ladder score?

Simulates the deterministic-first ladder (engine → embedding → LLM decision
arm, or engine → vtx on JevBench) **offline, over measured per-item rows** —
every probability, prediction and latency in this study was produced by a
harness arm; nothing here is re-inferred. This bounds what a live ladder
could score before we wire one into the policy (§19 uncertainty gates).

Three views per policy:
- single arms (reference rows);
- gated cascade with acceptance thresholds swept over a grid — an arm's
  decision is *accepted* when its winner probability clears the rung's
  threshold, else the item falls to the next rung; total per-item cost is
  the sum of the rungs actually incurred;
- oracle bounds: any-correct (no routing can beat it) and cheapest-correct.

Usage (suite):
  python3 runner/fusion_study.py --runs-dir <dir> \
      --engine "engine__relational-v1|builtin-lexical-v1.json" \
      --embed embed__gte-modernbert-onnx-fp32.json \
      --llm llama__Qwen3.5-2B.json --out fusion-suite.md

Usage (JevBench):
  python3 runner/fusion_study.py --jevbench \
      --engine-dir <jevbench/engine-v4> --fallback-dir <jevbench/vtx-v1> \
      --out fusion-jevbench.md

A JevBench ladder may carry a third rung (`--llm-dir`, e.g. the 4B fork
arm behind the vtx rung). Label-only arms (the fork's tree ships the
winner's mass, D15) have no `probs` in results.jsonl; their winner
probability is recovered from the arm's verbatim raw payloads
(`raw/`, indexed by the rows' `raw_sha256` content hash).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from run_laya import ece  # noqa: E402


def load_suite_arm(path: Path) -> dict[str, dict]:
    d = json.loads(path.read_text())
    rows = {}
    for r in d["single"]:
        rows[r["id"]] = {
            "id": r["id"],
            "pred": r["pred"],
            "answer": r["answer"],
            "correct": r["pred"] == r["answer"],
            "prob": float(r["prob"]),
            # Margin (top minus runner-up) when the arm recorded it —
            # run_embed.py emits it since the margin-gate study.
            "margin": float(r["margin"]) if r.get("margin") is not None else None,
            "class": r["class"],
            "ms": r.get("wall_ms") or r.get("server_ms"),
        }
    return rows


def raw_index(dir_path: Path) -> dict[str, Path]:
    """The arm's raw payloads keyed by `raw_sha256` = sha256(content)."""
    return {
        hashlib.sha256(f.read_bytes()).hexdigest(): f
        for f in (dir_path / "raw").glob("*.json")
    }


def raw_winner_prob(payload: dict, pred: str | None) -> float | None:
    """Winner probability from a label-only arm's raw response payload.

    The JevBench harness stores each response verbatim; three shapes exist
    across our arms and the winner's own mass is the whole
    calibrated-probability channel this study gates on, so each lookup is
    anchored to the row's `predicted` — an abstained row (no winner) and
    an unmatchable payload both gate as "never accept", which is what a
    live ladder does with an abstention (D27).
    """
    if pred is None:
        return None
    resp = payload.get("response") or {}
    answer = resp.get("answer") or {}
    for entry in (answer.get("distribution") or {}).get("entries") or []:
        if entry.get("key") == pred and entry.get("probability") is not None:
            return float(entry["probability"])
    field = resp.get("field") or {}
    if field.get("value") == pred and field.get("probability") is not None:
        return float(field["probability"])
    dist = resp.get("distribution")
    if isinstance(dist, dict) and dist.get(pred) is not None:
        return float(dist[pred])
    return None


def load_jevbench(dir_path: Path) -> dict[str, dict]:
    rows = {}
    raw = raw_index(dir_path)
    results = dir_path / "results.jsonl"
    for line in results.read_text().splitlines():
        if not line.strip():
            continue
        r = json.loads(line)
        probs = r.get("probs") or {}
        pred = r.get("predicted")
        prob = float(probs[pred]) if pred in probs else None
        if prob is None and r.get("raw_sha256") in raw:
            payload = json.loads(raw[r["raw_sha256"]].read_text())
            prob = raw_winner_prob(payload, pred)
        rows[r["task_id"]] = {
            "id": r["task_id"],
            "pred": pred,
            "answer": None,  # JevBench rows carry `correct`, not the label
            "correct": bool(r.get("correct")),
            "prob": prob,
            "class": r.get("family"),
            "ms": (r.get("latency_s") or 0) * 1000.0,
        }
    return rows


def join(arms: dict[str, dict[str, dict]]) -> list[dict]:
    """Inner-join arms on item id; answers must agree or the study aborts."""
    ids = set.intersection(*(set(a) for a in arms.values()))
    items = []
    for i in sorted(ids):
        base = next(iter(arms.values()))[i]
        for arm in arms.values():
            # Suite rows know the label; JevBench rows only know correctness,
            # so the agreement check runs only where labels exist.
            if base["answer"] is not None and arm[i]["answer"] != base["answer"]:
                raise SystemExit(f"answer mismatch on {i}: arms disagree")
        items.append({"id": i, **base})
    return items


def arm_cost(items: list[dict], rows: dict[str, dict], fallback_ms: float) -> float:
    ms = [rows[it["id"]]["ms"] or fallback_ms for it in items]
    return sum(ms) / len(ms) if ms else fallback_ms


def cascade(
    items: list[dict],
    ladder: list[tuple[str, dict[str, dict], float, float, str]],
) -> dict:
    """Walk the ladder per item: accept the first rung whose gate clears t.

    `ladder` entries are (name, rows, threshold, mean_ms, gate) where gate
    is "prob" (winner probability) or "margin" (top minus runner-up — the
    rank/margin form CALIBRATION.md finding 3 requires for rungs without a
    calibrated probability channel). The final rung always answers — by
    position, not by a 0.0 threshold, so a label-only final rung (winner
    mass only, D15) needs no probability to ship its answer. Latency
    accumulates over every rung the item actually passed through, gate
    evaluation itself is free.
    """

    def gate_value(r: dict, gate: str) -> float | None:
        if gate == "margin":
            return r["margin"] if r.get("margin") is not None else r["prob"]
        return r["prob"]

    rows_out = []
    last = len(ladder) - 1
    for it in items:
        spent = 0.0
        for i, (name, arm_rows, t, mean_ms, gate) in enumerate(ladder):
            r = arm_rows[it["id"]]
            spent += r["ms"] or mean_ms
            v = gate_value(r, gate)
            if i == last or (v is not None and v >= t):
                rows_out.append(
                    {"id": it["id"], "class": it["class"], "rung": name,
                     "prob": r["prob"], "correct": r["correct"],
                     "ms": spent}
                )
                break
    n = len(rows_out)
    by_rung: dict[str, int] = {}
    for r in rows_out:
        by_rung[r["rung"]] = by_rung.get(r["rung"], 0) + 1
    return {
        "accuracy": sum(1 for r in rows_out if r["correct"]) / n,
        "ece": ece([(r["prob"], r["correct"]) for r in rows_out]),
        "mean_ms": sum(r["ms"] for r in rows_out) / n,
        "routing": {k: v / n for k, v in sorted(by_rung.items())},
        "rows": rows_out,
    }


def by_class_acc(rows: list[dict]) -> dict:
    out = {}
    for cls in sorted({r["class"] for r in rows}):
        sub = [r for r in rows if r["class"] == cls]
        out[cls] = sum(1 for r in sub if r["correct"]) / len(sub)
    return out


def fmt_routing(r: dict) -> str:
    return ", ".join(f"{k} {v:.0%}" for k, v in r.items())


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs-dir", type=Path,
                    default=Path("/nas/Temp/work/oc-model-eval/runs"))
    ap.add_argument("--engine", default="engine__relational-v1|builtin-lexical-v1.json")
    ap.add_argument("--embed", default=None)
    ap.add_argument("--llm", default=None)
    ap.add_argument("--jevbench", action="store_true",
                    help="engine-dir/fallback-dir are JevBench run dirs")
    ap.add_argument("--engine-dir", type=Path, default=None)
    ap.add_argument("--fallback-dir", type=Path, default=None)
    ap.add_argument("--llm-dir", type=Path, default=None,
                    help="JevBench third rung (answers last), e.g. the "
                         "fork arm behind the vtx rung")
    ap.add_argument("--embed-gate", choices=("prob", "margin", "both"), default="prob",
                    help="gate form for the embedding rung: winner probability "
                    "or top-minus-runner-up margin (rank/margin form per "
                    "CALIBRATION.md finding 3); 'both' sweeps each and "
                    "compares in one report")
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()

    grid = [0.30, 0.35, 0.40, 0.45, 0.50, 0.55, 0.60, 0.65, 0.70,
            0.75, 0.80, 0.85, 0.90, 0.95]

    lines: list[str] = [
        "# Fusion study — confidence-gated escalation ladder (post-hoc)",
        "",
        "Simulated offline over measured per-item arm rows; every probability,",
        "prediction and latency is a harness measurement, nothing re-inferred.",
        "This bounds a live ladder before one is wired into the policy.",
        "",
    ]

    if args.jevbench:
        engine = load_jevbench(args.engine_dir)
        arms = {"engine": engine, "fallback": load_jevbench(args.fallback_dir)}
        if args.llm_dir:
            arms["llm"] = load_jevbench(args.llm_dir)
        items = join(arms)
        lines.append(f"Joined {len(items)} JevBench items across "
                     + " + ".join(d.name for d in (args.engine_dir,
                                                   args.fallback_dir,
                                                   args.llm_dir) if d)
                     + ".")
    else:
        engine = load_suite_arm(args.runs_dir / args.engine)
        arms = {"engine": engine}
        if args.embed:
            arms["embed"] = load_suite_arm(args.runs_dir / args.embed)
        if args.llm:
            arms["llm"] = load_suite_arm(args.runs_dir / args.llm)
        items = join(arms)
        lines.append(f"Joined {len(items)} suite items across "
                     + ", ".join(f"`{k}`" for k in arms) + ".")

    # Reference: each single arm on the joined set.
    lines += ["", "## Single arms (reference, joined set)", ""]
    for name, rows in arms.items():
        sub = [r for r in rows.values() if r["id"] in {i["id"] for i in items}]
        n = len(sub)
        acc = sum(1 for r in sub if r["correct"]) / n
        # Both loaders carry harness-judged correctness; JevBench rows have
        # no label to re-derive it from.
        probs = [(r["prob"], r["correct"]) for r in sub
                 if r["prob"] is not None]
        e = ece(probs) if probs else float("nan")
        lines.append(f"- **{name}**: acc {acc:.3f}, ECE {e:.3f}")

    # Oracle bounds.
    any_correct = sum(
        1 for it in items if any(arms[a][it["id"]]["correct"] for a in arms)
    ) / len(items)
    lines += ["", f"Oracle (any rung correct): **{any_correct:.3f}** — "
              "no routing policy can exceed this on the joined set.", ""]

    # Cascades: sweep the acceptance thresholds of every non-final rung.
    engine_mean = arm_cost(items, engine, 0.0)
    if args.jevbench:
        fallback_name, fallback_rows = "fallback", arms["fallback"]
    elif "llm" in arms:
        fallback_name, fallback_rows = "llm", arms["llm"]
    else:
        fallback_name, fallback_rows = "embed", arms["embed"]
    fb_mean = arm_cost(items, fallback_rows, 0.0)
    embed_mean = arm_cost(items, arms["embed"], 0.0) if "embed" in arms else 0.0
    llm_mean = arm_cost(items, arms["llm"], 0.0) if "llm" in arms else 0.0

    # The middle rung whose gate gets swept: the embedding arm on suite
    # runs, the fallback arm on a three-rung JevBench ladder (vtx behind
    # the model rung). With no middle rung the ladder is engine → final.
    mid = None
    if "embed" in arms:
        mid = ("embed", arms["embed"], embed_mean)
    elif args.jevbench and "llm" in arms:
        mid = ("fallback", arms["fallback"], fb_mean)

    results = []
    t_final = 0.0
    embed_gates = (
        ["prob", "margin"]
        if args.embed_gate == "both" and mid
        else [args.embed_gate if mid else "prob"]
    )
    for embed_gate in embed_gates:
        for t_e in grid:
            base_ladder = [("engine", engine, t_e, engine_mean, "prob")]
            if mid is None:
                final_name = "llm" if "llm" in arms else fallback_name
                final_rows = arms.get("llm", fallback_rows)
                ladder = base_ladder + [
                    (final_name, final_rows, t_final, fb_mean, "prob")
                ]
                results.append((t_e, None, embed_gate, cascade(items, ladder)))
                continue
            for t_m in grid:
                ladder = base_ladder + [(mid[0], mid[1], t_m, mid[2], embed_gate)]
                if "llm" in arms:
                    ladder.append(("llm", arms["llm"], t_final, llm_mean, "prob"))
                elif fallback_name == "llm":
                    ladder.append(("llm", arms["llm"], t_final, fb_mean, "prob"))
                results.append((t_e, t_m, embed_gate, cascade(items, ladder)))

    def render(res, t_e, t_m, gate):
        acc = res["accuracy"]
        return (f"| {t_e:.2f} | {('%.2f' % t_m) if t_m is not None else '—'} "
                f"| {gate} | {acc:.3f} | {res['ece']:.3f} | {res['mean_ms']:.1f} "
                f"| {fmt_routing(res['routing'])} |")

    best = max(results, key=lambda r: r[3]["accuracy"])
    budget = max(50.0, 0.25 * fb_mean)  # ladder mean ms must stay ≤ this
    cheap = max(
        (r for r in results if r[3]["mean_ms"] <= budget),
        key=lambda r: r[3]["accuracy"], default=None,
    )
    lines += [
        "## Cascade results (thresholds swept)", "",
        "Accept a rung when its gate clears the threshold (`prob` = winner",
        "probability, `margin` = top minus runner-up); final rung always",
        "answers. Cost = rungs actually incurred.", "",
        "| engine t | fallback t | gate | acc | ECE | mean ms | routing |",
        "|---|---|---|---|---|---|---|",
    ]
    shown = {id(r[3]) for r in (best, cheap) if r}
    for t_e, t_m, gate, res in results:
        if id(res) in shown or (t_e, t_m) in {(0.50, 0.50), (0.60, 0.50)}:
            lines.append(render(res, t_e, t_m, gate))
    lines += [
        "",
        f"Best accuracy: {best[3]['accuracy']:.3f} at engine t={best[0]:.2f}"
        + (f", fallback t={best[1]:.2f}" if best[1] is not None else "")
        + f", gate {best[2]}"
        + f" ({fmt_routing(best[3]['routing'])}; mean {best[3]['mean_ms']:.1f} ms).",
    ]
    if cheap:
        lines.append(
            f"Best under {budget:.0f} ms mean: {cheap[3]['accuracy']:.3f} at "
            f"engine t={cheap[0]:.2f}"
            + (f", fallback t={cheap[1]:.2f}" if cheap[1] is not None else "")
            + f", gate {cheap[2]}"
            + f" ({fmt_routing(cheap[3]['routing'])}; mean {cheap[3]['mean_ms']:.1f} ms)."
        )

    # Per-class view of the best operating point.
    bc = by_class_acc(best[3]["rows"])
    lines += ["", "Best operating point by class: "
              + ", ".join(f"{k} {v:.2f}" for k, v in bc.items()) + ".", ""]
    args.out.write_text("\n".join(lines) + "\n")
    print(f"wrote {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
