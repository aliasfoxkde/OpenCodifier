# Changelog

All notable changes to OpenCodifier are documented here. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the
project does not yet guarantee a stable public API — version `0.x`
releases may break, and every breaking change is recorded in this file
(and reflected in `docs/DECISIONS.md` where a binding decision moves).

## [Unreleased]

Decision-model feasibility benchmark (PLAN Phase 13, in progress):
candidate-conditioned question suite + runner harness under
`benchmarks/decision-model/`, measuring constrained-decode accuracy,
latency, and determinism for small local models against the engine's
own lexical/BM25 baseline and an embedding zero-shot arm.

## [0.1.1] — 2026-09-26

### Added

- `infrastructure/docker/ci-rust.Dockerfile` + `just ci-image` — the
  runner-local CI image (`opencodifier-ci-rust:1`) that bakes the
  pinned toolchain, rustfmt/clippy, cargo-deny, cargo-audit, aegis
  (rev-pinned), and a warm crate registry (PLAN Phase 12).

### Changed

- `.gitforge.yml` — all four jobs run on the baked CI image with zero
  runtime tool installs; job containers cannot reach host services
  (host firewall default-DROPs docker-sourced traffic), and
  `rust-toolchain.toml` had been silently overriding the image
  toolchain at job runtime (measured: latest stable downloaded
  in-job). `RUSTUP_TOOLCHAIN` in the image pins every job to the
  baked compiler (PLAN Phase 12).
- Pipeline of record validated green through GitForge: runs `fe4b0871`
  and `69d17ef2` (lint, test, doc, supply-chain all succeeded).
- Aegis baseline discipline: the gate runs after every content edit
  (run `f1153de0` failed the supply-chain lane on untriaged doc-edit
  findings); triaged appends recorded per entry (1,709 → 1,735).

## [0.1.0] — 2026-09-25

Phased build-out per `docs/PLAN.md`; hashes refer to the repository
history.

### Added

- `recipes/` — three runnable decision graphs with paired requests and
  byte-reproducible captured responses (`minimal-choice`,
  `strict-verify`, `boolean-score`), each demonstrating a different
  facet of the escalation ladder; `recipes/README.md` documents the
  capture-and-compare procedure (ea19aed; folded from Unreleased —
  this item shipped in 0.1.0 but was left unstaged there).
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
- **Phases 8–9 — interfaces** (563ad3f): `opencodifier-engine::EngineHandle`
  (the shared facade: `lexical` zero-ML constructor, `decide`,
  `decide_with_report`, `validate_graph`, `health`);
  `opencodifier-http` (axum **0.8** — the original D12 pin said "axum
  0.9", which does not exist; amended — `/v1` decide/graph-validate/
  healthz, owning-crate error codes, 1 MiB body cap, loopback-only
  bind with `allow_remote` opt-in, `spawn_blocking` off async workers,
  abstention is 200); `opencodifier-cli` (D13: decide / graph validate
  / serve / models verify; exit codes 0 accept / 1 input / 2
  policy-gate escalation / 3 internal, with clap's exit-2 normalized
  to 1). Real-socket e2e over both surfaces.
- Coverage ledger (2026-09-24): 7,604 instrumented lines, 73
  documented-unreachable, **99.04% line coverage** by lcov ground
  truth; every uncovered line carries an in-source justification.
  Updated 2026-09-25 after Phases 8–9: 8,340 lines, 83
  documented-unreachable, **99.00%**.

### Changed

- `opencodifier-http::serve_with_shutdown` — the graceful-shutdown
  future is now a parameter (the `serve` entry point keeps `Ctrl-C`),
  so embedders and tests can end the serve loop with their own signal
  (563ad3f; folded from Unreleased — this item shipped in 0.1.0 but
  was left unstaged there).

[Unreleased]: https://github.com/aliasfoxkde/OpenCodifier/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/aliasfoxkde/OpenCodifier/releases/tag/v0.1.1
[0.1.0]: https://github.com/aliasfoxkde/OpenCodifier/releases/tag/v0.1.0
