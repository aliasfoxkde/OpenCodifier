#!/usr/bin/env python3
"""Build the #124 quarantine-A/B arm-B training rows.

Arm B = the pinned arm-A SFT rows (merged-v3 as trained for r2)
plus the merged-v2 quarantine pool rows that arm A actually DROPPED,
re-admitted verbatim at a 0.05 training-loss weight through the
``_train_weight`` field (trainer --row-weight-field _train_weight).

ONE variable vs arm A: the dropped pool remainder at 0.05. Recipe,
row order, split seed, and evals are identical.

Pool accounting (why the remainder, not the whole file): the
quarantine pool holds 38,747 mixed-label cluster members. merged-v3
already re-admitted the recovered subset at weight 1.0 (qradj-v1,
28,937 records). Re-adding those at 0.05 would up-weight them to an
effective 1.05 -- a second, unintended variable. The pool members
whose record_id already appears in the arm-A rows are therefore
skipped and counted; only the dropped remainder re-enters.

Gates before the corpus is written:
  - input sha256s are asserted against the pinned values when
    --expect-*-sha is given (the chain script pins both);
  - pool records are segmented by the SAME prep code
    (decision_sft_prep) with the same skip rules and per-row asserts;
  - a pool record whose canonical state hash collides with any suite
    row in the parent corpus is excluded and counted (the v2 gate
    covered surviving train rows only; the pool was never
    suite-checked);
  - every arm-A row passes through untouched and must NOT carry
    _train_weight (the treatment stays one-sided and visible);
  - output rows = arm-A rows + admitted pool rows (asserted).
"""
from __future__ import annotations

import argparse
import hashlib
import json
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from analyze_merged_dups import canon_state  # noqa: E402
from decision_sft_prep import (  # noqa: E402
    option_ids, options_for, render_header, segments_for, state_text)


def sha256_of(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def state_sha(rec: dict) -> str:
    return hashlib.sha256(
        canon_state((rec.get("request") or {}).get("state"))
        .encode("utf-8")).hexdigest()


def say(msg: str) -> None:
    sys.stdout.write(msg + "\n")
    sys.stdout.flush()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--rows-a", type=Path, required=True,
                    help="pinned arm-A segmented rows jsonl")
    ap.add_argument("--pool", type=Path, required=True,
                    help="merged-v2.quarantine.jsonl")
    ap.add_argument("--corpus", type=Path, required=True,
                    help="merged-v3.jsonl; suite rows define the "
                         "state-collision exclusion set")
    ap.add_argument("--out", type=Path, required=True,
                    help="output dir for rows + manifest")
    ap.add_argument("--expect-rows-a-sha", default="",
                    help="assert arm-A rows sha256 (chain pins this)")
    ap.add_argument("--expect-pool-sha", default="",
                    help="assert pool sha256 (chain pins this)")
    ap.add_argument("--weight", type=float, default=0.05,
                    help="_train_weight for admitted pool rows")
    ap.add_argument("--max-state-chars", type=int, default=24000)
    args = ap.parse_args()

    args.out.mkdir(parents=True, exist_ok=True)
    out_rows = args.out / "e1-quarx005.rows.jsonl"
    counts: Counter = Counter()

    rows_a_sha = sha256_of(args.rows_a)
    pool_sha = sha256_of(args.pool)
    if args.expect_rows_a_sha and rows_a_sha != args.expect_rows_a_sha:
        say(f"GATE FAIL: arm-A rows sha {rows_a_sha} != pinned "
            f"{args.expect_rows_a_sha}")
        return 1
    if args.expect_pool_sha and pool_sha != args.expect_pool_sha:
        say(f"GATE FAIL: pool sha {pool_sha} != pinned "
            f"{args.expect_pool_sha}")
        return 1

    # Suite state-collision exclusion set, from the parent corpus.
    suite_shas: set[str] = set()
    with args.corpus.open(encoding="utf-8") as src:
        for line in src:
            if not line.strip():
                continue
            rec = json.loads(line)
            if str(rec.get("source") or "").startswith("suite"):
                suite_shas.add(state_sha(rec))
    counts["suite_states_in_corpus"] = len(suite_shas)

    # Arm-A record ids (recovered pool members already train at 1.0).
    arm_a_ids: set[str] = set()
    n_arm_a = 0
    with args.rows_a.open(encoding="utf-8") as src, \
            out_rows.open("w", encoding="utf-8") as dst:
        for line in src:
            if not line.strip():
                continue
            row = json.loads(line)
            if "_train_weight" in row:
                say("GATE FAIL: arm-A row already carries _train_weight "
                    f"({row.get('id')})")
                return 1
            arm_a_ids.add(str(row.get("record_id") or ""))
            dst.write(line if line.endswith("\n") else line + "\n")
            n_arm_a += 1
        counts["arm_a_rows"] = n_arm_a

        # Segment the pool with the prep code path, gate, append.
        with args.pool.open(encoding="utf-8") as psrc:
            for line in psrc:
                if not line.strip():
                    continue
                rec = json.loads(line)
                rid = str(rec.get("record_id") or "")
                source = str(rec.get("source") or "")
                if source.startswith("suite"):
                    counts["skipped: suite row"] += 1
                    continue
                if rid in arm_a_ids:
                    counts["skipped: already in arm A (recovered)"] += 1
                    continue
                state = state_text(rec)
                if not state:
                    counts["skipped: empty state"] += 1
                    continue
                if len(state) > args.max_state_chars:
                    counts["skipped: state over budget"] += 1
                    continue
                if state_sha(rec) in suite_shas:
                    counts["skipped: suite state collision"] += 1
                    continue
                req = rec.get("request") or {}
                for qname, q in (req.get("questions") or {}).items():
                    qtype = q.get("type")
                    options = options_for(q)
                    if options is None:
                        counts[f"skipped: unrenderable {qtype} "
                               f"criteria"] += 1
                        continue
                    ids = option_ids(q)
                    target = ((rec.get("target") or {}).get(qname) or {})
                    label = str(target.get("label") or "")
                    if target.get("type") != qtype or label not in ids:
                        counts["skipped: target label outside "
                               "options"] += 1
                        continue
                    instructions = str(q.get("instructions") or "").strip()
                    if not instructions:
                        counts["skipped: empty instructions"] += 1
                        continue
                    segs = ([{"t": render_header(qtype, instructions,
                                                options, state), "y": 0}]
                            + segments_for(ids, options, label))
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
                        "_train_weight": args.weight,
                    }, ensure_ascii=False) + "\n")
                    counts["emitted: pool"] += 1

    total = n_arm_a + counts["emitted: pool"]
    manifest = {
        "manifest_version": "opencodifier.e1-quarx-build/1",
        "treatment": f"dropped quarantine remainder at "
                     f"_train_weight={args.weight}",
        "rows_a": {"path": args.rows_a.name, "sha256": rows_a_sha,
                   "rows": n_arm_a},
        "pool": {"path": args.pool.name, "sha256": pool_sha},
        "corpus_suite_set": {"path": args.corpus.name,
                             "suite_states": len(suite_shas)},
        "rows_out": total,
        "counts": dict(sorted(counts.items())),
        "sha256": {"e1-quarx005.rows.jsonl": sha256_of(out_rows)},
    }
    (args.out / "e1-quarx005.manifest.json").write_text(
        json.dumps(manifest, indent=1) + "\n")
    say(json.dumps({k: manifest[k] for k in
                    ("treatment", "rows_a", "rows_out", "counts",
                     "sha256")}, indent=1))
    if counts["emitted: pool"] == 0:
        say("no pool rows admitted", )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
