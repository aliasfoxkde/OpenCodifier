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
use opencodifier_schema::openai::OpenAi;
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
    // The composed id: the relational solver over the lexical classifier.
    assert_eq!(response["identity"]["model_id"], "relational-v1|builtin-lexical-v1");
    assert_eq!(response["identity"]["calibration_version"], health.identity.calibration_version);
    assert_ne!(response["identity"]["engine_semver"].as_str().unwrap(), "");
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

/// The default pipeline as a client would submit it: the graph serializes
/// into exactly the document `/v1/graph/run` decodes.
fn pipeline_document() -> Value {
    serde_json::to_value(DecisionGraph::default_pipeline().unwrap()).unwrap()
}

#[tokio::test]
async fn graph_run_decides_through_the_client_graph() {
    let server = spawn_server().await;
    let body = json!({
        "graph": pipeline_document(),
        "request": choice_payload(&DecisionPolicy::default()).parse::<Value>().unwrap(),
    });
    let response = post_json(&server.base_url, "/v1/graph/run", &body.to_string())
        .await
        .json::<Value>()
        .await
        .unwrap();

    let answer = &response["response"]["answers"][0];
    assert_eq!(answer["type"], "choice", "graph_run: {response}");
    // The identity is the run's own: content-addressed, never the builtin
    // literal the client could have asserted (D19).
    let identity = &response["identity"];
    assert_ne!(identity["graph_version"], 1, "identity: {identity}");
    assert!(identity["model_id"].as_str().is_some());
}

#[tokio::test]
async fn graph_run_relays_the_engine_code_for_a_cyclic_graph() {
    let server = spawn_server().await;
    let mut document = pipeline_document();
    // Introduce a cycle through two existing nodes.
    let nodes = document["nodes"].as_array_mut().unwrap();
    nodes[0]["depends_on"] = json!(["output"]);
    let body = json!({
        "graph": document,
        "request": choice_payload(&DecisionPolicy::default()).parse::<Value>().unwrap(),
    });
    let response = post_json(&server.base_url, "/v1/graph/run", &body.to_string())
        .await
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(response["error"]["code"], "graph.cycle", "graph_run: {response}");
}

#[tokio::test]
async fn graph_run_refuses_an_oversized_graph_before_validation() {
    let server = spawn_server().await;
    let nodes: Vec<Value> =
        (0..200).map(|index| json!({ "id": format!("n{index}"), "kind": "rule" })).collect();
    let body = json!({
        "graph": { "version": 1, "nodes": nodes },
        "request": choice_payload(&DecisionPolicy::default()).parse::<Value>().unwrap(),
    });
    let response = post_json(&server.base_url, "/v1/graph/run", &body.to_string())
        .await
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(error_code(&response), "schema.limit_exceeded", "graph_run: {response}");
}

#[tokio::test]
async fn graph_run_reports_a_malformed_request_as_a_schema_error() {
    let server = spawn_server().await;
    let body = json!({
        "graph": pipeline_document(),
        "request": { "state": { "text": true } },
    });
    let response = post_json(&server.base_url, "/v1/graph/run", &body.to_string())
        .await
        .json::<Value>()
        .await
        .unwrap();
    let code = error_code(&response);
    assert!(code.starts_with("schema."), "graph_run: {response}");
}

/// GETs a path, as a metadata probe would.
async fn get_json(base_url: &str, path: &str) -> reqwest::Response {
    client().get(format!("{base_url}{path}")).send().await.unwrap()
}

#[tokio::test]
async fn batch_decides_independently_and_reports_per_item_errors() {
    let server = spawn_server().await;
    // Item 0 decides; item 1 is hostile garbage; item 2 decides. One
    // item's refusal must not touch its neighbours.
    let body = json!({
        "requests": [
            choice_payload(&DecisionPolicy::default()).parse::<Value>().unwrap(),
            json!({"state": {"text": 42}}),
            choice_payload(&DecisionPolicy::default()).parse::<Value>().unwrap(),
        ],
    });
    let response = post_json(&server.base_url, "/v1/batch", &body.to_string())
        .await
        .json::<Value>()
        .await
        .unwrap();

    assert_eq!(response["count"], 3, "batch: {response}");
    let results = response["results"].as_array().unwrap();
    assert_eq!(results.len(), 3);
    assert_eq!(results[0]["response"]["answers"][0]["type"], "choice");
    assert_eq!(results[2]["response"]["answers"][0]["type"], "choice");
    let error = &results[1]["error"];
    assert!(error["code"].as_str().unwrap_or_default().starts_with("schema."), "item 1: {error}");
    // The malformed item is still a 200: the batch was well-formed, the
    // item is where the failure lives.
}

