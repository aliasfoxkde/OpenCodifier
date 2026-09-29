#!/usr/bin/env python3
"""Run the decision benchmark suite through a Jev-Style model's NATIVE interface.

The other llama arms (`run_llama.py`) score candidate ids through the fork's
tree mode — a foreign interface for Jev-Style-Decision models, whose trained
readout is per-option verdict-slot logits over the raw `macjev-render-v1`
layout (D16 recorded the mismatch: 0.217 through tree mode, 0.0 through
JSON-writing chat). This arm runs the reference runtime the authors ship
beside the weights (`jev_style_decision_gguf.py` + their `jev-score` binary,
both sha256-checked against the repo's `manifest.json`), so what is measured
is the model's own interface: logits(" yes") - logits(" no") at each option's
" ->" slot, softmax over slots with their shipped calibration temperature.

Determinism holds by construction (greedy logits, no sampling); the suite is
replayed once to confirm it end to end. Latency is client wall time per item
(render + one fused decode over all slots + socket) — same-task as the other
arms' single phase, but not byte-identical plumbing, so compare per model,
not across runtimes.

Usage:
  python3 runner/run_jev_native.py --model-dir /path/to/jev-v3-native \
      --suite suite/suite.json --out /path/to/runs/jev_native__<model>.json

`--model-dir` holds the repo files (weights, tokenizer/, readout_config.json,
manifest.json, jev_style_decision_gguf.py) and build/jev-score — see
benchmarks/decision-model/NATIVE_VERDICT_ARM.md.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import sys
import time
from pathlib import Path

RUNNER_DIR = Path(__file__).parent
sys.path.insert(0, str(RUNNER_DIR))

from run_llama import ece, metrics, sha256_file  # noqa: E402  (shared, tested helpers)

ARM = "jev_native_verdict_slot"
PROVENANCE = "chaoliangUNSW/Jev-Style-0.8B-Decision-v3-GGUF (jev_style_decision_gguf.py + jev_score)"


def load_runtime(model_dir: Path, threads: int | None):
    """Import the reference runtime from the model dir and open it (verify=True)."""
    spec = importlib.util.spec_from_file_location(
        "jev_style_decision_gguf", model_dir / "jev_style_decision_gguf.py"
    )
    if spec is None or spec.loader is None:
        raise RuntimeError(f"{model_dir}/jev_style_decision_gguf.py not found")
    mod = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = mod
    spec.loader.exec_module(mod)
    return mod.JevStyleDecisionGGUF(
        str(model_dir), quant="Q4_K_M", verify=True, threads=threads, many_mode="exact"
    )


def run_once(engine, suite: dict, items: list[dict]) -> list[dict]:
    rows = []
    for it in items:
        options = {c["id"]: c["description"] for c in it["candidates"]}
        t0 = time.monotonic()
        res = engine.decide(it["context"], it["question"], options=options, category=None)
        wall_ms = (time.monotonic() - t0) * 1000.0
        rows.append(
            {
                "id": it["id"],
                "class": it["class"],
                "answer": it["answer"],
                "pred": res["answer"],
                "prob": float(res["probabilities"][res["answer"]]),
                "wall_ms": wall_ms,
            }
        )
    return rows


def metrics_wall(rows: list[dict]) -> dict:
    """The shared metrics() helper keys latency off server_ms; this arm only
    has client wall time, so feed it through under that key."""
    fed = [{**r, "server_ms": r["wall_ms"]} for r in rows]
    return metrics(fed)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--model-dir", type=Path, required=True)
    ap.add_argument("--suite", type=Path, default=RUNNER_DIR.parent / "suite" / "suite.json")
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--threads", type=int, default=12)
    ap.add_argument("--timeout", type=float, default=600.0, help="unused; kept for driver symmetry")
    args = ap.parse_args()

    suite = json.loads(args.suite.read_text())
    items = suite["items"]
    gguf = args.model_dir / "Jev-Style-0.8B-Decision-v3-Q4_K_M.gguf"
    if not gguf.is_file():
        raise SystemExit(f"{gguf} not found: this runner pins the Q4_K_M build name")

    engine = load_runtime(args.model_dir, args.threads)
    try:
        single = run_once(engine, suite, items)
        replay = run_once(engine, suite, items)
    finally:
        engine.close()

    determinism = {
        "predictions_match": all(a["pred"] == b["pred"] for a, b in zip(single, replay)),
        "max_prob_delta": max(abs(a["prob"] - b["prob"]) for a, b in zip(single, replay)),
    }
    result = {
        "arm": ARM,
        "provenance": PROVENANCE,
        "model": {"file": gguf.name, "sha256": sha256_file(gguf)},
        "suite_sha256": hashlib.sha256(args.suite.read_bytes()).hexdigest(),
        "config": {
            "threads": args.threads,
            "interface": "native verdict-slot readout (macjev-render-v1 / macjev-readout-v1)",
            "category": None,
            "many_mode": "exact",
            "device": "cpu",
            "latency_metric": "client wall_ms (render + fused decode), not server_ms",
        },
        "readout": engine.readout_config.get("format"),
        "template": engine.readout_config.get("template"),
        "calibration": {
            "source": "shipped readout_config.json (as trained)",
            "global_temperature": engine.temperatures.get("global"),
            "category_families": len(engine.temperatures.get("groups", {})),
        },
        # The D16 tree-mode row for these same weights (foreign interface):
        # context for the interface claim, measured by run_llama.py.
        "interface_mismatch_reference": {
            "arm": "llama_decision (tree mode, candidate-id paths)",
            "accuracy": 0.2167,
            "chat_json_accuracy": 0.0,
            "source": "runs/llama__Jev-Style-0.8B-Decision-v3-Q4_K_M.json (D16)",
        },
        "caveat": (
            "Native-interface arm: no instruction/system channel exists in the "
            "Jev-Style protocol, so the suite's instructions field is unused here "
            "(it is part of the prompt in every other arm). Calibration is the "
            "authors' shipped global temperature, not our D15 fit."
        ),
        "single": single,
        "determinism": determinism,
        "metrics": metrics_wall(single),
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, sort_keys=True, indent=1) + "\n")
    m = result["metrics"]
    e, _ = ece([(r["prob"], r["pred"] == r["answer"]) for r in single])
    print(
        f"acc={m['accuracy']:.3f} ece={e:.3f} "
        f"per_class={ {k: round(v, 3) for k, v in m['accuracy_by_class'].items()} } "
        f"p50={m['latency'].get('p50_ms', 0):.0f}ms determinism={determinism['predictions_match']}",
        flush=True,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
