//! Candidate narrowing (PLANNING.md §45).
//!
//! Never feed every candidate to a semantic stage when deterministic
//! constraints can eliminate some first. This module owns the bookkeeping
//! for that: which candidates were removed, how many survived, and the
//! reduction ratio reported in metrics.
//!
//! # Safe mode
//!
//! [`NarrowingOutcome::apply`] only ever removes candidates named by
//! deterministic rules. Lexical scores are recorded — they drive ordering
//! and the decision stage — but under safe mode they never eliminate
//! anything. Lexical pruning exists only through the opt-in
//! [`EngineConfig::with_lexical_prune_limit`](crate::engine::EngineConfig::with_lexical_prune_limit)
//! knob and is rejected by the engine when safe mode is on (PLANNING.md
//! §45: never eliminate based solely on weak semantic evidence).

use opencodifier_core::{Candidate, CandidateId, ChoiceQuestion, QuestionId};

use crate::lexical::Bm25Index;

/// The result of narrowing one choice question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NarrowingOutcome {
    question_id: QuestionId,
    before: usize,
    after: usize,
    removed: Vec<CandidateId>,
    surviving: Vec<Candidate>,
}

impl NarrowingOutcome {
    /// Applies rule exclusions to a choice question's candidate set.
    ///
    /// Candidates named by `pins` survive even when `exclusions` names
    /// them: include wins over exclude, deterministically. Order of both
    /// lists is preserved in the report so the trace explains *why* each
    /// candidate went.
    #[must_use]
    pub fn apply(
        question: &ChoiceQuestion,
        exclusions: &[CandidateId],
        pins: &[CandidateId],
    ) -> Self {
        let mut removed: Vec<CandidateId> = Vec::new();
        for candidate in exclusions {
            if pins.contains(candidate) || removed.contains(candidate) {
                continue;
            }
            removed.push(candidate.clone());
        }
        let surviving: Vec<Candidate> = question
            .candidates()
            .iter()
            .filter(|candidate| !removed.contains(candidate.id()))
            .cloned()
            .collect();
        Self {
            question_id: question.id().clone(),
            before: question.candidates().len(),
            after: surviving.len(),
            removed,
            surviving,
        }
    }

    /// The question this outcome belongs to.
    #[must_use]
    pub fn question_id(&self) -> &QuestionId {
        &self.question_id
    }

    /// Candidates visible before narrowing.
    #[must_use]
    pub fn before(&self) -> usize {
        self.before
    }

    /// Candidates that survived narrowing.
    #[must_use]
    pub fn after(&self) -> usize {
        self.after
    }

    /// Candidates removed, in exclusion order.
    #[must_use]
    pub fn removed(&self) -> &[CandidateId] {
        &self.removed
    }

    /// The surviving candidates, in request order.
    #[must_use]
    pub fn surviving(&self) -> &[Candidate] {
        &self.surviving
    }

    /// `true` when no candidate survived, which means the question cannot
    /// be decided (`NoValidCandidate`).
    #[must_use]
    pub fn is_starved(&self) -> bool {
        self.after == 0
    }

    /// `after / before`; `1.0` when nothing was narrowed.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn reduction_ratio(&self) -> f64 {
        if self.before == 0 { 1.0 } else { self.after as f64 / self.before as f64 }
    }

    /// Rebuilds the question restricted to its surviving candidates.
    ///
    /// `None` when the question starved: a choice question must carry at
    /// least one candidate, so a starved question is reported rather than
    /// silently reconstructed.
    #[must_use]
    pub fn narrowed_question(&self, question: &ChoiceQuestion) -> Option<ChoiceQuestion> {
        ChoiceQuestion::new(question.id().as_str(), question.text(), self.surviving.clone()).ok()
    }
}

/// Lexical scores for the surviving candidates of one question.
#[derive(Debug, Clone, PartialEq)]
pub struct LexicalScores {
    question_id: QuestionId,
    scores: Vec<(CandidateId, f64)>,
    pruned: Vec<CandidateId>,
}

