//! Focused-question extraction through the whole engine (PLANNING.md §45).
//!
//! Every test here drives the real [`DecisionEngine`] with a
//! [`FocusPolicy`](opencodifier_engine::FocusPolicy) configured, so the
//! assertions cover extraction, reverse escalation, cache identity, and
//! the report projection — not the `focus` function in isolation.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

mod common;

use std::sync::Arc;

use common::*;
use opencodifier_core::{
    Candidate, ChoiceQuestion, DecisionAnswer, DecisionQuestion, DecisionRequest, Distribution,
    FactValue, RequestMetadata, State,
};
use opencodifier_engine::{
    Classifier, DecisionEngine, EngineConfig, EngineError, FocusPolicy, LexicalClassifier,
    MockClassifier, SystemClock, execution_json,
};

/// Fill that shares no vocabulary with the [`restart_question`] and never
/// names a candidate: BM25 scores it at zero and no candidate id occurs in
/// it, so extraction has no reason to keep it.
fn filler(out: &mut String, range: std::ops::Range<usize>) {
    for index in range {
        let _ = std::fmt::Write::write_fmt(
            out,
            format_args!(
                "Ledger entry {index} records archived correspondence, \
                 scheduling minutiae and unrelated inventory counts.\n"
            ),
        );
    }
}

/// A state far over any budget with the decisive sentence buried mid-file.
fn long_state(evidence: &str) -> State {
    let mut text = String::new();
    filler(&mut text, 0..80);
    text.push_str(evidence);
    text.push('\n');
    filler(&mut text, 80..160);
    State::from_text(text)
}

/// A choice question whose two candidates are named in the evidence.
fn restart_question() -> DecisionQuestion {
    DecisionQuestion::Choice(
        ChoiceQuestion::new(
            "restart",
            "Which service should be restarted?",
            vec![
                Candidate::new("alpha", "the alpha service is healthy").unwrap(),
                Candidate::new("beta", "the beta service").unwrap(),
            ],
        )
        .unwrap(),
    )
}

/// A request over [`restart_question`] with the given state.
fn restart_request(state: State) -> DecisionRequest {
    request(state, vec![restart_question()], RequestMetadata::default())
}

/// The chosen candidate of the single choice answer.
fn chosen(response: &opencodifier_core::DecisionResponse) -> String {
    match &response.answers()[0] {
        DecisionAnswer::Choice { choice, .. } => choice.as_str().to_owned(),
        _ => panic!("expected a choice answer"),
    }
}

/// A classifier that is confident only when the whole state is visible.
///
/// It stands in for a decision model reading a truncated view: when the
/// decisive detail is absent the distribution sags below the policy's
/// `min_confidence`, which is exactly what reverse escalation exists for.
#[derive(Debug)]
struct NeedsFullState {
    decisive_detail: &'static str,
}

impl Classifier for NeedsFullState {
    fn decide(
        &self,
        state: &State,
        _question: &DecisionQuestion,
    ) -> Result<Distribution, EngineError> {
        if state.text().contains(self.decisive_detail) {
            Distribution::from_pairs(vec![("alpha", 0.04), ("beta", 0.96)])
        } else {
            Distribution::from_pairs(vec![("alpha", 0.45), ("beta", 0.55)])
        }
        .map_err(|error| EngineError::InvalidDistribution {
            question: "restart".into(),
            reason: error.to_string(),
        })
    }

    fn model_id(&self) -> &'static str {
        "mock/needs-full-state"
    }
}

fn focused_engine(config: EngineConfig, classifier: Arc<dyn Classifier>) -> DecisionEngine {
    DecisionEngine::new(
        config.with_focus(Some(FocusPolicy::new(24))),
        Arc::new(SystemClock),
        classifier,
        None,
    )
    .unwrap()
}

fn plain_engine(config: EngineConfig, classifier: Arc<dyn Classifier>) -> DecisionEngine {
    DecisionEngine::new(config, Arc::new(SystemClock), classifier, None).unwrap()
}

