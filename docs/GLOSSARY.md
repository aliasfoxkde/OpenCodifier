# Glossary

Terms of the OpenCodifier decision runtime, defined against the code that
implements them. Each entry cites its module (paths under `crates/`) and, where
one exists, the binding decision record in [`docs/DECISIONS.md`](DECISIONS.md).

Governance of terms: if a concept is **planned but not built**, it is not
defined here as current — it lives in [`docs/PLANNING.md`](PLANNING.md) (the
founding spec, §§1–80) or [`docs/PLAN.md`](PLAN.md) (live phases) in future
tense. This glossary describes only what ships. Intent documents are allowed
to aspire; everything else must match the code.

Sections: [Core concepts](#core-concepts) · [Decision IR](#decision-ir) ·
[Engine](#engine) · [Ladder & confidence](#ladder--confidence) ·
[Caching](#caching) · [Model & runtime](#model--runtime) ·
[Wire & adapters](#wire--adapters) · [Interfaces](#interfaces) ·
[Evaluation & release](#evaluation--release) · [References](#references)

---

## Core concepts

**Decision runtime** — the product category: a system that turns unstructured
state into small, machine-actionable *typed decisions* (choice / boolean /
score) with calibrated confidence. Not a chatbot, not an LLM wrapper, not a
decision-tree classifier. (`docs/PLANNING.md` §1; `docs/ARCHITECTURE.md`)

**Deterministic-first** — the execution principle: always spend the cheapest
reliable mechanism first — exact rule → cached decision → metadata filter →
lexical match → embedding similarity → small classifier → candidate-conditioned
decision model → verifier → external model. A more expensive layer never runs
when a cheaper one can decide with sufficient confidence. (`docs/PLANNING.md`
§2, §73)

**Abstention** — declining to answer because confidence is insufficient.
A *successful outcome*, never an error: the confidence report routes to an
explicit `Abstain`/`Escalate` outcome rather than failing. Confidence report
outcome routing: `crates/opencodifier-core/src/confidence.rs`
(`ConfidenceReport::outcome_for`).

**Selective prediction** — operating mode where a system answers only a
sub-set of inputs (the ones it is confident about) and reports *coverage* —
the fraction answered. Shipped end-to-end in v0.6.1; thresholds come from
fitted ladder profiles, never hardcoded constants (`ladder_fit.rs`; D25).

**Candidate-conditioned** — the shape of every decision: candidates are part
of the question (a runtime-defined list), and the model scores *those*
candidates rather than generating text. Free-form generation fields are
refused (`unsupported_generation_field`), never approximated.

**Trace** — the deterministic execution record of one decision (which rungs
fired, what the rules proved, where confidence came from). Explainability
means the trace, never chain-of-thought. Schema: `crates/opencodifier-core/src/trace.rs`;
traces carry a `trace_version` so consumers can pin behavior.

## Decision IR

The canonical internal representation. External formats (OpenAI, Anthropic,
Jev) are adapters that normalize *into* it — never the reverse
(`docs/ARCHITECTURE.md`; D11 keeps heavyweight deps out of this crate).

**`State`** — the input context of a decision: `text` plus a sorted map of
typed facts. `crates/opencodifier-core/src/state.rs:126`.

**`FactValue`** — the closed set of typed fact values (text, numbers, lists,
tags). Facts are tagged enums, not free strings.
`crates/opencodifier-core/src/state.rs:18`.

**`Candidate`** — one selectable option in a choice question: an id, a label,
and metadata used by deterministic narrowing.
`crates/opencodifier-core/src/question.rs:15`.

**`DecisionQuestion`** — the enum of the three supported kinds:
`Choice` (`ChoiceQuestion`, question.rs:65), `Boolean` (`BooleanQuestion`,
question.rs:130, carries a probability), `Score` (`ScoreQuestion`,
question.rs:217, ordered `ScoreLevel`s — the full distribution plus expected
value is retained). question.rs:309.

**`DecisionRequest`** — the validating envelope: state + question + limits +
policy/graph selectors. Its constructor enforces the size gates.
`crates/opencodifier-core/src/request.rs`.

**`Limits`** — the per-request ceiling set (input bytes, candidate count,
image bytes/count) checked at the IR boundary and re-checked by the native
adapter. `crates/opencodifier-core/src/policy.rs`.

**`CoreError` / `ir.*` codes** — the IR's stable error codes (e.g.
`ir.unsupported_media_type`). Codes are append-only: existing codes never
change meaning. `crates/opencodifier-core/src/error.rs`.

**Raw mirrors** — the serde-facing unvalidated shapes (e.g. `RawFactValue`)
that `TryFrom` into typed IR values, so wire decoding errors get precise
`ir.*` codes instead of generic serde errors. state.rs:21–59 idiom.

## Engine

`crates/opencodifier-engine` — the deterministic core. No async runtime, no
ML dependency in its default build (D5, D11).

**Graph / DAG** — a declarative, serializable decision pipeline. Graphs have
no embedded scripting language (§73); equivalence-preserving graph
optimization is bounded by D22 (`optimize.rs`).

**Rule engine** — exact deterministic decisions from conditions over facts
(`Condition::FactContains`, `ListContains`, …). `rules.rs`. A rule match is a
proof: confidence 1.0, no model involved.

**Relational solver** — exact proofs over extracted facts (relational /
contrastive reasoning), the default zero-ML stack. `RelationalSolver`
(`relational.rs:44`) wraps an inner classifier: it answers when it can prove,
otherwise delegates — the composition idiom for capability-narrowing
wrappers.

**BM25 / candidate narrowing** — the hand-written Okapi BM25 lexical scorer
used to narrow the candidate set before anything expensive runs; never
silently discards candidates on weak evidence. `narrowing.rs`; the scorer
itself is `lexical.rs` ("a *lexical* layer, not a semantic model" — its own
doc comment).

**Reranker seam** — a second opinion on *ordering*: returns surviving
candidates re-scored, never removes them, never decides. `Reranker` trait
(`rerank.rs:42`; `LexicalReranker`, `EmbeddingReranker`). D21.

**Focused extraction** — token-budgeted narrowing of the state to the parts a
question needs (`focus.rs`: `FocusPolicy`, `focus()`); a weak focused view
escalates to the full state before any model call (D18).

**Reverse escalation** — the executor's ordered fallback walk: when the
primary mechanism declines, the escalation ladder is consulted at the
threshold node (executor.rs:275); the walk terminates when a rung's outcome
accepts or every rung abstains (D27).

**`Classifier` trait** — the contract every decision mechanism implements:
`decide(&State, &DecisionQuestion) -> Result<Distribution, EngineError>` plus
`model_id()`. `classifier.rs:38`. Sync by design (D5); async belongs to the
interface crates.

**`EngineHandle`** — the public engine assembly: `new(config)` for the
deterministic stack, `lexical(config)` for the default zero-ML engine,
`with_rungs(config, classifier, verifier, ladder, fallbacks)` to attach the
escalation ladder. `handle.rs:48/132/105`.

**Ladder profile** — a data document of fitted per-rung gates and thresholds,
loaded through the `--ladder` seam (D25). Fitted offline from labeled
evidence by `ladder_fit.rs`; the empty profile is byte-identical in behavior
to no profile.

**Clock/Deadline seams** — the trait seams (`Clock`, `Deadline`) that keep
time out of the deterministic core and make sync timeouts testable
(`clock.rs`; the wasm platform seam, D23).

## Ladder & confidence

**Rung** — one escalation tier: `{ classifier, calibration, policy }`
(`ladder.rs:68`). Rungs fire only when the previous gate did not accept —
never two classifiers on the happy path (D27).

**Verifier** — a classifier consulted only when confidence is marginal:
agree → accept, disagree → abstain/escalate. Verification is
confidence-gated (§73), never run on every request.

**Confidence gate** — the threshold node where a calibrated distribution is
accepted, verified, or escalated. Raw softmax probability is *never* exposed
as confidence (§73).

**Calibration** — the mandatory mapping from model scores to truthful
probabilities. Contract: `Calibration` trait (`calibration.rs:56`,
`calibrate`/`calibrate_probability`/`version()`).

**Temperature scaling** — the shipped calibration family
(`TemperatureCalibration`, fitted temperature per question kind; artifacts
load through `from_artifact` with an ECE-non-worsening refusal gate and an
AURC floor of 200 items, calibration.rs:288–304). Per-rung artifacts attach
at the rung slot (`Rung::with_calibration`) because rung confidences are not
commensurable across rungs.

**ECE** — expected calibration error: mean |claimed probability − observed
frequency| over bins. The gate metric for accepting a calibration artifact
(fit offline in `benchmarks/decision-model/runner/fit_calibration.py`; the
runtime only *loads* artifacts, it never fits).

**AURC** — area under the risk–coverage curve: selective-prediction quality
across all operating points. Lower is better; the load-time floor requires
≥200 scored items.

**Coverage** — the fraction of requests answered rather than abstained.
Selective prediction trades coverage for accuracy; both numbers are always
reported together.

**OOD (out-of-distribution)** — a multi-dimensional confidence input: how far
the input sits from the data the rung was fitted on. Confidence is
deliberately multi-dimensional — top probability, margin, entropy, OOD,
verifier agreement — never a single number (§73).

## Caching

**Exact-decision cache** — caches *decisions* (not embeddings, not generations)
keyed by the full decision-relevant identity. `cache.rs`.

**`CacheKeyBuilder`** — the single normative cache-key construction
(`cache.rs:168`, `build` at :224): SHA-256 over the canonical request plus
`EngineIdentity` — graph version, model id, calibration version, engine
semver, embedding model. Every cache key includes model/calibration/policy/
graph versions so any artifact update invalidates stale decisions (§73).

**Engine identity decoration** — ladder engines append
`|rungs-v1@<model>@<calver>` per rung to the model id (engine.rs:362–375), so
changing a rung or its calibration re-keys automatically.

## Model & runtime

**`opencodifier-runtime`** — the ML seam crate: backend traits, ONNX behind
the `onnx` feature, tensors. No model code runs without an explicit feature
gate (D11); the base binary is useful with zero ML (§73).

**`InferenceBackend`** — the sync (D5) trait for logit-producing models:
`model_id()` + `infer(&BTreeMap<String, DenseTensor>)`. `backend.rs:31`.
**`EmbeddingBackend`** — the text-embedding trait: `dims()` +
`embed(&[&str])`. backend.rs:54. Both are object-safe (`dyn`-compatible).

**`DenseTensor`** — the f32 tensor exchange type (cap: 2²⁴ elements).
`tensor.rs`.

**ort / ONNX** — the ONNX Runtime binding (`ort` crate) behind the `onnx`
feature; `runtime-onnx-dynamic` loads ORT at runtime via `ORT_DYLIB_PATH`
(the only verifiable packaging on some hosts, D34). No Python at runtime,
ever (§73).

**Candidate-conditioned decision model** — the served model contract: logits
come from a backend, *all decision math (softmax, margins, expected value)
happens in Rust f64* (D7). `opencodifier-model/src/decision.rs`.

**Kai / Julia / NLI rungs** — the shipped serving contracts (prompt-shaped
decision model, julia-style, natural-language-inference), each a feature-free
contract module in `opencodifier-model` (`kai.rs`, `julia.rs`, `nli.rs`) with
frozen parity fixtures, plus feature-gated ONNX transports in
`opencodifier-runtime` (`kai.rs`, `julia.rs`, `nli.rs`). Prompt-shaped model
= `Classifier`, not `InferenceBackend` (D26).

**Model manifest** — the committed SHA-256 artifact manifest
(`ModelManifest::verify_artifact`, `opencodifier-model/src/manifest.rs:82`);
weights themselves are gitignored (D14). Models are never downloaded by
build or runtime — explicit operator opt-in only (release policy).

**Parity fixture** — a frozen, `include_str!`-ed reference record (encoded
inputs → reference outputs) that pins a contract to byte-level behavior
before any corpus-scale run. Fixture-first, corpus-second is the model-import
discipline; the 5-ULP rule (D2) is the f32 tolerance for numeric parity.

**Feature gate** — a Cargo feature confining one heavyweight dependency to
one crate. The matrix is recorded in D11; cross-crate activation exists only
where a row documents it (e.g. `nli`, `vision` in `opencodifier-model`).

## Wire & adapters

`crates/opencodifier-schema` — every external format is an adapter
normalizing into the canonical IR.

**Native** — the project's own request/response wire format; `native.rs`
validates with `reject_unknown_fields` and re-checks `Limits` field-by-field
(`check_limits`).

**OpenAI adapter** — `openai.rs` decodes `/v1/chat/completions`-style
requests into IR; response envelopes are OpenAI-shaped. Non-text content
blocks are skipped in the response direction (the runtime never generates
prose).

**Anthropic adapter** — `anthropic.rs` decodes `/v1/messages`-shaped
requests, preserving `cache_control` markers.

**Jev adapter** — `jev.rs` normalizes the Jev/System One wire format into the
IR — Jev compatibility is an adapter mode, not the runtime's identity. Jev is
text-only by contract. `macjev.rs` is the `macjev-render-v1` renderer: the
single byte-exact definition of the prompt text the decision-model rungs
train and serve on.

**`schema.*` codes** — the adapter layer's stable error codes (e.g.
`schema.invalid_value`, `schema.unsupported_generation_field`). Like `ir.*`:
append-only. `crates/opencodifier-schema/src/error.rs`.

**`engine::wire`** — the transport-neutral native decode → decide → encode
path shared byte-for-byte by the WASM binding, the C ABI, and the conformance
fixtures. `opencodifier-engine/src/wire.rs`. The HTTP surface deliberately
selects among all four formats per route instead.

## Interfaces

**`opencodifier-http`** — the axum 0.8 surface (D12): `/v1/decide`,
`/v1/chat/completions`, `/v1/messages`, `/v1/batch`, `/health`, `/metrics`.
Binds `127.0.0.1` by default; `0.0.0.0` requires an explicit flag (§73).

**`opencodifier-mcp`** — the Model Context Protocol server (rmcp, stdio):
six tools over the same engine. Error codes relay verbatim.

**`opencodifier-cli`** — the `opencodifier` binary: `decide`, `serve`,
`doctor`, ladder/profile tooling. Exit codes are stable (D13). Missing
optional capability is a typed refusal with a code (e.g.
`cli.model_rung_unavailable`), never a silent degrade.

**`opencodifier-wasm`** — the browser surface (core + engine + schema only —
no ML runtime, D23). **`opencodifier-ffi`** — the C-ABI staticlib (D24).
Both ride `engine::wire`, so a decision in a browser walks the same code as
one in a fixture.

**`opencodifier-itemgen`** — solver-verified relational/contrastive item
generation for the training corpus: every generated item is verified against
`RelationalSolver` itself (D37). A *training-data* tool — it never runs at
inference time.

## Evaluation & release

**The suite** — the internal decision benchmark split (231 items) used for
model arms. Numbers live in `docs/BENCHMARKS.md` (the comparison page of
record).

**JevBench** — the external anchor benchmark; one leg of the two release
gates.

**Benchmark Heaven (the board)** — the public multi-system board; one number
per surface, never a universal score (`docs/BENCHMARKS.md`; four eval
surfaces are kept separate by design).

**Release gate** — the two-leg publication bar for model weights: suite ≥
0.68 AND JevBench ≥ 0.6494 (`docs/RELEASE_POLICY.md`). Below the gate,
weights are withheld — the gate is the product.

**D9 budgets** — the per-stage performance budgets (`docs/DECISIONS.md`),
measured with criterion benches, never assumed; new rows land as
backend-declared until measured on the reference host.

**Ladder recipe / profile artifact** — the shipped, fitted gate sets
(`LadderProfile` documents) that make selective prediction configurable
without recompiling; loaded via `--ladder`.

## References

In-repo records: [`docs/DECISIONS.md`](DECISIONS.md) (binding, D1–D37+) ·
[`docs/RESEARCH.md`](RESEARCH.md) (research sweep + standing watch-list) ·
[`docs/TRAINING.md`](TRAINING.md) (model-arm verdicts) ·
[`docs/BENCHMARKS.md`](BENCHMARKS.md) (numbers of record) ·
[`docs/VALIDATION.md`](VALIDATION.md) (validation campaigns) ·
[`docs/SPEC_COVERAGE.md`](SPEC_COVERAGE.md) (spec ↔ implementation map) ·
[`docs/PLANNING.md`](PLANNING.md) (founding intent, §§1–80) ·
[`docs/PLAN.md`](PLAN.md) (live phases).

External: SigLIP 2 (arXiv:2502.14786) — distilled small vision encoders,
the reference class for the planned vision rung · Gatekeeper loss
(arXiv:2502.19335), early abstention in cascades (arXiv:2502.09054),
rational cascade tuning (arXiv:2501.09345) — cascade/calibration literature
anchoring the ladder design (full annotated list: RESEARCH.md) ·
[`ort`](https://docs.rs/ort) (ONNX Runtime binding) ·
[llama.cpp](https://github.com/ggml-org/llama.cpp) (serving fork used by the
model rungs) · [`tokenizers`](https://docs.rs/tokenizers) (HuggingFace
tokenizer, behind its feature gate) · Okapi BM25 (Robertson & Zaragoza) —
the narrowing scorer.
