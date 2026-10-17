//! The reranker seam (PLANNING.md §26, §54; DECISIONS.md D21).
//!
//! A reranker is a *second opinion on ordering*: given a query and the
//! candidates that survived deterministic narrowing, it returns the same
//! candidates scored by a stronger (or at least different) signal. It
//! never removes a candidate — the §26 pipeline narrows *before* the
//! reranker, and what the reranker returns is a permutation with scores,
//! not a decision.
//!
//! Two implementations ship:
//!
//! * [`LexicalReranker`] — BM25 over candidate descriptions, the same
//!   deterministic primitive the `lexical` node uses. Zero-ML, the base
//!   binary's default.
//! * [`EmbeddingReranker`] — cosine similarity through an
//!   [`EmbeddingBackend`],
//!   available when the handle was assembled with one. It is chosen
//!   explicitly by the node's `reranker` knob, never by silent fallback.

use std::sync::Arc;

use opencodifier_core::Candidate;
use opencodifier_runtime::EmbeddingBackend;

use crate::error::{EngineError, EngineResult};
use crate::lexical::Bm25Index;

/// One candidate with its reranker score.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredCandidate {
    /// The candidate, unchanged.
    pub candidate: Candidate,
    /// The reranker's score for it. Higher means better; the scale is the
    /// reranker's own (BM25 weights or cosine in `[-1, 1]`).
    pub score: f64,
}

/// The reranking seam of PLANNING.md §26, exactly as sketched there.
///
/// Reranking is never mandatory: a graph that does not name a `rerank`
/// node never invokes one.
pub trait Reranker: std::fmt::Debug + Send + Sync {
    /// Stable identifier of the reranking signal. Surfaces in traces and
    /// (through the embedding model in engine identity) in cache keys.
    #[must_use]
    fn model_id(&self) -> &str;

    /// Scores `candidates` against `query`, best first.
    ///
    /// # Errors
    ///
    /// [`EngineError::BackendFailed`] when an underlying backend call
    /// fails. The candidates themselves are validated upstream, so a
    /// reranker has nothing else to refuse.
    fn rerank(&self, query: &str, candidates: &[Candidate]) -> EngineResult<Vec<ScoredCandidate>>;
}

/// BM25 reranking over candidate descriptions.
///
/// Ties break by candidate id ascending, so the ordering is deterministic
/// even when descriptions score identically.
#[derive(Debug, Clone, Copy, Default)]
pub struct LexicalReranker;

impl Reranker for LexicalReranker {
    fn model_id(&self) -> &'static str {
        "builtin-lexical-v1"
    }

    fn rerank(&self, query: &str, candidates: &[Candidate]) -> EngineResult<Vec<ScoredCandidate>> {
        let index = Bm25Index::new(candidates.iter().map(Candidate::description));
        // The candidate's own position, by enumeration: the former
        // `position()` by id re-scanned the slice once per candidate
        // (B2). Ids are unique upstream (validated at question
        // construction), so the document index is identical.
        let mut scored: Vec<ScoredCandidate> = candidates
            .iter()
            .enumerate()
            .map(|(position, candidate)| ScoredCandidate {
                score: index.score(query, position),
                candidate: candidate.clone(),
            })
            .collect();
        scored.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.candidate.id().as_str().cmp(right.candidate.id().as_str()))
        });
        Ok(scored)
    }
}

/// Cosine-similarity reranking through an embedding backend.
///
/// The query and every candidate description are embedded in a single
/// batch call, preserving order. Vectors are cosine-normalized at
/// comparison time, so backends that do not pre-normalize still get
/// bounded scores; a zero vector scores 0 against everything.
#[derive(Debug)]
pub struct EmbeddingReranker {
    backend: Arc<dyn EmbeddingBackend>,
}

impl EmbeddingReranker {
    /// Wraps `backend`.
    #[must_use]
    pub fn new(backend: Arc<dyn EmbeddingBackend>) -> Self {
        Self { backend }
    }
}

