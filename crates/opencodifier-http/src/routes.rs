//! The `/v1` routes: decide, graph validation, and health.
//!
//! The surface is deliberately thin. Every endpoint normalizes through
//! [`opencodifier_schema`] and executes through
//! [`opencodifier_engine::EngineHandle`] — no pipeline logic lives here, so
//! HTTP and MCP cannot drift apart on what a decision is.
//!
//! ```text
//! bytes ──► serde_json::Value ──► <selected>.decode_request ──► EngineHandle.decide
//!                                                                        │
//! ◄── <selected>.encode_response ◄───────────────────────────────────────┘
//! ```
//!
//! The adapter is selected by the `x-opencodifier-format` header (D30);
//! absent or `native`, the surface behaves exactly as it did when every
//! route decoded through [`Native`] — existing callers are byte-identical.
//!
//! Posture: every request body is hostile input. The body size is capped
//! before parsing, payloads are never echoed back, and abstention is a
//! `200` — a refused decision is the runtime working, not a failure.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use opencodifier_core::{DecisionRequest, DecisionResponse, Limits};
use opencodifier_engine::{DecisionGraph, EngineHandle, EngineResult, GraphDocument, MAX_BATCH};
use opencodifier_schema::native::Native;
use opencodifier_schema::{
    SchemaError, WireFormat, anthropic::Anthropic, jev::Jev, openai::OpenAi,
};
use serde_json::{Value, json};

use crate::error::HttpError;

/// Maximum accepted request body, in bytes.
///
/// Sized for images (D38): [`Limits::max_input_bytes`] of text plus the
/// image ceilings ([`Limits::max_images`] × [`Limits::max_image_bytes`])
/// re-encoded as base64 — 1 MiB + 4 × 2 MiB × 4⁄3 ≈ 11.7 MiB — so a body
/// the IR would accept is never rejected at the socket, while anything
/// larger is refused before parsing. The unit test below pins the value,
/// so a change to the ceilings has to revisit this cap deliberately.
pub const MAX_BODY_BYTES: usize = 12 * 1_048_576;

/// The request header that selects the wire adapter (D30).
///
/// Values are the adapters' own stable `name()`s: `native` (the default,
/// also when the header is absent), `openai`, `anthropic`, `jev`. A value
/// that names no adapter is a `400` with a `schema.invalid_value` code —
/// the surface never guesses at a near-miss format name.
pub const FORMAT_HEADER: &str = "x-opencodifier-format";

/// A selected wire adapter, boxed with the thread-safety the transport
/// needs.
///
/// The `WireFormat` trait itself carries no `Send` bound; the HTTP
/// handlers need it because a selected adapter is held across the
/// `spawn_blocking` await (and the batch moves one into the blocking
/// task). Every shipped adapter is a stateless value object, so the
/// bound is free to satisfy — the narrower local alias keeps the
/// public trait untouched (D30).
type SelectedFormat = Box<dyn WireFormat + Send>;

/// Resolves [`FORMAT_HEADER`] to the wire adapter for this request.
///
/// Header absent ⇒ [`Native`], which is why every pre-D30 caller is
/// byte-identical. The adapters are stateless value objects, so building
/// one per request is free; a shared registry would buy nothing (D30).
///
/// # Errors
///
/// A header value that is not valid ASCII, or that names no known
/// adapter, is a `schema.invalid_value` refusal naming the supported set.
fn selected_format(headers: &HeaderMap) -> Result<SelectedFormat, HttpError> {
    let Some(value) = headers.get(FORMAT_HEADER) else {
        return Ok(Box::new(Native));
    };
    let name = value.to_str().map_err(|_| {
        HttpError::from(SchemaError::invalid_value(
            FORMAT_HEADER,
            "header value is not valid ASCII",
        ))
    })?;
    match name.trim().to_ascii_lowercase().as_str() {
        "native" => Ok(Box::new(Native)),
        "openai" => Ok(Box::new(OpenAi)),
        "anthropic" => Ok(Box::new(Anthropic)),
        "jev" => Ok(Box::new(Jev)),
        other => Err(HttpError::from(SchemaError::invalid_value(
            FORMAT_HEADER,
            format!("`{other}` names no wire format; supported: native, openai, anthropic, jev"),
        ))),
    }
}

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
        .route("/v1/graph/run", post(graph_run))
        .route("/v1/validate", post(validate_request))
        .route("/v1/models", get(models))
        .route("/v1/capabilities", get(capabilities))
        .route("/v1/healthz", get(healthz))
        .route("/v1/chat/completions", post(chat_completions))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(handle)
}

