#!/usr/bin/env python3
"""Build the e1-softlabel-v1 corpus: hard SFT rows + teacher slot_probs.

Joins three artifacts, all sha-pinned in the output manifest:

- the merged-line corpus (records: gold labels, rendering inputs,
  source tags for the trainer's family split),
- the teacher-scoring campaign items (which records/questions were
  scored, in which slot order),
- the teacher facts (per-row distributions over the closed option set,
  from teacher_score_rows.py score mode).

Rows are byte-identical to decision_sft_prep's hard rows (same header
render, same segments_for interleaving, verdicts gold-placed) plus one
field: ``slot_probs`` — the teacher distribution over option slots, in
segment order. Gold-conditioned, teacher-supervised: the trainer's
--soft-labels mode changes exactly one variable vs a hard arm, the
target distribution. Facts are stored RAW; temperature belongs to the
training run (--teacher-temp), not the corpus.

No filtering by teacher confidence happens here — dropping rows the
teacher found hard is how a distillation corpus loses exactly its most
informative examples. Noise gating is a separate decision made against
the same facts (corpus_gates), not baked in.

Output: ``<out>/e1-sft-v1.rows.jsonl`` (the trainer's rows filename by
design — a soft corpus dir is drop-in for the trainer's --data).

Usage:
    python3 build_softlabel_corpus.py \
        --corpus corpora/merged-v4.jsonl \
        --items corpora/teacher-items-v1/items.jsonl \
        --facts corpora/teacher-facts-v1/shard-0/facts.jsonl \
        --facts corpora/teacher-facts-v1/shard-1/facts.jsonl \
        --out corpora/e1-softlabel-v1
"""

from __future__ import annotations

import argparse
import json
import math
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import decision_sft_prep as prep  # noqa: E402
from e1_eval import sha256_of  # noqa: E402


