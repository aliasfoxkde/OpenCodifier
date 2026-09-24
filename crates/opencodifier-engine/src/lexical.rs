//! Hand-written BM25 lexical scoring — the engine's built-in semantic-ish
//! rung (PLANNING.md §43, §45).
//!
//! This is a *lexical* layer, not a semantic model: it measures term
//! overlap between candidate descriptions and state text with Okapi BM25.
//! It exists so the decision cascade has a cheap middle rung between exact
//! rules and the (future, Phase 6+) embedding/classifier layers, and so
//! the engine is useful with zero ML dependency.
//!
//! Properties that matter here:
//!
//! * **Deterministic.** No randomness, no parallel reduction, no I/O.
//!   Scores are summed in sorted term order, so the same inputs produce
//!   bit-identical scores on every run and platform.
//! * **Bounded.** Tokenization caps term length and drops stop words, so a
//!   hostile description cannot blow up the index
//!   (PLANNING.md §66).
//! * **Honest.** Lexical overlap is weak evidence. That is exactly why
//!   [`crate::engine::EngineConfig::safe_mode`] forbids eliminating
//!   candidates on it (PLANNING.md §45).

/// BM25 term-frequency saturation parameter (`k1`).
pub const K1: f64 = 1.2;

/// BM25 length-normalization parameter (`b`).
pub const B: f64 = 0.75;

/// Tokens longer than this are dropped, not indexed.
const MAX_TOKEN_LEN: usize = 32;

/// Negation words that flip the polarity of a boolean hypothesis.
///
/// Deliberately *not* stop words: the boolean lexical hypothesis reads
/// them, so they must survive tokenization.
pub const NEGATORS: [&str; 7] = ["not", "no", "never", "without", "cannot", "neither", "nor"];

/// Small deterministic English stop-word list.
///
/// Closed and hand-written on purpose: a learned or configurable list
/// would change scores between builds and break score reproducibility.
pub const STOP_WORDS: [&str; 40] = [
    "a", "an", "the", "and", "or", "but", "if", "then", "else", "when", "of", "to", "in", "on",
    "for", "with", "is", "are", "was", "were", "be", "been", "this", "that", "these", "those",
    "it", "its", "as", "at", "by", "from", "into", "do", "does", "did", "which", "what", "how",
    "should",
];

/// `true` when `token` is in [`STOP_WORDS`].
#[must_use]
pub fn is_stop_word(token: &str) -> bool {
    STOP_WORDS.contains(&token)
}

/// `true` when `token` is a negation word.
#[must_use]
pub fn is_negator(token: &str) -> bool {
    NEGATORS.contains(&token)
}

/// Lowercases, splits on non-alphanumeric characters, and drops stop words,
/// negation-neutral empties, and over-long tokens.
///
/// Unicode-aware: `is_alphanumeric` keeps non-ASCII letters so a
/// description in another script still tokenizes instead of collapsing to
/// nothing.
#[must_use]
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    for raw in text.split(|c: char| !c.is_alphanumeric()) {
        let token = raw.to_lowercase();
        if token.is_empty() || token.len() > MAX_TOKEN_LEN || is_stop_word(&token) {
            continue;
        }
        tokens.push(token);
    }
    tokens
}

/// Counts negation tokens in `text` and returns the polarity they imply:
/// `1.0` for an even count (including zero), `-1.0` for an odd count.
///
/// This is a deliberately crude heuristic used only by the boolean lexical
/// hypothesis; it is documented as such and is not a language model.
#[must_use]
pub fn negation_polarity(text: &str) -> f64 {
    let negators = tokenize(text).iter().filter(|token| is_negator(token)).count();
    if negators % 2 == 0 { 1.0 } else { -1.0 }
}

/// A small BM25 index over in-memory documents.
///
/// Documents are supplied once and referenced by position afterwards; the
/// caller owns the meaning of each position (typically "candidate `i`").
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Bm25Index {
    documents: Vec<Vec<String>>,
    lengths: Vec<f64>,
    average_length: f64,
    document_frequency: std::collections::BTreeMap<String, usize>,
}

impl Bm25Index {
    /// Builds an index over the supplied documents, in the order given.
    #[allow(clippy::cast_precision_loss)]
    pub fn new<I, D>(documents: I) -> Self
    where
        I: IntoIterator<Item = D>,
        D: AsRef<str>,
    {
        let documents: Vec<Vec<String>> =
            documents.into_iter().map(|document| tokenize(document.as_ref())).collect();
        let mut document_frequency: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for document in &documents {
            for term in document.iter().collect::<std::collections::BTreeSet<_>>() {
                *document_frequency.entry(term.clone()).or_insert(0) += 1;
            }
        }
        let lengths: Vec<f64> = documents.iter().map(|document| document.len() as f64).collect();
        let total: f64 = lengths.iter().sum();
        let average_length =
            if documents.is_empty() { 0.0 } else { total / documents.len() as f64 };
        Self { documents, lengths, average_length, document_frequency }
    }

