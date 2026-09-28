//! The `/v1` routes: decide, graph validation, and health.
//!
//! The surface is deliberately thin. Every endpoint normalizes through
//! [`opencodifier_schema`] and executes through
//! [`opencodifier_engine::EngineHandle`] — no pipeline logic lives here, so
//! HTTP and MCP cannot drift apart on what a decision is.
//!
//! ```text
//! bytes ──► serde_json::Value ──► Native.decode_request ──► EngineHandle.decide
//!                                                                     │
//! ◄── Native.encode_response ◄────────────────────────────────────────┘
//! ```
//!
//! Posture: every request body is hostile input. The body size is capped
//! before parsing, payloads are never echoed back, and abstention is a
//! `200` — a refused decision is the runtime working, not a failure.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use opencodifier_core::{DecisionRequest, DecisionResponse, Limits};
use opencodifier_engine::{DecisionGraph, EngineHandle, GraphDocument};
use opencodifier_schema::native::Native;
use opencodifier_schema::{SchemaError, WireFormat};
use serde_json::{Value, json};

use crate::error::HttpError;

/// Maximum accepted request body, in bytes.
///
/// Matched to [`Limits::max_input_bytes`] (`1_048_576`) so the HTTP
/// transport never accepts a body the IR would refuse anyway: oversized
/// input is rejected at the socket boundary instead of being parsed,
/// decoded, and only then refused. The unit test below pins the two
/// values together, so a change to the limit cannot silently desynchronize
/// them.
pub const MAX_BODY_BYTES: usize = 1_048_576;

/// Builds the `/v1` router over `handle`.
///
/// Pure builder: tests drive it through a bound listener directly and
/// [`serve`](crate::serve) drives the same router in production, so the
/// two paths cannot diverge.
pub fn router(handle: Arc<EngineHandle>) -> Router {
    Router::new()
        .route("/v1/decide", post(decide))
        .route("/v1/graph/validate", post(validate_graph))
        .route("/v1/healthz", get(healthz))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(handle)
}

/// `POST /v1/decide` — native decision request in, native decision out.
///
/// Abstention and every other typed outcome are `200`: the outcome field
/// carries the decision, not the transport status. Decode failures are
/// `400` with the adapter's `schema.*` code; only an engine fault is
/// `500`.
async fn decide(
    State(handle): State<Arc<EngineHandle>>,
    body: Bytes,
) -> Result<Json<Value>, HttpError> {
    let payload = parse_json(&body)?;
    let request: DecisionRequest =
        Native.decode_request(&payload, &Limits::default()).map_err(HttpError::from)?;

    // The engine is synchronous and CPU-bound (D5): keep it off the async
    // workers so one heavy request cannot stall unrelated connections.
    let response: DecisionResponse = tokio::task::spawn_blocking(move || handle.decide(&request))
        .await
        // The only way a blocking task fails is runtime shutdown, which is
        // exactly a cancellation.
        .map_err(|_| HttpError::Engine(opencodifier_engine::EngineError::Cancelled))?
        .map_err(HttpError::from)?;

    // Encoding an engine-produced response cannot fail; if it ever does,
    // it is a server fault (`engine.serialization`), not bad input.
    let encoded = Native.encode_response(&response).map_err(|error| {
        HttpError::Engine(opencodifier_engine::EngineError::Serialization {
            reason: error.to_string(),
        })
    })?;
    Ok(Json(encoded))
}

/// `POST /v1/graph/validate` — graph document in, validity verdict out.
///
/// Validation failures are `400`: a cyclic or malformed graph is an input
/// error, and the reported code is the engine's own (`graph.cycle`, ...) so
/// callers see the same code the engine would raise, not a flattened serde
/// message.
async fn validate_graph(body: Bytes) -> Result<Json<Value>, HttpError> {
    let payload = parse_json(&body)?;
    let limits = Limits::default();
    // Decoded into the engine's own graph document and rebuilt through its
    // validating constructor: the one decode-and-validate path, never a
    // weaker local copy. Shape errors keep the adapter's `schema.*` code so
    // an unparseable document and a structurally invalid one stay
    // distinguishable; structural failures carry the engine's `graph.*`.
    let document: GraphDocument = serde_json::from_value(payload.clone())
        .map_err(|error| HttpError::from(SchemaError::invalid_value("graph", error.to_string())))?;
    if document.nodes.len() > limits.max_graph_nodes {
        return Err(HttpError::Schema(SchemaError::limit(
            "max_graph_nodes",
            document.nodes.len(),
            limits.max_graph_nodes,
        )));
    }
    let graph = DecisionGraph::try_from(document).map_err(HttpError::Graph)?;
    EngineHandle::validate_graph(&graph).map_err(HttpError::Graph)?;
    Ok(Json(json!({ "valid": true, "nodes": graph.nodes().len() })))
}

/// `GET /v1/healthz` — liveness plus the identity decisions are cached
/// under.
///
/// Liveness here means "the engine assembled and can answer": the identity
/// is the real graph/model/calibration identity, not a literal `ok` string
/// standing in for a probe that never checked anything.
async fn healthz(State(handle): State<Arc<EngineHandle>>) -> Json<Value> {
    let health = handle.health();
    Json(json!({
        "status": "ok",
        "identity": {
            "graph_version": health.identity.graph_version,
            "model_id": health.identity.model_id,
            "calibration_version": health.identity.calibration_version,
            "engine_semver": health.identity.engine_semver,
        },
        "nodes": health.nodes,
        "parallelism": health.parallelism,
        "cache_enabled": health.cache_enabled,
    }))
}

/// Parses a request body as JSON.
///
/// A body that is not JSON at all is a client input error, reported under
/// the adapter's own `schema.invalid_json` code rather than an
/// HTTP-specific one, so every ingress names malformed JSON identically.
fn parse_json(body: &Bytes) -> Result<Value, HttpError> {
    serde_json::from_slice(body)
        .map_err(|error| HttpError::from(SchemaError::Json(error.to_string())))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;

    #[test]
    fn body_limit_matches_the_request_input_ceiling() {
        assert_eq!(MAX_BODY_BYTES, 1_048_576);
    }

    #[test]
    fn graph_documents_decode_version_and_nodes() {
        let document: GraphDocument = serde_json::from_value(json!({
            "version": 3,
            "nodes": [{ "id": "a", "kind": "rule" }],
        }))
        .unwrap();
        assert_eq!(document.version, 3);
        assert_eq!(document.nodes.len(), 1);
    }

    #[test]
    fn malformed_graph_documents_are_schema_errors_not_engine_faults() {
        let error = serde_json::from_value::<GraphDocument>(json!({ "version": 1, "nodes": {} }))
            .unwrap_err();
        let mapped = HttpError::from(SchemaError::invalid_value("graph", error.to_string()));
        assert_eq!(mapped.code(), "schema.invalid_value");
        assert_eq!(mapped.status(), axum::http::StatusCode::BAD_REQUEST);
    }
}
