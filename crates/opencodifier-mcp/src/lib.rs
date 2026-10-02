//! `opencodifier-mcp` — the Model Context Protocol surface (PLAN Phase 10,
//! PLANNING.md §31, §55).
//!
//! Six decision tools over an assembled [`EngineHandle`], served on stdio
//! via `opencodifier mcp serve`. The surface is deliberately thin in the
//! same way the HTTP surface is: every tool normalizes its payload through
//! [`opencodifier_schema`] and executes through the engine facade, so MCP
//! and HTTP cannot drift apart on what a decision is.
//!
//! ```text
//! tool arguments ──► serde_json::Value ──► Native.decode_request ──► EngineHandle
//!                                                                        │
//! ◄── CallToolResult (structured + text) ◄── Native.encode_response ◄────┘
//! ```
//!
//! Posture (§73 carries over verbatim):
//!
//! * Tool arguments are hostile input. They are decoded by the validating
//!   adapter and never echoed back; failures carry a stable code and a
//!   typed message ([`envelope`]).
//! * Abstention is a successful outcome, never an error: a refused
//!   decision is `is_error: false` with the typed `outcome`.
//! * `codify_explain` returns the deterministic execution trace — the
//!   machine's own record of what ran. There is no chain-of-thought
//!   anywhere in this crate, because there is none anywhere in the
//!   runtime.
//! * Stdio only. The MCP transport is the local process's stdin/stdout, so
//!   the surface inherits the runtime's local-first posture structurally:
//!   there is no socket to bind and no remote surface to refuse.
//!
//! Concurrency note (D5): the engine is synchronous, and the HTTP shell
//! moves each decision onto a blocking thread because a concurrent server
//! must not let one request stall the runtime. A stdio MCP session is
//! strictly sequential — rmcp answers one request at a time, and the
//! zero-ML stack answers in microseconds — so tools call the engine
//! inline; there is no concurrency here to starve.

pub mod envelope;

use std::sync::Arc;

use opencodifier_core::{DecisionRequest, DecisionResponse, Limits};
use opencodifier_engine::{
    DecisionGraph, EngineError, EngineHandle, GraphDocument, execution_json,
};
use opencodifier_schema::{Native, SchemaError, WireFormat};
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Implementation, ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;
use serde_json::Value;

use crate::envelope::{Failure, respond};

/// Maximum number of requests one `codify_batch` call accepts.
///
/// The engine owns the policy (PLANNING.md §36 `POST /v1/batch` shares
/// it); re-exported so MCP callers keep a stable path. A runaway batch
/// is refused up front with `mcp.batch_too_large`.
pub use opencodifier_engine::MAX_BATCH;

/// The MCP server: the six Phase-15 decision tools over an assembled
/// [`EngineHandle`].
#[derive(Debug, Clone)]
pub struct OpenCodifierServer {
    /// The runtime every tool executes through.
    handle: Arc<EngineHandle>,
    /// The tool registry the `#[tool_handler]` dispatch reads.
    tool_router: ToolRouter<Self>,
}

/// Arguments of the single-request tools (`codify_decide`,
/// `codify_verify`, `codify_explain`).
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RequestArgs {
    /// A canonical (native) decision request — the exact document
    /// `POST /v1/decide` accepts: `state`, `questions`, `policy`,
    /// `metadata`. The document is the contract; this argument carries it
    /// verbatim rather than re-describing it field by field.
    pub request: Value,
}

/// Arguments of `codify_batch`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BatchArgs {
    /// Canonical decision requests, decided independently: one item's
    /// refusal never fails the batch, and every item answers in order.
    pub requests: Vec<Value>,
}

/// Arguments of `codify_validate`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ValidateArgs {
    /// A graph document — `version` plus `nodes` — in the engine's own
    /// serialization shape (the same document `recipes/*.json` and
    /// `--graph` carry).
    pub graph: Value,
}