    /// Number of indexed documents.
    #[must_use]
    pub fn len(&self) -> usize {
        self.documents.len()
    }

    /// `true` when nothing is indexed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }

    /// Number of documents containing `term`.
    #[must_use]
    pub fn document_frequency(&self, term: &str) -> usize {
        self.document_frequency.get(term).copied().unwrap_or(0)
    }

    /// BM25 score of `query` against document `index`.
    ///
    /// Out-of-range indices score `0.0` rather than panicking: a scoring
    /// gap is never worth aborting a decision over.
    #[must_use]
    pub fn score(&self, query: &str, index: usize) -> f64 {
        let Some(document) = self.documents.get(index) else { return 0.0 };
        let Some(length) = self.lengths.get(index).copied() else { return 0.0 };
        let mut total = 0.0;
        for (term, count) in query_terms(query) {
            let frequency = document.iter().filter(|candidate| **candidate == term).count();
            if frequency == 0 {
                continue;
            }
            // Term frequencies are small; the cast cannot lose precision.
            #[allow(clippy::cast_precision_loss)]
            let count = count as f64;
            total += self.term_weight(&term) * self.saturated_frequency(frequency, length) * count;
        }
        total
    }

    /// BM25 scores of `query` against every document, in document order.
    #[must_use]
    pub fn score_all(&self, query: &str) -> Vec<f64> {
        (0..self.documents.len()).map(|index| self.score(query, index)).collect()
    }

    /// Inverse document frequency: `ln(1 + (N - df + 0.5) / (df + 0.5))`.
    ///
    /// The `+ 1` inside the logarithm keeps every weight non-negative, so
    /// a term that appears in every document contributes ~0 instead of a
    /// negative score.
    #[allow(clippy::cast_precision_loss)]
    fn term_weight(&self, term: &str) -> f64 {
        let total = self.documents.len() as f64;
        let seen = self.document_frequency(term) as f64;
        (1.0 + (total - seen + 0.5) / (seen + 0.5)).ln()
    }

    /// Frequency with BM25 saturation and length normalization applied:
    /// `tf * (k1 + 1) / (tf + k1 * (1 - b + b * |d| / avgdl))`.
    #[allow(clippy::cast_precision_loss)]
    fn saturated_frequency(&self, frequency: usize, length: f64) -> f64 {
        let tf = frequency as f64;
        if self.average_length <= 0.0 {
            return tf * (K1 + 1.0) / (tf + K1);
        }
        let normalization = K1 * (1.0 - B + B * length / self.average_length);
        tf * (K1 + 1.0) / (tf + normalization)
    }
}

/// Term frequencies of a query, in sorted term order.
///
/// Sorted iteration is what makes score summation reproducible: float
/// addition is not associative, so an order-dependent fold would make
/// scores (and therefore cacheable decisions) platform-dependent.
fn query_terms(query: &str) -> std::collections::BTreeMap<String, usize> {
    let mut counts = std::collections::BTreeMap::new();
    for token in tokenize(query) {
        *counts.entry(token).or_insert(0) += 1;
    }
    counts
}

