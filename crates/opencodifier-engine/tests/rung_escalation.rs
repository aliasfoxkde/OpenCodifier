//! Cross-rung escalation (D27): the configured rung list the threshold
//! node walks when a rung's gate does not accept.
//!
//! Every test drives the real engine through [`EngineHandle::with_rungs`]
//! — the same surface an interface uses — and a counting classifier makes
//! the central rule observable: an accepted question runs exactly one
//! decider, because only a non-accepting gate may fire the next rung.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use opencodifier_core::{
    DecisionOutcome, DecisionPolicy, DecisionQuestion, DecisionRequest, Distribution, FactValue,
    RequestMetadata, RiskLevel, State,
};
use opencodifier_engine::{
    CalibrationArtifact, CalibrationFit, Classifier, DecisionEngine, EngineError, EngineHandle,
    ManualClock, MockClassifier, Rung, TemperatureCalibration,
};

use common::{
    choice_question, choice_request, config, five_candidates, request_with_limit, trace_fact,
};

/// A scripted classifier that counts its decisions — the observable for
/// "the gate, and only the gate, fires a rung".
#[derive(Debug, Clone)]
struct Counting {
    calls: Arc<AtomicUsize>,
    delegate: MockClassifier,
    model_id: &'static str,
}

impl Counting {
    fn scripted(
        model_id: &'static str,
        top: (&str, f64),
        other: (&str, f64),
    ) -> (Self, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let delegate =
            MockClassifier::new(model_id).with_script("model", vec![top, other]).unwrap();
        (Self { calls: Arc::clone(&calls), delegate, model_id }, calls)
    }
}

impl Classifier for Counting {
    fn decide(
        &self,
        state: &State,
        question: &DecisionQuestion,
    ) -> Result<Distribution, EngineError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.delegate.decide(state, question)
    }

    fn model_id(&self) -> &str {
        self.model_id
    }
}

/// A handle with the built-in pipeline, `primary` deciding, and `rungs`
/// as the escalation tail.
fn engine_with_rungs(primary: Arc<dyn Classifier>, rungs: Vec<Rung>) -> EngineHandle {
    EngineHandle::with_rungs(
        config(1),
        primary,
        None,
        opencodifier_engine::LadderPolicy::default(),
        rungs,
    )
    .unwrap()
}

fn rung(classifier: Arc<dyn Classifier>) -> Rung {
    Rung::new(classifier)
}

/// The `rungs_fired` count of the threshold trace entry, if recorded.
fn rungs_fired(response: &opencodifier_core::DecisionResponse) -> Option<f64> {
    trace_fact(response, "threshold", "rungs_fired").and_then(FactValue::as_f64)
}

fn rung_chain(response: &opencodifier_core::DecisionResponse) -> String {
    trace_fact(response, "threshold", "rung_chain")
        .and_then(FactValue::as_str)
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn a_confident_primary_never_fires_a_rung() {
    let (primary, primary_calls) =
        Counting::scripted("mock/primary", ("cloud-large", 0.9), ("local-tiny", 0.1));
    let (fallback, fallback_calls) =
        Counting::scripted("mock/fallback", ("local-tiny", 0.9), ("cloud-large", 0.1));
    let engine = engine_with_rungs(Arc::new(primary), vec![rung(Arc::new(fallback))]);

    let response = engine.decide(&choice_request(&five_candidates())).unwrap();
    // The primary accepted: the escalation tail never ran (D27 — never
    // two classifiers on a happy path), and the answer is the primary's.
    assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fallback_calls.load(Ordering::SeqCst), 0);
    assert_eq!(rungs_fired(&response), None, "no rung facts on an unescalated answer");
    let answer = match response.answers().first().unwrap() {
        opencodifier_core::DecisionAnswer::Choice { choice, .. } => choice.as_str().to_owned(),
        other => panic!("unexpected answer shape: {other:?}"),
    };
    assert_eq!(answer, "cloud-large");
    assert_eq!(response.outcome(), DecisionOutcome::Accept);
}

#[test]
fn a_verifying_primary_escalates_to_an_accepting_rung() {
    let (primary, primary_calls) =
        Counting::scripted("mock/primary", ("cloud-large", 0.6), ("local-tiny", 0.4));
    let (fallback, fallback_calls) =
        Counting::scripted("mock/fallback", ("local-tiny", 0.9), ("cloud-large", 0.1));
    let engine = engine_with_rungs(Arc::new(primary), vec![rung(Arc::new(fallback))]);

    let response = engine.decide(&choice_request(&five_candidates())).unwrap();
    assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fallback_calls.load(Ordering::SeqCst), 1, "the gate fired exactly one rung");
    assert_eq!(rungs_fired(&response), Some(1.0));
    let chain = rung_chain(&response);
    assert!(chain.starts_with("mock/fallback(top=local-tiny, accept)"), "{chain}");
    // The final rung owns the answer: the shipped choice is the
    // fallback's, not the primary's.
    let answer = match response.answers().first().unwrap() {
        opencodifier_core::DecisionAnswer::Choice { choice, confidence, .. } => {
            assert!((*confidence - 0.9).abs() < 1e-12, "fallback confidence, got {confidence}");
            choice.as_str().to_owned()
        }
        other => panic!("unexpected answer shape: {other:?}"),
    };
    assert_eq!(answer, "local-tiny");
    assert_eq!(response.outcome(), DecisionOutcome::Accept);
}

