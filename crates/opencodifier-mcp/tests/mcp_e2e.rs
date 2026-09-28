//! End-to-end MCP sessions: a real rmcp client drives the real server over
//! an in-process duplex transport (PLANNING.md §55).
//!
//! Nothing here reaches into the server's internals — every assertion is
//! made from the client side of the wire, exactly as an MCP host would see
//! the runtime: the handshake, the tool list, tool calls, and the
//! tool-level error envelope.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::default_trait_access,
    unused_must_use
)]

use std::sync::Arc;

use opencodifier_engine::{EngineConfig, EngineHandle};
use opencodifier_mcp::OpenCodifierServer;
use rmcp::{
    ClientHandler, ServiceExt,
    model::{CallToolRequestParams, ClientInfo},
};
use serde_json::{Value, json};

/// A client that takes part in the handshake and nothing else.
#[derive(Debug, Clone, Default)]
struct TestClient;

impl ClientHandler for TestClient {
    fn get_info(&self) -> ClientInfo {
        ClientInfo::default()
    }
}

/// Spawns the server on one end of a duplex pipe and connects a client to
/// the other. Returns the connected client and the server's join handle.
async fn session()
-> (rmcp::service::RunningService<rmcp::RoleClient, TestClient>, tokio::task::JoinHandle<()>) {
    let (server_transport, client_transport) = tokio::io::duplex(64 * 1024);
    let handle = EngineHandle::lexical(EngineConfig::with_default_pipeline().unwrap()).unwrap();
    let server = OpenCodifierServer::new(Arc::new(handle));
    let server_task = tokio::spawn(async move {
        let running = server.serve(server_transport).await.unwrap();
        running.waiting().await.unwrap();
    });
    let client = TestClient.serve(client_transport).await.unwrap();
    (client, server_task)
}

/// The structured content of a tool result, failing the test with the raw
/// text when the server sent none.
fn structured(result: &rmcp::model::CallToolResult) -> Value {
    result.structured_content.clone().unwrap_or_else(|| {
        panic!(
            "no structured content; text was {:?}",
            result.content.first().map(|c| c.as_text().map(|t| t.text.clone()))
        )
    })
}

/// The `error.code` of a tool-level failure result.
fn error_code(result: &rmcp::model::CallToolResult) -> String {
    assert_eq!(result.is_error, Some(true), "expected a tool-level error");
    structured(result)["error"]["code"].as_str().unwrap().to_owned()
}

/// One canonical choice request the default pipeline can decide.
fn choice_document() -> Value {
    json!({
        "state": { "text": "complex reasoning over a large proof", "facts": {} },
        "questions": [{
            "type": "choice",
            "id": "model",
            "text": "Which model should serve this request?",
            "candidates": [
                { "id": "local-qwen", "description": "General coding and reasoning" },
                { "id": "local-glm", "description": "Complex reasoning" }
            ],
        }],
        "policy": { "min_confidence": 0.8, "verify_below": 0.65, "abstain_below": 0.5, "risk": "low" },
        "metadata": { "request_id": "mcp-e2e-0001", "limits": { "max_input_bytes": 1_048_576, "max_questions": 32, "max_candidates": 256, "max_graph_nodes": 128, "max_execution_time": { "secs": 10, "nanos": 0 }, "max_retrieval_results": 64 } },
    })
}

async fn call(
    client: &rmcp::service::RunningService<rmcp::RoleClient, TestClient>,
    name: &str,
    arguments: Value,
) -> rmcp::model::CallToolResult {
    client
        .call_tool(
            CallToolRequestParams::new(name.to_owned())
                .with_arguments(arguments.as_object().unwrap().clone()),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn the_handshake_announces_the_runtime() {
    let (client, server_task) = session().await;
    let info = client.peer_info().expect("the server completed the handshake");
    assert_eq!(info.server_info.name, "opencodifier");
    assert!(!info.instructions.clone().unwrap_or_default().is_empty());
    assert!(info.capabilities.tools.is_some(), "tools capability must be declared");
    client.cancel().await;
    server_task.await.unwrap();
}

#[tokio::test]
async fn the_tool_list_is_exactly_the_phase_15_set() {
    let (client, server_task) = session().await;
    let listed = client.list_tools(Default::default()).await.unwrap();
    let mut names: Vec<String> = listed.tools.iter().map(|tool| tool.name.to_string()).collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "codify_batch",
            "codify_decide",
            "codify_explain",
            "codify_graph",
            "codify_validate",
            "codify_verify",
        ]
    );
    // Every tool advertises a JSON schema for its arguments: an MCP host
    // can build a call without reading this repo.
    for tool in &listed.tools {
        assert!(!tool.input_schema.is_empty(), "{} has no input schema", tool.name);
    }
    client.cancel().await;
    server_task.await.unwrap();
}