/// Numerically stable softmax over `scores`, preserving input order.
///
/// Uniform when every score is equal, non-finite, or the list is empty:
/// lexical scoring degrades to "no opinion" instead of inventing a
/// preference.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn softmax(scores: &[f64]) -> Vec<f64> {
    if scores.is_empty() {
        return Vec::new();
    }
    let count = scores.len();
    let uniform = vec![1.0 / count as f64; count];
    let max = scores.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !max.is_finite() {
        return uniform;
    }
    let exponentials: Vec<f64> = scores.iter().map(|score| (score - max).exp()).collect();
    let sum: f64 = exponentials.iter().sum();
    if !sum.is_finite() || sum <= 0.0 {
        return uniform;
    }
    exponentials.iter().map(|value| value / sum).collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp)]

    use super::*;

    #[test]
    fn tokenizes_lowercases_and_splits_on_boundaries() {
        assert_eq!(tokenize("Refactor the Rust parser"), vec!["refactor", "rust", "parser"]);
        assert_eq!(
            tokenize("local-qwen: general coding"),
            vec!["local", "qwen", "general", "coding"]
        );
        assert_eq!(tokenize("  "), Vec::<String>::new());
        assert_eq!(tokenize("naïve café"), vec!["naïve", "café"]);
    }

    #[test]
    fn drops_stop_words_and_overlong_tokens() {
        assert_eq!(tokenize("the and of"), Vec::<String>::new());
        let overlong = "x".repeat(MAX_TOKEN_LEN + 1);
        assert!(tokenize(&overlong).is_empty());
        assert_eq!(tokenize("not").len(), 1, "negators must survive tokenization");
    }

    #[test]
    fn stop_word_and_negator_lists_are_consistent() {
        for negator in NEGATORS {
            assert!(!is_stop_word(negator), "{negator} must not be a stop word");
            assert!(is_negator(negator));
        }
        assert!(!is_negator("parser"));
    }

    #[test]
    fn negation_polarity_flips_on_odd_counts() {
        assert_eq!(negation_polarity("refactor the parser"), 1.0);
        assert_eq!(negation_polarity("do not deploy"), -1.0);
        assert_eq!(negation_polarity("never not once"), 1.0);
    }

    #[test]
    fn bm25_ranks_relevant_document_first() {
        let index = Bm25Index::new([
            "general coding and reasoning across languages",
            "long context research summarization",
            "vision and image understanding",
        ]);
        let scores = index.score_all("coding reasoning");
        assert!(scores[0] > scores[1]);
        assert!(scores[0] > scores[2]);
        assert!(scores[0] > 0.0);
    }

    #[test]
    fn bm25_scores_zero_when_no_term_overlaps() {
        let index = Bm25Index::new(["general coding", "vision understanding"]);
        assert!(index.score_all("quaternion orbital mechanics").iter().all(|score| *score == 0.0));
    }

    #[test]
    fn bm25_is_deterministic_across_repeated_calls() {
        let index = Bm25Index::new(["alpha beta gamma", "beta delta epsilon"]);
        let first = index.score_all("beta alpha");
        let second = index.score_all("beta alpha");
        assert_eq!(first, second);
    }

    #[test]
    fn bm25_handles_empty_and_out_of_range_documents() {
        let empty = Bm25Index::new(Vec::<&str>::new());
        assert!(empty.is_empty());
        assert_eq!(empty.score_all("anything"), Vec::<f64>::new());

        let index = Bm25Index::new(["one term"]);
        assert_eq!(index.score("one", 7), 0.0);
        assert_eq!(index.len(), 1);
        assert_eq!(index.document_frequency("one"), 1);
        assert_eq!(index.document_frequency("absent"), 0);
    }

    #[test]
    fn short_documents_outscore_long_documents_for_equal_hits() {
        let index = Bm25Index::new([
            "coding",
            "coding and a very long unrelated description of many things",
        ]);
        let scores = index.score_all("coding");
        assert!(scores[0] > scores[1], "length normalization must favour the short document");
    }

    #[test]
    fn softmax_is_stable_and_normalized() {
        let scores = softmax(&[1.0, 2.0, 3.0]);
        let sum: f64 = scores.iter().sum();
        assert!((sum - 1.0).abs() < 1e-12);
        assert!(scores[2] > scores[0]);
        // Large magnitudes must not overflow into NaN.
        let shifted = softmax(&[1000.0, 1001.0]);
        assert!((shifted[0] + shifted[1] - 1.0).abs() < 1e-12);
    }

    #[test]
    fn softmax_degrades_to_uniform_on_ties_and_garbage() {
        let uniform = softmax(&[0.0, 0.0, 0.0]);
        assert!(uniform.iter().all(|value| (value - 1.0 / 3.0).abs() < 1e-12));
        // A NaN loses to any real value in `f64::max`, so this tie still
        // resolves numerically rather than degrading.
        let garbage = softmax(&[f64::NAN, 1.0]);
        assert!(garbage.iter().all(|value| (value - 0.5).abs() < 1e-12));
        assert!(softmax(&[]).is_empty());
    }

    #[test]
    fn softmax_degrades_to_uniform_when_no_value_is_orderable() {
        // An infinite score has no finite reference point to subtract, and a
        // vector of NaNs folds to negative infinity: either way the only
        // defensible answer is the uniform one.
        let infinite = softmax(&[f64::INFINITY, 1.0]);
        assert!(infinite.iter().all(|value| (value - 0.5).abs() < 1e-12));
        let negative_infinite = softmax(&[f64::NEG_INFINITY, f64::NEG_INFINITY]);
        assert!(negative_infinite.iter().all(|value| (value - 0.5).abs() < 1e-12));
        let all_nan = softmax(&[f64::NAN, f64::NAN, f64::NAN]);
        assert!(all_nan.iter().all(|value| (value - 1.0 / 3.0).abs() < 1e-12));
    }

    #[test]
    fn index_equality_follows_contents() {
        assert_eq!(Bm25Index::new(["a b"]), Bm25Index::new(["a b"]));
        assert_ne!(Bm25Index::new(["a b"]), Bm25Index::new(["a c"]));
    }
}
