//! The native-schema wire boundary shared by the WASM and C ABI
//! surfaces (PLANNING.md §57, §68; D23, D24).
//!
//! The `*_impl` functions are the whole decode → decide → encode
//! contract: batch envelopes and their per-item errors, the diagnostics
//! a malformed envelope names its JSON type with, the ceiling both
//! embedded surfaces enforce, and the [`WireFailure`] taxonomy's
//! `source`/Display/JSON faces.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

mod common;

use std::error::Error as _;

use common::choice_question;
use opencodifier_core::{DecisionPolicy, DecisionRequest, RequestMetadata, State};
use opencodifier_engine::{
    EngineConfig, EngineError, EngineHandle, MAX_BATCH,
    wire::{
        WireFailure, decide_batch_impl, decide_impl, decide_value, run_graph_impl,
        validate_graph_impl,
    },
};
use opencodifier_schema::{SchemaError, WireFormat, native::Native};

/// A handle over the built-in pipeline with the lexical decider — the
/// zero-ML assembly the embedded surfaces ship.
fn handle() -> EngineHandle {
    EngineHandle::lexical(EngineConfig::with_default_pipeline().unwrap()).unwrap()
}

/// A valid native-schema request payload, both as text and as the parsed
/// value `decide_value` takes.
fn request_payload() -> (String, serde_json::Value) {
    let request = DecisionRequest::new(
        State::from_text("write code with tests for the parser"),
        vec![choice_question(
            "skill",
            &[("code", "write Rust code with tests"), ("prose", "draft release notes prose")],
        )],
        DecisionPolicy::default(),
        RequestMetadata::default(),
    )
    .unwrap();
    let text = serde_json::to_string(&Native.encode_request(&request).unwrap()).unwrap();
    let value = serde_json::from_str(&text).unwrap();
    (text, value)
}

/// `decide_impl` round-trips the native schema: request in, decision
/// with answers out — the same trip a browser page or a mobile library
/// makes.
#[test]
fn decide_round_trips_the_native_schema() {
    let (request, _) = request_payload();
    let response: serde_json::Value =
        serde_json::from_str(&decide_impl(&handle(), &request).unwrap()).unwrap();
    assert!(response.get("answers").is_some(), "{response}");
}

/// `decide_value` accepts the already-parsed payload and agrees with the
/// text-shaped `decide_impl` byte for byte — one core, two envelopes,
/// no drift.
#[test]
fn value_and_text_entries_decide_identically() {
    let (request, payload) = request_payload();
    let from_text = decide_impl(&handle(), &request).unwrap();
    let from_value = serde_json::to_string(&decide_value(&handle(), &payload).unwrap()).unwrap();
    assert_eq!(from_text, from_value);
}

/// A batch decides every item in input order, and one refused item is
/// that item's result — never a failure of the batch. Items are
/// native-schema request objects, not pre-stringified payloads.
#[test]
fn batch_decides_in_order_and_isolates_item_errors() {
    let (_, payload) = request_payload();
    let envelope = serde_json::json!({ "requests": [payload, "not a request", payload] });
    let report: serde_json::Value =
        serde_json::from_str(&decide_batch_impl(&handle(), &envelope.to_string()).unwrap())
            .unwrap();
    assert_eq!(report["count"], 3);
    let results = report["results"].as_array().unwrap();
    assert_eq!(results.len(), 3);
    assert!(results[0].get("response").is_some(), "{results:?}");
    assert!(results[2].get("response").is_some(), "{results:?}");
    let error = &results[1]["error"];
    assert!(error["code"].as_str().unwrap().starts_with("schema."), "{error}");
    assert_ne!(error["message"].as_str().unwrap(), "");
}

