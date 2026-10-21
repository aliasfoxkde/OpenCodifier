# Hybrid Retrieval — Design

**Status:** Design only. Not implemented.

## Problem

OpenCodifier currently has two retrieval modes:
1. **Lexical (BM25)**: good for exact token matches, no embedding backend needed
2. **Embedding (vector similarity)**: good for semantic similarity, requires `EmbeddingBackend`

Neither alone handles all cases. A query like "serverless function latency" is answered
better by semantic similarity; "HTTP 503 error" is answered better by BM25. Hybrid retrieval
combines both signals.

## Design

### Score combination

For each candidate, compute:
```
bm25_score: f64       // existing BM25 score
embed_score: f64       // cosine similarity from EmbeddingBackend

hybrid_score = alpha * bm25_score + (1 - alpha) * embed_score
```

`alpha` is a configuration parameter (0.0 = pure embedding, 1.0 = pure lexical).

### Candidate narrowing pipeline

The existing `narrow` node already has two sequential stages (metadata filter → BM25).
The hybrid mode adds a third stage:

```
narrow:
  stage 1: metadata filter (rule-based, fast)
  stage 2: BM25 (lexical, fast)
  stage 3: [optional] embedding rerank (semantic, slower)
```

Stage 3 runs only when:
- An `EmbeddingBackend` is configured
- The query is not purely lexical (detected by a heuristic: does the query contain
  rare tokens that BM25 handles well? Or does it contain general terms where semantics dominate?)
- `narrow_budget > N` (a minimum number of candidates to re-rank)

### Alpha configuration

`alpha` is a ladder knob: different ladder profiles set different alphas.

| Query type | alpha | Rationale |
|---|---|---|
| Technical jargon, error codes | 0.7 | BM25 handles rare tokens well |
| Natural language descriptions | 0.3 | Embeddings capture intent |
| Mixed | 0.5 | Equal weight |

Automatic alpha: compute BM25 idf for query tokens. If average idf is high (rare terms),
bias toward BM25 (high alpha). If average idf is low (common terms), bias toward embeddings
(low alpha).

### RRF combination (alternative)

Instead of linear combination, use Reciprocal Rank Fusion:
```
RRF_score(c) = sum_i 1 / (k + rank_i(c))
```
Where `rank_i` is the candidate's rank under BM25 (i=1) and embedding (i=2),
and k is a smoothing constant (default k=60).

RRF is parameter-free and handles the "which ranker is better" problem automatically.
This is the simpler approach — implement RRF first, linear combination as a future option.

### Trace disclosure

The execution trace already discloses narrowing. Hybrid narrowing adds:
- `narrow_hybrid: true`
- `narrow_rrf: true` (if using RRF)
- `alpha: 0.3` (if using linear combination)
- Per-candidate BM25 rank, embed rank, and hybrid rank (top N only, not all candidates)

### Interaction with existing narrowing

The existing `narrow` node already has budget-driven stop: if the top-K candidates
from BM25 already meet the budget, embedding re-ranking is skipped entirely.
Hybrid retrieval adds embedding scores to the mix but preserves the same stop behavior.

### Configuration

```rust
pub struct HybridConfig {
    /// RRF smoothing constant. k=60 is the standard (TREC default).
    /// Set alpha=None to use RRF, or provide an explicit alpha for linear.
    pub rrf_k: Option<f64>,
    /// Linear combination weight for BM25 (0.0–1.0). Ignored if rrf_k is set.
    pub alpha: Option<f64>,
    /// Minimum candidates to send to embedding re-ranker.
    pub embed_rerank_min: usize,
}
```

`None` disables the hybrid stage (reverts to existing behavior).

## Open questions

- **When does embedding re-ranking fire?** Currently: always when an embedding backend exists
  and budget allows. The automatic alpha approach above could also gate it.
- **Embedding rerank vs. embedding narrow:** the existing `EmbeddingReranker` re-ranks
  already-fetched candidates. A "hybrid narrow" would apply embedding scores *before*
  BM25 budget cutoff. This is a different seam.
- **Training alpha:** should alpha be fitted against accuracy, not hand-tuned?
  If so, alpha becomes a calibration artifact.

## Relationship to D9 BM25 bench

BM25 narrowing is already criterion-benched at 256 candidates with a 1 ms p99 budget.
Hybrid retrieval adds embedding scoring on top. The embedding score computation itself
is backend-declared (D9 row 4), so the budget is "backend-declared." The hybrid
combination is O(n) and fast.
