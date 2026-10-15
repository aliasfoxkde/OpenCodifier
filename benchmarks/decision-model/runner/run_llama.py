#!/usr/bin/env python3
"""Run the decision benchmark suite through llama.cpp parallel-decision.

Drives `llama-server` (thecodacus/llama.cpp branch `parallel-decision`) in
two arms:

- decision arm: `POST /v1/decision` scores every candidate id as a token
  path forked from one cached context prefix — tree mode returns the exact
  constrained distribution, nothing is sampled, so scoring is deterministic.
- chat arm (optional): the same questions as JSON-writing chat completions
  at temperature 0 — the token-by-token baseline the decision arm replaces.
- raw lane (`--raw-prompt`): the chat questions sent to raw `POST
  /completion` with a self-rendered prompt — the server's chat template is
  never invoked. RESEARCH.md §13.1: this is the readout lane for
  template-locked models (LFM2.5's `enable_thinking` kwarg is silently
  unsupported by their template, so the chat arm enters thinking and
  starves the answer) and the settling probe for what reported logprobs
  actually contain. `--raw-suffix` appends verbatim text after the
  rendered body (pre-filled/closed thinking blocks, an answer cue);
  `--raw-top-probs N` records the first generated token's top-N reported
  distribution in each row; `--raw-grammar FILE` sends a GBNF grammar
  (format compliance only — reported logprobs stay raw-logit per §13.1).
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

from resources import ResourceMonitor

LBRANCH = "thecodacus/llama.cpp parallel-decision ad129b0"


def load_now() -> float:
    """1-minute load average — the host-regime provenance of each run.

    This host is shared by several concurrent agent sessions; accuracy is
    load-invariant (tree mode is deterministic) but every latency column is
    only comparable between runs recorded under a similar regime, so the
    regime travels with the result.
    """
    try:
        return float(Path("/proc/loadavg").read_text().split()[0])
    except (OSError, ValueError, IndexError):
        return -1.0


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
    # Fork builds after 2026-09-29 also emit the full per-choice softmax
    # (`distribution`) next to the winner-only `probability`; record it when
    # present so calibration gets native Brier/ECE instead of winner-only
    # margins. Absent (stock ad129b0 binary) the row is unchanged.
    dist = field.get("distribution")
    return {
        "value": field["value"],
        "probability": field["probability"],
        **({"probs": dist} if isinstance(dist, dict) else {}),
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
                **({"probs": r["probs"]} if "probs" in r else {}),
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


def render_raw_prompt(suite: dict, it: dict, suffix: str) -> str:
    """Self-rendered prompt for the raw /completion lane.

    Same content as `run_chat`'s messages, flattened — no chat markup, no
    server template. `suffix` is verbatim caller text (a pre-filled and
    closed thinking block, an answer cue); the model's own template is
    never consulted, which is the point of the lane (RESEARCH.md §13.1).
    """
    opts = "\n".join(f"- {c['id']}: {c['description']}" for c in it["candidates"])
    body = (
        f"{it['context']}\n\n{it['question']}\n\nOptions:\n{opts}\n\n"
        'Answer with JSON: {"id": "<one option id>"}'
    )
    return f"{suite['instructions']}\n\n{body}{suffix}"


def run_chat_raw(
    port: int,
    suite: dict,
    items: list[dict],
    max_tokens: int,
    timeout: float,
    suffix: str,
    top_probs: int,
    grammar: str | None,
) -> list[dict]:
    """Chat questions through raw /completion — the template is bypassed.

    Rows keep the `run_chat` shape (`pred` parsed from generated JSON) plus
    `top_probs`: the first generated token's reported top-N distribution
    (`completion_probabilities[0]`), the position where label logprobs are
    read in the letters protocol.
    """
    out = []
    for it in items:
        payload: dict = {
            "prompt": render_raw_prompt(suite, it, suffix),
            "temperature": 0.0,
            "n_predict": max_tokens,
            "cache_prompt": True,
        }
        if top_probs:
            payload["n_probs"] = top_probs
        if grammar is not None:
            payload["grammar"] = grammar
        t0 = time.monotonic()
        resp = post(f"http://127.0.0.1:{port}/completion", payload, timeout)
        wall_ms = (time.monotonic() - t0) * 1000.0
        text = resp.get("content") or ""
        pred = None
        try:
            pred = json.loads(text).get("id")
        except (json.JSONDecodeError, AttributeError):
            pred = None
        row = {
            "id": it["id"],
            "class": it["class"],
            "answer": it["answer"],
            "pred": pred,
            "raw": text,
            "wall_ms": wall_ms,
        }
        probs = resp.get("completion_probabilities") or []
        if top_probs and probs:
            first = probs[0].get("probs") or []
            row["top_probs"] = [
                {
                    "tok": p.get("tok_str"),
                    "logprob": p.get("logprob"),
                    "prob": p.get("prob"),
                }
                for p in first[:top_probs]
            ]
        out.append(row)
    return out


def run_chat_arm(
    args: argparse.Namespace, port: int, suite: dict, items: list[dict]
) -> dict:
    """One chat-arm pass (raw lane or /v1/chat/completions) + metrics.

    The single call site for all three entry modes (full arm, --chat-only,
    --skip-decision) so the row schema and metrics dict cannot drift
    between them.
    """
    raw = getattr(args, "raw_prompt", False)
    grammar_text = None
    if raw and getattr(args, "raw_grammar", None) is not None:
        grammar_text = args.raw_grammar.read_text()
    if raw:
        rows = run_chat_raw(
            port,
            suite,
            items,
            args.chat_max_tokens,
            args.timeout,
            args.raw_suffix,
            args.raw_top_probs,
            grammar_text,
        )
    else:
        rows = run_chat(port, suite, items, args.chat_max_tokens, args.timeout)
    return {
        "rows": rows,
        "metrics": {
            "accuracy": sum(1 for r in rows if r["pred"] == r["answer"]) / len(rows),
            "accuracy_by_class": {
                cls: sum(1 for r in rows if r["class"] == cls and r["pred"] == r["answer"])
                / sum(1 for r in rows if r["class"] == cls)
                for cls in sorted({r["class"] for r in rows})
            },
            "p50_ms": sorted(r["wall_ms"] for r in rows)[len(rows) // 2],
            "mean_ms": sum(r["wall_ms"] for r in rows) / len(rows),
            "max_tokens": args.chat_max_tokens,
            "mode": "raw_completion" if raw else "chat_completions",
            **(
                {
                    "raw_suffix": args.raw_suffix,
                    "raw_top_probs": args.raw_top_probs,
                    "raw_grammar": (args.raw_grammar.name if grammar_text is not None else None),
                }
                if raw
                else {"thinking_disabled": True}
            ),
        },
    }


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
    ap.add_argument(
        "--ngl",
        type=int,
        default=0,
        help="layers offloaded to the GPU; 0 keeps the run CPU-only "
        "(the Vulkan A/B arm passes 99 to park every layer on the iGPU)",
    )
    ap.add_argument(
        "--context-diet",
        type=int,
        default=None,
        help="hard-truncate every item's context to this many characters "
        "(the #32 prompt-diet A/B: latency win of trimming the "
        "metadata class's long contexts, paid for in accuracy); "
        "default keeps full contexts",
    )
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
        "--raw-prompt",
        action="store_true",
        help="chat arm via raw POST /completion with a self-rendered "
        "prompt — the server's chat template is never invoked "
        "(template-locked models, RESEARCH.md §13.1)",
    )
    ap.add_argument(
        "--raw-suffix",
        default="",
        help="verbatim text appended after the rendered body in the raw "
        "lane (pre-filled/closed thinking block, answer cue)",
    )
    ap.add_argument(
        "--raw-top-probs",
        type=int,
        default=0,
        help="request n_probs=N and record the first generated token's "
        "reported top-N distribution per row (0 = off)",
    )
    ap.add_argument(
        "--raw-grammar",
        type=Path,
        default=None,
        help="GBNF grammar file sent with the raw request (format "
        "compliance only; reported logprobs stay raw-logit, §13.1)",
    )
    ap.add_argument(
        "--timeout",
        type=float,
        default=600.0,
        help="per-request client timeout in seconds; raise for large models "
        "whose tail-latency requests can exceed the default",
    )
    args = ap.parse_args()
    load_start = load_now()

    suite = json.loads(args.suite.read_text())
    items = suite["items"]
    if args.context_diet is not None:
        items = [
            {**it, "context": it["context"][: args.context_diet]} for it in items
        ]
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
    cmd += ["-ngl", str(args.ngl)]
    print("spawn:", " ".join(cmd), flush=True)
    with open(args.out.with_suffix(".server.log"), "wb") as log:
        proc = subprocess.Popen(cmd, stdout=log, stderr=subprocess.STDOUT)
    monitor = ResourceMonitor()
    monitor.start()
    try:
        wait_health(args.port, proc)

        if args.skip_decision:
            chat = run_chat_arm(args, args.port, suite, items)
            result = {
                "arm": "llama_chat_baseline_only",
                "llamacpp_branch": args.llamacpp_branch,
                "model": {"file": args.model, "sha256": sha256_file(model_path)},
                "suite_sha256": hashlib.sha256(args.suite.read_bytes()).hexdigest(),
                "config": {
                    "threads": args.threads,
                    "ctx": args.ctx,
                    "ngl": args.ngl,
                    "context_diet": args.context_diet,
                    "device": "vulkan-igpu" if args.ngl > 0 else "cpu",
                    "decision_arm": False,
                    "load_avg": {"start": load_start, "end": load_now()},
                },
                "chat": chat,
            }
            monitor.stop()
            result["resources"] = monitor.report()
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
            chat = run_chat_arm(args, args.port, suite, items)
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
            chat = run_chat_arm(args, args.port, suite, items)

        result = {
            "arm": "llama_decision",
            "llamacpp_branch": LBRANCH,
            "model": {"file": args.model, "sha256": sha256_file(model_path)},
            "suite_sha256": hashlib.sha256(args.suite.read_bytes()).hexdigest(),
            "config": {
                "threads": args.threads,
                "ctx": args.ctx,
                "decision_seqs": args.decision_seqs,
                "ngl": args.ngl,
                "context_diet": args.context_diet,
                "mode": "tree",
                "device": "vulkan-igpu" if args.ngl > 0 else "cpu",
                "load_avg": {"start": load_start, "end": load_now()},
            },
            "single": single,
            "determinism": determinism,
            "batched": batched,
            "chat": chat,
            "resources": monitor.report(),
            "metrics": metrics(single),
        }
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
        monitor.stop()

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, sort_keys=True, indent=1) + "\n")
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