/// A malformed envelope names the JSON type it actually carried — one
/// case per reachable `serde_json::Value` shape, plus the missing key.
/// (The `Array` arm of the diagnostic is matched by the envelope's own
/// `requests`-is-an-array arm first, so no envelope can reach it.)
#[test]
fn envelope_diagnostics_name_the_offending_json_type() {
    let cases: [(serde_json::Value, &str); 6] = [
        (serde_json::json!({ "requests": null }), "null"),
        (serde_json::json!({ "requests": true }), "boolean"),
        (serde_json::json!({ "requests": 7 }), "number"),
        (serde_json::json!({ "requests": "two" }), "string"),
        (serde_json::json!({ "requests": {} }), "map"),
        (serde_json::json!({ "other": [] }), "missing field `requests`"),
    ];
    for (envelope, expected) in cases {
        let failure = decide_batch_impl(&handle(), &envelope.to_string()).unwrap_err();
        assert_eq!(failure.code(), "schema.invalid_value", "{envelope}");
        assert!(
            failure.to_string().contains(expected),
            "{envelope}: expected `{expected}` in `{failure}`"
        );
    }
}

/// Unparseable JSON and an over-ceiling batch are refused before any
/// decision runs, with the codes the HTTP batch enforces too.
#[test]
fn batch_refuses_unparseable_and_oversized_envelopes() {
    let failure = decide_batch_impl(&handle(), "not json").unwrap_err();
    assert_eq!(failure.code(), "schema.invalid_json");

    let requests: Vec<String> = std::iter::repeat_n(request_payload().0, MAX_BATCH + 1).collect();
    let envelope = serde_json::json!({ "requests": requests });
    let failure = decide_batch_impl(&handle(), &envelope.to_string()).unwrap_err();
    assert_eq!(failure.code(), "schema.limit_exceeded");
}

/// `validate_graph_impl` returns the shape a client confirms before it
/// runs: the version and the node count of the graph it sent.
#[test]
fn graph_validation_reports_version_and_node_count() {
    let graph = serde_json::json!({
        "version": 3,
        "nodes": [
            { "id": "normalize", "kind": "normalize" },
            { "id": "output", "kind": "output", "depends_on": ["normalize"] },
        ],
    });
    let (version, nodes) = validate_graph_impl(&graph.to_string()).unwrap();
    assert_eq!((version, nodes), (3, 2));
}

/// Every [`WireFailure`] variant exposes its cause through
/// `Error::source`, downcastable to the layer that produced it — so a
/// client (or a test) can distinguish "not JSON" from "not a valid
/// request" from "the engine refused" without string matching.
#[test]
fn wire_failures_expose_their_source_error() {
    let (request, _) = request_payload();
    let engine_handle = handle();

    let malformed = decide_impl(&engine_handle, "not json").unwrap_err();
    let source = malformed.source().expect("MalformedJson carries its source");
    assert!(source.downcast_ref::<serde_json::Error>().is_some());

    let schema = decide_impl(&engine_handle, "{}").unwrap_err();
    let source = schema.source().expect("Schema carries its source");
    assert!(source.downcast_ref::<SchemaError>().is_some());

    let cyclic = serde_json::json!({
        "version": 1,
        "nodes": [
            { "id": "a", "kind": "rule", "depends_on": ["b"] },
            { "id": "b", "kind": "rule", "depends_on": ["a"] },
        ],
    });
    let engine_failure = run_graph_impl(&cyclic.to_string(), &request).unwrap_err();
    assert_eq!(engine_failure.code(), "graph.cycle");
    let source = engine_failure.source().expect("Engine carries its source");
    assert!(source.downcast_ref::<EngineError>().is_some());
}

/// The error JSON a batch item reports is the code plus the Display
/// message, built once — the same object a client reads on any
/// transport.
#[test]
fn error_json_carries_the_code_and_display_message() {
    let failure = WireFailure::MalformedJson(
        serde_json::from_str::<serde_json::Value>("not json").unwrap_err(),
    );
    let error = failure.error_json();
    assert_eq!(error["code"], "schema.invalid_json");
    assert_eq!(error["message"], failure.to_string());
}