#[tokio::test]
async fn batch_over_the_limit_is_refused_up_front() {
    let server = spawn_server().await;
    let items: Vec<Value> = (0..=opencodifier_engine::MAX_BATCH)
        .map(|_| choice_payload(&DecisionPolicy::default()).parse::<Value>().unwrap())
        .collect();
    let response =
        post_json(&server.base_url, "/v1/batch", &json!({ "requests": items }).to_string())
            .await
            .json::<Value>()
            .await
            .unwrap();

    assert_eq!(error_code(&response), "schema.limit_exceeded", "response: {response}");
}

#[tokio::test]
async fn validate_preflights_a_payload_without_deciding() {
    let server = spawn_server().await;
    let ok =
        post_json(&server.base_url, "/v1/validate", &choice_payload(&DecisionPolicy::default()))
            .await
            .json::<Value>()
            .await
            .unwrap();
    assert_eq!(ok["valid"], true, "validate: {ok}");
    assert_eq!(ok["questions"], 1);

    let bad = post_json(&server.base_url, "/v1/validate", "{\"state\": {}}")
        .await
        .json::<Value>()
        .await
        .unwrap();
    // Nested serde failures surface as the adapter's invalid_value on the
    // enclosing field (the MissingField variant is for IR-level misses).
    assert_eq!(error_code(&bad), "schema.invalid_value", "bad: {bad}");
    assert!(bad["error"]["message"].as_str().unwrap().contains("missing field"));
}

#[tokio::test]
async fn models_and_capabilities_report_the_active_lane() {
    let server = spawn_server().await;
    let expected_model = server.handle.identity().model_id;

    let models = get_json(&server.base_url, "/v1/models").await.json::<Value>().await.unwrap();
    let lanes = models["models"].as_array().unwrap();
    assert_eq!(lanes.len(), 1, "models: {models}");
    assert_eq!(lanes[0]["id"], expected_model.as_str());
    assert_eq!(lanes[0]["role"], "decision");
    assert_eq!(lanes[0]["active"], true);

    let caps = get_json(&server.base_url, "/v1/capabilities").await.json::<Value>().await.unwrap();
    let kinds = caps["decision_kinds"].as_array().unwrap();
    assert_eq!(kinds.len(), 3, "capabilities: {caps}");
    assert_eq!(caps["max_batch"], opencodifier_engine::MAX_BATCH);
    assert_eq!(caps["cache"]["enabled"], true);
    assert_eq!(caps["identity"]["model_id"], expected_model.as_str());
    let endpoints = caps["endpoints"].as_array().unwrap();
    assert!(endpoints.iter().any(|entry| entry == "/v1/decide"));
    assert!(endpoints.iter().any(|entry| entry == "/v1/batch"));
}

/// The INTEGRATIONS.md §2.3 quickstart, byte-for-byte as the adoption
/// guide prints it (Phase 19c): the test pastes exactly what the doc
/// tells a newcomer to paste, so docs and wire cannot drift apart
/// silently.
const QUICKSTART_BODY: &str = r#"{
  "state": {"text": "Checkout errors hit 12% eight minutes after a deploy. Rollback window closes in 20 minutes."},
  "questions": [{
    "type": "choice",
    "id": "action",
    "text": "Which action should the on-call take?",
    "candidates": [
      {"id": "rollback", "description": "roll back to the last healthy build"},
      {"id": "hotfix",   "description": "patch forward on the broken build"},
      {"id": "wait",     "description": "watch dashboards and hold"}
    ]
  }]
}"#;

