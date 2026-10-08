//! End-to-end runs of the itemgen package's two binaries.
//!
//! `main()` of a bin is invisible to library tests — the only way to
//! exercise CLI parsing, output writing, and the summary rendering is
//! to run the built binary itself. `CARGO_BIN_EXE_*` hands over the
//! freshly compiled executables, so these tests cover the whole CLI
//! surface deterministically: a happy run's artifacts and summary
//! lines, the family-code refusal, the loud write-failure path
//! (`/dev/full` accepts `create` and fails every write), and the
//! merge binary's minimal smoke.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fmt::Write as _;
use std::process::{Command, Output};

fn scratch(label: &str) -> std::path::PathBuf {
    static NEXT_ID: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "opencodifier-bin-runs-{label}-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("scratch directory");
    dir
}

fn output_lines(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// A valid (empty) suite guard file — every run needs a guard to load,
/// and an empty suite collides with nothing.
fn write_suite(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("suite.json");
    std::fs::write(&path, r#"{"items": []}"#).expect("suite writes");
    path
}

#[test]
fn itemgen_bin_writes_artifacts_and_renders_every_summary_line() {
    let dir = scratch("happy");
    let suite = write_suite(&dir);
    let output = Command::new(env!("CARGO_BIN_EXE_opencodifier-itemgen"))
        .arg("--n")
        .arg("24")
        .arg("--seed")
        .arg("11")
        .arg("--probe-every")
        .arg("6")
        .arg("--families")
        .arg("rcc,sb")
        .arg("--pair-every")
        .arg("4")
        .arg("--out-train")
        .arg(dir.join("train.jsonl"))
        .arg("--out-probe")
        .arg(dir.join("probe.jsonl"))
        .arg("--suite")
        .arg(&suite)
        .arg("--manifest")
        .arg(dir.join("manifest.json"))
        .output()
        .expect("bin runs");
    assert!(output.status.success(), "{}", output_lines(&output));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("generated"), "run summary: {stdout}");
    assert!(stdout.contains("train") && stdout.contains("probe"), "{stdout}");
    assert!(stdout.contains("rejected"), "rejection counters: {stdout}");
    assert!(stdout.contains("minimal pairs"), "pair cadence ran: {stdout}");
    assert!(stdout.contains("score levels"), "score census ran: {stdout}");
    assert!(stdout.contains("lexical"), "probe rates: {stdout}");

    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("manifest.json")).expect("manifest"),
    )
    .expect("manifest parses");
    assert_eq!(
        manifest["families"].as_array().map(Vec::len),
        Some(2),
        "the requested rotation is recorded"
    );

    let train = std::fs::read_to_string(dir.join("train.jsonl")).expect("train");
    let probe = std::fs::read_to_string(dir.join("probe.jsonl")).expect("probe");
    assert!(!train.is_empty() && !probe.is_empty(), "both splits carry records");
    for line in train.lines().chain(probe.lines()) {
        serde_json::from_str::<serde_json::Value>(line).expect("every line parses");
    }
    std::fs::remove_dir_all(&dir).expect("scratch cleans");
}

#[test]
fn itemgen_bin_rejects_unknown_family_codes() {
    let dir = scratch("bad-family");
    let suite = write_suite(&dir);
    let output = Command::new(env!("CARGO_BIN_EXE_opencodifier-itemgen"))
        .arg("--n")
        .arg("4")
        .arg("--families")
        .arg("rcc,zz")
        .arg("--out-train")
        .arg(dir.join("train.jsonl"))
        .arg("--out-probe")
        .arg(dir.join("probe.jsonl"))
        .arg("--suite")
        .arg(&suite)
        .arg("--manifest")
        .arg(dir.join("manifest.json"))
        .output()
        .expect("bin runs");
    assert!(!output.status.success(), "an unknown code must fail the run");
    let text = output_lines(&output);
    assert!(text.contains("unknown family code"), "{text}");
    assert!(text.contains("zz"), "the offending code is named: {text}");
    std::fs::remove_dir_all(&dir).expect("scratch cleans");
}

