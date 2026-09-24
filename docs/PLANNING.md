Yes. I would tighten the definition substantially before handing it to an implementation agent.

## What OpenCodifier actually is

Your phrase **“ultra-fast, sophisticated and intelligent, deterministic-first decision tree classifier”** captures the intent, but *decision tree* is too narrow technically. It implies a conventional ML tree such as CART/Random Forest, whereas what we're actually designing is more powerful:

> **OpenCodifier is a local-first decision runtime that combines deterministic rules, hierarchical decision graphs, lightweight NLP/semantic models, probabilistic classification, candidate narrowing, verification, caching, and calibrated confidence to turn unstructured input into small, machine-actionable decisions—without generating natural-language answers.**

Or, more product-oriented:

> **OpenCodifier is an ultra-fast, deterministic-first AI decision engine. It progressively understands, filters, narrows, scores, verifies, and selects—using the cheapest reliable mechanism at each stage—so expensive generative AI only runs when necessary.**

That distinction is important.

### It is not merely:

```text
text → classifier → label
```

### It is:

```text
                 UNSTRUCTURED INPUT
                        │
                        ▼
              ┌───────────────────┐
              │ Normalize / Parse │
              └─────────┬─────────┘
                        │
                        ▼
              ┌───────────────────┐
              │ Deterministic     │
              │ Rules / Filters   │
              └─────────┬─────────┘
                        │
                  candidate set
                        │
                        ▼
              ┌───────────────────┐
              │ Fast NLP /        │
              │ Semantic Analysis │
              └─────────┬─────────┘
                        │
                   narrowed set
                        │
                        ▼
              ┌───────────────────┐
              │ Decision Model    │
              │ Choice/Score/Bool │
              └─────────┬─────────┘
                        │
                   confidence
                        │
                ┌───────┴───────┐
                │               │
             certain         uncertain
                │               │
                ▼               ▼
             accept          verifier
                                │
                         ┌──────┴──────┐
                         │             │
                       agree        disagree
                         │             │
                         ▼             ▼
                       accept        abstain
                                      / escalate
```

And that entire thing can be represented as a **Decision Graph**.

This is much closer to what we should build.

The research reinforces this architecture. Jev itself is explicitly positioned as a typed-decision system rather than a chat model: unstructured state goes in, typed probabilistic decisions come out; its public primitives are essentially choice, score, and boolean/noul, with calibrated probabilities and parallel evaluation. ([TypeSafe AI][1])

The open ecosystem has independently converged on several useful pieces: Laya uses non-autoregressive typed decisions; Von targets calibrated local decisions; `open-jev` demonstrates one-pass candidate scoring with a normal language model; and `jevlike`/related projects demonstrate candidate-conditioned scoring and learned decision heads. ([GitHub][2])

So **OpenCodifier should not be "our Jev clone."** It should be the broader deterministic-first decision architecture that can implement Jev-like behavior as one mode.

---

# OpenCodifier V1 — Comprehensive Implementation Plan

# OpenCodifier V1

## Deterministic-First Local AI Decision Runtime

**Status:** Greenfield implementation plan
**Language:** Rust
**Primary targets:** Windows, Linux, macOS, WASM
**License:** Apache-2.0 recommended
**Primary inference target:** ONNX Runtime / ONNX models
**Secondary/future runtime:** Burn
**Primary integration:** Amortyx
**Protocol compatibility:** OpenCodifier native + Jev/System One + OpenAI + Anthropic schemas
**Default behavior:** Local, offline, private, no telemetry

---

# 1. Mission

Build OpenCodifier as an open-source, local-first decision engine that transforms unstructured state into narrow, machine-actionable decisions.

OpenCodifier must:

1. Prefer deterministic logic before ML inference.
2. Narrow candidate sets before expensive semantic inference.
3. Use lightweight NLP/semantic models for unresolved decisions.
4. Support typed probabilistic decisions:

   * Choice
   * Boolean
   * Score
5. Support dynamically supplied candidates.
6. Support multiple decision stages.
7. Support confidence, calibration, abstention, and verification.
8. Cache reusable work.
9. Batch independent decisions.
10. Produce machine-readable outputs without generated prose.
11. Support Jev-compatible decision schemas.
12. Accept OpenAI and Anthropic-style structured schemas.
13. Run completely locally and offline.
14. Be embeddable as a Rust library.
15. Expose CLI, HTTP, MCP, and WASM interfaces.
16. Remain useful without a neural model for deterministic workloads.
17. Allow the underlying model to be replaced without changing the decision API.

The project must optimize for:

**correctness → determinism → latency → resource efficiency → privacy → usability → extensibility.**

---

# 2. Product Definition

OpenCodifier is NOT:

* a chatbot
* a generative LLM
* an LLM wrapper
* an agent framework
* a vector database
* merely a classifier
* merely a decision tree
* a replacement for GPT/Claude/etc.

OpenCodifier IS:

> A deterministic-first decision runtime that progressively reduces uncertainty and candidate space until it can make, verify, or safely defer a machine-readable decision.

Its fundamental operation is:

```text
STATE
  +
DECISION QUESTION
  +
CANDIDATE SPACE
        ↓
DECISION
```

Examples:

```text
Which model should process this request?

Does this request require external information?

How difficult is this task?

Which tool should be used?

Should this tool call be allowed?

Does this cached result apply?

Which skill should be activated?

Does this context belong in the active context window?

Should this request escalate to a larger model?

Which of these 50 documents are relevant?

Does this result require verification?
```

---

# 3. Core Design Principle

OpenCodifier must use the cheapest reliable mechanism first.

Preferred execution order:

```text
1. Exact deterministic rule
2. Cached decision
3. Metadata/filtering
4. Lexical matching
5. Lightweight semantic similarity
6. Small classifier
7. Candidate-conditioned decision model
8. Secondary verifier
9. Larger local model
10. External generative model
```

Never invoke a more expensive layer if a previous layer can establish the decision with sufficient confidence.

This is the central optimization principle of OpenCodifier.

---

# 4. Terminology

Use the following terminology consistently.

## Decision

A machine-actionable result.

## Candidate

One possible answer.

Example:

```text
qwen
glm
kimi
claude
```

## Question

The semantic decision being made.

## State

The input/context against which the question is evaluated.

