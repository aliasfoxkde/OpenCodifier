//! The C ABI driven the way a C caller drives it: every test goes through
//! `extern "C"` declarations matching `include/ocffi.h` — the same
//! signatures, the same pointer discipline — never through Rust-level
//! shortcuts. This file is also the header-sync check: a signature that
//! changes on one side only stops compiling here.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::ffi::{CStr, CString, c_char};
use std::sync::Arc;

use opencodifier_core::{
    Candidate, ChoiceQuestion, DecisionPolicy, DecisionQuestion, RequestMetadata, State,
};
use opencodifier_engine::{EngineHandle, MAX_BATCH};
use opencodifier_schema::{WireFormat, native::Native};

unsafe extern "C" {
    fn oc_version() -> *const c_char;
    fn oc_engine_lexical_new() -> *mut opencodifier_ffi::OcEngine;
    fn oc_engine_destroy(engine: *mut opencodifier_ffi::OcEngine);
    fn oc_decide(
        engine: *const opencodifier_ffi::OcEngine,
        request_json: *const c_char,
    ) -> *mut c_char;
    fn oc_decide_batch(
        engine: *const opencodifier_ffi::OcEngine,
        requests_json: *const c_char,
    ) -> *mut c_char;
    fn oc_validate_graph(graph_json: *const c_char) -> *mut c_char;
    fn oc_run_graph(graph_json: *const c_char, request_json: *const c_char) -> *mut c_char;
    fn oc_identity(engine: *const opencodifier_ffi::OcEngine) -> *mut c_char;
    fn oc_last_error() -> *const c_char;
    fn oc_string_free(string: *mut c_char);
}

/// A native-schema choice request, small enough to read whole — the same
/// shape every other surface's tests pin.
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

/// The batch envelope: the `POST /v1/batch` body.
fn envelope(items: &[serde_json::Value]) -> String {
    serde_json::json!({ "requests": items }).to_string()
}

/// Owned-string round trip: C hands back a `char *`, the test frees it
/// like a C caller would, and what was read before the free is what the
/// caller got.
fn take(pointer: *mut c_char) -> String {
    assert!(!pointer.is_null(), "expected a string, got NULL");
    let text = unsafe { CStr::from_ptr(pointer) }.to_string_lossy().into_owned();
    unsafe { oc_string_free(pointer) };
    text
}

/// The last error as a JSON value: the refusal document a C caller
/// would parse.
fn last_error() -> serde_json::Value {
    let pointer = unsafe { oc_last_error() };
    assert!(!pointer.is_null(), "oc_last_error is never NULL");
    let text = unsafe { CStr::from_ptr(pointer) }.to_string_lossy().into_owned();
    serde_json::from_str(&text)
        .unwrap_or_else(|_| panic!("last error is not the documented JSON document: {text}"))
}

/// The engine handle shared across threads: the raw pointer travels, the
/// engine behind it is `Send + Sync` (asserted below), so sharing is the
/// documented posture, not a lone-pointer liberty.
struct Share(*const opencodifier_ffi::OcEngine);
// Safety: the engine handle is `Send + Sync` (see
// `engine_handle_is_send_sync`); the pointer is only ever used through
// the ABI's own entry points, which take it by shared reference.
unsafe impl Send for Share {}
unsafe impl Sync for Share {}

#[test]
fn engine_handle_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<EngineHandle>();
}

#[test]
fn the_reported_version_is_this_crate_semver() {
    // `oc_version` is a static, not caller-owned — reading it without
    // freeing is precisely the contract `take` exists to violate.
    let pointer = unsafe { oc_version() };
    assert!(!pointer.is_null());
    let version = unsafe { CStr::from_ptr(pointer) }.to_string_lossy().into_owned();
    assert_eq!(version, env!("CARGO_PKG_VERSION"));
}

#[test]
fn decide_round_trips_the_native_schema() {
    let engine = unsafe { oc_engine_lexical_new() };
    assert!(!engine.is_null());
    let request = CString::new(request_json()).unwrap();
    let response = take(unsafe { oc_decide(engine, request.as_ptr()) });
    let document: serde_json::Value = serde_json::from_str(&response).unwrap();
    assert!(document.get("answers").is_some(), "{document}");
    unsafe { oc_engine_destroy(engine) };
}