impl LexicalScores {
    /// Scores `candidates` against `query` with BM25.
    ///
    /// Results are ordered by score descending, then by candidate id
    /// ascending, so the ordering is deterministic even when scores tie.
    #[must_use]
    pub fn score(question: &ChoiceQuestion, candidates: &[Candidate], query: &str) -> Self {
        let documents: Vec<&str> =
            candidates.iter().map(opencodifier_core::Candidate::description).collect();
        let index = Bm25Index::new(documents);
        let raw = index.score_all(query);
        let mut scores: Vec<(CandidateId, f64)> = candidates
            .iter()
            .cloned()
            .zip(raw)
            .map(|(candidate, score)| (candidate.id().clone(), score))
            .collect();
        scores.sort_by(|left, right| right.1.total_cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
        Self { question_id: question.id().clone(), scores, pruned: Vec::new() }
    }

    /// The question these scores belong to.
    #[must_use]
    pub fn question_id(&self) -> &QuestionId {
        &self.question_id
    }

    /// Scores, best first.
    #[must_use]
    pub fn scores(&self) -> &[(CandidateId, f64)] {
        &self.scores
    }

    /// Candidates dropped by a lexical prune (empty under safe mode).
    #[must_use]
    pub fn pruned(&self) -> &[CandidateId] {
        &self.pruned
    }

    /// The highest score, if any candidates were scored.
    #[must_use]
    pub fn top_score(&self) -> Option<f64> {
        self.scores.first().map(|(_, score)| *score)
    }

    /// Drops candidates ranked worse than `keep`, returning the outcome.
    ///
    /// Ties at the cut are kept: the last retained slot may hold several
    /// equally-scored candidates, because removing a candidate on a score
    /// tie would be inventing evidence.
    ///
    /// This is the *only* lexical elimination path and it is gated: the
    /// engine refuses to call it in safe mode.
    #[must_use]
    pub fn prune(mut self, keep: usize) -> Self {
        if keep == 0 || self.scores.len() <= keep {
            return self;
        }
        let threshold = self.scores[keep - 1].1;
        let mut pruned = Vec::new();
        self.scores.retain(|(id, score)| {
            if *score >= threshold {
                true
            } else {
                pruned.push(id.clone());
                false
            }
        });
        self.pruned = pruned;
        self
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;
    use opencodifier_core::Candidate;

    fn question() -> ChoiceQuestion {
        ChoiceQuestion::new(
            "model",
            "Which model should answer?",
            vec![
                Candidate::new("cloud-large", "long context cloud reasoning").unwrap(),
                Candidate::new("local-small", "small local coding model").unwrap(),
                Candidate::new("local-tiny", "tiny local summarizer").unwrap(),
            ],
        )
        .unwrap()
    }

    fn id(value: &str) -> CandidateId {
        CandidateId::new(value).unwrap()
    }

    #[test]
    fn narrowing_removes_only_excluded_candidates() {
        let outcome =
            NarrowingOutcome::apply(&question(), &[id("local-tiny"), id("cloud-large")], &[]);
        assert_eq!(outcome.before(), 3);
        assert_eq!(outcome.after(), 1);
        assert_eq!(outcome.removed(), &[id("local-tiny"), id("cloud-large")]);
        assert_eq!(outcome.surviving().len(), 1);
        assert_eq!(outcome.surviving()[0].id(), &id("local-small"));
        assert!((outcome.reduction_ratio() - 1.0 / 3.0).abs() < 1e-12);
        assert!(!outcome.is_starved());
    }

    #[test]
    fn narrowing_deduplicates_and_honours_pins() {
        let outcome = NarrowingOutcome::apply(
            &question(),
            &[id("local-tiny"), id("local-tiny"), id("local-small")],
            &[id("local-tiny")],
        );
        assert_eq!(outcome.removed(), &[id("local-small")], "pinned candidates survive");
        assert_eq!(outcome.after(), 2);
    }

    #[test]
    fn empty_exclusions_change_nothing() {
        let outcome = NarrowingOutcome::apply(&question(), &[], &[]);
        assert_eq!(outcome.before(), outcome.after());
        assert_eq!(outcome.reduction_ratio(), 1.0);
        assert!(outcome.removed().is_empty());
        assert_eq!(outcome.narrowed_question(&question()), Some(question()));
    }

    #[test]
    fn starving_produces_no_rebuilt_question() {
        let all: Vec<CandidateId> =
            question().candidates().iter().map(|c| c.id().clone()).collect();
        let outcome = NarrowingOutcome::apply(&question(), &all, &[]);
        assert!(outcome.is_starved());
        assert_eq!(outcome.reduction_ratio(), 0.0);
        assert!(outcome.narrowed_question(&question()).is_none());
    }

    #[test]
    fn lexical_scores_order_best_first_with_deterministic_ties() {
        let scores =
            LexicalScores::score(&question(), question().candidates(), "small local coding model");
        let ordered: Vec<CandidateId> = scores.scores().iter().map(|(id, _)| id.clone()).collect();
        assert_eq!(ordered[0], id("local-small"), "exact description match must rank first");
        // Remaining candidates are ordered by score descending, never by
        // accident.
        assert!(scores.scores()[1].1 >= scores.scores()[2].1);
        assert!(scores.top_score().unwrap() > 0.0, "scores must be positive");
    }

    #[test]
    fn lexical_prune_keeps_top_ranked_and_all_ties() {
        let scores =
            LexicalScores::score(&question(), question().candidates(), "small local coding model");
        let pruned = scores.clone().prune(1);
        assert!(pruned.pruned().contains(&id("local-tiny")));
        assert!(pruned.pruned().contains(&id("cloud-large")));
        assert_eq!(pruned.scores().len(), 1);
        assert_eq!(pruned.scores()[0].0, id("local-small"));

        // A tie spanning the cut boundary keeps every tied candidate.
        let tied = LexicalScores {
            question_id: QuestionId::new("model").unwrap(),
            scores: vec![(id("a"), 1.0), (id("b"), 0.5), (id("c"), 0.5)],
            pruned: Vec::new(),
        };
        let kept = tied.clone().prune(2);
        assert_eq!(kept.scores().len(), 3, "candidates tied at the cut must survive");
        assert!(kept.pruned().is_empty());

        // The same tie below the cut is eliminated with the rest.
        let cut = tied.prune(1);
        assert_eq!(cut.scores().len(), 1);
        assert_eq!(cut.pruned().len(), 2);
    }

    #[test]
    fn lexical_prune_is_a_no_op_when_generous_or_zero() {
        let scores = LexicalScores::score(&question(), question().candidates(), "coding");
        assert_eq!(scores.clone().prune(10).scores().len(), 3);
        assert_eq!(scores.clone().prune(0).scores().len(), 3, "keep=0 means 'no pruning'");
        assert!(scores.clone().prune(0).pruned().is_empty());
    }

    #[test]
    fn lexical_scores_are_reproducible() {
        let first = LexicalScores::score(&question(), question().candidates(), "cloud reasoning");
        let second = LexicalScores::score(&question(), question().candidates(), "cloud reasoning");
        assert_eq!(first, second);
    }
}