#[tool_router]
impl OpenCodifierServer {
    /// Decides one canonical request and returns the canonical response.
    ///
    /// This is `POST /v1/decide` as a tool: native request in, native
    /// response out, abstention and every other typed outcome reported as
    /// a successful result.
    #[tool(
        name = "codify_decide",
        description = "Decide one canonical decision request (the native wire format: state, \
         questions, policy, metadata) and return the canonical response with per-question \
         answers, distributions, calibrated confidence, and the execution trace. Abstention is \
         a successful outcome, not an error."
    )]
    fn codify_decide(&self, Parameters(args): Parameters<RequestArgs>) -> CallToolResult {
        respond(self.decide_document(&args.request))
    }

    /// Decides a bounded list of requests independently.
    #[tool(
        name = "codify_batch",
        description = "Decide up to 16 canonical decision requests in one call. Items are \
         decided independently and in order: one item's refusal is that item's error envelope, \
         never a failure of the batch. Oversized batches are refused up front with \
         mcp.batch_too_large."
    )]
    fn codify_batch(&self, Parameters(args): Parameters<BatchArgs>) -> CallToolResult {
        respond(self.batch_document(&args.requests))
    }

    /// Reports the active pipeline.
    #[tool(
        name = "codify_graph",
        description = "Report the active decision pipeline: the engine identity decisions are \
         cached under (graph/model/calibration/semver), node count, parallelism, cache state, \
         and the full validated graph document actually being executed."
    )]
    fn codify_graph(&self) -> CallToolResult {
        respond(self.graph_document())
    }

    /// Validates a graph document without running it.
    #[tool(
        name = "codify_validate",
        description = "Validate a graph document (version + nodes) against the DAG contract — \
         ids, edges, single output, acyclicity, node limit — without running it. A valid graph \
         returns {valid: true, nodes: N}; a refused one returns the engine's own graph.* code."
    )]
    // A method, not an associated function, so the router registers it
    // exactly like its five siblings; the runtime it validates is ambient.
    #[allow(clippy::unused_self)]
    fn codify_validate(&self, Parameters(args): Parameters<ValidateArgs>) -> CallToolResult {
        respond(Self::validate_document(&args.graph))
    }

    /// Decides one request and returns the confidence-gate verdict.
    #[tool(
        name = "codify_verify",
        description = "Decide one canonical request and return the confidence-gate verdict: \
         the typed outcome, whether it is decisive, and the full multi-dimensional confidence \
         report (top probability, margin, entropy, OOD score, verifier agreement, calibrated \
         confidence). The full canonical response is included for the audit trail."
    )]
    fn codify_verify(&self, Parameters(args): Parameters<RequestArgs>) -> CallToolResult {
        respond(self.verify_document(&args.request))
    }

    /// Decides one request and returns the deterministic execution trace.
    #[tool(
        name = "codify_explain",
        description = "Decide one canonical request and return its explanation: the response's \
         deterministic execution trace (every node's typed record of what it did) plus the \
         executor's run report (waves, cache hit, candidate narrowing, lexical scores). This is \
         a machine trace of the deterministic pipeline — never generated reasoning."
    )]
    fn codify_explain(&self, Parameters(args): Parameters<RequestArgs>) -> CallToolResult {
        respond(self.explain_document(&args.request))
    }
}

// The handler's `call_tool` is async by trait definition, but the tools
// themselves are synchronous (the engine is sync, and this stdio session is
// strictly sequential) — the SDK's generated plumbing has nothing to await.
// Newer clippy lints exactly that as `unused_async_trait_impl`; older
// toolchains (the CI image's clippy) predate the lint, and `-D warnings`
// turns the unknown name into an error, so the lint group is allowed too.
#[allow(unknown_lints)]
#[allow(clippy::unused_async_trait_impl)]
#[tool_handler(router = self.tool_router)]
impl ServerHandler for OpenCodifierServer {
    fn get_info(&self) -> ServerInfo {
        let health = self.handle.health();
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("opencodifier", env!("CARGO_PKG_VERSION")))
            .with_instructions(format!(
                "OpenCodifier: a local-first, deterministic-first decision runtime. Every tool \
                 turns unstructured state into typed decisions (choice, boolean, score) with \
                 calibrated confidence; abstention is a successful outcome, never an error. \
                 Active pipeline: {} nodes, model {}.",
                health.nodes, health.identity.model_id
            ))
    }
}

impl OpenCodifierServer {
    /// Assembles the server over an already-constructed runtime.
    ///
    /// The handle is shared, not rebuilt: an MCP session sees exactly the
    /// pipeline, cache, and identity the embedding process assembled.
    #[must_use]
    pub fn new(handle: Arc<EngineHandle>) -> Self {
        Self { handle, tool_router: Self::tool_router() }
    }

    /// Native request document → native response document.
    fn decide_document(&self, document: &Value) -> Result<Value, Failure> {
        let request = Self::decode(document)?;
        let response = self.handle.decide(&request).map_err(|error| Failure::engine(&error))?;
        encode(&response)
    }