## Decision Graph

A directed acyclic graph of deterministic operations and decision nodes.

## Decision Node

A node that produces one or more typed decisions.

## Gate

A deterministic threshold or condition controlling whether execution proceeds.

## Verifier

A secondary model or deterministic mechanism used when the primary decision is uncertain or high-risk.

## Abstention

Explicitly refusing to make a sufficiently reliable decision.

Abstention is a successful outcome, not an error.

## Confidence

Calibrated estimate associated with the decision.

Do not treat raw neural softmax probability as calibrated confidence.

---

# 5. Canonical Decision Types

V1 must support three primary decision types.

## Choice

Select one candidate from a dynamic candidate list.

```json
{
  "type": "choice",
  "question": "Which model should handle this request?",
  "candidates": {
    "qwen": "General coding and reasoning",
    "glm": "Complex reasoning",
    "kimi": "Very large context"
  }
}
```

Response:

```json
{
  "type": "choice",
  "choice": "qwen",
  "probabilities": {
    "qwen": 0.78,
    "glm": 0.17,
    "kimi": 0.05
  },
  "confidence": 0.78
}
```

---

## Boolean

Equivalent to Jev's `noul` concept.

```json
{
  "type": "boolean",
  "question": "Does this request require external information?"
}
```

Response:

```json
{
  "type": "boolean",
  "value": true,
  "probability": 0.93,
  "confidence": 0.93
}
```

---

## Score

Ordered decision.

```json
{
  "type": "score",
  "question": "How difficult is this request?",
  "levels": [
    "trivial",
    "easy",
    "moderate",
    "difficult",
    "expert"
  ]
}
```

Return:

```json
{
  "type": "score",
  "value": 3.8,
  "level": "difficult",
  "probabilities": {
    "trivial": 0.01,
    "easy": 0.04,
    "moderate": 0.17,
    "difficult": 0.61,
    "expert": 0.17
  }
}
```

The engine should retain the complete distribution rather than only the selected answer.

---

# 6. Native Internal Representation

Everything must normalize into a canonical Rust representation.

```rust
pub enum DecisionQuestion {
    Choice(ChoiceQuestion),
    Boolean(BooleanQuestion),
    Score(ScoreQuestion),
}
```

Common structure:

```rust
pub struct DecisionRequest {
    pub state: State,
    pub questions: Vec<DecisionQuestion>,
    pub policy: DecisionPolicy,
    pub metadata: RequestMetadata,
}
```

Response:

```rust
pub struct DecisionResponse {
    pub answers: Vec<DecisionAnswer>,
    pub confidence: ConfidenceReport,
    pub trace: Option<DecisionTrace>,
    pub metrics: DecisionMetrics,
}
```

Do not make OpenAI, Anthropic, or Jev formats the internal representation.

They are adapters.

---

# 7. Compatibility Architecture

```text
                 External Schema
                      │
        ┌─────────────┼─────────────┐
        ▼             ▼             ▼
      OpenAI       Anthropic       Jev
        │             │             │
        └─────────────┼─────────────┘
                      ▼
              Canonical OC IR
                      │
                      ▼
             OpenCodifier Engine
```

OpenAI structured outputs and Anthropic tool schemas must be translated into the internal IR rather than executed directly. This keeps the core independent of vendor-specific semantics. OpenAI documents JSON Schema structured outputs, while Anthropic exposes JSON-schema-based `input_schema` for tools. Validate against the current vendor specifications during implementation rather than assuming all JSON Schema features are equivalent.

---

# 8. V1 JSON Schema Support

Support the decision-oriented subset first.

### Fully supported

```text
type
enum
const
boolean
integer
number
oneOf
anyOf where safely reducible
required
description
title
minimum
maximum
```

### Decision mappings

```text
enum
    ↓
Choice

boolean
    ↓
Boolean

ordered enum
    ↓
Score

numeric bounded range
    ↓
Score / bucketed Score

discriminator
    ↓
Choice
```

### Do not pretend to support arbitrary generation

A free-form string:

```json
{
  "explanation": {
    "type": "string"
  }
}
```

must not become an OpenCodifier task.

Return:

```text
unsupported_generation_field
```

The generative model remains responsible for prose.

---

# 9. Decision Graph

This is the heart of OpenCodifier.

A graph consists of nodes.

V1 node types:

```text
input
normalize
rule
cache
filter
lexical
embedding
classify
score
boolean
choice
verify
fuse
threshold
branch
retrieve
rerank
transform
output
```

Example:

```text
INPUT
  │
  ▼
NORMALIZE
  │
  ▼
CACHE?
 ├── HIT ───────────────► OUTPUT
 │
 └── MISS
      │
      ▼
  FILTER MODELS
      │
      ▼
  CLASSIFY TASK
      │
      ├── coding
      │
      ├── research
      │
      └── general
      │
      ▼
  SCORE COMPLEXITY
      │
      ▼
  CHOOSE MODEL
      │
      ▼
  CONFIDENCE GATE
      │
      ├── ACCEPT ────────► OUTPUT
      │
      └── VERIFY
              │
              ├── AGREE ─► OUTPUT
              │
              └── DISAGREE → ABSTAIN/ESCALATE
```

Graphs must be declarative and serializable.

Do NOT create an embedded scripting language in V1.

---

# 10. Deterministic Execution Engine

Implement this before sophisticated ML.

The engine must support:

* topological execution
* dependency tracking
* parallel independent nodes
* short-circuiting
* conditional branches
* result propagation
* timeout
* cancellation
* cache lookup
* trace generation

Example:

```yaml
nodes:

  normalize:
    type: normalize

  cache:
    type: cache
    depends_on: [normalize]

  context:
    type: score
    depends_on: [normalize]

  tools:
    type: boolean
    depends_on: [normalize]

  route:
    type: choice
    depends_on:
      - context
      - tools
```

`context` and `tools` can execute concurrently.

---

# 11. Deterministic Rules

Implement a rule engine.

Example:

```yaml
rules:

  - when:
      context_tokens:
        gt: 100000
    set:
      requires_long_context: true

  - when:
      modality:
        contains: image
    set:
      requires_vision: true

  - when:
      privacy:
        equals: local_only
    exclude_models:
      - cloud
```

Rules must execute before ML wherever possible.

