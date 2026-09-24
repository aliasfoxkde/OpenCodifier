# Architecture

How OpenCodifier is put together. Normative alongside PLANNING.md; this
document wins where it is more current. Status markers: **done** = shipped
in a committed phase, **contract** = frozen v1 surface.

## 1. Planes

| Plane | Crates | Talks to |
|-------|--------|----------|
| Decision core | `opencodifier-core` | nothing (pure IR + math) |
| Deterministic engine | `opencodifier-engine` | core |
| Schema adapters | `opencodifier-schema` | core |
| Inference runtime | `opencodifier-runtime` | core (+ `ort` behind one feature) |
| Decision model | `opencodifier-model` | engine, runtime, core |
| Interfaces | `opencodifier-cli`, `-http`, `-mcp`, `-wasm` | engine/model, schema |

The dependency arrows only ever point toward `core`. Nothing in the
workspace depends on a concrete inference library except
`opencodifier-runtime`, and only under its `onnx` feature.

## 2. The decision pipeline

Every request walks a fixed stage order; each stage must declare its cost
class before running (PLANNING.md §15, per-stage budgets in
`docs/DECISIONS.md`):

1. **Normalize** — wire request → canonical IR (`DecisionRequest`).
2. **Exact rule match** — deterministic rules produce a distribution
   outright; hits never touch a model.
3. **Exact decision cache** — keyed by `CacheKey` (see §4); hits are
   traced and returned as-is.
4. **Candidate narrowing** — metadata filters, then BM25 lexical scoring,
   then (if configured) embeddings, until the candidate set fits budget.
5. **Semantic scoring** — embedding similarity ranks surviving candidates.
6. **Decision model** — candidate-conditioned scorer produces the answer
   distribution (ONNX backend or deterministic fallback).
7. **Calibration + confidence gate** — raw scores → calibrated confidence
   (`ConfidenceReport`); `outcome_for(policy)` routes to
   accept / verify / abstain.
8. **Verification** — disagreement or low margin triggers a second,
   independent pass; verifier agreement feeds the report.
9. **Respond** — `DecisionResponse` + `DecisionTrace` + `DecisionMetrics`.

Stages 2–6 are skippable by configuration; 7–9 always run.

## 3. Frozen v1 contracts

### 3.1 Canonical IR (`opencodifier-core`)

- `State` — text + `BTreeMap<String, FactValue>` (canonical order is part
  of the cache-key contract).
- `DecisionQuestion` — `Choice` / `Boolean` / `Score`, internally tagged
  `"type"`; candidate/level uniqueness and non-emptiness validated at
  construction.
- `DecisionAnswer` — full `Distribution` (keys = candidate ids / levels),
  normalized within `SUM_TOLERANCE = 1e-6`.
- `DecisionPolicy` — `min_confidence 0.80`, `verify_below 0.65`,
  `abstain_below 0.50`, `RiskLevel`; ordering `abstain ≤ verify ≤ min`.
- Outcome routing: below `abstain_below` → `Abstain`; at/above
  `min_confidence` and risk Low/Medium → `Accept`; everything else →
  `Verify`. High/Critical risk never auto-accepts.

### 3.2 Trace

`DecisionTrace { trace_version: u32 = 1, entries }`; entries carry node +
sorted detail facts. Byte-stable serialization (BTreeMap everywhere).
Traces record execution facts, never chain-of-thought.

### 3.3 Cache key

Single normative builder lives in the engine (`CacheKeyBuilder`): SHA-256
over length-prefixed fields — canonical request (sorted candidates),
graph version, model id, calibration version, engine semver. Adding a
field = append + version bump; never re-order.

### 3.4 Errors

One enum per crate, each with `code()` returning stable strings
(`ir.*` in core). Wire surfaces map codes to HTTP/MCP payloads verbatim;
codes are additive-only after v1.

## 4. Floating-point policy

- All decision math (softmax, calibration, margins, entropy, weighted
  score means) is `f64` in Rust — **never** inside an ONNX graph, so
  CPU/GPU/quantization cannot change a decision.
- ONNX returns logits; Rust converts.
- Comparisons use `total_cmp` or explicit tolerances; `float_cmp` is
  allowed only in test modules.

## 5. Concurrency model

The core and engine are **synchronous** — deterministic, testable, and
trivially embeddable. I/O shapes are provided by traits (`Clock`,
`Deadline`, `CancellationToken`, `InferenceBackend`); async wrappers (Tokio,
wasm) live in interface crates. The HTTP/MCP servers own any threading;
the engine stays single-threaded per request.

## 6. Schema adaptation

`opencodifier-schema` is the only place wire formats are named. Native IR
is the truth; adapters are total functions in both directions with
fixture-locked round-trips (Phase 2): native ⇄ OpenAI structured outputs,
native ⇄ Anthropic tool schemas, native ⇄ Jev/System One (`state` /
`model` / `questions`, score = probability-weighted mean index, `noul` =
probability of yes).

## 7. Failure posture

- Malformed/hostile input → typed error with stable code, never a panic.
- Unknown wire fields are ignored forward-compatibly, except the native
  IR which denies unknown fields.
- Model artifacts are SHA-256-verified before load (`models/` is never
  committed).
- Default posture is local: bind `127.0.0.1`, no outbound calls.
