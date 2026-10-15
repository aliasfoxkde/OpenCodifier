#!/usr/bin/env python3
"""Fixture tests for build_softlabel_corpus.py (torch-free, stdlib only).

The corpus builder is provenance-critical: its rows feed the
e1-softlabel-v1 training arm, and its one-variable A/B design depends
on two properties these tests pin down:

1. byte-identity with the hard rows — segments are exactly
   ``[{"t": header, "y": 0}] + prep.segments_for(...)``, so the soft
   corpus changes only the target distribution, never the text;
2. facts are joined raw — no teacher-confidence filtering (a disagreeing
   teacher still emits a row), slot mismatches and degenerate facts are
   skipped loudly in the manifest counts.

Run: python3 -m pytest benchmarks/decision-model/runner/test_build_softlabel_corpus.py -q
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

RUNNER = Path(__file__).resolve().parent
BUILDER = RUNNER / "build_softlabel_corpus.py"

sys.path.insert(0, str(RUNNER))
import decision_sft_prep as prep  # noqa: E402


def make_record(rid: str = "typed-decisions:r1",
                source: str = "merged-v4",
                label: str = "v-a") -> dict:
    return {
        "record_id": rid,
        "source": source,
        "request": {
            "state": {"text": "The vendor deadline is Friday."},
            "questions": {
                "pick": {
                    "type": "choice",
                    "instructions": "Which vendor ships first?",
                    "criteria": {"v-a": "ships in 2 days",
                                 "v-b": "ships in 5 days"},
                },
            },
        },
        "target": {"pick": {"type": "choice", "label": label}},
    }


def make_item(rid: str, qname: str = "pick") -> dict:
    return {"id": f"{rid}::{qname}", "ids": ["v-a", "v-b"]}


def make_fact(rid: str, probs: list[float], argmax: str,
              qname: str = "pick") -> dict:
    return {"id": f"{rid}::{qname}", "probs": probs,
            "argmax_id": argmax, "top": max(probs),
            "margin": max(probs) - sorted(probs)[-2],
            "entropy": 0.61}


def run_builder(tmp: Path, records: list[dict], items: list[dict],
                fact_shards: list[list[dict]]) -> subprocess.CompletedProcess:
    corpus = tmp / "corpus.jsonl"
    corpus.write_text(
        "".join(json.dumps(r, ensure_ascii=False) + "\n" for r in records),
        encoding="utf-8")
    items_path = tmp / "items.jsonl"
    items_path.write_text(
        "".join(json.dumps(i, ensure_ascii=False) + "\n" for i in items),
        encoding="utf-8")
    cmd = [sys.executable, str(BUILDER),
           "--corpus", str(corpus), "--items", str(items_path),
           "--out", str(tmp / "out")]
    for i, shard in enumerate(fact_shards):
        fpath = tmp / f"facts-{i}.jsonl"
        fpath.write_text(
            "".join(json.dumps(f, ensure_ascii=False) + "\n" for f in shard),
            encoding="utf-8")
        cmd += ["--facts", str(fpath)]
    return subprocess.run(cmd, capture_output=True, text=True)


def read_rows(out: Path) -> list[dict]:
    return [json.loads(line)
            for line in (out / "e1-sft-v1.rows.jsonl")
            .read_text(encoding="utf-8").splitlines() if line.strip()]


def read_manifest(out: Path) -> dict:
    return json.loads((out / "e1-softlabel.manifest.json")
                      .read_text(encoding="utf-8"))


def test_happy_path_emits_row_manifest_and_sha_pins(tmp_path):
    proc = run_builder(tmp_path, [make_record()],
                       [make_item("typed-decisions:r1")],
                       [[make_fact("typed-decisions:r1", [0.7, 0.3], "v-a")]])
    assert proc.returncode == 0, proc.stderr
    rows = read_rows(tmp_path / "out")
    assert len(rows) == 1
    row = rows[0]
    assert row["id"] == "typed-decisions:r1::pick"
    assert row["label"] == "v-a"
    assert row["slot_probs"] == [0.7, 0.3]
    manifest = read_manifest(tmp_path / "out")
    assert manifest["manifest_version"] == "opencodifier.e1-softlabel-corpus/1"
    assert manifest["rows"] == 1
    import re
    for pin in (manifest["corpus"], manifest["items"], *manifest["facts"]):
        assert re.fullmatch(r"[0-9a-f]{64}", pin["sha256"]), pin
    assert manifest["teacher_vs_gold_by_qtype"]["choice"] == {
        "rows": 1, "teacher_argmax_eq_gold": 1, "rate": 1.0}
    assert manifest["counts"].get("emitted: choice") == 1


def test_segments_are_prep_identical(tmp_path):
    """The one-variable A/B premise: text is byte-identical to a hard
    row; only slot_probs is added."""
    rec = make_record()
    proc = run_builder(tmp_path, [rec],
                       [make_item("typed-decisions:r1")],
                       [[make_fact("typed-decisions:r1", [0.7, 0.3], "v-a")]])
    assert proc.returncode == 0, proc.stderr
    row = read_rows(tmp_path / "out")[0]
    q = rec["request"]["questions"]["pick"]
    ids = prep.option_ids(q)
    options = prep.options_for(q)
    state = prep.state_text(rec)
    expected = ([{"t": prep.render_header(
        "choice", q["instructions"], options, state), "y": 0}]
        + prep.segments_for(ids, options, "v-a"))
    assert row["segments"] == expected


def test_missing_fact_is_counted_not_fatal(tmp_path):
    # --facts is argparse-required; the "no fact" path is an empty shard
    # (the campaign scored nothing for this row)
    proc = run_builder(tmp_path, [make_record()],
                       [make_item("typed-decisions:r1")], [[]])
    assert proc.returncode == 0, proc.stderr
    assert read_rows(tmp_path / "out") == []
    manifest = read_manifest(tmp_path / "out")
    assert manifest["counts"].get("skipped: no teacher fact") == 1


def test_slot_mismatch_skips(tmp_path):
    bad = make_fact("typed-decisions:r1", [0.5, 0.3, 0.2], "v-a")
    proc = run_builder(tmp_path, [make_record()],
                       [make_item("typed-decisions:r1")], [[bad]])
    assert proc.returncode == 0, proc.stderr
    manifest = read_manifest(tmp_path / "out")
    assert manifest["counts"].get("skipped: slot mismatch vs item") == 1


def test_degenerate_fact_skips(tmp_path):
    for probs in ([0.0, 0.0], [-0.2, 1.2]):
        proc = run_builder(
            tmp_path, [make_record()], [make_item("typed-decisions:r1")],
            [[make_fact("typed-decisions:r1", probs, "v-a")]])
        assert proc.returncode == 0, proc.stderr
        manifest = read_manifest(tmp_path / "out")
        assert manifest["counts"].get("skipped: degenerate fact") == 1, probs


def test_disagreeing_teacher_still_emits(tmp_path):
    """No confidence filtering: the teacher's argmax disagreeing with
    gold must NOT drop the row — hard rows are exactly where the
    distribution is most informative."""
    proc = run_builder(tmp_path, [make_record()],
                       [make_item("typed-decisions:r1")],
                       [[make_fact("typed-decisions:r1", [0.4, 0.6], "v-b")]])
    assert proc.returncode == 0, proc.stderr
    rows = read_rows(tmp_path / "out")
    assert len(rows) == 1 and rows[0]["slot_probs"] == [0.4, 0.6]
    manifest = read_manifest(tmp_path / "out")
    assert manifest["teacher_vs_gold_by_qtype"]["choice"]["rate"] == 0.0


def test_suite_source_rows_never_emit(tmp_path):
    proc = run_builder(tmp_path, [make_record(source="suite:20260926")],
                       [make_item("typed-decisions:r1")],
                       [[make_fact("typed-decisions:r1", [0.7, 0.3], "v-a")]])
    assert proc.returncode == 0, proc.stderr
    manifest = read_manifest(tmp_path / "out")
    assert manifest["counts"].get("skipped: suite row") == 1
    assert read_rows(tmp_path / "out") == []


def test_shard_flags_merge(tmp_path):
    """One --facts flag per campaign shard; facts across flags attach by
    row id."""
    r1, r2 = "typed-decisions:r1", "typed-decisions:r2"
    recs = [make_record(rid=r1),
            make_record(rid=r2, label="v-b")]
    proc = run_builder(
        tmp_path, recs, [make_item(r1), make_item(r2)],
        [[make_fact(r1, [0.7, 0.3], "v-a")],
         [make_fact(r2, [0.25, 0.75], "v-b")]])
    assert proc.returncode == 0, proc.stderr
    rows = {r["id"]: r for r in read_rows(tmp_path / "out")}
    assert rows[f"{r1}::pick"]["slot_probs"] == [0.7, 0.3]
    assert rows[f"{r2}::pick"]["slot_probs"] == [0.25, 0.75]
    manifest = read_manifest(tmp_path / "out")
    assert len(manifest["facts"]) == 2