---

# 12. Candidate Narrowing

This is one of OpenCodifier's major differentiators.

Example:

```text
Registered models:
50

Deterministic filtering:
50 → 22

Capability filtering:
22 → 11

Embedding similarity:
11 → 6

Decision model:
6 → 2

Verifier:
2 → 1
```

Never feed all candidates into the semantic model if deterministic constraints can eliminate them first.

---

# 13. Candidate-Conditioned Classification

Do not build only a conventional fixed-label classifier.

The system must support:

```text
STATE
+
QUESTION
+
N dynamically supplied candidates
```

The candidate set must be runtime-defined.

This is one of the most important lessons from the current Jev-like ecosystem. `open-jev` demonstrates scoring pre-written options in one forward pass, while the `jevlike`-derived design uses a learned option/context scoring head. ([GitHub][3])

The architecture should therefore support:

```text
context representation
        +
candidate representation
        ↓
candidate score
        ↓
softmax
```

rather than:

```text
context
 ↓
fixed classifier
 ↓
class 1 / class 2 / class 3
```

---

# 14. First ML Model

Build the first OpenCodifier model as a small candidate-conditioned encoder.

Target characteristics:

```text
non-autoregressive
small
CPU friendly
quantizable
ONNX exportable
WASM-capable
dynamic candidate count
batch-friendly
```

The model should not generate text.

It should return scores.

Potential architecture:

```text
Tokenizer
    │
    ▼
Context Encoder
    │
    ├─────────────────┐
    │                 │
    ▼                 ▼
Context tokens    Candidate encoder
    │                 │
    └────────┬────────┘
             ▼
       Cross-attention
             │
             ▼
        scalar logit
```

Repeat candidate scoring in a batch.

---

# 15. Model Training Architecture

Training is not the initial blocker.

First establish the inference contract.

Then construct:

```text
teacher data
     ↓
labeled decision examples
     ↓
student model
     ↓
calibration
     ↓
evaluation
```

Potential teacher sources:

* synthetic deterministic rules
* human labels
* existing LLMs
* successful Amortyx routing outcomes
* public classification datasets
* Jev-like benchmark tasks
* specialized task datasets

Use teacher LLMs only during dataset construction.

The deployed OpenCodifier must not require those APIs.

---

# 16. Training Data Format

Use a simple JSONL representation.

```json
{
  "state": "...",
  "question": {
    "type": "choice",
    "text": "Which model should process this?"
  },
  "candidates": [
    {
      "id": "qwen",
      "description": "..."
    },
    {
      "id": "glm",
      "description": "..."
    }
  ],
  "label": "qwen"
}
```

For confidence-aware training, optionally retain:

```json
{
  "label": "qwen",
  "quality": 0.92,
  "teacher_confidence": 0.88
}
```

Do not blindly train on teacher confidence as ground truth.

---

# 17. Calibration

Calibration is mandatory.

Measure:

```text
ECE
Brier score
NLL
reliability
selective accuracy
coverage
```

Implement temperature scaling first.

Later support:

```text
isotonic regression
conformal prediction
task-specific calibration
```

The current Jev/open ecosystem strongly reinforces this requirement. Jev explicitly emphasizes calibrated decisions; Von reports Brier/CE-based calibration; `poorjev` demonstrates temperature scaling plus conformal abstention. ([TypeSafe AI][1])

---

# 18. Confidence Must Be Multi-Dimensional

Do not expose only:

```text
confidence: 0.91
```

Internally track:

```text
top_probability
margin
entropy
calibration
OOD_score
candidate_count
model_quality
verifier_agreement
```

Example:

```json
{
  "top_probability": 0.82,
  "margin": 0.57,
  "entropy": 0.31,
  "calibrated_confidence": 0.79,
  "ood_score": 0.06,
  "verifier_agreement": true
}
```

---

# 19. Two-Model Verification

Implement a cascade.

```text
Primary classifier
       │
       ▼
Uncertainty gate
       │
       ├── confident → accept
       │
       └── uncertain
              │
              ▼
          verifier
              │
        ┌─────┴─────┐
        ▼           ▼
      agree      disagree
        │           │
        ▼           ▼
      accept      abstain
```

Do NOT execute both models on every request.

Trigger verification when:

```text
entropy > threshold
OR margin < threshold
OR confidence < threshold
OR OOD > threshold
OR high-risk action
OR irreversible action
OR candidate ambiguity
```

The verifier should preferably use a meaningfully different architecture or training source to reduce correlated errors.

---

# 20. Risk-Aware Decisions

Every graph can classify actions:

```text
LOW
MEDIUM
HIGH
CRITICAL
```

Examples:

```text
Choose formatting style → LOW

Choose AI model → LOW/MEDIUM

Select tool → MEDIUM

Modify source code → MEDIUM/HIGH

Delete data → HIGH

Production deployment → CRITICAL
```

Higher-risk decisions require stronger confidence or verification.

This must be deterministic policy, not model preference.

---

# 21. Abstention

Abstention is a core feature.

Possible outputs:

```text
ACCEPT
VERIFY
ABSTAIN
ESCALATE
NO_VALID_CANDIDATE
```

Never force the classifier to select an answer merely because a candidate exists.

---

# 22. Batch Inference

If ten questions use the same state:

```text
Q1 task type
Q2 complexity
Q3 tool requirement
Q4 context size
Q5 privacy
Q6 model
...
```

the engine must batch them.

Jev explicitly emphasizes parallel evaluation; Laya likewise describes typed questions evaluated in a single forward pass. ([TypeSafe AI][1])

OpenCodifier should make this automatic.

---

# 23. Cache Architecture

Implement multiple caches.

## Exact Decision Cache

Key:

```text
hash(
  normalized_state
  +
  question
  +
  candidate_set
  +
  model_version
  +
  calibration_version
  +
  policy_version
)
```

## Compiled Schema Cache

Cache parsed schemas.

## Tokenization Cache

Cache repeated candidate tokenization.

## Candidate Representation Cache

Cache candidate embeddings/representations.

## Semantic Cache

Optional.

Use similarity only when policy permits approximate reuse.

Never silently reuse a semantic result where correctness requirements demand exact evaluation.

---

# 24. Embeddings

Embeddings should be included as an optional subsystem, not the core identity.

