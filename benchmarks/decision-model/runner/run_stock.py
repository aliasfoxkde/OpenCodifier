#!/usr/bin/env python3
"""Run the decision benchmark suite through stock llama.cpp endpoints.

Stock (upstream) llama-server exposes no `/v1/decision` tree readout, so
this runner implements the two readouts that work on stock endpoints:

- `letters`: one chat completion per item whose options are lettered
  A..N; `logit_bias` pins the letter token ids and `top_logprobs`
  returns the next-token distribution, renormalized over the letters.
  This is the readout letter-trained deciders (Winnow-E4B) were built
  for, and the cheapest screen for any chat model. The renormalized
  letter distribution is the constrained softmax — the +100 bias shifts
  every letter equally, so pairwise differences survive; non-letter
  tokens sit >100 logits down and never appear in the top list.
- `paths`: per-candidate teacher-forced scoring. Each candidate id is
  appended to the rendered prompt and scored with `prompt_logprobs`; a
  softmax over the summed candidate logprobs is the exact analog of the
  fork's token-path distribution and the basis of the parity diff
  against `run_llama.py` decision rows (`--compare-fork`). Rows whose
  prefix-token alignment fails are recorded invalid, never repaired.
  Requires a server build that returns per-prompt-token logprobs — the
  2026-10 restructured upstream server dropped `prompt_logprobs`, so the
  runner probes once and refuses loudly rather than emitting an
  all-invalid run. On such builds the parity question is answered with
  `letters` on the same model instead: stock constrained-letter accuracy
  vs the fork decision arm's accuracy, with `--compare-fork` for winner
  agreement.

`--permutations K` replays the suite under K deterministic candidate
orders (seeded per item), which is the instrument behind the position-
prior findings: rows carry the answer's position in the listing and
metrics decompose by it. The parity lane wants `paths` (or `letters`
where paths is unsupported) + a fork result; letter-trained models want
`letters`.

Usage:
  python3 runner/run_stock.py --llama-dir /path/to/llama.cpp \
      --build-dir build-stock --models-dir /path/to/models \
      --model Winnow-E4B-Q8_0.gguf --readout letters \
      --out results/stock__winnow-e4b.json
  python3 runner/run_stock.py ... --readout paths \
      --compare-fork results/llama__jebadiah-9b-v2-Q8_0.json \
      --out results/parity__jebadiah-9b-v2.json
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import random
import string
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from run_llama import (  # noqa: E402
    load_now,
    post,
    sha256_file,
    wait_health,
)
from resources import ResourceMonitor  # noqa: E402

LETTERS = string.ascii_uppercase


def letter_ids(port: int, count: int) -> list[int]:
    """Token id of each option letter, via the server's /tokenize."""
    ids = []
    for ch in LETTERS[:count]:
        resp = post(f"http://127.0.0.1:{port}/tokenize", {"content": ch})
        tokens = resp["tokens"]
        if len(tokens) != 1:
            raise RuntimeError(f"letter {ch!r} tokenized to {len(tokens)} tokens")
        ids.append(tokens[0])
    return ids


def softmax(xs: list[float]) -> list[float]:
    m = max(xs)
    ws = [math.exp(x - m) for x in xs]
    z = sum(ws)
    return [w / z for w in ws]


def permute(candidates: list[dict], perm: int, seed: int, item_id: str) -> list[dict]:
    """Deterministic candidate order for pass `perm` (0 = suite order)."""
    if perm == 0:
        return list(candidates)
    rng = random.Random(f"{seed}:{item_id}:{perm}")
    out = list(candidates)
    rng.shuffle(out)
    return out


def letters_prompt(suite: dict, it: dict, candidates: list[dict]) -> list[dict]:
    opts = "\n".join(
        f"{LETTERS[i]}. {c['description']}" for i, c in enumerate(candidates)
    )
    user = f"{it['context']}\n\n{it['question']}\n\nOptions:\n{opts}\n\nAnswer with the letter only."
    return [
        {"role": "system", "content": suite["instructions"]},
        {"role": "user", "content": user},
    ]


