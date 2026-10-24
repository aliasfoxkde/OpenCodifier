# Semantic Reranker — Design (cross-encoder pass)

**Status:** Design only. Not implemented. (2026-10-24, offline campaign
Track D — `docs/planning/offline-campaign-2026-10-23.md`.)

This is SPEC_COVERAGE §71's remaining reranker item. §26 already ships
reranking (`LexicalReranker`, `EmbeddingReranker`) — see
`docs/DECISIONS.md` D21 for the permute-never-remove contract. Those two
are *independent scorers*: BM25 scores the query against each document
term-by-term, a bi-encoder embeds each side separately and takes cosine.
Neither reads (query, document) **jointly**. That is the cross-encoder's
one advantage — and the entire design below is about buying it without
breaking the zero-ML default, the rung seam, or byte-identity.

## Problem

For some queries the ordering that decides the outcome depends on
understanding the two strings *together* — "HTTP 503 error" versus
"serverless latency": a bi-encoder sees two vectors and hopes their
cosine means what we need; a cross-encoder sees the pair and scores the
relevance directly. Ordering errors at the rerank stage are invisible to
everything after it: the decision only ever sees the top of the list.

## What already exists (the seams this rides)

| seam | state | what the design adds |
|---|---|---|
| `Reranker` trait (`opencodifier-engine/src/rerank.rs`) | `rerank(query, candidates) -> Vec<ScoredCandidate>`, `id()` for trace, scores are the reranker's own scale | one new implementor |
| node knob (`graph.rs` `reranker:`) | `"lexical"` \| `"embedding"`, unknown refused | a third value: `"model"` |
| rung URL (`opencodifier serve --llama <URL>`) | decision rung (`/v1/decision`) | a sibling flag for the rerank endpoint |
| D41 fork llama-server | **already exposes `/rerank`, `/reranking`, `/v1/rerank`** (Jina/TEI shapes, `relevance_score` field) | nothing — no fork change |
| local artifacts | `gte-modernbert-base-q8_0.gguf` + ONNX fp16/fp32/int8/q4 ladder, `deberta-v3-base-zeroshot-v2.0`, `nli-deberta-v3-small`, `modernbert-large-instruct` | the evaluation set |

## Design

### Topology: a second server process, not a second rung

llama-server serves one model in one mode. The decision rung is a
generative model (Qwen3.5-2B family); `/v1/rerank` refuses to boot
without `--reranking` and a RANK-pooling (classification-head) model.
So the reranker cannot share the rung's process:

```
opencodifier serve
  --llama   http://127.0.0.1:8080   # decision rung (generative)
  --rerank  http://127.0.0.1:8081   # cross-encoder (--reranking boot)
```

`--rerank` follows `--llama`'s posture exactly: loopback-by-default, the
URL is the operator's to point at, and — like a graph naming
`"embedding"` with no backend — **startup is refused when a graph names
`reranker: "model"` and no `--rerank` URL is configured**. Never a
silent fallback to cosine or lexical: an ordering that silently changes
signal mid-graph is worse than a refused boot (same rule as the
`EmbeddingReranker` missing-backend posture).

### The implementor

```rust
pub struct HttpReranker { url: Url, model_id: String }
// id() -> "cross-encoder@<model_id>"   (surfaces in traces, D21)
```

Request (Jina shape, one round-trip for the whole list):

```json
{ "query": "<question text>",
  "documents": ["<candidate id>: <description>", ...],
  "top_n": <len> }
```

