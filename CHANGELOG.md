# Changelog

All notable changes to OpenCodifier are documented here. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the
project does not yet guarantee a stable public API — version `0.x`
releases may break, and every breaking change is recorded in this file
(and reflected in `docs/DECISIONS.md` where a binding decision moves).

## [Unreleased]

### Added

- Hosted API tier design (`docs/HOSTED_TIER.md`): VPS phases,
  edge-owned auth/tenancy, measured cost anchors, local/product
  separation invariants. Design only.
- Amortyx integration design (`docs/INTEGRATION_AMORTYX.md`, PLANNING
  §60–§62): router decisions as typed IR, ladder/latency lane mapping,
  loopback `/v1/decide` contract, shadow mode on Amortyx's holdout, the
  VIVERE corpus path, and versioned training datasets. Design only.
- Focused-question extraction (§45, PLAN Phase 16, D18): with a
  `FocusPolicy` configured (`EngineConfig::with_focus`, CLI
  `--focus-budget`), questions on state longer than the token budget are
  decided on a deterministic BM25 view of the evidence-bearing sentences,
  with reverse escalation to the full state when the focused decision is
  weak and a blind-decline rule that never gambles on an empty view.
  Focus policies fold into cache identity (`focused-v1@<budget>`);
  unfocused runs have no focus surface anywhere in the trace or report
  projections. Long-suite A/B: answer-identical to full-state decisions
  on 120/120 items with views at a p50 of 93 tokens.

## [0.2.0] — 2026-09-28

Four plan phases in one minor bump: the decision-model benchmark and
D16 board (Phase 13), confidence truthfulness (Phase 14), the
relational solver that became the engine's default zero-ML stack
(Phase 15), and the MCP surface (Phase 10).

### Added

- Decision-model benchmark (PLAN Phase 13) — built and swept to 48
  runs: `benchmarks/decision-model/` holds a byte-locked 120-item
  candidate-conditioned suite (metadata_match / lexical_semantic /
  relational_compositional) and measured arm types (the engine's
  lexical pipeline, embedding zero-shot over three backends, Laya-421M,
  llama.cpp `parallel-decision` constrained scoring across the GGUF
  board, and chat-only screens for forks without a decision arm), with
  the committed summary + model manifest. Measured and recorded: the
  2-bit cliff (0.617 at 4B), the 9B IQ-quant latency reversal, the
  ≤350M tiny-decoder graveyard, and Jev-Style-0.8B-Decision-v3 as an
  interface-mismatch row (0.217 — verdict-slot readout, zero transfer
  to candidate-id path scoring; labeled in the summary). Extension
  arms (2026-09-28): Bonsai-4B ternary is the board's best
  accuracy-per-byte (0.650 from 546 MiB) at 4B-class latency;
  Ternary-Bonsai-8B is unmeasurable on the CPU host (>10 CPU-hours per
  decision request / unreadable tensors); LFM2.5-2.6B (0.608) is
  dominated by Qwen3.5-2B; the DavidAU X12 NEO MAX merge (0.667 @
  8.35 s) is latency-toxic. Ternary second pass (2026-09-28): Bonsai-8B
  Q1_0 hits a measured prefill cliff (≈1.5 s/token past a ~30-token
  knee — suite projects to 9–13 h) and Ternary-Bonsai-2-27B PTQ1_0 is
  unloadable (ggml type 143, prism-ml-fork-only), so on-CPU ternary
  viability in the ad129b0 build is 4B-and-below. Embedding-rung
  bake-off (2026-09-28): gte ONNX-fp32 ties torch to the digit at 4.2×
  the CPU speed (811 ms vs 3375 ms per item) and becomes the rung's
  runtime; int8 dynamic quantization rejected (0.442 for −17%
  latency); EmbeddingGemma-300M Q8_0 via llama.cpp recorded (0.500 /
  ECE 0.256); fp16 export unloadable (REPORT F20); blockwise int4 (the
  Q4_0 analog via ORT MatMulNBits, block 32) measured quality-free AND
  speed-free — 0.575 / ECE 0.330 identical to fp32 at 226 MB with no
  latency gain, so fp32 stays the rung's runtime and the q4 build is
  the RAM-bound fallback (REPORT F21; q4f16 untestable on CPU). Raw
  winner probability measured uncalibrated everywhere (ECE
  0.048–0.626), so D15 calibration still blocks any confidence
  exposure, and the relational class stayed escalation territory for
  every model arm until the relational solver (below). Runner
  hardening: per-request `--timeout` (9B tails exceeded the old fixed
  600 s and killed arms mid-run), chat-only screens included in the
  summary, and per-run caveat annotations. Full narrative record
  committed as `results/REPORT.md` + `results/charts/` (four
  deterministic SVG charts rendered from the run JSONs by
  `runner/plot.py`) — methodology, complete board, quant ladders,
  findings F1–F17, threats to validity, and reproduction commands.
