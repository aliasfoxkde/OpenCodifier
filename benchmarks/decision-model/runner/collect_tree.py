#!/usr/bin/env python3
"""Collect per-item tree-readout rows under the engine rung's exact contract.

The ladder gate refit (F27 follow-up) needs the model rung's tree
distributions for EVERY suite item; a fusion run only exercises the
escalated subset. This collector posts the same payload the D26 model rung
sends — `llamacpp.rs`'s DEFAULT_INSTRUCTIONS ("Select the correct
option."), candidate ids as choices, contexts verbatim, `mode: "tree"` —
so fitted gates see the confidence shape the engine actually serves.
(`run_llama.py` posts the suite's own instruction header instead; close,
but not the contract the rung ships.)

One pass plus a full replay; tree mode is deterministic, so rows must
match exactly. Records the full per-choice `distribution` beside the
winner when the fork build emits it (post-2026-09-29 builds do).

Usage:
  python3 runner/collect_tree.py \
      --model gemma-4-E2B-it-QAT-Q4_0.gguf \
      --llama-dir ~/oc-model-eval/llama.cpp \
      --out results/tree__e2bqat-q4_0-pd.json
"""

from __future__ import annotations

import argparse
import hashlib
import json
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from resources import ResourceMonitor  # noqa: E402
from run_llama import sha256_file, wait_health  # noqa: E402

# The engine rung's contract — keep byte-equal to llamacpp.rs
# DEFAULT_INSTRUCTIONS; a drift here silently re-shapes every fitted gate.
INSTRUCTIONS = "Select the correct option."


def decide(port: int, it: dict, timeout: float) -> dict:
    payload = {
        "instructions": INSTRUCTIONS,
        "schema": {
            "choice": {
                "type": "enum",
                "choices": [c["id"] for c in it["candidates"]],
                "description": it["question"],
            }
        },
        "contexts": [it["context"]],
        "mode": "tree",
    }
    req = urllib.request.Request(
        f"http://127.0.0.1:{port}/v1/decision",
        data=json.dumps(payload).encode(),
        headers={"Content-Type": "application/json"},
    )
    t0 = time.monotonic()
    with urllib.request.urlopen(req, timeout=timeout) as r:
        resp = json.loads(r.read())
    wall_ms = (time.monotonic() - t0) * 1000.0
    field = resp["results"][0]["fields"]["choice"]
    dist = field.get("distribution")
    return {
        "pred": field["value"],
        "prob": float(field["probability"]),
        **({"probs": dist} if isinstance(dist, dict) else {}),
        "wall_ms": wall_ms,
    }


def run_once(port: int, items: list[dict], timeout: float) -> list[dict]:
    rows = []
    for it in items:
        r = decide(port, it, timeout)
        rows.append({"id": it["id"], "class": it["class"], "answer": it["answer"], **r})
    return rows


def ece(pairs: list[tuple[float, int]], bins: int = 10) -> float:
    err = 0.0
    total = len(pairs)
    for i in range(bins):
        lo, hi = i / bins, (i + 1) / bins
        sub = [(p, ok) for p, ok in pairs if lo <= p < hi or (i == bins - 1 and p == hi)]
        if sub:
            conf = sum(p for p, _ in sub) / len(sub)
            acc = sum(1 for _, ok in sub if ok) / len(sub)
            err += (len(sub) / total) * abs(acc - conf)
    return err


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--model", required=True)
    ap.add_argument("--models-dir", type=Path, required=True)
    ap.add_argument("--llama-dir", type=Path, required=True)
    ap.add_argument("--build-dir", default="build-pd")
    ap.add_argument("--suite", type=Path, default=Path(__file__).parent.parent / "suite" / "suite.json")
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--port", type=int, default=8091)
    ap.add_argument("--threads", type=int, default=16)
    ap.add_argument("--ctx", type=int, default=4096)
    ap.add_argument("--decision-seqs", type=int, default=24)
    ap.add_argument("--timeout", type=float, default=600.0)
    args = ap.parse_args()

    suite = json.loads(args.suite.read_text())
    model_path = args.models_dir / args.model
    args.out.parent.mkdir(parents=True, exist_ok=True)
    cmd = [
        str(args.llama_dir / args.build_dir / "bin" / "llama-server"),
        "-m", str(model_path),
        "--port", str(args.port),
        "-c", str(args.ctx),
        "-fa", "on",
        "-t", str(args.threads),
        "--jinja",
        "--parallel", "1",
        "--decision-seqs", str(args.decision_seqs),
        "-ngl", "0",
    ]
    print("spawn:", " ".join(cmd), flush=True)
    with open(args.out.with_suffix(".server.log"), "wb") as log:
        proc = subprocess.Popen(cmd, stdout=log, stderr=subprocess.STDOUT)
    monitor = ResourceMonitor()
    monitor.start()
    try:
        wait_health(args.port, proc)
        rows = run_once(args.port, suite["items"], args.timeout)
        rows2 = run_once(args.port, suite["items"], args.timeout)
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
        monitor.stop()
        resources = monitor.report()

    determinism = {
        "predictions_match": all(a["pred"] == b["pred"] for a, b in zip(rows, rows2)),
        "max_prob_delta": max(abs(a["prob"] - b["prob"]) for a, b in zip(rows, rows2)),
    }
    by_class = {}
    for cls in sorted({r["class"] for r in rows}):
        sub = [r for r in rows if r["class"] == cls]
        by_class[cls] = sum(1 for r in sub if r["pred"] == r["answer"]) / len(sub)
    acc = sum(1 for r in rows if r["pred"] == r["answer"]) / len(rows)
    ms = sorted(r["wall_ms"] for r in rows)
    result = {
        "arm": "tree_collect",
        "model": {"file": args.model, "sha256": sha256_file(model_path)},
        "suite_sha256": hashlib.sha256(args.suite.read_bytes()).hexdigest(),
        "config": {
            "instructions": INSTRUCTIONS,
            "contract": "llamacpp.rs DEFAULT_INSTRUCTIONS (engine rung payload)",
            "threads": args.threads,
            "ctx": args.ctx,
            "decision_seqs": args.decision_seqs,
            "ngl": 0,
            "mode": "tree",
            "host": socket.gethostname(),
        },
        "single": rows,
        "determinism": determinism,
        "resources": resources,
        "metrics": {
            "accuracy": acc,
            "accuracy_by_class": by_class,
            "ece": ece([(r["prob"], r["pred"] == r["answer"]) for r in rows]),
            "p50_ms": ms[len(ms) // 2],
            "mean_ms": sum(ms) / len(ms),
        },
    }
    args.out.write_text(json.dumps(result, sort_keys=True, indent=1) + "\n")
    print(
        f"acc={acc:.3f} ece={result['metrics']['ece']:.3f} "
        f"per_class={ {k: round(v, 3) for k, v in by_class.items()} } "
        f"p50={result['metrics']['p50_ms']:.1f}ms "
        f"determinism={determinism['predictions_match']}",
        flush=True,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