    /// Native request document → the gate verdict plus the full response.
    fn verify_document(&self, document: &Value) -> Result<Value, Failure> {
        let request = Self::decode(document)?;
        let response = self.handle.decide(&request).map_err(|error| Failure::engine(&error))?;
        let encoded = encode(&response)?;
        Ok(serde_json::json!({
            "outcome": encoded["outcome"],
            "decisive": response.outcome().is_decisive(),
            "confidence": encoded["confidence"],
            "metrics": encoded["metrics"],
            "answers": encoded["answers"],
            "response": encoded,
        }))
    }

    /// Native request document → response trace plus executor run report.
    fn explain_document(&self, document: &Value) -> Result<Value, Failure> {
        let request = Self::decode(document)?;
        let (response, report) =
            self.handle.decide_with_report(&request).map_err(|error| Failure::engine(&error))?;
        let encoded = encode(&response)?;
        Ok(serde_json::json!({
            "response": encoded,
            "execution": execution_json(&report),
        }))
    }

    /// A list of request documents → per-item verdicts, in order.
    fn batch_document(&self, requests: &[Value]) -> Result<Value, Failure> {
        if requests.len() > MAX_BATCH {
            return Err(Failure::new(
                "mcp.batch_too_large",
                format!("batch holds {} requests; the limit is {MAX_BATCH}", requests.len()),
            ));
        }
        let mut results = Vec::with_capacity(requests.len());
        let mut accepted = 0usize;
        for document in requests {
            match self.decide_document(document) {
                Ok(response) => {
                    accepted += 1;
                    results.push(serde_json::json!({ "ok": true, "response": response }));
                }
                Err(failure) => results.push(serde_json::json!({
                    "ok": false,
                    "error": { "code": failure.code, "message": failure.message },
                })),
            }
        }
        let refused = results.len() - accepted;
        Ok(serde_json::json!({ "results": results, "accepted": accepted, "refused": refused }))
    }

    /// The active pipeline as a document.
    fn graph_document(&self) -> Result<Value, Failure> {
        let health = self.handle.health();
        let identity = &health.identity;
        let graph = self.handle.graph();
        Ok(serde_json::json!({
            "status": "ok",
            "identity": {
                "graph_version": identity.graph_version,
                "model_id": identity.model_id,
                "calibration_version": identity.calibration_version,
                "engine_semver": identity.engine_semver,
            },
            "nodes": health.nodes,
            "parallelism": health.parallelism,
            "cache_enabled": health.cache_enabled,
            "graph": serde_json::to_value(graph)
                .map_err(|error| {
                    Failure::engine(&EngineError::Serialization { reason: error.to_string() })
                })?,
        }))
    }

    /// A graph document → validity verdict, through the engine's own path.
    fn validate_document(document: &Value) -> Result<Value, Failure> {
        let limits = Limits::default();
        let parsed: GraphDocument = serde_json::from_value(document.clone()).map_err(|error| {
            Failure::schema(&SchemaError::invalid_value("graph", error.to_string()))
        })?;
        if parsed.nodes.len() > limits.max_graph_nodes {
            return Err(Failure::schema(&SchemaError::limit(
                "max_graph_nodes",
                parsed.nodes.len(),
                limits.max_graph_nodes,
            )));
        }
        let graph = DecisionGraph::try_from(parsed).map_err(|error| Failure::engine(&error))?;
        EngineHandle::validate_graph(&graph).map_err(|error| Failure::engine(&error))?;
        Ok(serde_json::json!({ "valid": true, "nodes": graph.nodes().len() }))
    }

    /// Decodes a hostile payload through the validating native adapter.
    fn decode(document: &Value) -> Result<DecisionRequest, Failure> {
        Native.decode_request(document, &Limits::default()).map_err(|error| Failure::schema(&error))
    }
}

/// Encodes an engine-produced response into its canonical document.
///
/// Encoding a valid response cannot fail; if it ever does, it is an engine
/// fault (`engine.serialization`), never a caller's.
fn encode(response: &DecisionResponse) -> Result<Value, Failure> {
    Native.encode_response(response).map_err(|error| Failure::schema(&error))
}

/// Everything that can end an MCP serving session.
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    /// The MCP handshake failed: the client never initialized correctly.
    #[error("mcp initialize failed: {0}")]
    Initialize(
        /// The refusing SDK error, boxed: the SDK's error enum is far larger
        /// than the rest of this one.
        #[from]
        Box<rmcp::service::ServerInitializeError>,
    ),
    /// The serving task ended abnormally (panic or cancellation).
    #[error("mcp session failed: {0}")]
    Run(
        /// The join error from the serving task.
        #[from]
        tokio::task::JoinError,
    ),
}

