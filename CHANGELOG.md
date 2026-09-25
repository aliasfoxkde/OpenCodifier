# Changelog

All notable changes to OpenCodifier are documented here. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the
project does not yet guarantee a stable public API — version `0.x`
releases may break, and every breaking change is recorded in this file
(and reflected in `docs/DECISIONS.md` where a binding decision moves).

## [Unreleased]

### Added

- `opencodifier-engine::EngineHandle` — the shared facade every
  interface crate (CLI, HTTP, MCP) assembles and drives the runtime
  through; includes `validate_graph`, `health`, and the deterministic
  `lexical` constructor (PLAN Phase 8–10 wiring).
- `opencodifier-http` crate in progress — axum **0.8** `/v1` surface
  (`POST /v1/decide`, `POST /v1/graph/validate`, `GET /v1/healthz`),
  loopback-by-default bind gate (D12). The original D12 pin said
  "axum 0.9"; that version does not exist and the record was amended
  to the measured reality.

### Changed

- `opencodifier-runtime` backend traits now require `Debug` (matching
  the engine `Classifier` precedent) so interface crates can hold
  boxed backends in debuggable facades.

## [0.1.0] — in development

Phased build-out per `docs/PLAN.md`; hashes refer to the repository
history.

### Added

- **Phase 0 — scaffold + governance** (37b8b63, 0611b5a): Rust
  workspace (edition 2024, MSRV 1.90), workspace lints (`unsafe_code`
  forbid, clippy all+pedantic with unwrap/expect/panic/todo denied),
  CI configs (`justfile`, `.gitforge.yml` — GitForge is the CI platform
  of record), founding docs.
- **Phase 1 — `opencodifier-core`** (4f8900c, fb995cd): canonical
  decision IR — validating constructors for Choice/Boolean/Score
  questions, distributions, confidence reports with outcome routing,
  stable `ir.*` error codes, `trace_version` on traces,
  `#[non_exhaustive]` public enums, IR validation on deserialize.
- **Phase 2 — `opencodifier-schema`** (5952769): native / OpenAI /
  Anthropic / Jev adapters normalizing wire formats into the IR;
  `schema.*` error codes; byte-locked fixtures under `fixtures/`;
  round-trip property tests.
- **Phases 3–5 — `opencodifier-engine`** (5952769): DAG executor
  (topological waves, cycle-checked, bounded), deterministic rule
  engine, metadata + hand-written BM25 candidate narrowing, exact
  decision cache keyed by `CacheKeyBuilder` (SHA-256, D6) with
  model/calibration/graph versions folded in, calibration trait +
  identity calibration (D15), criterion benches for the D9 per-stage
  budgets, sync `Clock`/`Deadline`/`CancellationToken` (D5).
- **Phase 6 — `opencodifier-runtime`** (5146c34): `InferenceBackend` /
  `EmbeddingBackend` traits (sync, object-safe, logits-only per D7),
  validating `DenseTensor`, deterministic mock backends. The `onnx`
  feature was **deferred by a measured gate** (D2): ort rc.13 requires
  ONNX Runtime 1.27.x (unavailable here), Rust-vs-ORT softmax differs
  by up to 5 ULP, latency passes with ~100x headroom. Re-entry
  conditions recorded in D2.
- **Phase 7 — `opencodifier-model`** (d3a5aa5): candidate-conditioned
  serving contract (named tensors `context`/`candidates`/`logits`,
  shape-validated, Rust-side softmax), `EmbeddingClassifier` turning
  any `EmbeddingBackend` into an engine `Classifier`, SHA-256
  `ModelManifest` + artifact verification (D14). No trained weights
  ship in V1; the engine is fully useful without them.
- Coverage ledger (2026-09-24): 7,604 instrumented lines, 73
  documented-unreachable, **99.04% line coverage** by lcov ground
  truth; every uncovered line carries an in-source justification.

[Unreleased]: https://github.com/aliasfoxkde/OpenCodifier/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/aliasfoxkde/OpenCodifier/releases/tag/v0.1.0