Useful for:

* semantic cache
* candidate retrieval
* skill selection
* memory selection
* duplicate detection
* document selection
* model capability matching
* graph routing

Rust `fastembed-rs` is a useful reference because it already provides local embedding and reranking support and includes quantized embedding models. ([GitHub][4])

Architecture:

```text
OpenCodifier Core
       │
       └── optional semantic feature
             ├── embeddings
             ├── similarity
             ├── retrieval
             └── reranking
```

---

# 25. Do Not Make Vector Search Mandatory

V1 must work without:

* vector DB
* database server
* Python
* Docker
* cloud
* API key

For local semantic retrieval, investigate:

* USearch
* embedded HNSW
* simple memory-resident vectors

For lexical retrieval, Tantivy is an excellent Rust reference and already provides BM25, low startup overhead, SIMD-related optimizations, and mmap-based storage. ([GitHub][5])

---

# 26. Reranking

Expose a reranker interface:

```rust
trait Reranker {
    fn rerank(
        &self,
        query: &str,
        candidates: &[Candidate]
    ) -> Result<Vec<ScoredCandidate>>;
}
```

Do not make reranking mandatory in V1.

Pipeline:

```text
50 candidates
   ↓
metadata
   ↓
BM25 / embedding
   ↓
10
   ↓
reranker
   ↓
3
   ↓
decision model
```

`fastembed-rs` already provides a useful reference implementation and supports multiple reranking models. ([GitHub][4])

---

# 27. Model Runtime

Implement a runtime abstraction.

```rust
trait InferenceBackend {
    fn load(&self, model: &ModelArtifact) -> Result<ModelHandle>;
    fn infer(
        &self,
        model: &ModelHandle,
        input: &InferenceInput
    ) -> Result<InferenceOutput>;
}
```

V1:

```text
ONNX Runtime
```

Future/parallel:

```text
Burn
```

Do not hard-code OpenCodifier to ONNX APIs.

---

# 28. Why ONNX + Burn

ONNX provides a portable model interchange layer.

Burn is particularly interesting because its Rust ecosystem supports CPU/GPU/WebAssembly backends and ONNX import. Its current ONNX tooling can generate native Burn Rust code and target WebAssembly, CUDA and WebGPU, although operator coverage and portability must still be tested against the exact OpenCodifier model. ([GitHub][6])

Recommended architecture:

```text
OpenCodifier Model
       │
       ├── ONNX artifact
       │
       ├── native ORT
       │
       └── optional Burn-native artifact
```

---

# 29. WASM

Browser deployment is a first-class target.

ONNX Runtime Web supports WASM CPU execution and browser GPU execution through WebGPU/WebNN paths; it also supports SIMD and threaded WASM builds. ([ONNX Runtime][7])

However, do not assume the Rust `ort` crate itself gives us a clean browser build.

Therefore define:

```text
Native:
Rust → ORT

Browser:
Rust/WASM core
    +
ONNX Runtime Web bridge
or
Burn WASM backend
```

The browser build must not require a server.

---

# 30. Tokenization

Use Hugging Face Tokenizers as the initial tokenizer foundation.

It is itself implemented in Rust and is designed for high-performance tokenization and production use. ([GitHub][8])

Tokenizer must be encapsulated behind:

```rust
trait Tokenizer {
    fn encode(&self, text: &str) -> Result<Encoding>;
}
```

This prevents future model changes from contaminating the engine.

---

# 31. MCP

MCP is a first-class interface.

Use the official Rust `rmcp` SDK. The current SDK implements the stable MCP `2026-07-28` specification and includes server/client functionality. ([GitHub][9])

Command:

```bash
opencodifier mcp serve
```

Initial tools:

```text
codify_decide
codify_batch
codify_graph
codify_validate
codify_explain
codify_classify
codify_score
codify_verify
```

Do not expose chain-of-thought.

`codify_explain` returns an execution trace:

```json
{
  "node": "model_selection",
  "candidate_count_before": 18,
  "candidate_count_after": 5,
  "confidence": 0.91,
  "verified": false,
  "cache_hit": false
}
```

---

# 32. MCP Tool Introspection

OpenCodifier should eventually consume MCP tool schemas.

Input:

```text
MCP server
   ↓
tool schemas
   ↓
candidate registry
   ↓
capability index
```

Then OpenCodifier can decide:

```text
Which tool?
Is a tool required?
Which tool is safest?
Does this tool require confirmation?
Can another tool satisfy the request?
```

This should be implemented after the core MCP server works.

---

# 33. Skills

Ship a skills directory:

```text
skills/

  routing/
  tool-selection/
  tool-gating/
  context-pruning/
  model-selection/
  escalation/
  verification/
  memory-selection/
```

A skill describes **how an AI should use OpenCodifier**.

It should not contain large amounts of duplicated decision logic.

Example:

```text
When deciding which model to use:

1. Send request to OpenCodifier.
2. Provide candidate model capabilities.
3. Apply deterministic constraints first.
4. Inspect confidence.
5. Verify low-confidence decisions.
6. Escalate if OpenCodifier abstains.
```

---

# 34. Built-in Recipes

Ship ready-to-use graphs.

```text
recipes/

model-routing
task-classification
tool-selection
tool-gating
context-pruning
cache-eligibility
skill-selection
memory-selection
escalation
verification
document-relevance
code-review-risk
```

Usage:

```bash
opencodifier recipe install model-routing
```

This is essential for adoption.

---

# 35. OpenCodifier CLI

Commands:

```text
opencodifier init
opencodifier decide
opencodifier batch
opencodifier graph
opencodifier graph validate
opencodifier graph run
opencodifier model list
opencodifier model install
opencodifier model remove
opencodifier model inspect
opencodifier recipe list
opencodifier recipe install
opencodifier benchmark
opencodifier serve
opencodifier mcp serve
opencodifier doctor
```

Example:

```bash
opencodifier decide \
  --schema model-routing.json \
  --state request.json
```

Output:

```json
{
  "decision": "qwen",
  "confidence": 0.91
}
```

Human-readable mode:

```text
Decision: qwen
Confidence: 91%
Verified: no
Latency: 3.8 ms
Cache: miss
Candidates: 8 → 3
```

---

# 36. HTTP API

Primary:

