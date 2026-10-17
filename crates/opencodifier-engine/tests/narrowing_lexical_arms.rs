//! Candidate narrowing and lexical scoring arms (PLANNING.md §43, §45).
//!
//! Narrowing is the deterministic cut and lexical scoring is the cheapest
//! evidence layer above it, so every contract here is one a downstream
//! stage depends on: include-wins exclusion arithmetic, the narrowed
//! question rebuild, exact BM25 values, tie ordering, the prune cut, and
//! the index the lexical node hands to the deciding classifier (#80).
//! Each test drives the real engine or the public scorer — never a test
//! double of the arithmetic — and asserts values, not line coverage.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

mod common;

use std::sync::{Arc, Mutex};

use common::{choice_request_with, config, engine_with, five_candidates, graph, node, rules};
use opencodifier_core::{
    Candidate, CandidateId, ChoiceQuestion, DecisionQuestion, Distribution, FactValue, State,
};
use opencodifier_engine::lexical::{B, K1};
use opencodifier_engine::{
    Action, Bm25Index, Classifier, Condition, EngineConfig, EngineError, LexicalClassifier,
    LexicalScores, MockClassifier, NarrowingOutcome, NodeKind, Rule,
};

/// A validated candidate id, for building exclusion and pin lists.
fn cid(value: &str) -> CandidateId {
    CandidateId::new(value).unwrap()
}

/// A choice question over `(id, description)` pairs.
fn question(candidates: &[(&str, &str)]) -> ChoiceQuestion {
    let candidates: Vec<Candidate> = candidates
        .iter()
        .map(|(id, description)| Candidate::new(*id, *description).unwrap())
        .collect();
    ChoiceQuestion::new("model", "Which model should answer?", candidates).unwrap()
}

#[test]
fn a_rule_pin_outweighs_a_rule_exclusion_end_to_end() {
    // The pin rule and the exclude rule fire on the same fact, so the
    // filter node resolves a real conflict: `cloud-large` is excluded by
    // tag and pinned by id at the same time.
    let conflicting = rules(vec![
        Rule {
            name: Some("keep the research specialist".to_owned()),
            when: Condition::FactExists { fact: "mode".to_owned() },
            then: vec![Action::IncludeCandidate { id: Some("cloud-large".to_owned()), tag: None }],
        },
        Rule {
            name: Some("no cloud at all".to_owned()),
            when: Condition::FactExists { fact: "mode".to_owned() },
            then: vec![Action::ExcludeCandidate { id: None, tag: Some("cloud".to_owned()) }],
        },
    ]);
    let engine =
        engine_with(config(1).with_rules(conflicting), Arc::new(MockClassifier::new("mock/pin")))
            .unwrap();
    let request =
        choice_request_with(common::state_with_fact("mode", "strict"), &five_candidates());

    let (response, report) = engine.decide_with_report(&request).unwrap();
    let narrowed = &report.narrowing()[0];
    // Five declared; the tag removes two cloud candidates and the pin
    // saves one of them, so exactly one candidate is gone.
    assert_eq!(narrowed.before(), 5);
    assert_eq!(narrowed.after(), 4);
    let removed: Vec<&str> = narrowed.removed().iter().map(CandidateId::as_str).collect();
    assert_eq!(removed, ["cloud-fast"], "the pinned candidate survives the tag exclusion");
    let surviving: Vec<&str> = narrowed.surviving().iter().map(|c| c.id().as_str()).collect();
    assert_eq!(surviving, ["local-tiny", "local-small", "local-large", "cloud-large"]);
    assert!(!narrowed.is_starved());
    assert!((narrowed.reduction_ratio() - 0.8).abs() < 1e-12);

    // The rule node reports both selectors it collected; the filter node
    // reports the net effect of applying them.
    assert_eq!(common::trace_int(&response, "rule", "excluded"), Some(&FactValue::Integer(2)));
    assert_eq!(common::trace_int(&response, "rule", "pinned"), Some(&FactValue::Integer(1)));
    assert_eq!(common::trace_int(&response, "filter", "removed"), Some(&FactValue::Integer(1)));
    assert_eq!(response.metrics().candidates_in, 5);
    assert_eq!(response.metrics().candidates_out, 4);
}

