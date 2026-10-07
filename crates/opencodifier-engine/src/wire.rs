//! The native-schema wire boundary, transport-neutral (PLANNING.md §57,
//! §68; D23).
//!
//! The functions here are the whole decode → decide → encode contract that
//! every embedding surface shares: the WASM binding
//! (`opencodifier-wasm`) and the C ABI (`opencodifier-ffi`) both call
//! them, so a request decided in a browser, through a mobile static
//! library, or in the conformance fixtures walks identical code and
//! reports identical error codes. The HTTP surface is deliberately not
//! routed through this module — it selects among all four wire formats
//! per route, which is its own contract in `opencodifier-http`.
//!
//! Wire formats are the native schema's, byte-for-byte what
//! `POST /v1/decide` accepts. All input is hostile: every entry point
//! decodes through the validating `opencodifier-schema` adapters and
//! answers with typed error codes (`schema.*`, `engine.*`, `graph.*`) as
//! JSON — the same codes the HTTP surface reports, so a client debugs
//! against one vocabulary.

use opencodifier_core::Limits;
use opencodifier_schema::WireFormat;

use crate::{DecisionGraph, EngineHandle, GraphDocument, MAX_BATCH};

/// Decodes, decides, and encodes a native-schema request payload: the
/// whole wire contract in one function.
///
/// # Errors
///
/// [`WireFailure`] for unparseable JSON, schema decode refusals, engine
/// errors, or encode failures.
pub fn decide_impl(handle: &EngineHandle, request_json: &str) -> Result<String, WireFailure> {
    let payload: serde_json::Value =
        serde_json::from_str(request_json).map_err(WireFailure::MalformedJson)?;
    let encoded = decide_value(handle, &payload)?;
    serde_json::to_string(&encoded).map_err(WireFailure::MalformedJson)
}

/// Decodes and decides one already-parsed request, returning the encoded
/// native response. The value-shaped core [`decide_impl`] and
/// [`decide_batch_impl`] both walk, so a batch never re-parses what the
/// envelope already parsed and a single request cannot drift from a
/// batched one.
///
/// # Errors
///
/// [`WireFailure`] naming the first violated invariant, with its stable
/// code.
pub fn decide_value(
    handle: &EngineHandle,
    payload: &serde_json::Value,
) -> Result<serde_json::Value, WireFailure> {
    let request =
        opencodifier_schema::native::Native.decode_request(payload, &Limits::default())?;
    let response = handle.decide(&request)?;
    Ok(opencodifier_schema::native::Native.encode_response(&response)?)
}

/// Decides every request of a batch envelope on the one engine — the
/// batch contract in one function (§11): `{"requests": [ ... ]}` in, one
/// decision each, `{"results": [...], "count": N}` out.
///
/// The envelope is the `POST /v1/batch` body — the same `requests` key
/// MCP `codify_batch` uses — and its items are native-schema request
/// objects, not pre-stringified payloads. Every request is decided here,
/// in input order, on this engine: one crossing for the whole batch, and
/// one cache across it, so a request repeated inside a batch is a cache
/// hit.
///
/// Each entry of `results` is that item's `{"response": ...}` or, when
/// the item's decode or execution failed, its
/// `{"error": {"code", "message"}}` with the same codes [`decide_impl`]
/// reports. One item's refusal is that item's result, never a failure of
/// the batch.
///
/// # Errors
///
/// A batch that is unusable as a batch: unparseable JSON
/// (`schema.invalid_json`), an envelope without a `requests` array
/// (`schema.invalid_value`), or more requests than [`MAX_BATCH`]
/// (`schema.limit_exceeded`) — the ceiling the HTTP batch enforces.
pub fn decide_batch_impl(
    handle: &EngineHandle,
    requests_json: &str,
) -> Result<String, WireFailure> {
    let payload: serde_json::Value =
        serde_json::from_str(requests_json).map_err(WireFailure::MalformedJson)?;
    let requests = batch_requests(&payload)?;
    if requests.len() > MAX_BATCH {
        return Err(opencodifier_schema::SchemaError::limit(
            "max_batch",
            requests.len(),
            MAX_BATCH,
        )
        .into());
    }
    let results = requests.iter().map(|document| batch_item(handle, document)).collect::<Vec<_>>();
    serde_json::to_string(&serde_json::json!({ "results": results, "count": results.len() }))
        .map_err(WireFailure::MalformedJson)
}