```text
POST /v1/decide
POST /v1/batch
POST /v1/graph/run
POST /v1/validate
```

Metadata:

```text
GET /v1/health
GET /v1/models
GET /v1/capabilities
```

Compatibility:

```text
POST /v1/systemone
```

The compatibility endpoint must emulate the supported Jev/System One schema, while the native `/v1/decide` API remains the canonical OpenCodifier API.

---

# 37. Local Server

Default:

```bash
opencodifier serve
```

Must bind locally by default:

```text
127.0.0.1
```

Do not expose network access unless explicitly requested.

Example:

```bash
opencodifier serve --host 0.0.0.0
```

must require an explicit flag.

---

# 38. Privacy

Default:

```text
No telemetry
No analytics
No cloud calls
No account
No API key
No remote logging
No automatic uploads
```

All model downloads must be explicit.

Model integrity:

```text
SHA-256
signature
manifest
license
```

---

# 39. Project Layout

Use a Rust workspace:

```text
opencodifier/

├── Cargo.toml
├── Cargo.lock
├── README.md
├── LICENSE
├── SECURITY.md
├── CONTRIBUTING.md
├── CHANGELOG.md
│
├── crates/
│   ├── opencodifier-core/
│   ├── opencodifier-schema/
│   ├── opencodifier-engine/
│   ├── opencodifier-runtime/
│   ├── opencodifier-model/
│   ├── opencodifier-http/
│   ├── opencodifier-mcp/
│   ├── opencodifier-cli/
│   └── opencodifier-wasm/
│
├── adapters/
│   ├── openai/
│   ├── anthropic/
│   └── jev/
│
├── recipes/
├── skills/
├── models/
├── benchmarks/
├── fixtures/
├── examples/
├── tests/
└── docs/
```

Keep dependencies directional.

`core` must not depend on:

```text
HTTP
MCP
CLI
Tokio
specific ML runtime
```

---

# 40. Phase 0 — Repository and Governance

## Tasks

1. Initialize Rust workspace.
2. Select Apache-2.0.
3. Add README.
4. Add security policy.
5. Add contribution policy.
6. Add CODEOWNERS if appropriate.
7. Add issue templates.
8. Add CI.
9. Configure rustfmt.
10. Configure Clippy.
11. Configure cargo-deny.
12. Configure dependency auditing.
13. Add MSRV policy.
14. Add semantic versioning policy.

## CI baseline

Every PR:

```text
cargo fmt --check
cargo clippy --all-targets --all-features
cargo test --all
cargo test --doc
cargo deny check
```

---

# 41. Phase 1 — Canonical Decision IR

Implement:

```text
DecisionRequest
DecisionQuestion
DecisionAnswer
Candidate
ScoreLevel
DecisionPolicy
Confidence
DecisionTrace
```

Implement serialization.

Tests:

```text
Choice round trip
Boolean round trip
Score round trip
empty candidates
duplicate candidates
invalid score ordering
missing question
malformed JSON
```

Deliverable:

```bash
cargo test
```

with no ML dependency.

---

# 42. Phase 2 — Schema Normalization

Implement adapters:

```text
native OpenCodifier
OpenAI
Anthropic
Jev
```

Create fixture suites.

Example:

```text
fixtures/openai/
fixtures/anthropic/
fixtures/jev/
```

Every fixture must normalize into identical canonical IR where semantics are equivalent.

Deliverable:

```text
schema → IR → schema
```

round-trip/compatibility tests.

---

# 43. Phase 3 — Deterministic Engine

Implement:

```text
rules
filters
candidate pruning
branching
thresholds
DAG scheduling
parallel independent nodes
short circuit
```

Build a fake deterministic classifier:

```text
MockClassifier
```

This allows graph testing before the real model exists.

Deliverable:

```text
complete functioning Decision Graph engine
```

without ML.

---

# 44. Phase 4 — Cache

Implement:

```text
exact cache
schema cache
candidate cache
tokenization cache
```

Requirements:

* deterministic keys
* model versioning
* policy versioning
* TTL
* max size
* persistence optional
* in-memory default

Test:

```text
same input → same key
model update → cache invalidated
policy update → cache invalidated
candidate order normalization
```

---

# 45. Phase 5 — Candidate Narrowing

Implement:

```text
capability filters
metadata filters
rule filters
lexical matching
```

Measure:

```text
candidate reduction ratio
latency
false elimination rate
```

Never optimize candidate count at the expense of silently removing the correct candidate.

Add a "safe mode":

```text
never eliminate based solely on weak semantic evidence
```

---

# 46. Phase 6 — NLP Runtime

Implement tokenizer abstraction.

Implement first ONNX backend.

Load a known small encoder/classifier.

Do NOT train OpenCodifier yet.

The first objective is proving:

```text
Rust
→ tokenizer
→ ONNX
→ inference
→ score
→ decision
```

Deliverable:

```text
real local NLP decision
```

---

# 47. Phase 7 — OpenCodifier Decision Model

Build/train the first candidate-conditioned model.

Initial architecture:

```text
small encoder
+
candidate representation
+
cross-attention/scoring head
```

Output:

```text
one scalar per candidate
```

For N candidates:

```text
N logits
```

Then:

```text
softmax
```

Do not generate text.

---

# 48. Phase 8 — Choice / Boolean / Score

Implement all three through the same model infrastructure.

### Choice

```text
candidate logits → softmax
```

### Boolean

```text
true/false logits → softmax
```

or:

```text
binary logit → sigmoid
```

### Score

```text
ordered levels → softmax
```

Calculate:

```text
expected score
```

Also retain the full probability distribution.

---

# 49. Phase 9 — Calibration

Create calibration dataset.

Run:

```text
raw model
→ temperature scaling
→ calibrated model
```

Measure:

```text
accuracy
ECE
Brier
NLL
coverage
selective accuracy
```

Reject deployment if calibration regression exceeds defined thresholds.

---

# 50. Phase 10 — Verification

Implement:

```text
uncertainty detector
verifier
fusion
abstention
```

Test:

```text
high confidence
low confidence
near tie
OOD
high risk
verifier disagreement
```

Produce explicit states:

```text
accepted
verified
abstained
escalated
```

---

# 51. Phase 11 — Decision Graph Optimization

