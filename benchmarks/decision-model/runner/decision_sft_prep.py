#!/usr/bin/env python3
"""Render a distill-record corpus into verdict-slot SFT rows (E1, #88).

The training surface is the native verdict-slot readout (the reference
architecture for an in-house decision model, NATIVE_VERDICT_ARM.md
§"Why this arm exists" / planned §15 path): the input is rendered as
raw text in the macjev-render-v1 layout —

    State:
    <state>

    Question [<type>]: <instructions>
    Options:
    - <option 1>
    ...
    - <option K>
    Judge each option:
    <option 1> ->
    ...
    <option K> ->

— and the supervision is one verdict token per option slot: " yes" at
the labeled option's `` ->`` position, " no" at every other slot. This
is exactly what the serving path scores (logit(" yes") - logit(" no")
per slot, softmax over slots); JSON-chat supervision would repeat the
D16 interface mismatch (0.217 / 0.0 through the readout arms).

Option rendering per question type (merged-v3 shapes):
  choice  criteria dict {id: description}  ->  "<id>: <description>"
  noul    criteria dict {false, true} or absent -> "false: <c>" / "true: <c>"
  score   criteria list (index = level)    ->  "level <i>: <description>"

Rows are one training example per (record, question): parent records
carry 1-3 questions; suite rows are excluded (they are evaluation
surface and must never become training rows). Over-long states are
skipped and counted, never truncated (the lane's loud-drop rule); the
length histogram in the manifest is what sizes each arm's training
context — measured, not assumed.

Output rows carry segmented text so the trainer can mask loss to the
verdict tokens exactly:

    {"id", "record_id", "question", "qtype", "label", "source",
     "segments": [{"t": <text>, "y": 0|1}, ...]}

y=1 segments are the verdict tokens (" yes"/" no"); everything else is
context. Tokenization and EOS handling belong to the trainer.

Usage:
    python3 runner/decision_sft_prep.py \
        --corpus ~/oc-model-eval/corpora/merged-v3.jsonl \
        --out    ~/oc-model-eval/corpora/e1-sft-v1 \
        [--max-state-chars 24000]
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from collections import Counter
from pathlib import Path

LETTERS = ("A", "B", "C", "D")


def sha256_of(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def state_text(record: dict) -> str:
    """State text in the render. Corpus rows carry state as a plain
    string, as {text, ...}, or as a structured dict with no text field
    (typed-decisions traces: agent/constraints/task/...). Structured
    states serialize to JSON exactly the way the serving adapter does
    (run_jevbench: ``json.dumps(t.state)``) so the trained layout
    matches what the rung renders at inference. Truly empty skips."""
    state = (record.get("request") or {}).get("state")
    if isinstance(state, str):
        return state.strip()
    if isinstance(state, dict):
        text = state.get("text")
        if isinstance(text, str) and text.strip():
            return text.strip()
        if state:
            return json.dumps(state, ensure_ascii=False)
    return ""


def options_for(q: dict) -> list[str] | None:
    """macjev option lines in slot order; None if the question is not
    renderable (bad/missing criteria)."""
    qtype = q.get("type")
    criteria = q.get("criteria")
    if qtype == "choice":
        if not isinstance(criteria, dict) or len(criteria) < 2:
            return None
        return [f"{cid}: {str(desc).strip()}" for cid, desc in criteria.items()]
    if qtype == "noul":
        held = ""
        not_held = ""
        if isinstance(criteria, dict):
            held = str(criteria.get("true") or "").strip()
            not_held = str(criteria.get("false") or "").strip()
        return [f"false: {not_held}".rstrip(), f"true: {held}".rstrip()]
    if qtype == "score":
        if not isinstance(criteria, list) or len(criteria) < 2:
            return None
        return [f"level {i}: {str(desc).strip()}" for i, desc in enumerate(criteria)]
    return None


def option_ids(q: dict) -> list[str]:
    qtype = q.get("type")
    if qtype == "choice":
        return list(q["criteria"].keys())
    if qtype == "noul":
        return ["false", "true"]
    return [str(i) for i in range(len(q["criteria"]))]


def render_header(qtype: str, instructions: str, options: list[str],
                  state: str) -> str:
    """Everything before the first option slot; the slot lines and their
    verdict tokens are the segments' job (they carry the loss mask)."""
    opts = "\n".join(f"- {o}" for o in options)
    return (f"State:\n{state}\n\nQuestion [{qtype}]: {instructions}\n"
            f"Options:\n{opts}\nJudge each option:\n")


