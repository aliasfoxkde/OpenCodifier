#!/usr/bin/env python3
"""Run the decision benchmark suite through an embedding zero-shot arm.

Scores each item by cosine similarity between the embedded context+question
and each candidate description, followed by a softmax over the cosines.
This is the "embedding similarity" layer of the OpenCodifier escalation
ladder: cheap, fully local (CPU), no tokens generated.

Three backends, identical math (mean pooling, L2 norm, cosine, softmax
tau=1):

- onnx (default): onnxruntime + tokenizers over a MiniLM-style encoder
  exported to ONNX with its tokenizer.json, e.g. Xenova/all-MiniLM-L6-v2:
  models/minilm/model.onnx models/minilm/tokenizer.json
- torch: transformers over an HF safetensors encoder directory (e.g.
  Alibaba-NLP/gte-modernbert-base); needs torch + transformers.
- llamacpp: a GGUF embedding model served by llama-server (`--embedding`),
  e.g. embeddinggemma-300M-Q8_0.gguf; needs --llama-dir with the llama.cpp
  build. The server's pooled embedding is L2-normalized client-side so the
  downstream math is identical to the other backends.

Usage:
  python3 runner/run_embed.py --model-dir /path/to/minilm \
      --out results/embed__minilm.json
  python3 runner/run_embed.py --backend torch --model-dir .../gte-modernbert-base \
      --out results/embed__gte-modernbert.json
  python3 runner/run_embed.py --backend llamacpp --model-dir .../embeddinggemma.gguf \
      --llama-dir /path/to/llama.cpp --out results/embed__embeddinggemma.json
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
from tokenizers import Tokenizer

from resources import ResourceMonitor


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


class Encoder:
    """Mean-pooled, L2-normalized sentence embeddings from a MiniLM ONNX."""

    def __init__(self, model_dir: Path, max_len: int = 256, threads: int = 8):
        import onnxruntime as ort

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


class TorchEncoder:
    """Mean-pooled, L2-normalized embeddings from an HF safetensors encoder."""

    def __init__(self, model_dir: Path, max_len: int = 256, threads: int = 8):
        import torch
        from transformers import AutoModel, AutoTokenizer

        self.max_len = max_len
        torch.set_num_threads(threads)
        torch.set_grad_enabled(False)
        self.tok = AutoTokenizer.from_pretrained(model_dir)
        self.model = AutoModel.from_pretrained(model_dir)
        self.model.eval()

    def encode(self, texts: list[str]) -> np.ndarray:
        batch = self.tok(
            texts, padding=True, truncation=True, max_length=self.max_len, return_tensors="pt"
        )
        out = self.model(**batch).last_hidden_state
        mask = batch["attention_mask"][:, :, None].to(out.dtype)
        mean = (out * mask).sum(dim=1) / mask.sum(dim=1).clamp(min=1e-9)
        norm = mean.norm(dim=1, keepdim=True).clamp(min=1e-12)
        return (mean / norm).numpy()


class LlamaCppEncoder:
    """GGUF embedding model served by llama-server's /v1/embeddings route.

    The server pools (model-default / --pooling) and L2-normalizes by
    default; vectors are re-normalized client-side so the math downstream
    is identical to the other backends. The server process is owned by
    this instance and terminated on close.
    """

    def __init__(
        self,
        model_dir: Path,
        max_len: int = 256,
        threads: int = 8,
        llama_dir: Path | None = None,
        build_dir: str = "build-pd",
        port: int = 8393,
        llamacpp_branch: str = "unknown",
    ):
        import atexit
        import subprocess
        import time
        import urllib.error
        import urllib.request

        if model_dir.is_file():
            gguf = model_dir
        else:
            found = sorted(model_dir.glob("*.gguf"))
            if len(found) != 1:
                raise ValueError(
                    f"--model-dir must be a .gguf file or a dir with exactly one, got {len(found)}"
                )
            gguf = found[0]
        if llama_dir is None:
            raise ValueError("--llama-dir is required for the llamacpp backend")
        self.gguf = gguf
        self.port = port
        self.llamacpp_branch = llamacpp_branch
        bin_path = llama_dir / build_dir / "bin" / "llama-server"
        self.proc = subprocess.Popen(
            [
                str(bin_path),
                "-m",
                str(gguf),
                "--port",
                str(port),
                "--embedding",
                "-c",
                "2048",
                "-fa",
                "on",
                "-t",
                str(threads),
                "-ngl",
                "0",
                "--parallel",
                "1",
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        atexit.register(self.close)
        base = f"http://127.0.0.1:{port}"
        deadline = time.monotonic() + 300.0
        while time.monotonic() < deadline:
            if self.proc.poll() is not None:
                raise RuntimeError(f"llama-server exited early with {self.proc.returncode}")
            try:
                with urllib.request.urlopen(f"{base}/health", timeout=5) as r:
                    if r.status == 200:
                        break
            except (urllib.error.URLError, OSError):
                time.sleep(1.0)
        else:
            self.close()
            raise RuntimeError("llama-server did not become healthy in 300 s")
        self.base = base

    def encode(self, texts: list[str]) -> np.ndarray:
        import json
        import urllib.request

        req = urllib.request.Request(
            f"{self.base}/v1/embeddings",
            data=json.dumps({"input": texts}).encode(),
            headers={"Content-Type": "application/json"},
        )
        with urllib.request.urlopen(req, timeout=600) as r:
            resp = json.load(r)
        data = sorted(resp["data"], key=lambda d: d["index"])
        vec = np.array([d["embedding"] for d in data], dtype=np.float32)
        norm = np.clip(np.linalg.norm(vec, axis=1, keepdims=True), 1e-12, None)
        return vec / norm

    def close(self) -> None:
        if self.proc.poll() is None:
            self.proc.kill()


def make_encoder(backend: str, model_dir: Path, max_len: int, threads: int, **kwargs):
    if backend == "onnx":
        return Encoder(model_dir, max_len=max_len, threads=threads)
    if backend == "torch":
        return TorchEncoder(model_dir, max_len=max_len, threads=threads)
    if backend == "llamacpp":
        return LlamaCppEncoder(model_dir, max_len=max_len, threads=threads, **kwargs)
    raise ValueError(f"unknown backend: {backend}")


def encoder_provenance(backend: str, model_dir: Path) -> dict:
    if backend == "onnx":
        return {
            "name": model_dir.name,
            "sha256_model_onnx": sha256_file(model_dir / "model.onnx"),
            "sha256_tokenizer": sha256_file(model_dir / "tokenizer.json"),
        }
    if backend == "llamacpp":
        gguf = model_dir if model_dir.is_file() else sorted(model_dir.glob("*.gguf"))[0]
        return {"name": gguf.stem, "sha256_gguf": sha256_file(gguf)}
    import torch
    import transformers

    return {
        "name": model_dir.name,
        "sha256_model_safetensors": sha256_file(model_dir / "model.safetensors"),
        "sha256_tokenizer": sha256_file(model_dir / "tokenizer.json"),
        "torch_transformers_versions": {
            "torch": torch.__version__,
            "transformers": transformers.__version__,
        },
    }


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
    ap.add_argument("--backend", choices=["onnx", "torch", "llamacpp"], default="onnx")
    ap.add_argument("--threads", type=int, default=8)
    ap.add_argument("--max-len", type=int, default=256)
    ap.add_argument(
        "--llama-dir",
        type=Path,
        default=None,
        help="llama.cpp checkout holding <build-dir>/bin/llama-server (llamacpp backend)",
    )
    ap.add_argument("--build-dir", default="build-pd", help="build subdir under --llama-dir")
    ap.add_argument("--port", type=int, default=8393, help="llama-server port (llamacpp backend)")
    ap.add_argument(
        "--llamacpp-branch",
        default="parallel-decision ad129b0",
        help="fork/branch provenance recorded for the llamacpp backend",
    )
    args = ap.parse_args()

    # Rooted at this process: in-process backends (onnx/torch) are measured
    # directly; the llamacpp backend's llama-server is a child, so the tree
    # walk covers it too.
    monitor = ResourceMonitor()
    monitor.start()

    suite = json.loads(args.suite.read_text())
    enc = make_encoder(
        args.backend,
        args.model_dir,
        args.max_len,
        args.threads,
        llama_dir=args.llama_dir,
        build_dir=args.build_dir,
        port=args.port,
        llamacpp_branch=args.llamacpp_branch,
    )
    provenance = encoder_provenance(args.backend, args.model_dir)
    if args.backend == "llamacpp":
        provenance["llamacpp_branch"] = args.llamacpp_branch
        provenance["server_flags"] = "--embedding -c 2048 -fa on -t <threads> -ngl 0 --parallel 1"

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
        "model": provenance,
        "suite_sha256": hashlib.sha256(args.suite.read_bytes()).hexdigest(),
        "config": {
            "pooling": "mean",
            "similarity": "cosine",
            "softmax": "tau=1",
            "device": "cpu",
            "backend": args.backend,
            "max_len": args.max_len,
            "threads": args.threads,
        },
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
    monitor.stop()
    result["resources"] = monitor.report()

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
