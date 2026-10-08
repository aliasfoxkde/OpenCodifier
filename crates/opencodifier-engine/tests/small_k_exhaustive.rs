//! Exhaustive small-answer-set verification (RESEARCH.md §15.8).
//!
//! `AlphaDev`'s correctness argument for its fast small-sort networks was
//! exhaustion: for the input sizes that matter, every permutation is
//! checked, so no property test has to hope it sampled the interesting
//! case. The engine's decision kernels have the same shape — a categorical
//! distribution over a handful of answers — so the small sets are verified
//! the same way here: every score-to-candidate assignment for sets of two
//! through six, every candidate presentation order, every boolean
//! confidence, and every score-level permutation.
//!
//! Exhaustion is the point: these kernels sit under every decision the
//! engine ships, and the whole file costs a few seconds.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

mod common;

use std::sync::Arc;

use common::{boolean_question, engine, question_request, score_question};
use opencodifier_core::{DecisionAnswer, Distribution};
use opencodifier_engine::{Classifier, DecisionEngine, MockClassifier};

/// Weights with one unambiguous maximum; a permutation test's expectation
/// is always "the winner is the candidate the largest weight landed on".
const WEIGHTS: [f64; 6] = [0.40, 0.25, 0.15, 0.10, 0.06, 0.04];

/// Answer-set positions as expected-value weights; a lookup keeps the
/// score-level arithmetic cast-free.
const POSITIONS: [f64; 4] = [0.0, 1.0, 2.0, 3.0];

/// Distinct ids and descriptions for answer sets of up to six.
const CANDIDATES: [(&str, &str); 6] = [
    ("c0", "alpha research summarizer"),
    ("c1", "beta code writer"),
    ("c2", "gamma long context model"),
    ("c3", "delta fast responder"),
    ("c4", "epsilon careful checker"),
    ("c5", "zeta cheap embedder"),
];

/// Every permutation of `items`, in lexicographic order of the recursion.
fn permutations<T: Clone>(items: &[T]) -> Vec<Vec<T>> {
    if items.len() <= 1 {
        return vec![items.to_vec()];
    }
    let mut out = Vec::new();
    for index in 0..items.len() {
        let mut rest = items.to_vec();
        rest.remove(index);
        for mut tail in permutations(&rest) {
            let mut whole = vec![items[index].clone()];
            whole.append(&mut tail);
            out.push(whole);
        }
    }
    out
}

/// An engine whose `model` answer is `pairs` — the scripted distribution
/// the decision stage must reproduce through clip and calibration.
fn scripted(pairs: Vec<(&str, f64)>) -> DecisionEngine {
    let classifier: Arc<dyn Classifier> =
        Arc::new(MockClassifier::new("mock/exhaustive").with_script("model", pairs).unwrap());
    engine(classifier, 1).unwrap()
}

/// The first answer as `(choice, confidence)`.
fn choice_answer(response: &opencodifier_core::DecisionResponse) -> (&str, f64) {
    match response.answers().first().unwrap() {
        DecisionAnswer::Choice { choice, confidence, .. } => (choice.as_str(), *confidence),
        other => panic!("expected a choice answer, got {other:?}"),
    }
}

/// `weights` truncated to `k` and renormalized — the mass the script
/// carries for the first `k` candidates.
fn normalized_prefix(k: usize) -> Vec<f64> {
    let slice = &WEIGHTS[..k];
    let total: f64 = slice.iter().sum();
    slice.iter().map(|weight| weight / total).collect()
}

