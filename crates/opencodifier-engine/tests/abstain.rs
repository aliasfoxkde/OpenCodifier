//! Elicited-abstain candidate (RESEARCH §15.6 item 3): a policy-configured
//! synthetic choice candidate the deciding rung may elect instead of
//! guessing. Election is an abstention — terminal in the gate and verifier
//! cascade, withheld from the answer set, and disclosed in the trace. The
//! default (no candidate configured) must be byte-identical to the
//! pre-field engine.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

mod common;

use std::sync::Arc;

use common::{choice_question, config, engine_with, request_with_policy, trace_fact};
use opencodifier_core::{
    AbstainCandidate, DecisionOutcome, DecisionPolicy, FactValue, RequestMetadata, State,
};
use opencodifier_engine::MockClassifier;

fn abstain_policy() -> DecisionPolicy {
    let candidate =
        AbstainCandidate::new("abstain", "None of the listed candidates decides this question")
            .unwrap();
    DecisionPolicy::default().with_abstain_candidate(candidate).unwrap()
}

fn request(policy: DecisionPolicy) -> opencodifier_core::DecisionRequest {
    request_with_policy(
        State::from_text("Summarize research across many sources and compare findings"),
        vec![choice_question("pick", &[("alpha", "model a"), ("beta", "model b")])],
        policy,
        RequestMetadata::default(),
    )
}

/// The configured candidate rides every choice question and an election of
/// it is a terminal abstention with no answer, disclosed in the trace.
#[test]
fn an_elected_abstain_is_a_terminal_abstention_with_no_answer() {
    // The scripted distribution tops the synthetic candidate at 0.62 —
    // inside the default verify band (0.50..0.65), so the outcome proves
    // the elicitation override, not the gate cascade.
    let classifier = Arc::new(
        MockClassifier::new("mock/abstain")
            .with_script("pick", vec![("abstain", 0.62), ("alpha", 0.28), ("beta", 0.10)])
            .unwrap(),
    );
    let engine = engine_with(config(1), classifier).unwrap();
    let (response, report) = engine.decide_with_report(&request(abstain_policy())).unwrap();

    assert_eq!(response.outcome(), DecisionOutcome::Abstain);
    assert_eq!(report.outcomes()[0].1, DecisionOutcome::Abstain);
    assert!(
        response.answers().is_empty(),
        "an elected abstention must not ship the synthetic candidate as an answer"
    );
    assert_eq!(
        trace_fact(&response, "choice", "abstain_candidate"),
        Some(&FactValue::Text("abstain".to_owned()))
    );
    assert_eq!(
        trace_fact(&response, "choice", "abstain_elicited"),
        Some(&FactValue::Boolean(true))
    );
    // Terminal: the verifier cascade never ran for the question (the
    // threshold trace records the gate it applied — an elected abstention
    // records `abstain`, never `verify`).
    let outcome_fact = trace_fact(&response, "threshold", "outcome");
    assert_eq!(outcome_fact, Some(&FactValue::Text("abstain".to_owned())));
}

/// When the rung answers with a declared candidate, elicitation changes
/// nothing about the answer — the synthetic candidate is scored and lost.
#[test]
fn a_declared_answer_ignores_the_riding_candidate() {
    let script = vec![("alpha", 0.90), ("beta", 0.08), ("abstain", 0.02)];
    let classifier =
        Arc::new(MockClassifier::new("mock/declared").with_script("pick", script.clone()).unwrap());
    let engine = engine_with(config(1), classifier).unwrap();
    let (response, _) = engine.decide_with_report(&request(abstain_policy())).unwrap();

    assert_eq!(response.outcome(), DecisionOutcome::Accept);
    assert_eq!(response.answers().len(), 1);
    let answer = &response.answers()[0];
    let opencodifier_core::DecisionAnswer::Choice { choice, .. } = answer else {
        panic!("expected a choice answer, got {answer:?}");
    };
    assert_eq!(choice.as_str(), "alpha");
    assert_eq!(
        trace_fact(&response, "choice", "abstain_candidate"),
        Some(&FactValue::Text("abstain".to_owned()))
    );
    assert_eq!(trace_fact(&response, "choice", "abstain_elicited"), None);
}

/// A declared candidate that already owns the configured id *takes the
/// abstain role*: the configured id marks the abstain candidate, declared
/// or synthetic, so electing it abstains either way. Nothing is appended
/// when the id is already present.
#[test]
fn a_declared_candidate_owning_the_id_takes_the_abstain_role() {
    let classifier = Arc::new(
        MockClassifier::new("mock/collision")
            .with_script("pick", vec![("abstain", 0.95), ("alpha", 0.03), ("beta", 0.02)])
            .unwrap(),
    );
    let engine = engine_with(config(1), classifier).unwrap();
    let request = request_with_policy(
        State::from_text("Summarize research across many sources and compare findings"),
        vec![choice_question(
            "pick",
            &[("alpha", "model a"), ("beta", "model b"), ("abstain", "model abstain")],
        )],
        abstain_policy(),
        RequestMetadata::default(),
    );
    let (response, _) = engine.decide_with_report(&request).unwrap();

    assert_eq!(response.outcome(), DecisionOutcome::Abstain);
    assert_eq!(response.answers(), []);
    assert_eq!(
        trace_fact(&response, "choice", "abstain_elicited"),
        Some(&FactValue::Boolean(true))
    );
}

/// The default (no candidate configured) is byte-identical to the
/// pre-field engine and records no abstention facts.
#[test]
fn the_default_records_no_abstention_surface() {
    let script = vec![("alpha", 0.90), ("beta", 0.10)];
    let classifier =
        Arc::new(MockClassifier::new("mock/plain").with_script("pick", script.clone()).unwrap());
    let request = request(DecisionPolicy::default());

    let plain = engine_with(config(1), classifier.clone()).unwrap();
    let (baseline, _) = plain.decide_with_report(&request).unwrap();

    let engine = engine_with(config(1), classifier).unwrap();
    let (response, _) = engine.decide_with_report(&request).unwrap();

    assert_eq!(baseline, response);
    assert_eq!(response.outcome(), DecisionOutcome::Accept);
    assert_eq!(trace_fact(&response, "choice", "abstain_candidate"), None);
    assert_eq!(trace_fact(&response, "choice", "abstain_elicited"), None);
}

/// An election survives the escalation walk's outcome re-derivation: the
/// walk recomputes each decision's outcome from its report, and a
/// confident "none of these" must not re-derive into an acceptance.
#[test]
fn an_elected_abstain_is_not_rederived_into_an_acceptance() {
    // 0.99 on the synthetic candidate: the gate cascade alone would accept.
    let classifier = Arc::new(
        MockClassifier::new("mock/confident")
            .with_script("pick", vec![("abstain", 0.99), ("alpha", 0.005), ("beta", 0.005)])
            .unwrap(),
    );
    let engine = engine_with(config(1), classifier).unwrap();
    let (response, _) = engine.decide_with_report(&request(abstain_policy())).unwrap();

    assert_eq!(response.outcome(), DecisionOutcome::Abstain);
    assert_eq!(response.answers(), []);
    assert_eq!(
        trace_fact(&response, "choice", "abstain_elicited"),
        Some(&FactValue::Boolean(true))
    );
}
