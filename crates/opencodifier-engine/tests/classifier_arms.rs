//! The classifier seam (`src/classifier.rs`) pinned across the crate
//! boundary (PLANNING.md §13, §43, D28).
//!
//! `src/classifier.rs` carries unit tests, but they sit next to the code
//! they pin. These tests re-pin the public contract from outside the crate:
//! the mock's script arms and its "no opinion" uniform, the lexical rung's
//! scoring paths and its coverage evidence, the build-once index handoff
//! (#80), and the model identity every cache key folds in.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

mod common;

use opencodifier_core::{CandidateId, DecisionQuestion, State};
use opencodifier_engine::{Bm25Index, Classifier, EngineError, LexicalClassifier, MockClassifier};

use common::{boolean_question, choice_question, score_question};

/// The two-candidate choice question the scoring tests use: the state text
/// below names the second candidate's description, not the first's.
fn two_candidate_question() -> DecisionQuestion {
    choice_question(
        "model",
        &[("local-small", "small local coding model"), ("cloud-large", "cloud research model")],
    )
}

#[test]
fn an_unscripted_question_answers_uniformly_over_its_own_answers() {
    // The "no opinion" arm of the mock: a question with no script entry is
    // answered uniformly over *that question's* answer keys — candidate ids
    // for a choice, true/false for a boolean, level labels for a score.
    let mock = MockClassifier::new("mock/uniform");
    let state = State::from_text("anything at all");

    let choice = mock.decide(&state, &two_candidate_question()).unwrap();
    assert_eq!(choice.entries().len(), 2, "one entry per candidate");
    assert_eq!(choice.probability_of("local-small"), Some(0.5));
    assert_eq!(choice.probability_of("cloud-large"), Some(0.5));

    let boolean =
        mock.decide(&state, &boolean_question("tools", "Does this request need tools?")).unwrap();
    assert_eq!(boolean.probability_of("true"), Some(0.5));
    assert_eq!(boolean.probability_of("false"), Some(0.5));

    let score = mock
        .decide(&state, &score_question("difficulty", &["trivial", "complex", "expert"]))
        .unwrap();
    assert_eq!(score.entries().len(), 3, "one entry per level");
    for label in ["trivial", "complex", "expert"] {
        assert_eq!(score.probability_of(label), Some(1.0 / 3.0), "{label}");
    }
}

#[test]
fn a_scripted_entry_wins_and_other_questions_stay_uniform() {
    // The lookup arm: a scripted question answers exactly its script, an
    // unscripted one falls back to uniform, and both arms are pure — the
    // same pair answers identically twice.
    let mock = MockClassifier::new("mock/scripted")
        .with_script("model", vec![("cloud-large", 0.9), ("local-small", 0.1)])
        .unwrap();
    assert_eq!(mock.scripted(), 1, "one scripted question");
    let state = State::from_text("irrelevant: the mock never reads the input");

    let scripted = mock.decide(&state, &two_candidate_question()).unwrap();
    assert_eq!(scripted.top().key, "cloud-large");
    assert_eq!(scripted.top().probability, 0.9);

    let unscripted = mock.decide(&state, &boolean_question("tools", "tools?")).unwrap();
    assert_eq!(unscripted.probability_of("true"), Some(0.5), "no entry for `tools`");
    assert_eq!(mock.decide(&state, &two_candidate_question()).unwrap(), scripted, "deterministic");
}

#[test]
fn a_script_that_is_not_a_distribution_is_refused() {
    // `with_script` validates its pairs, so a biased mock cannot be built
    // by accident: the refusal is an engine error carrying the core
    // validation code, not a silently wrong distribution.
    let short = MockClassifier::new("m").with_script("model", vec![("a", 0.5)]);
    let over = MockClassifier::new("m").with_script("model", vec![("a", 0.6), ("b", 0.6)]);
    for error in [short.unwrap_err(), over.unwrap_err()] {
        assert!(matches!(error, EngineError::Core(_)), "{error:?}");
        assert_eq!(error.code(), "ir.invalid", "a core validation refusal");
    }
}

#[test]
fn the_mock_rung_exposes_no_lexical_support() {
    // D28: a rung without lexical evidence reports `None` — never a made-up
    // coverage — so the lexical-band OOD channel has nothing to read from
    // it, whether or not an index was handed over.
    let mock = MockClassifier::new("mock/plain");
    let prebuilt = Bm25Index::new(["small local coding model", "cloud research model"]);
    for index in [None, Some(&prebuilt)] {
        let (distribution, support) =
            mock.decide_extended(&State::from_text("x"), &two_candidate_question(), index).unwrap();
        assert_eq!(support, None, "no lexical evidence from the mock");
        assert_eq!(
            distribution,
            mock.decide(&State::from_text("x"), &two_candidate_question()).unwrap(),
            "the trait default delegates to `decide` bit for bit",
        );
    }
}

#[test]
fn the_lexical_query_is_the_question_text_then_the_state_text() {
    // The evidence the lexical rung scores against is composed the same way
    // for every question kind: question text, one space, state text.
    let state = State::from_text("refactor the parser");
    assert_eq!(
        LexicalClassifier::query_for(&state, &two_candidate_question()),
        "Which model should answer this request? refactor the parser",
    );
    assert_eq!(
        LexicalClassifier::query_for(&state, &boolean_question("tools", "tools needed?")),
        "tools needed? refactor the parser",
    );
}

