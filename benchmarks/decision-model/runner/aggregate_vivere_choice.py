#!/usr/bin/env python3
"""Aggregate the choice-family donor run (OC_GAPFILL_SPEC §2 item 3).

Applies the raw-teacher + self-consistency contract to choice votes.
Each (state) question has k votes rendered under rotations; a vote's
letter maps back to a query through that vote's letter→query mapping,
so unanimity is required on the mapped QUERY (rotation-robust — a
position-biased donor cannot pass by always answering "A"). A question
gets a hard label only when all k votes parse AND map to the true
query; the hard target carries the one-hot distribution over the vote-1
candidate order, the true query's vote-1 letter as ``label``, and the
mean vote confidence. Everything else is counted in the gap report,
never smoothed into labels.

The emitter guarantees the true query is at position idx % 4 across
the sample, so the per-position yield table doubles as the position-
bias readout for this donor run.

Usage:
    python3 runner/aggregate_vivere_choice.py \
        --rows ~/vivere_corpora/choice_full_v1/rows.jsonl \
        --mapping ~/oc-model-eval/corpora/vivere-choice-v1/vivere-choice-v1.mapping.jsonl \
        --corpus ~/oc-model-eval/corpora/merged-v2.jsonl \
        --out ~/oc-model-eval/results/choice-v1
"""

from __future__ import annotations

import argparse
import json
import statistics
import sys
from collections import Counter
from pathlib import Path

LETTERS = ("A", "B", "C", "D")