#[test]
fn itemgen_bin_surfaces_a_write_failure_instead_of_a_partial_corpus() {
    // `/dev/full` accepts open and fails every write, so the run
    // generates a full corpus whose records cannot land — the corpus
    // must never ship degraded, so the run fails loudly.
    let dir = scratch("write-fail");
    let suite = write_suite(&dir);
    let output = Command::new(env!("CARGO_BIN_EXE_opencodifier-itemgen"))
        .arg("--n")
        .arg("400")
        .arg("--seed")
        .arg("5")
        .arg("--out-train")
        .arg("/dev/full")
        .arg("--out-probe")
        .arg("/dev/full")
        .arg("--suite")
        .arg(&suite)
        .arg("--manifest")
        .arg(dir.join("manifest.json"))
        .output()
        .expect("bin runs");
    assert!(!output.status.success(), "a failed record write must fail the run");
    let text = output_lines(&output);
    assert!(text.contains("write record"), "{text}");
    std::fs::remove_dir_all(&dir).expect("scratch cleans");
}

#[test]
fn corpus_merge_bin_writes_rows_and_a_manifest() {
    let dir = scratch("merge");
    // The merge's gates reason about family mix, so the base mirrors
    // the toy universe the crate's own e2e runs use: noul 4 / choice 2
    // / score 10, score modal on "0".
    let mut base = String::new();
    for i in 0..4 {
        writeln!(
            base,
            "{}",
            serde_json::json!({
                "id": format!("n{i}"), "qtype": "noul",
                "label": if i % 2 == 0 { "true" } else { "false" },
                "segments": [{ "t": "x", "y": 0 }],
            })
        )
        .expect("base row writes");
    }
    for i in 0..2 {
        writeln!(
            base,
            "{}",
            serde_json::json!({
                "id": format!("c{i}"), "qtype": "choice", "label": format!("c{i}"),
                "segments": [{ "t": "x", "y": 0 }],
            })
        )
        .expect("base row writes");
    }
    for i in 0..6 {
        writeln!(
            base,
            "{}",
            serde_json::json!({
                "id": format!("d{i}"), "qtype": "score", "label": "0",
                "segments": [{ "t": "x", "y": 0 }],
            })
        )
        .expect("base row writes");
    }
    for i in 0..4 {
        writeln!(
            base,
            "{}",
            serde_json::json!({
                "id": format!("v{i}"), "qtype": "score", "label": "1",
                "segments": [{ "t": "x", "y": 0 }],
            })
        )
        .expect("base row writes");
    }
    let base_path = dir.join("base.jsonl");
    std::fs::write(&base_path, base).expect("base writes");

    let output = Command::new(env!("CARGO_BIN_EXE_opencodifier-corpus-merge"))
        .arg("--base")
        .arg(&base_path)
        .arg("--out")
        .arg(dir.join("rows.jsonl"))
        .arg("--manifest")
        .arg(dir.join("manifest.json"))
        .arg("--target-total")
        .arg("20")
        .arg("--seed")
        .arg("4242")
        .output()
        .expect("bin runs");
    assert!(output.status.success(), "{}", output_lines(&output));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("merged"), "{stdout}");
    // The manifest is the row-count contract: the file carries exactly
    // the rows the manifest records.
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("manifest.json")).expect("manifest"),
    )
    .expect("manifest parses");
    let rows = std::fs::read_to_string(dir.join("rows.jsonl")).expect("rows");
    let rows_out =
        usize::try_from(manifest["rows_out"].as_u64().unwrap_or(0)).expect("row count fits usize");
    assert_eq!(rows.lines().count(), rows_out, "the file matches the manifest's row count");
    assert!(rows.lines().count() >= 16, "the base universe is sampled, not shrunk");
    serde_json::from_str::<serde_json::Value>(
        &std::fs::read_to_string(dir.join("manifest.json")).expect("manifest"),
    )
    .expect("manifest parses");
    std::fs::remove_dir_all(&dir).expect("scratch cleans");
}
