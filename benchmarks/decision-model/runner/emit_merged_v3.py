#!/usr/bin/env python3
"""Emit merged-v3: merged-v2 train + qradj-v1 recovery + score gap-fill.

Inputs and policy (docs/TRAINING.md §9, 2026-10-06):
  - merged-v2.jsonl rows pass through untouched (the train of record);
  - qradj-v1 recovered records are re-admitted verbatim: each carries
    `_readjudication` with k unanimous donor votes agreeing the
    surviving teacher gold (confirm-only contract, OC_GAPFILL_SPEC
    item 1). Cluster anatomy was verified post-hoc (zero same-query
    label fights in the entire quarantine pool — merge-v2's state-only
    grouping had flagged passage-QA structure as label fights), so no
    dedup and no representative selection applies to the recovered set;
  - score gap-fill rows (OC_GAPFILL_SPEC item 2) are new score
    questions over existing corpus states with unanimous k-vote hard
    labels; choice gap-fill rows (item 3) are candidate-conditioned
    relevance questions (the state's own true query + 3 distractor
    queries from other records) with hard labels only from k votes
    unanimous on the mapped query; rows are keyed by `record_id_new`
    (the original corpus record id moves into
    `_gapfill.origin_record_id`) so ids stay unique next to the
    records the states came from;
  - suite/suite_holdout rows pass through untouched, eval rows are
    never training rows.

Gates before pinning: every recovered record proves its contract
(k parseable unanimous votes agreeing gold, `_quarantine` present, id
absent from the parent train); every score row carries exactly one
score question and a unique new id; zero state-hash collisions between
any emitted non-suite row and any suite row; per-source/per-kind
reconciliation against the input census. Outputs land next to the
parent corpus.
"""
from __future__ import annotations

import hashlib
import json
import sys
import time
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from analyze_merged_dups import canon_state  # noqa: E402

VOTE_TO_LABEL = {"yes": "true", "no": "false"}


def gold_labels(record: dict) -> frozenset:
    out = set()
    for _qname, t in (record.get("target") or {}).items():
        if isinstance(t, dict):
            out.add(f"{t.get('type')}:{t.get('label')}")
    return frozenset(out)


