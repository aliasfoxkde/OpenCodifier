//! F2 byte-parity lane for the macjev renderer (TRAINING.md §9.7
//! Phase F2).
//!
//! Two tests over one committed corpus sample
//! (`tests/fixtures/macjev/corpus-sample.jsonl` — real merged-v3
//! records: one structured-state record rendering five questions of
//! every kind, one suite row exercising the skip):
//!
//! * `renders_match_the_python_derived_fixture` — always on. Compares
//!   the Rust renderer against `expected-render.jsonl`, which the
//!   python prep renderer produced. CI has no python; this pins the
//!   workspace renderer to python-derived bytes.
//! * `renders_match_the_live_python_renderer` — `#[ignore]`d (needs
//!   `python3` + the runner scripts; this host lane only, via `just
//!   check-macjev`). Re-derives the expectation from the live python
//!   implementation and fails if either side drifted — including the
//!   committed fixture going stale relative to the prep.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::process::Stdio;

use opencodifier_schema::macjev::{MacjevOutcome, render_record};

const MAX_STATE_CHARS: usize = 24_000;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/macjev")
}

fn fixture_lines(name: &str) -> Vec<String> {
    let raw =
        std::fs::read_to_string(fixture_dir().join(name)).expect("macjev fixture file is present");
    raw.lines().filter(|line| !line.trim().is_empty()).map(str::to_owned).collect()
}

/// Compares one Rust outcome against one python outcome object,
/// asserting byte equality everywhere the trainer reads.
fn assert_same(outcome: &MacjevOutcome, expected: &serde_json::Value) {
    match outcome {
        MacjevOutcome::SuiteRow => {
            assert_eq!(expected["outcome"], "suite_row", "suite skip");
        }
        MacjevOutcome::EmptyState => {
            assert_eq!(expected["outcome"], "empty_state", "empty-state skip");
        }
        MacjevOutcome::StateOverBudget { state_chars } => {
            assert_eq!(expected["outcome"], "state_over_budget", "budget skip");
            assert_eq!(
                *state_chars,
                expected["state_chars"]
                    .as_u64()
                    .map(usize::try_from)
                    .transpose()
                    .expect("usize")
                    .expect("python reports state_chars"),
                "state length"
            );
        }
        MacjevOutcome::Rendered(render) => {
            assert_eq!(expected["outcome"], "rendered", "rendered outcome");
            assert_eq!(
                &render.state,
                expected["state"].as_str().expect("python state"),
                "state text bytes"
            );
            let expected_rows = expected["rows"].as_array().expect("python rows list");
            assert_eq!(render.rows.len(), expected_rows.len(), "row count");
            for (row, want) in render.rows.iter().zip(expected_rows) {
                assert_eq!(row.id, want["id"].as_str().expect("row id"), "row id");
                assert_eq!(row.question, want["question"].as_str().expect("question"), "question");
                assert_eq!(row.qtype.as_str(), want["qtype"].as_str().expect("qtype"), "qtype");
                assert_eq!(row.label, want["label"].as_str().expect("label"), "label");
                assert_eq!(
                    row.n_options,
                    want["n_options"]
                        .as_u64()
                        .map(usize::try_from)
                        .transpose()
                        .expect("usize")
                        .expect("n_options"),
                    "n_options"
                );
                assert_eq!(
                    row.render_chars,
                    want["render_chars"]
                        .as_u64()
                        .map(usize::try_from)
                        .transpose()
                        .expect("usize")
                        .expect("chars"),
                    "render_chars"
                );
                let want_segments = want["segments"].as_array().expect("segments");
                assert_eq!(row.segments.len(), want_segments.len(), "segment count");
                for (segment, want_segment) in row.segments.iter().zip(want_segments) {
                    assert_eq!(
                        segment.text,
                        want_segment["t"].as_str().expect("segment text"),
                        "segment text bytes"
                    );
                    let supervised = want_segment["y"].as_i64().expect("segment mask") != 0;
                    assert_eq!(segment.supervised, supervised, "segment mask");
                }
                assert_eq!(
                    row.full_text(),
                    want["text"].as_str().expect("full text"),
                    "full prompt bytes"
                );
            }
            let want_skips = expected["skips"].as_array().expect("skips list");
            assert_eq!(render.skips.len(), want_skips.len(), "skip count");
            for ((name, skip), want) in render.skips.iter().zip(want_skips) {
                assert_eq!(name, want[0].as_str().expect("skip question"), "skip question");
                assert_eq!(skip.to_string(), want[1].as_str().expect("skip reason"), "skip reason");
            }
        }
    }
}

/// Renders every fixture record through the workspace renderer and
/// compares against the python-derived expected bytes. This is the
/// CI-visible half of the parity lane.
#[test]
fn renders_match_the_python_derived_fixture() {
    let records = fixture_lines("corpus-sample.jsonl");
    let expected_lines = fixture_lines("expected-render.jsonl");
    assert_eq!(records.len(), expected_lines.len(), "fixture files line up");
    for (record, expected) in records.iter().zip(expected_lines.iter()) {
        let expected: serde_json::Value =
            serde_json::from_str(expected).expect("expected line parses");
        let outcome = render_record(record, MAX_STATE_CHARS).expect("fixture record renders");
        assert_same(&outcome, &expected);
    }
}

/// Re-derives the expectation from the live python renderer and
/// compares both against the Rust render and against the committed
/// fixture (staleness guard). Run on the host lane via
/// `just check-macjev`.
#[test]
#[ignore = "requires python3 and the runner scripts (host lane): just check-macjev"]
fn renders_match_the_live_python_renderer() {
    let records = fixture_lines("corpus-sample.jsonl");
    let driver = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../benchmarks/decision-model/runner/macjev_render_rows.py");
    let mut child = std::process::Command::new("python3")
        .arg(driver)
        .arg("--max-state-chars")
        .arg(MAX_STATE_CHARS.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("python3 is available on the host lane");
    let mut stdin = child.stdin.take().expect("driver stdin");
    std::io::Write::write_all(&mut stdin, records.join("\n").as_bytes())
        .expect("driver accepts the fixture");
    drop(stdin);
    let output = child.wait_with_output().expect("driver completes");
    assert!(output.status.success(), "driver exited {:?}", output.status);
    let live = String::from_utf8(output.stdout).expect("driver emits UTF-8");

    let expected_lines = fixture_lines("expected-render.jsonl");
    let live_lines: Vec<&str> = live.lines().filter(|line| !line.trim().is_empty()).collect();
    assert_eq!(records.len(), live_lines.len(), "driver answered every record");
    assert_eq!(records.len(), expected_lines.len(), "committed fixture line count");
    for ((record, expected), live_line) in
        records.iter().zip(expected_lines.iter()).zip(live_lines.iter())
    {
        assert_eq!(expected, live_line, "committed fixture matches live python");
        let expected: serde_json::Value =
            serde_json::from_str(expected).expect("expected line parses");
        let outcome = render_record(record, MAX_STATE_CHARS).expect("fixture record renders");
        assert_same(&outcome, &expected);
    }
}
