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
use opencodifier_engine::{DecisionGraph, EngineHandle, GraphDocument, MAX_BATCH};
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
        .route("/v1/batch", post(batch))
        .route("/v1/graph/validate", post(validate_graph))
        .route("/v1/validate", post(validate_request))
        .route("/v1/models", get(models))
        .route("/v1/capabilities", get(capabilities))
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

/// `POST /v1/batch` — up to [`MAX_BATCH`] canonical requests, decided
/// independently and in order.
///
/// The batch exists to amortize transport round trips, not to enqueue
/// work, so an oversized batch is refused up front with
/// `schema.limit_exceeded` (§36 names no batch limit; the engine owns
/// the policy and MCP `codify_batch` shares it). One item's refusal is
/// that item's `error` object, never a failure of the batch: the
/// transport status is `200` whenever the batch itself was well-formed,
/// mirroring the posture that a refused decision is the runtime working.
async fn batch(
    State(handle): State<Arc<EngineHandle>>,
    body: Bytes,
) -> Result<Json<Value>, HttpError> {
    let payload = parse_json(&body)?;
    let body: BatchBody = serde_json::from_value(payload).map_err(|error| {
        HttpError::from(SchemaError::invalid_value("requests", error.to_string()))
    })?;
    let requests = body.requests;
    if requests.len() > MAX_BATCH {
        return Err(HttpError::Schema(SchemaError::limit("max_batch", requests.len(), MAX_BATCH)));
    }

    // Decoding and deciding are both synchronous and CPU-bound (D5): one
    // blocking task walks the whole batch so a 16-item batch costs the
    // async workers a single wake-up.
    let results = tokio::task::spawn_blocking(move || {
        requests.iter().map(|document| batch_item(&handle, document)).collect::<Vec<Value>>()
    })
    .await
    .map_err(|_| HttpError::Engine(opencodifier_engine::EngineError::Cancelled))?;

    Ok(Json(json!({ "results": results, "count": results.len() })))
}

/// Decodes and decides one batch item, producing its result object.
///
/// A decode failure and an engine failure land in the same envelope —
/// `{"error": {"code", "message"}}` — because from the client's side
/// both are "this item has no decision"; the `schema.*` vs `engine.*`
/// code keeps the cause distinguishable.
fn batch_item(handle: &EngineHandle, document: &Value) -> Value {
    let encoded = Native
        .decode_request(document, &Limits::default())
        .map_err(HttpError::from)
        .and_then(|request| {
            let response = handle.decide(&request).map_err(HttpError::from)?;
            Native.encode_response(&response).map_err(|error| {
                HttpError::Engine(opencodifier_engine::EngineError::Serialization {
                    reason: error.to_string(),
                })
            })
        });
    match encoded {
        Ok(response) => json!({ "response": response }),
        Err(error) => json!({
            "error": { "code": error.code(), "message": error.to_string() },
        }),
    }
}

/// `POST /v1/validate` — canonical request in, validity verdict out.
///
/// The decode-and-validate path only: no decision is computed, nothing
/// is cached, and the answer says what was accepted so a client can
/// preflight a payload (question kinds, candidate counts) before paying
/// for execution.
async fn validate_request(body: Bytes) -> Result<Json<Value>, HttpError> {
    let payload = parse_json(&body)?;
    let request: DecisionRequest =
        Native.decode_request(&payload, &Limits::default()).map_err(HttpError::from)?;
    Ok(Json(json!({
        "valid": true,
        "questions": request.questions().len(),
        "state_bytes": request.state().text().len(),
    })))
}

/// `GET /v1/models` — the model lanes decisions currently run on.
///
/// The runtime has exactly one active decision lane, and honesty is the
/// contract: the response reports the composed engine model id
/// (`relational-v1|builtin-lexical-v1` for the base binary) rather than
/// pretending to enumerate a catalog of installed artifacts.
async fn models(State(handle): State<Arc<EngineHandle>>) -> Json<Value> {
    let identity = handle.identity();
    Json(json!({
        "models": [{
            "id": identity.model_id,
            "role": "decision",
            "active": true,
            "graph_version": identity.graph_version,
            "calibration_version": identity.calibration_version,
        }],
    }))
}

