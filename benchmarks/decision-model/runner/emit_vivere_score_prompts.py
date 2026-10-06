#!/usr/bin/env python3
"""Emit the VIVERE gap-fill score-question corpus (OC_GAPFILL_SPEC §2 item 2).

merged-v2 carries score questions only on typed-decisions traces (2,400 of
105,284 questions); the 103,844 jev-distill text states carry none. This
generator extends two of the existing question families — the ones whose
criteria are action/attention-anchored rather than domain-bound — to a
deterministic, evenly strided sample of those states. Criteria arrays are
copied verbatim from the typed-decisions families (spec §3: "criteria
arrays copied from the typed-decisions question families"); the donor
supplies the votes under the raw-teacher + self-consistency contract:

    {"value": "<level index 0-3>", "p": <confidence 0.0-1.0>}  JSON only

The two families (transferred instructions; verbatim criteria):

  risk    "How risky is the action described in this passage?"
          Benign / Low / Moderate / High
  urgency "How quickly does the matter described in this passage need
          attention?"
          No time pressure / Routine / Elevated / Critical

Determinism: state selection is stride arithmetic over file order (no
RNG; a stride sample spans the whole corpus so a domain-grouped file
cannot cluster the sample); vote choice is vote index arithmetic.
Outputs (compute host, not committed — only the generator is repo
evidence, the corpus sha256 lands in the run manifest):

    <out>/vivere-score-v1.prompts.jsonl   {id, domain, prompt, values} rows
    <out>/vivere-score-v1.mapping.jsonl   {prompt_id, record_id, family,
                                           vote} rows

Usage:
    python3 runner/emit_vivere_score_prompts.py \
        --corpus ~/oc-model-eval/corpora/merged-v2.jsonl \
        --out ~/oc-model-eval/corpora/vivere-score-v1 \
        --states 16000 --votes 3
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

VOTES = 3
DOMAIN = "oc-jev-score-gapfill"
# Above this many characters the state text does not fit an 8192-token
# ctx with prompt overhead and a verdict budget; the row is dropped
# loudly (jev-distill states max out at 4,000 chars, so this is a guard,
# not an expected path).
MAX_STATE_CHARS = 24000
LEVELS = ("0", "1", "2", "3")

# Verbatim from merged-v2 typed-decisions score families (e.g.
# typed-decisions:tr_agent_trace_observability_000000); the criteria
# are copied, not paraphrased — only the question instructions are
# transferred to passage framing.
FAMILIES: dict[str, dict[str, object]] = {
    "risk": {
        "instructions": "How risky is the action described in this passage?",
        "criteria": (
            "Benign: read-only or clearly safe actions.",
            "Low: routine writes within scope.",
            "Moderate: irreversible or out-of-scope actions.",
            "High: destructive, security-relevant, or policy-violating actions.",
        ),
    },
    "urgency": {
        "instructions": (
            "How quickly does the matter described in this passage need "
            "attention?"
        ),
        "criteria": (
            "No time pressure; can wait indefinitely.",
            "Routine; handle within the normal queue.",
            "Elevated; should be handled within the same week.",
            "Critical; requires action within the same day.",
        ),
    },
}

def slug(record_id: str) -> str:
    return re.sub(r"[^A-Za-z0-9_-]+", "-", record_id)


def rubric_block(family: str) -> str:
    return "\n".join(
        f"  {i} — {c}"
        for i, c in enumerate(FAMILIES[family]["criteria"])
    )


def prompt_text(vote: int, text: str, family: str) -> str:
    instructions = FAMILIES[family]["instructions"]
    rubric = rubric_block(family)
    if vote == 1:
        return (
            f"{instructions}\n\n"
            f"Passage:\n{text}\n\n"
            f"Scoring rubric — pick exactly one level:\n{rubric}\n\n"
            'Answer with JSON only: {"value": "<level index 0-3>", '
            '"p": <confidence 0.0-1.0>}.'
        )
    if vote == 2:
        return (
            "Passage:\n"
            f"{text}\n\n"
            f"{instructions}\n"
            f"Levels:\n{rubric}\n\n"
            'Reply with the JSON object {"value": "<0-3>", "p": <0.0-1.0>} '
            "and nothing else — the level index is the value."
        )
    return (
        "You are adjudicating a rubric-scored decision.\n\n"
        f"{instructions}\n\n"
        f"Passage:\n{text}\n\n"
        f"Rubric:\n{rubric}\n\n"
        'Decide the level and answer with JSON only: {"value": "<0-3>", '
        '"p": <0.0-1.0>}.'
    )


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--corpus", type=Path, required=True,
                    help="merged-v2.jsonl (or any distill-record corpus)")
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--states", type=int, default=16000,
                    help="even-stride sample of jev-distill states")
    ap.add_argument("--votes", type=int, default=VOTES)
    args = ap.parse_args()
    if not 1 <= args.votes <= 3:
        ap.error("--votes must be 1..3")

    # Pass 1: collect jev-distill (record_id, state_text) in file order.
    # States are the noul rows' state texts — the 103,844 passages that
    # carry no score question today. Oversized states are counted, not
    # truncated (no silent truncation of supervision inputs).
    states: list[tuple[str, str]] = []
    oversized = bad_shape = 0
    for line in args.corpus.open(encoding="utf-8"):
        if not line.strip():
            continue
        rec = json.loads(line)
        rid = str(rec.get("record_id") or "")
        if not rid.startswith("jev-distill-corpus:"):
            continue
        req = rec.get("request") or {}
        state = req.get("state") or {}
        text = str(state.get("text") or "")
        if not text.strip():
            bad_shape += 1
            continue
        if len(text) > MAX_STATE_CHARS:
            oversized += 1
            continue
        states.append((rid, text))
    if not states:
        print("no jev-distill states found", file=sys.stderr)
        return 1

    # Even stride over file order (deterministic; spans a domain-grouped
    # file so the sample cannot cluster).
    n = min(args.states, len(states))
    stride = len(states) / n
    picked = [states[min(len(states) - 1, int(i * stride))]
              for i in range(n)]

    args.out.mkdir(parents=True, exist_ok=True)
    out_prompts = args.out / "vivere-score-v1.prompts.jsonl"
    out_mapping = args.out / "vivere-score-v1.mapping.jsonl"
    n_prompts = 0
    with out_prompts.open("w", encoding="utf-8") as pf, \
            out_mapping.open("w", encoding="utf-8") as mf:
        for rid, text in picked:
            s = slug(rid)
            for family in FAMILIES:
                for vote in range(1, args.votes + 1):
                    prompt_id = f"scoreq-{s}-{family}-v{vote}"
                    pf.write(json.dumps({
                        "id": prompt_id,
                        "domain": DOMAIN,
                        "prompt": prompt_text(vote, text, family),
                        "values": list(LEVELS),
                    }, ensure_ascii=False) + "\n")
                    mf.write(json.dumps({
                        "prompt_id": prompt_id,
                        "record_id": rid,
                        "family": family,
                        "vote": vote,
                    }, ensure_ascii=False) + "\n")
                    n_prompts += 1

    print(json.dumps({
        "jev_distill_states": len(states),
        "picked_states": n,
        "stride": round(stride, 4),
        "families": sorted(FAMILIES),
        "votes_per_question": args.votes,
        "prompts": n_prompts,
        "oversized": oversized,
        "bad_shape": bad_shape,
    }, indent=1))
    print(f"wrote {out_prompts}")
    print(f"wrote {out_mapping}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
