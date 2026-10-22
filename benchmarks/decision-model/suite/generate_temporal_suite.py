#!/usr/bin/env python3
"""Generate the temporal-ordering suite: the engine's date-reasoning slice.

`suite.json` and its holdout siblings carry three decision classes —
metadata match, lexical/semantic, and relational/compositional. The
relational class is generated from the same grammar the deterministic
solver proves over (dependency chains, health counts, bring-up order).
This file adds the fourth class the solver gained with its temporal
operators (`relational-v2`): items whose context states dates and whose
answer is decided by ordering those dates.

Two families, one per temporal operator:

- **deadline** — every entity carries a deadline ("... is due by
  <date>"); the answer is the one whose deadline falls strictly first.
- **event** — every entity carries an event ("... completed on
  <date>"); the answer is the one whose event falls strictly last.

The grammar is deliberately narrow (see `crates/opencodifier-engine/
src/facts.rs`): the context sentences are exactly what the extractor
accepts, so a failure to answer is a failure of the operator, not of
the phrasing. Entity names are date-free and the dates are unique
within an item, so neither the answer nor its date is lexically
salient — the dates cannot be recovered by lexical overlap alone.

The file is written to `suite_temporal_v1.json` beside this script. It
is a *new* artifact: the pinned suites are not regenerated, so the
release-gate numbers stay comparable.

Usage:
  python3 suite/generate_temporal_suite.py [--out suite_temporal_v1.json]
"""

from __future__ import annotations

import argparse
import json
import random
import sys
from datetime import date, timedelta
from pathlib import Path

SEED = 20261021
VERSION = 1

# Entity pools per family. Names are name-shaped (`[A-Za-z0-9_-]`) and
# carry no date, so the answer cannot be spotted by lexical overlap.
_DEADLINE_ENTITIES = [
    "invoice-1042",
    "invoice-2317",
    "contract-88",
    "contract-91",
    "tax-filing-2026",
    "warranty-4471",
    "audit-17",
    "audit-22",
    "permit-305",
    "permit-311",
]

_EVENT_ENTITIES = [
    "deploy-771",
    "deploy-784",
    "rollback-12",
    "rollback-19",
    "migration-303",
    "migration-318",
    "incident-55",
    "incident-62",
    "audit-41",
    "audit-46",
]

_DEADLINE_QUESTION = "Which deadline falls soonest?"
_EVENT_QUESTION = "Which event happened most recently?"

_START = date(2026, 1, 5)
_SPAN_DAYS = 330


def _dates(rng: random.Random, count: int) -> list[date]:
    """`count` distinct dates inside the window, in ascending order."""
    offsets = sorted(rng.sample(range(_SPAN_DAYS), count))
    return [_START + timedelta(days=offset) for offset in offsets]


def _item(
    item_id: str,
    context: str,
    question: str,
    cands: list[str],
    answer: str,
) -> dict:
    return {
        "id": item_id,
        "class": "temporal_ordering",
        "context": context,
        "question": question,
        "candidates": [{"id": name, "description": f"item {name}"} for name in cands],
        "answer": answer,
    }


def _deadline_item(rng: random.Random, item_id: str) -> dict:
    names = rng.sample(_DEADLINE_ENTITIES, 4)
    dates = _dates(rng, len(names))
    # The soonest deadline is the answer; the other three are all later.
    # Every candidate is named in the context, so the item cannot be
    # answered by spotting which candidate was mentioned at all — only
    # by ordering the dates.
    answer = names[0]
    lines = [f"{name} is due by {due.isoformat()}." for name, due in zip(names, dates)]
    rng.shuffle(lines)
    return _item(item_id, " ".join(lines), _DEADLINE_QUESTION, sorted(names), answer)


def _event_item(rng: random.Random, item_id: str) -> dict:
    names = rng.sample(_EVENT_ENTITIES, 4)
    dates = _dates(rng, len(names))
    # The most recent event is the answer; the other three are all
    # earlier. The dates run the other way round from the deadline
    # family, so the two operators cannot be confused.
    answer = names[-1]
    lines = [f"{name} completed on {when.isoformat()}." for name, when in zip(names, dates)]
    rng.shuffle(lines)
    return _item(item_id, " ".join(lines), _EVENT_QUESTION, sorted(names), answer)


def build_suite(seed: int = SEED, per_family: int = 50) -> dict:
    rng = random.Random(seed)
    items: list[dict] = []
    for k in range(per_family):
        items.append(_deadline_item(rng, f"T-00{k:03d}"))
    for k in range(per_family):
        items.append(_event_item(rng, f"T-01{k:03d}"))
    rng.shuffle(items)
    return {
        "instructions": (
            "You are choosing from a fixed list of options. Read the context and answer "
            "with exactly one option id from the allowed choices. Do not invent options. "
            "If unsure, still pick the closest option."
        ),
        "suite_version": VERSION,
        "class": "temporal_ordering",
        "items": items,
    }


def main() -> int:
    here = Path(__file__).parent
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, default=here / "suite_temporal_v1.json")
    ap.add_argument("--seed", type=int, default=SEED)
    ap.add_argument("--per-family", type=int, default=50)
    args = ap.parse_args()

    suite = build_suite(seed=args.seed, per_family=args.per_family)
    args.out.write_text(json.dumps(suite, indent=1, sort_keys=True) + "\n", encoding="utf-8")
    sys.stderr.write(f"wrote {args.out} ({len(suite['items'])} items)\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
