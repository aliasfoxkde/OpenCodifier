#!/usr/bin/env python3
"""Derive the long-context suite variant: every item padded with filler.

The committed suite's contexts are short (p50 ≈ 20 words), so focused
extraction never engages on them — they sit far under any token budget.
This generator derives `suite_long.json`: the same 120 items, same gold
answers, same candidates, with each context wrapped in deterministic
distractor prose (~2,400 estimated tokens per item) so the focused path
has real work to do.

The distractors are built to be *ignorable*: their vocabulary is disjoint
from every item's question and candidate descriptions (asserted, not
assumed), they name no candidate id, and they match none of the
relational fact grammar's patterns — so the only way they can influence a
decision is by diluting the context, which is exactly what the focused
A/B measures.

Usage:
  python3 runner/make_long_suite.py   # writes suite/suite_long.json
"""

from __future__ import annotations

import json
import random
import re
import sys
from pathlib import Path

SUITE = Path(__file__).parent.parent / "suite" / "suite.json"
OUT = Path(__file__).parent.parent / "suite" / "suite_long.json"

# Per-item padding: sentences before and after the original context. Each
# sentence is 10-16 words, so 70+70 sentences is roughly 2,400 estimated
# tokens (bytes-over-four) — far above the engine's default 512 budget.
PAD_BEFORE = 70
PAD_AFTER = 70

# Distractor templates. Words are chosen from the language of meetings,
# ledgers, and filing cabinets — deliberately far from the suite's
# domains (deployments, billing, components, support queues). The
# disjointness is asserted per item below; if a future suite class shares
# vocabulary with these templates, the generator fails loudly rather than
# producing a quietly contaminated benchmark.
TEMPLATES = [
    "Ledger note {n}: the quarterly archive lists correspondence, {a} and {b}.",
    "Appendix {n}: procedural paperwork, {a}, {b} and filing queues were reviewed.",
    "Minute {n}: committee attendance was recorded and the {a} discussion adjourned.",
    "Register {n}: the custodian catalogued {a} alongside {b} without further action.",
    "Bulletin {n}: administrative circulars covered {a} and {b} for the quarter.",
    "Summary {n}: clerical routines processed {a}, {b} and routine correspondence.",
]

FILLER_A = [
    "inventories", "reimbursements", "archived memoranda", "filing queues",
    "stationery totals", "committee rosters", "custodial schedules",
    "circulation lists", "retention schedules", "registry updates",
    "notarised copies", "draft circulars", "postal manifests",
    "shelf audits", "binding orders",
]

FILLER_B = [
    "scheduling minutiae", "unrelated correspondence", "administrative overhead",
    "procedural minutiae", "logistics paperwork", "archival minutiae",
    "vendor contracts", "travel reimbursements", "quarterly circulars",
    "registry errata", "filing backlogs", "clerical rotations",
    "attendance rosters", "room bookings", "photocopy quotas",
]


# The engine's own stopword list (crates/opencodifier-engine/src/lexical.rs
# STOP_WORDS): BM25 never scores these tokens, so disjointness is asserted
# over content words only. Kept verbatim; the A/B fails loudly if the two
# ever drift (a colliding stopword would make distractors marginally less
# ignorable, not silently wrong).
STOP_WORDS = frozenset(
    "a an the and or but if then else when of to in on for with is are was "
    "were be been this that these those it its as at by from into do does "
    "did which what how should".split()
)


def item_vocabulary(item: dict) -> set[str]:
    """Every *content* word the focus query and classifier can match on."""
    words: set[str] = set()
    for text in [item["question"], item["context"]]:
        words.update(re.findall(r"[A-Za-z0-9_]+", text.lower()))
    for candidate in item["candidates"]:
        words.update(re.findall(r"[A-Za-z0-9_]+", candidate["description"].lower()))
        words.add(candidate["id"].lower())
    return words - STOP_WORDS


def distractor_sentences(item: dict, index: int) -> list[str]:
    """`PAD_BEFORE + PAD_AFTER` filler sentences, deterministic per item."""
    rng = random.Random(f"{item['id']}-{index}")
    vocabulary = item_vocabulary(item)
    sentences: list[str] = []
    for position in range(PAD_BEFORE + PAD_AFTER):
        for _attempt in range(64):
            template = rng.choice(TEMPLATES)
            # Zero-padded note numbers: bare small integers can collide
            # with digits that appear in a context ("replicas=2").
            sentence = template.format(
                n=f"{position:04d}", a=rng.choice(FILLER_A), b=rng.choice(FILLER_B)
            )
            words = set(re.findall(r"[A-Za-z0-9_]+", sentence.lower()))
            if words.isdisjoint(vocabulary):
                sentences.append(sentence)
                break
        else:
            raise SystemExit(
                f"item {item['id']}: no disjoint distractor after 64 attempts"
            )
    return sentences


def main() -> int:
    suite = json.loads(SUITE.read_text())
    items = []
    for index, item in enumerate(suite["items"]):
        padding = distractor_sentences(item, index)
        before = " ".join(padding[:PAD_BEFORE])
        after = " ".join(padding[PAD_BEFORE:])
        items.append(
            {
                **item,
                "id": f"{item['id']}-long",
                "context": f"{before}\n{item['context']}\n{after}",
            }
        )
    out = {
        "suite_version": f"{suite['suite_version']}+long",
        "seed": suite["seed"],
        "instructions": suite["instructions"],
        "derived_from": "suite.json",
        "derivation": (
            f"each context padded with {PAD_BEFORE}+{PAD_AFTER} distractor "
            "sentences, vocabulary-disjoint from question, context and "
            "candidates; gold answers and candidates unchanged"
        ),
        "items": items,
    }
    OUT.write_text(json.dumps(out, indent=1) + "\n")

    lengths = [len(i["context"]) // 4 for i in items]
    print(
        f"{OUT}: {len(items)} items, "
        f"estimated tokens p50 {sorted(lengths)[len(lengths) // 2]}, "
        f"min {min(lengths)}, max {max(lengths)}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
