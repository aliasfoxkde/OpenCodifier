//! Runs over several questions at once (PLANNING.md §18, §19).
//!
//! A request may ask a choice, a boolean, and a score question together.
//! Each kind is decided by its own node, and the response is only as
//! decisive as its least decisive question: the outcome is the most
//! conservative per-question outcome, and the confidence report is the one
//! belonging to the question that drove it. Both rules are deterministic,
//! including how ties break.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

mod common;

use std::sync::Arc;

use common::{boolean_question, choice_question, config, engine, question_request, score_question};
use opencodifier_core::{
    DecisionAnswer, DecisionOutcome, DecisionPolicy, DecisionRequest, RequestMetadata, RiskLevel,
    State,
};
use opencodifier_engine::{Classifier, DecisionEngine, MockClassifier, SystemClock};

/// A mock scripted over the three questions of [`mixed`].
///
/// `model` and `tools` are two-way distributions whose winning key carries
/// the given probability; `difficulty` is supplied as explicit level masses
/// so a test can choose both the confidence and the expected value.
fn scripted(model: f64, tools: f64, difficulty: Vec<(&'static str, f64)>) -> Arc<MockClassifier> {
    Arc::new(
        MockClassifier::new("mock/scripted")
            .with_script("model", vec![("local-small", model), ("cloud-large", 1.0 - model)])
            .unwrap()
            .with_script("tools", vec![("true", tools), ("false", 1.0 - tools)])
            .unwrap()
            .with_script("difficulty", difficulty)
            .unwrap(),
    )
}

/// The built-in three-kind request.
fn mixed() -> DecisionRequest {
    question_request(vec![
        choice_question(
            "model",
            &[("local-small", "small local model"), ("cloud-large", "cloud model")],
        ),
        boolean_question("tools", "Does this request need tools?"),
        score_question("difficulty", &["trivial", "complex", "expert"]),
    ])
}

#[test]
fn every_question_kind_is_decided_in_one_run() {
    let engine =
        engine(scripted(0.9, 0.9, vec![("trivial", 0.4), ("complex", 0.35), ("expert", 0.25)]), 2)
            .unwrap();
    let (response, report) = engine.decide_with_report(&mixed()).unwrap();

    assert_eq!(response.answers().len(), 3, "{:?}", response.answers());
    assert_eq!(report.outcomes().len(), 3);
    assert_eq!(report.narrowing().len(), 1, "only the choice question narrows");

    // Choice: the scripted winner, with its scripted confidence.
    match &response.answers()[0] {
        DecisionAnswer::Choice { choice, confidence, .. } => {
            assert_eq!(choice.as_str(), "local-small");
            assert!((confidence - 0.9).abs() < 1e-12);
        }
        other => panic!("expected a choice answer, got {other:?}"),
    }
    // Boolean: the affirmative hypothesis, with the same confidence.
    match &response.answers()[1] {
        DecisionAnswer::Boolean { value, probability, confidence, .. } => {
            assert!(value);
            assert!((probability - 0.9).abs() < 1e-12);
            assert!((confidence - 0.9).abs() < 1e-12);
        }
        other => panic!("expected a boolean answer, got {other:?}"),
    }
    // Score: the expected value over positional weights, mapped back to a
    // level — 0.35·1 + 0.25·2 = 0.85 → "trivial".
    match &response.answers()[2] {
        DecisionAnswer::Score { expected, level, confidence, .. } => {
            assert!((expected - 0.85).abs() < 1e-9, "expected {expected}");
            assert_eq!(level, "trivial");
            assert!((confidence - 0.4).abs() < 1e-12);
        }
        other => panic!("expected a score answer, got {other:?}"),
    }

    // Each kind left its own trace entry, named after its node.
    let nodes: std::collections::BTreeSet<&str> =
        response.trace().entries().iter().map(|entry| entry.node.as_str()).collect();
    for expected in ["choice", "boolean", "score", "threshold"] {
        assert!(nodes.contains(expected), "{expected} did not run: {nodes:?}");
    }
    // 0.9 accepts, 0.9 accepts, 0.4 abstains: the run abstains.
    assert_eq!(response.outcome(), DecisionOutcome::Abstain);
    assert_eq!(report.outcomes()[2].1, DecisionOutcome::Abstain);
    assert!((response.confidence().calibrated_confidence - 0.4).abs() < 1e-12);
}

#[test]
fn the_response_is_only_as_decisive_as_its_least_decisive_question() {
    // 0.9 accepts, 0.7 verifies, 0.45 abstains (the default policy abstains
    // below 0.50) — so the run abstains, and the report is the abstaining
    // question's.
    let engine =
        engine(scripted(0.9, 0.7, vec![("trivial", 0.2), ("complex", 0.35), ("expert", 0.45)]), 1)
            .unwrap();
    let (response, report) = engine.decide_with_report(&mixed()).unwrap();

    assert_eq!(
        report.outcomes().iter().map(|(_, outcome)| *outcome).collect::<Vec<_>>(),
        vec![DecisionOutcome::Accept, DecisionOutcome::Verify, DecisionOutcome::Abstain]
    );
    assert_eq!(response.outcome(), DecisionOutcome::Abstain);
    assert!(
        (response.confidence().calibrated_confidence - 0.45).abs() < 1e-12,
        "the abstaining question drives the report: {:?}",
        response.confidence()
    );
    assert!(!response.outcome().is_decisive());
}

#[test]
fn an_all_confident_run_reports_accept_and_the_tightest_question() {
    // Every question clears the gate, so the run accepts — and the report is
    // the weakest of the three, not the strongest.
    let engine =
        engine(scripted(0.95, 0.9, vec![("trivial", 0.05), ("complex", 0.1), ("expert", 0.85)]), 2)
            .unwrap();
    let (response, report) = engine.decide_with_report(&mixed()).unwrap();

    assert_eq!(response.outcome(), DecisionOutcome::Accept);
    assert!(report.outcomes().iter().all(|(_, outcome)| *outcome == DecisionOutcome::Accept));
    assert!(
        (response.confidence().calibrated_confidence - 0.85).abs() < 1e-12,
        "the tightest question drives the report: {:?}",
        response.confidence()
    );
    assert!(!response.metrics().verification_triggered);
}

#[test]
fn a_verifier_covers_every_question_that_needs_it() {
    // A strict gate puts all three questions in the verify band; a verifier
    // that agrees with each upgrades the whole run to verified.
    let policy = DecisionPolicy::new(0.95, 0.85, 0.5, RiskLevel::Low).unwrap();
    let request = DecisionRequest::new(
        State::from_text("Summarize research across many sources and compare findings"),
        mixed().questions().to_vec(),
        policy,
        RequestMetadata::default(),
    )
    .unwrap();
    let verifier = scripted(0.9, 0.9, vec![("trivial", 0.05), ("complex", 0.1), ("expert", 0.85)]);
    let engine = DecisionEngine::new(
        config(1),
        Arc::new(SystemClock),
        scripted(0.9, 0.9, vec![("trivial", 0.05), ("complex", 0.1), ("expert", 0.85)]),
        Some(verifier as Arc<dyn Classifier>),
    )
    .unwrap();

    let (response, report) = engine.decide_with_report(&request).unwrap();
    assert_eq!(response.outcome(), DecisionOutcome::Verified);
    assert!(report.outcomes().iter().all(|(_, outcome)| *outcome == DecisionOutcome::Verified));
    assert!(response.metrics().verification_triggered);
    assert_eq!(response.confidence().verifier_agreement, Some(true));
}

#[test]
fn confidence_ties_break_on_question_id() {
    // Two questions with the same confidence and the same outcome are
    // ordered by id, so the reported confidence is reproducible: "model"
    // (two candidates, margin 0.8) is the driver ahead of "zulu" (three
    // candidates, margin 0.85) only because it sorts first.
    let mock = Arc::new(
        MockClassifier::new("mock/tied")
            .with_script("model", vec![("local-small", 0.9), ("cloud-large", 0.1)])
            .unwrap()
            .with_script("zulu", vec![("a", 0.9), ("b", 0.05), ("c", 0.05)])
            .unwrap(),
    );
    let request = question_request(vec![
        choice_question("zulu", &[("a", "alpha model"), ("b", "beta model"), ("c", "gamma model")]),
        choice_question("model", &[("local-small", "small local"), ("cloud-large", "cloud")]),
    ]);
    let engine = engine(mock, 1).unwrap();

    let (response, _) = engine.decide_with_report(&request).unwrap();
    assert_eq!(response.outcome(), DecisionOutcome::Accept);
    assert!(
        (response.confidence().calibrated_confidence - 0.9).abs() < 1e-12,
        "{:?}",
        response.confidence()
    );
    assert!(
        (response.confidence().margin - 0.8).abs() < 1e-12,
        "the driver must be the first question id, not the wider margin: {:?}",
        response.confidence()
    );
}
