//! End-to-end tests of the `opencodifier` binary.
//!
//! Every test drives the real executable against real files in a private
//! scratch directory and asserts on what a caller can observe: exit code,
//! `stdout` JSON, and the stable code on `stderr`. No fixture pretends to
//! be a model, no assertion reaches into internals.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

use std::fs;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};

use assert_cmd::Command;
use opencodifier_core::{
    Candidate, ChoiceQuestion, DecisionPolicy, DecisionQuestion, DecisionRequest, RequestMetadata,
    ScoreLevel, ScoreQuestion, State,
};
use opencodifier_engine::DecisionGraph;
use opencodifier_schema::WireFormat;
use opencodifier_schema::native::Native;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Uniqueness counter for scratch directories, so parallel tests and
/// repeated runs never share a path.
static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

/// A private temporary directory, removed when the test ends.
struct Scratch(PathBuf);

impl Scratch {
    /// Creates a uniquely named directory under the temp dir (`TMPDIR`).
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "opencodifier-cli-{label}-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).expect("scratch directory");
        Self(path)
    }

    /// The directory's path, for tests that need a name that must not
    /// exist.
    fn path(&self) -> &Path {
        &self.0
    }

    /// Writes `bytes` into a file named `name` and returns its path.
    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, bytes).expect("fixture file");
        path
    }

    /// Writes `value` as JSON into `name` and returns its path.
    fn write_json(&self, name: &str, value: &Value) -> PathBuf {
        let mut bytes = serde_json::to_vec_pretty(value).expect("serialize fixture");
        bytes.push(b'\n');
        self.write(name, &bytes)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The binary under test.
fn opencodifier() -> Command {
    Command::new(env!("CARGO_BIN_EXE_opencodifier"))
}

/// A policy that accepts whatever the lexical engine produces, so the
/// end-to-end assertions exercise the pipeline rather than the gate.
fn permissive_policy() -> Value {
    json!({ "min_confidence": 0.0, "verify_below": 0.0, "abstain_below": 0.0, "risk": "low" })
}

/// A `DecisionPolicy` decoded from its JSON form, exactly as `--policy`
/// decodes one.
fn policy(value: Value) -> DecisionPolicy {
    serde_json::from_value(value).expect("valid policy")
}

/// A choice request over two candidates, in IR form.
fn choice_request_ir(policy_json: Value) -> DecisionRequest {
    let question = DecisionQuestion::Choice(
        ChoiceQuestion::new(
            "model",
            "Which model should answer this request?",
            vec![
                Candidate::new("local-qwen", "fast general coding").expect("valid candidate"),
                Candidate::new("local-glm", "deep reasoning specialist").expect("valid candidate"),
            ],
        )
        .expect("valid question"),
    );
    DecisionRequest::new(
        State::from_text("deep reasoning over a large proof"),
        vec![question],
        policy(policy_json),
        RequestMetadata::default(),
    )
    .expect("valid request")
}

/// A native choice request over two candidates, in its wire form.
fn choice_request(policy_json: Value) -> Value {
    Native.encode_request(&choice_request_ir(policy_json)).expect("native encoding")
}

/// The schema adapter a `--format` name selects, for building fixtures.
fn adapter_for(name: &str) -> Box<dyn WireFormat> {
    match name {
        "openai" => Box::new(opencodifier_schema::openai::OpenAi),
        "anthropic" => Box::new(opencodifier_schema::anthropic::Anthropic),
        "jev" => Box::new(opencodifier_schema::jev::Jev),
        other => panic!("no fixture adapter for format {other}"),
    }
}

/// A native score request over three ordered levels, in its wire form.
fn score_request(policy_json: Value) -> Value {
    let levels: Vec<ScoreLevel> = ["trivial", "moderate", "expert"]
        .iter()
        .map(|label| ScoreLevel::new(*label).expect("valid level"))
        .collect();
    let question = DecisionQuestion::Score(
        ScoreQuestion::new("difficulty", "How difficult is it?", levels).expect("valid question"),
    );
    let request = DecisionRequest::new(
        State::from_text("rewrite the parser and add regression tests"),
        vec![question],
        policy(policy_json),
        RequestMetadata::default(),
    )
    .expect("valid request");
    Native.encode_request(&request).expect("native encoding")
}

/// Runs `decide` against a payload file.
fn decide(payload: &Path) -> Command {
    let mut command = opencodifier();
    command.arg("decide").arg("--input").arg(payload);
    command
}

/// SHA-256 of `bytes`, lowercase hex — the form a manifest carries.
fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("{digest:x}")
}

