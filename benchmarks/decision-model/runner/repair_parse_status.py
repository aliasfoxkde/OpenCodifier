#!/usr/bin/env python3
"""Repair mislabeled decision rows by re-parsing the kept raw text.

extract_local.py before 2026-10-06 lowercased every parsed verdict value,
so any row whose answer space was case-sensitive (the choice family's
"A".."D") failed membership and was recorded ``parse_status: json_error``
even when the model's raw output was valid JSON. The raw text is kept on
every row, so those verdicts are fully recoverable offline: this tool
re-parses each row's raw against the row's canonical value space, applies
the same exact-match-then-casefold rule as the fixed lane parser, and
rewrites ``parse_status``/``decision``. Rows whose raw genuinely fails
(value outside the space, malformed JSON, bad p) keep their original
status and are counted — nothing is smoothed.

Usage:
    python3 runner/repair_parse_status.py \
        --rows  ~/vivere_corpora/choice_full_v1/rows.jsonl \
        --values A,B,C,D \
        --out   ~/vivere_corpora/choice_full_v1/rows-repaired.jsonl
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


def parse_raw(raw: str, values: tuple[str, ...]) -> tuple[str, dict | None]:
    """Mirror the fixed lane parser: exact value match, then casefold to
    the row's canonical spelling; strict numeric p in [0, 1]."""
    stripped = raw.strip()
    if not stripped:
        return "empty", None
    try:
        obj = json.loads(stripped)
    except json.JSONDecodeError:
        i, j = stripped.find("{"), stripped.rfind("}")
        if i < 0 or j <= i:
            return "json_error", None
        try:
            obj = json.loads(stripped[i:j + 1])
        except json.JSONDecodeError:
            return "json_error", None
    if not isinstance(obj, dict):
        return "json_error", None
    value = str(obj.get("value", "")).strip()
    if value not in values:
        canon = {v.lower(): v for v in values}
        value = canon.get(value.lower(), value)
    p = obj.get("p")
    if (value not in values or not isinstance(p, (int, float))
            or isinstance(p, bool) or not 0.0 <= float(p) <= 1.0):
        return "json_error", None
    return "ok", {"value": value, "p": float(p)}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--rows", type=Path, required=True)
    ap.add_argument("--values", required=True,
                    help="comma-separated canonical value space, e.g. A,B,C,D")
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()
    values = tuple(v.strip() for v in args.values.split(",") if v.strip())
    if len(set(values)) != len(values) or not values:
        ap.error("--values must be non-empty and distinct")

    repaired = dropped = kept = 0
    with args.rows.open(encoding="utf-8") as src, \
            args.out.open("w", encoding="utf-8") as dst:
        for line in src:
            if not line.strip():
                continue
            row = json.loads(line)
            if row.get("parse_status") == "ok":
                kept += 1
            else:
                status, decision = parse_raw(str(row.get("raw") or ""), values)
                if status == "ok" and decision is not None:
                    row["parse_status"] = "ok"
                    row["decision"] = decision
                    row["repair"] = "reparse-raw-v1"
                    repaired += 1
                else:
                    dropped += 1
            dst.write(json.dumps(row, ensure_ascii=False) + "\n")

    print(json.dumps({
        "rows_in": kept + repaired + dropped,
        "already_ok": kept,
        "repaired": repaired,
        "still_bad": dropped,
        "values": list(values),
    }, indent=1))
    print(f"wrote {args.out}")
    if repaired == 0:
        print("nothing repaired — check --values against the corpus",
              file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