/// `POST /v1/chat/completions` — the OpenAI-compatible decision surface.
///
/// Amortyx and every other OpenAI-shapeable client speak chat
/// completions, so the surface accepts the constrained slice of that
/// shape which *is* a decision: a strict `json_schema` response format
/// whose schema is exactly the strict-mode subset [`OpenAi`] documents.
/// Messages become the state text, the response format's schema becomes
/// the adapter's schema map (`required` synthesized from the property
/// names when the caller omits it — strict mode makes every property
/// required), and the answer is a chat completion
/// whose content is the adapter's structured-output JSON. Everything
/// flows through the same engine as `/v1/decide` — the route is a
/// projection, never a second pipeline (D12: the `/v1` surface is
/// adapter-first).
///
/// Free-form generation is refused with
/// `schema.unsupported_generation_field`: a request without a
/// `response_format`, with `response_format.type` other than
/// `json_schema`, or with message content that is neither a string nor
/// text/`image_url` parts asks the runtime to write prose, and the runtime
/// decides instead of pretending. Images in message content ride into the
/// IR state (D38); whether the runtime can read them is the engine's
/// vision gate, so an image the configured ladder has no rung for is a
/// `400` (`engine.vision_required`), never a silently text-only decision.
///
/// Abstention and every other typed outcome stay `200`; the
/// `opencodifier` extension object carries the outcome and calibrated
/// confidence a chat shape has no slot for.
async fn chat_completions(
    State(handle): State<Arc<EngineHandle>>,
    body: Bytes,
) -> Result<Json<Value>, HttpError> {
    let payload = parse_json(&body)?;
    let obj = payload.as_object().ok_or_else(|| {
        HttpError::from(SchemaError::invalid_type("body", "a JSON object", &payload))
    })?;

    let model = obj.get("model").and_then(Value::as_str).unwrap_or("opencodifier");
    let input = state_from_messages(obj.get("messages"))?;
    let properties = decision_properties(obj.get("response_format"))?;
    let required = schema_required(obj.get("response_format"), &properties);

    let request: DecisionRequest = OpenAi
        .decode_request(
            &json!({
                "input": input,
                "format": { "strict": true, "schema": {
                    "properties": properties, "required": required,
                } },
            }),
            &Limits::default(),
        )
        .map_err(HttpError::from)?;

    let response: DecisionResponse = tokio::task::spawn_blocking(move || handle.decide(&request))
        .await
        .map_err(|_| HttpError::Engine(opencodifier_engine::EngineError::Cancelled))?
        .map_err(HttpError::from)?;

    let content = OpenAi.encode_response(&response).map_err(|error| {
        HttpError::Engine(opencodifier_engine::EngineError::Serialization {
            reason: error.to_string(),
        })
    })?;
    let created = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    Ok(Json(json!({
        "id": format!("chatcmpl-{created}"),
        "object": "chat.completion",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": content.to_string() },
            "finish_reason": "stop",
        }],
        // The decision ran on the deterministic engine: no model tokens were
        // consumed, and the zeroed accounting says so honestly. OpenAI-shape
        // clients (and gateways that decode the envelope) require the field.
        "usage": {
            "prompt_tokens": 0,
            "completion_tokens": 0,
            "total_tokens": 0,
        },
        "opencodifier": {
            "outcome": response.outcome(),
            "calibrated_confidence": response.confidence().calibrated_confidence,
        },
    })))
}

