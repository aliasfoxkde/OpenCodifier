#!/usr/bin/env python3
"""GGUF conversion/quant parity fixtures (model-import discipline).

Fixture-first gate for any merged-and-converted GGUF arm: BEFORE any
benchmark corpus run, prove the artifact carries the HF model's
behavior. Two layers, per quality.md's model-import rules:

1. Tokenization parity — the GGUF's tokenizer (its ``tokenizer.ggml.pre``
   pre-tokenizer choice, the classic silent corruptor for a fresh vocab)
   against the HF fast tokenizer on real suite prompts. Exact id match
   required; anything else halts the lane.
2. Logit parity — next-token distribution at the last prompt position,
   fp32 HF reference vs the GGUF through llama-server: greedy top-1
   agreement plus per-token logprob deltas on the comparable slice.
   Sub-chance accuracy on fixtures is a bug alarm, never a model
   verdict (the 0.2251 mis-anchored-index lesson).

The sequence truth for the logit layer is the SERVER's own tokenization
(special tokens included) — the HF forward runs on exactly those ids,
so an embedding-row mismatch shows up here too.

Usage (one invocation per GGUF):

  python3 gguf_parity.py --hf-dir merged-hf \
      --gguf gguf/mc5-jev-q8_0.gguf --label mc5-jev-q8 \
      --server-bin ~/oc-model-eval/llama.cpp/build-pd/bin/llama-server \
      --suite ../suite/suite.json --out parity-mc5-jev-q8.json
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from run_llama import render_raw_prompt  # noqa: E402 (shared rendering)


def post(url: str, payload: dict, timeout: float = 240.0) -> dict:
    data = json.dumps(payload).encode()
    req = urllib.request.Request(
        url, data=data, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read())


def wait_health(port: int, proc: subprocess.Popen, deadline_s: float = 180.0) -> None:
    t0 = time.monotonic()
    while time.monotonic() - t0 < deadline_s:
        if proc.poll() is not None:
            sys.exit(f"llama-server exited rc={proc.returncode}")
        try:
            with urllib.request.urlopen(
                    f"http://127.0.0.1:{port}/health", timeout=5) as r:
                if r.status == 200:
                    return
        except urllib.error.URLError:
            pass
        time.sleep(1.0)
    sys.exit("llama-server health check timed out")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--hf-dir", type=Path, required=True)
    ap.add_argument("--gguf", type=Path, required=True)
    ap.add_argument("--label", required=True)
    ap.add_argument("--server-bin", type=Path, required=True)
    ap.add_argument("--suite", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--n", type=int, default=48)
    ap.add_argument("--port", type=int, default=8471)
    ap.add_argument("--threads", type=int, default=8)
    ap.add_argument("--ctx", type=int, default=4096)
    ap.add_argument("--top-hf", type=int, default=64,
                    help="HF reference tokens kept per fixture")
    ap.add_argument("--n-probs", type=int, default=128,
                    help="server top-K requested per fixture")
    ap.add_argument("--gate-conversion", action="store_true",
                    help="f16-vs-HF conversion leg: also require argmax "
                    "agreement >= 0.98 for exit 0")
    ap.add_argument("--gate-quant", action="store_true",
                    help="quantized leg: alarm threshold argmax >= 0.90 "
                    "(sub-0.90 means halt-and-diagnose, not acceptance)")
    args = ap.parse_args()

    import torch  # noqa: PLC0415 (arm-local heavy imports)
    from transformers import AutoModelForCausalLM, AutoTokenizer  # noqa: PLC0415

    suite = json.loads(args.suite.read_text())
    items = suite["items"][:args.n]
    prompts = [render_raw_prompt(suite, it, "") for it in items]

    tok = AutoTokenizer.from_pretrained(str(args.hf_dir))
    sys.stdout.write(f"loading {args.hf_dir} in fp32...\n")
    sys.stdout.flush()
    model = AutoModelForCausalLM.from_pretrained(
        str(args.hf_dir), torch_dtype=torch.float32)
    model.eval()

    server = subprocess.Popen([
        str(args.server_bin), "-m", str(args.gguf),
        "--host", "127.0.0.1", "--port", str(args.port),
        "-t", str(args.threads), "-c", str(args.ctx),
    ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    wait_health(args.port, server)

    rows = []
    try:
        for it, text in zip(items, prompts):
            # Layer 1: raw tokenization (no specials) HF vs server.
            hf_ids = tok(text, add_special_tokens=False)["input_ids"]
            srv = post(f"http://127.0.0.1:{args.port}/tokenize",
                       {"content": text, "add_special_token": False})
            ids_match = srv.get("tokens") == hf_ids

            # Layer 2: logits on the SERVER's sequence (specials incl.).
            srv_full = post(f"http://127.0.0.1:{args.port}/tokenize",
                            {"content": text, "add_special_token": True})
            seq = srv_full.get("tokens") or hf_ids
            with torch.no_grad():
                logits = model(input_ids=torch.tensor([seq])).logits[0, -1]
            lp = torch.log_softmax(logits.float(), dim=-1)
            top = torch.topk(lp, args.top_hf)
            hf_top = {int(i): float(v) for i, v in
                      zip(top.indices.tolist(), top.values.tolist())}

            resp = post(f"http://127.0.0.1:{args.port}/completion", {
                "prompt": text, "n_predict": 1, "temperature": 0.0,
                "n_probs": args.n_probs, "cache_prompt": False,
            })
            ent = (resp.get("completion_probabilities") or [{}])[0]
            # This fork names the top-K slice "top_logprobs" (upstream
            # historically "probs"); entries carry the token id directly
            # (piece text "token"/"tok_str" as the older fallback).
            probs = ent.get("top_logprobs") or ent.get("probs") or []
            gg = {}
            for p in probs:
                tid = p.get("id")
                if tid is None:  # older servers: match by piece text
                    tid = tok.convert_tokens_to_ids(
                        ((p.get("token") or p.get("tok_str") or "")
                         .replace(" ", "Ġ")))
                if tid is not None and tid >= 0:
                    gg[int(tid)] = float(p.get("logprob") or 0.0)

            gg_top1 = max(gg, key=gg.get) if gg else None
            hf_top1 = max(hf_top, key=hf_top.get)
            common = sorted(set(hf_top) & set(gg))
            deltas = [abs(hf_top[i] - gg[i]) for i in common]
            rows.append({
                "id": it["id"],
                "class": it.get("class"),
                "ids_match": bool(ids_match),
                "n_tokens": len(hf_ids),
                "argmax_match": bool(gg_top1 == hf_top1),
                "mean_abs_dlp": (sum(deltas) / len(deltas)) if deltas else None,
                "max_abs_dlp": max(deltas) if deltas else None,
                "n_common": len(common),
                "n_gguf": len(gg),
            })
    finally:
        server.terminate()
        server.wait(timeout=30)

    n = len(rows)
    tok_rate = sum(r["ids_match"] for r in rows) / n
    agr = sum(r["argmax_match"] for r in rows) / n
    ds = [r["mean_abs_dlp"] for r in rows if r["mean_abs_dlp"] is not None]
    report = {
        "label": args.label,
        "hf_dir": str(args.hf_dir),
        "gguf": str(args.gguf),
        "gguf_sha256": hashlib.sha256(args.gguf.read_bytes()).hexdigest(),
        "suite": str(args.suite),
        "suite_sha256": hashlib.sha256(args.suite.read_bytes()).hexdigest(),
        "n": n,
        "tokenize_match_rate": tok_rate,
        "argmax_agreement": agr,
        "mean_abs_dlp": sum(ds) / len(ds) if ds else None,
        "worst_abs_dlp": max(ds) if ds else None,
        "rows": rows,
    }
    args.out.write_text(json.dumps(report, indent=1) + "\n")
    sys.stdout.write(
        f"{args.label}: n={n} tokenize={tok_rate:.3f} argmax={agr:.3f} "
        f"mean|dlp|={report['mean_abs_dlp']} worst={report['worst_abs_dlp']}"
        f" -> {args.out}\n")
    # Gate: tokenization must be exact; the f16 conversion leg additionally
    # expects near-perfect argmax agreement (pass --gate-conversion there).
    ok = tok_rate == 1.0
    if args.gate_conversion:
        ok = ok and agr >= 0.98
    if args.gate_quant:
        ok = ok and agr >= 0.90  # alarm threshold, not an acceptance bar
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