def first_token_tops(choice: dict) -> list[dict]:
    """(token, logprob) pairs for the first generated token.

    llama.cpp shapes varied across builds: OpenAI chat style
    (`logprobs.content[0].top_logprobs`), completions style
    (`top_logprobs` as {token: logprob} dicts), or a bare list of
    floats. Normalize all three; empty list when unreadable.
    """
    lp = choice.get("logprobs")
    if isinstance(lp, dict):
        content = lp.get("content")
        if isinstance(content, list) and content:
            first = content[0]
            if isinstance(first, dict):
                return [
                    {"token": e["token"], "logprob": e["logprob"]}
                    for e in first.get("top_logprobs", [])
                ]
    if isinstance(lp, dict):
        tops = lp.get("top_logprobs")
        if isinstance(tops, list) and tops and isinstance(tops[0], dict):
            return [
                {"token": t, "logprob": v} for t, v in tops[0].items()
            ]
    if isinstance(lp, list) and lp and isinstance(lp[0], dict):
        tops = lp[0].get("top_logprobs")
        if isinstance(tops, list) and tops and isinstance(tops[0], dict):
            return [{"token": t, "logprob": v} for t, v in tops[0].items()]
    return []


def decide_letters(
    port: int, suite: dict, it: dict, candidates: list[dict], timeout: float
) -> dict:
    """Letter-constrained readout: argmax letter + constrained softmax."""
    n = len(candidates)
    payload = {
        "messages": letters_prompt(suite, it, candidates),
        "temperature": 0,
        "max_tokens": 1,
        "logprobs": True,
        "top_logprobs": 20,
        "logit_bias": {str(tid): 100.0 for tid in letter_ids(port, n)},
        "chat_template_kwargs": {"enable_thinking": False},
    }
    t0 = time.monotonic()
    resp = post(f"http://127.0.0.1:{port}/v1/chat/completions", payload, timeout)
    wall_ms = (time.monotonic() - t0) * 1000.0
    tops = first_token_tops(resp["choices"][0])
    if not tops:
        return {
            "value": None,
            "probability": 0.0,
            "invalid_distribution": True,
            "missing_letters": ["<unreadable logprobs shape>"],
            "wall_ms": wall_ms,
        }
    by_letter = {}
    for entry in tops:
        tok = entry["token"].strip()
        if tok in LETTERS[:n]:
            by_letter[tok] = entry["logprob"]
    missing = [LETTERS[i] for i in range(n) if LETTERS[i] not in by_letter]
    if missing:
        return {
            "value": None,
            "probability": 0.0,
            "invalid_distribution": True,
            "missing_letters": missing,
            "wall_ms": wall_ms,
        }
    letters = LETTERS[:n]
    ps = softmax([by_letter[ch] for ch in letters])
    probs = {candidates[i]["id"]: ps[i] for i in range(n)}
    winner = max(probs, key=probs.get)
    return {
        "value": winner,
        "probability": probs[winner],
        "probs": probs,
        "invalid_distribution": False,
        "wall_ms": wall_ms,
        "server_ms": resp.get("timings", {}).get("prompt_ms"),
    }


def render_prompt(port: int, messages: list[dict], timeout: float) -> tuple[str, str]:
    """Chat-template rendering via POST /template, raw fallback.

    Returns (prompt_text, render_mode). The paths readout needs the
    rendered string because /v1/completions scores a raw prompt; when the
    server has no /template route the raw concatenation is used and the
    result records it (a raw prompt on an instruct model is a different
    operating point, so parity claims are only made render=jinja).
    """
    try:
        resp = post(f"http://127.0.0.1:{port}/template", {"messages": messages}, 30.0)
        return resp["prompt"], "jinja"
    except Exception:
        text = ""
        for m in messages:
            text += f"{m['role']}: {m['content']}\n"
        return text, "raw_fallback"


