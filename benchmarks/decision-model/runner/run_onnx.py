#!/usr/bin/env python3
"""Run the decision benchmark suite through the ONNX Qwen3.5-2B export.

The comparison arm for `run_llama.py`'s decision arm: instead of
llama-server's `/v1/decision`, this drives the optimum-exported
`onnx-community/Qwen3.5-2B-ONNX` graphs directly with onnxruntime and
re-implements the fork's exact tree scoring in Python.

Faithfulness notes (all checked against
`tools/parallel-decision/decision-engine.cpp` in thecodacus/llama.cpp
branch `parallel-decision`):

- prompt: `compile_schema` + `render_prompt` produce system text
  "Select the requested field value ... Allowed values: ..." + a user
  message holding the bare context, then the assistant generation prompt
  followed by "{\\n" and the field suffix `  "choice": "` (JSON-encoded
  field name plus the quote shared by every encoded value). Replicated
  byte-for-byte via the HF chat template with the same system/user split
  (enable_thinking=false, matching `render_prompt`).
- paths: each candidate is its JSON encoding minus the shared leading
  quote, tokenized with a trailing "\\n" terminator
  (`paths.push_back(tokenize(c + "\\n", false))`).
- score: a trie over the paths; at every divergence node the raw logits
  are log-softmaxed over that node's allowed tokens only
  (`finish_tree`), summed along each candidate path, then softmaxed over
  candidates (`probs`). Tokens after the last divergence are free, ties
  break to the lowest candidate index (`max_element`).
- execution shape: one prefill of prompt+suffix, then all candidate
  branches advance in lockstep one token per batched forward — the same
  compute shape as the fork's branch pool, with the hybrid qwen3_5 state
  (conv + linear-recurrent + 6 GQA KV layers) threaded manually. The
  present→past name mapping is asymmetric: attention layers emit
  `present.<l>.key` but consume `past_key_values.<l>.key`, and `logits`
  is not the first graph output, so outputs are indexed by name.

Quantizations differ from the GGUF arm (q4 = MatMulNBits vs Q4_K_M), so
accuracy parity is expected, not exact-probability parity. `quantized`
(int8 MatMulInteger) is supported but measured ~20x slower on this CPU;
use it only for spot checks.

Usage:
  conv-venv/bin/python runner/run_onnx.py \
      --model-dir /nas/Temp/work/oc-model-eval/models/qwen3.5-2b-onnx \
      --variant q4 --out results/onnx__qwen35-2b-q4.json [--limit 3]
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
import time
from importlib.metadata import version as pkg_version
from pathlib import Path

import numpy as np
import onnxruntime as ort
from transformers import AutoTokenizer

from resources import ResourceMonitor
from run_llama import metrics

SUFFIX_PREFIX = '  "choice": "'
SYSTEM_HEAD = (
    "Select the requested field value from its allowed values, based on the "
    "context. Respond with the JSON value only.\n\nFields:\n"
)


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def system_text(suite: dict, item: dict) -> str:
    """compile_schema's system block for the runner's single choice field."""
    catalog = (
        "choice: " + item["question"] + "\nAllowed values: "
        + ", ".join(json.dumps(c["id"]) for c in item["candidates"])
    )
    return SYSTEM_HEAD + catalog + "\n" + suite["instructions"]


def build_prompt(tok, suite: dict, item: dict) -> str:
    """render_prompt's head + context + tail — the suffix is appended in
    `run_suite` as a separate tokenization, matching the fork's
    `tokenize(shared_text, true)` + `tokenize(suffix, false)` split (a
    single encode could merge tokens across the boundary)."""
    return tok.apply_chat_template(
        [
            {"role": "system", "content": system_text(suite, item)},
            {"role": "user", "content": item["context"]},
        ],
        add_generation_prompt=True,
        tokenize=False,
        chat_template_kwargs={"enable_thinking": False},
    ) + "{\n"


def candidate_paths(tok, item: dict) -> list[list[int]]:
    """Each candidate's JSON encoding minus the shared quote, plus "\\n"."""
    encoded = [json.dumps(c["id"]) for c in item["candidates"]]
    common = encoded[0]
    for v in encoded[1:]:
        while not v.startswith(common):
            common = common[:-1]
    return [
        tok.encode(v[len(common):] + "\n", add_special_tokens=False)
        for v in encoded
    ]


def divergence_nodes(paths: list[list[int]]) -> dict[int, dict[int, list[int]]]:
    """Per candidate: depth -> (options at that trie node, index of own token).

    A depth is a divergence node when the paths sharing this candidate's
    prefix split into more than one next token — exactly the nodes
    `finish_tree` scores.
    """
    branches: dict[tuple[int, ...], set[int]] = {}
    for p in paths:
        for d in range(len(p)):
            branches.setdefault(tuple(p[:d]), set()).add(p[d])
    nodes: dict[int, dict[int, list[int]]] = {}
    for i, p in enumerate(paths):
        for d in range(len(p)):
            opts = branches.get(tuple(p[:d]), set())
            if len(opts) > 1:
                options = sorted(opts)
                nodes.setdefault(i, {})[d] = (options, options.index(p[d]))
    return nodes


