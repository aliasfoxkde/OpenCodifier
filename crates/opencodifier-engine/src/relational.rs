//! The deterministic relational solver: proven answers from extracted
//! facts, delegating everything it cannot prove (PLANNING.md §43 —
//! never invoke a more expensive mechanism when a cheaper one decides,
//! and never let a cheaper one guess).
//!
//! [`RelationalSolver`] wraps any [`Classifier`]. For each choice
//! question it extracts relational facts from the state text
//! ([`crate::facts`]) and runs every operator whose facts are present.
//! Each operator computes over the **whole** extracted structure, either
//! proving one unique answer — by transitive closure, aggregation, or
//! ordering, plain deterministic arithmetic — or abstaining; the proof
//! only stands if its answer is among the question's candidates. The
//! solver answers only when at least one operator proves an answer and
//! every proven answer agrees; anything else delegates to the inner
//! classifier, unchanged.
//!
//! # Why this is safe on hostile input
//!
//! Facts enter only through the exact grammar in [`crate::facts`]; the
//! operators then do closed-form set/array work over those facts, never
//! over raw text. Proofs are computed globally before the candidate
//! check, so a candidate set that narrowed the true answer away is
//! answered with a delegation, never with a second-best guess.
//!
//! The three operators are general ops-analytics primitives — root cause
//! over a dependency graph, healthiest group by count, first to restore
//! under ordering constraints — not fixtures for any particular
//! question: they fire on whatever question has the facts that make a
//! proof possible, and they must agree when several fire at once.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use opencodifier_core::{
    Candidate, CandidateId, ChoiceQuestion, DecisionQuestion, Distribution, State,
};

use crate::classifier::{Classifier, LexicalClassifier};
use crate::error::EngineError;
use crate::facts::{Health, RelationalFact, extract};

/// A classifier that answers what it can prove and delegates the rest.
#[derive(Debug)]
pub struct RelationalSolver {
    inner: Arc<dyn Classifier>,
    model_id: String,
}

impl RelationalSolver {
    /// Wraps `inner`: proofs are tried first, delegation is the fallback.
    ///
    /// The model id composes both layers (solver + inner), so a swap or
    /// refit of either produces a distinct cache key (PLANNING.md §64).
    #[must_use]
    pub fn new(inner: Arc<dyn Classifier>) -> Self {
        Self { model_id: format!("relational-v1|{}", inner.model_id()), inner }
    }

    /// The solver over the built-in lexical classifier — the fully
    /// deterministic, zero-ML decision stack.
    #[must_use]
    pub fn lexical() -> Self {
        Self::new(Arc::new(LexicalClassifier::new()))
    }

    /// The unique proven answer for `question`, if the facts prove one.
    ///
    /// Every operator whose facts are present must agree on the same
    /// single candidate; a lone dissent or any ambiguity means no proof.
    fn prove(question: &ChoiceQuestion, state: &State) -> Option<String> {
        let facts = extract(state.text());
        if facts.is_empty() {
            return None;
        }
        let candidates: Vec<&str> =
            question.candidates().iter().map(Candidate::id).map(CandidateId::as_str).collect();
        let proven: Vec<String> = [
            root_cause(&facts, &candidates),
            healthiest(&facts, &candidates),
            first_restored(&facts, &candidates),
        ]
        .into_iter()
        .flatten()
        .collect();
        let first = proven.first()?;
        proven.iter().all(|answer| answer == first).then(|| first.clone())
    }
}

impl Classifier for RelationalSolver {
    /// A proven distribution: mass `1.0` on the proven candidate, `0.0`
    /// on the rest, in candidate order. Everything else — non-choice
    /// questions, no provable answer, a proof outside the candidate set —
    /// delegates to the inner classifier untouched.
    fn decide(
        &self,
        state: &State,
        question: &DecisionQuestion,
    ) -> Result<Distribution, EngineError> {
        let DecisionQuestion::Choice(choice) = question else {
            return self.inner.decide(state, question);
        };
        let Some(answer) = Self::prove(choice, state) else {
            return self.inner.decide(state, question);
        };
        let pairs: Vec<(String, f64)> = choice
            .candidates()
            .iter()
            .map(|candidate| {
                let probability = if candidate.id().as_str() == answer { 1.0 } else { 0.0 };
                (candidate.id().as_str().to_owned(), probability)
            })
            .collect();
        Distribution::from_pairs(pairs).map_err(|error| EngineError::InvalidDistribution {
            question: question.id().to_string(),
            reason: error.to_string(),
        })
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }
}

