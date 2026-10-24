#!/usr/bin/env python3
"""#188 premise validation: does the engine gate beat a bare LLM?

ESCALATION_LANES_PLAN.md §9 — "the most important item in the
document": if the gated arm does not beat the bare-LLM arm, the lanes
and the escalation contract are decoration.

Two arms over one suite, fixed decision rule measured after both:

  Arm A (bare LLM)   the host LLM answers every item alone.
  Arm B (gated)      the engine decides first; `outcome == accept`
                     means its answer is used and the LLM is NOT called
                     (a saved call); `verify`/`abstain` escalates to the
                     same LLM with the same payload Arm A got.

  engine_sufficient  engine accepted AND correct, / all items
  engine_wrong       engine accepted AND wrong,   / all items  <- the
                     cost of the gate: this number can kill the concept
  saved_calls        engine accepted / all items (LLM calls avoided)

  Gate ships if     engine_wrong < Arm A error rate
                 AND saved_calls >= 0.20.

The escalation contract (#189, `escalate_below`, EscalationReport) does
not exist yet, so the existing accept band IS the gate — that is the
deliberate ordering in the plan: 188 can invalidate the rest, so it runs
first, against the mechanism as it stands today.

Arm A/B LLM payloads are the shipping rung contract verbatim
(LlamaDecisionClassifier: instructions, schema.choice.enum over the
declared ids, question as description, state text as the single
context, mode tree). The engine payload is run_engine.py's, byte for
byte the same policy (min 0.8 / verify 0.65 / abstain 0.5, risk low) so
engine outcomes line up with the recorded board rows.

Determinism: each arm replays once; predictions and distributions must
match bit-for-bit (the board's fork rows carry the same claim).

Usage:
  python3 runner/run_gate_ab.py \
      --engine-binary ../../target/release/opencodifier \
      --llama-server <repo>/forks/llama.cpp/build/bin/llama-server \
      --model /nas/Temp/work/oc-model-eval/models/Qwen3.5-2B-Q4_K_M.gguf \
      --out results/gate-ab-suite-rel200.json
"""

from __future__ import annotations

import argparse
import hashlib
import json
import socket
import statistics
import subprocess
import sys
import time
import urllib.error
import urllib.request
from collections import Counter
from pathlib import Path

RUNNER_DIR = Path(__file__).resolve().parent
SUITE_DEFAULT = RUNNER_DIR.parent / "suite" / "suite_holdout_rel200.json"
RESULTS_DIR = RUNNER_DIR.parent / "results"

# The shipping rung's default instructions (LlamaDecisionConfig).
LLM_INSTRUCTIONS_FALLBACK = "Select the correct option."


def post(url: str, payload: dict, timeout: float = 120.0, retries: int = 3) -> dict:
    body = json.dumps(payload).encode()
    last: Exception | None = None
    for _ in range(retries):
        try:
            req = urllib.request.Request(
                url, data=body, headers={"Content-Type": "application/json"}
            )
            with urllib.request.urlopen(req, timeout=timeout) as r:
                return json.loads(r.read())
        except (urllib.error.URLError, ConnectionError, socket.timeout) as e:
            last = e
            time.sleep(0.5)
    raise RuntimeError(f"POST {url} failed after {retries} tries: {last}")


def wait_http(url: str, proc: subprocess.Popen, deadline_s: float, what: str) -> None:
    end = time.monotonic() + deadline_s
    while time.monotonic() < end:
        if proc.poll() is not None:
            raise RuntimeError(f"{what} exited early with rc={proc.returncode}")
        try:
            with urllib.request.urlopen(url, timeout=2) as r:
                if r.status == 200:
                    return
        except (urllib.error.URLError, ConnectionError, socket.timeout):
            time.sleep(0.3)
    raise RuntimeError(f"{what} not healthy within {deadline_s}s: {url}")


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


# ---- LLM arm ---------------------------------------------------------------

