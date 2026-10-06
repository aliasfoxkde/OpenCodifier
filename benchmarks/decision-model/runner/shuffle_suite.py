#!/usr/bin/env python3
"""Derived probe suite: deterministically shuffle each item's context chunks.

Order-sensitivity OOD probe for Board B. The locked suite (suite.json) keeps
every fact in a conventional position — constraints line first, catalog
lines after, causal sentences in narration order. This generator shuffles
those chunks (newline-delimited lines, sentence-split within a line) under a
fixed per-item seed, leaving question/candidates/answer byte-identical. A
system that decides on extracted facts should barely move; one that leans on
positional conventions should degrade on metadata_match and
relational_compositional. Single-chunk contexts (most lexical_semantic
items) pass through unchanged and act as an invariance control: the engine
must reproduce its baseline numbers on them exactly.

The output suite is a probe artifact, not a locked suite: it records its
lineage (digest of the source suite, transform, seed) in-suite.

Usage: python3 runner/shuffle_suite.py [--suite suite/suite.json]
                                      [--out suite/suite_shuffled.json]
"""

from __future__ import annotations

import argparse
import hashlib
import json
import random
import re
from pathlib import Path

TRANSFORM = "context chunk shuffle (lines, sentence-split within lines)"


def chunks_of(context: str) -> list[str]:
    """Split into movable units: lines, then sentences inside each line."""
    out: list[str] = []
    for line in context.split("\n"):
        parts = [p.strip() for p in re.split(r"(?<=[.!?])\s+", line) if p.strip()]
        out.extend(parts if parts else [line.strip()])
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    here = Path(__file__).resolve().parent
    ap.add_argument("--suite", type=Path, default=here.parent / "suite" / "suite.json")
    ap.add_argument("--out", type=Path, default=here.parent / "suite" / "suite_shuffled.json")
    args = ap.parse_args()

    raw = args.suite.read_bytes()
    suite = json.loads(raw)
    digest = hashlib.sha256(raw).hexdigest()[:16]
    base_seed = int(suite["seed"])
    items = suite["items"]

    out_items = []
    changed = {}
    for it in items:
        chunks = chunks_of(it["context"])
        rng = random.Random(f"{base_seed}:{it['id']}")
        shuffled = list(chunks)
        rng.shuffle(shuffled)
        # Content-preservation check: same multiset of chunks, no loss.
        if sorted(chunks) != sorted(shuffled):
            raise SystemExit(f"chunk loss on {it['id']}")
        new_ctx = "\n".join(shuffled)
        if new_ctx != it["context"]:
            changed[it["class"]] = changed.get(it["class"], 0) + 1
        out_items.append({**it, "context": new_ctx})

    out = {
        "instructions": suite["instructions"],
        "items": out_items,
        "seed": base_seed,
        "suite_version": suite["suite_version"],
        "derived_from": {
            "suite": args.suite.name,
            "sha256_16": digest,
            "transform": TRANSFORM,
            "per_item_seed": f"{base_seed}:<item id>",
        },
    }
    args.out.write_text(json.dumps(out, indent=1) + "\n")

    n = len(items)
    print(f"{n} items -> {args.out} (source {args.suite.name}@{digest})")
    for cls in sorted(changed):
        print(f"  {cls}: {changed[cls]}/{sum(1 for i in items if i['class'] == cls)} contexts reordered")
    for cls in sorted({i["class"] for i in items} - set(changed)):
        print(f"  {cls}: 0 reordered (pure control)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