/// Flattens chat messages into the `input` the `OpenAi` adapter decodes.
///
/// System and user contents join in message order (assistant turns are
/// skipped — the runtime decides over the caller's state, not over a
/// conversation to continue). String contents join with a blank line and
/// produce the plain-string `input` exactly as before D38. Content-block
/// arrays concatenate their text and collect their `image_url` parts; with
/// images present, `input` becomes the adapter's parts array — one
/// `input_text` part then the images — so the single normalization grammar
/// is the adapter's own, never a second pipeline here (D12). Any other
/// content part is a refusal: silently dropping it would decide over less
/// than the caller sent.
fn state_from_messages(messages: Option<&Value>) -> Result<Value, HttpError> {
    let messages = messages
        .and_then(Value::as_array)
        .ok_or_else(|| HttpError::from(SchemaError::missing("messages")))?;
    let mut text: Vec<String> = Vec::new();
    let mut images: Vec<Value> = Vec::new();
    for message in messages {
        let role = message.get("role").and_then(Value::as_str).unwrap_or("");
        if role != "system" && role != "user" {
            continue;
        }
        match message.get("content") {
            Some(Value::String(content)) => text.push(content.clone()),
            Some(Value::Array(blocks)) => {
                let mut chunk = String::new();
                for block in blocks {
                    let block = block.as_object().ok_or_else(|| {
                        HttpError::from(SchemaError::invalid_value(
                            "messages[].content[]",
                            "content parts must be objects",
                        ))
                    })?;
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            let part =
                                block.get("text").and_then(Value::as_str).ok_or_else(|| {
                                    HttpError::from(SchemaError::invalid_value(
                                        "messages[].content[].text",
                                        "text parts must carry string text",
                                    ))
                                })?;
                            chunk.push_str(part);
                        }
                        Some("image_url") => images.push(Value::Object(block.clone())),
                        other => {
                            return Err(HttpError::from(SchemaError::invalid_value(
                                "messages[].content[].type",
                                format!(
                                    "`{}` content parts carry no decision semantics; \
                                     only text and image_url are supported",
                                    other.unwrap_or("(missing)")
                                ),
                            )));
                        }
                    }
                }
                text.push(chunk);
            }
            _ => {
                return Err(HttpError::from(SchemaError::invalid_value(
                    "messages[].content",
                    "only string content or text/image_url parts are supported; \
                     the runtime decides over text and attached images",
                )));
            }
        }
    }
    let joined = text.join("\n\n");
    if joined.is_empty() && images.is_empty() {
        return Err(HttpError::from(SchemaError::invalid_value(
            "messages",
            "no system or user message content to decide over",
        )));
    }
    if images.is_empty() {
        return Ok(Value::String(joined));
    }
    let mut parts = vec![json!({ "type": "input_text", "text": joined })];
    parts.extend(images);
    Ok(Value::Array(parts))
}

/// Extracts the strict-mode schema map a decision needs from
/// `response_format`, or refuses with
/// `schema.unsupported_generation_field` when the request is a
/// generation request instead.
fn decision_properties(response_format: Option<&Value>) -> Result<Value, HttpError> {
    let format = response_format.ok_or_else(|| {
        HttpError::from(SchemaError::UnsupportedGenerationField { field: "response_format".into() })
    })?;
    let kind = format.get("type").and_then(Value::as_str).unwrap_or("");
    if kind != "json_schema" {
        return Err(HttpError::from(SchemaError::UnsupportedGenerationField {
            field: "response_format.type".into(),
        }));
    }
    let schema = format
        .get("json_schema")
        .and_then(|js| js.get("schema"))
        .and_then(|s| s.get("properties"))
        .filter(|s| s.is_object())
        .ok_or_else(|| {
            HttpError::from(SchemaError::invalid_value(
                "response_format.json_schema.schema",
                "expected an object schema with `properties` naming one question per property",
            ))
        })?;
    Ok(schema.clone())
}

/// The strict-mode `required` list the adapter needs alongside
/// `properties`.
///
/// A caller-supplied list is honored verbatim — the adapter refuses an
/// incomplete one itself. A caller who wrote a well-shaped object schema
/// but left the list off gets the rule applied rather than a refusal:
/// every property being required is what strict mode means.
fn schema_required(response_format: Option<&Value>, properties: &Value) -> Value {
    let schema = response_format
        .and_then(|format| format.get("json_schema"))
        .and_then(|js| js.get("schema"));
    if let Some(required) = schema.and_then(|s| s.get("required")) {
        return required.clone();
    }
    Value::Array(
        properties
            .as_object()
            .map(|props| props.keys().map(|name| json!(name)).collect())
            .unwrap_or_default(),
    )
}