/// The single member of a single-element list, as an owned string.
fn only(candidates: &[&str]) -> Option<String> {
    match candidates {
        [single] => Some((*single).to_owned()),
        _ => None,
    }
}

/// Root cause (dependency graphs): the unique failing node that has
/// dependents and is not itself caused by another failing node, provided
/// it is a candidate.
///
/// With edges `dependent depends on dependency`, a failure at `d` breaks
/// everything that transitively depends on `d`. The root cause of a set
/// of failures is the failing node nothing else failing leads to: it has
/// at least one dependent, and no *other* failing node is among its
/// transitive dependencies. Two such nodes, or none, means no proof.
fn root_cause(facts: &[RelationalFact], candidates: &[&str]) -> Option<String> {
    let mut dependents_of: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut dependencies_of: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut failing: BTreeSet<&str> = BTreeSet::new();
    for fact in facts {
        match fact {
            RelationalFact::DependsOn { dependent, dependency } => {
                dependents_of.entry(dependency.as_str()).or_default().push(dependent.as_str());
                dependencies_of.entry(dependent.as_str()).or_default().push(dependency.as_str());
            }
            RelationalFact::Health { entity, status: Health::Down } => {
                failing.insert(entity.as_str());
            }
            _ => {}
        }
    }
    if failing.is_empty() || dependents_of.is_empty() {
        return None;
    }
    // Every transitive dependency of `start`, via DFS.
    let transitive_dependencies = |start: &str| -> BTreeSet<&str> {
        let mut seen = BTreeSet::new();
        let mut stack: Vec<&str> = dependencies_of.get(start).cloned().unwrap_or_default();
        while let Some(next) = stack.pop() {
            if seen.insert(next) {
                stack.extend(dependencies_of.get(next).cloned().unwrap_or_default());
            }
        }
        seen
    };
    let roots: Vec<&str> = failing
        .iter()
        .copied()
        .filter(|node| {
            dependents_of.get(*node).is_some_and(|deps| !deps.is_empty())
                && !transitive_dependencies(node)
                    .into_iter()
                    .any(|dependency| dependency != *node && failing.contains(dependency))
        })
        .collect();
    let root = only(roots.as_slice())?;
    candidates.contains(&root.as_str()).then_some(root)
}

/// Healthiest group (aggregation): the unique entity with the most
/// `healthy` reports, strictly ahead of every other entity and a
/// candidate.
///
/// Counts come from both forms — "X is healthy" and "X: healthy,
/// healthy, down" — so single-entity statuses and group reports
/// aggregate uniformly. A tie, a board where nobody is healthy, or a
/// winner outside the candidate set means no proof.
fn healthiest(facts: &[RelationalFact], candidates: &[&str]) -> Option<String> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for fact in facts {
        match fact {
            RelationalFact::Health { entity, status: Health::Healthy } => {
                *counts.entry(entity.as_str()).or_default() += 1;
            }
            RelationalFact::HealthList { entity, statuses } => {
                let healthy = statuses.iter().filter(|status| **status == Health::Healthy).count();
                *counts.entry(entity.as_str()).or_default() += healthy;
            }
            _ => {}
        }
    }
    let mut ranked: Vec<(&str, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let (best, best_count) = ranked.first().copied()?;
    // Strictly ahead of the runner-up, and actually healthy at all.
    if best_count == 0 || ranked.len() > 1 && best_count <= ranked[1].1 {
        return None;
    }
    candidates.contains(&best).then(|| best.to_owned())
}