def llm_payload(suite_instructions: str, it: dict) -> dict:
    """The Rust client's tree-mode payload, verbatim in shape."""
    return {
        "instructions": suite_instructions or LLM_INSTRUCTIONS_FALLBACK,
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


def llm_decide(port: int, suite_instructions: str, it: dict) -> dict:
    t0 = time.monotonic()
    resp = post(f"http://127.0.0.1:{port}/v1/decision", llm_payload(suite_instructions, it))
    wall_ms = (time.monotonic() - t0) * 1000.0
    choice = resp["results"][0]["fields"]["choice"]
    dist = {k: float(v) for k, v in choice["distribution"].items()}
    pred = max(dist, key=dist.get)
    usage = resp.get("usage", {})
    return {
        "pred": pred,
        "dist": dist,
        "prompt_tokens": int(usage.get("prompt_tokens", 0)),
        "wall_ms": wall_ms,
        "total_ms": float(resp.get("timings", {}).get("total_ms", wall_ms)),
    }


# ---- engine gate -----------------------------------------------------------

def engine_payload(it: dict) -> dict:
    """run_engine.py's payload, unchanged (policy pinned for board parity)."""
    return {
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
            "request_id": f"gate-ab-{it['id']}",
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


def engine_decide(port: int, it: dict) -> dict:
    t0 = time.monotonic()
    resp = post(f"http://127.0.0.1:{port}/v1/decide", engine_payload(it))
    wall_ms = (time.monotonic() - t0) * 1000.0
    ans = resp["answers"][0]
    entries = ans["distribution"]["entries"]
    dist = {e["key"]: float(e["probability"]) for e in entries}
    return {
        "pred": ans["choice"],
        "prob": float(ans["confidence"]),
        "dist": dist,
        "outcome": resp.get("outcome"),
        "wall_ms": wall_ms,
    }


# ---- runs ------------------------------------------------------------------

def run_arm_a(llm_port: int, suite: dict) -> list[dict]:
    rows = []
    for it in suite["items"]:
        r = llm_decide(llm_port, suite.get("instructions", ""), it)
        rows.append({"id": it["id"], "class": it["class"], "answer": it["answer"], **r})
    return rows


def run_engine(engine_port: int, suite: dict) -> list[dict]:
    rows = []
    for it in suite["items"]:
        r = engine_decide(engine_port, it)
        rows.append({"id": it["id"], "class": it["class"], "answer": it["answer"], **r})
    return rows


def run_arm_b(llm_port: int, suite: dict, engine_rows: list[dict]) -> list[dict]:
    """Gate first: accept => engine answers (saved call); else the LLM."""
    rows = []
    by_id = {r["id"]: r for r in engine_rows}
    for it in suite["items"]:
        eng = by_id[it["id"]]
        row = {
            "id": it["id"],
            "class": it["class"],
            "answer": it["answer"],
            "engine_outcome": eng["outcome"],
            "engine_pred": eng["pred"],
            "engine_prob": eng["prob"],
        }
        if eng["outcome"] == "accept":
            row.update(
                source="engine",
                pred=eng["pred"],
                prompt_tokens=0,
                wall_ms=eng["wall_ms"],
            )
        else:
            r = llm_decide(llm_port, suite.get("instructions", ""), it)
            row.update(
                source="llm",
                pred=r["pred"],
                prompt_tokens=r["prompt_tokens"],
                wall_ms=r["wall_ms"],
                dist=r["dist"],
            )
        rows.append(row)
    return rows


def accuracy(rows: list[dict]) -> float:
    return sum(r["pred"] == r["answer"] for r in rows) / len(rows)


def p50(values: list[float]) -> float:
    return statistics.median(values)


def per_class(rows: list[dict]) -> dict:
    out: dict[str, dict] = {}
    for cls in sorted({r["class"] for r in rows}):
        sub = [r for r in rows if r["class"] == cls]
        out[cls] = {"n": len(sub), "accuracy": accuracy(sub)}
    return out


def distributions_match(a: list[dict], b: list[dict]) -> bool:
    if len(a) != len(b):
        return False
    for ra, rb in zip(a, b, strict=True):
        if ra.get("pred") != rb.get("pred"):
            return False
        da, db = ra.get("dist"), rb.get("dist")
        if da is None and db is None:
            continue
        if da is None or db is None or da != db:
            return False
    return True


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--suite", type=Path, default=SUITE_DEFAULT)
    ap.add_argument("--engine-binary", type=Path,
                    default=RUNNER_DIR.parents[2] / "target" / "release" / "opencodifier")
    ap.add_argument("--llama-server", type=Path,
                    default=Path("/nas/Temp/repos/OpenCodifier-fork/forks/llama.cpp/build/bin/llama-server"))
    ap.add_argument("--model", type=Path,
                    default=Path("/nas/Temp/work/oc-model-eval/models/Qwen3.5-2B-Q4_K_M.gguf"))
    ap.add_argument("--out", type=Path, default=RESULTS_DIR / "gate-ab-suite-rel200.json")
    ap.add_argument("--no-replay", action="store_true",
                    help="skip the determinism replay of both arms")
    args = ap.parse_args()

    suite = json.loads(args.suite.read_text())
    items = suite["items"]
    print(f"suite: {args.suite.name} ({len(items)} items)")

    for p, what in ((args.engine_binary, "engine binary"),
                    (args.llama_server, "llama-server"),
                    (args.model, "model GGUF")):
        if not p.exists():
            print(f"ERROR: {what} not found: {p}", file=sys.stderr)
            return 2

    engine_port, llm_port = free_port(), free_port()
    engine_proc = llm_proc = None
    results: dict = {}
    try:
        engine_proc = subprocess.Popen(
            [str(args.engine_binary), "serve", "--bind", f"127.0.0.1:{engine_port}"],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        wait_http(f"http://127.0.0.1:{engine_port}/v1/healthz", engine_proc, 30, "engine")
        print(f"engine healthy on :{engine_port}")

        # --decision-seqs is mandatory: the fork refuses /v1/decision
        # without reserved sequences (serve.sh validates the same).
        llm_proc = subprocess.Popen(
            [str(args.llama_server), "-m", str(args.model),
             "--host", "127.0.0.1", "--port", str(llm_port),
             "-c", "2048", "-np", "1", "--decision-seqs", "3"],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        wait_http(f"http://127.0.0.1:{llm_port}/health", llm_proc, 90, "llama-server")
        print(f"llama-server healthy on :{llm_port}")

        # Arm A — bare LLM.
        t0 = time.monotonic()
        arm_a = run_arm_a(llm_port, suite)
        print(f"arm A done in {time.monotonic() - t0:.0f}s: acc={accuracy(arm_a):.4f}")

        # Engine pass — the gate's decisions (also Arm B's engine half).
        engine_rows = run_engine(engine_port, suite)
        print(f"engine pass done: {dict(Counter(r['outcome'] for r in engine_rows))}")

        # Arm B — gated.
        t0 = time.monotonic()
        arm_b = run_arm_b(llm_port, suite, engine_rows)
        print(f"arm B done in {time.monotonic() - t0:.0f}s: acc={accuracy(arm_b):.4f}")

        replay = {"ran": False}
        if not args.no_replay:
            arm_a2 = run_arm_a(llm_port, suite)
            engine2 = run_engine(engine_port, suite)
            arm_b2 = run_arm_b(llm_port, suite, engine2)
            replay = {
                "ran": True,
                "arm_a_identical": distributions_match(arm_a, arm_a2),
                "engine_identical": distributions_match(engine_rows, engine2),
                "arm_b_identical": distributions_match(arm_b, arm_b2),
            }
            print(f"replay: {replay}")

        # ---- the numbers #188 is judged on -------------------------------
        n = len(items)
        engine_accepted = [r for r in engine_rows if r["outcome"] == "accept"]
        engine_sufficient = sum(r["pred"] == r["answer"] for r in engine_accepted) / n
        engine_wrong = sum(r["pred"] != r["answer"] for r in engine_accepted) / n
        saved_calls = len(engine_accepted) / n
        arm_a_acc = accuracy(arm_a)
        arm_a_error = 1.0 - arm_a_acc
        arm_b_acc = accuracy(arm_b)
        ship = engine_wrong < arm_a_error and saved_calls >= 0.20

        llm_calls_b = [r for r in arm_b if r["source"] == "llm"]
        engine_wall = [r["wall_ms"] for r in engine_rows]
        arm_a_wall = [r["wall_ms"] for r in arm_a]
        arm_b_wall = [r["wall_ms"] for r in arm_b]

        results = {
            "meta": {
                "task": "#188 premise validation (ESCALATION_LANES_PLAN.md s9)",
                "suite": args.suite.name,
                "suite_sha256": hashlib.sha256(args.suite.read_bytes()).hexdigest(),
                "n_items": n,
                "engine_binary": str(args.engine_binary),
                "llama_server": str(args.llama_server),
                "model": args.model.name,
                "llm_instructions": suite.get("instructions") or LLM_INSTRUCTIONS_FALLBACK,
                "engine_policy": {"min_confidence": 0.8, "verify_below": 0.65,
                                  "abstain_below": 0.5, "risk": "low"},
                "date": time.strftime("%Y-%m-%d"),
                "host": "local (CPU, offline campaign 2026-10-23)",
            },
            "decision_rule": {
                "engine_wrong": engine_wrong,
                "arm_a_error_rate": arm_a_error,
                "saved_calls": saved_calls,
                "thresholds": {"engine_wrong": "< arm_a_error_rate",
                               "saved_calls": ">= 0.20"},
                "ship": ship,
            },
            "engine_sufficient": engine_sufficient,
            "arm_a": {"accuracy": arm_a_acc, "per_class": per_class(arm_a),
                      "p50_ms": p50(arm_a_wall),
                      "prompt_tokens": sum(r["prompt_tokens"] for r in arm_a)},
            "arm_b": {"accuracy": arm_b_acc, "per_class": per_class(arm_b),
                      "p50_ms": p50(arm_b_wall),
                      "llm_calls": len(llm_calls_b),
                      "prompt_tokens": sum(r["prompt_tokens"] for r in arm_b)},
            "engine": {
                "outcomes": dict(Counter(r["outcome"] for r in engine_rows)),
                "accuracy": sum(r["pred"] == r["answer"] for r in engine_rows) / n,
                "accept_accuracy": (sum(r["pred"] == r["answer"] for r in engine_accepted)
                                    / len(engine_accepted)) if engine_accepted else None,
                "p50_ms": p50(engine_wall),
            },
            "replay": replay,
            "rows": {"arm_a": arm_a, "engine": engine_rows, "arm_b": arm_b},
        }

        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(json.dumps(results, indent=1, sort_keys=True))
        print(f"\nwrote {args.out}")
        print(f"  engine outcomes      : {results['engine']['outcomes']}")
        print(f"  engine_sufficient    : {engine_sufficient:.4f}")
        print(f"  engine_wrong         : {engine_wrong:.4f}  (must be < {arm_a_error:.4f})")
        print(f"  saved_calls          : {saved_calls:.4f}  (must be >= 0.20)")
        print(f"  arm A (bare LLM) acc : {arm_a_acc:.4f}")
        print(f"  arm B (gated) acc    : {arm_b_acc:.4f}")
        print(f"  VERDICT              : {'GATE SHIPS' if ship else 'GATE DOES NOT SHIP'}")
    finally:
        for proc in (llm_proc, engine_proc):
            if proc is not None:
                proc.terminate()
                try:
                    proc.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    proc.kill()
    return 0


if __name__ == "__main__":
    sys.exit(main())