/// The batch envelope's `requests` array, borrowed: the key is required
/// and must be an array, and other envelope members are ignored — the
/// `POST /v1/batch` body's contract. Both refusals name `requests` under
/// `schema.invalid_value`, so a client sees one diagnostic on either
/// transport.
///
/// # Errors
///
/// [`WireFailure`] with `schema.invalid_value` when the envelope is not
/// a `requests` array.
fn batch_requests(payload: &serde_json::Value) -> Result<&Vec<serde_json::Value>, WireFailure> {
    match payload.get("requests") {
        Some(serde_json::Value::Array(requests)) => Ok(requests),
        Some(other) => Err(opencodifier_schema::SchemaError::invalid_value(
            "requests",
            format!("invalid type: {}, expected a sequence", json_type_name(other)),
        )
        .into()),
        None => Err(opencodifier_schema::SchemaError::invalid_value(
            "requests",
            "missing field `requests`",
        )
        .into()),
    }
}

/// The JSON type of `value`, for diagnostics: what a malformed envelope
/// actually carried.
fn json_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "sequence",
        serde_json::Value::Object(_) => "map",
    }
}

/// Decodes and decides one batch item, producing its result object: the
/// envelope the HTTP batch reports, so a client reading one reads the
/// other. A decode failure and an engine failure land in the same
/// `{"error": {"code", "message"}}` shape — from the client's side both
/// are "this item has no decision", and the `schema.*` vs `engine.*`
/// code keeps the cause distinguishable.
fn batch_item(handle: &EngineHandle, document: &serde_json::Value) -> serde_json::Value {
    match decide_value(handle, document) {
        Ok(response) => serde_json::json!({ "response": response }),
        Err(failure) => serde_json::json!({ "error": failure.error_json() }),
    }
}

/// Validates a decision graph without running it: construction enforces
/// the DAG contract (ids, edges, single output, cycles, node limit, knob
/// kinds).
///
/// Returns the graph version and node count, so a client can confirm the
/// shape it is about to run.
///
/// # Errors
///
/// [`WireFailure`] naming the first violated invariant (`graph.*`).
pub fn validate_graph_impl(graph_json: &str) -> Result<(u64, usize), WireFailure> {
    let document: GraphDocument =
        serde_json::from_str(graph_json).map_err(WireFailure::MalformedJson)?;
    let (version, nodes) = (document.version, document.nodes.len());
    DecisionGraph::new(version, document.nodes)?;
    Ok((version, nodes))
}

/// Validates a graph, decodes a request, decides on a throwaway engine
/// (D19 parity with `POST /v1/graph/run`): the graph's cache identity is
/// derived from its own content, the engine is assembled fresh per call,
/// and no state survives the call — a client cannot warm, read, or
/// poison anyone else's decisions.
///
/// # Errors
///
/// Graph validation, request decode, or engine errors, as
/// [`WireFailure`].
pub fn run_graph_impl(graph_json: &str, request_json: &str) -> Result<String, WireFailure> {
    let document: GraphDocument =
        serde_json::from_str(graph_json).map_err(WireFailure::MalformedJson)?;
    let graph = DecisionGraph::new(document.version, document.nodes)?;
    let handle = EngineHandle::ephemeral(graph)?;
    decide_impl(&handle, request_json)
}

/// The error half of the wire contract: a stable code plus its message,
/// serializable to JSON exactly once, at the boundary.
#[derive(Debug)]
pub enum WireFailure {
    /// The payload was not JSON at all — refused as `schema.invalid_json`.
    MalformedJson(serde_json::Error),
    /// The payload was JSON but not a valid native-schema request,
    /// envelope, or graph — refused with the schema adapter's code.
    Schema(opencodifier_schema::SchemaError),
    /// The request was valid and the engine refused or failed it —
    /// `engine.timeout`, `graph.cycle`, `engine.missing_backend`, ...
    Engine(crate::error::EngineError),
}

impl From<opencodifier_schema::SchemaError> for WireFailure {
    fn from(error: opencodifier_schema::SchemaError) -> Self {
        Self::Schema(error)
    }
}

impl From<crate::error::EngineError> for WireFailure {
    fn from(error: crate::error::EngineError) -> Self {
        Self::Engine(error)
    }
}

impl WireFailure {
    /// The stable error code (`schema.invalid_value`, `engine.timeout`,
    /// `graph.cycle`, ...).
    pub fn code(&self) -> String {
        match self {
            Self::MalformedJson(_) => "schema.invalid_json".to_owned(),
            Self::Schema(error) => error.code().to_owned(),
            Self::Engine(error) => error.code().to_owned(),
        }
    }

    /// The JSON object the boundary reports: the stable code plus its
    /// message, built exactly once. A batch item's error is the same
    /// object, so one refusal reads identically inside an envelope and
    /// across the boundary.
    pub fn error_json(&self) -> serde_json::Value {
        serde_json::json!({ "code": self.code(), "message": self.to_string() })
    }
}

impl std::fmt::Display for WireFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MalformedJson(error) => write!(formatter, "{error}"),
            Self::Schema(error) => write!(formatter, "{error}"),
            Self::Engine(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for WireFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::MalformedJson(error) => Some(error),
            Self::Schema(error) => Some(error),
            Self::Engine(error) => Some(error),
        }
    }
}