def token_span(port: int, prefix: str, full: str) -> tuple[list[int], int] | None:
    """Tokens of `full` and the token offset where the suffix starts.

    None when tokenization is not prefix-stable (the candidate suffix
    merged across the boundary); the caller records the row invalid
    rather than guessing a span.
    """
    tp = post(f"http://127.0.0.1:{port}/tokenize", {"content": prefix})
    tf = post(f"http://127.0.0.1:{port}/tokenize", {"content": full})
    pre, ful = tp["tokens"], tf["tokens"]
    if ful[: len(pre)] != pre:
        return None
    return ful, len(pre)


def decide_paths(
    port: int,
    suite: dict,
    it: dict,
    candidates: list[dict],
    rendered: str,
    timeout: float,
) -> dict:
    """Teacher-forced per-candidate scoring: softmax over summed logprobs."""
    totals: dict[str, float] = {}
    invalid = 0
    t0 = time.monotonic()
    for c in candidates:
        full = rendered + c["id"]
        span = token_span(port, rendered, full)
        if span is None:
            totals[c["id"]] = -math.inf
            invalid += 1
            continue
        tokens, offset = span
        resp = post(
            f"http://127.0.0.1:{port}/v1/completions",
            {
                "prompt": full,
                "max_tokens": 1,
                "temperature": 0,
                "prompt_logprobs": 0,
                "cache_prompt": True,
            },
            timeout,
        )
        plp = resp["choices"][0].get("prompt_logprobs")
        if not isinstance(plp, list) or len(plp) != len(tokens):
            totals[c["id"]] = -math.inf
            invalid += 1
            continue
        tail = plp[offset:]
        if any(x is None for x in tail):
            totals[c["id"]] = -math.inf
            invalid += 1
            continue
        totals[c["id"]] = sum(tail)
    wall_ms = (time.monotonic() - t0) * 1000.0
    if invalid == len(candidates):
        return {
            "value": None,
            "probability": 0.0,
            "invalid_distribution": True,
            "wall_ms": wall_ms,
        }
    ids = [c["id"] for c in candidates]
    ps = softmax([totals[i] for i in ids])
    probs = {ids[i]: ps[i] for i in range(len(ids))}
    winner = max(probs, key=probs.get)
    return {
        "value": winner,
        "probability": probs[winner],
        "probs": probs,
        "invalid_distribution": invalid > 0,
        "invalid_candidates": invalid,
        "wall_ms": wall_ms,
    }


def ece(pairs: list[tuple[float, int]], bins: int = 10) -> tuple[float, list[dict]]:
    binned = [
        {"lo": i / bins, "hi": (i + 1) / bins, "n": 0, "conf": 0.0, "acc": 0.0}
        for i in range(bins)
    ]
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


def run_pass(
    port: int,
    suite: dict,
    items: list[dict],
    readout: str,
    timeout: float,
    perm: int,
    seed: int,
    rendered_cache: dict[tuple[int, str], str],
) -> list[dict]:
    rows = []
    for it in items:
        cands = permute(it["candidates"], perm, seed, it["id"])
        answer_position = next(
            (i for i, c in enumerate(cands) if c["id"] == it["answer"]), None
        )
        if readout == "letters":
            r = decide_letters(port, suite, it, cands, timeout)
        else:
            key = (perm, it["id"])
            if key not in rendered_cache:
                text, _ = render_prompt(port, letters_prompt(suite, it, cands), timeout)
                rendered_cache[key] = text
            r = decide_paths(port, suite, it, cands, rendered_cache[key], timeout)
        rows.append(
            {
                "id": it["id"],
                "class": it["class"],
                "answer": it["answer"],
                "perm": perm,
                "answer_position": answer_position,
                "pred": r["value"],
                "prob": r["probability"],
                **({"probs": r["probs"]} if "probs" in r else {}),
                "invalid_distribution": r["invalid_distribution"],
                "wall_ms": r["wall_ms"],
            }
        )
    return rows