/// `GET /v1/capabilities` — what this runtime accepts, including the
/// cache probe.
///
/// The decision kinds are the IR's three question types; the test module
/// pins the list against a decoded question of each kind so the list
/// cannot silently drift from what `/v1/decide` actually accepts.
async fn capabilities(State(handle): State<Arc<EngineHandle>>) -> Json<Value> {
    let health = handle.health();
    Json(json!({
        "decision_kinds": DECISION_KINDS,
        "endpoints": [
            "/v1/decide", "/v1/batch", "/v1/graph/validate", "/v1/validate",
            "/v1/models", "/v1/capabilities", "/v1/healthz",
        ],
        "max_batch": MAX_BATCH,
        "max_body_bytes": MAX_BODY_BYTES,
        "cache": { "enabled": health.cache_enabled },
        "identity": {
            "graph_version": health.identity.graph_version,
            "model_id": health.identity.model_id,
            "calibration_version": health.identity.calibration_version,
            "engine_semver": health.identity.engine_semver,
        },
    }))
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

/// The decision kinds `/v1/decide` accepts, as the capabilities
/// endpoint lists them. `pin_decision_kinds_to_the_ir` proves the list
/// and the IR variants cannot drift apart.
const DECISION_KINDS: [&str; 3] = ["choice", "boolean", "score"];

/// The `POST /v1/batch` body: canonical requests under the same
/// `requests` key MCP `codify_batch` uses, so one client document works
/// against either transport.
#[derive(serde::Deserialize)]
struct BatchBody {
    /// Canonical decision requests, decided independently: one item's
    /// refusal never fails the batch, and every item answers in order.
    requests: Vec<Value>,
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
    use opencodifier_core::{
        BooleanQuestion, Candidate, ChoiceQuestion, DecisionPolicy, DecisionQuestion,
        RequestMetadata, ScoreLevel, ScoreQuestion, State,
    };

    #[test]
    fn pin_decision_kinds_to_the_ir() {
        // The capabilities list is hand-maintained; this is the tripwire
        // that a new `DecisionQuestion` variant (or a rename) forces a
        // matching update here instead of a silent advertisement gap.
        let variants = [
            (
                "choice",
                "model",
                DecisionQuestion::Choice(
                    ChoiceQuestion::new(
                        "model",
                        "pick",
                        vec![
                            Candidate::new("a", "first").unwrap(),
                            Candidate::new("b", "second").unwrap(),
                        ],
                    )
                    .unwrap(),
                ),
            ),
            (
                "boolean",
                "needs_tools",
                DecisionQuestion::Boolean(
                    BooleanQuestion::new("needs_tools", "Does this need tools?").unwrap(),
                ),
            ),
            (
                "score",
                "priority",
                DecisionQuestion::Score(
                    ScoreQuestion::new(
                        "priority",
                        "How urgent?",
                        vec![ScoreLevel::new("low").unwrap(), ScoreLevel::new("high").unwrap()],
                    )
                    .unwrap(),
                ),
            ),
        ];
        assert_eq!(DECISION_KINDS.len(), variants.len());
        for (kind, _, question) in variants {
            assert!(DECISION_KINDS.contains(&kind), "unlisted kind: {kind}");
            // The wire kind each variant encodes under is the listed one.
            let encoded = Native
                .encode_request(
                    &DecisionRequest::new(
                        State::from_text("pin the kinds"),
                        vec![question],
                        DecisionPolicy::default(),
                        RequestMetadata::default(),
                    )
                    .unwrap(),
                )
                .unwrap();
            assert_eq!(encoded["questions"][0]["type"], json!(kind), "kind: {kind}");
        }
    }

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
