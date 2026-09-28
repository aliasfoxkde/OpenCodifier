#!/usr/bin/env python3
"""Run the decision benchmark suite through llama.cpp parallel-decision.

Drives `llama-server` (thecodacus/llama.cpp branch `parallel-decision`) in
two arms:

- decision arm: `POST /v1/decision` scores every candidate id as a token
  path forked from one cached context prefix — tree mode returns the exact
  constrained distribution, nothing is sampled, so scoring is deterministic.
- chat arm (optional): the same questions as JSON-writing chat completions
  at temperature 0 — the token-by-token baseline the decision arm replaces.
- `--skip-decision`: chat-baseline-only run for forks without
  `POST /v1/decision` (e.g. MBZUAI-IFM/llama.cpp `model/K2Horizon`). The
  result carries `"arm": "llama_chat_baseline_only"` and no decision
  metrics; it is a screen, not comparable to decision-arm accuracy.

For each model the runner measures accuracy per difficulty class, expected
calibration error of the winner probability, per-decision latency single
and batched, and replays the whole suite once to confirm determinism.

Usage:
  python3 runner/run_llama.py --llama-dir /path/to/llama.cpp \
      --models-dir /path/to/models --model qwen2.5-0.5b-instruct-q4_k_m.gguf \
      --out results/llama__qwen05.json
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

LBRANCH = "thecodacus/llama.cpp parallel-decision ad129b0"


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def post(url: str, payload: dict, timeout: float = 600.0) -> dict:
    req = urllib.request.Request(
        url, data=json.dumps(payload).encode(), headers={"Content-Type": "application/json"}
    )
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read())


def wait_health(port: int, proc: subprocess.Popen, deadline_s: float = 300.0) -> None:
    end = time.monotonic() + deadline_s
    while time.monotonic() < end:
        if proc.poll() is not None:
            raise RuntimeError(f"llama-server exited early with {proc.returncode}")
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=2) as r:
                if r.status == 200:
                    return
        except (urllib.error.URLError, ConnectionError, socket.timeout):
            time.sleep(0.5)
    raise RuntimeError("llama-server did not become healthy in time")


def decide_single(port: int, suite: dict, it: dict, timeout: float) -> dict:
    payload = {
        "instructions": suite["instructions"],
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
    t0 = time.monotonic()
    resp = post(f"http://127.0.0.1:{port}/v1/decision", payload, timeout)
    wall_ms = (time.monotonic() - t0) * 1000.0
    res = resp["results"][0]
    field = res["fields"]["choice"]
    usage = resp.get("usage", {}) or res.get("usage", {})
    return {
        "value": field["value"],
        "probability": field["probability"],
        "server_ms": resp.get("timings", {}).get("per_decision_ms"),
        "prefill_ms": resp.get("timings", {}).get("prefill_ms"),
        "cached_tokens": usage.get("cached_tokens"),
        "context_tokens": usage.get("context_tokens"),
        "wall_ms": wall_ms,
    }


def run_suite(port: int, suite: dict, items: list[dict], timeout: float) -> list[dict]:
    out = []
    for it in items:
        r = decide_single(port, suite, it, timeout)
        out.append(
            {
                "id": it["id"],
                "class": it["class"],
                "answer": it["answer"],
                "pred": r["value"],
                "prob": r["probability"],
                "server_ms": r["server_ms"],
                "prefill_ms": r["prefill_ms"],
                "cached_tokens": r["cached_tokens"],
                "context_tokens": r["context_tokens"],
            }
        )
    return out


def run_batched(port: int, suite: dict, items: list[dict], timeout: float) -> dict:
    """Bulk phase: all contexts of one class in a single call.

    Contexts in one call must share the schema, so this only works when the
    candidate set is identical across the batch; per class the union of ids
    is used, which means the scored candidate set is wider than per item.
    Reported separately: it measures throughput, not the same task as the
    single phase. Items whose per-item candidate set differs from the union
    are still scored against the union (an easier task for extra choices).
    """
    by_class: dict[str, list[dict]] = {}
    for it in items:
        by_class.setdefault(it["class"], []).append(it)
    out = {"per_class": {}}
    for cls, cls_items in sorted(by_class.items()):
        union = sorted({c["id"] for it in cls_items for c in it["candidates"]})
        payload = {
            "instructions": suite["instructions"],
            "schema": {
                "choice": {"type": "enum", "choices": union, "description": "The chosen option id"}
            },
            "contexts": [it["context"] for it in cls_items],
            "mode": "tree",
        }
        t0 = time.monotonic()
        resp = post(f"http://127.0.0.1:{port}/v1/decision", payload, timeout)
        wall_ms = (time.monotonic() - t0) * 1000.0
        timings = resp.get("timings", {})
        preds = [r["fields"]["choice"]["value"] for r in resp["results"]]
        correct = sum(
            1 for it, p in zip(cls_items, preds) if p == it["answer"]
        )
        out["per_class"][cls] = {
            "n": len(cls_items),
            "union_candidates": len(union),
            "accuracy_union_task": correct / len(cls_items),
            "prefill_ms": timings.get("prefill_ms"),
            "scoring_ms": timings.get("scoring_ms"),
            "total_ms": timings.get("total_ms"),
            "per_decision_ms": timings.get("per_decision_ms"),
            "wall_ms": wall_ms,
            "rounds": timings.get("rounds"),
        }
    return out


def run_chat(
    port: int, suite: dict, items: list[dict], max_tokens: int, timeout: float
) -> list[dict]:
    out = []
    for it in items:
        opts = "\n".join(f"- {c['id']}: {c['description']}" for c in it["candidates"])
        user = (
            f"{it['context']}\n\n{it['question']}\n\nOptions:\n{opts}\n\n"
            'Answer with JSON: {"id": "<one option id>"}'
        )
        payload = {
            "messages": [
                {"role": "system", "content": suite["instructions"]},
                {"role": "user", "content": user},
            ],
            "temperature": 0,
            "max_tokens": max_tokens,
            "response_format": {"type": "json_object"},
            # Thinking-mode templates (Qwen3.5, MiniCPM5) otherwise spend the
            # whole budget on reasoning_content and return empty content.
            "chat_template_kwargs": {"enable_thinking": False},
        }
        t0 = time.monotonic()
        resp = post(f"http://127.0.0.1:{port}/v1/chat/completions", payload, timeout)
        wall_ms = (time.monotonic() - t0) * 1000.0
        text = resp["choices"][0]["message"]["content"]
        pred = None
        try:
            pred = json.loads(text).get("id")
        except (json.JSONDecodeError, AttributeError):
            pred = None
        out.append(
            {
                "id": it["id"],
                "class": it["class"],
                "answer": it["answer"],
                "pred": pred,
                "raw": text,
                "wall_ms": wall_ms,
            }
        )
    return out


def ece(pairs: list[tuple[float, int]], bins: int = 10) -> tuple[float, list[dict]]:
    binned = [ { "lo": i / bins, "hi": (i + 1) / bins, "n": 0, "conf": 0.0, "acc": 0.0 } for i in range(bins) ]
    for p, ok in pairs:
        idx = min(bins - 1, int(p * bins))
        b = binned[idx]
        b["n"] += 1
        b["conf"] += p
        b["acc"] += 1 if ok else 0
    total = len(pairs)
    err = 0.0
    for b in binned:
        if b["n"]:
            b["conf"] /= b["n"]
            b["acc"] /= b["n"]
            err += (b["n"] / total) * abs(b["acc"] - b["conf"])
    return err, binned


def metrics(rows: list[dict]) -> dict:
    acc_pairs = [(r["prob"], r["pred"] == r["answer"]) for r in rows]
    e, bins = ece(acc_pairs)
    by_class = {}
    for cls in sorted({r["class"] for r in rows}):
        sub = [r for r in rows if r["class"] == cls]
        by_class[cls] = sum(1 for r in sub if r["pred"] == r["answer"]) / len(sub)
    ms = sorted(r["server_ms"] for r in rows if r["server_ms"] is not None)
    lat = (
        {
            "p50_ms": ms[len(ms) // 2],
            "p95_ms": ms[int(len(ms) * 0.95)],
            "mean_ms": sum(ms) / len(ms),
        }
        if ms
        else {}
    )
    return {
        "accuracy": sum(1 for r in rows if r["pred"] == r["answer"]) / len(rows),
        "accuracy_by_class": by_class,
        "ece": e,
        "ece_bins": bins,
        "latency": lat,
        "n": len(rows),
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--llama-dir", type=Path, required=True)
    ap.add_argument("--models-dir", type=Path, required=True)
    ap.add_argument("--model", required=True)
    ap.add_argument("--suite", type=Path, default=Path(__file__).parent.parent / "suite" / "suite.json")
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--port", type=int, default=8391)
    ap.add_argument("--threads", type=int, default=12)
    ap.add_argument("--ctx", type=int, default=8192)
    ap.add_argument("--decision-seqs", type=int, default=24)
    ap.add_argument("--build-dir", default="build-pd", help="build subdir under --llama-dir holding bin/llama-server")
    ap.add_argument("--llamacpp-branch", default=LBRANCH, help="fork/branch provenance recorded in the result")
    ap.add_argument("--skip-chat", action="store_true")
    ap.add_argument(
        "--skip-decision",
        action="store_true",
        help="chat-baseline-only screen for forks without POST /v1/decision",
    )
    ap.add_argument("--skip-batched", action="store_true")
    ap.add_argument(
        "--chat-only",
        action="store_true",
        help="re-run only the chat baseline, merging into the existing out file",
    )
    ap.add_argument("--chat-max-tokens", type=int, default=512)
    ap.add_argument(
        "--timeout",
        type=float,
        default=600.0,
        help="per-request client timeout in seconds; raise for large models "
        "whose tail-latency requests can exceed the default",
    )
    args = ap.parse_args()

    suite = json.loads(args.suite.read_text())
    items = suite["items"]
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
    ]
    if not args.skip_decision:
        cmd += ["--decision-seqs", str(args.decision_seqs)]
    cmd += ["-ngl", "0"]
    print("spawn:", " ".join(cmd), flush=True)
    with open(args.out.with_suffix(".server.log"), "wb") as log:
        proc = subprocess.Popen(cmd, stdout=log, stderr=subprocess.STDOUT)
    try:
        wait_health(args.port, proc)

        if args.skip_decision:
            chat_rows = run_chat(args.port, suite, items, args.chat_max_tokens, args.timeout)
            chat = {
                "rows": chat_rows,
                "metrics": {
                    "accuracy": sum(1 for r in chat_rows if r["pred"] == r["answer"])
                    / len(chat_rows),
                    "accuracy_by_class": {
                        cls: sum(1 for r in chat_rows if r["class"] == cls and r["pred"] == r["answer"])
                        / sum(1 for r in chat_rows if r["class"] == cls)
                        for cls in sorted({r["class"] for r in chat_rows})
                    },
                    "p50_ms": sorted(r["wall_ms"] for r in chat_rows)[len(chat_rows) // 2],
                    "mean_ms": sum(r["wall_ms"] for r in chat_rows) / len(chat_rows),
                    "max_tokens": args.chat_max_tokens,
                    "thinking_disabled": True,
                },
            }
            result = {
                "arm": "llama_chat_baseline_only",
                "llamacpp_branch": args.llamacpp_branch,
                "model": {"file": args.model, "sha256": sha256_file(model_path)},
                "suite_sha256": hashlib.sha256(args.suite.read_bytes()).hexdigest(),
                "config": {
                    "threads": args.threads,
                    "ctx": args.ctx,
                    "ngl": 0,
                    "device": "cpu",
                    "decision_arm": False,
                },
                "chat": chat,
            }
            args.out.write_text(json.dumps(result, sort_keys=True, indent=1) + "\n")
            cm = chat["metrics"]
            print(
                f"chat-only acc={cm['accuracy']:.3f} "
                f"per_class={ {k: round(v, 3) for k, v in cm['accuracy_by_class'].items()} } "
                f"p50={cm['p50_ms']:.0f}ms (decision arm skipped: fork has no /v1/decision)",
                flush=True,
            )
            return 0

        if args.chat_only:
            prior_path = args.out
            prior = json.loads(prior_path.read_text()) if prior_path.exists() else None
            chat_rows = run_chat(args.port, suite, items, args.chat_max_tokens, args.timeout)
            chat = {
                "rows": chat_rows,
                "metrics": {
                    "accuracy": sum(1 for r in chat_rows if r["pred"] == r["answer"])
                    / len(chat_rows),
                    "accuracy_by_class": {
                        cls: sum(1 for r in chat_rows if r["class"] == cls and r["pred"] == r["answer"])
                        / sum(1 for r in chat_rows if r["class"] == cls)
                        for cls in sorted({r["class"] for r in chat_rows})
                    },
                    "p50_ms": sorted(r["wall_ms"] for r in chat_rows)[len(chat_rows) // 2],
                    "mean_ms": sum(r["wall_ms"] for r in chat_rows) / len(chat_rows),
                    "max_tokens": args.chat_max_tokens,
                    "thinking_disabled": True,
                },
            }
            if prior is None:
                raise RuntimeError("--chat-only requires an existing result file to merge into")
            prior["chat"] = chat
            prior_path.write_text(json.dumps(prior, sort_keys=True, indent=1) + "\n")
            cm = chat["metrics"]
            print(
                f"chat acc={cm['accuracy']:.3f} p50={cm['p50_ms']:.0f}ms (merged into {prior_path})",
                flush=True,
            )
            return 0

        single = run_suite(args.port, suite, items, args.timeout)
        single_again = run_suite(args.port, suite, items, args.timeout)
        max_delta = max(
            abs(a["prob"] - b["prob"]) for a, b in zip(single, single_again)
        )
        determinism = {
            "predictions_match": all(
                a["pred"] == b["pred"] for a, b in zip(single, single_again)
            ),
            "max_prob_delta": max_delta,
        }

        batched = (
            run_batched(args.port, suite, items, args.timeout)
            if not args.skip_batched
            else None
        )
        chat = None
        if not args.skip_chat:
            chat_rows = run_chat(args.port, suite, items, args.chat_max_tokens, args.timeout)
            chat = {
                "rows": chat_rows,
                "metrics": {
                    "accuracy": sum(1 for r in chat_rows if r["pred"] == r["answer"])
                    / len(chat_rows),
                    "accuracy_by_class": {
                        cls: sum(1 for r in chat_rows if r["class"] == cls and r["pred"] == r["answer"])
                        / sum(1 for r in chat_rows if r["class"] == cls)
                        for cls in sorted({r["class"] for r in chat_rows})
                    },
                    "p50_ms": sorted(r["wall_ms"] for r in chat_rows)[len(chat_rows) // 2],
                    "mean_ms": sum(r["wall_ms"] for r in chat_rows) / len(chat_rows),
                },
            }

        result = {
            "arm": "llama_decision",
            "llamacpp_branch": LBRANCH,
            "model": {"file": args.model, "sha256": sha256_file(model_path)},
            "suite_sha256": hashlib.sha256(args.suite.read_bytes()).hexdigest(),
            "config": {
                "threads": args.threads,
                "ctx": args.ctx,
                "decision_seqs": args.decision_seqs,
                "ngl": 0,
                "mode": "tree",
                "device": "cpu",
            },
            "single": single,
            "determinism": determinism,
            "batched": batched,
            "chat": chat,
            "metrics": metrics(single),
        }
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, sort_keys=True, indent=1) + "\n")
    m = result["metrics"]
    m = result["metrics"]
    print(
        f"acc={m['accuracy']:.3f} ece={m['ece']:.3f} "
        f"per_class={ {k: round(v, 3) for k, v in m['accuracy_by_class'].items()} } "
        f"determinism={determinism['predictions_match']}",
        flush=True,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