def load_jsonl(path: Path) -> list[dict]:
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()
            if line.strip()]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--rows", type=Path, required=True)
    ap.add_argument("--mapping", type=Path, required=True)
    ap.add_argument("--corpus", type=Path, required=True,
                    help="merged corpus the sampled states came from")
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--votes", type=int, default=3)
    args = ap.parse_args()

    rows: dict[str, dict] = {}
    for row in load_jsonl(args.rows):
        rows[row["prompt_id"]] = row
    mapping = {m["prompt_id"]: m for m in load_jsonl(args.mapping)}

    wanted: set[str] = {m["record_id"] for m in mapping.values()}
    state_text: dict[str, str] = {}
    for line in args.corpus.open(encoding="utf-8"):
        if not line.strip():
            continue
        rec = json.loads(line)
        rid = str(rec.get("record_id") or "")
        if rid in wanted:
            state_text[rid] = str(((rec.get("request") or {})
                                   .get("state") or {}).get("text") or "")

    votes_by_q: dict[str, list[tuple[dict, dict]]] = {}
    for prompt_id, m in mapping.items():
        row = rows.get(prompt_id)
        if row is not None:
            votes_by_q.setdefault(m["record_id"], []).append((row, m))

    parse_ok = sum(1 for r in rows.values() if r.get("parse_status") == "ok")
    truncated = sum(1 for r in rows.values() if r.get("finish_reason") == "length")
    errored = sum(1 for r in rows.values() if r.get("error"))
    probs = [r["decision"]["p"] for r in rows.values()
             if r.get("parse_status") == "ok" and r.get("decision")]
    letter_hist = Counter(r["decision"]["value"] for r in rows.values()
                          if r.get("parse_status") == "ok" and r.get("decision"))

    emitted: list[dict] = []
    agreement: Counter = Counter()
    yield_by_pos: Counter = Counter()
    total_by_pos: Counter = Counter()
    n_questions = n_complete = 0
    for record_id, pairs in sorted(votes_by_q.items()):
        n_questions += 1
        vote1 = next((m for _r, m in pairs if m["vote"] == 1), None)
        if vote1 is None:
            agreement["vote-1 missing"] += 1
            continue
        true_letter = vote1["true_letter"]
        total_by_pos[true_letter] += 1
        oks = [(row, m) for row, m in pairs
               if row.get("parse_status") == "ok" and row.get("decision")]
        if len(oks) < args.votes:
            agreement[f"<{args.votes} parseable"] += 1
            continue
        n_complete += 1
        # Map each vote's letter back to its query; unanimity is on the
        # mapped query, not the letter.
        mapped = [m["letter_to_query"].get(row["decision"]["value"])
                  for row, m in oks]
        if len(set(mapped)) != 1:
            agreement["split (mapped queries differ)"] += 1
            continue
        if mapped[0] != vote1["true_query"]:
            agreement["unanimous wrong candidate"] += 1
            continue
        agreement[f"{args.votes}/{args.votes} unanimous"] += 1
        yield_by_pos[true_letter] += 1
        ps = [row["decision"]["p"] for row, _m in oks]
        emitted.append({
            "record_id": record_id,
            "record_id_new": f"vivere-choice-v1:{record_id.split(':', 1)[-1]}",
            "source": "vivere/gemma-4-E2B-it-QAT-Q4_0",
            "source_config": "gapfill-v1/choice",
            "request": {
                "state": {"text": state_text.get(record_id, ""), "facts": {}},
                "questions": {
                    "choice": {
                        "type": "choice",
                        "instructions":
                            "Which ONE of these questions does this "
                            "passage answer?",
                        "criteria": dict(vote1["letter_to_query"]),
                    },
                },
            },
            "target": {
                "choice": {
                    "type": "choice",
                    "label": true_letter,
                    "probabilities": {letter: (1.0 if letter == true_letter
                                               else 0.0)
                                      for letter in vote1["letter_to_query"]},
                    "confidence": round(statistics.fmean(ps), 6),
                },
            },
            "teacher": {
                "choice": {"votes": [
                    {"letter": row["decision"]["value"],
                     "query": m["letter_to_query"].get(
                         row["decision"]["value"]),
                     "p": row["decision"]["p"]}
                    for row, m in oks]},
            },
            "_gapfill": {"corpus": "vivere-choice-v1", "votes": args.votes,
                         "agreement": "unanimous",
                         "distractors": "other records' queries"},
        })

    args.out.mkdir(parents=True, exist_ok=True)
    out_rows = args.out / "choice-v1.rows.jsonl"
    with out_rows.open("w", encoding="utf-8") as out:
        for rec in emitted:
            out.write(json.dumps(rec, ensure_ascii=False) + "\n")

    report = {
        "aggregation_version": "opencodifier.choice-v1/1",
        "rows": len(rows),
        "parse_ok": parse_ok,
        "parse_rate": round(parse_ok / len(rows), 4) if rows else 0.0,
        "truncated_at_max_tokens": truncated,
        "http_or_other_errors": errored,
        "donor_letter_distribution": dict(sorted(letter_hist.items())),
        "donor_p": {
            "p50": round(statistics.median(probs), 4) if probs else None,
            "mean": round(statistics.fmean(probs), 4) if probs else None,
            "extreme_0_or_1": sum(1 for p in probs if p in (0.0, 1.0)),
        },
        "questions": n_questions,
        "questions_with_full_votes": n_complete,
        "agreement": dict(sorted(agreement.items())),
        "hard_labels": len(emitted),
        "hard_label_rate": round(len(emitted) / n_questions, 4) if n_questions else 0.0,
        "hard_label_yield_by_true_position": {
            pos: {"yield": yield_by_pos.get(pos, 0), "total": total_by_pos.get(pos, 0),
                  "rate": round(yield_by_pos.get(pos, 0) / total_by_pos[pos], 4)
                  if total_by_pos.get(pos) else None}
            for pos in LETTERS},
        "states_seen": len(state_text),
    }
    (args.out / "choice-v1.gap-report.json").write_text(
        json.dumps(report, indent=1) + "\n", encoding="utf-8")

    md = [
        "# Choice-family gap fill — gap report (choice-v1)",
        "",
        f"Donor run `{args.rows.parent.name}` over {len(rows)} prompts "
        f"({parse_ok} parsed, {parse_ok / len(rows):.1%}).",
        "",
        f"Vote agreement over {n_complete} complete (state) sets: "
        + ", ".join(f"{k} = {v}" for k, v in sorted(agreement.items())) + ".",
        "",
        f"**{len(emitted)} hard labels emitted of {n_questions} questions "
        f"({len(emitted) / n_questions:.1%})** — unanimous k-vote sets "
        "only, unanimity on the mapped query (rotation-robust); splits "
        "and unanimous wrong-candidate votes are counted, never smoothed.",
        "",
        "Yield by true-candidate position: "
        + ", ".join(
            f"{pos} = {yield_by_pos.get(pos, 0)}/{total_by_pos.get(pos, 0)}"
            for pos in LETTERS)
        + " (the emitter placed the true query at position idx % 4, so "
        "this is the donor's position-bias readout).",
        "",
    ]
    (args.out / "choice-v1.gap-report.md").write_text("\n".join(md), encoding="utf-8")

    print(json.dumps(report, indent=1))
    print(f"wrote {out_rows}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
