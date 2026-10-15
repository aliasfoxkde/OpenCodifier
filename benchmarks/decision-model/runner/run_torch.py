#!/usr/bin/env python3
"""Run the decision benchmark suite through a transformers/torch model.

The safetensors arm of the format A/B: same suite, same letters readout,
same output schema as run_stock.py, different engine. llama.cpp cannot
read safetensors, so "is safetensors faster" is really engine-vs-engine:
torch eager on the HF repo against llama.cpp on the ready-made GGUFs of
the same weights (mradermacher f16 / Q4_K_M for the 1B-0.8B-tiny pair).

Readout parity with run_stock.py: the letters readout there adds +100
`logit_bias` to every letter token and renormalizes the first-token
top-logprobs over the letters. The +100 shift is common to all letters
and cancels in that renormalization, so the identical computation is a
softmax over the raw final-position logits at the K letter token ids.
This runner computes exactly that — no bias, no top-k truncation, one
forward pass per item. Any accuracy gap between the torch arm and the
f16 GGUF arm is engine numerics (or a chat-template mismatch, which the
recorded probe section makes diagnosable).

Everything reusable is imported from run_stock.py — permutation, prompt
construction, ECE, metrics and the pass/replay determinism protocol are
byte-identical to the GGUF arms by construction, not by copy.

Usage:
  python3 runner/run_torch.py --hf-dir /path/to/hf-repo \
      --dtype float32 --permutations 3 --out results/torch__tiny1b-fp32.json
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
import tempfile
import time
from pathlib import Path


def _ensure_torchvision_importable() -> None:
    """Shadow a broken torchvision install with a minimal empty package.

    This host's torchvision fails op registration against the installed
    torch (RuntimeError on `import torchvision`, even version-matched),
    and transformers' gemma4 config chain imports torchvision.io at module
    load even though text-only inference never touches image decoding.
    When the real package cannot import, put a minimal shadow package
    first on sys.path so `import torchvision` succeeds; any image-path
    feature would raise on use, which the letters readout never does.
    """
    try:
        import torchvision  # noqa: F401
        return
    except Exception:
        pass
    root = Path(tempfile.mkdtemp(prefix="torchvision-shadow-"))
    pkg = root / "torchvision"
    permissive = (
        "def __getattr__(name):\n"
        "    if name.startswith('__'):\n"
        "        raise AttributeError(name)\n"
        "    return None\n"
    )
    # transforms must expose real enum members: transformers' image_utils
    # builds a resampling map at module level from these attributes.
    transforms_init = (
        "class InterpolationMode:\n"
        "    NEAREST = 'nearest'\n"
        "    NEAREST_EXACT = 'nearest-exact'\n"
        "    BILINEAR = 'bilinear'\n"
        "    BICUBIC = 'bicubic'\n"
        "    LANCZOS = 'lanczos'\n"
        "    BOX = 'box'\n"
        "    HAMMING = 'hamming'\n"
        + permissive
    )
    for mod, body in (
        (pkg / "__init__.py", permissive),
        (pkg / "io" / "__init__.py", permissive),
        (pkg / "transforms" / "__init__.py", transforms_init),
        (pkg / "transforms" / "functional.py", permissive),
        (pkg / "transforms" / "functional_tensor.py", permissive),
    ):
        mod.parent.mkdir(parents=True, exist_ok=True)
        mod.write_text(body)
    sys.path.insert(0, str(root))


_ensure_torchvision_importable()

import torch
from transformers import AutoModelForCausalLM, AutoTokenizer

sys.path.insert(0, str(Path(__file__).parent))
from run_llama import load_now  # noqa: E402
from run_stock import (  # noqa: E402
    LETTERS,
    letters_prompt,
    metrics,
    permute,
    softmax,
)
from resources import ResourceMonitor  # noqa: E402
import transformers  # noqa: E402


def letter_ids(tok, count: int) -> list[int]:
    """Token id of each option letter under the HF tokenizer."""
    ids = []
    for ch in LETTERS[:count]:
        enc = tok.encode(ch, add_special_tokens=False)
        if len(enc) != 1:
            raise RuntimeError(f"letter {ch!r} tokenized to {len(enc)} tokens")
        ids.append(enc[0])
    return ids


def template_ids(rendered) -> list[int]:
    """Normalize apply_chat_template output to a flat id list.

    transformers 5.x returns a BatchEncoding (a UserDict, so not a dict
    subclass); other paths return a tokenizers.Encoding or a bare id list."""
    if hasattr(rendered, "keys"):
        try:
            return list(rendered["input_ids"])
        except (KeyError, TypeError):
            pass
    ids = getattr(rendered, "ids", None)
    if ids is not None:
        return list(ids)
    return list(rendered)


def decide_letters(
    model, tok, suite: dict, it: dict, candidates: list[dict], letters: list[int]
) -> dict:
    """Constrained letter distribution: softmax over final-position logits."""
    messages = letters_prompt(suite, it, candidates)
    t0 = time.monotonic()
    rendered = tok.apply_chat_template(
        messages, add_generation_prompt=True, tokenize=True, enable_thinking=False
    )
    ids = template_ids(rendered)
    input_ids = torch.tensor([ids], dtype=torch.long)
    with torch.inference_mode():
        logits = model(input_ids=input_ids).logits[0, -1]
    forward_ms = (time.monotonic() - t0) * 1000.0
    letter_logits = logits[letters].to(torch.float64).tolist()
    ps = softmax(letter_logits)
    probs = {candidates[i]["id"]: ps[i] for i in range(len(candidates))}
    winner = max(probs, key=probs.get)
    return {
        "value": winner,
        "probability": probs[winner],
        "probs": probs,
        "invalid_distribution": False,
        "wall_ms": (time.monotonic() - t0) * 1000.0,
        "forward_ms": forward_ms,
        "n_prompt_tokens": len(ids),
    }


def run_pass(
    model, tok, suite: dict, items: list[dict], perm: int, seed: int, letters: list[int]
) -> list[dict]:
    rows = []
    for it in items:
        cands = permute(it["candidates"], perm, seed, it["id"])
        answer_position = next(
            (i for i, c in enumerate(cands) if c["id"] == it["answer"]), None
        )
        r = decide_letters(model, tok, suite, it, cands, letters[: len(cands)])
        rows.append(
            {
                "id": it["id"],
                "class": it["class"],
                "answer": it["answer"],
                "perm": perm,
                "answer_position": answer_position,
                "pred": r["value"],
                "prob": r["probability"],
                "probs": r["probs"],
                "invalid_distribution": False,
                "wall_ms": r["wall_ms"],
                "forward_ms": r["forward_ms"],
                "n_prompt_tokens": r["n_prompt_tokens"],
            }
        )
    return rows


def sha256_safetensors(hf_dir: Path) -> str:
    h = hashlib.sha256()
    for shard in sorted(hf_dir.glob("*.safetensors")):
        h.update(shard.read_bytes())
    return h.hexdigest()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--hf-dir", type=Path, required=True)
    ap.add_argument("--suite", type=Path,
                    default=Path(__file__).parent.parent / "suite" / "suite.json")
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--dtype", choices=["float32", "bfloat16"], default="float32")
    ap.add_argument("--threads", type=int, default=12)
    ap.add_argument("--permutations", type=int, default=3)
    ap.add_argument("--seed", type=int, default=20260926)
    ap.add_argument("--chat-template-file", type=Path, default=None,
                    help="jinja chat template overriding the tokenizer's; "
                         "point it at the llama.cpp-side template so both "
                         "arms of a comparison render identical prompts")
    args = ap.parse_args()

    torch.manual_seed(0)
    torch.set_num_threads(args.threads)
    try:
        torch.set_num_interop_threads(1)
    except RuntimeError:
        pass  # already initialized in this process

    load_start = load_now()
    suite = json.loads(args.suite.read_text())
    items = suite["items"]
    args.out.parent.mkdir(parents=True, exist_ok=True)

    dt = {"float32": torch.float32, "bfloat16": torch.bfloat16}[args.dtype]
    tok = AutoTokenizer.from_pretrained(args.hf_dir)
    template_sha256 = None
    if args.chat_template_file is not None:
        template = args.chat_template_file.read_text()
        tok.chat_template = template
        template_sha256 = hashlib.sha256(template.encode()).hexdigest()
    model = AutoModelForCausalLM.from_pretrained(
        args.hf_dir, dtype=dt, attn_implementation="sdpa"
    )
    model.eval()
    letters = letter_ids(tok, len(LETTERS))

    # Probe section: enough to diagnose a chat-template mismatch against
    # the llama.cpp arms without re-running anything.
    probe_ids = template_ids(tok.apply_chat_template(
        letters_prompt(suite, items[0], permute(items[0]["candidates"], 0, args.seed, items[0]["id"])),
        add_generation_prompt=True,
        tokenize=True,
        enable_thinking=False,
    ))
    probe = {
        "n_prompt_tokens": len(probe_ids),
        "prompt_tail": tok.decode(probe_ids[-24:]),
        "add_generation_prompt_ids": probe_ids[-8:],
    }

    monitor = ResourceMonitor()
    monitor.start()
    single = run_pass(model, tok, suite, items, 0, args.seed, letters)
    single_again = run_pass(model, tok, suite, items, 0, args.seed, letters)
    monitor.stop()
    determinism = {
        "valid_rows_match": len(single) == len(single_again),
        "predictions_match": all(
            a["pred"] == b["pred"] for a, b in zip(single, single_again)
        ),
        "max_prob_delta": max(
            (abs(a["prob"] - b["prob"]) for a, b in zip(single, single_again)),
            default=0.0,
        ),
    }
    perms = {"0": single, "identity_replay": single_again}
    for k in range(1, args.permutations):
        monitor.start()
        perms[str(k)] = run_pass(model, tok, suite, items, k, args.seed, letters)
        monitor.stop()

    fts = [
        r["n_prompt_tokens"] / (r["forward_ms"] / 1000.0)
        for r in single
        if r["forward_ms"] > 0
    ]
    result = {
        "arm": f"torch_safetensors_{args.dtype}",
        "model": {"hf_dir": str(args.hf_dir), "sha256": sha256_safetensors(args.hf_dir)},
        "suite_sha256": hashlib.sha256(args.suite.read_bytes()).hexdigest(),
        "config": {
            "dtype": args.dtype,
            "threads": args.threads,
            "device": "cpu",
            "attn_implementation": "sdpa",
            "torch": torch.__version__,
            "transformers": transformers.__version__,
            "chat_template_sha256": template_sha256,
            "permutations": args.permutations,
            "seed": args.seed,
            "load_avg": {"start": load_start, "end": load_now()},
        },
        "probe": probe,
        "passes": perms,
        "determinism": determinism,
        "resources": monitor.report(),
        "metrics": metrics(single),
        "prefill_tps_mean": sum(fts) / len(fts) if fts else 0.0,
    }
    args.out.write_text(json.dumps(result, sort_keys=True, indent=1) + "\n")
    m = result["metrics"]
    print(
        f"acc={m['accuracy']:.3f} ece={m['ece']:.3f} "
        f"p50={m['p50_ms']:.0f}ms fwd_tps={result['prefill_tps_mean']:.0f} "
        f"determinism={determinism['predictions_match']}",
        flush=True,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
