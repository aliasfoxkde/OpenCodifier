#!/usr/bin/env python3
"""Emit the VIVERE gap-fill quarantine re-adjudication corpus (task #110).

merged-v2 quarantined 38,747 near-duplicate records whose relevance labels
conflict (docs/TRAINING.md §9). This generator turns that pool into the
donor prompt corpus for the first gap-fill pass: per record, k=3 relevance
votes across paraphrased prompts (OC_GAPFILL_SPEC §3 contract — raw teacher
values stay the gold; donor votes only confirm). The vote contract:

    {"value": "yes" | "no", "p": <0..1>}   JSON only, no prose

Determinism: paraphrase choice is vote index arithmetic, no RNG; record
order is file order. Outputs (compute host, not committed — only the
generator is repo evidence, the corpus sha256 lands in the run manifest):

    <out>/vivere-qradj-v1.prompts.jsonl   {id, domain, prompt} rows
    <out>/vivere-qradj-v1.mapping.jsonl   {prompt_id, record_id, cluster_id,
                                           gold_label, teacher_jev} rows

The mapping is out-of-band: the donor never sees gold labels, and
aggregation joins votes back through prompt_id. Oversized states are NOT
silently truncated (no silent truncation of supervision inputs) — they are
counted and dropped into the gap report at aggregation time.

Usage:
    python3 runner/emit_vivere_gapfill_prompts.py \
        --quarantine ~/oc-model-eval/corpora/merged-v2.quarantine.jsonl \
        --out ~/oc-model-eval/corpora/vivere-qradj-v1
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

VOTES = 3
DOMAIN = "oc-quarantine-readj"
# Above this many characters the state text does not fit a 8192-token ctx
# with prompt overhead and a verdict budget; the row is dropped loudly.
MAX_STATE_CHARS = 24000

PROMPT_TEMPLATES = (
    # v1 — direct
    "Decide whether the passage satisfies the query.\n\n"
    "Passage:\n{text}\n\n"
    "Query: {query}\n\n"
    'Answer with JSON only: {{"value": "yes" or "no", "p": <confidence '
    '0.0-1.0>}} — "yes" if the passage contains information that satisfies '
    'the query, "no" otherwise.',
    # v2 — criterion-anchored
    "Passage:\n{text}\n\n"
    "Query: {query}\n\n"
    'Scoring rubric — "true": the text satisfies the query; "false": the '
    "text does not satisfy the query.\n"
    'Answer with JSON only: {{"value": "yes" or "no", "p": <confidence '
    '0.0-1.0>}} where "yes" maps to the true criterion.',
    # v3 — adjudication framing
    "You are adjudicating a relevance decision.\n\n"
    "Query: {query}\n\n"
    "Passage:\n{text}\n\n"
    'Does the passage answer or resolve the query? Reply with the JSON '
    'object {{"value": "yes" or "no", "p": <0.0-1.0>}} and nothing else.',
)


def slug(record_id: str) -> str:
    return re.sub(r"[^A-Za-z0-9_-]+", "-", record_id)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--quarantine", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--votes", type=int, default=VOTES)
    args = ap.parse_args()
    if not 1 <= args.votes <= len(PROMPT_TEMPLATES):
        ap.error(f"--votes must be 1..{len(PROMPT_TEMPLATES)}")

    args.out.mkdir(parents=True, exist_ok=True)
    out_prompts = args.out / "vivere-qradj-v1.prompts.jsonl"
    out_mapping = args.out / "vivere-qradj-v1.mapping.jsonl"

    n_records = n_prompts = oversized = bad_shape = 0
    with out_prompts.open("w", encoding="utf-8") as pf, \
            out_mapping.open("w", encoding="utf-8") as mf:
        for line in args.quarantine.open(encoding="utf-8"):
            if not line.strip():
                continue
            rec = json.loads(line)
            n_records += 1
            req = rec.get("request") or {}
            state = req.get("state") or {}
            questions = req.get("questions") or {}
            # The quarantine pool is a relevance-label fight: exactly the
            # noul question carries the gold conflict this pass adjudicates.
            q = questions.get("relevance")
            if (q is None or q.get("type") != "noul"
                    or not str(state.get("text") or "").strip()):
                bad_shape += 1
                continue
            text = str(state["text"])
            if len(text) > MAX_STATE_CHARS:
                oversized += 1
                continue
            query = str(state.get("query") or q.get("instructions") or "").strip()
            target = (rec.get("target") or {}).get("relevance") or {}
            gold = target.get("label")
            teacher = rec.get("teacher") or {}
            quaran = rec.get("_quarantine") or {}
            rid = str(rec.get("record_id") or f"row-{n_records}")
            for vote in range(1, args.votes + 1):
                prompt_id = f"qradj-{slug(rid)}-v{vote}"
                pf.write(json.dumps({
                    "id": prompt_id,
                    "domain": DOMAIN,
                    "prompt": PROMPT_TEMPLATES[vote - 1].format(
                        text=text, query=query),
                }, ensure_ascii=False) + "\n")
                mf.write(json.dumps({
                    "prompt_id": prompt_id,
                    "record_id": rid,
                    "cluster_id": quaran.get("cluster_id"),
                    "cluster_labels": quaran.get("cluster_labels"),
                    "gold_label": gold,
                    "teacher_jev": teacher.get("jev"),
                    "vote": vote,
                }, ensure_ascii=False) + "\n")
                n_prompts += 1

    print(json.dumps({
        "records": n_records,
        "prompts": n_prompts,
        "bad_shape": bad_shape,
        "oversized": oversized,
        "votes_per_record": args.votes,
    }, indent=1))
    print(f"wrote {out_prompts}")
    print(f"wrote {out_mapping}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