Once the graph engine works, add optimization passes.

Example:

```text
Before:

rule A
 ↓
classifier
 ↓
rule B
 ↓
classifier
```

Optimizer:

```text
rule A + rule B
       ↓
candidate reduction
       ↓
single classifier
```

Other optimizations:

```text
common subexpression elimination
constant folding
dead-node elimination
parallelization
cache insertion
batching
candidate pruning
early exit
```

This is where OpenCodifier starts behaving like a **decision compiler**, rather than merely a classifier.

That is a useful conceptual direction for the name "Codifier":

> It converts semantic decision logic into an executable, optimized decision graph.

---

# 52. Phase 12 — Embeddings

Implement feature-gated embedding subsystem.

Interface:

```rust
trait Embedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vector>>;
}
```

Implement:

```text
local ONNX embedding
```

Use cases:

```text
candidate retrieval
semantic cache
skill retrieval
memory selection
document relevance
model capability matching
```

Do not allow embeddings to override deterministic constraints.

---

# 53. Phase 13 — Retrieval

Implement optional:

```text
lexical retrieval
dense retrieval
hybrid retrieval
```

Use Tantivy as the lexical reference/implementation where practical; it is a Rust full-text engine with BM25 and low startup overhead. ([GitHub][5])

Use a lightweight vector implementation rather than requiring an external database.

---

# 54. Phase 14 — Reranking

Implement:

```text
Reranker trait
```

Then:

```text
retrieve top-N
     ↓
rerank
     ↓
top-K
     ↓
decision
```

Keep this optional.

---

# 55. Phase 15 — MCP

Add `rmcp`.

Implement:

```text
codify_decide
codify_batch
codify_graph
codify_validate
codify_verify
codify_explain
```

Add integration tests against MCP clients.

Then implement:

```text
MCP schema discovery
MCP tool indexing
tool candidate generation
```

---

# 56. Phase 16 — Recipes and Skills

Ship initial decision graphs:

```text
model-routing
tool-selection
tool-gating
context-pruning
cache-selection
skill-selection
memory-selection
escalation
verification
```

Each recipe must have:

```text
graph
schema
example input
example output
benchmark
documentation
```

---

# 57. Phase 17 — WASM

Build:

```text
opencodifier-wasm
```

Support:

```text
WASM CPU
WASM SIMD
optional threads
WebGPU path
```

Use ONNX Runtime Web and/or Burn according to the model/runtime implementation. ONNX Runtime Web supports WASM CPU and browser GPU paths, while Burn provides a Rust-native route to WASM/WebGPU. ([ONNX Runtime][10])

Browser demo:

```text
drag/drop text
       ↓
OpenCodifier WASM
       ↓
local decision
```

Show a network inspector demonstrating that inference occurs locally.

---

# 58. Phase 18 — CLI / Distribution

Build binaries for:

```text
Windows x86_64
Linux x86_64
Linux ARM64
macOS x86_64
macOS ARM64
```

Provide:

```text
GitHub Releases
Cargo
Homebrew
winget
install scripts
```

Model downloads remain separate.

---

# 59. Phase 19 — Benchmarking

Create a permanent benchmark suite.

Measure:

### Correctness

```text
accuracy
top-k accuracy
precision
recall
F1
```

### Calibration

```text
ECE
Brier
NLL
```

### Efficiency

```text
cold-start latency
warm latency
p50
p95
p99
throughput
RAM
model size
CPU utilization
```

### System efficiency

```text
candidate reduction
cache hit rate
verification rate
abstention rate
LLM escalation rate
tokens avoided
```

The last metrics are especially important for Amortyx.

---

# 60. Phase 20 — Amortyx Integration

Amortyx should consume OpenCodifier as an external decision subsystem.

Architecture:

```text
Amortyx
   │
   ▼
OpenCodifier
   │
   ├── classify task
   ├── estimate complexity
   ├── estimate context requirement
   ├── determine tool requirement
   ├── determine freshness requirement
   └── rank eligible models
          │
          ▼
Amortyx policy engine
          │
          ├── price
          ├── latency
          ├── availability
          ├── quota
          ├── user policy
          └── privacy
          │
          ▼
actual provider/model
```

OpenCodifier makes semantic decisions.

Amortyx makes economic/operational decisions.

Neither should absorb the other's responsibilities.

---

# 61. Amortyx Shadow Mode

Before OpenCodifier controls routing:

```text
request
 ├── existing Amortyx route
 └── OpenCodifier shadow route
```

Record:

```text
OC prediction
actual selected model
actual latency
actual cost
actual success
quality
```

Then calculate:

```text
Would OC have chosen correctly?
Would OC have saved cost?
Would OC have reduced latency?
Would verification have been necessary?
```

Only after sufficient evidence should OpenCodifier become an active routing component.

---

# 62. Training From Amortyx

Build an optional dataset pipeline:

```text
production/shadow logs
       ↓
privacy filter
       ↓
deduplication
       ↓
label extraction
       ↓
training dataset
       ↓
OpenCodifier model
       ↓
benchmark
       ↓
candidate deployment
```

Do not automatically train from every production result.

Use explicit dataset/version controls.

---

# 63. OpenCodifier Decision Registry

Implement a registry concept.

Example:

```yaml
decision:
  id: amortyx.model_selection
  version: 1

  question:
    type: choice
    text: "Which model should process the request?"

  candidates:
    dynamic: true

  policy:
    min_confidence: 0.80
    verify_below: 0.65
    abstain_below: 0.50
```

This lets decision definitions become reusable artifacts.

---

# 64. Decision Graph Versioning

Graphs must be versioned.

Cache keys must include:

```text
graph_version
model_version
calibration_version
policy_version
```

Never allow a new graph/model/calibration artifact to accidentally reuse incompatible cached decisions.

---

# 65. Explainability

OpenCodifier must never expose chain-of-thought.

Instead expose deterministic execution facts.

Example:

```json
{
  "decision": "qwen",
  "confidence": 0.91,
  "trace": [
    {
      "node": "candidate_filter",
      "before": 18,
      "after": 7
    },
    {
      "node": "semantic_classifier",
      "model": "oc-lite-1",
      "latency_ms": 4.7
    },
    {
      "node": "confidence_gate",
      "threshold": 0.80,
      "passed": true
    }
  ]
}
```

