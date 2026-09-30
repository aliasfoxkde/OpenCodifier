#!/usr/bin/env python3
"""Run the decision benchmark suite through VTX-JEV-3 (VTXAI).

VTX-JEV-3 is a Model2Vec-style static embedding decision engine: a
255,753 x 256 embedding table (2-bit LF2-quantized by default, FP32
variant available) with a position-gated attention pooler
(`gate_params.npz`) that restores word-order sensitivity mean pooling
loses. Its `Choice` interface is candidate-conditioned exactly like the
suite: the state encodes on its own, each candidate encodes as
"<question> <description>", and a softmax over the cosine similarities
(scale 15) is the model's own distribution over the runtime-defined
candidate list. Nothing is sampled and no prose is generated.

The vendor client (`inference.py`) must drive the pooler — loading the
FP32 table through plain Model2Vec mean-pooling does not reproduce this
decision head, so the runner always goes through `JevClient`. This arm
measures the smallest end of the "fast semantic scoring" layer of the
escalation ladder: 19.5 MB on disk, NumPy-only, sub-millisecond on CPU.

Usage:
  python3 runner/run_vtx.py --model-dir /path/to/vtx-jev-3 \
      --out results/vtx__VTX-JEV-3-lf2.json
  python3 runner/run_vtx.py --model-dir /path/to/vtx-jev-3 --weights fp32 \
      --out results/vtx__VTX-JEV-3-fp32.json
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
import time
from pathlib import Path

# The suite contract (row shape, ECE, per-class metrics, determinism) is
# shared with the Laya encoder arm; reuse instead of duplicating it.
sys.path.insert(0, str(Path(__file__).parent))
from run_laya import ece, metrics, sha256_file  # noqa: E402
from resources import ResourceMonitor  # noqa: E402


def run_once(client, items: list[dict]) -> list[dict]:
    from inference import Choice

    rows = []
    for it in items:
        question = Choice(
            it["question"], {c["id"]: c["description"] for c in it["candidates"]}
        )
        t0 = time.monotonic()
        response = client.system_one(state=it["context"], questions={"decision": question})
        wall_ms = (time.monotonic() - t0) * 1000.0

        result = response.choices.get("decision")
        if result is None:
            rows.append(
                {
                    "id": it["id"],
                    "class": it["class"],
                    "answer": it["answer"],
                    "pred": None,
                    "prob": 0.0,
                    "invalid_distribution": True,
                    "wall_ms": wall_ms,
                    "vendor_ms": response.latency_ms,
                }
            )
            continue
        # A distribution that misses a candidate id or does not sum to
        # within 2% of 1.0 counts as wrong and is never renormalized into
        # validity (same contract as the Laya arm).
        missing = [c["id"] for c in it["candidates"] if c["id"] not in result.distribution]
        total = sum(result.distribution.values())
        invalid = bool(missing) or not 0.98 <= total <= 1.02
        rows.append(
            {
                "id": it["id"],
                "class": it["class"],
                "answer": it["answer"],
                "pred": result.choice,
                "prob": float(result.confidence),
                "invalid_distribution": invalid,
                "wall_ms": wall_ms,
                "vendor_ms": response.latency_ms,
            }
        )
    return rows


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--model-dir", type=Path, required=True,
                    help="local checkout of VTXAI/VTX-JEV-3 (inference.py + weights)")
    ap.add_argument("--weights", choices=["lf2", "fp32"], default="lf2",
                    help="embedding table format (lf2 = 2-bit, ~20 MB; fp32 = ~262 MB)")
    ap.add_argument("--suite", type=Path,
                    default=Path(__file__).parent.parent / "suite" / "suite.json")
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()

    sys.path.insert(0, str(args.model_dir))
    from inference import JevClient

    monitor = ResourceMonitor()
    monitor.start()

    weights_file = "model_lf2.safetensors" if args.weights == "lf2" else "model.safetensors"
    client = JevClient.from_pretrained(str(args.model_dir), prefer_lf2=args.weights == "lf2")
    used_lf2 = bool(client.is_lf2)
    if (args.weights == "lf2") != used_lf2:
        print(f"error: requested {args.weights} but the client loaded "
              f"{'lf2' if used_lf2 else 'fp32'}", file=sys.stderr)
        return 1

    suite = json.loads(args.suite.read_text())
    items = suite["items"]

    single = run_once(client, items)
    single_again = run_once(client, items)
    determinism = {
        "predictions_match": all(a["pred"] == b["pred"] for a, b in zip(single, single_again)),
        "max_prob_delta": max(abs(a["prob"] - b["prob"]) for a, b in zip(single, single_again)),
    }

    result = {
        "arm": "static_embedding_decision",
        "model": {
            "file": f"VTXAI/VTX-JEV-3 ({args.weights} table)",
            "name": "VTX-JEV-3",
            "architecture": "static embedding table 255753x256 + position-gated attention pooler",
            "base_model": "VTXAI/VTX-JEV-2",
            "trained_on": "SargeDev/jev-distill-corpus-v3",
            "sha256": sha256_file(args.model_dir / weights_file),
            "sha256_tokenizer": sha256_file(args.model_dir / "tokenizer.json"),
            "sha256_gate_params": sha256_file(args.model_dir / "gate_params.npz"),
            "sha256_inference": sha256_file(args.model_dir / "inference.py"),
            "embedding_dim": client.dim,
            "vocab_size": client.vocab,
        },
        "suite_sha256": hashlib.sha256(args.suite.read_bytes()).hexdigest(),
        "config": {
            "weights_format": args.weights,
            "pooling": "position_gated_attention" if client.gate_params is not None else "mean",
            "choice_scoring": "softmax over cosine similarities, scale 15.0",
            "choice_encoding": "'<question> <candidate description>' vs the state",
            "client": "vendor inference.py JevClient",
            "device": "cpu",
        },
        "single": single,
        "determinism": determinism,
        "metrics": metrics(single),
    }
    # The client's own perf_counter timing (encoding + decision, excluding
    # runner overhead) — the vendor's headline latency claim, measured.
    vendor = sorted(r["vendor_ms"] for r in single)
    result["metrics"]["vendor_p50_ms"] = vendor[len(vendor) // 2]
    result["metrics"]["vendor_mean_ms"] = sum(vendor) / len(vendor)

    monitor.stop()
    result["resources"] = monitor.report()

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, sort_keys=True, indent=1) + "\n")
    m = result["metrics"]
    print(
        f"acc={m['accuracy']:.3f} ece={m['ece']:.3f} "
        f"per_class={ {k: round(v, 3) for k, v in m['accuracy_by_class'].items()} } "
        f"p50={m['latency']['p50_ms']:.2f}ms vendor_p50={m['vendor_p50_ms']:.2f}ms "
        f"invalid={m['invalid_distributions']} "
        f"determinism={determinism['predictions_match']}",
        flush=True,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
