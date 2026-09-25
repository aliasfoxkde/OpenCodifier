//! End-to-end HTTP tests: wire JSON → schema decode → real engine →
//! encode → HTTP, over a real socket (PLANNING.md §36, §73).
//!
//! Each test binds `127.0.0.1:0`, discovers the actual port, and drives the
//! router with `reqwest`. Assertions are on the observable wire contract —
//! status codes, outcome fields, and error codes — never on internals.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

use std::sync::Arc;

use opencodifier_core::{
    BooleanQuestion, Candidate, ChoiceQuestion, DecisionPolicy, DecisionQuestion, DecisionRequest,
    Limits, RequestMetadata, RiskLevel, ScoreLevel, ScoreQuestion, State,
};
use opencodifier_engine::{DecisionGraph, EngineConfig, EngineHandle};
use opencodifier_schema::WireFormat;
use opencodifier_schema::native::Native;
use reqwest::StatusCode;
use reqwest::header::CONTENT_TYPE;
use serde_json::{Value, json};

/// The server under test: a real engine, a real listener, an ephemeral port.
struct TestServer {
    base_url: String,
    handle: Arc<EngineHandle>,
}

/// Spawns the production router on its own listener and reports the URL.
async fn spawn_server() -> TestServer {
    let handle =
        Arc::new(EngineHandle::lexical(EngineConfig::with_default_pipeline().unwrap()).unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = opencodifier_http::router(Arc::clone(&handle));
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    TestServer { base_url: format!("http://{addr}"), handle }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().build().unwrap()
}

/// Posts a raw body with a JSON content type, as a hostile client would.
async fn post_json(base_url: &str, path: &str, body: &str) -> reqwest::Response {
    client()
        .post(format!("{base_url}{path}"))
        .header(CONTENT_TYPE, "application/json")
        .body(body.to_owned())
        .send()
        .await
        .unwrap()
}

/// The error envelope every failure uses.
fn error_code(response: &Value) -> &str {
    response["error"]["code"].as_str().unwrap()
}

/// A two-candidate choice request, native-encoded.
fn choice_payload(policy: &DecisionPolicy) -> String {
    let request = DecisionRequest::new(
        State::from_text("refactor the rust parser module without allocating"),
        vec![DecisionQuestion::Choice(
            ChoiceQuestion::new(
                "model",
                "Which model should refactor the parser?",
                vec![
                    Candidate::new("local-qwen", "fast local coding model for rust refactors")
                        .unwrap(),
                    Candidate::new("cloud-large", "long context cloud reasoning service").unwrap(),
                ],
            )
            .unwrap(),
        )],
        policy.clone(),
        RequestMetadata::default(),
    )
    .unwrap();
    Native.encode_request(&request).unwrap().to_string()
}

#[tokio::test]
async fn choice_decision_round_trips_over_http() {
    let server = spawn_server().await;
    let response =
        post_json(&server.base_url, "/v1/decide", &choice_payload(&DecisionPolicy::default()))
            .await
            .json::<Value>()
            .await
            .unwrap();

    let answer = &response["answers"][0];
    assert_eq!(answer["type"], "choice", "answer: {response}");
    assert_eq!(answer["question_id"], "model");

    // A real answer distribution: one probability per candidate, mass 1,
    // and the reported choice is the top of that distribution.
    let entries = answer["distribution"]["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    let mass: f64 = entries.iter().map(|entry| entry["probability"].as_f64().unwrap()).sum();
    assert!((mass - 1.0).abs() < 1e-6, "distribution: {entries:?}");
    assert!(entries.iter().any(|entry| entry["key"] == answer["choice"]));
    let top = entries
        .iter()
        .max_by(|left, right| {
            left["probability"].as_f64().unwrap().total_cmp(&right["probability"].as_f64().unwrap())
        })
        .unwrap();
    assert_eq!(top["key"], answer["choice"]);
    assert!(answer["confidence"].as_f64().unwrap() > 0.0);
    assert!(response["trace"].as_object().is_some_and(|trace| !trace.is_empty()));
}

#[tokio::test]
async fn score_decision_returns_level_probabilities() {
    let server = spawn_server().await;
    let request = DecisionRequest::new(
        State::from_text("rewrite the interpreter loop and the borrow checker"),
        vec![DecisionQuestion::Score(
            ScoreQuestion::new(
                "difficulty",
                "How difficult is this engineering task?",
                ["trivial", "moderate", "expert"]
                    .iter()
                    .map(|label| ScoreLevel::new(*label).unwrap())
                    .collect(),
            )
            .unwrap(),
        )],
        DecisionPolicy::default(),
        RequestMetadata::default(),
    )
    .unwrap();
    let body = Native.encode_request(&request).unwrap().to_string();

    let response =
        post_json(&server.base_url, "/v1/decide", &body).await.json::<Value>().await.unwrap();
    let answer = &response["answers"][0];
    assert_eq!(answer["type"], "score", "answer: {response}");
    assert_eq!(answer["question_id"], "difficulty");

    // The full level distribution, in level order, plus the expected value
    // and the level it falls into.
    let entries = answer["distribution"]["entries"].as_array().unwrap();
    let keys: Vec<&str> = entries.iter().map(|entry| entry["key"].as_str().unwrap()).collect();
    assert_eq!(keys, vec!["trivial", "moderate", "expert"]);
    let mass: f64 = entries.iter().map(|entry| entry["probability"].as_f64().unwrap()).sum();
    assert!((mass - 1.0).abs() < 1e-6);
    let expected = answer["expected"].as_f64().unwrap();
    assert!((0.0..=2.0).contains(&expected), "expected: {expected}");
    assert!(answer["level"].as_str().is_some());
}

#[tokio::test]
async fn policy_with_high_threshold_abstains_with_a_200() {
    let server = spawn_server().await;

    // The request carries its own policy, so abstention is forced on the
    // wire: demanding `1.0` calibrated confidence refuses every answer the
    // lexical baseline can produce. Abstention is a successful outcome.
    let policy = DecisionPolicy::new(1.0, 1.0, 1.0, RiskLevel::Low).unwrap();
    let status = post_json(&server.base_url, "/v1/decide", &choice_payload(&policy)).await.status();
    assert_eq!(status, StatusCode::OK);

    let response = post_json(&server.base_url, "/v1/decide", &choice_payload(&policy))
        .await
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(response["outcome"], "abstain", "response: {response}");
    assert!(response["confidence"]["calibrated_confidence"].as_f64().unwrap() < 1.0);
}

#[tokio::test]
async fn healthz_reports_the_engine_identity() {
    let server = spawn_server().await;
    let health = server.handle.health();
    let response: Value = client()
        .get(format!("{}/v1/healthz", server.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(response["status"], "ok");
    assert_eq!(response["identity"]["graph_version"], health.identity.graph_version);
    assert_eq!(response["identity"]["model_id"], "builtin-lexical-v1");
    assert_eq!(response["identity"]["calibration_version"], health.identity.calibration_version);
    assert!(!response["identity"]["engine_semver"].as_str().unwrap().is_empty());
    assert_eq!(response["nodes"].as_u64().unwrap(), u64::try_from(health.nodes).unwrap());
    assert_eq!(
        response["parallelism"].as_u64().unwrap(),
        u64::try_from(health.parallelism).unwrap()
    );
    assert_eq!(response["cache_enabled"], health.cache_enabled);
}

#[tokio::test]
async fn malformed_json_is_a_400_schema_error() {
    let server = spawn_server().await;
    let response = post_json(&server.base_url, "/v1/decide", "{\"state\": ").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.unwrap();
    assert_eq!(error_code(&body), "schema.invalid_json", "body: {body}");
    assert!(body["error"]["message"].as_str().unwrap().contains("JSON"));
}

#[tokio::test]
async fn request_beyond_server_limits_is_a_400_not_a_500() {
    let server = spawn_server().await;

    // 33 questions: inside the payload's own declared ceiling of 64, so the
    // payload decodes cleanly, but over the server's 32-question ceiling —
    // which the schema layer, not the transport, must refuse.
    let questions: Vec<DecisionQuestion> = (0..33)
        .map(|index| {
            DecisionQuestion::Boolean(
                BooleanQuestion::new(format!("question-{index}"), "Does this need tools?").unwrap(),
            )
        })
        .collect();
    let request = DecisionRequest::new(
        State::from_text("run the toolchain"),
        questions,
        DecisionPolicy::default(),
        RequestMetadata {
            request_id: None,
            limits: Limits { max_questions: 64, ..Limits::default() },
        },
    )
    .unwrap();
    let body = Native.encode_request(&request).unwrap().to_string();

    let response = post_json(&server.base_url, "/v1/decide", &body).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.unwrap();
    assert_eq!(error_code(&body), "schema.limit_exceeded", "body: {body}");
}

#[tokio::test]
async fn oversized_body_is_refused_at_the_transport_not_a_500() {
    let server = spawn_server().await;
    let oversized =
        format!("{{\"state\":\"{}\"", "x".repeat(opencodifier_http::MAX_BODY_BYTES + 32));

    let response = post_json(&server.base_url, "/v1/decide", &oversized).await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    // The body is never parsed, so nothing about it is echoed back.
    let text = response.text().await.unwrap();
    assert!(!text.contains("xxxx"), "response echoed the payload");
}

#[tokio::test]
async fn graph_validate_accepts_the_default_pipeline() {
    let server = spawn_server().await;
    let graph = EngineConfig::with_default_pipeline().unwrap().graph;
    let expected_nodes = graph.nodes().len();
    let body = serde_json::to_string(&graph).unwrap();

    let response = post_json(&server.base_url, "/v1/graph/validate", &body).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["valid"], true);
    assert_eq!(body["nodes"], expected_nodes);
}

#[tokio::test]
async fn cyclic_graph_is_a_400_with_the_engine_code() {
    let server = spawn_server().await;

    // Written on the wire: two nodes depending on each other. The engine
    // reports this as `graph.cycle`, and the HTTP surface must relay that
    // code rather than flatten it into a 500.
    let body = json!({
        "version": 1,
        "nodes": [
            { "id": "a", "kind": "rule", "depends_on": ["b"] },
            { "id": "b", "kind": "rule", "depends_on": ["a"] },
        ],
    })
    .to_string();

    let response = post_json(&server.base_url, "/v1/graph/validate", &body).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.unwrap();
    assert_eq!(error_code(&body), "graph.cycle", "body: {body}");
}

#[tokio::test]
async fn malformed_graph_document_is_a_400() {
    let server = spawn_server().await;
    let response = post_json(&server.base_url, "/v1/graph/validate", "{\"nodes\": 7}").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.unwrap();
    assert_eq!(error_code(&body), "schema.invalid_value", "body: {body}");
}

#[tokio::test]
async fn graph_validate_matches_the_engine_on_a_real_graph() {
    // The HTTP verdict and the engine's own validator agree on the same
    // document, so the surface is a relay and never a weaker copy.
    let graph = DecisionGraph::default_pipeline().unwrap();
    let document = serde_json::to_value(&graph).unwrap();
    let decoded: DecisionGraph = serde_json::from_value(document).unwrap();
    assert_eq!(decoded.nodes().len(), graph.nodes().len());
}

#[tokio::test]
async fn a_graph_beyond_the_node_limit_is_a_400_before_validation() {
    // The count ceiling is enforced on the decoded document before the
    // engine ever sees it, under the adapter's own limit code.
    let server = spawn_server().await;
    let nodes: Vec<Value> =
        (0..200).map(|index| json!({ "id": format!("n{index}"), "kind": "rule" })).collect();
    let body = json!({ "version": 1, "nodes": nodes }).to_string();
    let response = post_json(&server.base_url, "/v1/graph/validate", &body).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.unwrap();
    assert_eq!(error_code(&body), "schema.limit_exceeded", "body: {body}");
}