/// A [`FocusSummary`](opencodifier_engine::FocusSummary) literal, for assertions.
fn report_focus(
    decided: usize,
    engaged: usize,
    escalated: usize,
) -> opencodifier_engine::FocusSummary {
    opencodifier_engine::FocusSummary { decided, engaged, escalated }
}

#[test]
fn an_unfocused_run_has_no_focus_surface_at_all() {
    let engine = plain_engine(
        EngineConfig::with_default_pipeline().unwrap(),
        Arc::new(LexicalClassifier::new()),
    );
    let state = long_state("The beta service is failing and needs a restart now.");
    let (response, report) = engine.decide_with_report(&restart_request(state)).unwrap();

    assert_eq!(report.focus().decided, 0);
    assert!(trace_fact(&response, "choice", "focus_engaged").is_none());
    assert!(execution_json(&report).get("focus").is_none());
}

#[test]
fn a_state_within_the_budget_is_decided_on_whole() {
    let engine = focused_engine(
        EngineConfig::with_default_pipeline().unwrap(),
        Arc::new(LexicalClassifier::new()),
    );
    let state = State::from_text("The beta service is failing and needs a restart now.");
    let (response, report) = engine.decide_with_report(&restart_request(state)).unwrap();

    let focus = report.focus();
    assert_eq!((focus.decided, focus.engaged, focus.escalated), (1, 0, 0));
    assert_eq!(trace_fact(&response, "choice", "focus_engaged"), Some(&FactValue::Boolean(false)));
    assert_eq!(chosen(&response), "beta");
}

#[test]
fn extraction_engages_without_escalating_when_the_view_decides_confidently() {
    let classifier = Arc::new(
        MockClassifier::new("mock/focus-strong")
            .with_script("restart", vec![("beta", 0.96), ("alpha", 0.04)])
            .unwrap(),
    );
    let engine = focused_engine(EngineConfig::with_default_pipeline().unwrap(), classifier);
    let state = long_state("The beta service is failing and needs a restart now.");
    let (response, report) = engine.decide_with_report(&restart_request(state)).unwrap();

    let focus = report.focus();
    assert_eq!((focus.decided, focus.engaged, focus.escalated), (1, 1, 0));
    assert_eq!(trace_fact(&response, "choice", "focus_engaged"), Some(&FactValue::Boolean(true)));
    assert_eq!(
        trace_fact(&response, "choice", "focus_escalated"),
        Some(&FactValue::Boolean(false))
    );
    let kept = match trace_int(&response, "choice", "focus_kept") {
        Some(FactValue::Integer(kept)) => *kept,
        other => panic!("focus_kept, got {other:?}"),
    };
    let total = match trace_int(&response, "choice", "focus_total") {
        Some(FactValue::Integer(total)) => *total,
        other => panic!("focus_total, got {other:?}"),
    };
    assert!(kept < total, "kept {kept} of {total}");
    assert_eq!(chosen(&response), "beta");
}

#[test]
fn the_focused_run_agrees_with_the_whole_context_run() {
    let config = EngineConfig::with_default_pipeline().unwrap().with_parallelism(1);
    let evidence = "The beta service is failing and needs a restart now.";
    let state = long_state(evidence);

    let full = plain_engine(config.clone(), Arc::new(LexicalClassifier::new()));
    let (full_response, full_report) =
        full.decide_with_report(&restart_request(state.clone())).unwrap();
    assert_eq!(full_report.focus().decided, 0);

    let focused = focused_engine(config, Arc::new(LexicalClassifier::new()));
    let (focused_response, focused_report) =
        focused.decide_with_report(&restart_request(state)).unwrap();

    // The lexical rung's softmax confidence sits below the policy gate, so
    // the focused decision legitimately escalates to the full state (and
    // finds the same answer); the point of the A/B is that extraction never
    // changes the winner.
    assert_eq!(focused_report.focus(), report_focus(1, 1, 1));
    assert_eq!(chosen(&focused_response), chosen(&full_response));
    assert_eq!(chosen(&focused_response), "beta");
}