class OnnxQwen35:
    """Manual hybrid-state driver over the split embed + merged decoder."""

    KV_LAYERS = (3, 7, 11, 15, 19, 23)
    N_LAYERS = 24
    EOS = (248044, 248046)

    def __init__(self, model_dir: Path, variant: str, threads: int):
        opt = ort.SessionOptions()
        opt.intra_op_num_threads = threads
        providers = ["CPUExecutionProvider"]
        self.emb = ort.InferenceSession(
            str(model_dir / f"onnx/embed_tokens_{variant}.onnx"), opt, providers=providers
        )
        self.dec = ort.InferenceSession(
            str(model_dir / f"onnx/decoder_model_merged_{variant}.onnx"),
            opt,
            providers=providers,
        )
        self.out_names = [o.name for o in self.dec.get_outputs()]
        self.ort_version = pkg_version("onnxruntime")
        self.threads = threads

    def zero_states(self, past_len: int) -> dict[str, np.ndarray]:
        s: dict[str, np.ndarray] = {}
        for layer in range(self.N_LAYERS):
            if layer in self.KV_LAYERS:
                s[f"past_key_values.{layer}.key"] = np.zeros((1, 2, past_len, 256), np.float32)
                s[f"past_key_values.{layer}.value"] = np.zeros((1, 2, past_len, 256), np.float32)
            else:
                s[f"past_conv.{layer}"] = np.zeros((1, 6144, 4), np.float32)
                s[f"past_recurrent.{layer}"] = np.zeros((1, 16, 128, 128), np.float32)
        return s

    def present_to_past(self, outputs: list[np.ndarray]) -> dict[str, np.ndarray]:
        out = {}
        for name, t in zip(self.out_names, outputs):
            if name == "logits":
                continue
            key = (
                "past_key_values." + name.split(".", 1)[1]
                if name.startswith("present.")
                else name.replace("present", "past", 1)
            )
            out[key] = t
        return out

    def state_total_len(self, state: dict[str, np.ndarray]) -> int:
        return int(state[f"past_key_values.{self.KV_LAYERS[0]}.key"].shape[2])

    def embed(self, ids: np.ndarray) -> np.ndarray:
        return self.emb.run(["inputs_embeds"], {"input_ids": ids})[0]

    def forward(self, embeds: np.ndarray, state: dict[str, np.ndarray]):
        """One batched forward; returns (logits[b, -1, vocab], next state)."""
        batch = embeds.shape[0]
        total = self.state_total_len(state) + embeds.shape[1]
        mask = np.ones((batch, total), np.int64)
        pos = np.tile(
            np.arange(self.state_total_len(state), total, dtype=np.int64), (3, batch, 1)
        )
        outs = self.dec.run(
            None, {"inputs_embeds": embeds, "attention_mask": mask, "position_ids": pos, **state}
        )
        named = dict(zip(self.out_names, outs))
        return named["logits"][:, -1, :], self.present_to_past(outs)

    @staticmethod
    def logsoftmax_over(logits: np.ndarray, options: list[int]) -> np.ndarray:
        """log-softmax restricted to the node's allowed tokens (finish_tree)."""
        picked = logits[options]
        shifted = picked - picked.max()
        return shifted - np.log(np.exp(shifted).sum())

    def score_item(
        self, prompt_ids: list[int], paths: list[list[int]], nodes: dict
    ) -> tuple[list[float], dict]:
        """Prefill once, walk every candidate branch in lockstep, finish_tree."""
        n = len(paths)
        t0 = time.monotonic()
        state = self.zero_states(0)
        ids = np.array([prompt_ids], np.int64)
        logits, state = self.forward(self.embed(ids), state)
        prefill_ms = (time.monotonic() - t0) * 1000.0
        prefill_logits = logits[0]

        state = {k: np.repeat(v, n, axis=0) for k, v in state.items()}
        # Per-candidate next-position distribution; stale rows are never read.
        row_logits = np.repeat(prefill_logits[None, :], n, axis=0)
        path_lp = np.zeros(n)
        steps = 0
        branch_ms = 0.0
        max_len = max(len(p) for p in paths)
        rows = list(range(n))  # state row -> candidate index
        for depth in range(max_len):
            keep = [(slot, i) for slot, i in enumerate(rows) if depth < len(paths[i])]
            if not keep:
                break
            t1 = time.monotonic()
            slots = [slot for slot, _ in keep]
            cands = [i for _, i in keep]
            for i in cands:
                node = nodes.get(i, {}).get(depth)
                if node is not None:
                    options, own = node
                    path_lp[i] += self.logsoftmax_over(row_logits[i], options)[own]
            toks = np.array([[paths[i][depth]] for i in cands], np.int64)
            sub_state = {k: v[slots] for k, v in state.items()}
            logits, sub_state = self.forward(self.embed(toks), sub_state)
            for row, i in enumerate(cands):
                row_logits[i] = logits[row]
            state = sub_state
            rows = cands
            steps += 1
            branch_ms += (time.monotonic() - t1) * 1000.0
        # finish_tree: softmax over summed path log-probs, first-max wins
        best = int(np.argmax(path_lp))
        probs = np.exp(path_lp - path_lp[best])
        probs /= probs.sum()
        timing = {
            "prefill_ms": prefill_ms,
            "branch_ms": branch_ms,
            "branch_steps": steps,
            "server_ms": prefill_ms + branch_ms,
        }
        return probs.tolist(), timing


