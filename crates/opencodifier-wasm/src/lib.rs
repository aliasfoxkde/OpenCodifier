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

use opencodifier_engine::{
    EngineConfig, EngineHandle,
    wire::{
        WireFailure as EngineFailure, decide_batch_impl, decide_impl, run_graph_impl,
        validate_graph_impl,
    },
};

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

    /// Decides a batch of native-schema requests in one boundary
    /// crossing (§11): `{"requests": [ ... ]}` in, one decision each,
    /// `{"results": [...], "count": N}` out.
    ///
    /// The envelope is the `POST /v1/batch` body — the same `requests`
    /// key MCP `codify_batch` uses — and its items are native-schema
    /// request objects (what [`Self::decide`] takes), not pre-stringified
    /// payloads. Every request is decided here, in wasm, in input order,
    /// on this engine: one crossing for the whole batch, and one cache
    /// across it, so a request repeated inside a batch is a cache hit and
    /// a page's warmed decisions stay warm.
    ///
    /// Each entry of `results` is that item's `{"response": ...}` or,
    /// when the item's decode or execution failed, its
    /// `{"error": {"code", "message"}}` with the same codes
    /// [`Self::decide`] reports. One item's refusal is that item's
    /// result, never a failure of the batch.
    ///
    /// # Errors
    ///
    /// A JSON error value for a batch that is unusable as a batch:
    /// unparseable JSON (`schema.invalid_json`), an envelope without a
    /// `requests` array (`schema.invalid_value`), or more requests than
    /// [`opencodifier_engine::MAX_BATCH`] (`schema.limit_exceeded`) —
    /// the ceiling the HTTP
    /// batch enforces, so a page cannot enqueue what the runtime would
    /// refuse.
    #[wasm_bindgen]
    pub fn decide_batch(&self, requests_json: &str) -> Result<String, JsValue> {
        decide_batch_impl(&self.handle, requests_json).map_err(wasm_error)
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

/// Lifts a failure into JS as a JSON error value: `{"code", "message"}`.
fn wasm_error(error: impl Into<EngineFailure>) -> JsValue {
    JsValue::from_str(&error.into().error_json().to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;
    use opencodifier_core::{
        Candidate, ChoiceQuestion, DecisionPolicy, DecisionQuestion, RequestMetadata, State,
    };
    use opencodifier_engine::{MAX_BATCH, wire::decide_value};
    use opencodifier_schema::{WireFormat, native::Native};

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

    /// The choice request of [`request_json`], as a value — what a batch
    /// item is, since the envelope carries objects, not strings.
    fn request_value() -> serde_json::Value {
        serde_json::from_str(&request_json()).unwrap()
    }

    /// A second request, different in kind, so a batch can be proven to
    /// answer in order rather than to repeat one answer.
    fn boolean_request_value() -> serde_json::Value {
        let request = opencodifier_core::DecisionRequest::new(
            State::from_text("the parser needs tests before the release"),
            vec![DecisionQuestion::Boolean(
                opencodifier_core::BooleanQuestion::new("needs_tests", "Does this need tests?")
                    .unwrap(),
            )],
            DecisionPolicy::default(),
            RequestMetadata::default(),
        )
        .unwrap();
        Native.encode_request(&request).unwrap()
    }

    /// The response as a client receives it: serialized, then parsed
    /// again. The canonical form the conformance fixtures are compared
    /// in — both sides cross the same wire, so neither side's in-memory
    /// float bit is expected to survive (see the conformance test).
    fn wire_canonical(response: &serde_json::Value) -> serde_json::Value {
        serde_json::from_str(&serde_json::to_string(response).unwrap()).unwrap()
    }

    /// Whether two floats sit at most one ulp apart, negatives ordered so
    /// the bit patterns stay monotonic. The zero-ML stack is deterministic
    /// within a build, but not bit-identical across build targets: the
    /// same BM25 fold over the same query lands one ulp apart on x86-64
    /// and on wasm32 (measured, not assumed). The conformance fixtures pin
    /// the wire decision and let the last bit of a float move.
    fn within_one_ulp(left: f64, right: f64) -> bool {
        let ordered = |bits: u64| if bits >> 63 == 1 { !bits } else { bits | (1 << 63) };
        let (left, right) = (ordered(left.to_bits()), ordered(right.to_bits()));
        left.abs_diff(right) <= 1
    }

    /// The first place two wire documents disagree, or `None` when they
    /// agree canonically: same structure, same text, same integers, and
    /// float leaves within one ulp of each other. Both sides have already
    /// been through [`wire_canonical`], so this compares what a client
    /// receives, never an in-memory representation.
    fn canonical_difference(
        actual: &serde_json::Value,
        expected: &serde_json::Value,
        path: &str,
    ) -> Option<String> {
        use serde_json::Value;
        match (actual, expected) {
            (Value::Number(actual), Value::Number(expected)) => {
                let (actual, expected) =
                    (actual.as_f64().unwrap_or(f64::NAN), expected.as_f64().unwrap_or(f64::NAN));
                (actual != expected && !within_one_ulp(actual, expected))
                    .then(|| format!("{path}: {actual} vs {expected}"))
            }
            (Value::String(actual), Value::String(expected)) => {
                (actual != expected).then(|| format!("{path}: {actual:?} vs {expected:?}"))
            }
            (Value::Bool(actual), Value::Bool(expected)) => {
                (actual != expected).then(|| format!("{path}: {actual} vs {expected}"))
            }
            (Value::Null, Value::Null) => None,
            (Value::Array(actual), Value::Array(expected)) => {
                if actual.len() != expected.len() {
                    return Some(format!("{path}: {} entries vs {}", actual.len(), expected.len()));
                }
                actual.iter().zip(expected).enumerate().find_map(|(index, (actual, expected))| {
                    canonical_difference(actual, expected, &format!("{path}[{index}]"))
                })
            }
            (Value::Object(actual), Value::Object(expected)) => actual
                .iter()
                .find_map(|(key, actual)| match expected.get(key) {
                    Some(expected) => {
                        canonical_difference(actual, expected, &format!("{path}.{key}"))
                    }
                    None => Some(format!("{path}.{key}: unexpected")),
                })
                .or_else(|| {
                    expected.iter().find_map(|(key, _)| {
                        (!actual.contains_key(key)).then(|| format!("{path}.{key}: missing"))
                    })
                }),
            _ => Some(format!("{path}: {actual} vs {expected}")),
        }
    }

    /// Wraps items in the batch envelope: the `POST /v1/batch` body.
    fn envelope(items: &[serde_json::Value]) -> String {
        serde_json::json!({ "requests": items }).to_string()
    }

    /// A batch answers every request in input order, each in its own
    /// `response` slot, with a `count` that matches what went in (§11:
    /// one crossing, not one crossing per request).
    ///
    /// The references come from fresh engines: a shared engine would have
    /// the batch's own cache state in its traces, and the assertion is
    /// about ordering, not cache warmth (that is the next test's).
    #[test]
    fn batch_answers_every_request_in_order() {
        let choice = decide_impl(&engine().handle, &request_json()).unwrap();
        let alone = decide_impl(
            &engine().handle,
            &serde_json::to_string(&boolean_request_value()).unwrap(),
        )
        .unwrap();

        let batched: serde_json::Value = serde_json::from_str(
            &decide_batch_impl(
                &engine().handle,
                &envelope(&[request_value(), boolean_request_value()]),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(batched.get("count"), Some(&serde_json::json!(2)), "{batched}");
        let results = batched.get("results").and_then(serde_json::Value::as_array).unwrap();
        assert_eq!(results.len(), 2, "{batched}");
        assert_eq!(
            results[0].get("response"),
            Some(&serde_json::from_str::<serde_json::Value>(&choice).unwrap()),
            "first in, first answered"
        );
        assert_eq!(
            results[1].get("response"),
            Some(&serde_json::from_str::<serde_json::Value>(&alone).unwrap()),
            "second in, second answered"
        );
    }

    /// One item's refusal is that item's result: a batch of one good and
    /// one malformed request still answers the good one, and the bad slot
    /// carries the same typed code `decide` would have thrown.
    #[test]
    fn batch_isolates_a_failing_request() {
        let engine = engine();
        let batched: serde_json::Value = serde_json::from_str(
            &decide_batch_impl(
                &engine.handle,
                &envelope(&[request_value(), serde_json::json!({ "state": { "text": "" } })]),
            )
            .unwrap(),
        )
        .unwrap();
        let results = batched.get("results").and_then(serde_json::Value::as_array).unwrap();
        assert_eq!(results.len(), 2, "{batched}");
        assert!(results[0].get("response").is_some(), "the good item answers: {batched}");
        let error =
            results[1].get("error").unwrap_or_else(|| panic!("bad item reports: {batched}"));
        assert!(
            error
                .get("code")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .starts_with("schema."),
            "{error}"
        );
        assert!(
            !error
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .is_empty(),
            "{error}"
        );
    }

    /// An empty batch is a successful batch of zero decisions, not an
    /// error: `count` says so.
    #[test]
    fn an_empty_batch_answers_zero_results() {
        let batched: serde_json::Value =
            serde_json::from_str(&decide_batch_impl(&engine().handle, &envelope(&[])).unwrap())
                .unwrap();
        assert_eq!(batched, serde_json::json!({ "results": [], "count": 0 }), "{batched}");
    }

    /// The batch ceiling is the HTTP batch's own [`MAX_BATCH`]: one more
    /// request than that is refused up front, before any item is decoded,
    /// with `schema.limit_exceeded`.
    #[test]
    fn an_oversized_batch_is_refused_up_front() {
        let failure =
            decide_batch_impl(&engine().handle, &envelope(&vec![request_value(); MAX_BATCH + 1]))
                .unwrap_err();
        assert_eq!(failure.code(), "schema.limit_exceeded", "{failure}");
    }

    /// A batch that is not a batch — unparseable JSON, an envelope
    /// without `requests`, a `requests` that is not a sequence — is a
    /// controlled refusal, never a panic.
    #[test]
    fn a_malformed_envelope_refuses_with_typed_codes() {
        for (payload, code) in [
            ("not json", "schema.invalid_json"),
            ("{}", "schema.invalid_value"),
            (r#"{"requests": 5}"#, "schema.invalid_value"),
            (r#"{"requests": null}"#, "schema.invalid_value"),
        ] {
            let failure = decide_batch_impl(&engine().handle, payload).unwrap_err();
            assert_eq!(failure.code(), code, "{payload}: got {}", failure.code());
        }
    }

    /// The batch shares the one engine, so its cache is the page's: a
    /// request repeated inside a batch is answered from the cache on its
    /// second appearance (`metrics.cache_hit`), the same as it would be
    /// across two `decide` calls.
    #[test]
    fn a_batch_shares_the_engine_cache() {
        let engine = engine();
        let batched: serde_json::Value = serde_json::from_str(
            &decide_batch_impl(&engine.handle, &envelope(&[request_value(), request_value()]))
                .unwrap(),
        )
        .unwrap();
        let results = batched.get("results").and_then(serde_json::Value::as_array).unwrap();
        let metrics = |slot: usize| results[slot].get("response").unwrap().get("metrics").cloned();
        assert_eq!(
            metrics(0).and_then(|m| m.get("cache_hit").cloned()),
            Some(serde_json::json!(false)),
            "the first appearance decides"
        );
        assert_eq!(
            metrics(1).and_then(|m| m.get("cache_hit").cloned()),
            Some(serde_json::json!(true)),
            "the second appearance hits the cache"
        );
    }

    // §34: the conformance fixtures under `fixtures/conformance/` are the
    // record that native and wasm are one engine on two runtimes. The
    // native half of the contract lives here, over the same `*_impl`
    // functions the binding calls; the wasm half runs the identical files
    // in `tests/node/smoke.cjs`.

    /// A request fixture: its path names the behavior it locks.
    const CHOICE_FIXTURE: &str =
        include_str!("../../../fixtures/conformance/choice-lexical-accept.json");
    /// The relational fixture: a proof, not a score — mass 1.0 on the
    /// proven answer.
    const RELATIONAL_FIXTURE: &str =
        include_str!("../../../fixtures/conformance/relational-proof.json");
    /// The abstention fixture: no evidence, four candidates, no answer.
    const ABSTAIN_FIXTURE: &str =
        include_str!("../../../fixtures/conformance/abstain-no-evidence.json");
    /// The malformed-request fixture: a typed refusal, not a panic.
    const INVALID_REQUEST_FIXTURE: &str =
        include_str!("../../../fixtures/conformance/invalid-request.json");
    /// The malformed-graph fixture: a cycle the constructor refuses.
    const INVALID_GRAPH_FIXTURE: &str =
        include_str!("../../../fixtures/conformance/invalid-graph.json");

    /// The native engine reproduces every recorded expectation.
    ///
    /// The comparison is **canonical, never lexical**: both sides meet as
    /// `serde_json::Value` after the *same* JSON round-trip — the engine's
    /// response is serialized and re-parsed exactly as a client receives
    /// it, the fixture is parsed exactly as it ships — so a fixture's
    /// indentation and key order are free while its structure, text, and
    /// integers are exact. Float leaves are allowed one ulp (see
    /// [`within_one_ulp`]), because the same Rust source does not produce
    /// bit-identical floats on every target, and this crate exists to be
    /// built for two of them. Refusals are compared through the same
    /// object the boundary reports, `{"code", "message"}`.
    ///
    /// Regeneration: with `OPENCODIFIER_UPDATE_CONFORMANCE` set, a
    /// mismatching decision fixture is re-recorded from the live engine
    /// instead of failing. Cache keys embed `engine_semver`
    /// (`cache.rs`), so a release bump re-keys every recorded decision —
    /// re-record, then inspect the diff: the answers, confidence, and
    /// outcome must be untouched (a version bump re-keys, it does not
    /// re-decide). The Node smoke over `tests/node/smoke.cjs` judges the
    /// regenerated fixtures on the wasm side.
    #[test]
    fn conformance_fixtures_hold_on_the_native_engine() {
        let engine = engine();
        let update = std::env::var("OPENCODIFIER_UPDATE_CONFORMANCE").is_ok();
        for (name, document) in [
            ("choice-lexical-accept", CHOICE_FIXTURE),
            ("relational-proof", RELATIONAL_FIXTURE),
            ("abstain-no-evidence", ABSTAIN_FIXTURE),
            ("invalid-request", INVALID_REQUEST_FIXTURE),
        ] {
            let document: serde_json::Value = serde_json::from_str(document).unwrap();
            let expected = document.get("expected").cloned().unwrap();
            let request = document.get("request").cloned().unwrap();
            let actual = decide_value(&engine.handle, &request).map(|value| wire_canonical(&value));
            match actual {
                Ok(response) => {
                    let difference = canonical_difference(&response, &expected, name);
                    if let (true, true) = (update, difference.is_some()) {
                        let path = format!(
                            "{}/../../fixtures/conformance/{name}.json",
                            env!("CARGO_MANIFEST_DIR")
                        );
                        let updated =
                            serde_json::json!({ "request": request, "expected": response });
                        std::fs::write(
                            &path,
                            serde_json::to_string_pretty(&updated).unwrap() + "\n",
                        )
                        .unwrap();
                        continue;
                    }
                    assert!(
                        difference.is_none(),
                        "{name}: the engine no longer matches its recorded decision: \
                         {difference:?}"
                    );
                }
                Err(failure) => assert_eq!(
                    failure.error_json(),
                    expected,
                    "{name}: the engine no longer refuses as recorded"
                ),
            }
        }
    }

    /// The graph fixture is refused at construction, exactly as a page's
    /// `validate_graph` refuses it — same code, same message.
    #[test]
    fn the_graph_conformance_fixture_refuses_as_recorded() {
        let document: serde_json::Value = serde_json::from_str(INVALID_GRAPH_FIXTURE).unwrap();
        let expected = document.get("expected").cloned().unwrap();
        let graph = document.get("graph").cloned().unwrap().to_string();
        let failure = validate_graph_impl(&graph).unwrap_err();
        assert_eq!(failure.error_json(), expected, "the cycle is refused as recorded");
    }
}
