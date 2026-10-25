//! Escalation wiring (#189 slice 2): when the calibrated confidence
//! lands in the escalation band (`abstain_below <= c < escalate_below`),
//! the response carries a built [`EscalationReport`]; absent the band,
//! nothing changes and no report appears. The report is a pure function
//! of the decision, so a cache hit reattaches the same report.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

mod common;

use std::sync::Arc;

use opencodifier_core::{DecisionOutcome, DecisionPolicy, DecisionRequest, RequestMetadata};
use opencodifier_engine::{EngineConfig, EngineHandle, MockClassifier};
use opencodifier_schema::{Native, WireFormat};

use common::{choice_question, request_with_policy, state_with_fact};

fn engine_with(top: (&'static str, f64)) -> EngineHandle {
    let config = EngineConfig::with_default_pipeline().unwrap();
    EngineHandle::new(config, scripted(top), None).unwrap()
}

fn candidates() -> Vec<(&'static str, &'static str)> {
    vec![
        ("alpha", "database indexing and query performance"),
        ("beta", "network latency and routing reliability"),
    ]
}

fn scripted(top: (&'static str, f64)) -> Arc<MockClassifier> {
    Arc::new(
        MockClassifier::new("mock-esc")
            .with_script("model", vec![top, ("beta", 1.0 - top.1)])
            .unwrap(),
    )
}

fn request_in_band() -> DecisionRequest {
    let policy = DecisionPolicy::default().with_escalate_below(0.65).unwrap();
    request_with_policy(
        state_with_fact("env", "production"),
        vec![choice_question("model", &candidates())],
        policy,
        RequestMetadata::default(),
    )
}

#[test]
fn escalate_outcome_carries_a_valid_report() {
    // Confidence 0.5: inside [abstain_below = 0.5? no — 0.5 is exactly
    // abstain_below, so use 0.55 to sit strictly inside the band.
    let engine = engine_with(("alpha", 0.55));
    let response = engine.decide(&request_in_band()).expect("decide");
    assert_eq!(response.outcome(), DecisionOutcome::Escalate);
    let report = response.escalation().expect("escalate carries a report");
    report.validate().expect("report valid");
    assert_eq!(report.reason, opencodifier_core::EscalationReason::BelowConfidence);
    assert_eq!(report.gate.threshold, 0.65);
    assert_eq!(report.payload.question.text, "Which model should answer this request?");
    assert!(report.payload.partial_answer.is_some());
    // Deterministic provenance from the live identity.
    assert_eq!(report.engine.model_id, "mock-esc");
}

#[test]
fn accept_outcome_has_no_report() {
    let engine = engine_with(("alpha", 0.9));
    let response = engine.decide(&request_in_band()).expect("decide");
    assert_eq!(response.outcome(), DecisionOutcome::Accept);
    assert!(response.escalation().is_none());
}

#[test]
fn absent_band_is_byte_identical_to_before() {
    // Same confidence 0.55, no escalate_below: Verify (default cascade),
    // and certainly no escalation report.
    let engine = engine_with(("alpha", 0.55));
    let policy = DecisionPolicy::default();
    let request = request_with_policy(
        state_with_fact("env", "production"),
        vec![choice_question("model", &candidates())],
        policy,
        RequestMetadata::default(),
    );
    let response = engine.decide(&request).expect("decide");
    assert_ne!(response.outcome(), DecisionOutcome::Escalate);
    assert!(response.escalation().is_none());
}

#[test]
fn cache_hit_reattaches_the_same_report() {
    let engine = engine_with(("alpha", 0.55));
    let request = request_in_band();
    let first = engine.decide(&request).expect("first");
    let second = engine.decide(&request).expect("second (cache hit)");
    assert_eq!(second.escalation(), first.escalation(), "same report on the hit");
}

#[test]
fn report_rides_the_native_wire() {
    let engine = engine_with(("alpha", 0.55));
    let response = engine.decide(&request_in_band()).expect("decide");
    let value = Native.encode_response(&response).expect("native encode");
    let obj = value.as_object().expect("object");
    let escalation = obj.get("escalation").expect("escalation block present");
    assert_eq!(escalation["reason"], "below_confidence", "snake_case on the wire");
}

#[test]
fn native_round_trip_preserves_the_report() {
    let engine = engine_with(("alpha", 0.55));
    let response = engine.decide(&request_in_band()).expect("decide");
    let value = Native.encode_response(&response).expect("encode");
    let decoded = Native
        .decode_response(&value, &request_in_band(), &opencodifier_core::Limits::default())
        .expect("decode");
    assert_eq!(decoded.escalation(), response.escalation(), "report survives the round trip");
}
