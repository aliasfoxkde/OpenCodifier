//! Failure paths (PLANNING.md §10, §43–§45).
//!
//! Every way a run can fail, and what it reports: a panic in one node must
//! not take the decision down, a deadline is observed at the node boundary
//! it actually crosses, an unusable distribution is an error rather than a
//! guess, and incoherent configuration is rejected before anything runs.
//! Each assertion pins a stable [`EngineError`] code, because those codes
//! are what HTTP handlers and MCP payloads map onto.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{
    Cancelling, ClockAdvancing, boolean_question, choice_question, choice_request, config, engine,
    engine_with, engine_with_clock, five_candidates, graph, long_chain, node, question_request,
    request_with_limit, score_question, trace_int, two_way,
};
use opencodifier_core::{
    DecisionAnswer, DecisionOutcome, DecisionQuestion, DecisionRequest, Distribution, FactValue,
    Limits, RequestMetadata, State,
};
use opencodifier_engine::{
    Classifier, Condition, DecisionEngine, EngineConfig, EngineError, ManualClock, MockClassifier,
    NodeKind, SystemClock,
};

/// A classifier that panics for one question id and delegates the rest.
///
/// Only a worker thread of a parallel wave can contain a panic, so this
/// double exists to prove the containment is real and not decorative.
#[derive(Debug)]
struct Panicking {
    /// The question id whose decision blows up.
    on: &'static str,
    /// Where every other distribution comes from.
    delegate: MockClassifier,
}

impl Classifier for Panicking {
    fn decide(
        &self,
        state: &State,
        question: &DecisionQuestion,
    ) -> Result<Distribution, EngineError> {
        assert!(
            question.id().as_str() != self.on,
            "the decision model exploded on question `{}`",
            self.on
        );
        self.delegate.decide(state, question)
    }

    fn model_id(&self) -> &'static str {
        "test/panicking"
    }
}

/// A verifier that cannot produce a usable distribution.
///
/// Verification is a model call like any other, so it can fail; the run
/// must report the failure instead of silently treating the answer as
/// verified.
#[derive(Debug)]
struct BrokenVerifier;

impl Classifier for BrokenVerifier {
    fn decide(
        &self,
        _state: &State,
        question: &DecisionQuestion,
    ) -> Result<Distribution, EngineError> {
        Err(EngineError::InvalidDistribution {
            question: question.id().to_string(),
            reason: "verifier model unavailable".to_owned(),
        })
    }

    fn model_id(&self) -> &'static str {
        "test/broken-verifier"
    }
}

/// A request carrying one question of every kind, so the boolean, score,
/// and filter nodes of the built-in pipeline all have work to do.
fn mixed_request() -> DecisionRequest {
    question_request(vec![
        choice_question(
            "model",
            &[("local-small", "small local model"), ("cloud-large", "cloud model")],
        ),
        boolean_question("tools", "Does this request need tools?"),
        score_question("difficulty", &["trivial", "expert"]),
    ])
}

#[test]
fn a_panicking_node_is_reported_as_a_node_failure() {
    // The score node shares a wave with the boolean and filter nodes, so at
    // this parallelism it runs on its own thread and its panic is contained
    // there instead of unwinding the decision.
    let classifier = Arc::new(Panicking { on: "difficulty", delegate: MockClassifier::new("m") });
    let engine = engine(classifier, 4).unwrap();

    let error = engine.decide(&mixed_request()).unwrap_err();
    assert_eq!(error.code(), "engine.node_failed");
    assert_eq!(
        error,
        EngineError::NodeFailed {
            node: "score".to_owned(),
            reason: "worker thread panicked".to_owned(),
        }
    );
}

#[test]
fn a_panic_in_a_sequential_wave_reaches_the_caller() {
    // Sequential execution has no worker thread to contain a panic, so the
    // caller sees it. That asymmetry is the documented cost of a
    // synchronous executor, and a caller that cannot tolerate it must run
    // with parallelism above one.
    let classifier = Arc::new(Panicking { on: "difficulty", delegate: MockClassifier::new("m") });
    let engine = engine(classifier, 1).unwrap();
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.decide(&mixed_request())));
    assert!(result.is_err(), "a sequential panic must be observable to the caller");
}

#[test]
fn cancellation_is_observed_at_the_next_boundary_of_a_parallel_wave() {
    let cancelling = Arc::new(Cancelling::new());
    let slot = Arc::clone(&cancelling.token);
    let engine = DecisionEngine::new(config(4), Arc::new(SystemClock), cancelling, None).unwrap();
    slot.set(engine.cancellation_token()).expect("token set once");

    // The boolean node cancels from inside wave 2; the nodes it shares the
    // wave with are already running and are not preemptible, so the run
    // stops at the next boundary rather than mid-node.
    let error = engine.decide(&choice_request(&five_candidates())).unwrap_err();
    assert_eq!(error, EngineError::Cancelled);
    assert_eq!(error.code(), "engine.cancelled");
}

