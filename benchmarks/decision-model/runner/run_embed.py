#!/usr/bin/env python3
"""Run the decision benchmark suite through an embedding zero-shot arm.

Scores each item by cosine similarity between the embedded context+question
and each candidate description, followed by a softmax over the cosines.
This is the "embedding similarity" layer of the OpenCodifier escalation
ladder: cheap, fully local (ONNX Runtime, CPU), no tokens generated.

Requires: onnxruntime, tokenizers, and a MiniLM-style encoder exported to
ONNX with its tokenizer.json, e.g. Xenova/all-MiniLM-L6-v2:

  models/minilm/model.onnx models/minilm/tokenizer.json

Usage:
  python3 runner/run_embed.py --model-dir /path/to/minilm \
      --out results/embed__minilm.json
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import sys
import time
from pathlib import Path

import numpy as np
import onnxruntime as ort
from tokenizers import Tokenizer


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


class Encoder:
    """Mean-pooled, L2-normalized sentence embeddings from a MiniLM ONNX."""

    def __init__(self, model_dir: Path, max_len: int = 256, threads: int = 8):
        self.tok = Tokenizer.from_file(str(model_dir / "tokenizer.json"))
        self.tok.enable_truncation(max_length=max_len)
        self.tok.enable_padding(pad_id=0, pad_token="[PAD]", length=None)
        so = ort.SessionOptions()
        so.intra_op_num_threads = threads
        self.sess = ort.InferenceSession(
            str(model_dir / "model.onnx"), sess_options=so, providers=["CPUExecutionProvider"]
        )
        self.input_names = {i.name for i in self.sess.get_inputs()}

    def encode(self, texts: list[str]) -> np.ndarray:
        enc = self.tok.encode_batch(texts)
        ids = np.array([e.ids for e in enc], dtype=np.int64)
        mask = np.array([e.attention_mask for e in enc], dtype=np.int64)
        feed = {}
        if "input_ids" in self.input_names:
            feed["input_ids"] = ids
        if "attention_mask" in self.input_names:
            feed["attention_mask"] = mask
        if "token_type_ids" in self.input_names:
            feed["token_type_ids"] = np.zeros_like(ids)
        out = self.sess.run(None, feed)[0]
        maskf = mask[:, :, None].astype(np.float32)
        summed = (out * maskf).sum(axis=1)
        counts = np.clip(maskf.sum(axis=1), 1e-9, None)
        mean = summed / counts
        norm = np.clip(np.linalg.norm(mean, axis=1, keepdims=True), 1e-12, None)
        return mean / norm


def run_once(enc: Encoder, suite: dict) -> tuple[list[dict], float]:
    rows = []
    t_embed = 0.0
    for it in suite["items"]:
        query = f"{it['context']}\n{it['question']}"
        cands = [c["description"] for c in it["candidates"]]
        t0 = time.monotonic()
        vecs = enc.encode([query, *cands])
        t_embed += time.monotonic() - t0
        sims = (vecs[1:] @ vecs[0]).tolist()
        mx = max(sims)
        exps = [math.exp(s - mx) for s in sims]
        z = sum(exps)
        probs = [e / z for e in exps]
        best = max(range(len(probs)), key=lambda i: probs[i])
        rows.append(
            {
                "id": it["id"],
                "class": it["class"],
                "answer": it["answer"],
                "pred": it["candidates"][best]["id"],
                "prob": probs[best],
            }
        )
    return rows, t_embed


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
    ap.add_argument("--model-dir", type=Path, required=True)
    ap.add_argument("--suite", type=Path, default=Path(__file__).parent.parent / "suite" / "suite.json")
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()

    suite = json.loads(args.suite.read_text())
    enc = Encoder(args.model_dir)

    rows, t1 = run_once(enc, suite)
    rows2, t2 = run_once(enc, suite)
    determinism = {
        "predictions_match": all(a["pred"] == b["pred"] for a, b in zip(rows, rows2)),
        "max_prob_delta": max(abs(a["prob"] - b["prob"]) for a, b in zip(rows, rows2)),
    }

    by_class = {}
    for cls in sorted({r["class"] for r in rows}):
        sub = [r for r in rows if r["class"] == cls]
        by_class[cls] = sum(1 for r in sub if r["pred"] == r["answer"]) / len(sub)
    acc = sum(1 for r in rows if r["pred"] == r["answer"]) / len(rows)

    result = {
        "arm": "embedding_zero_shot",
        "model": {
            "name": args.model_dir.name,
            "sha256_model_onnx": sha256_file(args.model_dir / "model.onnx"),
            "sha256_tokenizer": sha256_file(args.model_dir / "tokenizer.json"),
        },
        "suite_sha256": hashlib.sha256(args.suite.read_bytes()).hexdigest(),
        "config": {"pooling": "mean", "similarity": "cosine", "softmax": "tau=1", "device": "cpu"},
        "single": rows,
        "determinism": determinism,
        "metrics": {
            "accuracy": acc,
            "accuracy_by_class": by_class,
            "ece": ece([(r["prob"], r["pred"] == r["answer"]) for r in rows]),
            "embed_seconds_total": t1,
            "ms_per_item": t1 * 1000.0 / len(rows),
        },
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, sort_keys=True, indent=1) + "\n")
    print(
        f"acc={acc:.3f} ece={result['metrics']['ece']:.3f} "
        f"per_class={ {k: round(v, 3) for k, v in by_class.items()} } "
        f"ms/item={result['metrics']['ms_per_item']:.2f} "
        f"determinism={determinism['predictions_match']}",
        flush=True,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