---

# 66. Security

Treat all input as hostile.

Particularly:

```text
candidate descriptions
tool descriptions
MCP schemas
JSON schemas
state text
graph files
model manifests
model files
```

Do not let input text modify:

```text
policy
thresholds
graph structure
filesystem paths
network configuration
tool permissions
```

Separate data from executable configuration.

---

# 67. Resource Limits

Every request must have:

```text
max input bytes
max tokens
max candidates
max questions
max graph nodes
max execution time
max memory
max semantic retrieval results
```

Defaults should be conservative.

---

# 68. WASM Security

Browser builds must have:

```text
no filesystem assumptions
no arbitrary networking
no telemetry
no dynamic native code
```

Model downloads must be explicit and integrity checked.

Use browser storage such as IndexedDB only through the WASM/web adapter.

---

# 69. V1 Acceptance Criteria

OpenCodifier V1 is complete when all of the following work:

### Core

* [ ] Rust library
* [ ] Decision IR
* [ ] Choice
* [ ] Boolean
* [ ] Score
* [ ] Dynamic candidates
* [ ] Confidence
* [ ] Abstention

### Deterministic engine

* [ ] Rules
* [ ] Candidate filtering
* [ ] DAG
* [ ] Branching
* [ ] Parallel execution
* [ ] Cache
* [ ] Trace

### Compatibility

* [ ] Native OC schema
* [ ] Jev/System One compatibility
* [ ] OpenAI structured schema
* [ ] Anthropic tool schema

### ML

* [ ] ONNX inference
* [ ] local tokenizer
* [ ] OpenCodifier model
* [ ] calibration
* [ ] verifier

### Interfaces

* [ ] CLI
* [ ] HTTP
* [ ] MCP
* [ ] Rust API
* [ ] WASM

### Distribution

* [ ] Windows
* [ ] Linux
* [ ] macOS
* [ ] browser
* [ ] checksums
* [ ] reproducible release artifacts

### Ecosystem

* [ ] recipes
* [ ] skills
* [ ] documentation
* [ ] examples
* [ ] benchmark suite

### Amortyx

* [ ] shadow integration
* [ ] routing integration
* [ ] decision telemetry
* [ ] cost/latency comparison

---

# 70. V1 Performance Goals

These are engineering targets, not claims.

The implementation agent must benchmark rather than assume them.

## Deterministic decisions

Target:

```text
<100 µs
```

for trivial in-memory rules where practical.

## Cached decisions

Target:

```text
sub-millisecond
```

where practical.

## Lightweight classifier

Target:

```text
single-digit to low-double-digit milliseconds
```

on ordinary desktop CPU for short inputs.

## Batched decisions

Target:

```text
amortized <5 ms/question
```

where hardware/model size permits.

## Memory

Default runtime should remain small.

The base binary must not require a large ML model.

Model size is separate from executable size.

---

# 71. V1.1

After V1 proves the architecture:

```text
better model
multilingual
semantic cache
reranker
hybrid retrieval
graph optimizer
more recipes
more MCP integrations
```

---

# 72. V2

Potential future work:

```text
adaptive decision graphs
online calibration
federated/private training
automatic distillation
model specialization
learned candidate pruning
distributed decision cache
GPU batching
mobile
edge workers
```

Do not implement these during V1 unless required to validate the architecture.

---

# 73. Critical Implementation Rules

The implementing AI must follow these rules.

### Rule 1

Do not start by training a model.

Build the contract and deterministic engine first.

### Rule 2

Do not create a monolithic Rust crate.

Maintain clear boundaries.

### Rule 3

Do not make ONNX Runtime a hard dependency of the core.

### Rule 4

Do not require Python for runtime.

Python may exist in a separate training/research repository if necessary.

### Rule 5

Do not make a vector database mandatory.

### Rule 6

Do not force every request through a neural model.

### Rule 7

Do not force every decision to produce an answer.

Abstention is valid.

### Rule 8

Do not expose raw probability as calibrated confidence.

### Rule 9

Do not execute two classifiers on every request.

Use confidence-gated verification.

### Rule 10

Do not use generated prose where a typed decision will suffice.

### Rule 11

Do not silently discard candidates because of weak semantic similarity.

### Rule 12

Do not invent support for unsupported JSON Schema constructs.

### Rule 13

Do not add telemetry.

### Rule 14

Do not add cloud dependencies to the local runtime.

### Rule 15

Do not optimize before benchmarks exist.

---

# 74. Initial Example: Amortyx Router

Input:

```json
{
  "request": "Refactor this Rust parser to remove the lifetime bug and add tests.",
  "context_tokens": 42000,
  "tools_available": [
    "filesystem",
    "shell",
    "git"
  ],
  "models": [
    "local-qwen",
    "local-glm",
    "cloud-qwen",
    "cloud-kimi"
  ]
}
```

Deterministic filtering:

```text
vision required?       no
context > 32K?         yes
local-only?             no
tools required?         yes

4 models
```

Capability filtering:

```text
local-qwen     eligible
local-glm      eligible
cloud-qwen     eligible
cloud-kimi     eligible
```

Semantic classification:

```text
coding                  0.99
reasoning               0.88
tool-use                0.94
complexity              0.79
```

Candidate scoring:

```text
local-qwen     0.44
local-glm      0.39
cloud-qwen     0.12
cloud-kimi     0.05
```

Amortyx policy then applies:

```text
cost
latency
availability
user policy
current provider health
```

Final decision:

```text
local-glm
```

OpenCodifier did not make the economic decision.

It supplied the semantic decision information.

Amortyx remains responsible for routing policy.

---

# 75. Example: Tool Selection

Input:

```text
"Find the PR that introduced this regression."
```

Candidates:

```text
git_log
github_search
filesystem_search
ripgrep
```

Deterministic:

```text
requires repository history → eliminate filesystem_search
```

Semantic:

```text
git_log       0.52
github_search 0.39
ripgrep       0.09
```

Margin is insufficient.

Verifier executes.

Result:

```text
git_log       0.67
github_search 0.28
ripgrep       0.05
```

Accept.

No LLM generation was required.

---

# 76. Example: Context Pruning

Question:

```text
Does this document belong in the active context?
```

Deterministic:

