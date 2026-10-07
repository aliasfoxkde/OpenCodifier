//! Refusal arms of the `/v1` surface: the malformed-request paths the
//! happy-path round trips never reach.
//!
//! Same harness as `http_e2e.rs` — a real engine, a real listener, an
//! ephemeral port — but every test here posts something the surface must
//! refuse, and asserts the refusal is typed (`schema.*`, status 400)
//! rather than a 500 or a silent success.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

use std::sync::Arc;

use opencodifier_core::{
    BooleanQuestion, DecisionPolicy, DecisionQuestion, DecisionRequest, RequestMetadata, State,
};
use opencodifier_engine::{EngineConfig, EngineHandle};
use opencodifier_schema::WireFormat;
use opencodifier_schema::native::Native;
use opencodifier_schema::openai::OpenAi;
use reqwest::header::CONTENT_TYPE;
use serde_json::{Value, json};

/// The server under test: a real engine, a real listener, an ephemeral port.
struct TestServer {
    base_url: String,
}

/// Spawns the production router on its own listener and reports the URL.
async fn spawn_server() -> TestServer {
    let handle =
        Arc::new(EngineHandle::lexical(EngineConfig::with_default_pipeline().unwrap()).unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = opencodifier_http::router(handle);
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    TestServer { base_url: format!("http://{addr}") }
}

/// Posts a raw body with a JSON content type, as a hostile client would.
async fn post_json(server: &TestServer, path: &str, body: &str) -> reqwest::Response {
    reqwest::Client::builder()
        .build()
        .unwrap()
        .post(format!("{}{path}", server.base_url))
        .header(CONTENT_TYPE, "application/json")
        .body(body.to_owned())
        .send()
        .await
        .unwrap()
}

/// Posts a raw body with an explicitly built header value, so a test can
/// send bytes `&str` headers cannot carry.
async fn post_with_header(
    server: &TestServer,
    path: &str,
    name: &str,
    value: &reqwest::header::HeaderValue,
    body: &str,
) -> reqwest::Response {
    reqwest::Client::builder()
        .build()
        .unwrap()
        .post(format!("{}{path}", server.base_url))
        .header(CONTENT_TYPE, "application/json")
        .header(name, value.clone())
        .body(body.to_owned())
        .send()
        .await
        .unwrap()
}

/// The `error.code` of a refusal envelope, failing loudly if absent.
fn error_code(response: &Value) -> &str {
    response["error"]["code"].as_str().unwrap()
}

/// A canonical one-question request, encoded for the chat route's
/// `response_format`: the adapter's own strict schema, verbatim.
fn strict_response_format() -> Value {
    let request = DecisionRequest::new(
        State::from_text("run the toolchain"),
        vec![DecisionQuestion::Boolean(
            BooleanQuestion::new("rollback", "Roll back now?").unwrap(),
        )],
        DecisionPolicy::default(),
        RequestMetadata::default(),
    )
    .unwrap();
    let schema = OpenAi.encode_request(&request).unwrap()["format"]["schema"].clone();
    json!({ "type": "json_schema", "json_schema": { "name": "decisions", "schema": schema } })
}

/// A chat envelope over `response_format`, with the model field the caller
/// supplied (or none at all, to pin the runtime's own default naming).
fn chat_body(model: Option<&str>, response_format: &Value) -> String {
    let mut envelope = json!({
        "messages": [{ "role": "user", "content": "run the toolchain" }],
        "response_format": response_format,
    });
    if let Some(model) = model {
        envelope["model"] = json!(model);
    }
    envelope.to_string()
}

/// D30: a format header carrying bytes that are not ASCII (obs-text, which
/// the HTTP grammar permits in a header value) is refused before the body
/// is even parsed, under the adapter's `schema.invalid_value` code.
#[tokio::test]
async fn a_non_ascii_format_header_is_refused_before_the_body_is_parsed() {
    let server = spawn_server().await;
    let obs_text = reqwest::header::HeaderValue::from_bytes(&[0xC3, 0x28]).unwrap();
    let response =
        post_with_header(&server, "/v1/decide", "x-opencodifier-format", &obs_text, "{\"state\": ")
            .await;
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.unwrap();
    assert_eq!(error_code(&body), "schema.invalid_value", "body: {body}");
    // The refusal names the header and why, not the bytes: the payload is
    // never echoed back.
    let message = body["error"]["message"].as_str().unwrap();
    assert!(message.contains("x-opencodifier-format"), "message: {message}");
    assert!(message.contains("ASCII"), "message: {message}");
}

/// A chat completion whose body is valid JSON but not an object has no
/// slot for `messages` or `response_format`, so it is refused with the
/// adapter's `schema.invalid_type` code naming what was expected.
#[tokio::test]
async fn a_chat_body_that_is_not_a_json_object_is_a_schema_refusal() {
    let server = spawn_server().await;
    for body in ["[1, 2, 3]", "\"decide for me\"", "null"] {
        let response = post_json(&server, "/v1/chat/completions", body).await;
        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST, "body: {body}");
        let error: Value = response.json().await.unwrap();
        assert_eq!(error_code(&error), "schema.invalid_type", "body: {body}");
        let message = error["error"]["message"].as_str().unwrap();
        assert!(message.contains("body"), "message: {message}");
        assert!(message.contains("a JSON object"), "message: {message}");
    }
}

/// `response_format.type == "json_schema"` with a schema that is not an
/// object-with-properties asks for a shape the runtime cannot decide over:
/// refused, never guessed at.
#[tokio::test]
async fn a_strict_response_format_without_an_object_schema_is_refused() {
    let server = spawn_server().await;
    let cases = [
        // No `properties` at all.
        json!({ "type": "json_schema", "json_schema": { "schema": { "type": "object" } } }),
        // `properties` present but not an object.
        json!({ "type": "json_schema", "json_schema": { "schema": { "properties": [1, 2] } } }),
        // No `schema` inside `json_schema`.
        json!({ "type": "json_schema", "json_schema": { "name": "decisions" } }),
    ];
    for response_format in cases {
        let response =
            post_json(&server, "/v1/chat/completions", &chat_body(Some("m"), &response_format))
                .await;
        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        let error: Value = response.json().await.unwrap();
        assert_eq!(error_code(&error), "schema.invalid_value", "format: {error}");
        let message = error["error"]["message"].as_str().unwrap();
        assert!(message.contains("response_format.json_schema.schema"), "message: {message}");
        assert!(message.contains("properties"), "the refusal must say what was missing: {message}");
    }
}

/// An absent `model` field is not an error: the runtime answers under its
/// own name rather than inventing a model lane it does not run.
#[tokio::test]
async fn a_chat_request_without_a_model_reports_the_runtime_name() {
    let server = spawn_server().await;
    let response =
        post_json(&server, "/v1/chat/completions", &chat_body(None, &strict_response_format()))
            .await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["model"], "opencodifier", "body: {body}");
    // The answer itself still names the question it decided.
    let content: Value =
        serde_json::from_str(body["choices"][0]["message"]["content"].as_str().unwrap()).unwrap();
    assert!(content["rollback"].is_boolean(), "content: {content}");
    // The canonical decode path is untouched by the envelope: the same
    // request decided natively answers the same question id.
    let request = DecisionRequest::new(
        State::from_text("run the toolchain"),
        vec![DecisionQuestion::Boolean(
            BooleanQuestion::new("rollback", "Roll back now?").unwrap(),
        )],
        DecisionPolicy::default(),
        RequestMetadata::default(),
    )
    .unwrap();
    assert_eq!(Native.encode_request(&request).unwrap()["questions"][0]["id"], "rollback");
}
