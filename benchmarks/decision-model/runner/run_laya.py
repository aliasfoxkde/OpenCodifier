#!/usr/bin/env python3
"""Run the decision benchmark suite through Laya (Convai Innovations).

Laya is a ModernBERT-large (421M) non-autoregressive decision encoder:
every option is scored at its own option-marker position in a single
forward pass and softmaxed over that question's option set, so — like the
llama.cpp decision arm — it returns an exact distribution over a
runtime-defined candidate list. Nothing is sampled and no prose is
generated. This arm measures the "small classifier" layer of the
OpenCodifier escalation ladder at its strongest open data point.

Mapping (matches JevBench's preregistered `laya_local` adapter): the suite
context becomes the state, the question becomes the `instructions`, and the
candidate id/description pairs become `criteria`. Probabilities are the
model's own `answers.<name>.probabilities` (native softmax over option
markers). A distribution that misses a candidate id or does not sum to
within 2% of 1.0 is a schema failure: the item counts as wrong and is never
renormalized into validity.

Requires the `laya` package (CPU torch is enough); the English checkpoint
is fetched from `convaiinnovations/laya` on first use (~808 MB) and cached.

Usage:
  python3 runner/run_laya.py --out results/laya__en.json
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
import time
from pathlib import Path


def sha256_file(path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def run_once(agent, suite: dict, items: list[dict]) -> list[dict]:
    rows = []
    for it in items:
        criteria = {c["id"]: c["description"] for c in it["candidates"]}
        questions = {
            "decision": {
                "type": "choice",
                "instructions": it["question"],
                "criteria": criteria,
            }
        }
        t0 = time.monotonic()
        out = agent.predict(it["context"], questions)
        wall_ms = (time.monotonic() - t0) * 1000.0

        pred = None
        prob = 0.0
        invalid = False
        ans = (out.get("answers") or {}).get("decision") or {}
        probs = ans.get("probabilities")
        if not isinstance(probs, dict):
            invalid = True
        else:
            missing = [c["id"] for c in it["candidates"] if c["id"] not in probs]
            total = 0.0
            try:
                vals = {str(k): float(v) for k, v in probs.items()}
                total = sum(vals.values())
            except (TypeError, ValueError):
                invalid = True
                vals = {}
            if invalid or missing or not 0.98 <= total <= 1.02:
                invalid = True
            else:
                best = max(vals, key=lambda k: vals[k])
                pred = best
                prob = vals[best]
        rows.append(
            {
                "id": it["id"],
                "class": it["class"],
                "answer": it["answer"],
                "pred": pred,
                "prob": prob,
                "invalid_distribution": invalid,
                "wall_ms": wall_ms,
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


def metrics(rows: list[dict]) -> dict:
    by_class = {}
    for cls in sorted({r["class"] for r in rows}):
        sub = [r for r in rows if r["class"] == cls]
        by_class[cls] = sum(1 for r in sub if r["pred"] == r["answer"]) / len(sub)
    ms = sorted(r["wall_ms"] for r in rows)
    return {
        "accuracy": sum(1 for r in rows if r["pred"] == r["answer"]) / len(rows),
        "accuracy_by_class": by_class,
        "ece": ece([(r["prob"], r["pred"] == r["answer"]) for r in rows]),
        "invalid_distributions": sum(1 for r in rows if r["invalid_distribution"]),
        "latency": {
            "p50_ms": ms[len(ms) // 2],
            "p95_ms": ms[int(len(ms) * 0.95)],
            "mean_ms": sum(ms) / len(ms),
        },
        "ms_per_item": sum(ms) / len(ms),
        "n": len(rows),
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", default="convaiinnovations/laya",
                    help="HF repo, or a local checkpoint directory")
    ap.add_argument("--suite", type=Path,
                    default=Path(__file__).parent.parent / "suite" / "suite.json")
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--threads", type=int, default=4)
    args = ap.parse_args()

    suite = json.loads(args.suite.read_text())
    items = suite["items"]

    import laya
    import torch

    torch.set_num_threads(args.threads)
    agent = laya.load(args.repo)
    laya_version = getattr(laya, "__version__", None)

    # Content-hash the checkpoint (D14 practice). hf_hub_download is a no-op
    # when the checkpoint is already in the cache and returns its path.
    sha = None
    if Path(args.repo).exists():
        cached = Path(args.repo) / "model.safetensors"
        if cached.exists():
            sha = sha256_file(cached)
    else:
        try:
            from huggingface_hub import hf_hub_download

            sha = sha256_file(hf_hub_download(args.repo, "model.safetensors"))
        except OSError:
            sha = None

    single = run_once(agent, suite, items)
    single_again = run_once(agent, suite, items)
    determinism = {
        "predictions_match": all(
            a["pred"] == b["pred"] for a, b in zip(single, single_again)
        ),
        "max_prob_delta": max(
            abs(a["prob"] - b["prob"]) for a, b in zip(single, single_again)
        ),
    }

    result = {
        "arm": "laya_encoder_decision",
        "model": {
            "file": f"{args.repo} (English root, ModernBERT-large 421M)",
            "package": f"laya {laya_version}",
            "sha256": sha,
            "torch": torch.__version__,
        },
        "suite_sha256": hashlib.sha256(args.suite.read_bytes()).hexdigest(),
        "config": {
            "device": "cpu",
            "threads": args.threads,
            "probability_origin": "native-softmax-over-option-markers",
        },
        "single": single,
        "determinism": determinism,
        "metrics": metrics(single),
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, sort_keys=True, indent=1) + "\n")
    m = result["metrics"]
    print(
        f"acc={m['accuracy']:.3f} ece={m['ece']:.3f} "
        f"per_class={ {k: round(v, 3) for k, v in m['accuracy_by_class'].items()} } "
        f"p50={m['latency']['p50_ms']:.0f}ms "
        f"invalid={m['invalid_distributions']} "
        f"determinism={determinism['predictions_match']}",
        flush=True,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