def say(msg: str) -> None:
    sys.stdout.write(msg + "\n")
    sys.stdout.flush()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--corpus", type=Path, required=True,
                    help="merged-line corpus (records with gold "
                         "targets)")
    ap.add_argument("--items", type=Path, required=True,
                    help="teacher_score_rows emit output (defines the "
                         "scored row set and slot order)")
    ap.add_argument("--facts", type=Path, action="append",
                    required=True,
                    help="teacher facts jsonl (repeatable: one flag "
                         "per shard)")
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--max-state-chars", type=int, default=24000)
    ap.add_argument("--limit-records", type=int, default=0,
                    help="smoke: stop after N corpus records")
    args = ap.parse_args()

    # Items define the universe: their order and slot ids are what the
    # facts were computed against, so a fact can only be attached to a
    # row whose option ids match the item's exactly.
    items: dict[str, dict] = {}
    for line in args.items.open(encoding="utf-8"):
        if line.strip():
            it = json.loads(line)
            items[it["id"]] = it
    facts: dict[str, dict] = {}
    for fpath in args.facts:
        for line in fpath.open(encoding="utf-8"):
            if line.strip():
                fc = json.loads(line)
                if fc.get("probs") is not None:
                    facts[fc["id"]] = fc

    counts: Counter = Counter()
    agree: Counter = Counter()
    n_out = 0
    args.out.mkdir(parents=True, exist_ok=True)
    rows_path = args.out / "e1-sft-v1.rows.jsonl"
    tops: list[float] = []
    entropies: list[float] = []
    with args.corpus.open(encoding="utf-8") as src, \
            rows_path.open("w", encoding="utf-8") as dst:
        for line in src:
            if args.limit_records and n_out >= args.limit_records:
                break
            if not line.strip():
                continue
            rec = json.loads(line)
            counts["records_seen"] += 1
            rid = str(rec.get("record_id") or "")
            source = str(rec.get("source") or "")
            if source.startswith("suite"):
                counts["skipped: suite row"] += 1
                continue
            state = prep.state_text(rec)
            if not state:
                counts["skipped: empty state"] += 1
                continue
            if len(state) > args.max_state_chars:
                counts["skipped: state over budget"] += 1
                continue
            req = rec.get("request") or {}
            for qname, q in (req.get("questions") or {}).items():
                if args.limit_records and n_out >= args.limit_records:
                    break
                qtype = q.get("type")
                options = prep.options_for(q)
                if options is None:
                    counts[f"skipped: unrenderable {qtype} "
                           f"criteria"] += 1
                    continue
                ids = prep.option_ids(q)
                if len(ids) < 2:
                    counts["skipped: under 2 options"] += 1
                    continue
                target = (rec.get("target") or {}).get(qname) or {}
                label = str(target.get("label") or "")
                if target.get("type") != qtype or label not in ids:
                    counts["skipped: target label outside "
                           "options"] += 1
                    continue
                instructions = str(q.get("instructions") or "").strip()
                if not instructions:
                    counts["skipped: empty instructions"] += 1
                    continue
                row_id = f"{rid}::{qname}"
                item = items.get(row_id)
                fact = facts.get(row_id)
                if item is None or fact is None:
                    counts["skipped: no teacher fact"] += 1
                    continue
                probs = fact["probs"]
                if len(probs) != len(ids) or item["ids"] != ids:
                    counts["skipped: slot mismatch vs item"] += 1
                    continue
                if any(p is None or not math.isfinite(p) or p < 0
                       for p in probs) or sum(probs) <= 0:
                    counts["skipped: degenerate fact"] += 1
                    continue
                header = prep.render_header(qtype, instructions,
                                            options, state)
                segs = [{"t": header, "y": 0}] + \
                    prep.segments_for(ids, options, label)
                dst.write(json.dumps({
                    "id": row_id,
                    "record_id": rid,
                    "question": qname,
                    "qtype": qtype,
                    "label": label,
                    "source": source,
                    "n_options": len(ids),
                    "segments": segs,
                    "slot_probs": [float(p) for p in probs],
                    "_teacher": {
                        "argmax_id": fact.get("argmax_id"),
                        "top": fact.get("top"),
                        "margin": fact.get("margin"),
                        "entropy": fact.get("entropy"),
                    },
                }, ensure_ascii=False) + "\n")
                counts[f"emitted: {qtype}"] += 1
                n_out += 1
                tops.append(float(fact.get("top") or 0.0))
                entropies.append(float(fact.get("entropy") or 0.0))
                agree_total = f"{qtype}"
                agree[agree_total] += 1
                if fact.get("argmax_id") == label:
                    agree[f"agree: {qtype}"] += 1

    per_qtype = {
        qt: {"rows": agree[qt],
             "teacher_argmax_eq_gold": agree[f"agree: {qt}"],
             "rate": round(agree[f"agree: {qt}"] / max(1, agree[qt]),
                           4)}
        for qt in sorted(k for k in agree if not k.startswith("agree:"))}
    manifest = {
        "manifest_version": "opencodifier.e1-softlabel-corpus/1",
        "conditioning": "gold (segments_for verdicts at the gold "
                        "slot); supervision is the teacher "
                        "distribution in slot_probs",
        "corpus": {"path": args.corpus.name,
                   "sha256": sha256_of(args.corpus)},
        "items": {"path": args.items.name,
                  "sha256": sha256_of(args.items)},
        "facts": [{"path": str(f), "sha256": sha256_of(f)}
                  for f in args.facts],
        "max_state_chars": args.max_state_chars,
        "rows": n_out,
        "teacher_top_mean": round(sum(tops) / max(1, len(tops)), 4),
        "teacher_entropy_mean": round(
            sum(entropies) / max(1, len(entropies)), 4),
        "teacher_vs_gold_by_qtype": per_qtype,
        "counts": dict(counts),
    }
    (args.out / "e1-softlabel.manifest.json").write_text(
        json.dumps(manifest, indent=1) + "\n")
    say(json.dumps({"rows": n_out,
                    "counts": dict(counts),
                    "teacher_vs_gold_by_qtype": per_qtype}, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
