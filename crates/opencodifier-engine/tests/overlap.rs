//! Label-set overlap preflight (RESEARCH §15.6 item 4): a policy
//! ceiling over the candidate labels' maximum pairwise lexical overlap.
//! At or above the ceiling the rung's choice between near-duplicate
//! labels is label-noise-dominated, so the question abstains instead of
//! guessing. The measurement is disclosed in the trace; the default (no
//! ceiling) is byte-identical to the pre-field engine.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

mod common;

use std::sync::Arc;

use common::{choice_question, config, engine_with, request_with_policy, trace_fact};
use opencodifier_core::{DecisionOutcome, DecisionPolicy, FactValue, RequestMetadata, State};
use opencodifier_engine::MockClassifier;

fn overlap_policy(limit: f64) -> DecisionPolicy {
    DecisionPolicy::default().with_max_label_overlap(limit).unwrap()
}

fn request(policy: DecisionPolicy) -> opencodifier_core::DecisionRequest {
    request_with_policy(
        State::from_text("Summarize research across many sources and compare findings"),
        vec![choice_question(
            "pick",
            &[
                ("billing_intl", "international billing"),
                ("billing-intl", "billing, international"),
                ("weather", "the weather service"),
            ],
        )],
        policy,
        RequestMetadata::default(),
    )
}

/// Two labels sharing a token set at or above the ceiling abstain the
/// question: the answer is withheld and the trace discloses the
/// ceiling, the measurement, the pair, and the abstention.
#[test]
fn an_overlapped_pair_abstains_at_the_ceiling() {
    // The scripted distribution tops one of the overlapping labels at
    // 0.90 — the gate alone would accept, so the outcome proves the
    // preflight, not the cascade.
    let classifier = Arc::new(
        MockClassifier::new("mock/overlap")
            .with_script(
                "pick",
                vec![("billing_intl", 0.90), ("billing-intl", 0.06), ("weather", 0.04)],
            )
            .unwrap(),
    );
    let engine = engine_with(config(1), classifier).unwrap();
    let (response, report) = engine.decide_with_report(&request(overlap_policy(0.9))).unwrap();

    assert_eq!(response.outcome(), DecisionOutcome::Abstain);
    assert_eq!(report.outcomes()[0].1, DecisionOutcome::Abstain);
    assert_eq!(response.answers(), []);
    assert_eq!(trace_fact(&response, "choice", "max_label_overlap"), Some(&FactValue::Float(0.9)));
    assert_eq!(trace_fact(&response, "choice", "label_overlap"), Some(&FactValue::Float(1.0)));
    assert_eq!(
        trace_fact(&response, "choice", "label_overlap_pair"),
        Some(&FactValue::Text("billing_intl|billing-intl".to_owned()))
    );
    assert_eq!(
        trace_fact(&response, "choice", "label_overlap_abstained"),
        Some(&FactValue::Boolean(true))
    );
}

/// Below the ceiling the decision stands and only the measurement is
/// disclosed — no abstention fact.
#[test]
fn a_distinct_set_accepts_under_the_ceiling() {
    let script = vec![("alpha", 0.90), ("beta", 0.06), ("gamma", 0.04)];
    let mock =
        Arc::new(MockClassifier::new("mock/distinct").with_script("pick", script.clone()).unwrap());
    let engine = engine_with(config(1), mock).unwrap();
    let request = request_with_policy(
        State::from_text("Summarize research across many sources and compare findings"),
        vec![choice_question(
            "pick",
            &[("alpha", "model a"), ("beta", "model b"), ("gamma", "model c")],
        )],
        overlap_policy(0.9),
        RequestMetadata::default(),
    );
    let (response, _) = engine.decide_with_report(&request).unwrap();

    assert_eq!(response.outcome(), DecisionOutcome::Accept);
    assert_eq!(response.answers().len(), 1);
    assert_eq!(trace_fact(&response, "choice", "max_label_overlap"), Some(&FactValue::Float(0.9)));
    assert_eq!(trace_fact(&response, "choice", "label_overlap"), Some(&FactValue::Float(0.0)));
    assert_eq!(trace_fact(&response, "choice", "label_overlap_abstained"), None);
}

/// The default (no ceiling) is byte-identical to the pre-field engine
/// and records no overlap facts at all.
#[test]
fn the_default_records_no_overlap_surface() {
    let script = vec![("billing_intl", 0.90), ("billing-intl", 0.06), ("weather", 0.04)];
    let classifier =
        Arc::new(MockClassifier::new("mock/plain").with_script("pick", script.clone()).unwrap());
    let request = request(DecisionPolicy::default());

    let plain = engine_with(config(1), classifier.clone()).unwrap();
    let (baseline, _) = plain.decide_with_report(&request).unwrap();

    let engine = engine_with(config(1), classifier).unwrap();
    let (response, _) = engine.decide_with_report(&request).unwrap();

    assert_eq!(baseline, response);
    assert_eq!(response.outcome(), DecisionOutcome::Accept);
    assert_eq!(trace_fact(&response, "choice", "max_label_overlap"), None);
    assert_eq!(trace_fact(&response, "choice", "label_overlap"), None);
}

/// An overlap abstention survives the escalation walk's outcome
/// re-derivation, and the withdrawn answer never ships.
#[test]
fn an_overlap_abstain_is_not_rederived_into_an_acceptance() {
    let classifier = Arc::new(
        MockClassifier::new("mock/confident")
            .with_script(
                "pick",
                vec![("billing_intl", 0.99), ("billing-intl", 0.005), ("weather", 0.005)],
            )
            .unwrap(),
    );
    let engine = engine_with(config(1), classifier).unwrap();
    let (response, _) = engine.decide_with_report(&request(overlap_policy(1.0))).unwrap();

    assert_eq!(response.outcome(), DecisionOutcome::Abstain);
    assert_eq!(response.answers(), []);
    assert_eq!(
        trace_fact(&response, "choice", "label_overlap_abstained"),
        Some(&FactValue::Boolean(true))
    );
}