/// First to restore (ordering): the unique entity that gates at least
/// one other component, waits for none, and is a candidate.
///
/// "`later` comes back online only after `earlier`" means `later` waits
/// on `earlier`. The first component online is one that appears as
/// `earlier` somewhere — it actually gates something — and never as
/// `later`. Two such entities, none, or a winner outside the candidate
/// set means no proof.
fn first_restored(facts: &[RelationalFact], candidates: &[&str]) -> Option<String> {
    let mut waits_on_anything: BTreeSet<&str> = BTreeSet::new();
    let mut gates_something: BTreeSet<&str> = BTreeSet::new();
    for fact in facts {
        if let RelationalFact::RestoresAfter { later, earlier } = fact {
            waits_on_anything.insert(later.as_str());
            gates_something.insert(earlier.as_str());
        }
    }
    let first: Vec<&str> = gates_something.difference(&waits_on_anything).copied().collect();
    let first = only(first.as_slice())?;
    candidates.contains(&first.as_str()).then(|| first.clone())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use opencodifier_core::{BooleanQuestion, Candidate, ChoiceQuestion, State};

    use super::*;
    use crate::classifier::MockClassifier;

    fn choice(candidates: &[&str]) -> ChoiceQuestion {
        ChoiceQuestion::new(
            "q",
            "Which single component?",
            candidates
                .iter()
                .map(|name| Candidate::new((*name).to_owned(), format!("component {name}")))
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
        )
        .unwrap()
    }

    fn decide(solver: &RelationalSolver, text: &str, candidates: &[&str]) -> Distribution {
        solver
            .decide(&State::from_text(text), &DecisionQuestion::Choice(choice(candidates)))
            .unwrap()
    }

    #[test]
    fn proves_the_root_cause_of_a_dependency_chain() {
        let solver = RelationalSolver::lexical();
        let text = "gateway depends on auth. billing depends on gateway. \
                    auth is failing. catalog is failing.";
        let distribution = decide(&solver, text, &["auth", "billing", "catalog", "dispatch"]);
        assert_eq!(distribution.top().key, "auth");
        assert_eq!(distribution.top().probability, 1.0);
        assert_eq!(distribution.entries().len(), 4, "every candidate stays in the distribution");
    }

    #[test]
    fn proves_the_healthiest_region_by_count() {
        let solver = RelationalSolver::lexical();
        let text = "north: healthy, healthy, degraded. south: healthy, down, down. \
                    east: down, down, degraded. west: degraded, degraded, down.";
        let distribution = decide(&solver, text, &["north", "south", "east", "west"]);
        assert_eq!(distribution.top().key, "north");
        assert_eq!(distribution.top().probability, 1.0);
    }

    #[test]
    fn proves_the_first_component_to_restore() {
        let solver = RelationalSolver::lexical();
        let text = "index comes back online only after feed. \
                    feed comes back online only after auth. \
                    export comes back online only after auth.";
        let distribution = decide(&solver, text, &["auth", "feed", "index", "export"]);
        assert_eq!(distribution.top().key, "auth");
        assert_eq!(distribution.top().probability, 1.0);
    }

    #[test]
    fn delegates_untouched_when_nothing_is_provable() {
        let inner = Arc::new(MockClassifier::new("mock/test"));
        let solver = RelationalSolver::new(inner.clone());
        let text = "Refactor the parser and add tests. The parser module is 400 lines.";
        let question = DecisionQuestion::Choice(choice(&["a", "b"]));
        let state = State::from_text(text);
        assert_eq!(
            solver.decide(&state, &question).unwrap(),
            inner.decide(&state, &question).unwrap()
        );
    }

    #[test]
    fn delegates_when_the_global_proof_is_outside_the_candidates() {
        let solver = RelationalSolver::lexical();
        let text = "gateway depends on auth. billing depends on gateway. \
                    auth is failing. catalog is failing.";
        // auth is the proven root cause but is not offered: no
        // second-best guess.
        let distribution = decide(&solver, text, &["billing", "gateway", "catalog"]);
        assert!(distribution.top().probability < 1.0, "must delegate, not prove");
    }

    #[test]
    fn abstains_when_two_root_causes_qualify() {
        let solver = RelationalSolver::lexical();
        let text = "b depends on a. c depends on d. a is failing. d is failing.";
        // a and d are both failing with dependents and no failing cause.
        let distribution = decide(&solver, text, &["a", "b", "c", "d"]);
        assert!(distribution.top().probability < 1.0);
    }

    #[test]
    fn abstains_when_the_healthiest_is_a_tie() {
        let solver = RelationalSolver::lexical();
        let text = "north: healthy, healthy, down. south: healthy, healthy, down.";
        let distribution = decide(&solver, text, &["north", "south"]);
        assert!(distribution.top().probability < 1.0);
    }

    #[test]
    fn abstains_when_a_non_candidate_would_win() {
        let solver = RelationalSolver::lexical();
        let text = "north: healthy, healthy, healthy. south: healthy, down.";
        // north wins globally but is not on the ballot.
        let distribution = decide(&solver, text, &["south", "east"]);
        assert!(distribution.top().probability < 1.0);
    }

    #[test]
    fn disagreeing_operators_decline() {
        let solver = RelationalSolver::lexical();
        // healthiest proves "a" (2 healthy vs 1); first_restored proves
        // "b" (a waits on b, b gates something and waits for nothing).
        // Two different proofs: no answer.
        let text = "a comes back online only after b. a: healthy, healthy. b: healthy. c: down.";
        let distribution = decide(&solver, text, &["a", "b", "c"]);
        assert!(distribution.top().probability < 1.0, "conflicting proofs must delegate");
    }

    #[test]
    fn agreeing_operators_answer() {
        let solver = RelationalSolver::lexical();
        // root_cause and first_restored both prove "auth"; healthiest has
        // no healthy reports and abstains.
        let text = "gateway depends on auth. auth is failing. \
                    gateway comes back online only after auth.";
        let distribution = decide(&solver, text, &["auth", "gateway", "billing"]);
        assert_eq!(distribution.top().key, "auth");
        assert_eq!(distribution.top().probability, 1.0);
    }

    #[test]
    fn boolean_and_score_questions_delegate() {
        let inner = Arc::new(MockClassifier::new("mock/test"));
        let solver = RelationalSolver::new(inner.clone());
        let state = State::from_text("auth is healthy.");
        let boolean =
            DecisionQuestion::Boolean(BooleanQuestion::new("b", "Is auth healthy?").unwrap());
        assert_eq!(
            solver.decide(&state, &boolean).unwrap(),
            inner.decide(&state, &boolean).unwrap()
        );
    }

    #[test]
    fn model_id_composes_inner_and_solver() {
        let solver = RelationalSolver::lexical();
        assert!(solver.model_id().starts_with("relational-v1|"));
        assert!(solver.model_id().ends_with(LexicalClassifier::new().model_id()));
        let other = RelationalSolver::new(Arc::new(MockClassifier::new("mock/x")));
        assert_ne!(solver.model_id(), other.model_id(), "inner swaps must change the cache key");
    }

    #[test]
    fn repeated_decisions_are_bit_identical() {
        let solver = RelationalSolver::lexical();
        let text = "gateway depends on auth. billing depends on gateway. \
                    auth is failing. catalog is failing.";
        let one = decide(&solver, text, &["auth", "billing", "catalog", "dispatch"]);
        let two = decide(&solver, text, &["auth", "billing", "catalog", "dispatch"]);
        assert_eq!(one, two);
    }

    #[test]
    fn the_solver_decides_where_bare_lexical_only_orders() {
        // The regression this module exists for: on relational state text
        // the wrapped stack commits to the proven answer; bare BM25 only
        // produces a soft ordering (no candidate reaches certainty).
        let text = "gateway depends on auth. billing depends on gateway. \
                    auth is failing. catalog is failing.";
        let question =
            DecisionQuestion::Choice(choice(&["auth", "billing", "catalog", "dispatch"]));
        let state = State::from_text(text);
        let bare = LexicalClassifier::new().decide(&state, &question).unwrap();
        let wrapped = RelationalSolver::lexical().decide(&state, &question).unwrap();
        assert!(bare.top().probability < 1.0);
        assert_eq!(wrapped.top().key, "auth");
        assert_eq!(wrapped.top().probability, 1.0);
    }
}