```text
same repository?       yes
same task?             yes
modified recently?     yes
```

Semantic:

```text
relevance = 0.91
```

Decision:

```text
KEEP
```

Another document:

```text
repository mismatch
old timestamp
semantic relevance 0.21
```

Decision:

```text
DEMOTE
```

This becomes directly useful for the broader AI memory/context architecture.

---

# 77. Example: Skill Selection

```text
Incoming task
    ↓
OpenCodifier
    ↓
task family = GIS
    ↓
candidate skills:
    GIS
    CAD
    Rust
    web
    database
    ↓
semantic ranking
    ↓
GIS = 0.94
CAD = 0.73
Rust = 0.52
    ↓
activate GIS
    ↓
optional CAD
```

Again, no generated response required.

---

# 78. Example: Escalation

```text
simple request
    ↓
OC Micro
    ↓
confidence = 0.96
    ↓
local model
```

Versus:

```text
ambiguous request
    ↓
OC Micro
    ↓
confidence = 0.54
    ↓
OC Small verifier
    ↓
confidence = 0.61
    ↓
abstain
    ↓
larger LLM
```

This is exactly the kind of token-saving architecture OpenCodifier should enable.

---

# 79. Final Architectural Relationship

The final AI stack should look like:

```text
                         USER / AGENT
                              │
                              ▼
                    ┌───────────────────┐
                    │   OpenCodifier    │
                    │                   │
                    │ deterministic     │
                    │ decision graphs   │
                    │ NLP               │
                    │ classifiers       │
                    │ semantic search   │
                    │ verification      │
                    │ calibration       │
                    │ caching            │
                    └─────────┬─────────┘
                              │
                  machine-actionable intent
                              │
                              ▼
                    ┌───────────────────┐
                    │      Amortyx       │
                    │                   │
                    │ routing           │
                    │ cost              │
                    │ latency           │
                    │ provider health   │
                    │ model selection   │
                    │ prompt batching   │
                    │ caching            │
                    └─────────┬─────────┘
                              │
                  ┌───────────┼────────────┐
                  ▼           ▼            ▼
                Local       Cheap         Frontier
                Models       APIs           APIs
```

OpenCodifier becomes the **semantic decision plane**.

Amortyx becomes the **inference optimization/routing plane**.

Generative models remain the **execution/generation plane**.

That separation is the key architectural decision.

---

# 80. The Core Thesis

OpenCodifier should be built around one fundamental idea:

> **Don't ask a large generative model to make a decision that can be reduced to deterministic logic, classification, scoring, retrieval, or a narrowly constrained probabilistic decision.**

Instead:

```text
Understand
   ↓
Normalize
   ↓
Filter
   ↓
Narrow
   ↓
Classify
   ↓
Score
   ↓
Verify
   ↓
Decide
   ↓
Escalate only if necessary
```

The result is not another AI assistant.

It is a **decision substrate for AI systems**.

That is why I would keep it separate from Amortyx.

Amortyx can consume it.

Coding agents can consume it.

MCP servers can consume it.

Your future memory/context system can consume it.

A web application can run it locally through WASM.

An enterprise can embed it without sending data anywhere.

And an AI agent can use it as a deterministic/semantic control layer before invoking an expensive generative model.

That is a much larger and more defensible project than simply implementing "an open-source Jev."

### One correction I would make to the original framing

I would **not market OpenCodifier as a "decision tree classifier."** Keep that as an intuitive explanation, but the technical/product term should be **decision runtime**, **decision engine**, or **decision fabric**.

A literal decision tree is only one possible implementation. OpenCodifier's real architecture is a **deterministic-first decision DAG with progressively more expensive semantic operators**.

That distinction gives us room for:

* deterministic rules
* NLP classifiers
* dynamic candidate scoring
* embeddings
* retrieval
* reranking
* calibration
* dual-model verification
* caching
* graph optimization
* MCP
* WASM
* model routing
* context pruning
* skill selection

without having to redesign the product later.

And the research supports the specific technical direction: Jev validates typed probabilistic decisions and parallel evaluation; `open-jev` validates one-pass candidate scoring; Laya and Von demonstrate small/non-autoregressive local decision models; and the broader ecosystem already includes calibration, abstention, distillation, and tiny models. ([TypeSafe AI][1])

**I would therefore create OpenCodifier as the independent greenfield project now, and make Amortyx its first deep integration rather than putting the technology inside Amortyx.**

[1]: https://typesafe.ai/blog/introducing-system-one-models-and-jev?utm_source=chatgpt.com "Introducing System One Models & Jev - TypeSafe AI Blog"
[2]: https://github.com/NandhaKishorM/laya?utm_source=chatgpt.com "GitHub - NandhaKishorM/laya · GitHub"
[3]: https://github.com/daseinlabs/open-jev?utm_source=chatgpt.com "GitHub - daseinlabs/open-jev: Open Jev implementation with custom finetuning · GitHub"
[4]: https://github.com/anush008/fastembed-rs?utm_source=chatgpt.com "GitHub - Anush008/fastembed-rs: Rust library for generating vector embeddings, reranking locally! · GitHub"
[5]: https://github.com/quickwit-oss/tantivy?utm_source=chatgpt.com "GitHub - quickwit-oss/tantivy: Tantivy is a full-text search engine library inspired by Apache Lucene and written in Rust · GitHub"
[6]: https://github.com/tracel-ai/burn?utm_source=chatgpt.com "GitHub - tracel-ai/burn: Burn is a next generation tensor library and Deep Learning Framework that doesn't compromise on flexibility, efficiency and portability. · GitHub"
[7]: https://onnxruntime.ai/docs/build/web.html?utm_source=chatgpt.com "Build for web | onnxruntime"
[8]: https://github.com/huggingface/tokenizers/blob/main/tokenizers/README.md?plain=1&utm_source=chatgpt.com "tokenizers/tokenizers/README.md at main · huggingface/tokenizers · GitHub"
[9]: https://github.com/modelcontextprotocol/rust-sdk/blob/main/README.md?utm_source=chatgpt.com "rust-sdk/README.md at main · modelcontextprotocol/rust-sdk · GitHub"
[10]: https://onnxruntime.ai/docs/tutorials/web/?utm_source=chatgpt.com "Web | onnxruntime"