#[test]
fn choice_request_decides_and_prints_canonical_json() {
    let scratch = Scratch::new("choice");
    let payload = scratch.write_json("request.json", &choice_request(permissive_policy()));

    let output = decide(&payload).output().expect("run opencodifier");
    assert_eq!(output.status.code(), Some(0), "stderr: {:?}", output.stderr);

    let response: Value =
        serde_json::from_slice(&output.stdout).expect("stdout is one JSON document");
    assert_eq!(response["outcome"], "accept", "{response}");
    let answers = response["answers"].as_array().expect("answers array");
    assert_eq!(answers.len(), 1);
    assert_eq!(answers[0]["type"], "choice");
    assert_eq!(answers[0]["question_id"], "model");
    let choice = answers[0]["choice"].as_str().expect("chosen candidate id");
    assert!(
        choice == "local-qwen" || choice == "local-glm",
        "answer must name a real candidate, got {choice}"
    );
    // The distribution is real evidence, not a placeholder: two entries,
    // each a probability, summing to one.
    let entries = answers[0]["distribution"]["entries"].as_array().expect("distribution");
    assert_eq!(entries.len(), 2);
    let total: f64 =
        entries.iter().map(|entry| entry["probability"].as_f64().expect("probability")).sum();
    assert!((total - 1.0).abs() < 1e-6, "distribution sums to {total}");
}

#[test]
fn score_request_decides_and_prints_level_keys() {
    let scratch = Scratch::new("score");
    let payload = scratch.write_json("request.json", &score_request(permissive_policy()));

    let output = decide(&payload).output().expect("run opencodifier");
    assert_eq!(output.status.code(), Some(0), "stderr: {:?}", output.stderr);

    let response: Value =
        serde_json::from_slice(&output.stdout).expect("stdout is one JSON document");
    assert_eq!(response["outcome"], "accept", "{response}");
    let answer = &response["answers"].as_array().expect("answers array")[0];
    assert_eq!(answer["type"], "score");
    assert_eq!(answer["question_id"], "difficulty");

    let level = answer["level"].as_str().expect("level label");
    let keys: Vec<&str> = answer["distribution"]["entries"]
        .as_array()
        .expect("distribution")
        .iter()
        .map(|entry| entry["key"].as_str().expect("level key"))
        .collect();
    assert_eq!(keys, vec!["trivial", "moderate", "expert"], "levels keep their order");
    assert!(keys.contains(&level), "level `{level}` is one of the declared keys {keys:?}");
    assert!(answer["expected"].as_f64().expect("expected value").is_finite());
}

#[test]
fn trace_prints_the_execution_report_after_the_response() {
    let scratch = Scratch::new("trace");
    let payload = scratch.write_json("request.json", &choice_request(permissive_policy()));

    let output = decide(&payload).arg("--trace").output().expect("run opencodifier");
    assert_eq!(output.status.code(), Some(0), "stderr: {:?}", output.stderr);

    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    let documents: Vec<Value> = serde_json::Deserializer::from_str(&stdout)
        .into_iter::<Value>()
        .collect::<Result<_, _>>()
        .expect("two JSON documents");
    assert_eq!(documents.len(), 2, "response, then the execution report");

    let report = &documents[1];
    let waves = report["waves"].as_array().expect("waves array");
    assert!(!waves.is_empty(), "the default pipeline executes at least one wave");
    assert_eq!(report["cache_hit"], false, "a first run is a cache miss");
    assert_eq!(report["cache_key"].as_str().expect("cache key").len(), 64);
    assert!(report["outcomes"].as_array().expect("outcomes")[0]["outcome"] == "accept", "{report}");
}

