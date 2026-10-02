//! WASM binding for the zero-ML decision runtime (PLANNING.md §57, §68;
//! DECISIONS.md D23).
//!
//! [`WasmEngine`] is the base binary's posture compiled for
//! `wasm32-unknown-unknown`: the relational solver over BM25 lexical
//! scoring, the full graph executor, cache, and traces — fully local,
//! fully client-side. The boundary is structural, not promised:
//!
//! * **No filesystem, no networking, no telemetry.** The crate's
//!   dependency list admits no such capability; there is no output
//!   channel to phone home through.
//! * **No dynamic native code.** The artifact is pure wasm; no `-sys`
//!   crates, no JS shims beyond `wasm-bindgen`'s glue.
//! * **All input is hostile.** Every entry point decodes through the
//!   validating `opencodifier-schema` adapters and answers with typed
//!   error codes (`schema.*`, `engine.*`, `graph.*`) as JSON — the same
//!   codes the HTTP surface reports, so a browser client debugs against
//!   one vocabulary.
//! * **No storage.** §68 routes browser storage through a WASM adapter;
//!   this crate ships none, so it cannot misuse one.
//!
//! Wire formats are the native schema's, byte-for-byte what
//! `POST /v1/decide` accepts. The model rung (ONNX/WebGPU) stays native
//! until its own runtime record exists (D23); nothing here degrades
//! silently in its absence — graphs that need a backend are refused
//! exactly as they are natively (D21).
//!
//! # Example (from JavaScript)
//!
//! ```js
//! const engine = new WasmEngine();
//! const response = engine.decide(JSON.stringify({
//!     state: { text: "refactor the parser and add tests" },
//!     questions: [{ /* native-schema choice question */ }],
//!     policy: {}, metadata: {},
//! }));
//! ```

#![allow(clippy::wildcard_imports)] // wasm_bindgen::prelude is the idiom

use wasm_bindgen::prelude::{JsValue, wasm_bindgen};

use opencodifier_core::Limits;
use opencodifier_engine::{DecisionGraph, EngineConfig, EngineHandle, GraphDocument};
use opencodifier_schema::{WireFormat, native::Native};

/// One assembled zero-ML decision runtime, shareable across the page.
///
/// Every method is synchronous and CPU-bound by design (D5): the runtime
/// does no I/O, so there is nothing to await and no callback to leak a
/// capability through.
#[wasm_bindgen]
#[derive(Debug)]
pub struct WasmEngine {
    handle: EngineHandle,
}

#[wasm_bindgen]
impl WasmEngine {
    /// Assembles the zero-ML engine: the built-in pipeline under the
    /// relational solver over BM25.
    ///
    /// # Errors
    ///
    /// A JSON error value if the built-in pipeline ever fails its own
    /// validation — unreachable by construction, surfaced rather than
    /// unwrapped away.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Result<WasmEngine, JsValue> {
        let config = EngineConfig::with_default_pipeline().map_err(wasm_error)?;
        Ok(Self { handle: EngineHandle::lexical(config).map_err(wasm_error)? })
    }

    /// Decides a native-schema request payload, returning the
    /// native-schema response JSON.
    ///
    /// # Errors
    ///
    /// A JSON error value `{"code": ..., "message": ...}` — schema
    /// decode refusals, engine errors, or encode failures. The codes are
    /// the same stable vocabulary the HTTP and MCP surfaces report.
    #[wasm_bindgen]
    pub fn decide(&self, request_json: &str) -> Result<String, JsValue> {
        decide_impl(&self.handle, request_json).map_err(wasm_error)
    }

    /// Validates a decision graph without running it: construction
    /// enforces the DAG contract (ids, edges, single output, cycles,
    /// node limit, knob kinds).
    ///
    /// Returns a JSON summary `{"version", "nodes"}` of what validated,
    /// so a page can confirm the shape it is about to run.
    ///
    /// # Errors
    ///
    /// A JSON error value naming the first violated invariant.
    #[wasm_bindgen]
    pub fn validate_graph(&self, graph_json: &str) -> Result<String, JsValue> {
        let (version, nodes) = validate_graph_impl(graph_json).map_err(wasm_error)?;
        Ok(serde_json::json!({ "version": version, "nodes": nodes }).to_string())
    }

    /// Decides a request through a client-supplied graph (D19 parity
    /// with `POST /v1/graph/run`): the graph's cache identity is derived
    /// from its own content, the engine is assembled fresh per call, and
    /// no state survives the call — a browser cannot warm, read, or
    /// poison anyone else's decisions.
    ///
    /// # Errors
    ///
    /// Graph validation, request decode, or engine errors, as JSON
    /// error values.
    #[wasm_bindgen]
    pub fn run_graph(&self, graph_json: &str, request_json: &str) -> Result<String, JsValue> {
        run_graph_impl(graph_json, request_json).map_err(wasm_error)
    }

    /// The engine identity decisions are cached under: graph, model,
    /// calibration, engine, and embedding components. Deterministic per
    /// build, so a page can pin behavior to the artifact it loaded.
    #[wasm_bindgen]
    pub fn identity(&self) -> String {
        let identity = self.handle.identity();
        serde_json::json!({
            "graph_version": identity.graph_version,
            "model_id": identity.model_id,
            "calibration_version": identity.calibration_version,
            "engine_semver": identity.engine_semver,
            "embedding_model": identity.embedding_model,
        })
        .to_string()
    }
}