def segments_for(ids: list[str], options: list[str], label: str) -> list[dict]:
    """Interleaved segments: context up to and including each option's
    `` ->``, then the verdict token as its own y=1 segment. The label is
    an option ID (candidate id / false|true / level index), so the yes
    slot is found by position over the ids — never by string-matching
    the rendered option line."""
    yes_at = ids.index(label)
    segs: list[dict] = []
    for i, opt in enumerate(options):
        sep = "" if i == 0 else "\n"
        segs.append({"t": f"{sep}{opt} ->", "y": 0})
        segs.append({"t": " yes" if i == yes_at else " no", "y": 1})
    segs.append({"t": "\n", "y": 0})
    return segs


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--corpus", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--max-state-chars", type=int, default=24000,
                    help="rows with longer states are skipped and "
                         "counted, never truncated")
    args = ap.parse_args()

    args.out.mkdir(parents=True, exist_ok=True)
    out_rows = args.out / "e1-sft-v1.rows.jsonl"

    counts: Counter = Counter()
    len_buckets: Counter = Counter()
    n_out = 0
    with args.corpus.open(encoding="utf-8") as src, \
            out_rows.open("w", encoding="utf-8") as dst:
        for line in src:
            if not line.strip():
                continue
            rec = json.loads(line)
            rid = str(rec.get("record_id") or "")
            source = str(rec.get("source") or "")
            if source.startswith("suite"):
                counts["skipped: suite row"] += 1
                continue
            state = state_text(rec)
            if not state:
                counts["skipped: empty state"] += 1
                continue
            if len(state) > args.max_state_chars:
                counts["skipped: state over budget"] += 1
                len_buckets["over"] += 1
                continue
            bucket = min(len(state) // 4000, 5)
            len_buckets[f"<={bucket + 1}x4k"] += 1
            req = rec.get("request") or {}
            for qname, q in (req.get("questions") or {}).items():
                qtype = q.get("type")
                options = options_for(q)
                if options is None:
                    counts[f"skipped: unrenderable {qtype} criteria"] += 1
                    continue
                ids = option_ids(q)
                target = (rec.get("target") or {}).get(qname) or {}
                label = str(target.get("label") or "")
                if target.get("type") != qtype or label not in ids:
                    counts["skipped: target label outside options"] += 1
                    continue
                instructions = str(q.get("instructions") or "").strip()
                if not instructions:
                    counts["skipped: empty instructions"] += 1
                    continue
                segs = ([{"t": render_header(qtype, instructions, options,
                                            state), "y": 0}]
                        + segments_for(ids, options, label))
                # Gate: every option slot gets exactly one verdict, and
                # exactly one slot says yes.
                assert sum(s["y"] for s in segs) == len(options), rid
                assert sum(1 for s in segs
                           if s["y"] and s["t"] == " yes") == 1, rid
                dst.write(json.dumps({
                    "id": f"{rid}::{qname}",
                    "record_id": rid,
                    "question": qname,
                    "qtype": qtype,
                    "label": label,
                    "n_options": len(options),
                    "source": source,
                    "segments": segs,
                    "render_chars": sum(len(s["t"]) for s in segs),
                }, ensure_ascii=False) + "\n")
                counts[f"emitted: {qtype}"] += 1
                n_out += 1

    manifest = {
        "manifest_version": "opencodifier.e1-sft-prep/1",
        "readout": "macjev-render-v1 slots, yes/no verdict tokens",
        "corpus": {"path": args.corpus.name, "sha256": sha256_of(args.corpus)},
        "max_state_chars": args.max_state_chars,
        "rows_out": n_out,
        "counts": dict(sorted(counts.items())),
        "state_len_buckets": dict(sorted(len_buckets.items())),
        "sha256": {"e1-sft-v1.rows.jsonl": sha256_of(out_rows)},
    }
    (args.out / "e1-sft-v1.manifest.json").write_text(
        json.dumps(manifest, indent=1) + "\n")
    print(json.dumps({k: manifest[k] for k in
                      ("rows_out", "counts", "state_len_buckets", "sha256")},
                     indent=1))
    if n_out == 0:
        print("no rows emitted", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