def metrics(rows: list[dict]) -> dict:
    valid = [r for r in rows if r["pred"] is not None]
    acc_pairs = [(r["prob"], r["pred"] == r["answer"]) for r in valid]
    e, bins = ece(acc_pairs)
    by_class = {}
    for cls in sorted({r["class"] for r in rows}):
        sub = [r for r in rows if r["class"] == cls and r["pred"] is not None]
        if sub:
            by_class[cls] = sum(1 for r in sub if r["pred"] == r["answer"]) / len(sub)
    by_pos = {}
    for p in sorted({r["answer_position"] for r in rows if r["answer_position"] is not None}):
        sub = [r for r in rows if r["answer_position"] == p and r["pred"] is not None]
        if sub:
            by_pos[str(p)] = sum(1 for r in sub if r["pred"] == r["answer"]) / len(sub)
    ms = sorted(r["wall_ms"] for r in rows)
    return {
        "accuracy": sum(1 for r in valid if r["pred"] == r["answer"]) / len(rows),
        "accuracy_by_class": by_class,
        "accuracy_by_answer_position": by_pos,
        "ece": e,
        "ece_bins": bins,
        "invalid_distributions": sum(1 for r in rows if r["invalid_distribution"]),
        "n": len(rows),
        "p50_ms": ms[len(ms) // 2],
        "mean_ms": sum(ms) / len(ms),
    }


def compare_fork(rows: list[dict], fork_path: Path) -> dict:
    """Winner agreement and distribution deltas against fork decision rows.

    Winner-level agreement needs only `pred` on both sides. The
    distribution deltas (mean L1, max prob delta) are computed only over
    rows where BOTH sides carry full `probs` — the fork's margin-era
    rows carry the winner probability only, and those rows still answer
    the parity question."""
    fork = json.loads(fork_path.read_text())
    by_id = {r["id"]: r for r in fork["single"]}
    agree = 0
    max_delta = 0.0
    l1s = []
    compared = 0
    dist_compared = 0
    for r in rows:
        f = by_id.get(r["id"])
        if f is None or r["pred"] is None:
            continue
        compared += 1
        agree += 1 if r["pred"] == f["pred"] else 0
        if "probs" in r and "probs" in f:
            dist_compared += 1
            keys = set(r["probs"]) | set(f["probs"])
            l1s.append(sum(abs(r["probs"].get(k, 0.0) - f["probs"].get(k, 0.0)) for k in keys))
            max_delta = max(max_delta, abs(r["prob"] - f["prob"]))
    if not compared:
        return {"compared": 0}
    if dist_compared == compared:
        mode = "winner+distribution"
    elif dist_compared:
        mode = "winner+partial-distribution"
    else:
        mode = "winner-only"
    out: dict = {
        "compared": compared,
        "mode": mode,
        "winner_agreement": agree / compared,
    }
    if l1s:
        out["mean_l1"] = sum(l1s) / len(l1s)
        out["max_prob_delta"] = max_delta
    return out


def recompare(args: argparse.Namespace) -> int:
    """Recompute parity_vs_fork on an existing stock result (post-hoc).

    For results written before the compare join accepted winner-only
    fork rows: loads the recorded passes, reruns compare_fork against
    --compare-fork, and rewrites the JSON in place. No server, no
    re-measurement."""
    if args.compare_fork is None:
        print("--recompare requires --compare-fork", file=sys.stderr)
        return 2
    result = json.loads(args.recompare.read_text())
    result["parity_vs_fork"] = compare_fork(result["passes"]["0"], args.compare_fork)
    result["parity_vs_fork"]["fork_result"] = str(args.compare_fork)
    args.recompare.write_text(json.dumps(result, sort_keys=True, indent=1) + "\n")
    pv = result["parity_vs_fork"]
    m = result["metrics"]
    if "winner_agreement" in pv:
        print(
            f"recompare: compared={pv['compared']} ({pv['mode']}) "
            f"parity={pv['winner_agreement']:.3f} acc={m['accuracy']:.3f}",
            flush=True,
        )
    else:
        print("recompare: compared=0 — no rows joined; check id spaces", flush=True)
    return 0


def self_check() -> int:
    """Pure-logic checks runnable without a server."""
    cands = [{"id": f"c{i}"} for i in range(4)]
    assert permute(cands, 0, 7, "x") == cands
    a = permute(cands, 1, 7, "x")
    b = permute(cands, 1, 7, "x")
    assert a == b and a != cands
    assert sorted(c["id"] for c in a) == sorted(c["id"] for c in cands)
    ps = softmax([-1.0, -2.0, -3.0])
    assert abs(sum(ps) - 1.0) < 1e-12 and ps[0] > ps[1] > ps[2]
    ps2 = softmax([x + 100 for x in [-1.0, -2.0, -3.0]])
    assert all(abs(a - b) < 1e-12 for a, b in zip(ps, ps2))
    err, _ = ece([(0.9, 1), (0.9, 0), (0.2, 0)])
    assert err > 0
    print("self-check ok", flush=True)
    return 0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--llama-dir", type=Path, default=None)
    ap.add_argument("--build-dir", default="build-stock")
    ap.add_argument("--models-dir", type=Path, default=None)
    ap.add_argument("--model", default=None)
    ap.add_argument("--suite", type=Path, default=Path(__file__).parent.parent / "suite" / "suite.json")
    ap.add_argument("--out", type=Path, default=None)
    ap.add_argument("--readout", choices=["letters", "paths"], default=None)
    ap.add_argument("--port", type=int, default=8391)
    ap.add_argument("--threads", type=int, default=12)
    ap.add_argument("--ctx", type=int, default=8192)
    ap.add_argument("--ngl", type=int, default=0)
    ap.add_argument("--permutations", type=int, default=1,
                    help="passes over the suite; 1 = identity order only, "
                    "K > 1 adds K-1 seeded candidate-order permutations")
    ap.add_argument("--seed", type=int, default=20260926)
    ap.add_argument("--compare-fork", type=Path, default=None,
                    help="run_llama.py decision result to diff the paths "
                    "readout against (parity lane)")
    ap.add_argument("--recompare", type=Path, default=None,
                    help="existing stock result JSON: recompute its "
                    "parity_vs_fork against --compare-fork and rewrite it "
                    "(post-hoc, no server)")
    ap.add_argument("--self-check", action="store_true")
    ap.add_argument("--timeout", type=float, default=600.0)
    args = ap.parse_args()
    if args.self_check:
        return self_check()
    if args.recompare is not None:
        return recompare(args)
    missing = [n for n, v in (("--llama-dir", args.llama_dir), ("--models-dir", args.models_dir),
                              ("--model", args.model), ("--out", args.out),
                              ("--readout", args.readout)) if v is None]
    if missing:
        ap.error(f"{', '.join(missing)} are required unless --self-check or --recompare")

    load_start = load_now()
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
        "-ngl", str(args.ngl),
    ]
    print("spawn:", " ".join(cmd), flush=True)
    with open(args.out.with_suffix(".server.log"), "wb") as log:
        proc = subprocess.Popen(cmd, stdout=log, stderr=subprocess.STDOUT)
    monitor = ResourceMonitor()
    monitor.start()
    try:
        wait_health(args.port, proc)
        render_mode = None
        rendered_cache: dict[tuple[int, str], str] = {}
        if args.readout == "paths":
            # The paths readout needs per-prompt-token logprobs. Server
            # builds differ on this: the 2026-10 restructured upstream
            # server dropped `prompt_logprobs` entirely, and a run without
            # it is 30 minutes of invalid rows, so probe once and refuse
            # loudly before any suite work.
            probe = post(
                f"http://127.0.0.1:{args.port}/v1/completions",
                {"prompt": "probe", "max_tokens": 1, "temperature": 0, "prompt_logprobs": 1},
                60.0,
            )
            plp = probe["choices"][0].get("prompt_logprobs")
            if not isinstance(plp, list) or not plp:
                proc.terminate()
                proc.wait(timeout=10)
                print(
                    "error: this llama.cpp build does not return per-prompt-token "
                    "logprobs (`prompt_logprobs` absent from /v1/completions); the "
                    "paths readout needs a build that supports it. Use --readout "
                    "letters (the upstream-parity readout) instead.",
                    file=sys.stderr,
                )
                return 1
            modes = set()
            for it in items:
                text, mode = render_prompt(
                    args.port, letters_prompt(suite, it, it["candidates"]), args.timeout
                )
                rendered_cache[(0, it["id"])] = text
                modes.add(mode)
            render_mode = modes.pop() if len(modes) == 1 else "mixed"

        single = run_pass(args.port, suite, items, args.readout, args.timeout, 0, args.seed, rendered_cache)
        single_again = run_pass(args.port, suite, items, args.readout, args.timeout, 0, args.seed, rendered_cache)
        valid = [r for r in single if r["pred"] is not None]
        again_valid = [r for r in single_again if r["pred"] is not None]
        same_validity = len(valid) == len(again_valid)
        determinism = {
            "valid_rows_match": same_validity,
            "predictions_match": same_validity and all(
                a["pred"] == b["pred"] for a, b in zip(valid, again_valid)
            ),
            "max_prob_delta": max(
                (abs(a["prob"] - b["prob"]) for a, b in zip(single, single_again)),
                default=0.0,
            ),
        }
        perms = {"0": single, "identity_replay": single_again}
        for k in range(1, args.permutations):
            perms[str(k)] = run_pass(
                args.port, suite, items, args.readout, args.timeout, k, args.seed, rendered_cache
            )
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
        monitor.stop()

    result = {
        "arm": f"llama_stock_{args.readout}",
        "model": {"file": args.model, "sha256": sha256_file(model_path)},
        "suite_sha256": hashlib.sha256(args.suite.read_bytes()).hexdigest(),
        "config": {
            "threads": args.threads,
            "ctx": args.ctx,
            "ngl": args.ngl,
            "readout": args.readout,
            "render_mode": render_mode,
            "permutations": args.permutations,
            "seed": args.seed,
            "device": "vulkan-igpu" if args.ngl > 0 else "cpu",
            "load_avg": {"start": load_start, "end": load_now()},
        },
        "passes": perms,
        "determinism": determinism,
        "resources": monitor.report(),
        "metrics": metrics(single),
    }
    if args.compare_fork is not None:
        result["parity_vs_fork"] = compare_fork(single, args.compare_fork)
        result["parity_vs_fork"]["fork_result"] = str(args.compare_fork)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, sort_keys=True, indent=1) + "\n")
    m = result["metrics"]
    pv = result.get("parity_vs_fork")
    if pv is None:
        extra = ""
    elif "winner_agreement" in pv:
        extra = f" parity={pv['winner_agreement']:.3f} ({pv['mode']})"
    else:
        extra = " parity=unavailable (no rows joined)"
    print(
        f"acc={m['accuracy']:.3f} ece={m['ece']:.3f} "
        f"per_class={ {k: round(v, 3) for k, v in m['accuracy_by_class'].items()} } "
        f"by_pos={ {k: round(v, 3) for k, v in m['accuracy_by_answer_position'].items()} } "
        f"invalid={m['invalid_distributions']} determinism={determinism['predictions_match']}{extra}",
        flush=True,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