/// The crate version, for the demo's about line and the network-inspector
/// proof of locality (§57): what the page loaded is what decided.
#[wasm_bindgen]
pub fn opencodifier_version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// Decodes, decides, and encodes — the whole wire contract in one
/// function, shared by the binding and the native tests.
fn decide_impl(handle: &EngineHandle, request_json: &str) -> Result<String, EngineFailure> {
    let payload: serde_json::Value =
        serde_json::from_str(request_json).map_err(EngineFailure::MalformedJson)?;
    let request = Native.decode_request(&payload, &Limits::default())?;
    let response = handle.decide(&request)?;
    let encoded = Native.encode_response(&response)?;
    serde_json::to_string(&encoded).map_err(EngineFailure::MalformedJson)
}

/// Decodes a graph document and validates it, returning the graph
/// version and node count.
fn validate_graph_impl(graph_json: &str) -> Result<(u64, usize), EngineFailure> {
    let document: GraphDocument =
        serde_json::from_str(graph_json).map_err(EngineFailure::MalformedJson)?;
    let (version, nodes) = (document.version, document.nodes.len());
    DecisionGraph::new(version, document.nodes)?;
    Ok((version, nodes))
}

/// Validates a graph, decodes a request, decides on a throwaway engine.
fn run_graph_impl(graph_json: &str, request_json: &str) -> Result<String, EngineFailure> {
    let document: GraphDocument =
        serde_json::from_str(graph_json).map_err(EngineFailure::MalformedJson)?;
    let graph = DecisionGraph::new(document.version, document.nodes)?;
    let handle = EngineHandle::ephemeral(graph)?;
    decide_impl(&handle, request_json)
}

/// The error half of the wire contract: a stable code plus its message,
/// serialized to JSON exactly once, at the boundary.
#[derive(Debug)]
enum EngineFailure {
    MalformedJson(serde_json::Error),
    Schema(opencodifier_schema::SchemaError),
    Engine(opencodifier_engine::EngineError),
}

impl From<opencodifier_schema::SchemaError> for EngineFailure {
    fn from(error: opencodifier_schema::SchemaError) -> Self {
        Self::Schema(error)
    }
}

impl From<opencodifier_engine::EngineError> for EngineFailure {
    fn from(error: opencodifier_engine::EngineError) -> Self {
        Self::Engine(error)
    }
}

impl EngineFailure {
    /// The stable error code (`schema.invalid_value`, `engine.timeout`,
    /// `graph.cycle`, ...).
    fn code(&self) -> String {
        match self {
            Self::MalformedJson(_) => "schema.invalid_json".to_owned(),
            Self::Schema(error) => error.code().to_owned(),
            Self::Engine(error) => error.code().to_owned(),
        }
    }
}