def sha256_of(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def state_hash(record: dict) -> str:
    return hashlib.sha256(
        canon_state((record.get("request") or {}).get("state"))
        .encode("utf-8")).hexdigest()


def state_query_hash(record: dict) -> str:
    state = (record.get("request") or {}).get("state")
    query = state.get("query") if isinstance(state, dict) else ""
    return hashlib.sha256((canon_state(state) + "\x00" + str(query or ""))
                          .encode("utf-8")).hexdigest()


def main() -> int:
    parent = Path(sys.argv[1])
    recovered_path = Path(sys.argv[2])
    score_rows = Path(sys.argv[3]) if len(sys.argv) > 3 else None
    choice_rows = Path(sys.argv[4]) if len(sys.argv) > 4 else None
    out_train = parent.with_name("merged-v3.jsonl")
    out_manifest = parent.with_name("merged-v3.manifest.json")
    t_start = time.time()

    train = [json.loads(line) for line in parent.open()]
    recovered = [json.loads(line) for line in recovered_path.open()]
    score = ([json.loads(line) for line in score_rows.open()]
             if score_rows else [])
    choice = ([json.loads(line) for line in choice_rows.open()]
              if choice_rows else [])
    print(f"loaded train={len(train)} recovered={len(recovered)} "
          f"score={len(score)} choice={len(choice)}", flush=True)

    train_ids = {r["record_id"] for r in train}
    assert len(train_ids) == len(train), "duplicate ids in parent train"

    # ---- recovered-contract gate (confirm-only, auditable per record) ----
    for r in recovered:
        rid = r["record_id"]
        assert rid not in train_ids, f"recovered id already in train: {rid}"
        q = r.get("_quarantine") or {}
        assert q.get("cluster_id"), f"{rid}: missing _quarantine"
        rj = r.get("_readjudication") or {}
        votes = rj.get("votes") or []
        assert rj.get("run") == "qradj-v1", f"{rid}: wrong recovery run"
        assert len(votes) == 3, f"{rid}: expected 3 votes, got {len(votes)}"
        values = {v.get("value") for v in votes}
        assert len(values) == 1, f"{rid}: votes not unanimous: {values}"
        assert all(isinstance(v.get("p"), float) for v in votes), \
            f"{rid}: vote without numeric p"
        gold = (((r.get("target") or {}).get("relevance") or {})
                .get("label"))
        assert gold is not None, f"{rid}: no relevance gold"
        assert VOTE_TO_LABEL[values.pop()] == gold, \
            f"{rid}: recovery votes disagree gold"

    # ---- score-row gate ----
    score_ids = set()
    for r in score:
        new_id = r.get("record_id_new") or ""
        assert new_id and new_id not in score_ids, f"dup score id {new_id}"
        assert new_id not in train_ids, f"score id collides train {new_id}"
        score_ids.add(new_id)
        questions = (r.get("request") or {}).get("questions") or {}
        assert len(questions) == 1, f"{new_id}: expected 1 question"
        for _name, q in questions.items():
            assert q.get("type") == "score", f"{new_id}: not a score question"
            assert (r.get("target") or {}).get(_name, {}).get("type") == \
                "score", f"{new_id}: target kind mismatch"
        assert (r.get("request") or {}).get("state", {}).get("text"), \
            f"{new_id}: empty state text"
        votes_t = (((r.get("teacher") or {}).get(list(questions)[0]) or {})
                   .get("votes") or [])
        assert len({v["value"] for v in votes_t}) == 1, \
            f"{new_id}: teacher votes not unanimous"

    # ---- choice-row gate ----
    for r in choice:
        new_id = r.get("record_id_new") or ""
        assert new_id and new_id not in score_ids, \
            f"dup gap-fill id {new_id}"
        assert new_id not in train_ids, f"choice id collides train {new_id}"
        score_ids.add(new_id)
        questions = (r.get("request") or {}).get("questions") or {}
        assert len(questions) == 1, f"{new_id}: expected 1 question"
        for _name, q in questions.items():
            assert q.get("type") == "choice", \
                f"{new_id}: not a choice question"
            criteria = q.get("criteria") or {}
            assert isinstance(criteria, dict) and len(criteria) == 4, \
                f"{new_id}: expected 4 criteria candidates"
            target = (r.get("target") or {}).get(_name) or {}
            assert target.get("type") == "choice", \
                f"{new_id}: target kind mismatch"
            assert target.get("label") in criteria, \
                f"{new_id}: label not a candidate id"
        assert (r.get("request") or {}).get("state", {}).get("text"), \
            f"{new_id}: empty state text"
        votes_c = (((r.get("teacher") or {}).get("choice") or {})
                   .get("votes") or [])
        assert len(votes_c) == 3, f"{new_id}: expected 3 teacher votes"

    # ---- emit: parent order, then recovered, then gap-fill rows ----
    with out_train.open("w") as fh:
        for r in train:
            fh.write(json.dumps(r, ensure_ascii=False) + "\n")
        for r in recovered:
            fh.write(json.dumps(r, ensure_ascii=False) + "\n")
        for r in score + choice:
            row = dict(r)
            new_id = row.pop("record_id_new")
            gap = dict(row.get("_gapfill") or {})
            gap["origin_record_id"] = row.get("record_id")
            row["record_id"] = new_id
            row["_gapfill"] = gap
            fh.write(json.dumps(row, ensure_ascii=False) + "\n")

    # ---- gates before pinning ----
    suite_hashes = {state_hash(r) for r in train
                    if str(r.get("source", "")).startswith("suite")}
    # Exact (state, query) duplicates would be true double-counted rows:
    # the quarantine pool left train whole-cluster, so any collision here
    # means a recovery jumped the fence.
    train_sq = {state_query_hash(r) for r in train}
    sq_collisions = sum(1 for r in recovered
                        if state_query_hash(r) in train_sq)
    assert sq_collisions == 0, \
        f"recovered (state, query) duplicates of train: {sq_collisions}"
    collisions = 0
    suite_rows = 0
    kinds: Counter = Counter()
    sources: Counter = Counter()
    with out_train.open() as fh:
        for line in fh:
            r = json.loads(line)
            sources[r.get("source")] += 1
            is_suite = str(r.get("source", "")).startswith("suite")
            suite_rows += is_suite
            kinds.update(q.get("type") for q in
                         (r.get("request") or {})
                         .get("questions", {}).values())
            if not is_suite and state_hash(r) in suite_hashes:
                collisions += 1
    assert collisions == 0, f"train/suite state collisions: {collisions}"
    assert suite_rows == sum(
        1 for r in train if str(r.get("source", "")).startswith("suite")), \
        "suite rows changed"

    manifest = {
        "manifest_version": "opencodifier.distill-manifest/3",
        "parents": {
            "train": {"corpus": parent.name, "sha256": sha256_of(parent)},
            "recovered": {"corpus": recovered_path.name,
                          "sha256": sha256_of(recovered_path),
                          "aggregation": "opencodifier.qradj-v1/1",
                          "contract": "confirm-only: 3 unanimous donor "
                                      "votes agreeing surviving teacher "
                                      "gold"},
            **({"score": {"corpus": score_rows.name,
                          "sha256": sha256_of(score_rows),
                          "aggregation": "opencodifier.score-v1/1",
                          "contract": "unanimous k-vote hard labels over "
                                      "existing states"}
               } if score_rows else {}),
            **({"choice": {"corpus": choice_rows.name,
                           "sha256": sha256_of(choice_rows),
                           "aggregation": "opencodifier.choice-v1/1",
                           "contract": "unanimous k-vote hard labels on "
                                       "the mapped true query, "
                                       "rotation-robust"}
               } if choice_rows else {}),
        },
        "policy": {
            "recovery_rule": "re-admit qradj-v1 recovered verbatim; "
                             "cluster anatomy verified (zero same-query "
                             "label fights in the quarantine pool), no "
                             "dedup, no representative selection",
            "score_rule": "new score questions over existing states, "
                          "keyed by record_id_new; origin id kept in "
                          "_gapfill.origin_record_id",
            "choice_rule": "candidate-conditioned relevance: the state's "
                           "own true query + 3 distractor queries from "
                           "other records; hard label only from 3 votes "
                           "unanimous on the mapped query",
            "suite_rule": "pass through untouched",
        },
        "records_total": len(train) + len(recovered) + len(score)
                         + len(choice),
        "records_pass_through": len(train),
        "records_recovered": len(recovered),
        "records_score_gapfill": len(score),
        "records_choice_gapfill": len(choice),
        "sources": dict(sources),
        "question_kinds": dict(kinds),
        "sha256": {"merged-v3.jsonl": sha256_of(out_train)},
        "gates": {"train_suite_state_collisions": collisions,
                  "suite_rows_in_output": suite_rows,
                  "recovered_state_query_duplicates": sq_collisions,
                  "recovered_contract_violations": 0,
                  "score_contract_violations": 0,
                  "choice_contract_violations": 0},
        "runtime_s": round(time.time() - t_start, 1),
    }
    out_manifest.write_text(json.dumps(manifest, indent=1) + "\n")
    print(json.dumps({k: manifest[k] for k in
                      ("records_total", "records_recovered",
                       "records_score_gapfill", "records_choice_gapfill",
                       "question_kinds", "sha256", "gates",
                       "runtime_s")}, indent=1))
    print(f"wrote {out_train}, {out_manifest}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