#[tokio::test]
async fn codify_decide_returns_the_canonical_response() {
    let (client, server_task) = session().await;
    let result = call(&client, "codify_decide", json!({ "request": choice_document() })).await;
    assert_eq!(result.is_error, Some(false));
    let response = structured(&result);
    assert_eq!(response["answers"].as_array().unwrap().len(), 1);
    assert_eq!(response["answers"][0]["question_id"], "model");
    assert_eq!(response["answers"][0]["type"], "choice");
    assert!(response["confidence"]["calibrated_confidence"].is_number());
    assert!(!response["trace"]["entries"].as_array().unwrap().is_empty());
    // The trace names the deciding model — the same identity healthz and
    // codify_graph report, never a free-text excuse.
    let choice_entry = response["trace"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["node"] == "choice")
        .unwrap();
    assert!(choice_entry["detail"]["model"]["value"].as_str().unwrap().contains("lexical"));
    client.cancel().await;
    server_task.await.unwrap();
}

#[tokio::test]
async fn a_hostile_request_is_a_typed_tool_error() {
    let (client, server_task) = session().await;
    let result =
        call(&client, "codify_decide", json!({ "request": { "questions": "drop table" } })).await;
    assert_eq!(result.is_error, Some(true));
    let error = structured(&result);
    assert!(error["error"]["code"].as_str().unwrap().starts_with("schema."));
    // The typed message names the refusing construct; the request is not
    // echoed back as a document.
    let rendered = error.to_string();
    assert!(!rendered.contains("\"state\""));
    client.cancel().await;
    server_task.await.unwrap();
}

#[tokio::test]
async fn codify_batch_answers_independently_and_in_order() {
    let (client, server_task) = session().await;
    let requests = json!({
        "requests": [
            choice_document(),
            { "state": 1, "questions": [] },
            choice_document(),
        ]
    });
    let result = call(&client, "codify_batch", requests).await;
    assert_eq!(result.is_error, Some(false));
    let batch = structured(&result);
    assert_eq!(batch["accepted"], 2);
    assert_eq!(batch["refused"], 1);
    let items = batch["results"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["ok"], true);
    assert_eq!(items[1]["ok"], false);
    assert!(items[1]["error"]["code"].as_str().unwrap().starts_with("schema."));
    assert_eq!(items[2]["ok"], true);
    // In order: the first and third items are the same request, so the
    // same answers.
    assert_eq!(items[0]["response"]["answers"], items[2]["response"]["answers"]);
    client.cancel().await;
    server_task.await.unwrap();
}

#[tokio::test]
async fn codify_batch_refuses_oversize_up_front() {
    let (client, server_task) = session().await;
    let requests = json!({ "requests": (0..=opencodifier_mcp::MAX_BATCH).map(|_| choice_document()).collect::<Vec<_>>() });
    let result = call(&client, "codify_batch", requests).await;
    assert_eq!(error_code(&result), "mcp.batch_too_large");
    client.cancel().await;
    server_task.await.unwrap();
}

#[tokio::test]
async fn codify_graph_reports_the_pipeline_actually_running() {
    let (client, server_task) = session().await;
    let result = call(&client, "codify_graph", json!({})).await;
    assert_eq!(result.is_error, Some(false));
    let document = structured(&result);
    assert_eq!(document["status"], "ok");
    let nodes = document["graph"]["nodes"].as_array().unwrap();
    assert_eq!(document["nodes"], nodes.len());
    assert!(nodes.iter().any(|node| node["kind"] == "choice"), "the pipeline decides");
    assert!(nodes.iter().any(|node| node["kind"] == "threshold"), "and gates");
    client.cancel().await;
    server_task.await.unwrap();
}