Response maps `index → candidates[i]`, carrying `relevance_score`
(`[0, 1]`, the backend's own scale — never mixed with BM25/cosine, per
the trait's contract). The result is a permutation with scores: **the
node reorders, never removes** (§26/D21 unchanged).

### Wire compatibility

- Graphs naming `"lexical"` / `"embedding"`: response bytes
  **identical** — the new knob value, flag, and backend are unreachable
  from them.
- `"model"` is additive: graph validation gains one accepted string;
  unknown strings are still refused.
- The native/OpenAI/Anthropic projections are untouched — rerank
  ordering rides the trace, not the response body (D21 as-is).

### Budget and gating

- **Bounded K.** The cross-encoder runs only after `narrow` has cut the
  list (metadata → BM25, the existing 1 ms p99/256-candidate D9 stages),
  and only over the top `rerank_top_k` candidates (ladder/profile knob;
  start at 16, measure before moving). One `/v1/rerank` call scores all
  K documents in one server round-trip.
- **Own budget row.** BM25/embedding budgets are unaffected; the rerank
  stage's latency is *backend-declared* like D9 row 4, and lands as a
  new D9 row when implemented — not promised by this doc.
- **Engagement knob.** A profile can gate the stage on ordering
  uncertainty (e.g. only when the BM25 top-2 margin is thin). Knob, not
  heuristic baked into the engine.

### Trace disclosure

`rerank` trace entries gain `reranker: "model"`, `model_id`,
`server_ms`, and the K scores alongside the permutation actually
applied — the disclosure depth D21 already establishes, one field
deeper.

### Confidence stays where it is

Cross-encoder scores are **not** calibrated probabilities and never
become `confidence`, never gate `accept`/`verify`/`abstain`, never
enter calibration artifacts. Rerank permutes; the ladder decides. This
is the same firewall as §26 ("rerankers never remove") and the D42
routing rule ("cannot borrow confidence from another signal").

### Determinism

Scoring is a deterministic forward pass, but the response bytes of a
`"model"` graph carry the model's cross-kernel float behaviour, exactly
like every model-rung decision today: determinism is claimed per
identity/replicate (the board's determinism columns), not globally.
Graphs that don't name `"model"` keep full byte-identity.

## Evaluation plan (offline-capable)

1. **Rerank shuffle rate** on `suite_holdout_rel200`: does
   cross-encoder ordering move the winner vs lexical/cosine? (Count
   answer-changed items — if ~0, the pass buys nothing on this suite
   and the honest verdict is "needs agentic corpora".)
2. **End-to-end arm**: engine + `reranker: "model"` vs engine
   (`embedding`) on the same suite — accuracy delta, latency delta,
   recorded as a board row with the backend identified.
3. **Probe first** (deferred while #188 owns the CPU): boot the fork
   binary with `--reranking -m gte-modernbert-base-q8_0.gguf` and post
   one `/v1/rerank` — verifies the local GGUF carries a RANK-pooling
   head. If GGUF conversion drops the head, the ONNX ladder
   (`gte-modernbert-onnx-int8`) becomes the reference backend via a
   different transport, and this doc's transport section revises.

## Open questions

- **Model of record**: `gte-modernbert-base-q8_0.gguf` (CPU-friendly,
  same-server path) vs the ONNX deberta ladder (known-good on this host
  from the NLI board rows). Probe decides; the trait hides the choice.
- **`rerank_top_k` default**: 16 is a starting guess; fit it like D9
  fits budgets (criterion over the suite), don't hard-code folklore.
- **Margin-as-verify**: cross-encoder top-2 margin as a *verify*
  trigger is tempting (it measures ordering uncertainty directly).
  Deferred deliberately — it is a confidence signal from a non-calibrated
  model, i.e. exactly the borrowing §26/D42 forbid. Revisit only with a
  measured calibration story.
- **Cost vs the bi-encoder**: when does the delta justify a second
  process on a small host? The shuffle-rate experiment answers it with
  numbers.

## Rejected alternatives

- **Make the cross-encoder the default reranker**: breaks the zero-ML
  default — the base binary must decide with no model files anywhere
  (PLANNING Rule 1's sequencing, engine identity).
- **Run the reranker inside the decision rung process**: one
  llama-server, one model, one mode; sharing would put the rung's
  availability on the reranker's boot path.
- **Let rerank scores nudge confidence**: scores are uncalibrated
  logits-by-another-name; the ladder owns confidence (§26, D42).
- **`EmbeddingReranker` replacement**: the bi-encoder stays — it needs
  no second process and wins where independent scoring is enough
  (HYBRID_RETRIEVAL's stage-3). Cross-encoder is the precision stage
  after hybrid's recall stage, not its successor.
