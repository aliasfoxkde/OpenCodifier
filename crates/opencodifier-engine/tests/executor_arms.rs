//! Executor arms the rest of the suite reaches only from one side: the
//! outcome names the threshold trace ships, the transitive skip closure,
//! the whole run-report shape, the candidate order a rerank node hands the
//! decider, and the per-kind answer sets a distribution is clipped to.
//!
//! Every test drives the real engine through the public surface, so the
//! assertion covers the whole pipeline rather than a slice of it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

mod common;

use std::sync::Arc;

use common::{
    boolean_question, choice_question, config, engine, engine_with, node, question_request, rules,
    score_question, state_with_fact, trace_fact, trace_int,
};
use opencodifier_core::{
    Candidate, CandidateId, ChoiceQuestion, DecisionAnswer, DecisionOutcome, DecisionQuestion,
    FactValue, NodeId, State,
};
use opencodifier_engine::{
    Classifier, Condition, DecisionEngine, DecisionGraph, EngineConfig, EngineError,
    MockClassifier, NodeKind, SystemClock,
};

/// The `outcome` fact of the threshold trace entry, if recorded.
fn gated_outcome(response: &opencodifier_core::DecisionResponse) -> String {
    trace_fact(response, "threshold", "outcome")
        .and_then(FactValue::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// An engine over the built-in pipeline whose `model` answer puts `top` on
/// `local-small`, with an identically-answering verifier when `verifying`.
/// Valid only while `top` is the larger share of a two-candidate question.
fn gated(top: f64, verifying: bool) -> DecisionEngine {
    let classifier = common::two_way(("local-small", top), ("cloud-large", 1.0 - top));
    let verifier: Option<Arc<dyn Classifier>> = if verifying {
        Some(common::two_way(("local-small", top), ("cloud-large", 1.0 - top)))
    } else {
        None
    };
    DecisionEngine::new(config(1), Arc::new(SystemClock), classifier, verifier).unwrap()
}

/// An engine whose `model` answer tops out at 0.4 over three candidates —
/// under the 0.50 abstain floor, which a two-candidate question cannot
/// reach (`top` is the maximum of the distribution).
fn abstaining() -> DecisionEngine {
    let classifier = Arc::new(
        MockClassifier::new("mock/abstaining")
            .with_script(
                "model",
                vec![("local-small", 0.4), ("cloud-large", 0.35), ("local-tiny", 0.25)],
            )
            .unwrap(),
    );
    DecisionEngine::new(config(1), Arc::new(SystemClock), classifier, None).unwrap()
}

/// A classifier that answers the first candidate it is handed with all the
/// decisive mass, so whatever order the decision stage sees is exactly the
/// answer it gives back. Exactly two candidates are supported — the shape
/// every question in this file carries.
#[derive(Debug)]
struct FirstAnswer;

impl Classifier for FirstAnswer {
    fn decide(
        &self,
        _state: &State,
        question: &DecisionQuestion,
    ) -> Result<opencodifier_core::Distribution, EngineError> {
        let DecisionQuestion::Choice(choice) = question else {
            return Err(EngineError::InvalidDistribution {
                question: question.id().to_string(),
                reason: "only choice questions are decided here".to_owned(),
            });
        };
        match choice.candidates() {
            [first, second] => opencodifier_core::Distribution::from_pairs(vec![
                (first.id().as_str(), 0.9),
                (second.id().as_str(), 0.1),
            ])
            .map_err(|error| EngineError::InvalidDistribution {
                question: question.id().to_string(),
                reason: error.to_string(),
            }),
            _ => Err(EngineError::InvalidDistribution {
                question: question.id().to_string(),
                reason: "this fixture decides exactly two candidates".to_owned(),
            }),
        }
    }

    fn model_id(&self) -> &'static str {
        "test/first-answer"
    }
}

/// The two-candidate question the rerank arm decides, with descriptions
/// the lexical reranker separates on the words `write`, `code`, `tests`.
fn coding_question() -> DecisionQuestion {
    let candidates: Vec<Candidate> = vec![
        Candidate::new("prose", "draft release notes prose").unwrap(),
        Candidate::new("code", "write Rust code with tests").unwrap(),
    ];
    DecisionQuestion::Choice(
        ChoiceQuestion::new("model", "write code with tests", candidates).unwrap(),
    )
}

/// `normalize -> choice -> threshold -> output`: the same decision stage
/// with no reranker in front of it.
fn plain_decision_graph() -> DecisionGraph {
    common::graph(vec![
        node("normalize", NodeKind::Normalize, &[]),
        node("choice", NodeKind::Choice, &["normalize"]),
        node("gate", NodeKind::Threshold, &["choice"]).with_threshold(0.8),
        node("output", NodeKind::Output, &["gate"]),
    ])
}

#[test]
fn the_threshold_trace_names_the_outcome_it_gated() {
    let two = &[("local-small", "small local model"), ("cloud-large", "cloud model")];
    // 0.9 clears the 0.80 gate, 0.7 needs verification: the two gate
    // verdicts a two-candidate question can reach.
    let accept = gated(0.9, false).decide(&common::choice_request(two)).unwrap();
    assert_eq!(accept.outcome(), DecisionOutcome::Accept);
    assert_eq!(gated_outcome(&accept), "accept");

    let verify = gated(0.7, false).decide(&common::choice_request(two)).unwrap();
    assert_eq!(verify.outcome(), DecisionOutcome::Verify);
    assert_eq!(gated_outcome(&verify), "verify");

    // 0.4 is under the 0.50 abstain floor.
    let three = &[
        ("local-small", "small local model"),
        ("cloud-large", "cloud model"),
        ("local-tiny", "tiny local model"),
    ];
    let abstain = abstaining().decide(&common::choice_request(three)).unwrap();
    assert_eq!(abstain.outcome(), DecisionOutcome::Abstain);
    assert_eq!(gated_outcome(&abstain), "abstain");

    // A verifier that agrees turns the same 0.7 into a verified answer,
    // and the trace says so under the fourth reachable name.
    let verified = gated(0.7, true).decide(&common::choice_request(two)).unwrap();
    assert_eq!(verified.outcome(), DecisionOutcome::Verified);
    assert_eq!(gated_outcome(&verified), "verified");
}

#[test]
fn a_whole_skipped_wave_leaves_only_skip_markers_behind() {
    // The branch conditions on a fact the request does not carry, so it
    // does not fire: everything downstream of it is skipped transitively,
    // and each skipped wave is skipped whole.
    let graph = common::graph(vec![
        node("normalize", NodeKind::Normalize, &[]),
        node("branch", NodeKind::Branch, &["normalize"]).with_condition(Condition::FactEquals {
            fact: "mode".into(),
            value: FactValue::Text("never".into()),
        }),
        node("score", NodeKind::Lexical, &["branch"]),
        node("decide", NodeKind::Choice, &["score"]),
        node("gate", NodeKind::Threshold, &["decide"]).with_threshold(0.8),
        node("output", NodeKind::Output, &["gate"]),
    ]);
    let engine = engine_with(
        EngineConfig::new(graph).with_parallelism(1),
        Arc::new(MockClassifier::new("mock/test")),
    )
    .unwrap();
    let request = common::request(
        State::from_text("summarize research"),
        vec![choice_question("model", &[("local-small", "small local model")])],
        opencodifier_core::RequestMetadata::default(),
    );

    let (response, report) = engine.decide_with_report(&request).unwrap();
    // The closure is the whole tail: every transitive dependent of the
    // branch, and nothing else.
    let skipped: Vec<&str> = report.skipped().iter().map(NodeId::as_str).collect();
    assert_eq!(skipped, vec!["decide", "gate", "output", "score"]);

    // Exactly one marker per skipped node, carrying nothing but the skip.
    for id in &skipped {
        let entries: Vec<_> = response.trace().entries().iter().filter(|e| e.node == *id).collect();
        assert_eq!(entries.len(), 1, "{id} was traced more than once");
        assert_eq!(entries[0].detail.len(), 1, "{id} recorded extra facts");
        assert_eq!(entries[0].detail.get("skipped"), Some(&FactValue::Boolean(true)));
    }

    // The rest of the trace is the branch that declined to fire plus the
    // entry node — no decision, no reach marker for the skipped tail.
    let nodes: std::collections::BTreeSet<&str> =
        response.trace().entries().iter().map(|entry| entry.node.as_str()).collect();
    assert_eq!(nodes.len(), 6, "unexpected trace nodes: {nodes:?}");
    assert_eq!(trace_fact(&response, "branch", "fired"), Some(&FactValue::Boolean(false)));
    assert_eq!(trace_fact(&response, "output", "reached"), None);

    // A skipped tail is not a failure: the run succeeds, answers nothing,
    // and declines per question rather than guessing.
    assert_eq!(response.answers(), []);
    assert_eq!(response.outcome(), DecisionOutcome::Abstain);
    assert_eq!(
        report.outcomes().iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
        vec!["model"]
    );
    assert_eq!(report.outcomes()[0].1, DecisionOutcome::Abstain);
}

#[test]
fn the_run_report_is_one_runs_full_diagnostics_surface() {
    // Two choice questions declared out of id order. A rule that names the
    // tag `cloud-large` narrows `beta` to one candidate and leaves `alpha`
    // (whose second candidate mentions no excluded tag) alone, so the two
    // questions report different narrowing under one run.
    let narrowed = config(2).with_rules(rules(vec![common::exclude_tag_rule(
        "no cloud",
        "privacy",
        "local_only",
        "cloud-large",
    )]));
    let classifier = Arc::new(
        MockClassifier::new("mock/two-question")
            .with_script("beta", vec![("local-small", 0.9), ("cloud-large", 0.1)])
            .unwrap()
            .with_script("alpha", vec![("local-small", 0.6), ("cloud-small", 0.4)])
            .unwrap(),
    );
    let engine = engine_with(narrowed, classifier).unwrap();
    let request = common::request(
        state_with_fact("privacy", "local_only"),
        vec![
            choice_question(
                "beta",
                &[("local-small", "small local model"), ("cloud-large", "cloud model")],
            ),
            choice_question(
                "alpha",
                &[("local-small", "small local model"), ("cloud-small", "cloud model")],
            ),
        ],
        opencodifier_core::RequestMetadata::default(),
    );

    let (response, report) = engine.decide_with_report(&request).unwrap();
    // The built-in pipeline decomposes into seven waves, exactly two of
    // which hold more than one runnable node ({cache, rule} and
    // {boolean, filter, score}) and therefore run in parallel.
    assert_eq!(report.waves().len(), 7, "waves: {:?}", report.waves());
    assert_eq!(report.parallel_waves(), 2);
    assert!(report.threads().len() >= 2, "threads: {:?}", report.threads());
    // Nothing was skipped and no focus policy was configured.
    assert_eq!(report.skipped(), [], "nothing was skipped: {:?}", report.skipped());
    let focus = report.focus();
    assert_eq!((focus.decided, focus.engaged, focus.escalated), (0, 0, 0));
    // A first run is a miss under a non-empty key.
    assert!(!report.cache_hit());
    let key = report.cache_key().map(opencodifier_engine::CacheKey::as_hex).unwrap();
    // A SHA-256 digest as hex: the whole identity in 64 characters.
    assert_eq!(key.len(), 64, "{key}");

    // Narrowing and lexical evidence are keyed by question id, whatever
    // order the request declared.
    let narrowed: std::collections::BTreeMap<&str, &opencodifier_engine::NarrowingOutcome> =
        report.narrowing().iter().map(|o| (o.question_id().as_str(), o)).collect();
    let alpha = narrowed["alpha"];
    assert_eq!((alpha.before(), alpha.after()), (2, 2), "alpha matched no exclusion");
    assert_eq!(alpha.removed(), []);
    let beta = narrowed["beta"];
    assert_eq!((beta.before(), beta.after()), (2, 1));
    assert_eq!(beta.removed().iter().map(CandidateId::as_str).collect::<Vec<_>>(), ["cloud-large"]);

    let lexical: std::collections::BTreeMap<&str, &opencodifier_engine::LexicalScores> =
        report.lexical().iter().map(|s| (s.question_id().as_str(), s)).collect();
    assert_eq!(lexical["alpha"].scores().len(), 2);
    assert_eq!(lexical["beta"].scores().len(), 1, "only the surviving candidate is scored");
    for scores in lexical.values() {
        assert_eq!(scores.pruned().len(), 0, "a rule exclusion is not a lexical prune");
        assert!(scores.top_score().is_some_and(f64::is_finite));
    }

    // Outcomes, by contrast, follow request order — the two orders differ
    // here on purpose. `beta` narrowed to one candidate, so its clipped
    // answer is certain and accepts; `alpha` decided over both at 0.6.
    let outcomes: Vec<(&str, DecisionOutcome)> =
        report.outcomes().iter().map(|(id, outcome)| (id.as_str(), *outcome)).collect();
    assert_eq!(outcomes, [("beta", DecisionOutcome::Accept), ("alpha", DecisionOutcome::Verify)]);
    assert_eq!(
        response.outcome(),
        DecisionOutcome::Verify,
        "the run is the least decisive question"
    );

    // An unchanged identity replays the same key as a hit.
    let (_, replay) = engine.decide_with_report(&request).unwrap();
    assert!(replay.cache_hit());
    assert_eq!(replay.cache_key().map(opencodifier_engine::CacheKey::as_hex), Some(key));
    assert_eq!(engine.cache().len().unwrap(), 1);
}

#[test]
fn a_rerank_node_reorders_the_view_the_decider_sees() {
    // Control: the decider sees the declared order and answers its first
    // candidate.
    let control =
        engine_with(EngineConfig::new(plain_decision_graph()), Arc::new(FirstAnswer)).unwrap();
    let request = common::request(
        State::from_text("write code with tests"),
        vec![coding_question()],
        opencodifier_core::RequestMetadata::default(),
    );
    let response = control.decide(&request).unwrap();
    let plain = match response.answers().first().unwrap() {
        DecisionAnswer::Choice { choice, .. } => choice.as_str().to_owned(),
        other => panic!("expected a choice answer, got {other:?}"),
    };
    assert_eq!(plain, "prose", "the declared order is what the decider saw");

    // With a lexical rerank node in front, the same decider answers the
    // reranker's first candidate instead: the order is applied, not just
    // reported.
    let reranked_graph = common::graph(vec![
        node("normalize", NodeKind::Normalize, &[]),
        node("rerank", NodeKind::Rerank, &["normalize"]).with_reranker("lexical"),
        node("choice", NodeKind::Choice, &["rerank"]),
        node("gate", NodeKind::Threshold, &["choice"]).with_threshold(0.8),
        node("output", NodeKind::Output, &["gate"]),
    ]);
    let reranked = engine_with(EngineConfig::new(reranked_graph), Arc::new(FirstAnswer)).unwrap();
    let response = reranked.decide(&request).unwrap();
    let order =
        trace_fact(&response, "rerank", "order").and_then(FactValue::as_str).unwrap_or_default();
    assert!(order.starts_with("code,"), "the coding candidate ranked first: {order}");
    assert_eq!(
        match response.answers().first().unwrap() {
            DecisionAnswer::Choice { choice, .. } => choice.as_str().to_owned(),
            other => panic!("expected a choice answer, got {other:?}"),
        },
        "code",
        "the decider answered in the reranked order"
    );
}

#[test]
fn clipping_is_bound_to_each_kinds_answer_set() {
    // Every script names one key that does not exist for its kind: the
    // invalid key is dropped, the remainder carries all the mass, and the
    // clip is counted in the trace rather than hidden.
    let classifier = Arc::new(
        MockClassifier::new("mock/clipper")
            .with_script("model", vec![("ghost", 0.8), ("local-small", 0.2)])
            .unwrap()
            .with_script("tools", vec![("maybe", 0.8), ("true", 0.2)])
            .unwrap()
            .with_script("difficulty", vec![("extreme", 0.5), ("trivial", 0.2), ("expert", 0.3)])
            .unwrap(),
    );
    let engine = engine(classifier, 1).unwrap();
    let request = question_request(vec![
        choice_question(
            "model",
            &[("local-small", "small local model"), ("cloud-large", "cloud model")],
        ),
        boolean_question("tools", "Does this request need tools?"),
        score_question("difficulty", &["trivial", "moderate", "expert"]),
    ]);

    let (response, report) = engine.decide_with_report(&request).unwrap();
    for node_id in ["choice", "boolean", "score"] {
        assert_eq!(
            trace_int(&response, node_id, "clipped"),
            Some(&FactValue::Integer(1)),
            "{node_id} did not report the dropped key"
        );
    }

    // Choice: a removed candidate is not an answer, so the surviving
    // candidate takes the whole distribution.
    match &response.answers()[0] {
        DecisionAnswer::Choice { choice, confidence, .. } => {
            assert_eq!(choice.as_str(), "local-small");
            assert!(
                (*confidence - 1.0).abs() < 1e-12,
                "renormalized to certainty, got {confidence}"
            );
        }
        other => panic!("expected a choice answer, got {other:?}"),
    }
    // Boolean: "maybe" is not a boolean answer; the surviving hypothesis
    // is certain and the probability is the clipped one, not the raw 0.2.
    match &response.answers()[1] {
        DecisionAnswer::Boolean { value, probability, confidence, .. } => {
            assert!(value);
            assert!(
                (*probability - 1.0).abs() < 1e-12,
                "renormalized to certainty, got {probability}"
            );
            assert!((*confidence - 1.0).abs() < 1e-12);
        }
        other => panic!("expected a boolean answer, got {other:?}"),
    }
    // Score: "extreme" is not a level, so it is dropped and the remaining
    // pair renormalized (trivial 0.4, expert 0.6). Expected value weights
    // distribution *positions*, not level indices: 0.4·0 + 0.6·1 = 0.6,
    // which maps back to the first level.
    match &response.answers()[2] {
        DecisionAnswer::Score { expected, level, confidence, .. } => {
            assert!((*expected - 0.6).abs() < 1e-12, "expected {expected}");
            assert_eq!(level, "trivial");
            assert!((*confidence - 0.6).abs() < 1e-12, "the top probability, got {confidence}");
        }
        other => panic!("expected a score answer, got {other:?}"),
    }
    // Choice and boolean landed on certainty and accept; the score
    // question's top probability is 0.6, so it verifies — and the run is
    // only as decisive as that.
    let outcomes: Vec<(&str, DecisionOutcome)> =
        report.outcomes().iter().map(|(id, outcome)| (id.as_str(), *outcome)).collect();
    assert_eq!(
        outcomes,
        [
            ("model", DecisionOutcome::Accept),
            ("tools", DecisionOutcome::Accept),
            ("difficulty", DecisionOutcome::Verify)
        ]
    );
    assert_eq!(response.outcome(), DecisionOutcome::Verify);
}
