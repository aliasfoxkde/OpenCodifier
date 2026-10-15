#!/usr/bin/env python3
"""Coverage-floor gate for the CI coverage lane.

Reads an lcov tracefile (cargo llvm-cov --lcov) and fails below the
documented floor on the `DA` basis — the per-line truth docs/COVERAGE.md
records its headline against (98.97 % at the Phase 18 close-out). The
floor sits below the headline by design: it is a regression guard, not a
restatement of the aspiration; the 99 % target and the waived residual
live in COVERAGE.md with their reasons.

Usage: coverage_floor.py <lcov.info> [--floor 98.5]
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path


def totals(tracefile: Path) -> tuple[int, int]:
    """(hit, found) summed over all SF records' DA lines."""
    hit = found = 0
    in_record = False
    for raw in tracefile.read_text(errors="replace").splitlines():
        if raw.startswith("SF:"):
            in_record = True
        elif raw == "end_of_record":
            in_record = False
        elif in_record and raw.startswith("DA:"):
            # DA:<line>,<count>[,<checksum>] — a count of 0 is a missed
            # line; the optional checksum field is legal lcov and ignored
            fields = raw.split(",")
            if len(fields) < 2 or not fields[1].strip().isdigit():
                continue
            found += 1
            if int(fields[1]) > 0:
                hit += 1
    return hit, found


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("tracefile", type=Path)
    ap.add_argument("--floor", type=float, default=98.5)
    args = ap.parse_args()

    hit, found = totals(args.tracefile)
    if found == 0:
        print("coverage_floor: tracefile has no DA lines — is it an lcov export?")
        return 2
    pct = 100.0 * hit / found
    print(f"coverage_floor: {hit}/{found} lines hit = {pct:.2f}% (floor {args.floor:.1f}%)")
    if pct < args.floor:
        print(f"coverage_floor: FAIL — below the {args.floor:.1f}% regression floor")
        return 1
    print("coverage_floor: PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