#[test]
fn an_abstaining_primary_escalates_the_same_way() {
    // Abstain is "this rung could not decide" too: < 0.50 calibrated.
    let (primary, _) =
        Counting::scripted("mock/primary", ("cloud-large", 0.3), ("local-tiny", 0.7));
    let (fallback, fallback_calls) =
        Counting::scripted("mock/fallback", ("local-tiny", 0.95), ("cloud-large", 0.05));
    let engine = engine_with_rungs(Arc::new(primary), vec![rung(Arc::new(fallback))]);

    let response = engine.decide(&choice_request(&five_candidates())).unwrap();
    assert_eq!(fallback_calls.load(Ordering::SeqCst), 1);
    assert_eq!(response.outcome(), DecisionOutcome::Accept);
}

#[test]
fn an_exhausted_list_keeps_the_final_rungs_outcome() {
    // Both rungs land in the verify band and no verifier is configured:
    // the answer stays marked unverified — abstention and verify remain
    // successful outcomes, never errors (PLANNING.md §19).
    let (primary, _) =
        Counting::scripted("mock/primary", ("cloud-large", 0.6), ("local-tiny", 0.4));
    let (fallback, fallback_calls) =
        Counting::scripted("mock/fallback", ("local-tiny", 0.6), ("cloud-large", 0.4));
    let engine = engine_with_rungs(Arc::new(primary), vec![rung(Arc::new(fallback))]);

    let response = engine.decide(&choice_request(&five_candidates())).unwrap();
    assert_eq!(fallback_calls.load(Ordering::SeqCst), 1);
    assert_eq!(response.outcome(), DecisionOutcome::Verify);
    let chain = rung_chain(&response);
    assert!(chain.ends_with("verify)"), "{chain}");
}

#[test]
fn a_later_rung_is_skipped_after_an_accepting_one() {
    let (primary, _) =
        Counting::scripted("mock/primary", ("cloud-large", 0.3), ("local-tiny", 0.7));
    let (first, first_calls) =
        Counting::scripted("mock/first", ("local-tiny", 0.85), ("cloud-large", 0.15));
    let (second, second_calls) =
        Counting::scripted("mock/second", ("cloud-large", 0.9), ("local-tiny", 0.1));
    let engine =
        engine_with_rungs(Arc::new(primary), vec![rung(Arc::new(first)), rung(Arc::new(second))]);

    let response = engine.decide(&choice_request(&five_candidates())).unwrap();
    assert_eq!(first_calls.load(Ordering::SeqCst), 1);
    assert_eq!(second_calls.load(Ordering::SeqCst), 0, "an accepting rung ends the walk");
    assert_eq!(rungs_fired(&response), Some(1.0));
    assert_eq!(response.outcome(), DecisionOutcome::Accept);
    let answer = match response.answers().first().unwrap() {
        opencodifier_core::DecisionAnswer::Choice { choice, .. } => choice.as_str().to_owned(),
        other => panic!("unexpected answer shape: {other:?}"),
    };
    assert_eq!(answer, "local-tiny");
}

#[test]
fn the_verifier_cascade_reads_the_final_rung() {
    // Both rungs verify-band; a verifier agreeing with the fallback's
    // answer verifies it, one disagreeing abstains it.
    for (verifier_top, expected) in [
        (("local-tiny", 0.9), DecisionOutcome::Verified),
        (("cloud-large", 0.9), DecisionOutcome::Abstain),
    ] {
        let (primary, _) =
            Counting::scripted("mock/primary", ("cloud-large", 0.6), ("local-tiny", 0.4));
        let (fallback, _) =
            Counting::scripted("mock/fallback", ("local-tiny", 0.6), ("cloud-large", 0.4));
        let verifier = MockClassifier::new("mock/verifier")
            .with_script("model", vec![verifier_top, ("other", 0.1)])
            .unwrap();
        let engine = EngineHandle::with_rungs(
            config(1),
            Arc::new(primary),
            Some(Arc::new(verifier)),
            opencodifier_engine::LadderPolicy::default(),
            vec![rung(Arc::new(fallback))],
        )
        .unwrap();
        let response = engine.decide(&choice_request(&five_candidates())).unwrap();
        assert_eq!(response.outcome(), expected, "verifier picks {verifier_top:?}");
        assert!(response.metrics().verification_triggered);
    }
}