#[test]
fn two_engines_decide_identically() {
    let request = CString::new(request_json()).unwrap();
    let first_engine = unsafe { oc_engine_lexical_new() };
    let second_engine = unsafe { oc_engine_lexical_new() };
    let first = take(unsafe { oc_decide(first_engine, request.as_ptr()) });
    let second = take(unsafe { oc_decide(second_engine, request.as_ptr()) });
    assert_eq!(first, second, "separately assembled engines must not drift");
    unsafe { oc_engine_destroy(first_engine) };
    unsafe { oc_engine_destroy(second_engine) };
}

#[test]
fn hostile_input_refuses_with_typed_codes() {
    let engine = unsafe { oc_engine_lexical_new() };
    let not_json = CString::new("not json").unwrap();
    assert!(unsafe { oc_decide(engine, not_json.as_ptr()) }.is_null());
    assert_eq!(
        last_error().get("code").and_then(serde_json::Value::as_str),
        Some("schema.invalid_json"),
        "{}",
        last_error()
    );

    let empty_state = CString::new(r#"{"state":{"text":""}}"#).unwrap();
    assert!(unsafe { oc_decide(engine, empty_state.as_ptr()) }.is_null());
    let failure = last_error();
    let code = failure.get("code").and_then(serde_json::Value::as_str).unwrap();
    assert!(code.starts_with("schema."), "got {code}");
    unsafe { oc_engine_destroy(engine) };
}

#[test]
fn null_arguments_are_refused_not_dereferenced() {
    let engine = unsafe { oc_engine_lexical_new() };
    let request = CString::new(request_json()).unwrap();

    assert!(unsafe { oc_decide(std::ptr::null(), request.as_ptr()) }.is_null());
    assert_eq!(
        last_error().get("code").and_then(serde_json::Value::as_str),
        Some("ffi.invalid_argument"),
        "{}",
        last_error()
    );

    assert!(unsafe { oc_decide(engine, std::ptr::null()) }.is_null());
    assert_eq!(
        last_error().get("code").and_then(serde_json::Value::as_str),
        Some("ffi.invalid_argument"),
        "{}",
        last_error()
    );
    unsafe { oc_engine_destroy(engine) };
}

#[test]
fn a_success_leaves_the_previous_error_readable() {
    let engine = unsafe { oc_engine_lexical_new() };
    let bad = CString::new("not json").unwrap();
    assert!(unsafe { oc_decide(engine, bad.as_ptr()) }.is_null());
    let recorded = last_error();

    let request = CString::new(request_json()).unwrap();
    assert!(!unsafe { oc_decide(engine, request.as_ptr()) }.is_null());
    // Successes do not clear the last error: it reports the most recent
    // failure, per the header contract.
    assert_eq!(last_error(), recorded, "a success must not rewrite the error");
    unsafe { oc_engine_destroy(engine) };
}

#[test]
fn a_batch_answers_in_order_and_shares_the_cache() {
    let engine = unsafe { oc_engine_lexical_new() };
    let payload = serde_json::from_str::<serde_json::Value>(&request_json()).unwrap();
    let response = take(unsafe {
        oc_decide_batch(
            engine,
            CString::new(envelope(&[payload.clone(), payload])).unwrap().as_ptr(),
        )
    });
    let batched: serde_json::Value = serde_json::from_str(&response).unwrap();
    assert_eq!(batched.get("count"), Some(&serde_json::json!(2)), "{batched}");
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
        "the second appearance hits the one cache"
    );
    unsafe { oc_engine_destroy(engine) };
}

#[test]
fn an_oversized_batch_is_refused_up_front() {
    let engine = unsafe { oc_engine_lexical_new() };
    let payload = serde_json::from_str::<serde_json::Value>(&request_json()).unwrap();
    let oversized = envelope(&vec![payload; MAX_BATCH + 1]);
    assert!(
        unsafe { oc_decide_batch(engine, CString::new(oversized).unwrap().as_ptr()) }.is_null()
    );
    assert_eq!(
        last_error().get("code").and_then(serde_json::Value::as_str),
        Some("schema.limit_exceeded"),
        "{}",
        last_error()
    );
    unsafe { oc_engine_destroy(engine) };
}