#[test]
fn every_wire_format_decides_the_same_choice_request() {
    let scratch = Scratch::new("formats");
    let request = choice_request_ir(permissive_policy());
    // Foreign formats deliberately carry no policy: the gates are local.
    // The CLI's `--policy` is what supplies them, so the same request
    // decides decisively whatever shape it arrived in.
    let policy_path = scratch.write_json("policy.json", &permissive_policy());

    for name in ["openai", "anthropic", "jev"] {
        let payload = scratch.write_json(
            &format!("{name}-request.json"),
            &adapter_for(name).encode_request(&request).expect("wire encoding"),
        );

        let output = decide(&payload)
            .arg("--format")
            .arg(name)
            .arg("--policy")
            .arg(&policy_path)
            .output()
            .expect("run opencodifier");
        assert_eq!(output.status.code(), Some(0), "{name}: stderr: {:?}", output.stderr);
        let response: Value =
            serde_json::from_slice(&output.stdout).expect("{name}: stdout is one JSON document");
        assert_eq!(response["answers"][0]["type"], "choice", "{response}");
    }
}

#[test]
fn stdin_is_the_default_input() {
    let scratch = Scratch::new("stdin");
    let payload = scratch.write_json("request.json", &choice_request(permissive_policy()));

    let bytes = std::fs::read(&payload).expect("fixture bytes");
    opencodifier()
        .args(["decide", "--input", "-"])
        .write_stdin(bytes)
        .assert()
        .success()
        .stdout(predicates::str::contains("\"outcome\": \"accept\""));
}

#[test]
fn an_unknown_format_is_an_input_error() {
    let scratch = Scratch::new("bad-format");
    let payload = scratch.write_json("request.json", &choice_request(permissive_policy()));

    decide(&payload)
        .arg("--format")
        .arg("gpt")
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains("error:"));
}

#[test]
fn abstention_exits_two_and_zero_with_abstain_is_success() {
    let scratch = Scratch::new("abstain");
    let payload = scratch.write_json("request.json", &choice_request(permissive_policy()));
    // A gate nothing can clear: every confidence below 1.0 abstains.
    let strict = scratch.write_json(
        "strict-policy.json",
        &json!({ "min_confidence": 1.0, "verify_below": 1.0, "abstain_below": 1.0, "risk": "low" }),
    );

    let refused = decide(&payload).arg("--policy").arg(&strict).output().expect("run opencodifier");
    assert_eq!(refused.status.code(), Some(2), "stderr: {:?}", refused.stderr);
    let response: Value =
        serde_json::from_slice(&refused.stdout).expect("stdout is still one JSON document");
    assert_eq!(response["outcome"], "abstain", "{response}");
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(stderr.contains("cli.escalated"), "{stderr}");

    let accepted = decide(&payload)
        .arg("--policy")
        .arg(&strict)
        .arg("--abstain-is-success")
        .output()
        .expect("run opencodifier");
    assert_eq!(accepted.status.code(), Some(0), "stderr: {:?}", accepted.stderr);
    let response: Value =
        serde_json::from_slice(&accepted.stdout).expect("stdout is one JSON document");
    assert_eq!(response["outcome"], "abstain", "{response}");
}

#[test]
fn a_policy_that_contradicts_the_ir_is_an_input_error() {
    let scratch = Scratch::new("policy");
    let payload = scratch.write_json("request.json", &choice_request(permissive_policy()));
    // Parses as a policy but violates `abstain_below <= verify_below <=
    // min_confidence`, so the IR must refuse it.
    let incoherent = scratch.write_json(
        "incoherent-policy.json",
        &json!({ "min_confidence": 0.2, "verify_below": 0.9, "abstain_below": 0.5, "risk": "low" }),
    );

    let output =
        decide(&payload).arg("--policy").arg(&incoherent).output().expect("run opencodifier");
    assert_eq!(output.status.code(), Some(1), "stderr: {:?}", output.stderr);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("schema.invalid_value"), "{stderr}");
}