/// Serves the runtime over stdio (§55: `opencodifier mcp serve`) until the
/// client disconnects or the session fails.
///
/// # Errors
///
/// [`ServeError::Initialize`] when the MCP handshake fails,
/// [`ServeError::Run`] when the serving task ends abnormally.
pub async fn serve_stdio(handle: Arc<EngineHandle>) -> Result<(), ServeError> {
    let server = OpenCodifierServer::new(handle);
    let transport = rmcp::transport::io::stdio();
    let running = server.serve(transport).await.map_err(Box::new)?;
    running.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;
    use crate::envelope::respond;
    use opencodifier_engine::EngineConfig;

    /// A request the default pipeline can decide.
    fn choice_document() -> Value {
        serde_json::json!({
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
            "metadata": { "request_id": "mcp-unit-0001", "limits": { "max_input_bytes": 1_048_576, "max_questions": 32, "max_candidates": 256, "max_graph_nodes": 128, "max_execution_time": { "secs": 10, "nanos": 0 }, "max_retrieval_results": 64 } },
        })
    }

    fn server() -> OpenCodifierServer {
        let handle = EngineHandle::lexical(EngineConfig::with_default_pipeline().unwrap()).unwrap();
        OpenCodifierServer::new(Arc::new(handle))
    }

    #[tokio::test]
    async fn decide_documents_round_trip_through_the_canonical_response() {
        let call = respond(server().decide_document(&choice_document()));
        assert_eq!(call.is_error, Some(false));
        let response = call.structured_content.unwrap();
        assert_eq!(response["answers"].as_array().unwrap().len(), 1);
        assert_eq!(response["answers"][0]["type"], "choice");
        assert!(response["confidence"]["top_probability"].is_number());
        assert_ne!(
            response["trace"]["entries"].as_array().unwrap(),
            &Vec::<serde_json::Value>::new()
        );
    }

    #[tokio::test]
    async fn hostile_payloads_are_typed_failures_not_panics() {
        let hostile = serde_json::json!({ "questions": "drop the table", "state": 42 });
        let call = respond(server().decide_document(&hostile));
        assert_eq!(call.is_error, Some(true));
        let error = call.structured_content.unwrap()["error"].clone();
        assert!(error["code"].as_str().unwrap().starts_with("schema."), "got {}", error["code"]);
        // The typed message names the refusing construct; the request is
        // not echoed back as a document.
        assert!(!error["message"].as_str().unwrap().contains("\"state\""));
    }

    #[tokio::test]
    async fn batches_refuse_oversize_up_front() {
        let requests = vec![choice_document(); MAX_BATCH + 1];
        let call = respond(server().batch_document(&requests));
        assert_eq!(call.is_error, Some(true));
        assert_eq!(call.structured_content.unwrap()["error"]["code"], "mcp.batch_too_large");
    }

    #[tokio::test]
    async fn batches_decide_items_independently() {
        let requests =
            vec![choice_document(), serde_json::json!({ "state": 1 }), choice_document()];
        let call = respond(server().batch_document(&requests));
        assert_eq!(call.is_error, Some(false));
        let batch = call.structured_content.unwrap();
        assert_eq!(batch["accepted"], 2);
        assert_eq!(batch["refused"], 1);
        let results = batch["results"].as_array().unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0]["ok"], true);
        assert_eq!(results[1]["ok"], false);
        assert!(results[1]["error"]["code"].as_str().unwrap().starts_with("schema."));
        assert_eq!(results[2]["ok"], true);
    }

    #[tokio::test]
    async fn graph_documents_report_the_active_pipeline() {
        let document = server().graph_document().unwrap();
        assert_eq!(document["status"], "ok");
        assert_eq!(document["nodes"], document["graph"]["nodes"].as_array().unwrap().len());
        assert!(document["identity"]["model_id"].as_str().unwrap().contains("lexical"));
        assert_eq!(document["graph"]["version"], 1);
    }

    #[test]
    fn validate_accepts_the_recipe_shape_and_reports_structural_codes() {
        let valid = serde_json::json!({
            "version": 1,
            "nodes": [
                { "id": "normalize", "kind": "normalize" },
                { "id": "choice", "kind": "choice", "depends_on": ["normalize"] },
                { "id": "threshold", "kind": "threshold", "depends_on": ["choice"], "threshold": 0.5 },
                { "id": "output", "kind": "output", "depends_on": ["threshold"] },
            ],
        });
        let verdict = OpenCodifierServer::validate_document(&valid).unwrap();
        assert_eq!(verdict["valid"], true);
        assert_eq!(verdict["nodes"], 4);

        let cyclic = serde_json::json!({
            "version": 1,
            "nodes": [
                { "id": "a", "kind": "normalize", "depends_on": ["b"] },
                { "id": "b", "kind": "filter", "depends_on": ["a"] },
            ],
        });
        let failure = OpenCodifierServer::validate_document(&cyclic).unwrap_err();
        assert_eq!(failure.code, "graph.cycle");
    }

    #[test]
    fn validate_reports_a_shape_error_as_a_schema_code() {
        // `nodes` must be an array; anything else never reaches the engine,
        // so the caller sees the adapter's code, not a `graph.*` one.
        let not_a_graph = serde_json::json!({ "version": 1, "nodes": { "a": "rule" } });
        let failure = OpenCodifierServer::validate_document(&not_a_graph).unwrap_err();
        assert_eq!(failure.code, "schema.invalid_value");
        assert!(failure.message.contains("graph"), "{}", failure.message);
    }

    #[test]
    fn validate_refuses_a_pipeline_over_the_node_limit_before_building_it() {
        // One node past the IR ceiling: the refusal is the adapter's limit
        // code, raised before `DecisionGraph` is ever constructed.
        let nodes: Vec<Value> = (0..=Limits::default().max_graph_nodes)
            .map(|index| serde_json::json!({ "id": format!("n{index:04}"), "kind": "normalize" }))
            .collect();
        let oversized = serde_json::json!({ "version": 1, "nodes": nodes });
        let failure = OpenCodifierServer::validate_document(&oversized).unwrap_err();
        assert_eq!(failure.code, "schema.limit_exceeded");
        assert!(failure.message.contains("max_graph_nodes"), "{}", failure.message);

        // The ceiling itself is still a legal pipeline, so the check is
        // strictly `>`: a chain from one entry node to one terminal `output`
        // that is exactly `max_graph_nodes` long validates.
        let limit = Limits::default().max_graph_nodes;
        let mut nodes = vec![serde_json::json!({ "id": "n0000", "kind": "normalize" })];
        for index in 1..limit - 1 {
            nodes.push(serde_json::json!({
                "id": format!("n{index:04}"),
                "kind": "rule",
                "depends_on": [format!("n{:04}", index - 1)],
            }));
        }
        nodes.push(serde_json::json!({
            "id": format!("n{:04}", limit - 1),
            "kind": "output",
            "depends_on": [format!("n{:04}", limit - 2)],
        }));
        let verdict = OpenCodifierServer::validate_document(&serde_json::json!({
            "version": 1,
            "nodes": nodes,
        }))
        .unwrap();
        assert_eq!(verdict["valid"], true);
        assert_eq!(verdict["nodes"], limit);
    }

    #[tokio::test]
    async fn verify_reports_the_gate_verdict() {
        let call = respond(server().verify_document(&choice_document()));
        assert_eq!(call.is_error, Some(false));
        let verdict = call.structured_content.unwrap();
        assert_eq!(verdict["answers"].as_array().unwrap().len(), 1);
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
    }

    #[tokio::test]
    async fn explain_is_the_deterministic_trace_only() {
        let call = respond(server().explain_document(&choice_document()));
        assert_eq!(call.is_error, Some(false));
        let explanation = call.structured_content.unwrap();
        let keys: Vec<&str> = explanation.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["execution", "response"]);
        assert_ne!(
            explanation["response"]["trace"]["entries"].as_array().unwrap(),
            &Vec::<serde_json::Value>::new()
        );
        assert_ne!(
            explanation["execution"]["waves"].as_array().unwrap(),
            &Vec::<serde_json::Value>::new()
        );
        assert!(explanation["execution"]["cache_hit"].is_boolean());
        // No generated reasoning anywhere in the document: the explanation
        // is the pipeline's own record, byte for byte.
        let rendered = explanation.to_string();
        assert!(!rendered.to_lowercase().contains("chain"));
        assert!(!rendered.to_lowercase().contains("reasoning"));
    }

    #[tokio::test]
    async fn abstention_is_a_successful_result() {
        let mut document = choice_document();
        document["policy"] = serde_json::json!({
            "min_confidence": 1.0, "verify_below": 1.0, "abstain_below": 1.0, "risk": "low",
        });
        let call = respond(server().decide_document(&document));
        assert_eq!(call.is_error, Some(false));
        assert_eq!(call.structured_content.unwrap()["outcome"], "abstain");
    }
}
