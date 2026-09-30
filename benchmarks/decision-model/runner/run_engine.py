#!/usr/bin/env python3
"""Run the decision benchmark suite through OpenCodifier's own engine.

Spawns `opencodifier serve` with the built-in default pipeline (lexical /
BM25 candidate scoring — the cheapest reliable layers of the escalation
ladder, no ML) and drives each suite item through `POST /v1/decide` as a
canonical Choice question. This arm is the in-repo baseline: it measures
how far deterministic lexical machinery gets on the same tasks the model
arms are scored on.

Replays the suite once to confirm determinism; the engine is a sync,
deterministic runtime, so predictions and distributions must match.

Usage:
  python3 runner/run_engine.py --binary ../../target/release/opencodifier \
      --out results/engine__builtin-lexical.json
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

from resources import ResourceMonitor


def post(url: str, payload: dict, timeout: float = 120.0) -> dict:
    req = urllib.request.Request(
        url, data=json.dumps(payload).encode(), headers={"Content-Type": "application/json"}
    )
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read())


def wait_health(port: int, proc: subprocess.Popen, deadline_s: float = 60.0) -> dict:
    end = time.monotonic() + deadline_s
    while time.monotonic() < end:
        if proc.poll() is not None:
            raise RuntimeError(f"opencodifier serve exited early with {proc.returncode}")
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{port}/v1/healthz", timeout=2) as r:
                if r.status == 200:
                    return json.loads(r.read())
        except (urllib.error.URLError, ConnectionError, socket.timeout):
            time.sleep(0.2)
    raise RuntimeError("opencodifier serve did not become healthy in time")


def decide(port: int, suite: dict, it: dict) -> dict:
    payload = {
        "state": {"text": it["context"], "facts": {}},
        "questions": [
            {
                "type": "choice",
                "id": "decision",
                "text": it["question"],
                "candidates": it["candidates"],
            }
        ],
        "policy": {
            "min_confidence": 0.8,
            "verify_below": 0.65,
            "abstain_below": 0.5,
            "risk": "low",
        },
        "metadata": {
            "request_id": f"bench-{it['id']}",
            "limits": {
                "max_input_bytes": 1048576,
                "max_questions": 32,
                "max_candidates": 256,
                "max_graph_nodes": 128,
                "max_execution_time": {"secs": 10, "nanos": 0},
                "max_retrieval_results": 64,
            },
        },
    }
    t0 = time.monotonic()
    resp = post(f"http://127.0.0.1:{port}/v1/decide", payload)
    wall_ms = (time.monotonic() - t0) * 1000.0
    ans = resp["answers"][0]
    return {
        "choice": ans["choice"],
        "prob": ans["confidence"],
        "outcome": resp.get("outcome"),
        "wall_ms": wall_ms,
    }


def run_once(port: int, suite: dict) -> list[dict]:
    rows = []
    for it in suite["items"]:
        r = decide(port, suite, it)
        rows.append(
            {
                "id": it["id"],
                "class": it["class"],
                "answer": it["answer"],
                "pred": r["choice"],
                "prob": r["prob"],
                "outcome": r["outcome"],
                "wall_ms": r["wall_ms"],
            }
        )
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
    ap = argparse.ArgumentParser()
    ap.add_argument("--binary", type=Path, required=True)
    ap.add_argument("--suite", type=Path, default=Path(__file__).parent.parent / "suite" / "suite.json")
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--port", type=int, default=8177)
    ap.add_argument(
        "--focus-budget",
        type=int,
        default=None,
        help="pass --focus-budget to `serve`: decide on focused views of at "
        "most this many estimated tokens (the A/B arm for PLANNING §45 "
        "focused extraction)",
    )
    args = ap.parse_args()

    suite = json.loads(args.suite.read_text())
    args.out.parent.mkdir(parents=True, exist_ok=True)
    cmd = [str(args.binary), "serve", "--bind", f"127.0.0.1:{args.port}"]
    if args.focus_budget is not None:
        cmd += ["--focus-budget", str(args.focus_budget)]
    with open(args.out.with_suffix(".server.log"), "wb") as log:
        proc = subprocess.Popen(cmd, stdout=log, stderr=subprocess.STDOUT)
    monitor = ResourceMonitor()
    monitor.start()
    try:
        health = wait_health(args.port, proc)
        model_id = health["identity"]["model_id"]
        rows = run_once(args.port, suite)
        rows2 = run_once(args.port, suite)
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
    lat = {
        "p50_ms": ms[len(ms) // 2],
        "p95_ms": ms[int(len(ms) * 0.95)],
        "mean_ms": sum(ms) / len(ms),
    }
    outcomes = {}
    for r in rows:
        outcomes[r["outcome"]] = outcomes.get(r["outcome"], 0) + 1

    result = {
        # The engine's live identity names the arm (e.g. the relational
        # solver composes to `relational-v1|builtin-lexical-v1`), so the
        # run JSON can never mislabel the deciding stack.
        "arm": f"engine__{model_id}",
        "model": {"name": f"{model_id} (no ML)"},
        "suite_sha256": hashlib.sha256(args.suite.read_bytes()).hexdigest(),
        "single": rows,
        "determinism": determinism,
        "resources": resources,
        "metrics": {
            "accuracy": acc,
            "accuracy_by_class": by_class,
            "ece": ece([(r["prob"], r["pred"] == r["answer"]) for r in rows]),
            "latency": lat,
            "outcomes": outcomes,
        },
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, sort_keys=True, indent=1) + "\n")
    print(
        f"acc={acc:.3f} ece={result['metrics']['ece']:.3f} "
        f"per_class={ {k: round(v, 3) for k, v in by_class.items()} } "
        f"p50={lat['p50_ms']:.2f}ms determinism={determinism['predictions_match']}",
        flush=True,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