#[test]
fn malformed_json_exits_one_with_a_schema_code() {
    let scratch = Scratch::new("malformed");
    let payload = scratch.write("request.json", b"{ not json");

    decide(&payload)
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains("schema.invalid_json"));
}

#[test]
fn an_unreadable_input_file_exits_one() {
    let scratch = Scratch::new("unreadable");
    let missing = scratch.path().join("absent.json");

    decide(&missing)
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains("cli.unreadable_input"));
}

#[test]
fn graph_validate_accepts_the_default_pipeline_and_rejects_cycles() {
    let scratch = Scratch::new("graph");
    let pipeline = scratch.write_json(
        "pipeline.json",
        &serde_json::to_value(DecisionGraph::default_pipeline().expect("built-in pipeline"))
            .expect("serialize graph"),
    );

    opencodifier()
        .args(["graph", "validate"])
        .arg(&pipeline)
        .assert()
        .success()
        .stdout(predicates::str::contains("ok: graph"));

    // Two rule nodes depending on each other: no topological order exists.
    let cyclic = scratch.write_json(
        "cycle.json",
        &json!({
            "version": 1,
            "nodes": [
                { "id": "a", "kind": "rule", "depends_on": ["b"] },
                { "id": "b", "kind": "rule", "depends_on": ["a"] }
            ]
        }),
    );
    opencodifier()
        .args(["graph", "validate"])
        .arg(&cyclic)
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains("graph.cycle"));
}

#[test]
fn graph_validate_rejects_a_document_without_the_graph_shape() {
    let scratch = Scratch::new("graph-shape");
    let not_a_graph = scratch.write_json("wrong.json", &json!({ "nodes": "everything" }));

    opencodifier()
        .args(["graph", "validate"])
        .arg(&not_a_graph)
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains("cli.invalid_graph_json"));
}

#[test]
fn models_verify_accepts_matching_bytes_and_rejects_tampered_ones() {
    let scratch = Scratch::new("models");
    let bytes = b"opencodifier decision model artifact bytes";
    let artifact = scratch.write("decision.onnx", bytes);
    let manifest = scratch.write_json(
        "decision.onnx.manifest.json",
        &json!({
            "model_id": "decision-v0",
            "sha256": sha256_hex(bytes),
            "format": "onnx",
            "revision": "r2026.09.1"
        }),
    );

    // No `--artifact`: the manifest's sibling is the artifact it names.
    opencodifier()
        .args(["models", "verify", "--manifest"])
        .arg(&manifest)
        .assert()
        .success()
        .stdout(predicates::str::contains("ok: artifact"));

    // An explicit path verifies the same bytes.
    opencodifier()
        .args(["models", "verify", "--manifest"])
        .arg(&manifest)
        .arg("--artifact")
        .arg(&artifact)
        .assert()
        .success();

    // One changed byte is a different artifact, and must never load.
    let tampered = scratch.write("decision.onnx", b"opencodifier decision model artifact bytes!");
    opencodifier()
        .args(["models", "verify", "--manifest"])
        .arg(&manifest)
        .arg("--artifact")
        .arg(&tampered)
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains("model.sha256_mismatch"));
}

#[test]
fn models_verify_rejects_a_missing_artifact() {
    let scratch = Scratch::new("models-missing");
    let manifest = scratch.write_json(
        "decision.onnx.manifest.json",
        &json!({
            "model_id": "decision-v0",
            "sha256": sha256_hex(b"artifacts that were never written"),
            "format": "onnx",
            "revision": "r2026.09.1"
        }),
    );

    opencodifier()
        .args(["models", "verify", "--manifest"])
        .arg(&manifest)
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains("model.unreadable_artifact"));
}