#[test]
fn the_lexical_rung_ranks_supported_candidates_and_normalizes() {
    // Choice scoring: BM25 over candidate descriptions, softmaxed into a
    // normalized distribution in candidate order — the candidate the state
    // text lexically supports wins, deterministically.
    let classifier = LexicalClassifier::new();
    let state = State::from_text("a cloud research model with long context");
    let question = two_candidate_question();

    let first = classifier.decide(&state, &question).unwrap();
    let sum: f64 = first.entries().iter().map(|entry| entry.probability).sum();
    assert!((sum - 1.0).abs() < 1e-12, "normalized, got {sum}");
    assert!(
        first.probability_of("cloud-large").unwrap() > first.probability_of("local-small").unwrap(),
        "{first:?}"
    );
    assert_eq!(first.probability_of("cloud-large").unwrap(), first.top().probability);
    assert_eq!(classifier.decide(&state, &question).unwrap(), first, "deterministic");
}

#[test]
fn the_lexical_model_id_is_versioned_and_rekeyable() {
    // The model id rides every cache key (PLANNING.md §64): the built-in is
    // the D35-bumped `v2`, and `with_model_id` is how a changed heuristic
    // invalidates cached lexical decisions instead of replaying them.
    assert_eq!(LexicalClassifier::new().model_id(), "builtin-lexical-v2");
    assert_eq!(LexicalClassifier::new().with_model_id("lexical-v3").model_id(), "lexical-v3");
}

#[test]
fn the_lexical_boolean_hypothesis_reads_support_and_flips_on_negation() {
    // Boolean scoring: the state text is scored against the question's terms
    // as an affirmative hypothesis over a zero null; negators in the
    // question invert the polarity of the same evidence.
    let classifier = LexicalClassifier::new();
    let question = boolean_question("research", "research sources summarization?");
    let supporting = State::from_text("summarize research from many sources");

    let affirmative = classifier.decide(&supporting, &question).unwrap();
    assert!(affirmative.probability_of("true").unwrap() > 0.5, "{affirmative:?}");

    // No lexical overlap: evidence and null are both zero, so the answer is
    // a coin flip — a weak distribution the gate may abstain on, never an
    // error.
    let neutral =
        classifier.decide(&State::from_text("unrelated gibberish zzz"), &question).unwrap();
    assert_eq!(neutral.probability_of("true"), neutral.probability_of("false"));

    let negated = boolean_question("research", "not never without research?");
    let flipped = classifier.decide(&supporting, &negated).unwrap();
    assert!(flipped.probability_of("true").unwrap() < 0.5, "{flipped:?}");
    assert!(flipped.probability_of("false").unwrap() > 0.5, "{flipped:?}");
}

#[test]
fn the_lexical_score_rung_leans_towards_mentioned_levels() {
    // Score scoring: BM25 over level labels, so a state that names a level's
    // term leans towards that level while the distribution stays normalized.
    let classifier = LexicalClassifier::new();
    let score = score_question("difficulty", &["trivial", "complex", "expert"]);
    let state = State::from_text("an expert migration with complex constraints");

    let distribution = classifier.decide(&state, &score).unwrap();
    assert!(
        distribution.probability_of("expert").unwrap()
            > distribution.probability_of("trivial").unwrap(),
        "{distribution:?}"
    );
    let sum: f64 = distribution.entries().iter().map(|entry| entry.probability).sum();
    assert!((sum - 1.0).abs() < 1e-12, "normalized, got {sum}");
}

#[test]
fn decide_extended_reports_coverage_without_moving_the_distribution() {
    // D28: every kind exposes its best document's lexical coverage, and the
    // extended path never prices an answer differently from `decide`.
    let classifier = LexicalClassifier::new();
    let state = State::from_text("summarize research across many sources");
    let questions = [
        two_candidate_question(),
        boolean_question("research", "research sources summarization?"),
        score_question("difficulty", &["trivial", "expert"]),
    ];
    for question in &questions {
        let (extended, support) = classifier.decide_extended(&state, question, None).unwrap();
        let coverage = support.expect("the lexical rung always exposes coverage");
        assert!((0.0..=1.0).contains(&coverage), "{question:?}: coverage {coverage}");
        assert_eq!(extended, classifier.decide(&state, question).unwrap(), "{question:?}");
    }
}

#[test]
fn a_prebuilt_index_over_identical_documents_is_indistinguishable() {
    // #80's build-once handoff, at the classifier: an index the caller
    // already built over the same documents yields the same scores and the
    // same coverage; any other index is ignored, never trusted.
    let classifier = LexicalClassifier::new();
    let state = State::from_text("summarize research");
    let question = two_candidate_question();
    let prebuilt = Bm25Index::new(["small local coding model", "cloud research model"]);

    let (with_index, index_support) =
        classifier.decide_extended(&state, &question, Some(&prebuilt)).unwrap();
    let (local, local_support) = classifier.decide_extended(&state, &question, None).unwrap();
    assert_eq!(with_index, local, "same documents, same scores");
    assert_eq!(index_support, local_support, "same documents, same coverage");

    // A stale index (a different document count) is discarded: a caller's
    // bookkeeping error may not shift the engine's distribution.
    let wrong_length = Bm25Index::new(["unrelated"]);
    let (mismatched, _) =
        classifier.decide_extended(&state, &question, Some(&wrong_length)).unwrap();
    assert_eq!(mismatched, local);
}

#[test]
fn the_key_helpers_bridge_distribution_keys_and_ir_ids() {
    // Boolean distributions are keyed "true"/"false"; the helper is the one
    // spelling of that bridge, and `candidate_id` validates on the way back.
    assert_eq!(opencodifier_engine::classifier::boolean_key(true), "true");
    assert_eq!(opencodifier_engine::classifier::boolean_key(false), "false");

    assert_eq!(
        opencodifier_engine::classifier::candidate_id("local-small").unwrap(),
        CandidateId::new("local-small").unwrap()
    );
    let error = opencodifier_engine::classifier::candidate_id("bad id").unwrap_err();
    assert_eq!(error.code(), "ir.invalid", "an invalid candidate id is a core refusal");
}