#[test]
fn a_rungs_own_policy_and_calibration_gate_its_distribution() {
    // The engine-level calibration and the node policy would accept 0.9,
    // but this rung carries a T = 2 temperature (flattening 0.9 to ~0.75
    // — inside the verify band) and a stricter gate of its own; the walk
    // must exhaust with the rung's verdict, not the node policy's.
    let (primary, _) =
        Counting::scripted("mock/primary", ("cloud-large", 0.3), ("local-tiny", 0.7));
    let fallback = Counting::scripted("mock/fallback", ("local-tiny", 0.9), ("cloud-large", 0.1)).0;
    let artifact = CalibrationArtifact {
        format_version: 1,
        scheme: "temperature".to_owned(),
        model_id: "mock/fallback".to_owned(),
        calibration_version: 7,
        default_temperature: 2.0,
        temperatures: std::collections::BTreeMap::new(),
        fit: CalibrationFit {
            items: 120,
            ece_before: 0.5,
            ece_after: 0.1,
            source: "test".to_owned(),
            aurc_before: None,
            aurc_after: None,
            accuracy_at_coverage: None,
        },
    };
    let calibrated = Rung::new(Arc::new(fallback))
        .with_calibration(Arc::new(TemperatureCalibration::from_artifact(artifact).unwrap()))
        .with_policy(DecisionPolicy::new(0.95, 0.7, 0.5, RiskLevel::Low).unwrap());
    let engine = engine_with_rungs(Arc::new(primary), vec![calibrated]);

    let response = engine.decide(&choice_request(&five_candidates())).unwrap();
    // Raw 0.9 tempered to ~0.75 by the rung's own calibration, and the
    // rung's own 0.95 gate keeps it out of accept — against the node
    // policy (0.80) it would have accepted.
    assert_eq!(response.outcome(), DecisionOutcome::Verify);
    let confidence = match response.answers().first().unwrap() {
        opencodifier_core::DecisionAnswer::Choice { confidence, .. } => *confidence,
        other => panic!("unexpected answer shape: {other:?}"),
    };
    assert!((0.74..=0.76).contains(&confidence), "tempered confidence, got {confidence}");
    let chain = rung_chain(&response);
    assert!(chain.contains("verify"), "{chain}");
}

#[test]
fn the_rung_list_rekeys_the_cache_identity() {
    let (primary, _) =
        Counting::scripted("mock/primary", ("cloud-large", 0.9), ("local-tiny", 0.1));
    let (fallback, _) =
        Counting::scripted("mock/fallback", ("local-tiny", 0.9), ("cloud-large", 0.1));
    let (other, _) = Counting::scripted("mock/other", ("local-tiny", 0.9), ("cloud-large", 0.1));

    let plain = EngineHandle::new(config(1), Arc::new(primary.clone()), None).unwrap();
    let with_rung =
        engine_with_rungs(Arc::new(primary.clone()), vec![rung(Arc::new(fallback.clone()))]);
    let reordered = engine_with_rungs(
        Arc::new(primary.clone()),
        vec![rung(Arc::new(other.clone())), rung(Arc::new(fallback.clone()))],
    );

    let base = plain.identity().model_id;
    assert!(!base.contains("rungs-v1"), "{base}");
    assert_eq!(with_rung.identity().model_id, format!("{base}|rungs-v1@mock/fallback@0"));
    // Order and membership are part of the composition: a different list
    // is a different identity, so cached decisions re-key.
    assert_eq!(
        reordered.identity().model_id,
        format!("{base}|rungs-v1@mock/other@0+mock/fallback@0")
    );
    assert_ne!(with_rung.identity().model_id, reordered.identity().model_id);
}

#[test]
fn determinism_two_engines_decide_identically() {
    let build = || {
        let (primary, _) =
            Counting::scripted("mock/primary", ("cloud-large", 0.6), ("local-tiny", 0.4));
        let (fallback, _) =
            Counting::scripted("mock/fallback", ("local-tiny", 0.9), ("cloud-large", 0.1));
        engine_with_rungs(Arc::new(primary), vec![rung(Arc::new(fallback))])
    };
    let request = choice_request(&five_candidates());
    let first = build().decide(&request).unwrap();
    let second = build().decide(&request).unwrap();
    assert_eq!(first.answers(), second.answers());
    assert_eq!(first.outcome(), second.outcome());
    // The traces carry the same facts (the walk is deterministic), and a
    // repeat within one engine is served from the cache under the rung
    // identity — the cached response is the escalated one.
    let engine = build();
    let cached = engine.decide(&request).unwrap();
    let again = engine.decide(&request).unwrap();
    assert!(again.metrics().cache_hit);
    assert_eq!(cached.answers(), again.answers());
    assert_eq!(again.outcome(), DecisionOutcome::Accept, "the cache stores the escalated answer");
}