#[tokio::test]
async fn codify_validate_accepts_good_graphs_and_names_bad_ones() {
    let (client, server_task) = session().await;
    let valid = json!({
        "graph": {
            "version": 1,
            "nodes": [
                { "id": "normalize", "kind": "normalize" },
                { "id": "choice", "kind": "choice", "depends_on": ["normalize"] },
                { "id": "threshold", "kind": "threshold", "depends_on": ["choice"], "threshold": 0.5 },
                { "id": "output", "kind": "output", "depends_on": ["threshold"] },
            ],
        }
    });
    let result = call(&client, "codify_validate", valid).await;
    assert_eq!(result.is_error, Some(false));
    let verdict = structured(&result);
    assert_eq!(verdict["valid"], true);
    assert_eq!(verdict["nodes"], 4);

    let cyclic = json!({
        "graph": {
            "version": 1,
            "nodes": [
                { "id": "a", "kind": "normalize", "depends_on": ["b"] },
                { "id": "b", "kind": "filter", "depends_on": ["a"] },
            ],
        }
    });
    let result = call(&client, "codify_validate", cyclic).await;
    assert_eq!(error_code(&result), "graph.cycle");
    client.cancel().await;
    server_task.await.unwrap();
}

#[tokio::test]
async fn codify_verify_reports_the_confidence_gate_verdict() {
    let (client, server_task) = session().await;
    let result = call(&client, "codify_verify", json!({ "request": choice_document() })).await;
    assert_eq!(result.is_error, Some(false));
    let verdict = structured(&result);
    assert!(verdict["decisive"].is_boolean());
    for key in [
        "top_probability",
        "margin",
        "entropy",
        "ood_score",
        "verifier_agreement",
        "calibrated_confidence",
    ] {
        assert!(verdict["confidence"].get(key).is_some(), "confidence lacks `{key}`");
    }
    assert_eq!(verdict["response"]["outcome"], verdict["outcome"]);
    client.cancel().await;
    server_task.await.unwrap();
}

#[tokio::test]
async fn codify_explain_is_the_deterministic_trace_never_generated_reasoning() {
    let (client, server_task) = session().await;
    let result = call(&client, "codify_explain", json!({ "request": choice_document() })).await;
    assert_eq!(result.is_error, Some(false));
    let explanation = structured(&result);
    let keys: Vec<&str> = explanation.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["execution", "response"]);
    assert!(!explanation["response"]["trace"]["entries"].as_array().unwrap().is_empty());
    let waves = explanation["execution"]["waves"].as_array().unwrap();
    assert!(!waves.is_empty(), "the executor ran in waves");
    // The explanation is the pipeline's own record. There is no
    // chain-of-thought field, because there is no chain-of-thought.
    let rendered = explanation.to_string().to_lowercase();
    assert!(!rendered.contains("chain_of_thought"));
    assert!(!rendered.contains("reasoning"));
    client.cancel().await;
    server_task.await.unwrap();
}

#[tokio::test]
async fn abstention_is_a_successful_result_not_an_error() {
    let (client, server_task) = session().await;
    let mut document = choice_document();
    document["policy"] = json!({
        "min_confidence": 1.0, "verify_below": 1.0, "abstain_below": 1.0, "risk": "low",
    });
    let result = call(&client, "codify_decide", json!({ "request": document })).await;
    assert_eq!(result.is_error, Some(false), "a refused decision is the runtime working");
    assert_eq!(structured(&result)["outcome"], "abstain");
    client.cancel().await;
    server_task.await.unwrap();
}

#[tokio::test]
async fn an_unknown_tool_is_a_protocol_error_not_a_tool_result() {
    let (client, server_task) = session().await;
    let error = client
        .call_tool(
            CallToolRequestParams::new("codify_prose").with_arguments(serde_json::Map::new()),
        )
        .await
        .unwrap_err();
    let rmcp::service::ServiceError::McpError(mcp_error) = error else {
        panic!("an unknown tool is an MCP-level error, got: {error}");
    };
    assert_eq!(mcp_error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    client.cancel().await;
    server_task.await.unwrap();
}
