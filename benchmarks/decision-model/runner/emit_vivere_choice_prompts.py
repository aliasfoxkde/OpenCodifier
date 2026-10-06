#!/usr/bin/env python3
"""Emit the VIVERE gap-fill choice-question corpus (OC_GAPFILL_SPEC §2 item 3).

merged-v2 carries choice questions only on typed-decisions traces (2,040
of 105,284 questions); the 103,844 jev-distill text states carry none.
This generator turns each sampled state's own relevance pairing into a
candidate-conditioned choice: the passage plus four candidate questions
— the state's TRUE query (gold by construction: the corpus paired them)
and three distractor queries drawn deterministically from other records
— and asks which one the passage answers. No gold is invented: the
true pairing is corpus structure, distractors are other records'
queries, and the donor votes decide admission under the raw-teacher +
self-consistency contract (OC_GAPFILL_SPEC §3: choice is emitted only
from k-vote agreement, single-vote answers dropped, never smoothed).

Position control: the true candidate's position is idx % 4 over file
order, and each vote renders the candidates under a further rotation,
so a position-biased donor cannot pass unanimity without tracking
content. Votes map back through the per-vote letter→query mapping and
unanimity is required on the mapped QUERY, not the letter.

    {"value": "<letter A-D>", "p": <confidence 0.0-1.0>}  JSON only

Determinism: state selection is stride arithmetic over file order (no
RNG); distractor sampling is prime-multiplication arithmetic over the
deduped query pool; vote rendering is vote-index arithmetic.
Outputs (compute host, not committed — the corpus sha256 lands in the
run manifest):

    <out>/vivere-choice-v1.prompts.jsonl   {id, domain, prompt, values}
    <out>/vivere-choice-v1.mapping.jsonl   {prompt_id, record_id,
        true_query, letter_to_query, letter_pos, vote}

Usage:
    python3 runner/emit_vivere_choice_prompts.py \
        --corpus ~/oc-model-eval/corpora/merged-v2.jsonl \
        --out ~/oc-model-eval/corpora/vivere-choice-v1 \
        --states 16000 --votes 3
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

VOTES = 3
DOMAIN = "oc-jev-choice-gapfill"
# Same guard as the score emitter: a state that cannot fit the donor
# context is dropped loudly, never truncated.
MAX_STATE_CHARS = 24000
LETTERS = ("A", "B", "C", "D")
INSTRUCTIONS = "Which ONE of these questions does this passage answer?"

# Distractor sampling strides over the deduped query pool (primes, so
# consecutive states spread across the pool instead of walking it).
DISTRACT_PRIMES = (7919, 104729, 1299709)


def slug(record_id: str) -> str:
    return re.sub(r"[^A-Za-z0-9_-]+", "-", record_id)


def options_block(order: list[str]) -> str:
    return "\n".join(f"  {letter} — {q}"
                     for letter, q in zip(LETTERS, order))


def prompt_text(vote: int, text: str, order: list[str]) -> str:
    options = options_block(order)
    if vote == 1:
        return (
            f"{INSTRUCTIONS}\n\n"
            f"Passage:\n{text}\n\n"
            f"Candidate questions:\n{options}\n\n"
            'Answer with JSON only: {"value": "<letter>", '
            '"p": <confidence 0.0-1.0>}.'
        )
    if vote == 2:
        return (
            "Passage:\n"
            f"{text}\n\n"
            f"{INSTRUCTIONS}\n"
            f"Options:\n{options}\n\n"
            'Reply with the JSON object {"value": "<A-D>", '
            '"p": <0.0-1.0>} and nothing else — the letter is the value.'
        )
    return (
        "You are adjudicating a multiple-choice decision.\n\n"
        f"{INSTRUCTIONS}\n\n"
        f"Passage:\n{text}\n\n"
        f"Options:\n{options}\n\n"
        'Decide the letter and answer with JSON only: '
        '{"value": "<A-D>", "p": <0.0-1.0>}.'
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

    # Pass 1: jev-distill states with their true query, in file order,
    # plus the deduped pool of all queries for distractor sampling.
    states: list[tuple[str, str, str]] = []
    query_pool: dict[str, None] = {}
    oversized = bad_shape = no_query = 0
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
        query = str(state.get("query") or "").strip()
        if not text.strip():
            bad_shape += 1
            continue
        if not query:
            no_query += 1
            continue
        if len(text) > MAX_STATE_CHARS:
            oversized += 1
            continue
        states.append((rid, text, query))
        query_pool.setdefault(query, None)
    if not states:
        print("no jev-distill states with queries found", file=sys.stderr)
        return 1
    queries = list(query_pool)
    print(f"states={len(states)} distinct queries={len(queries)}",
          flush=True)

    # Even stride over file order (deterministic; spans a domain-grouped
    # file so the sample cannot cluster).
    n = min(args.states, len(states))
    stride = len(states) / n
    picked = [states[min(len(states) - 1, int(i * stride))]
              for i in range(n)]

    args.out.mkdir(parents=True, exist_ok=True)
    out_prompts = args.out / "vivere-choice-v1.prompts.jsonl"
    out_mapping = args.out / "vivere-choice-v1.mapping.jsonl"
    n_prompts = 0
    with out_prompts.open("w", encoding="utf-8") as pf, \
            out_mapping.open("w", encoding="utf-8") as mf:
        for i, (rid, text, true_query) in enumerate(picked):
            # Deterministic distractors: prime strides over the deduped
            # pool, rejecting collisions and anything equal to the true
            # query (a distractor identical to the true question would
            # make the item genuinely ambiguous).
            distractors: list[str] = []
            for j, prime in enumerate(DISTRACT_PRIMES):
                k = (i * prime + (j + 1) * 104729) % len(queries)
                q = queries[k]
                if q != true_query and q not in distractors:
                    distractors.append(q)
            if len(distractors) < 3:
                # Walk forward deterministically until the pool yields
                # three distinct non-true queries.
                step = 0
                while len(distractors) < 3 and step < len(queries):
                    q = queries[(i * DISTRACT_PRIMES[0] + step) % len(queries)]
                    if q != true_query and q not in distractors:
                        distractors.append(q)
                    step += 1
            if len(distractors) < 3:
                print(f"WARNING: {rid}: no 3 distractors, skipped",
                      file=sys.stderr)
                continue
            # Base order puts the true query at position i % 4.
            base = list(distractors)
            pos = i % 4
            base.insert(pos, true_query)
            s = slug(rid)
            for vote in range(1, args.votes + 1):
                rotation = vote % 4
                order = [base[(rotation + k) % 4] for k in range(4)]
                letter_to_query = dict(zip(LETTERS, order))
                letter_pos = {letter: k for k, letter in enumerate(LETTERS)}
                prompt_id = f"choiceq-{s}-v{vote}"
                pf.write(json.dumps({
                    "id": prompt_id,
                    "domain": DOMAIN,
                    "prompt": prompt_text(vote, text, order),
                    "values": list(LETTERS),
                }, ensure_ascii=False) + "\n")
                mf.write(json.dumps({
                    "prompt_id": prompt_id,
                    "record_id": rid,
                    "true_query": true_query,
                    "letter_to_query": letter_to_query,
                    "letter_pos": letter_pos,
                    "true_letter": next(
                        letter for letter, q in letter_to_query.items()
                        if q == true_query),
                    "vote": vote,
                }, ensure_ascii=False) + "\n")
                n_prompts += 1

    print(json.dumps({
        "jev_distill_states": len(states),
        "picked_states": n,
        "stride": round(stride, 4),
        "distinct_query_pool": len(queries),
        "votes_per_question": args.votes,
        "prompts": n_prompts,
        "oversized": oversized,
        "bad_shape": bad_shape,
        "no_query": no_query,
    }, indent=1))
    print(f"wrote {out_prompts}")
    print(f"wrote {out_mapping}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
