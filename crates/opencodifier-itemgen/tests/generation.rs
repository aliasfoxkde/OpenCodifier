//! End-to-end generation: the compiled CLI run against a suite guard,
//! asserting the artifact contract a training run depends on — exact
//! counts, a collision-free manifest, byte determinism, and probe
//! records that stay out of the train file.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::Command;

/// A small synthetic suite: one item whose exact context the generator
/// could otherwise plausibly approach, so the guard has something to
/// reject and the manifest's collision count is meaningful.
const SUITE: &str = r#"{"items": [{"context": "auth is failing. billing depends on auth.",
    "question": "Which component is failing at the root, rather than because of another failure?"}]}"#;

struct Run {
    manifest: serde_json::Value,
    train_lines: Vec<String>,
    probe_lines: Vec<String>,
}

/// Runs the CLI once into `dir` with the given seed and returns the
/// parsed artifacts.
fn run_cli(dir: &std::path::Path, seed: u64, n: usize, tag: &str) -> Run {
    let suite_path = dir.join(format!("suite-{tag}.json"));
    std::fs::write(&suite_path, SUITE).unwrap();
    let train_path = dir.join(format!("train-{tag}.jsonl"));
    let probe_path = dir.join(format!("probe-{tag}.jsonl"));
    let manifest_path = dir.join(format!("manifest-{tag}.json"));

    let output = Command::new(env!("CARGO_BIN_EXE_opencodifier-itemgen"))
        .args([
            "--n",
            &n.to_string(),
            "--seed",
            &seed.to_string(),
            "--probe-every",
            "25",
            "--out-train",
            train_path.to_str().unwrap(),
            "--out-probe",
            probe_path.to_str().unwrap(),
            "--suite",
            suite_path.to_str().unwrap(),
            "--manifest",
            manifest_path.to_str().unwrap(),
        ])
        .output()
        .expect("the itemgen binary must run");
    assert!(
        output.status.success(),
        "itemgen exited {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let read_lines = |path: &std::path::Path| -> Vec<String> {
        std::fs::read_to_string(path).unwrap_or_default().lines().map(ToOwned::to_owned).collect()
    };
    Run {
        manifest: serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap(),
        train_lines: read_lines(&train_path),
        probe_lines: read_lines(&probe_path),
    }
}

#[test]
fn the_cli_emits_the_full_artifact_contract() {
    let dir = std::env::temp_dir().join(format!("oc-itemgen-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let n = 500;
    let run = run_cli(&dir, 2026, n, "a");
    assert_eq!(run.train_lines.len() + run.probe_lines.len(), n);
    // probe_every = 25 over 500 items -> exactly 20 probe rows, none of
    // them in the train file.
    assert_eq!(run.probe_lines.len(), 20);
    assert_eq!(run.manifest["probe"], 20);
    assert_eq!(run.manifest["train"], 480);
    assert_eq!(run.manifest["rejected_by_suite_collision"], 0);
    assert_eq!(run.manifest["manifest_version"], "opencodifier.itemgen/1");

    // Every emitted row parses, carries exactly one gold "yes" slot on
    // a >=5-slot ballot, and its label is a ballot entry.
    for line in run.train_lines.iter().chain(&run.probe_lines) {
        let record: serde_json::Value = serde_json::from_str(line).unwrap();
        let questions = record["request"]["questions"].as_object().unwrap();
        assert_eq!(questions.len(), 1, "one question per record");
        let (_, question) = questions.iter().next().unwrap();
        let criteria = question["criteria"].as_object().unwrap();
        assert!(criteria.len() >= 5);
        let label = record["target"]
            .as_object()
            .unwrap()
            .values()
            .next()
            .unwrap()
            .as_object()
            .unwrap()
            .get("label")
            .and_then(serde_json::Value::as_str)
            .unwrap();
        assert!(criteria.contains_key(label));
        assert!(record["source"].as_str().unwrap().starts_with("itemgen/"));
    }

    // Byte determinism: the same seed reproduces both files exactly.
    let again = run_cli(&dir, 2026, n, "b");
    assert_eq!(run.train_lines, again.train_lines);
    assert_eq!(run.probe_lines, again.probe_lines);
    assert_eq!(run.manifest["families"], again.manifest["families"]);

    // A different seed diverges.
    let other = run_cli(&dir, 2027, n, "c");
    assert_ne!(run.train_lines, other.train_lines);

    let _ = std::fs::remove_dir_all(&dir);
}