- Confidence truthfulness (PLAN Phase 14): the §73 calibration mandate
  and the dead OOD channel are both closed. The engine gains a
  `Calibration` seam — identity (raw, version 0) by default,
  `TemperatureCalibration` from validated versioned artifacts whose
  `calibration_version` folds into cache keys — plus §19 uncertainty
  gates on `DecisionPolicy` (entropy ceiling / minimum margin / OOD
  ceiling, disabled by default, byte-stable canonical form) fed by a
  live distributional OOD score (`entropy / log2(k)`) traced alongside
  the calibrated confidence. Fitted temperature artifacts for the six
  D16 tier / rung arms ship under
  `benchmarks/decision-model/results/calibration/` (offline fitter
  `runner/fit_calibration.py`; evidence in `results/CALIBRATION.md`):
  LLM tiers are overconfident (T 0.84–1.67, frontier MiMo-9B ECE 0.048
  → 0.026), and the embedding rung's fit is degenerate — no finite
  temperature calibrates gte zero-shot scores, which are ordering
  evidence only, so no artifact is shipped for it.
- Relational solver (PLAN Phase 15): state text is mined for
  relational facts with an exact grammar ("X depends on Y", "X is
  healthy", "X: healthy, degraded, down", "X comes back online only
  after Y"); three general operators (root cause by transitive
  dependency closure, healthiest group by strict count, first restored
  by ordering) prove an answer over the whole extracted structure —
  and answer only when the proof is unique, inside the candidate set,
  and agreed by every operator, delegating to the lexical classifier
  otherwise. Measured on the byte-locked benchmark suite:
  relational_compositional 0.350 → 0.950, blended 0.483 → 0.683, p50
  5.3 ms → 1.3 ms, other classes bit-identical, determinism replay
  exact — the 0.50 relational ceiling that held for every model arm is
  broken at zero cost by proof rather than likelihood (REPORT F22).
  Calibration of record for the engine arm stays identity — the global
  temperature fit worsens ECE on the bimodal proof/delegate
  distribution, and the fitter now refuses to ship any artifact whose
  fit does not improve ECE (the retired `builtin-lexical-v1` artifact
  was removed with the stack it described).
- MCP surface (PLAN Phase 10): `opencodifier-mcp` — six decision tools
  (`codify_decide`, `codify_batch`, `codify_graph`, `codify_validate`,
  `codify_verify`, `codify_explain`) served over stdio with rmcp 2.2.0
  (D1's pin, honored; the 3.x major line is not adopted — D17), wired
  as `opencodifier mcp serve`. Same engine as HTTP (`EngineHandle`),
  same error envelope and stable codes (one MCP-specific code:
  `mcp.batch_too_large`), same deterministic traces with no
  chain-of-thought; batch items are decided independently under a
  16-request cap.

### Changed

- The engine's default zero-ML stack is the relational solver over the
  lexical classifier (Phase 15); D16 amended ×4 into a four-tier
  scheme: MiMo-V2.6-9B Q3_K_S is the frontier (0.817, ECE 0.048 best,
  first arm over the old 0.50 relational ceiling at 0.525 — MoE,
  14.3 s p50, verifier tier); Qwen3.5-4B is the interactive reference
  (0.800 @ 4.9–6.8 s, superseding Qwen3.8-4B-Distill); Qwen3.5-2B
  stays balanced (0.725, best ECE 0.062) and Qwen3.5-0.8B fast
  (0.650 @ 613 ms — now undercut by the engine floor).
- Cache-key honesty (D6): the engine now derives the identity's model
  id from the live classifier, so wrapper ids like
  `relational-v1|builtin-lexical-v1` invalidate cached decisions on
  swap without caller bookkeeping. Two shared-surface folds landed
  with the MCP work: the engine's graph-document shim (`GraphDocument`)
  is now public and the HTTP surface's private mirror was deleted, and
  the execution-report JSON projection moved into the engine
  (`opencodifier_engine::report`) so CLI `--trace` and MCP
  `codify_explain` cannot drift.
- Aegis baseline regenerated at the release commit (2,370 entries;
  line-shift re-flags after the CI-clippy allow fix in c09e39d turned
  24 already-triaged findings into false "new" ones on pipeline
  aaa2eefa).

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
