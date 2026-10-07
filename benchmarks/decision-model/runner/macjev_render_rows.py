#!/usr/bin/env python3
"""Render corpus records to their macjev readout using the prep's own
rendering functions — the python side of the F2 byte-parity lane
(TRAINING.md Phase F2; the Rust counterpart is
``crates/opencodifier-schema/src/macjev.rs``).

Reads record lines (the ``decision_sft_prep`` input shape) on stdin and
writes one JSON line per record on stdout:

``{"outcome": ..., "state": ..., "rows": [...], "skips": [...]}``

where ``outcome`` is ``rendered`` | ``suite_row`` | ``empty_state`` |
``state_over_budget``, rows carry the prep row fields plus the
concatenated ``text``, and skips are ``[question name, reason]`` pairs
using the same reason strings the Rust renderer's ``Display`` prints.

Usage::

    python3 macjev_render_rows.py [--max-state-chars N] < records.jsonl
"""
from __future__ import annotations

import argparse
import json
import sys

from decision_sft_prep import (
    option_ids,
    options_for,
    render_header,
    segments_for,
    state_text,
)


def render_one(rec: dict, max_state_chars: int) -> dict:
    source = str(rec.get("source") or "")
    if source.startswith("suite"):
        return {"outcome": "suite_row"}
    state = state_text(rec)
    if not state:
        return {"outcome": "empty_state"}
    if len(state) > max_state_chars:
        return {"outcome": "state_over_budget", "state_chars": len(state)}
    rid = str(rec.get("record_id") or "")
    req = rec.get("request") or {}
    rows: list[dict] = []
    skips: list[list[str]] = []
    for qname, q in (req.get("questions") or {}).items():
        qtype = q.get("type")
        options = options_for(q)
        if options is None:
            skips.append([qname, "unrenderable criteria"])
            continue
        ids = option_ids(q)
        target = (rec.get("target") or {}).get(qname) or {}
        label = str(target.get("label") or "")
        if target.get("type") != qtype or label not in ids:
            skips.append([qname, "target label outside options"])
            continue
        instructions = str(q.get("instructions") or "").strip()
        if not instructions:
            skips.append([qname, "empty instructions"])
            continue
        segs = ([{"t": render_header(qtype, instructions, options,
                                    state), "y": 0}]
                + segments_for(ids, options, label))
        rows.append({
            "id": f"{rid}::{qname}",
            "question": qname,
            "qtype": qtype,
            "label": label,
            "n_options": len(options),
            "segments": segs,
            "render_chars": sum(len(s["t"]) for s in segs),
            "text": "".join(s["t"] for s in segs),
        })
    return {"outcome": "rendered", "state": state, "rows": rows,
            "skips": skips}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--max-state-chars", type=int, default=24000)
    args = ap.parse_args()
    for line in sys.stdin:
        if not line.strip():
            continue
        rec = json.loads(line)
        json.dump(render_one(rec, args.max_state_chars), sys.stdout,
                  ensure_ascii=False)
        sys.stdout.write("\n")
    sys.stdout.flush()
    return 0


if __name__ == "__main__":
    sys.exit(main())
