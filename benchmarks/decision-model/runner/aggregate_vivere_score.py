#!/usr/bin/env python3
"""Aggregate the score-family donor run (OC_GAPFILL_SPEC §2 item 2).

Applies the raw-teacher + self-consistency contract to score votes: a
(state, family) question gets a hard label only when all k votes parse
AND are unanimous; the hard target carries the vote histogram as
``probabilities`` (all mass on the unanimous level — a hard label is a
degenerate distribution, same convention as merged-v2's typed-decisions
score targets), the level index as ``label``, and the expected score
under the histogram. Non-unanimous or incomplete vote sets keep the raw
teacher rows out of train: they are counted in the gap report, never
smoothed into labels.

The donor never saw gold — there is no gold to confirm against; the
donor IS the teacher. Class balance is reported, not corrected (no
invented rebalancing of supervision).

Usage:
    python3 runner/aggregate_vivere_score.py \
        --rows ~/vivere_corpora/score_full_v1/rows.jsonl \
        --mapping ~/oc-model-eval/corpora/vivere-score-v1/vivere-score-v1.mapping.jsonl \
        --corpus ~/oc-model-eval/corpora/merged-v2.jsonl \
        --out ~/oc-model-eval/results/score-v1
"""

from __future__ import annotations

import argparse
import json
import statistics
import sys
from collections import Counter
from pathlib import Path

LEVELS = ("0", "1", "2", "3")


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

    # record_id -> state text (only the records the mapping references).
    wanted: set[str] = {m["record_id"] for m in mapping.values()}
    state_text: dict[str, str] = {}
    for line in args.corpus.open(encoding="utf-8"):
        if not line.strip():
            continue
        rec = json.loads(line)
        rid = str(rec.get("record_id") or "")
        if rid in wanted:
            state_text[rid] = str(((rec.get("request") or {}).get("state") or {}).get("text") or "")

    votes_by_q: dict[tuple[str, str], list[dict]] = {}
    for prompt_id, m in mapping.items():
        row = rows.get(prompt_id)
        if row is not None:
            votes_by_q.setdefault((m["record_id"], m["family"]), []).append(row)

    parse_ok = sum(1 for r in rows.values() if r.get("parse_status") == "ok")
    truncated = sum(1 for r in rows.values() if r.get("finish_reason") == "length")
    errored = sum(1 for r in rows.values() if r.get("error"))
    probs = [r["decision"]["p"] for r in rows.values()
             if r.get("parse_status") == "ok" and r.get("decision")]
    values = Counter(r["decision"]["value"] for r in rows.values()
                     if r.get("parse_status") == "ok" and r.get("decision"))

    emitted: list[dict] = []
    agreement: Counter = Counter()
    label_dist: Counter = Counter()
    per_family: dict[str, Counter] = {}
    n_questions = n_complete = 0
    for (record_id, family), pairs in sorted(votes_by_q.items()):
        n_questions += 1
        oks = [row for row in pairs if row.get("parse_status") == "ok"
               and row.get("decision")]
        if len(oks) < args.votes:
            agreement[f"<{args.votes} parseable"] += 1
            continue
        n_complete += 1
        cast = [row["decision"]["value"] for row in oks]
        if len(set(cast)) != 1:
            agreement[f"{max(Counter(cast).values())}/{args.votes} split"] += 1
            continue
        agreement[f"{args.votes}/{args.votes} unanimous"] += 1
        level = cast[0]
        if level not in LEVELS:
            agreement["level out of range"] += 1
            continue
        label_dist[level] += 1
        per_family.setdefault(family, Counter())[level] += 1
        emitted.append({
            "record_id": record_id,
            "record_id_new": (
                f"vivere-score-v1:{record_id.split(':', 1)[-1]}:{family}"),
            "source": "vivere/gemma-4-E2B-it-QAT-Q4_0",
            "source_config": "gapfill-v1/score",
            "request": {
                "state": {"text": state_text.get(record_id, ""), "facts": {}},
                "questions": {
                    family: {
                        "type": "score",
                        "instructions": (
                            "How risky is the action described in this passage?"
                            if family == "risk" else
                            "How quickly does the matter described in this "
                            "passage need attention?"),
                        "criteria": list(
                            ("Benign: read-only or clearly safe actions.",
                             "Low: routine writes within scope.",
                             "Moderate: irreversible or out-of-scope actions.",
                             "High: destructive, security-relevant, or "
                             "policy-violating actions.")
                            if family == "risk" else
                            ("No time pressure; can wait indefinitely.",
                             "Routine; handle within the normal queue.",
                             "Elevated; should be handled within the same week.",
                             "Critical; requires action within the same day.")),
                    },
                },
            },
            "target": {
                family: {
                    "type": "score",
                    "label": level,
                    "probabilities": {lev: (1.0 if lev == level else 0.0)
                                      for lev in LEVELS},
                    "score": float(LEVELS.index(level)) / (len(LEVELS) - 1),
                },
            },
            "teacher": {
                family: {"votes": [{"value": row["decision"]["value"],
                                    "p": row["decision"]["p"]}
                                   for row in oks]},
            },
            "_gapfill": {"corpus": "vivere-score-v1", "family": family,
                         "votes": args.votes, "agreement": "unanimous"},
        })

    args.out.mkdir(parents=True, exist_ok=True)
    out_rows = args.out / "score-v1.rows.jsonl"
    with out_rows.open("w", encoding="utf-8") as out:
        for rec in emitted:
            out.write(json.dumps(rec, ensure_ascii=False) + "\n")

    states_known = len(state_text)
    report = {
        "aggregation_version": "opencodifier.score-v1/1",
        "rows": len(rows),
        "parse_ok": parse_ok,
        "parse_rate": round(parse_ok / len(rows), 4) if rows else 0.0,
        "truncated_at_max_tokens": truncated,
        "http_or_other_errors": errored,
        "donor_value_distribution": dict(sorted(values.items())),
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
        "hard_label_level_distribution": dict(sorted(label_dist.items())),
        "hard_label_level_by_family": {
            fam: dict(sorted(c.items())) for fam, c in sorted(per_family.items())
        },
        "states_seen": states_known,
    }
    (args.out / "score-v1.gap-report.json").write_text(
        json.dumps(report, indent=1) + "\n", encoding="utf-8")

    md = [
        "# Score-family gap fill — gap report (score-v1)",
        "",
        f"Donor run `{args.rows.parent.name}` over {len(rows)} prompts "
        f"({parse_ok} parsed, {parse_ok / len(rows):.1%}).",
        "",
        f"Vote agreement over {n_complete} complete (state, family) sets: "
        + ", ".join(f"{k} = {v}" for k, v in sorted(agreement.items())) + ".",
        "",
        f"**{len(emitted)} hard labels emitted of {n_questions} questions "
        f"({len(emitted) / n_questions:.1%})** — unanimous k-vote sets only; "
        "split votes are counted, never smoothed.",
        "",
        "Hard-label level distribution: "
        + ", ".join(f"L{lev} = {n}" for lev, n in sorted(label_dist.items()))
        + " (reported as measured, not rebalanced).",
        "",
    ]
    (args.out / "score-v1.gap-report.md").write_text("\n".join(md), encoding="utf-8")

    print(json.dumps(report, indent=1))
    print(f"wrote {out_rows}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