#[test]
fn a_deadline_reached_inside_a_wave_reports_a_timeout() {
    let clock = Arc::new(ManualClock::new());
    // Every decision charges five seconds, so the wave that follows the
    // first decision is already past the one-second budget.
    let classifier = Arc::new(ClockAdvancing::new(Arc::clone(&clock), Duration::from_secs(5)));
    let engine = engine_with_clock(config(4), clock, classifier).unwrap();
    let request = request_with_limit(&five_candidates(), Duration::from_secs(1));

    match engine.decide(&request).unwrap_err() {
        EngineError::Timeout { limit_ms, .. } => assert_eq!(limit_ms, 1_000),
        other => panic!("expected a timeout, got {other:?}"),
    }
}

#[test]
fn a_verifier_that_cannot_answer_fails_the_run() {
    // 0.7 confidence falls in the verify band, so the verifier runs.
    let primary = two_way(("local-small", 0.7), ("cloud-large", 0.3));
    let engine = DecisionEngine::new(
        config(1),
        Arc::new(SystemClock),
        primary,
        Some(Arc::new(BrokenVerifier) as Arc<dyn Classifier>),
    )
    .unwrap();
    let request = choice_request(&[("local-small", "small local model"), ("cloud-large", "cloud")]);

    let error = engine.decide(&request).unwrap_err();
    assert_eq!(error.code(), "engine.invalid_distribution");
    assert!(error.to_string().contains("verifier model unavailable"), "{error}");
}

#[test]
fn a_distribution_over_no_surviving_candidate_is_rejected() {
    // The classifier answers a candidate deterministic narrowing removed;
    // nothing it names survives, which is an error rather than an answer.
    let classifier = Arc::new(
        MockClassifier::new("mock/ghost").with_script("model", vec![("ghost", 1.0)]).unwrap(),
    );
    let engine = engine(classifier, 1).unwrap();
    let request = choice_request(&[("local-small", "small local model")]);

    let error = engine.decide(&request).unwrap_err();
    assert_eq!(error.code(), "engine.invalid_distribution");
    assert!(error.to_string().contains("no key overlaps"), "{error}");
}

#[test]
fn a_survivor_with_no_mass_becomes_certain_rather_than_dropped() {
    // One key names a removed candidate, the survivor carries zero mass, so
    // the surviving set is renormalized onto what is left: the answer stays
    // decidable and the clip is reported in the trace.
    let classifier = Arc::new(
        MockClassifier::new("mock/degenerate")
            .with_script("model", vec![("ghost", 1.0), ("local-small", 0.0)])
            .unwrap(),
    );
    let engine = engine(classifier, 1).unwrap();
    let request = choice_request(&[("local-small", "small local model")]);

    let (response, report) = engine.decide_with_report(&request).unwrap();
    assert_eq!(trace_int(&response, "choice", "clipped"), Some(&FactValue::Integer(1)));
    assert_eq!(response.outcome(), DecisionOutcome::Accept);
    assert!((response.confidence().calibrated_confidence - 1.0).abs() < 1e-12);
    match response.answers().first() {
        Some(DecisionAnswer::Choice { choice, .. }) => {
            assert_eq!(choice.as_str(), "local-small");
        }
        other => panic!("expected a choice answer, got {other:?}"),
    }
    assert_eq!(report.lexical()[0].scores().len(), 1, "narrowing kept one candidate");
}

#[test]
fn a_graph_without_a_rule_node_decides_with_no_exclusions() {
    // Candidate narrowing reads whatever the rule node produced. Without
    // one, the selectors are empty and narrowing is a no-op — the pipeline
    // degrades instead of failing.
    let pipeline = graph(vec![
        node("normalize", NodeKind::Normalize, &[]),
        node("filter", NodeKind::Filter, &["normalize"]),
        node("lexical", NodeKind::Lexical, &["filter"]),
        node("choice", NodeKind::Choice, &["lexical"]),
        node("threshold", NodeKind::Threshold, &["choice"]).with_threshold(0.8),
        node("output", NodeKind::Output, &["threshold"]),
    ]);
    let engine = engine_with(
        EngineConfig::new(pipeline).with_parallelism(2),
        two_way(("local-small", 0.9), ("cloud-large", 0.1)),
    )
    .unwrap();

    let (response, report) = engine
        .decide_with_report(&choice_request(&[
            ("local-small", "small local model"),
            ("cloud-large", "cloud model"),
        ]))
        .unwrap();
    assert_eq!(response.outcome(), DecisionOutcome::Accept);
    assert_eq!(report.narrowing()[0].removed().len(), 0);
    assert_eq!(report.skipped(), &[]);
    assert!(
        response.trace().entries().iter().all(|entry| entry.node != "rule"),
        "no rule node means no rule trace entry"
    );
}

