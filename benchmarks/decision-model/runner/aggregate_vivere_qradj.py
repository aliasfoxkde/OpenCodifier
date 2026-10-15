#!/usr/bin/env python3
"""Aggregate the quarantine re-adjudication donor run (task #110).

Joins the donor's vote rows back through the out-of-band mapping and
applies the confirm-only recovery contract (OC_GAPFILL_SPEC §3): a
quarantined record is re-admitted to the merged corpus only when all k
votes are parseable, unanimous, AND agree with the surviving teacher
gold label. Recovered records carry a ``_readjudication`` block with the
raw donor votes, so the recovery is auditable from the record alone.
Split votes and unanimous contradictions stay quarantined —
a near-duplicate cluster with mixed gold labels is exactly the evidence
that at least one side's label is wrong, so the pool may only shrink
when the donor independently confirms the surviving label, never grow.

The donor never saw gold: votes map through prompt_id → mapping row →
record. Gold labels are "true"/"false" strings; donor votes are
"yes"/"no" (mapped here, once, in one place).

Usage:
    python3 runner/aggregate_vivere_qradj.py \
        --rows ~/vivere_corpora/qradj_full_v3/rows.jsonl \
        --mapping ~/oc-model-eval/corpora/vivere-qradj-v1/vivere-qradj-v1.mapping.jsonl \
        --quarantine ~/oc-model-eval/corpora/merged-v2.quarantine.jsonl \
        --out ~/oc-model-eval/results/qradj-v1
"""

from __future__ import annotations

import argparse
import json
import statistics
import sys
from collections import Counter
from pathlib import Path

VOTE_TO_LABEL = {"yes": "true", "no": "false"}


def load_jsonl(path: Path) -> list[dict]:
    # File iteration, not str.splitlines(): splitlines also breaks on
    # U+2028/U+2029, which corpus text can contain and ensure_ascii=False
    # writes raw, which would split a JSON line mid-string.
    return [json.loads(line) for line in path.open(encoding="utf-8")
            if line.strip()]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--rows", type=Path, required=True)
    ap.add_argument("--mapping", type=Path, required=True)
    ap.add_argument("--quarantine", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--votes", type=int, default=3)
    args = ap.parse_args()

    # Last row wins per prompt_id (deterministic under any resume dupes).
    rows: dict[str, dict] = {}
    for row in load_jsonl(args.rows):
        rows[row["prompt_id"]] = row
    mapping = {m["prompt_id"]: m for m in load_jsonl(args.mapping)}
    records = {r["record_id"]: r for r in load_jsonl(args.quarantine)}

    votes_by_record: dict[str, list[tuple[dict, dict]]] = {}
    for prompt_id, m in mapping.items():
        row = rows.get(prompt_id)
        if row is not None:
            votes_by_record.setdefault(m["record_id"], []).append((row, m))

    parse_ok = sum(1 for r in rows.values() if r.get("parse_status") == "ok")
    truncated = sum(1 for r in rows.values() if r.get("finish_reason") == "length")
    errored = sum(1 for r in rows.values() if r.get("error"))
    probs = [r["decision"]["p"] for r in rows.values()
             if r.get("parse_status") == "ok" and r.get("decision")]
    values = Counter(r["decision"]["value"] for r in rows.values()
                     if r.get("parse_status") == "ok" and r.get("decision"))

    recovered: list[dict] = []
    agreement: Counter = Counter()
    n_complete = n_incomplete = 0
    contradiction_ids: list[str] = []
    for record_id, pairs in sorted(votes_by_record.items()):
        oks = [row for row, _ in pairs if row.get("parse_status") == "ok"
               and row.get("decision")]
        if len(oks) < args.votes:
            n_incomplete += 1
            agreement[f"<{args.votes} parseable"] += 1
            continue
        n_complete += 1
        cast = [row["decision"]["value"] for row in oks]
        unanimous = len(set(cast)) == 1
        agreement["3/3" if unanimous else "2/3 split"] += 1
        if not unanimous:
            continue
        record = records.get(record_id)
        if record is None:
            # A vote for a record no longer in the pool: loud, not silent.
            print(f"WARNING: votes for unknown record {record_id}", file=sys.stderr)
            continue
        gold = ((record.get("target") or {}).get("relevance") or {}).get("label")
        vote_label = VOTE_TO_LABEL[cast[0]]
        if vote_label == gold:
            # The recovery must be auditable from the record alone: attach
            # the raw donor votes that justify it (same convention as the
            # score aggregator's teacher votes).
            record["_readjudication"] = {
                "run": "qradj-v1",
                "donor": "vivere/gemma-4-E2B-it-QAT-Q4_0",
                "vote_labels": [VOTE_TO_LABEL[v] for v in cast],
                "votes": [{"value": row["decision"]["value"],
                           "p": row["decision"]["p"]} for row in oks],
            }
            recovered.append(record)
        else:
            contradiction_ids.append(record_id)
            agreement["3/3 unanimous contradiction"] += 1

    args.out.mkdir(parents=True, exist_ok=True)
    recovered_path = args.out / "qradj-v1.recovered.jsonl"
    with recovered_path.open("w", encoding="utf-8") as out:
        for record in recovered:
            out.write(json.dumps(record, ensure_ascii=False) + "\n")

    n_pool = len(records)
    report = {
        "aggregation_version": "opencodifier.qradj-v1/1",
        "rows": len(rows),
        "parse_ok": parse_ok,
        "parse_rate": round(parse_ok / len(rows), 4) if rows else 0.0,
        "truncated_at_max_tokens": truncated,
        "http_or_other_errors": errored,
        "donor_value_distribution": dict(values),
        "donor_p": {
            "p50": round(statistics.median(probs), 4) if probs else None,
            "mean": round(statistics.fmean(probs), 4) if probs else None,
            "extreme_0_or_1": sum(1 for p in probs if p in (0.0, 1.0)),
        },
        "records_joined": len(votes_by_record),
        "records_in_pool": n_pool,
        "complete_vote_sets": n_complete,
        "incomplete_vote_sets": n_incomplete,
        "agreement": dict(agreement),
        "recovered": len(recovered),
        "quarantine_recovery_rate": round(len(recovered) / n_pool, 4) if n_pool else 0.0,
        "unanimous_contradiction_staying_quarantined": len(contradiction_ids),
    }
    (args.out / "qradj-v1.gap-report.json").write_text(
        json.dumps(report, indent=1) + "\n", encoding="utf-8")

    md = [
        "# Quarantine re-adjudication — gap report (qradj-v1, task #110)",
        "",
        f"Donor run `{args.rows.parent.name}` over {len(rows)} prompts "
        f"({parse_ok} parsed, {parse_ok / len(rows):.1%}); "
        f"{truncated} truncated at max_tokens, {errored} transport errors.",
        "",
        f"Vote agreement over {n_complete} complete sets: "
        + ", ".join(f"{k} = {v}" for k, v in sorted(agreement.items())) + ".",
        "",
        f"**Recovered {len(recovered)} of {n_pool} quarantined records "
        f"({len(recovered) / n_pool:.1%})** under the confirm-only "
        "contract: 3/3 unanimous votes agreeing with the surviving teacher "
        "gold. Split votes and unanimous contradictions stay quarantined "
        "(the pool only shrinks, never grows).",
        "",
    ]
    (args.out / "qradj-v1.gap-report.md").write_text("\n".join(md), encoding="utf-8")

    print(json.dumps(report, indent=1))
    print(f"wrote {recovered_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