#[test]
fn the_narrowed_question_is_the_original_over_its_survivors() {
    let original = question(&[
        ("alpha", "fast local edits"),
        ("beta", "cloud research model"),
        ("gamma", "small local summarizer"),
    ]);
    // `beta` and `gamma` are both excluded, but `gamma` is pinned, so the
    // outcome must report one removal and two survivors in request order.
    let outcome = NarrowingOutcome::apply(
        &original,
        &[cid("beta"), cid("gamma"), cid("beta")],
        &[cid("gamma")],
    );

    assert_eq!(outcome.question_id(), original.id());
    assert_eq!(outcome.before(), 3);
    assert_eq!(outcome.after(), 2);
    let removed: Vec<&str> = outcome.removed().iter().map(CandidateId::as_str).collect();
    assert_eq!(removed, ["beta"], "duplicates collapse and the pin rescues `gamma`");
    assert!((outcome.reduction_ratio() - 2.0 / 3.0).abs() < 1e-12);

    // The rebuild is the same question restricted to the survivors, never
    // reordered and never re-identified.
    let rebuilt = outcome.narrowed_question(&original).expect("two candidates survive");
    assert_eq!(rebuilt.id(), original.id());
    assert_eq!(rebuilt.text(), original.text());
    let ids: Vec<&str> = rebuilt.candidates().iter().map(|c| c.id().as_str()).collect();
    assert_eq!(ids, ["alpha", "gamma"], "request order is preserved, not score order");
}

#[test]
fn bm25_scores_match_the_documented_okapi_formula() {
    // One-term corpus: "coding" appears once in a one-token document and
    // twice in a two-token document, so idf, both lengths, and the average
    // length are all hand-checkable.
    let documents = question(&[("once", "coding"), ("twice", "coding coding")]);
    let scores = LexicalScores::score(&documents, documents.candidates(), "coding");

    // `df == N == 2`: the term is in every document, so
    // idf = ln(1 + (2 - 2 + 0.5) / (2 + 0.5)) = ln(1.2) — small, but never
    // negative.
    let idf = (1.0 + 0.5 / 2.5f64).ln();
    // Document lengths are 1 and 2 tokens, so the average is 1.5.
    let saturated =
        |tf: f64, length: f64| tf * (K1 + 1.0) / (tf + K1 * (1.0 - B + B * (length / 1.5)));
    let expected_once = idf * saturated(1.0, 1.0);
    let expected_twice = idf * saturated(2.0, 2.0);

    let ranked: Vec<(&str, f64)> =
        scores.scores().iter().map(|(id, score)| (id.as_str(), *score)).collect();
    assert_eq!(ranked.len(), 2);
    assert!(
        (ranked[0].1 - expected_twice).abs() < 1e-12,
        "highest score {} must be the repeated-term document's {}",
        ranked[0].1,
        expected_twice
    );
    assert!((ranked[1].1 - expected_once).abs() < 1e-12);
    assert_eq!(ranked[0].0, "twice");
    assert!(expected_twice > expected_once, "term frequency must outrank document length");
    assert_eq!(scores.question_id().as_str(), "model");
}

#[test]
fn identical_descriptions_tie_and_break_on_candidate_id() {
    let tied = question(&[
        ("zulu", "identical evidence"),
        ("alpha", "identical evidence"),
        ("mike", "identical evidence"),
    ]);
    let scores = LexicalScores::score(&tied, tied.candidates(), "identical evidence");

    let ordered: Vec<&str> = scores.scores().iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ordered, ["alpha", "mike", "zulu"], "score ties break on candidate id");
    assert_eq!(scores.scores()[0].1, scores.scores()[2].1, "identical documents tie exactly");
    assert_eq!(scores.top_score(), Some(scores.scores()[0].1), "top_score is the first score");
    // The scorer always builds the index over the candidates it ranked.
    let index = scores.index().expect("the scorer records the index it built");
    assert_eq!(index.len(), 3);
    assert_eq!(index.document_frequency("identical"), 3);
}

#[test]
fn a_prune_cut_that_lands_on_a_tie_keeps_every_tied_candidate() {
    let documents = question(&[
        ("a", "cloud reasoning"),
        ("b", "cloud reasoning"),
        ("c", "local coding"),
        ("d", "vision understanding"),
    ]);
    let scores = LexicalScores::score(&documents, documents.candidates(), "cloud reasoning");
    assert_eq!(scores.scores()[0].1, scores.scores()[1].1, "a and b describe the same text");
    assert_eq!(scores.scores()[2].1, 0.0, "no overlap means no score");
    assert_eq!(scores.scores()[3].1, 0.0, "so the bottom two tie at zero");

    // A cut of one cannot split the tie for first place.
    let keep_one = scores.clone().prune(1);
    let kept: Vec<&str> = keep_one.scores().iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(kept, ["a", "b"], "candidates tied at the cut all survive");
    let dropped: Vec<&str> = keep_one.pruned().iter().map(CandidateId::as_str).collect();
    assert_eq!(dropped, ["c", "d"], "the pruned list stays in score order");
    assert_eq!(keep_one.top_score(), scores.top_score(), "pruning never moves the top score");

    // A cut of two is the same cut: the tie still spans the boundary.
    let keep_two = scores.clone().prune(2);
    assert_eq!(keep_two.scores().len(), 2);
    assert_eq!(keep_two.pruned().len(), 2);

    // A cut of three lands *on* the zero-score tie, so nothing at all is
    // dropped — a generous cut never invents a preference between ties.
    let keep_three = scores.clone().prune(3);
    assert_eq!(keep_three.scores().len(), 4);
    assert_eq!(keep_three.pruned(), []);
}

