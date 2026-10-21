# Semantic Cache — Design

**Status:** Design only. Not implemented.

## Problem

The current decision cache is **exact-match**: a cached decision is returned only
when the request's canonical serialization hashes to the same key. This is correct
but misses the obvious optimization: semantically similar states that produce the
same decision should share a cache entry.

## The Core Tension

A semantic cache must answer: *when are two states "the same decision" for the
same question?*

Two states that differ only in surface form ("cloud-native" vs "kubernetes-based")
should hit the same cache if the decision is identical. But two states that differ
in a decision-relevant way must not share a cache entry.

## Design

### Layered cache: exact → semantic

```
Request arrives
  → exact cache key (SHA-256 of canonical IR) → hit: return cached response
  → miss → compute decision
    → semantic cache key (state embedding + question) → hit: return cached response (with a trace flag)
    → miss → escalate to model
```

The semantic cache is a second-tier lookup, not a replacement for exact cache.
Exact cache preserves byte-for-byte determinism; semantic cache is an optimization.

### Semantic key

The semantic cache key is:
- The state text embedded by the existing `EmbeddingBackend`
- The question text embedded similarly
- The candidate set (hash of candidate IDs)

Two requests with cosine similarity > `semantic_threshold` on both embeddings
share a cache entry.

### Cache entry

```rust
struct SemanticCacheEntry {
    response: Arc<DecisionResponse>,
    embedded_at: Instant,
    hits: usize,
    // Which exact cache key this was derived from
    source_key: CacheKey,
}
```

### Semantic cache policy

Configured via `DecisionPolicy`:
- `semantic_cache_threshold: Option<f64>` (None = disabled)
- `semantic_cache_max_age: Duration` (default: 1 hour)
- `semantic_cache_max_entries: usize` (LRU eviction)

### Hit behavior

When semantic cache hits, the response is returned with a `semantic_cache_hit: true`
trace fact. The client can distinguish:
- `cache_hit: true, semantic_cache_hit: false` — exact match (fastest)
- `cache_hit: true, semantic_cache_hit: true` — semantic match (trust but verify)
- `cache_hit: false` — fresh computation

### Constraints

1. **Cache key version**: if the embedding model changes, all semantic cache entries
   are invalidated. The `EngineIdentity.embedding_version` is already in the exact
   cache key; semantic cache adds `embedding_model_id` to its key.

2. **Calibration**: if different surfaces need different calibrations (per §26), the
   semantic cache must not return a response calibrated for surface A on surface B.
   The calibration artifact version must be part of the semantic key.

3. **Never serve a stale calibration**: semantic cache hits must re-apply the
   current calibration before returning. The cached response stores raw logits;
   calibration is applied at serve time.

4. **Trace disclosure**: semantic cache hits are trace-disclosed (the executor already
   traces cache hits). This is the only honest way to let callers know.

## Open questions

- **What embedding model?** The existing `EmbeddingBackend` serves semantic nodes.
  Reusing it for the semantic cache means the cache inherits the embedding model's
  biases. A dedicated sentence-transformer might be more appropriate.
- **Threshold calibration**: `semantic_threshold` is currently hand-tuned. Should it
  be measured against decision accuracy? If two states are cosine-similar but
  produce different decisions, the threshold is wrong.
- **Memory cost**: semantic cache entries hold full responses plus embeddings. With
  256-byte embeddings and 10k entries, that's ~2.6 MB. LRU eviction at 10k entries
  is fine.

## Interaction with exact cache

The semantic cache does NOT write to the exact cache — the exact cache is written
by the primary decision path. Semantic cache hits return cached responses without
writing to the exact cache, to avoid poisoning exact cache keys with semantically
derived entries.

## Rejected alternatives

- **Replace exact cache with semantic**: breaks determinism guarantees and the
  `CacheKeyBuilder` contract
- **Store raw logits in semantic cache**: calibration is per-surface; serving a
  surface-A calibration on surface B is wrong. Store responses, recompute calibration.