/// `POST /v1/decide` — decision request in (adapter-selected), the same
/// adapter's projection out.
///
/// Abstention and every other typed outcome are `200`: the outcome field
/// carries the decision, not the transport status. Decode failures are
/// `400` with the adapter's `schema.*` code; only an engine fault is
/// `500`. The error envelope itself is always the native one (D30): a
/// client needs exactly one error contract regardless of request format.
async fn decide(
    State(handle): State<Arc<EngineHandle>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, HttpError> {
    let format = selected_format(&headers)?;
    let payload = parse_json(&body)?;
    let request: DecisionRequest =
        format.decode_request(&payload, &Limits::default()).map_err(HttpError::from)?;

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
    let encoded = format.encode_response(&response).map_err(|error| {
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
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, HttpError> {
    let format = selected_format(&headers)?;
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
        requests
            .iter()
            .map(|document| batch_item(&handle, format.as_ref(), document))
            .collect::<Vec<Value>>()
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
/// code keeps the cause distinguishable. The per-item response is
/// projected through the batch's selected adapter (D30).
fn batch_item(handle: &EngineHandle, format: &dyn WireFormat, document: &Value) -> Value {
    let encoded = format
        .decode_request(document, &Limits::default())
        .map_err(HttpError::from)
        .and_then(|request| {
            let response = handle.decide(&request).map_err(HttpError::from)?;
            format.encode_response(&response).map_err(|error| {
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

/// `POST /v1/validate` — wire request in (adapter-selected), validity
/// verdict out.
///
/// The decode-and-validate path only: no decision is computed, nothing
/// is cached, and the answer says what was accepted so a client can
/// preflight a payload (question kinds, candidate counts) before paying
/// for execution. The verdict shape is the runtime's own — it does not
/// project through the adapter, because there is no external shape to
/// preserve for a preflight.
async fn validate_request(headers: HeaderMap, body: Bytes) -> Result<Json<Value>, HttpError> {
    let format = selected_format(&headers)?;
    let payload = parse_json(&body)?;
    let request: DecisionRequest =
        format.decode_request(&payload, &Limits::default()).map_err(HttpError::from)?;
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
/// (`relational-v2|builtin-lexical-v2` for the base binary) rather than
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
            "/v1/decide", "/v1/batch", "/v1/graph/validate", "/v1/graph/run",
            "/v1/validate", "/v1/models", "/v1/capabilities", "/v1/healthz",
        ],
        "max_batch": MAX_BATCH,
        "max_body_bytes": MAX_BODY_BYTES,
        "cache": { "enabled": health.cache_enabled },
        "identity": {
            "graph_version": health.identity.graph_version,
            "model_id": health.identity.model_id,
            "calibration_version": health.identity.calibration_version,
            "engine_semver": health.identity.engine_semver,
            "embedding_model": health.identity.embedding_model,
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

/// `POST /v1/graph/run` — decide one request through a client-supplied
/// graph (D19).
///
/// The graph is the one piece of pipeline structure a client may steer,
/// so every rule D19 sets is structural here, not conventional: the graph
/// is capped and validated through the same constructor
/// [`validate_graph`] uses, and execution goes through
/// [`EngineHandle::ephemeral`], which derives a content-addressed
/// identity and disables the shared cache — an ad-hoc run can never read
/// a decision made under a different graph. The response carries that
/// identity alongside the ordinary decision envelope, and abstention
/// stays a `200`.
async fn graph_run(body: Bytes) -> Result<Json<Value>, HttpError> {
    let payload = parse_json(&body)?;
    let body: GraphRunBody = serde_json::from_value(payload).map_err(|error| {
        HttpError::from(SchemaError::invalid_value("graph_run", error.to_string()))
    })?;

    let limits = Limits::default();
    if body.graph.nodes.len() > limits.max_graph_nodes {
        return Err(HttpError::Schema(SchemaError::limit(
            "max_graph_nodes",
            body.graph.nodes.len(),
            limits.max_graph_nodes,
        )));
    }
    let graph = DecisionGraph::try_from(body.graph).map_err(HttpError::Graph)?;

    let request: DecisionRequest =
        Native.decode_request(&body.request, &limits).map_err(HttpError::from)?;

    // Assembly and decide are both synchronous (D5): one blocking task
    // builds the scoped engine, decides, and drops it.
    let (response, identity) = tokio::task::spawn_blocking(move || -> EngineResult<_> {
        let handle = EngineHandle::ephemeral(graph)?;
        let response = handle.decide(&request)?;
        Ok((response, handle.identity()))
    })
    .await
    .map_err(|_| HttpError::Engine(opencodifier_engine::EngineError::Cancelled))?
    .map_err(HttpError::from)?;

    let encoded = Native.encode_response(&response).map_err(|error| {
        HttpError::Engine(opencodifier_engine::EngineError::Serialization {
            reason: error.to_string(),
        })
    })?;
    Ok(Json(json!({
        "response": encoded,
        "identity": {
            "graph_version": identity.graph_version,
            "model_id": identity.model_id,
            "calibration_version": identity.calibration_version,
            "engine_semver": identity.engine_semver,
            "embedding_model": identity.embedding_model,
        },
    })))
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
            "embedding_model": health.identity.embedding_model,
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

/// The `POST /v1/graph/run` body (D19): a graph document plus one
/// canonical request to decide through it.
#[derive(serde::Deserialize)]
struct GraphRunBody {
    /// The graph to execute, validated through the engine's own
    /// constructor and capped at the IR's node limit before assembly.
    graph: GraphDocument,
    /// The canonical request decided under `graph`.
    request: Value,
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
        BooleanQuestion, Candidate, ChoiceQuestion, DecisionPolicy, DecisionQuestion, Limits,
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
    fn body_limit_admits_the_ir_image_ceilings() {
        // The widest legal body: 1 MiB of text plus four 2 MiB images
        // base64-encoded (4/3) ≈ 11.7 MiB — inside the cap. A value this
        // size must not be refused at the socket before the IR sees it.
        let limits = Limits::default();
        let widest = limits.max_input_bytes + limits.max_images * limits.max_image_bytes * 4 / 3;
        assert!(widest <= MAX_BODY_BYTES, "widest legal body {widest} exceeds the cap");
        assert_eq!(MAX_BODY_BYTES, 12 * 1_048_576);
    }

    /// Text-only chat messages keep the exact pre-D38 input: a plain
    /// string, messages joined with a blank line.
    #[test]
    fn text_only_chat_messages_stay_a_plain_string() {
        let messages = json!([
            {"role": "system", "content": "be brief"},
            {"role": "assistant", "content": "skipped"},
            {"role": "user", "content": "route "},
            {"role": "user", "content": "this request"},
        ]);
        let input = state_from_messages(Some(&messages)).unwrap();
        assert_eq!(input, json!("be brief\n\nroute \n\nthis request"));
    }

    /// Image parts ride into the adapter's parts array, text first; the
    /// adapter's own grammar does the decoding from there.
    #[test]
    fn image_parts_project_onto_the_adapter_input() {
        let messages = json!([
            {"role": "user", "content": [
                {"type": "text", "text": "classify the chart"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,iVBORw0KGgo="}},
            ]},
        ]);
        let input = state_from_messages(Some(&messages)).unwrap();
        let parts = input.as_array().unwrap();
        assert_eq!(parts[0], json!({"type": "input_text", "text": "classify the chart"}));
        assert_eq!(parts[1]["type"], json!("image_url"));
        assert_eq!(parts[1]["image_url"]["url"], json!("data:image/png;base64,iVBORw0KGgo="));

        // The projected input decodes through the adapter end to end.
        let decoded = OpenAi
            .decode_request(
                &json!({ "input": input, "format": {"strict": true, "schema": {
                    "type": "object",
                    "properties": {"needs_tools": {"type": "boolean"}},
                    "required": ["needs_tools"],
                    "additionalProperties": false,
                }}}),
                &Limits::default(),
            )
            .unwrap();
        assert_eq!(decoded.state().text(), "classify the chart");
        assert_eq!(decoded.state().image_count(), 1);
    }

    /// Content the runtime cannot decide over — a non-string,
    /// non-array part, or an unknown part type — is a typed refusal, never
    /// silently dropped.
    #[test]
    fn undecidable_chat_content_is_refused() {
        let number = json!([{"role": "user", "content": 42}]);
        assert_eq!(state_from_messages(Some(&number)).unwrap_err().code(), "schema.invalid_value");

        let audio = json!([{"role": "user", "content": [
            {"type": "input_audio", "audio": "data:audio/wav;base64,aGk="},
        ]}]);
        assert_eq!(state_from_messages(Some(&audio)).unwrap_err().code(), "schema.invalid_value");

        let empty = json!([{"role": "assistant", "content": "only assistant turns"}]);
        assert_eq!(state_from_messages(Some(&empty)).unwrap_err().code(), "schema.invalid_value");
    }

    /// An image-bearing chat completion against the zero-ML engine (no
    /// vision rung) is the engine's typed `engine.vision_required` refusal
    /// as a 400 — the decision is refused, not silently made text-only.
    #[tokio::test]
    async fn an_image_chat_completion_without_a_vision_rung_is_a_400() {
        let handle = zero_ml_engine();
        let body = json!({
            "model": "opencodifier",
            "messages": [
                {"role": "user", "content": [
                    {"type": "text", "text": "classify the attached chart"},
                    {"type": "image_url", "image_url": {"url": "data:image/png;base64,iVBORw0KGgo="}},
                ]},
            ],
            "response_format": {"type": "json_schema", "json_schema": {"schema": {
                "type": "object",
                "properties": {"needs_tools": {"type": "boolean"}},
                "required": ["needs_tools"],
                "additionalProperties": false,
            }}},
        });
        let error = chat_completions(
            State(Arc::clone(&handle)),
            Bytes::from(serde_json::to_vec(&body).unwrap()),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code(), "engine.vision_required", "{error}");
        assert_eq!(error.status(), axum::http::StatusCode::BAD_REQUEST);
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

    fn zero_ml_engine() -> Arc<EngineHandle> {
        Arc::new(
            EngineHandle::lexical(
                opencodifier_engine::EngineConfig::with_default_pipeline().unwrap(),
            )
            .unwrap(),
        )
    }

    /// A batch envelope that is not `{"requests": [...]}` is a client
    /// error named at the boundary — the handler never guesses.
    #[tokio::test]
    async fn a_malformed_batch_body_is_a_schema_refusal() {
        let handle = zero_ml_engine();
        for payload in ["{}", r#"{"requests": "all at once"}"#] {
            let error = batch(
                State(Arc::clone(&handle)),
                HeaderMap::new(),
                Bytes::from(payload.to_owned()),
            )
            .await
            .unwrap_err();
            assert_eq!(error.code(), "schema.invalid_value", "{payload}");
            assert_eq!(error.status(), axum::http::StatusCode::BAD_REQUEST);
        }
    }

    /// The one-question choice every format test decides: two candidates,
    /// lexical-decidable, deterministic under the zero-ML engine.
    fn one_choice_request() -> DecisionRequest {
        DecisionRequest::new(
            State::from_text("deploy is failing; choose an action"),
            vec![DecisionQuestion::Choice(
                ChoiceQuestion::new(
                    "action",
                    "Which action should the on-call take?",
                    vec![
                        Candidate::new("rollback", "roll back to the last healthy build").unwrap(),
                        Candidate::new("hotfix", "patch forward on the broken build").unwrap(),
                    ],
                )
                .unwrap(),
            )],
            DecisionPolicy::default(),
            RequestMetadata::default(),
        )
        .unwrap()
    }

    /// D30: the format header selects the adapter end to end. An
    /// `OpenAI`-encoded body is decoded by the `OpenAI` adapter and answered
    /// with the `OpenAI` projection of the *same* decision the native body
    /// produces; an explicit `native` header is byte-identical to the
    /// absent header.
    #[tokio::test]
    async fn the_format_header_selects_the_wire_adapter() {
        let handle = zero_ml_engine();
        let request = one_choice_request();
        let expected = OpenAi.encode_response(&handle.decide(&request).unwrap()).unwrap();
        let native_body = Native.encode_request(&request).unwrap();
        let openai_body = OpenAi.encode_request(&request).unwrap();

        let native = decide(
            State(Arc::clone(&handle)),
            HeaderMap::new(),
            Bytes::from(serde_json::to_vec(&native_body).unwrap()),
        )
        .await
        .unwrap();

        let mut openai_headers = HeaderMap::new();
        openai_headers.insert(FORMAT_HEADER, "openai".parse().unwrap());
        let openai = decide(
            State(Arc::clone(&handle)),
            openai_headers,
            Bytes::from(serde_json::to_vec(&openai_body).unwrap()),
        )
        .await
        .unwrap();

        let mut native_headers = HeaderMap::new();
        native_headers.insert(FORMAT_HEADER, "native".parse().unwrap());
        let explicit_native = decide(
            State(Arc::clone(&handle)),
            native_headers,
            Bytes::from(serde_json::to_vec(&native_body).unwrap()),
        )
        .await
        .unwrap();

        assert_eq!(openai.0, expected, "OpenAI body in, OpenAI projection out");
        assert_eq!(explicit_native.0, native.0, "explicit native == absent header");
        assert_ne!(openai.0, native.0, "the projections are distinct shapes");
    }

    /// D30: a format name that names no adapter is refused before the
    /// body is parsed, with the stable `schema.invalid_value` code.
    #[tokio::test]
    async fn an_unknown_format_header_is_a_schema_refusal() {
        let handle = zero_ml_engine();
        let mut headers = HeaderMap::new();
        headers.insert(FORMAT_HEADER, "yaml".parse().unwrap());
        let error = decide(State(Arc::clone(&handle)), headers, Bytes::from_static(b"{}"))
            .await
            .unwrap_err();
        assert_eq!(error.code(), "schema.invalid_value");
        assert_eq!(error.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    /// D30: the batch applies the header's adapter to every item, each in
    /// its own per-item envelope.
    #[tokio::test]
    async fn batch_applies_the_selected_format_per_item() {
        let handle = zero_ml_engine();
        let request = one_choice_request();
        let wire = OpenAi.encode_request(&request).unwrap();
        let expected = OpenAi.encode_response(&handle.decide(&request).unwrap()).unwrap();

        let mut headers = HeaderMap::new();
        headers.insert(FORMAT_HEADER, "openai".parse().unwrap());
        let body = json!({ "requests": [wire, wire] });
        let batched = batch(
            State(Arc::clone(&handle)),
            headers,
            Bytes::from(serde_json::to_vec(&body).unwrap()),
        )
        .await
        .unwrap();

        assert_eq!(batched.0["count"], json!(2));
        let items = batched.0["results"].as_array().unwrap();
        for item in items {
            assert_eq!(item["response"], expected);
            assert!(item.get("error").is_none());
        }
    }

    /// D30: the preflight route decodes through the selected adapter too,
    /// while its verdict stays the runtime's own shape.
    #[tokio::test]
    async fn validate_accepts_the_selected_format() {
        let request = one_choice_request();
        let wire = OpenAi.encode_request(&request).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(FORMAT_HEADER, "openai".parse().unwrap());
        let verdict = validate_request(headers, Bytes::from(serde_json::to_vec(&wire).unwrap()))
            .await
            .unwrap();
        assert_eq!(verdict.0["valid"], json!(true));
        assert_eq!(verdict.0["questions"], json!(1));
    }

    /// The same posture for `POST /v1/graph/run`: a body without a
    /// well-typed graph and request is refused before any graph runs.
    #[tokio::test]
    async fn a_malformed_graph_run_body_is_a_schema_refusal() {
        for payload in ["{}", r#"{"graph": "not a graph", "request": {}}"#] {
            let error = graph_run(Bytes::from(payload.to_owned())).await.unwrap_err();
            assert_eq!(error.code(), "schema.invalid_value", "{payload}");
            assert_eq!(error.status(), axum::http::StatusCode::BAD_REQUEST);
        }
    }
}