#[test]
fn serve_refuses_a_non_loopback_bind_without_allow_remote() {
    opencodifier()
        .args(["serve", "--bind", "0.0.0.0:9999"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains("cli.bind_requires_allow_remote"));
}

#[test]
fn serve_starts_serves_healthz_and_warns_about_inapplicable_policy() {
    // The one long-running-path test: `serve` is spawned for real, its
    // startup lines are read off stderr, and the HTTP surface is probed
    // over an actual socket — everything short of Ctrl-C.
    let scratch = Scratch::new("serve-e2e");
    let policy_path = scratch.write_json("policy.json", &permissive_policy());

    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_opencodifier"))
        .args(["serve", "--bind", "127.0.0.1:0", "--policy"])
        .arg(&policy_path)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn opencodifier serve");

    let stderr = child.stderr.take().expect("piped stderr");
    let (sender, receiver) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        use std::io::BufRead as _;
        for line in std::io::BufReader::new(stderr).lines().map_while(Result::ok) {
            if sender.send(line).is_err() {
                break;
            }
        }
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let mut seen: Vec<String> = Vec::new();
    let mut bound_url = None;
    while bound_url.is_none() && std::time::Instant::now() < deadline {
        match receiver.recv_timeout(std::time::Duration::from_millis(500)) {
            Ok(line) => {
                if line.contains("listening on http://") {
                    bound_url = Some(line);
                } else {
                    seen.push(line);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    let bound_url = bound_url.unwrap_or_else(|| {
        child.kill().ok();
        panic!("serve never reported its bound address; stderr lines: {seen:?}");
    });

    // The validated-but-inapplicable `--policy` was reported before the
    // socket opened, exactly as documented.
    assert!(
        seen.iter().any(|line| line.contains("cli.policy_inapplicable")),
        "policy warning must precede the listening line; got {seen:?}"
    );

    let bound = bound_url.split("http://").nth(1).expect("bound address in line");
    let mut stream = std::net::TcpStream::connect(bound).expect("connect to served port");
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).expect("read timeout");
    write!(stream, "GET /v1/healthz HTTP/1.1\r\nHost: {bound}\r\nConnection: close\r\n\r\n")
        .expect("write request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read response");
    assert!(response.starts_with("HTTP/1.1 200"), "healthz must answer 200: {response}");
    assert!(response.contains("\"status\":\"ok\""), "healthz body: {response}");

    child.kill().ok();
    child.wait().ok();
}

#[test]
fn serve_rejects_a_bad_graph_before_opening_the_socket() {
    let scratch = Scratch::new("serve-graph");
    let cyclic = scratch.write_json(
        "cycle.json",
        &json!({
            "version": 1,
            "nodes": [
                { "id": "a", "kind": "rule", "depends_on": ["b"] },
                { "id": "b", "kind": "rule", "depends_on": ["a"] }
            ]
        }),
    );

    opencodifier()
        .args(["serve", "--bind", "127.0.0.1:0", "--graph"])
        .arg(&cyclic)
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains("graph.cycle"));
}

#[test]
fn serve_rejects_an_incoherent_policy_before_opening_the_socket() {
    let scratch = Scratch::new("serve-policy");
    let incoherent = scratch.write_json(
        "policy.json",
        &json!({ "min_confidence": 0.2, "verify_below": 0.9, "abstain_below": 0.5, "risk": "low" }),
    );

    opencodifier()
        .args(["serve", "--bind", "127.0.0.1:0", "--policy"])
        .arg(&incoherent)
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains("schema.invalid_value"));
}

#[test]
fn serve_rejects_an_unparsable_bind() {
    opencodifier()
        .args(["serve", "--bind", "not-an-address"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains("cli.invalid_bind"));
}

#[test]
fn bad_arguments_exit_one() {
    // Clap's own default is exit 2, which this CLI reserves for a
    // policy-gate escalation, so an unrecognized subcommand is normalized
    // to the input code.
    opencodifier()
        .arg("definitely-not-a-subcommand")
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains("error:"));
}

#[test]
fn help_exits_zero_and_documents_the_exit_codes() {
    let output = opencodifier().arg("--help").output().expect("run opencodifier");
    assert_eq!(output.status.code(), Some(0));
    let help = String::from_utf8(output.stdout).expect("help is utf-8");
    assert!(help.contains("EXIT CODES"), "{help}");
    assert!(help.contains("--abstain-is-success"), "{help}");
}