def run_suite(scorer: OnnxQwen35, tok, suite: dict, items: list[dict]) -> list[dict]:
    out = []
    suffix_ids = tok.encode(SUFFIX_PREFIX, add_special_tokens=False)
    for it in items:
        prompt_ids = tok.encode(build_prompt(tok, suite, it), add_special_tokens=False)
        prompt_ids = prompt_ids + suffix_ids
        paths = candidate_paths(tok, it)
        nodes = divergence_nodes(paths)
        probs, timing = scorer.score_item(prompt_ids, paths, nodes)
        winner = int(np.argmax(probs))
        out.append(
            {
                "id": it["id"],
                "class": it["class"],
                "answer": it["answer"],
                "pred": it["candidates"][winner]["id"],
                "prob": probs[winner],
                "probs": {c["id"]: p for c, p in zip(it["candidates"], probs)},
                "prompt_tokens": len(prompt_ids),
                "path_tokens": sum(len(p) for p in paths),
                "prefill_ms": timing["prefill_ms"],
                "server_ms": timing["server_ms"],
                "wall_ms": timing["server_ms"],
                "branch_steps": timing["branch_steps"],
            }
        )
        print(f"  {it['id']}: {out[-1]['pred']} p={probs[winner]:.3f} "
              f"{timing['server_ms']:.0f}ms", flush=True)
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--model-dir", type=Path, required=True)
    ap.add_argument("--variant", choices=("q4", "quantized"), default="q4")
    ap.add_argument("--suite", type=Path,
                    default=Path(__file__).parent.parent / "suite" / "suite.json")
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--threads", type=int, default=12)
    ap.add_argument("--limit", type=int, default=0,
                    help="run only the first N items (smoke runs)")
    ap.add_argument("--caveat", default=None,
                    help="recording condition worth flagging in summaries "
                    "(e.g. host contention during scoring); stored verbatim "
                    "in the result and surfaced by summarize.py")
    args = ap.parse_args()

    suite = json.loads(args.suite.read_text())
    items = suite["items"][: args.limit or None]
    args.out.parent.mkdir(parents=True, exist_ok=True)

    tok = AutoTokenizer.from_pretrained(str(args.model_dir))
    monitor = ResourceMonitor()
    monitor.start()
    try:
        scorer = OnnxQwen35(args.model_dir, args.variant, args.threads)
        single = run_suite(scorer, tok, suite, items)
        single_again = run_suite(scorer, tok, suite, items)
        determinism = {
            "predictions_match": all(a["pred"] == b["pred"] for a, b in zip(single, single_again)),
            "max_prob_delta": max(abs(a["prob"] - b["prob"]) for a, b in zip(single, single_again)),
        }
        model_files = {}
        for name in sorted(p.name for p in (args.model_dir / "onnx").iterdir()
                           if p.is_file() and args.variant in p.name):
            model_files[name] = sha256_file(args.model_dir / "onnx" / name)
        result = {
            "arm": "onnx_decision",
            "model": {
                "dir": str(args.model_dir),
                "variant": args.variant,
                "files": model_files,
                # Flat manifest keys are <prefix>/<file> (summarize.py reads
                # this for models.manifest.json; D14 keeps weights in-tree).
                "manifest_prefix": f"{args.model_dir.name}/onnx",
                "hf": "onnx-community/Qwen3.5-2B-ONNX",
            },
            "suite_sha256": hashlib.sha256(args.suite.read_bytes()).hexdigest(),
            "config": {
                "threads": args.threads,
                "mode": "tree",
                "device": "cpu",
                "provider": "CPUExecutionProvider",
                "onnxruntime": scorer.ort_version,
                "state": "manual hybrid (conv+recurrent+GQA), present->past remapped",
            },
            "single": single,
            "determinism": determinism,
            "batched": None,
            **({"caveat": args.caveat} if args.caveat else {}),
            "resources": monitor.report(),
            "metrics": metrics(single),
        }
    finally:
        monitor.stop()

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