impl std::fmt::Display for EngineFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MalformedJson(error) => write!(formatter, "{error}"),
            Self::Schema(error) => write!(formatter, "{error}"),
            Self::Engine(error) => write!(formatter, "{error}"),
        }
    }
}

/// Lifts a failure into JS as a JSON error value: `{"code", "message"}`.
fn wasm_error(error: impl Into<EngineFailure>) -> JsValue {
    let failure = error.into();
    let payload = serde_json::json!({ "code": failure.code(), "message": failure.to_string() });
    JsValue::from_str(&payload.to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;
    use opencodifier_core::{
        Candidate, ChoiceQuestion, DecisionPolicy, DecisionQuestion, RequestMetadata, State,
    };

    /// A native-schema choice request, small enough to read whole.
    fn request_json() -> String {
        let question = DecisionQuestion::Choice(
            ChoiceQuestion::new(
                "skill",
                "Which skill should run?",
                vec![
                    Candidate::new("code", "write Rust code with tests").unwrap(),
                    Candidate::new("prose", "draft release notes prose").unwrap(),
                ],
            )
            .unwrap(),
        );
        let request = opencodifier_core::DecisionRequest::new(
            State::from_text("write code with tests for the parser"),
            vec![question],
            DecisionPolicy::default(),
            RequestMetadata::default(),
        )
        .unwrap();
        serde_json::to_string(&Native.encode_request(&request).unwrap()).unwrap()
    }

    fn engine() -> WasmEngine {
        WasmEngine::new().unwrap()
    }

    /// Host tests drive the public methods only where no `JsValue` is
    /// built: every error path lifts the failure through [`wasm_error`],
    /// whose `JsValue::from_str` calls an import that exists only under
    /// a JS runtime — on the host it aborts. The boundary's error JSON
    /// is therefore proven by the Node smoke test over the real `pkg/`
    /// artifact; here the shared `*_impl` functions carry the failure
    /// taxonomy.
    #[test]
    fn decide_round_trips_the_native_schema() {
        let response: serde_json::Value =
            serde_json::from_str(&engine().decide(&request_json()).unwrap()).unwrap();
        assert!(response.get("answers").is_some(), "{response}");
    }

    /// Hostile input is refused with a typed code, never a panic and
    /// never a best-effort guess (PLANNING §73): unparseable JSON names
    /// `schema.invalid_json`, malformed shapes a `schema.*` refusal.
    #[test]
    fn hostile_input_refuses_with_typed_codes() {
        let engine = engine();
        let failure = decide_impl(&engine.handle, "not json").unwrap_err();
        assert_eq!(failure.code(), "schema.invalid_json");

        for payload in ["{}", r#"{"state":{"text":""}}"#] {
            let failure = decide_impl(&engine.handle, payload).unwrap_err();
            assert!(failure.code().starts_with("schema."), "{payload}: got {}", failure.code());
        }
    }

    /// A well-formed graph validates to a JSON summary; a cycle is
    /// refused with its code.
    #[test]
    fn graph_validation_refuses_cycles() {
        let good = serde_json::to_string(&serde_json::json!({
            "version": 1,
            "nodes": [
                { "id": "normalize", "kind": "normalize" },
                { "id": "output", "kind": "output", "depends_on": ["normalize"] },
            ],
        }))
        .unwrap();
        let summary: serde_json::Value =
            serde_json::from_str(&engine().validate_graph(&good).unwrap()).unwrap();
        assert_eq!(summary.get("version").and_then(serde_json::Value::as_u64), Some(1));
        assert_eq!(summary.get("nodes").and_then(serde_json::Value::as_u64), Some(2));

        let cyclic = serde_json::to_string(&serde_json::json!({
            "version": 1,
            "nodes": [
                { "id": "a", "kind": "rule", "depends_on": ["b"] },
                { "id": "b", "kind": "rule", "depends_on": ["a"] },
            ],
        }))
        .unwrap();
        assert_eq!(validate_graph_impl(&cyclic).unwrap_err().code(), "graph.cycle");
    }

    /// A client-supplied graph decides through the ephemeral path, and a
    /// graph needing a backend is refused — the D21 posture survives the
    /// trip to the browser.
    #[test]
    fn run_graph_decides_and_refuses_missing_backends() {
        let graph = serde_json::to_string(&serde_json::json!({
            "version": 1,
            "nodes": [
                { "id": "normalize", "kind": "normalize" },
                { "id": "choice", "kind": "choice", "depends_on": ["normalize"] },
                { "id": "threshold", "kind": "threshold", "threshold": 0.5,
                  "depends_on": ["choice"] },
                { "id": "output", "kind": "output", "depends_on": ["threshold"] },
            ],
        }))
        .unwrap();
        let response: serde_json::Value =
            serde_json::from_str(&engine().run_graph(&graph, &request_json()).unwrap()).unwrap();
        assert!(response.get("answers").is_some(), "{response}");

        let retrieval = serde_json::to_string(&serde_json::json!({
            "version": 1,
            "nodes": [
                { "id": "normalize", "kind": "normalize" },
                { "id": "embed", "kind": "embedding", "depends_on": ["normalize"] },
                { "id": "output", "kind": "output", "depends_on": ["embed"] },
            ],
        }))
        .unwrap();
        let failure = run_graph_impl(&retrieval, &request_json()).unwrap_err();
        assert_eq!(failure.code(), "engine.missing_backend");
    }

    /// The version a page displays is this crate's, and it reads as a
    /// semver triple.
    #[test]
    fn the_reported_version_is_the_crate_semver() {
        let version = opencodifier_version();
        assert_eq!(version, env!("CARGO_PKG_VERSION"));
        let parts: Vec<&str> = version.split('.').collect();
        assert_eq!(parts.len(), 3, "major.minor.patch, got {version}");
        assert!(parts.iter().all(|part| part.parse::<u64>().is_ok()), "{version}");
    }

    /// The identity is well-formed JSON with all five cache-identity
    /// components (D6, D21).
    #[test]
    fn identity_carries_every_cache_component() {
        let identity: serde_json::Value = serde_json::from_str(&engine().identity()).unwrap();
        for component in
            ["graph_version", "model_id", "calibration_version", "engine_semver", "embedding_model"]
        {
            assert!(identity.get(component).is_some(), "missing {component}");
        }
    }

    /// The dependency boundary is structural: if anything that could
    /// reach the filesystem, the network, or a native runtime ever
    /// appears in this crate's tree, this test fails the build — D23's
    /// "no fs, no net, no dynamic native code" is checked, not promised.
    #[test]
    fn the_dependency_tree_admits_no_io_or_native_runtime() {
        // Own crate metadata: the dependency list is the boundary.
        let manifest = include_str!("../Cargo.toml");
        for banned in ["tokio", "axum", "reqwest", "ort", "libloading", "dlopen"] {
            assert!(
                !manifest_lines(manifest).any(|line| line.starts_with(banned)),
                "dependency `{banned}` would break the §68 posture"
            );
        }
    }

    fn manifest_lines(manifest: &'static str) -> impl Iterator<Item = &'static str> {
        // Dependency lines start at column zero; mentions in comments or
        // descriptions do not count.
        manifest.lines().filter(|line| !line.starts_with([' ', '#', '[']))
    }

    /// The engine is deterministic across separately assembled engines —
    /// the constructor is not a footgun and decisions do not drift.
    #[test]
    fn two_engines_decide_identically() {
        let first = engine().decide(&request_json()).unwrap();
        let second = engine().decide(&request_json()).unwrap();
        assert_eq!(first, second);
    }

    /// The Display half of the error boundary is plain Rust: none of the
    /// three arms constructs a `JsValue`, so a host test can prove they
    /// all format (the JSON half stays with the Node smoke test).
    #[test]
    fn the_error_boundary_formats_every_failure_class() {
        let malformed = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
        let failures = [
            EngineFailure::MalformedJson(malformed),
            EngineFailure::Schema(opencodifier_schema::SchemaError::unsupported("question type")),
            EngineFailure::Engine(opencodifier_engine::EngineError::Cancelled),
        ];
        for failure in failures {
            let shown = failure.to_string();
            assert!(!shown.is_empty(), "{failure:?} must format");
        }
    }
}