#[test]
fn only_the_final_question_of_a_walk_is_shipped_per_question() {
    // Two questions: one the primary accepts, one it escalates. The
    // replacement map must not displace the accepted question's primary
    // decision, and the escalated question ships its final rung.
    let state = State::from_text("Summarize research across many sources and compare findings");
    let questions = vec![
        choice_question("model", &five_candidates()),
        choice_question(
            "storage",
            &[("ssd", "fast local ssd storage"), ("tape", "slow archival tape storage")],
        ),
    ];
    let request = DecisionRequest::new(
        state,
        questions,
        DecisionPolicy::default(),
        RequestMetadata::default(),
    )
    .unwrap();

    let (primary, _) =
        Counting::scripted("mock/primary", ("cloud-large", 0.9), ("local-tiny", 0.1));
    // Storage is unscripted at the primary: Mock answers uniformly
    // (0.5/0.5 — verify band), so it escalates; the fallback scripts it
    // decisively.
    let fallback = MockClassifier::new("mock/fallback")
        .with_script("storage", vec![("ssd", 0.95), ("tape", 0.05)])
        .unwrap();
    let engine = engine_with_rungs(Arc::new(primary), vec![rung(Arc::new(fallback))]);

    let response = engine.decide(&request).unwrap();
    assert_eq!(response.answers().len(), 2);
    assert_eq!(
        response.outcome(),
        DecisionOutcome::Accept,
        "the accepting question carries the run"
    );
    for answer in response.answers() {
        match answer {
            opencodifier_core::DecisionAnswer::Choice {
                question_id, choice, confidence, ..
            } => {
                if question_id.as_str() == "model" {
                    assert_eq!(choice.as_str(), "cloud-large", "primary answer untouched");
                    assert!((*confidence - 0.9).abs() < 1e-12);
                } else {
                    assert_eq!(
                        choice.as_str(),
                        "ssd",
                        "escalated question ships the rung's answer"
                    );
                    assert!((*confidence - 0.95).abs() < 1e-12);
                }
            }
            other => panic!("unexpected answer shape: {other:?}"),
        }
    }
    let storage_entry = response
        .trace()
        .entries()
        .iter()
        .find(|entry| {
            entry.node == "threshold"
                && matches!(entry.detail.get("question"), Some(value) if
                    matches!(value, FactValue::Text(text) if text == "storage"))
        })
        .unwrap();
    assert_eq!(
        storage_entry.detail.get("rungs_fired").and_then(FactValue::as_f64),
        Some(1.0),
        "only the escalated question records a walk"
    );
}

#[test]
fn a_rung_walk_respects_the_deadline() {
    // The fallback charges ten manual seconds per decision and there is
    // a second rung behind it: the walk must reach the deadline guard
    // between rungs instead of running the whole list (D27 — the walk
    // cannot outrun the request's budget).
    let (primary, _) =
        Counting::scripted("mock/primary", ("cloud-large", 0.6), ("local-tiny", 0.4));
    let clock = Arc::new(ManualClock::new());
    let slow = common::ClockAdvancing::new(Arc::clone(&clock), Duration::from_secs(10));
    let also_slow = common::ClockAdvancing::new(Arc::clone(&clock), Duration::from_secs(10));
    let engine = DecisionEngine::new_with_rungs(
        config(1),
        Arc::clone(&clock) as Arc<_>,
        Arc::new(primary),
        None,
        vec![rung(Arc::new(slow)), rung(Arc::new(also_slow))],
    )
    .unwrap();

    let request = request_with_limit(&five_candidates(), Duration::from_secs(5));
    let error = engine.decide(&request).unwrap_err();
    assert!(
        matches!(error, EngineError::Timeout { .. }),
        "expected a budget refusal, got {error:?}"
    );
}

#[test]
fn an_engine_without_rungs_decides_unchanged() {
    // The default posture: no rung facts, no identity decoration, and
    // the same outcome the pre-D27 engine produced.
    let (primary, primary_calls) =
        Counting::scripted("mock/primary", ("cloud-large", 0.6), ("local-tiny", 0.4));
    let engine = engine_with_rungs(Arc::new(primary), Vec::new());
    let response = engine.decide(&choice_request(&five_candidates())).unwrap();
    assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
    assert_eq!(rungs_fired(&response), None);
    assert!(!engine.identity().model_id.contains("rungs-v1"));
    assert_eq!(response.outcome(), DecisionOutcome::Verify);
}