impl Reranker for EmbeddingReranker {
    fn model_id(&self) -> &str {
        self.backend.model_id()
    }

    fn rerank(&self, query: &str, candidates: &[Candidate]) -> EngineResult<Vec<ScoredCandidate>> {
        let mut texts: Vec<&str> = vec![query];
        texts.extend(candidates.iter().map(Candidate::description));
        let vectors = self.backend.embed(&texts).map_err(|error| EngineError::BackendFailed {
            model_id: self.backend.model_id().to_owned(),
            reason: error.to_string(),
        })?;
        if vectors.len() != texts.len() {
            return Err(EngineError::BackendFailed {
                model_id: self.backend.model_id().to_owned(),
                reason: format!(
                    "backend returned {} vectors for {} texts",
                    vectors.len(),
                    texts.len()
                ),
            });
        }
        let query_vector = &vectors[0];
        let mut scored: Vec<ScoredCandidate> = candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| ScoredCandidate {
                score: cosine(query_vector, &vectors[index + 1]),
                candidate: candidate.clone(),
            })
            .collect();
        scored.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.candidate.id().as_str().cmp(right.candidate.id().as_str()))
        });
        Ok(scored)
    }
}

/// Cosine similarity, `0.0` when either vector has no magnitude.
pub(crate) fn cosine(left: &[f32], right: &[f32]) -> f64 {
    if left.len() != right.len() {
        return 0.0;
    }
    let mut dot = 0.0_f64;
    let mut left_norm = 0.0_f64;
    let mut right_norm = 0.0_f64;
    for (a, b) in left.iter().zip(right.iter()) {
        let (a, b) = (f64::from(*a), f64::from(*b));
        dot += a * b;
        left_norm += a * a;
        right_norm += b * b;
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        return 0.0;
    }
    dot / (left_norm.sqrt() * right_norm.sqrt())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;
    use opencodifier_runtime::MockEmbeddingBackend;

    fn candidate(id: &str, description: &str) -> Candidate {
        Candidate::new(id, description).unwrap()
    }

    /// The §26 contract: reranking returns the same candidates, permuted
    /// and scored — never a subset.
    #[test]
    fn reranking_permutes_but_never_removes() {
        let candidates = [
            candidate("a", "write Rust code with tests"),
            candidate("b", "draft release notes prose"),
            candidate("c", "run the benchmark suite"),
        ];
        for reranker in [
            &LexicalReranker as &dyn Reranker,
            &EmbeddingReranker::new(Arc::new(
                MockEmbeddingBackend::new("mock-embed-v1", 16).unwrap(),
            )),
        ] {
            let scored = reranker.rerank("write code", &candidates).unwrap();
            assert_eq!(scored.len(), 3, "{}", reranker.model_id());
            let mut ids: Vec<_> = scored.iter().map(|s| s.candidate.id().as_str()).collect();
            ids.sort_unstable();
            assert_eq!(ids, ["a", "b", "c"], "{}", reranker.model_id());
            // Best-first: the scores are non-increasing.
            for pair in scored.windows(2) {
                assert!(pair[0].score >= pair[1].score, "{}", reranker.model_id());
            }
        }
    }

    /// The lexical reranker puts the coding candidate first for a coding
    /// query — the ordering is meaningful, not decorative.
    #[test]
    fn lexical_reranker_orders_by_relevance() {
        let candidates = [
            candidate("prose", "draft release notes prose"),
            candidate("code", "write Rust code with tests"),
        ];
        let scored = LexicalReranker.rerank("write code with tests", &candidates).unwrap();
        assert_eq!(scored[0].candidate.id().as_str(), "code");
    }

    /// Cosine is `0.0` for zero vectors and dimension mismatches — a
    /// degenerate vector abstains from the comparison instead of
    /// poisoning the ordering with NaN.
    #[test]
    fn cosine_survives_degenerate_vectors() {
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 0.0]), 0.0);
        assert_eq!(cosine(&[1.0], &[1.0, 0.0]), 0.0);
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-12);
    }
}