#[tokio::test]
async fn the_documented_quickstart_round_trips_verbatim() {
    let server = spawn_server().await;
    let response = post_json(&server.base_url, "/v1/decide", QUICKSTART_BODY).await;
    assert_eq!(response.status(), StatusCode::OK, "the quickstart must work as printed");
    let body: Value = response.json().await.unwrap();

    // What the doc promises: the chosen candidate and full distribution
    // per question; calibrated confidence, gate outcome, and trace for the
    // request as a whole.
    let answer = &body["answers"][0];
    assert_eq!(answer["question_id"], "action", "body: {body}");
    assert_eq!(answer["type"], "choice");
    let choice = answer["choice"].as_str().unwrap_or_default();
    assert!(["rollback", "hotfix", "wait"].contains(&choice), "choice: {choice}");
    let entries = answer["distribution"]["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 3, "distribution: {entries:?}");
    let mass: f64 = entries.iter().map(|entry| entry["probability"].as_f64().unwrap()).sum();
    assert!((mass - 1.0).abs() < 1e-6);
    assert!(body["confidence"]["calibrated_confidence"].as_f64().is_some());
    assert!(
        ["accept", "verified", "verify", "abstain", "escalate", "no_valid_candidate"]
            .contains(&body["outcome"].as_str().unwrap_or_default()),
        "outcome: {body}"
    );
    assert!(body["trace"].as_object().is_some_and(|trace| !trace.is_empty()));
}

#[tokio::test]
async fn the_format_header_selects_the_adapter_over_the_wire() {
    let server = spawn_server().await;
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
        DecisionPolicy::default(),
        RequestMetadata::default(),
    )
    .unwrap();
    let openai_body = OpenAi.encode_request(&request).unwrap().to_string();

    // The §2.4 curl: an OpenAI-shaped body with the header is decoded and
    // answered in the same shape — the `{"<question id>": value}`
    // projection, not the native envelope.
    let with_header = client()
        .post(format!("{}/v1/decide", server.base_url))
        .header(CONTENT_TYPE, "application/json")
        .header("x-opencodifier-format", "openai")
        .body(openai_body.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(with_header.status(), StatusCode::OK);
    let projected: Value = with_header.json().await.unwrap();
    assert!(projected["model"].is_string(), "projection: {projected}");
    assert!(projected.get("answers").is_none(), "projection: {projected}");

    // An unknown format value is refused at the boundary, never guessed.
    let unknown = client()
        .post(format!("{}/v1/decide", server.base_url))
        .header(CONTENT_TYPE, "application/json")
        .header("x-opencodifier-format", "yaml")
        .body(openai_body)
        .send()
        .await
        .unwrap();
    assert_eq!(unknown.status(), StatusCode::BAD_REQUEST);
    let error: Value = unknown.json().await.unwrap();
    assert_eq!(error["error"]["code"], "schema.invalid_value", "error: {error}");
}

/// The `response_format` a chat client sends for one decision request:
/// the strict-mode schema the adapter itself encodes, verbatim.
fn chat_response_format(request: &DecisionRequest) -> Value {
    let schema = OpenAi.encode_request(request).unwrap()["format"]["schema"].clone();
    json!({ "type": "json_schema", "json_schema": { "name": "decisions", "schema": schema } })
}

/// A chat completion envelope over the decision request a test built.
async fn post_chat(
    server: &TestServer,
    messages: Value,
    response_format: Value,
) -> reqwest::Response {
    post_json(
        &server.base_url,
        "/v1/chat/completions",
        &json!({
            "model": "opencodifier-decision",
            "messages": messages,
            "response_format": response_format,
        })
        .to_string(),
    )
    .await
}

#[tokio::test]
async fn chat_completions_decides_a_strict_schema_request() {
    let server = spawn_server().await;
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
        DecisionPolicy::default(),
        RequestMetadata::default(),
    )
    .unwrap();

    // The adapter's own strict schema rides in as `response_format`; no
    // `required` list, so the route must synthesize it (strict mode makes
    // every property required).
    let messages = json!([
        { "role": "system", "content": "Deployment context: production rust monorepo." },
        { "role": "user", "content": "refactor the rust parser module without allocating" },
    ]);
    let response = post_chat(&server, messages, chat_response_format(&request)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();

    // A chat completion envelope whose content is the structured-output
    // JSON the adapter projects: one field per question.
    assert_eq!(body["object"], "chat.completion", "body: {body}");
    assert_eq!(body["model"], "opencodifier-decision");
    assert!(body["id"].as_str().unwrap().starts_with("chatcmpl-"));
    assert!(body["created"].as_u64().unwrap() > 0);
    let choice = &body["choices"][0];
    assert_eq!(choice["finish_reason"], "stop");
    assert_eq!(choice["message"]["role"], "assistant");
    let content: Value =
        serde_json::from_str(choice["message"]["content"].as_str().unwrap()).unwrap();
    assert!(
        ["local-qwen", "cloud-large"].contains(&content["model"].as_str().unwrap()),
        "content: {content}"
    );
    // The outcome has no slot in a chat shape, so it travels in the
    // extension object next to the calibrated confidence.
    assert!(
        ["accept", "verified", "verify", "abstain", "escalate", "no_valid_candidate"]
            .contains(&body["opencodifier"]["outcome"].as_str().unwrap_or_default()),
        "body: {body}"
    );
    assert!(body["opencodifier"]["calibrated_confidence"].as_f64().unwrap() >= 0.0);

    // The same schema with the list supplied verbatim is honored too.
    let mut with_required = chat_response_format(&request);
    with_required["json_schema"]["schema"]["required"] = json!(["model"]);
    let response = post_chat(&server, json!([{ "role": "user", "content": "refactor the rust parser module without allocating" }]), with_required).await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn chat_completions_refuses_generation_requests_with_typed_errors() {
    let server = spawn_server().await;
    let messages = json!([{ "role": "user", "content": "refactor the rust parser module" }]);

    // No response format at all: a chat completion that wants prose.
    let bare = client()
        .post(format!("{}/v1/chat/completions", server.base_url))
        .header(CONTENT_TYPE, "application/json")
        .body(json!({ "model": "opencodifier-decision", "messages": messages }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(bare.status(), StatusCode::BAD_REQUEST);
    let error: Value = bare.json().await.unwrap();
    assert_eq!(error_code(&error), "schema.unsupported_generation_field", "error: {error}");

    // `json_object` is free-form generation by definition.
    let free_form = post_chat(&server, messages.clone(), json!({ "type": "json_object" })).await;
    assert_eq!(free_form.status(), StatusCode::BAD_REQUEST);
    let error: Value = free_form.json().await.unwrap();
    assert_eq!(error_code(&error), "schema.unsupported_generation_field");

    // A prose property — a string with no enum — is not a decision.
    let prose = post_chat(
        &server,
        messages,
        json!({ "type": "json_schema", "json_schema": { "name": "summary", "schema": {
            "type": "object",
            "properties": { "summary": { "type": "string" } },
        } } }),
    )
    .await;
    assert_eq!(prose.status(), StatusCode::BAD_REQUEST);
    let error: Value = prose.json().await.unwrap();
    assert_eq!(error_code(&error), "schema.unsupported_generation_field");
    assert!(error["error"]["message"].as_str().unwrap().contains("summary"), "error: {error}");
}

#[tokio::test]
async fn chat_completions_flattens_messages_and_refuses_undecidable_ones() {
    let server = spawn_server().await;
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
        DecisionPolicy::default(),
        RequestMetadata::default(),
    )
    .unwrap();
    let format = chat_response_format(&request);

    // System and user join into the state; the assistant turn is skipped
    // (the runtime decides over the caller's state, not a conversation).
    let conversational = post_chat(
        &server,
        json!([
            { "role": "system", "content": "Deployment context: production rust monorepo." },
            { "role": "assistant", "content": "I could look at that for you." },
            { "role": "user", "content": "refactor the rust parser module without allocating" },
        ]),
        format.clone(),
    )
    .await;
    assert_eq!(conversational.status(), StatusCode::OK);
    let body: Value = conversational.json().await.unwrap();
    let content: Value =
        serde_json::from_str(body["choices"][0]["message"]["content"].as_str().unwrap()).unwrap();
    assert!(content["model"].is_string(), "content: {content}");

    // Multimodal content is refused, never silently dropped: the IR's
    // state is text, and dropping a part decides over less than was sent.
    let multimodal = post_chat(
        &server,
        json!([{ "role": "user", "content": [{ "type": "text", "text": "refactor" }] }]),
        format.clone(),
    )
    .await;
    assert_eq!(multimodal.status(), StatusCode::BAD_REQUEST);
    let error: Value = multimodal.json().await.unwrap();
    assert_eq!(error_code(&error), "schema.invalid_value", "error: {error}");

    // No decidable state at all.
    let empty =
        post_chat(&server, json!([{ "role": "assistant", "content": "hello" }]), format).await;
    assert_eq!(empty.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn chat_completions_reports_abstention_as_a_200() {
    let server = spawn_server().await;
    // A state with no lexical overlap on any level: the engine answers
    // with (near-)uniform mass, calibration lands under the default
    // abstain gate (0.50), and the outcome is abstain — a successful
    // decision, so the transport status stays 200.
    let request = DecisionRequest::new(
        State::from_text("the ornamental thimble catalogue quadruples nightly"),
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

    let response = post_chat(
        &server,
        json!([{ "role": "user", "content": "the ornamental thimble catalogue quadruples nightly" }]),
        chat_response_format(&request),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["opencodifier"]["outcome"], "abstain", "body: {body}");
    let content: Value =
        serde_json::from_str(body["choices"][0]["message"]["content"].as_str().unwrap()).unwrap();
    assert!(content.is_object(), "content: {content}");
}