#[test]
fn every_score_assignment_for_two_through_six_candidates() {
    for k in 2..=6 {
        let weights = normalized_prefix(k);
        let best = weights
            .iter()
            .enumerate()
            .max_by(|left, right| left.1.total_cmp(right.1))
            .map_or(usize::MAX, |(index, _)| index);
        let assignments = permutations(&(0..k).collect::<Vec<_>>());
        assert_eq!(assignments.len(), factorial(k), "k = {k}: every permutation is exercised");
        for assignment in &assignments {
            // `assignment[i]` is the weight index candidate `i` receives;
            // the winner is whichever candidate holds `best`.
            let winner = assignment.iter().position(|weight| *weight == best).unwrap_or(0);
            let pairs: Vec<(&str, f64)> = (0..k)
                .map(|candidate| (CANDIDATES[candidate].0, weights[assignment[candidate]]))
                .collect();
            let engine = scripted(pairs);
            let request = common::choice_request(&CANDIDATES[..k]);
            let response = engine.decide(&request).unwrap();

            let (choice, confidence) = choice_answer(&response);
            assert_eq!(
                choice, CANDIDATES[winner].0,
                "k = {k}, assignment {assignment:?}: the largest weight wins"
            );
            let expected = weights[best];
            assert!(
                (confidence - expected).abs() < 1e-12,
                "k = {k}, assignment {assignment:?}: confidence {confidence} != {expected}"
            );
        }
    }
}

#[test]
fn every_candidate_presentation_order_yields_the_same_decision() {
    // The script is fixed; only the order candidates are declared in
    // varies. The decision must not.
    let pairs = vec![("c1", 0.50), ("c0", 0.30), ("c3", 0.15), ("c2", 0.05)];
    for order in permutations(&[0usize, 1, 2, 3]) {
        let candidates: Vec<(&str, &str)> = order.iter().map(|index| CANDIDATES[*index]).collect();
        let engine = scripted(pairs.clone());
        let request = common::choice_request(&candidates);
        let response = engine.decide(&request).unwrap();

        let (choice, confidence) = choice_answer(&response);
        assert_eq!(choice, "c1", "order {order:?}: the 0.50 candidate wins");
        assert!((confidence - 0.50).abs() < 1e-12, "order {order:?}: confidence {confidence}");
    }
}

#[test]
fn every_boolean_confidence_decides_the_same_way_from_either_side() {
    let grid: Vec<f64> = (0..=20).map(|step| f64::from(step) / 20.0).collect();
    for p in grid {
        for pairs in [vec![("true", p), ("false", 1.0 - p)], vec![("false", 1.0 - p), ("true", p)]]
        {
            let classifier: Arc<dyn Classifier> = Arc::new(
                MockClassifier::new("mock/exhaustive").with_script("flag", pairs.clone()).unwrap(),
            );
            let engine = engine(classifier, 1).unwrap();
            let request = question_request(vec![boolean_question("flag", "should this run?")]);
            let response = engine.decide(&request).unwrap();

            let expected = if p == 0.5 { None } else { Some(p > 0.5) };
            match response.answers().first().unwrap() {
                DecisionAnswer::Boolean { value, probability, confidence, .. } => {
                    if let Some(expected) = expected {
                        assert_eq!(*value, expected, "p = {p}, pairs {pairs:?}");
                        let top = p.max(1.0 - p);
                        assert!(
                            (*probability - top).abs() < 1e-12,
                            "p = {p}: probability {probability}"
                        );
                    }
                    // The exact tie must at least decide identically from
                    // both script orders — the kernel is order-insensitive.
                    assert!(
                        (*confidence - *probability).abs() < 1e-12,
                        "p = {p}: confidence is the top probability"
                    );
                }
                other => panic!("expected a boolean answer, got {other:?}"),
            }
        }
    }
}