#[test]
fn graph_validation_refuses_cycles() {
    let good = CString::new(
        serde_json::json!({
            "version": 1,
            "nodes": [
                { "id": "normalize", "kind": "normalize" },
                { "id": "output", "kind": "output", "depends_on": ["normalize"] },
            ],
        })
        .to_string(),
    )
    .unwrap();
    let summary = take(unsafe { oc_validate_graph(good.as_ptr()) });
    let document: serde_json::Value = serde_json::from_str(&summary).unwrap();
    assert_eq!(document.get("version").and_then(serde_json::Value::as_u64), Some(1));
    assert_eq!(document.get("nodes").and_then(serde_json::Value::as_u64), Some(2));

    let cyclic = CString::new(
        serde_json::json!({
            "version": 1,
            "nodes": [
                { "id": "a", "kind": "rule", "depends_on": ["b"] },
                { "id": "b", "kind": "rule", "depends_on": ["a"] },
            ],
        })
        .to_string(),
    )
    .unwrap();
    assert!(unsafe { oc_validate_graph(cyclic.as_ptr()) }.is_null());
    assert_eq!(
        last_error().get("code").and_then(serde_json::Value::as_str),
        Some("graph.cycle"),
        "{}",
        last_error()
    );
}

#[test]
fn run_graph_decides_and_refuses_missing_backends() {
    let engine = unsafe { oc_engine_lexical_new() };
    let graph = CString::new(
        serde_json::json!({
            "version": 1,
            "nodes": [
                { "id": "normalize", "kind": "normalize" },
                { "id": "choice", "kind": "choice", "depends_on": ["normalize"] },
                { "id": "threshold", "kind": "threshold", "threshold": 0.5,
                  "depends_on": ["choice"] },
                { "id": "output", "kind": "output", "depends_on": ["threshold"] },
            ],
        })
        .to_string(),
    )
    .unwrap();
    let request = CString::new(request_json()).unwrap();
    let response = take(unsafe { oc_run_graph(graph.as_ptr(), request.as_ptr()) });
    let document: serde_json::Value = serde_json::from_str(&response).unwrap();
    assert!(document.get("answers").is_some(), "{document}");

    let retrieval = CString::new(
        serde_json::json!({
            "version": 1,
            "nodes": [
                { "id": "normalize", "kind": "normalize" },
                { "id": "embed", "kind": "embedding", "depends_on": ["normalize"] },
                { "id": "output", "kind": "output", "depends_on": ["embed"] },
            ],
        })
        .to_string(),
    )
    .unwrap();
    assert!(unsafe { oc_run_graph(retrieval.as_ptr(), request.as_ptr()) }.is_null());
    assert_eq!(
        last_error().get("code").and_then(serde_json::Value::as_str),
        Some("engine.missing_backend"),
        "{}",
        last_error()
    );
    unsafe { oc_engine_destroy(engine) };
}

#[test]
fn identity_carries_every_cache_component() {
    let engine = unsafe { oc_engine_lexical_new() };
    let identity = take(unsafe { oc_identity(engine) });
    let document: serde_json::Value = serde_json::from_str(&identity).unwrap();
    for component in
        ["graph_version", "model_id", "calibration_version", "engine_semver", "embedding_model"]
    {
        assert!(document.get(component).is_some(), "missing {component}: {document}");
    }
    unsafe { oc_engine_destroy(engine) };
}

#[test]
fn one_engine_decides_identically_across_threads() {
    let engine = unsafe { oc_engine_lexical_new() };
    let shared = Arc::new(Share(engine));
    // `CString` is `Send`, so the payload crosses into each thread; the
    // raw engine pointer travels inside `Share`.
    let request = Arc::new(CString::new(request_json()).unwrap());

    let handles: Vec<_> = (0..4)
        .map(|_| {
            let shared = Arc::clone(&shared);
            let request = Arc::clone(&request);
            std::thread::spawn(move || take(unsafe { oc_decide(shared.0, request.as_ptr()) }))
        })
        .collect();
    let responses: Vec<String> = handles.into_iter().map(|handle| handle.join().unwrap()).collect();
    // The *decisions* must not depend on interleaving; `metrics.cache_hit`
    // and the trace may, because four identical requests race one shared
    // cache and exactly one of them is the miss that fills it.
    let decisions: Vec<String> = responses
        .iter()
        .map(|response| {
            serde_json::to_string(
                &serde_json::from_str::<serde_json::Value>(response)
                    .unwrap()
                    .get("answers")
                    .cloned()
                    .unwrap_or_else(|| panic!("no answers in {response}")),
            )
            .unwrap()
        })
        .collect();
    assert!(
        decisions.windows(2).all(|pair| pair[0] == pair[1]),
        "shared-engine decisions must be deterministic: {decisions:?}"
    );
    unsafe { oc_engine_destroy(engine) };
}

#[test]
fn null_handles_are_no_ops() {
    unsafe {
        oc_engine_destroy(std::ptr::null_mut());
        oc_string_free(std::ptr::null_mut());
    }
}