#[test]
fn a_branch_condition_that_holds_runs_its_dependents() {
    // The sibling case — a condition that does not hold — is covered by
    // `a_branch_that_does_not_fire_skips_its_dependents`; this one pins the
    // other half: a firing branch must leave the graph intact.
    let condition = Condition::FactEquals {
        fact: "privacy".into(),
        value: FactValue::Text("local_only".into()),
    };
    let pipeline = graph(vec![
        node("normalize", NodeKind::Normalize, &[]),
        node("branch", NodeKind::Branch, &["normalize"]).with_condition(condition),
        node("filter", NodeKind::Filter, &["branch"]),
        node("lexical", NodeKind::Lexical, &["filter"]),
        node("choice", NodeKind::Choice, &["lexical"]),
        node("threshold", NodeKind::Threshold, &["choice"]).with_threshold(0.8),
        node("output", NodeKind::Output, &["threshold"]),
    ]);
    let engine = engine_with(
        EngineConfig::new(pipeline).with_parallelism(1),
        two_way(("local-small", 0.9), ("cloud-large", 0.1)),
    )
    .unwrap();

    let state = State::from_text("summarize research")
        .with_fact("privacy", FactValue::Text("local_only".into()));
    let request = common::request(
        state,
        vec![choice_question(
            "model",
            &[("local-small", "small local model"), ("cloud-large", "cloud model")],
        )],
        RequestMetadata::default(),
    );

    let (response, report) = engine.decide_with_report(&request).unwrap();
    assert!(report.skipped().is_empty(), "nothing may be skipped: {:?}", report.skipped());
    assert_eq!(response.answers().len(), 1);
    let fired = response
        .trace()
        .entries()
        .iter()
        .find(|entry| entry.node == "branch")
        .and_then(|entry| entry.detail.get("fired"));
    assert_eq!(fired, Some(&FactValue::Boolean(true)));
    assert_eq!(response.outcome(), DecisionOutcome::Accept);
}

#[test]
fn incoherent_engine_configuration_is_rejected_before_anything_runs() {
    let zero_threads = DecisionEngine::new(
        config(0),
        Arc::new(SystemClock),
        Arc::new(MockClassifier::new("m")),
        None,
    )
    .unwrap_err();
    assert_eq!(zero_threads.code(), "engine.invalid_config");
    assert!(zero_threads.to_string().contains("parallelism"), "{zero_threads}");

    let zero_prune = DecisionEngine::new(
        config(1).with_lexical_prune_limit(Some(0)),
        Arc::new(SystemClock),
        Arc::new(MockClassifier::new("m")),
        None,
    )
    .unwrap_err();
    assert_eq!(zero_prune.code(), "engine.invalid_config");
    assert!(zero_prune.to_string().contains("lexical_prune_limit"), "{zero_prune}");

    // 129 rule nodes plus the output node is 130 — one past the engine's
    // own node limit of 128.
    let oversized = DecisionEngine::new(
        EngineConfig::new(long_chain(129)),
        Arc::new(SystemClock),
        Arc::new(MockClassifier::new("m")),
        None,
    )
    .unwrap_err();
    assert_eq!(oversized.code(), "graph.limit_exceeded");
    match oversized {
        EngineError::GraphTooLarge { nodes, limit } => {
            assert_eq!(nodes, 130);
            assert_eq!(limit, 128);
        }
        other => panic!("expected a graph limit error, got {other:?}"),
    }
}

#[test]
fn a_request_may_carry_a_tighter_node_budget_than_the_engine() {
    let metadata = RequestMetadata {
        limits: Limits { max_graph_nodes: 3, ..Limits::default() },
        ..Default::default()
    };
    let request = common::request(
        State::from_text("Summarize research across many sources"),
        vec![choice_question(
            "model",
            &[("local-small", "small local model"), ("cloud-large", "cloud model")],
        )],
        metadata,
    );
    let engine = engine(Arc::new(MockClassifier::new("mock/budget")), 1).unwrap();

    match engine.decide(&request).unwrap_err() {
        EngineError::GraphTooLarge { nodes, limit } => {
            assert_eq!(nodes, 10, "the built-in pipeline declares ten nodes");
            assert_eq!(limit, 3);
        }
        other => panic!("expected a graph limit error, got {other:?}"),
    }
}

#[test]
fn duplicate_question_ids_are_rejected() {
    // Core allows repeated question ids; the engine does not, because
    // narrowing and traces are keyed by question id.
    let request = question_request(vec![
        choice_question("model", &[("local-small", "small local model")]),
        choice_question("model", &[("cloud-large", "cloud model")]),
    ]);
    let engine = engine(Arc::new(MockClassifier::new("mock/duplicate")), 1).unwrap();

    let error = engine.decide(&request).unwrap_err();
    assert_eq!(error, EngineError::DuplicateQuestion { question: "model".to_owned() });
    assert_eq!(error.code(), "engine.duplicate_question");
}