/// Captures the index the executor hands the deciding classifier, so the
/// build-once handoff (#80) can be compared by content, not by count.
#[derive(Debug)]
struct IndexCapture {
    /// Where the distribution comes from.
    delegate: MockClassifier,
    /// The last index handed over, if any.
    handed: Mutex<Option<Bm25Index>>,
}

impl IndexCapture {
    /// A capture wrapping the uniform mock.
    fn wrapped() -> Arc<Self> {
        Arc::new(Self { delegate: MockClassifier::new("mock/capture"), handed: Mutex::new(None) })
    }

    /// The index the lexical node handed over, if it did.
    fn handed_index(&self) -> Option<Bm25Index> {
        self.handed.lock().ok().and_then(|slot| slot.clone())
    }
}

impl Classifier for IndexCapture {
    fn decide(
        &self,
        state: &State,
        question: &DecisionQuestion,
    ) -> Result<Distribution, EngineError> {
        self.delegate.decide(state, question)
    }

    fn decide_extended(
        &self,
        state: &State,
        question: &DecisionQuestion,
        index: Option<&Bm25Index>,
    ) -> Result<(Distribution, Option<f64>), EngineError> {
        if let Some(index) = index
            && let Ok(mut slot) = self.handed.lock()
        {
            *slot = Some(index.clone());
        }
        self.delegate.decide_extended(state, question, index)
    }

    fn model_id(&self) -> &str {
        self.delegate.model_id()
    }
}

#[test]
fn the_handed_index_is_the_index_the_report_records() {
    let candidates: Vec<(&str, &str)> = vec![
        ("local-small", "summarize local research notes"),
        ("cloud-large", "cloud archive of research"),
    ];
    // filter -> lexical -> choice: the lexical node builds the index and
    // the choice node reuses it instead of rebuilding (#80).
    let pipeline = graph(vec![
        node("normalize", NodeKind::Normalize, &[]),
        node("rule", NodeKind::Rule, &["normalize"]),
        node("filter", NodeKind::Filter, &["rule"]),
        node("lexical", NodeKind::Lexical, &["filter"]),
        node("choice", NodeKind::Choice, &["lexical"]),
        node("output", NodeKind::Output, &["choice"]),
    ]);
    let capture = IndexCapture::wrapped();
    let engine =
        engine_with(EngineConfig::new(pipeline), Arc::clone(&capture) as Arc<dyn Classifier>)
            .unwrap();

    let state = State::from_text("Summarize research across many sources");
    let request = choice_request_with(state.clone(), &candidates);
    let (_, report) = engine.decide_with_report(&request).unwrap();

    let descriptions: Vec<&str> = candidates.iter().map(|(_, description)| *description).collect();
    let expected = Bm25Index::new(descriptions);
    // The handoff is content-identical to the index built over the
    // surviving candidates, in request order.
    let handed = capture.handed_index().expect("the choice node received the lexical index");
    assert_eq!(handed, expected, "same documents, same order");
    assert_eq!(handed.len(), 2);

    // The report exposes the very same index, so a caller can reproduce
    // the run's lexical evidence without rebuilding anything.
    let recorded = report.lexical()[0].index().expect("the report names the index it built");
    assert_eq!(recorded, &handed);
    assert_eq!(recorded, &expected);

    // And the recorded scores are exactly that index's scores for the
    // query the deciding classifier derives from state plus question text.
    let question = common::choice_question("model", &candidates);
    let query = LexicalClassifier::query_for(&state, &question);
    let expected_scores = handed.score_all(&query);
    assert_eq!(report.lexical()[0].scores().len(), expected_scores.len());
    for (candidate, score) in report.lexical()[0].scores() {
        let position = candidates.iter().position(|(id, _)| *id == candidate.as_str()).unwrap();
        assert!(
            (expected_scores[position] - score).abs() < 1e-12,
            "candidate {candidate} must score {score} in the handed index"
        );
    }
}