#[test]
fn a_weak_focused_decision_escalates_to_the_full_state() {
    let engine = focused_engine(
        EngineConfig::with_default_pipeline().unwrap(),
        Arc::new(NeedsFullState { decisive_detail: "rolled back" }),
    );
    // Sentence one names a candidate (kept); the decisive detail lives in
    // a sentence that shares no query vocabulary (dropped). The focused
    // view therefore sags to 0.55, below the policy's 0.80 gate, and the
    // run re-decides on the whole state.
    let evidence = "The beta service needs attention.\n\
                    Deploy hash 9f2c1e is broken and the rollout must be rolled back tonight.";
    let (response, report) =
        engine.decide_with_report(&restart_request(long_state(evidence))).unwrap();

    let focus = report.focus();
    assert_eq!((focus.decided, focus.engaged, focus.escalated), (1, 1, 1));
    assert_eq!(trace_fact(&response, "choice", "focus_escalated"), Some(&FactValue::Boolean(true)));
    // The accepted answer is the whole-state one, not the degraded view's.
    assert_eq!(chosen(&response), "beta");
    assert_eq!(response.answers()[0].confidence(), 0.96);
}

#[test]
fn focus_policy_is_part_of_the_cache_identity() {
    let classifier: Arc<dyn Classifier> = Arc::new(MockClassifier::new("mock/cache-probe"));
    let base = EngineConfig::with_default_pipeline().unwrap().with_parallelism(1);
    let request = restart_request(State::from_text("The beta service is failing."));

    let plain = plain_engine(base.clone(), Arc::clone(&classifier));
    let (_, plain_report) = plain.decide_with_report(&request).unwrap();

    let budget24 = focused_engine(base.clone(), Arc::clone(&classifier));
    let (_, budget24_report) = budget24.decide_with_report(&request).unwrap();

    let budget128 = DecisionEngine::new(
        base.with_focus(Some(FocusPolicy::new(128))),
        Arc::new(SystemClock),
        Arc::clone(&classifier),
        None,
    )
    .unwrap();
    let (_, budget128_report) = budget128.decide_with_report(&request).unwrap();

    let hex = |report: &opencodifier_engine::RunReport| report.cache_key().unwrap().as_hex();
    assert_ne!(hex(&plain_report), hex(&budget24_report));
    assert_ne!(hex(&budget24_report), hex(&budget128_report));
    assert_ne!(hex(&plain_report), hex(&budget128_report));

    // The focused engine still caches its own decisions.
    let (_, again) = budget24.decide_with_report(&request).unwrap();
    assert!(again.cache_hit());
}

#[test]
fn the_projection_reports_focus_counts_only_when_focused() {
    let classifier = Arc::new(NeedsFullState { decisive_detail: "rolled back" });
    let evidence = "The beta service needs attention.\n\
                    Deploy hash 9f2c1e is broken and the rollout must be rolled back tonight.";

    let focused = focused_engine(EngineConfig::with_default_pipeline().unwrap(), classifier);
    let (_, report) = focused.decide_with_report(&restart_request(long_state(evidence))).unwrap();
    let document = execution_json(&report);
    assert_eq!(document["focus"]["decided"], 1);
    assert_eq!(document["focus"]["engaged"], 1);
    assert_eq!(document["focus"]["escalated"], 1);

    // An engine with a policy that never extracts still counts decided
    // questions — the run went through the focused code path.
    let short = focused_engine(
        EngineConfig::with_default_pipeline().unwrap(),
        Arc::new(MockClassifier::new("mock/short")),
    );
    let (_, report) = short
        .decide_with_report(&restart_request(State::from_text("The beta service is failing.")))
        .unwrap();
    let document = execution_json(&report);
    assert_eq!(document["focus"]["decided"], 1);
    assert_eq!(document["focus"]["engaged"], 0);
    assert_eq!(document["focus"]["escalated"], 0);
}