#[test]
fn every_score_level_assignment_keeps_expected_value_consistent() {
    let levels = ["trivial", "moderate", "expert", "critical"];
    let weights = normalized_prefix(4);
    for assignment in permutations(&[0usize, 1, 2, 3]) {
        // `assignment[i]` is the weight index level `i` receives.
        let probs: Vec<f64> = (0..4).map(|level| weights[assignment[level]]).collect();
        let pairs: Vec<(&str, f64)> =
            levels.iter().zip(&probs).map(|(label, weight)| (*label, *weight)).collect();
        let classifier: Arc<dyn Classifier> = Arc::new(
            MockClassifier::new("mock/exhaustive").with_script("depth", pairs.clone()).unwrap(),
        );
        let engine = engine(classifier, 1).unwrap();
        let request = question_request(vec![score_question("depth", &levels)]);
        let response = engine.decide(&request).unwrap();

        // Mirror the engine's arithmetic exactly: `clip` renormalizes by
        // the fp-sum in entry order, and an EV a ulp under 1.0 floors to a
        // different level than one a ulp over. Summing in the same order
        // keeps the expectation bit-identical to what the engine computes.
        let total: f64 = probs.iter().sum();
        let expected_value: f64 = probs
            .iter()
            .zip(POSITIONS)
            .map(|(probability, position)| (probability / total) * position)
            .sum();
        // The shipped level is the expected value's floor bracket, not the
        // highest-probability level: `level_for` maps EV = 1.05 to the
        // second level even when the top mass sits elsewhere. `rposition`
        // over the position weights recovers the floor without a cast.
        let bracket = POSITIONS.iter().rposition(|start| expected_value >= *start).unwrap_or(0);
        match response.answers().first().unwrap() {
            DecisionAnswer::Score { expected, level, .. } => {
                assert_eq!(*level, levels[bracket], "assignment {assignment:?}: EV floor wins");
                assert!(
                    (*expected - expected_value).abs() < 1e-15,
                    "assignment {assignment:?}: expected value {} != {expected_value}",
                    *expected
                );
            }
            other => panic!("expected a score answer, got {other:?}"),
        }
    }
}

#[test]
fn every_assignment_survives_a_hallucinated_key_with_mass_renormalized() {
    // A scripted key that is not a candidate is dropped by `clip`; the
    // surviving mass renormalizes, and no permutation of the survivors
    // changes that arithmetic.
    let allowed = [0.40f64, 0.25, 0.15, 0.10];
    let survivors = allowed.iter().sum::<f64>();
    for assignment in permutations(&[0usize, 1, 2, 3]) {
        let mut pairs: Vec<(&str, f64)> = (0..4)
            .map(|candidate| (CANDIDATES[candidate].0, allowed[assignment[candidate]]))
            .collect();
        pairs.push(("ghost", 1.0 - survivors));
        let engine = scripted(pairs);
        let request = common::choice_request(&CANDIDATES[..4]);
        let response = engine.decide(&request).unwrap();

        let (choice, confidence) = choice_answer(&response);
        // The 0.40 weight lands on whichever candidate `assignment` sends
        // it to; that candidate wins at 0.40/0.9.
        let winner = assignment.iter().position(|weight| *weight == 0).unwrap_or(0);
        assert_eq!(
            choice, CANDIDATES[winner].0,
            "assignment {assignment:?}: the 0.40 candidate wins"
        );
        let expected = allowed[0] / survivors;
        assert!(
            (confidence - expected).abs() < 1e-9,
            "assignment {assignment:?}: renormalized {confidence} != {expected}"
        );
        assert_eq!(
            common::trace_int(&response, "choice", "clipped"),
            Some(&opencodifier_core::FactValue::Integer(1)),
            "assignment {assignment:?}: the dropped key is counted, not hidden"
        );
    }
}

/// `k!`, for the exhaustion assertion.
fn factorial(k: usize) -> usize {
    (1..=k).product()
}

/// A distribution built from `pairs` keeps its supplied order — the
/// invariant the permutation tests above lean on for determinism.
#[test]
fn scripted_distributions_preserve_supplied_order() {
    let distribution = Distribution::from_pairs([("b", 0.5), ("a", 0.5)]).unwrap();
    let keys: Vec<&str> = distribution.entries().iter().map(|entry| entry.key.as_str()).collect();
    assert_eq!(keys, ["b", "a"]);
}
