# Changelog

All notable changes to OpenCodifier are documented here. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the
project does not yet guarantee a stable public API — version `0.x`
releases may break, and every breaking change is recorded in this file
(and reflected in `docs/DECISIONS.md` where a binding decision moves).

## [Unreleased]

- B6 — cross-rung escalation (D27): a configured, ordered `Rung` list
  (`classifier` + optional per-rung calibration and gate policy) the
  threshold node walks when a rung's gate does not accept — `Verify` and
  `Abstain` both mean "this rung could not decide". The gate is the only
  trigger: an accepted question runs exactly one classifier; the walk
  stops at the first accepting rung or the end of the list; the existing
  verifier cascade reads the final rung's distribution; each firing
  passes the deadline/cancellation guard. Cache identity carries the list
  (`|rungs-v1@<model>@<cal-ver>+…`, in order) so composition changes
  re-key mechanically; traces gain `rungs_fired` + `rung_chain` naming
  each rung's model, answer, and gate outcome. Empty list = byte-identical
  engine (12 new integration tests).
- B5 — the model rung of the escalation ladder (`LlamaDecisionClassifier`,
  D26): the measured `POST /v1/decision` tree-mode contract of the
  llama.cpp `parallel-decision` fork as an `opencodifier_engine::Classifier`
  in `opencodifier-model`. The classifier itself is transport-agnostic and
  always compiled; the loopback HTTP transport (`UreqTransport`, ureq,
  plain HTTP — the endpoint is loopback by the bind rule) sits behind the
  `llamacpp` feature. Honesty rules enforced in type: the fork's full
  `distribution` object is **required** (a winner-only response is a typed
  refusal, never a synthesized probability), entries are read in the
  question's declared answer order, refused unless they cover exactly the
  declared answer set, and renormalized in Rust f64 before
  `Distribution::from_pairs`. Errors carry the HTTP status in the message
  (the engine keeps only the reason string). 15 tests including an
  end-to-end escalation: default policy accepts a confident distribution,
  a 0.95 per-kind `LadderPolicy` sends it to `Verify` with the
  `policy_source` trace fact naming the model rung.

## [0.4.0] — 2026-10-02

The engine-enhancement track (B1–B4) plus the research record: D9's
performance budgets are finally measured, the hot paths they exposed
are fixed, and the escalation ladder ships as configuration.

### Added

- D9 criterion benches + the first committed baseline bundle (PLAN
  18i, B1): benches for the unmeasured per-stage budgets — normalize
  decode, rule match at scale, BM25 narrowing at 256 candidates
  (build + score-all), embedding rerank — plus an HTTP bench for the
  2 ms round-trip budget, a `just bench` recipe, and
  `benchmarks/baselines/criterion` (d9-baseline). Measured at Class L
  (load < 30): `bm25_index_build_256` (3332 µs) and
  `bm25_score_all_256` (1164 µs) run over their 1 ms budgets — the
  measured justification for the B3 build-once work; every other row
  is in budget (cache hit 25 µs vs 2232 µs full pipeline).
- Escalation-ladder wiring (PLANNING.md §24, D25):
  `opencodifier_engine::LadderPolicy` — optional per-node then
  per-kind `DecisionPolicy` overrides resolved at the confidence gate
  the deciding node already uses, on `EngineConfig::with_ladder` /
  `EngineHandle::with_ladder`. An empty ladder is byte-identical to no
  ladder (no decoration, no trace facts, unchanged fixtures); a
  configured ladder decorates the model id (`|ladder-v1@<id>`) so every
  cache key re-keys — the cache stores completed gated responses, so
  identity decoration is what keeps a changed ladder from replaying an
  old gate — and a non-empty ladder without an id is refused at
  assembly (`EngineError::InvalidConfig`). Traces name the rung whose
  gate fired (`policy_source`).
- Per-rung calibration and shipped ladder profiles (B4): a rung may
  carry its own `Calibration` — key-set and precedence identical to
  the policy overrides, `None` byte-identical, a
  `calibration_source` fact in the trace — plus
  `ProofAwareCalibration`, which passes single-entry distributions
  (exact proofs, p = 1.0) through raw and delegates the hedging tail
  (its `version()` is the inner version; the ladder id re-keys).
  Profiles are data: `LadderProfile` (`deny_unknown_fields`) parses
  a JSON document into a validated, fitted ladder via
  `into_ladder()`, refusing empty profiles and artifacts that fail
  `TemperatureCalibration::from_artifact` (the error names the
  rung). One `runtime::engine_config()` loads the `--ladder` flag
  for CLI decide, serve, and mcp serve, so the interfaces cannot
  drift. Shipped under `ladders/`: `fusion-v1` (the fusion-study
  ladder: proofs gate themselves, classifier kinds accept on margin
  alone) and `proofs-only-v1` (accept exact proofs only).
- Semantic graph nodes and the D22 optimizer (PLAN 18h, D21/D22):
  `embedding` (annotate-only), `retrieve` (`top_n`/`floor`,
  floor-never-starves, every drop trace-named), and `rerank` (permutes,
  never removes; `LexicalReranker`/`EmbeddingReranker`). Engines without
  an embedding backend refuse such graphs at assembly
  (`engine.missing_backend`); safe mode refuses `retrieve`; the
  embedding backend rides engine identity and every cache key.
  `opencodifier_engine::optimize` = dead-node elimination + pure-node
  CSE (cache/threshold/output excluded), proven answer-identical on all
  15 committed graphs.
- WASM runtime (PLAN 18j, §57/§68, D23): `crates/opencodifier-wasm`
  compiles the zero-ML stack for `wasm32-unknown-unknown`; `WasmEngine`
  (decide / validate_graph / run_graph / identity) answers native-schema
  JSON with typed error codes, proven in Node over the real artifact
  (`just check-wasm`, not part of `just ci`). The model rung stays
  native (D23).
- `just check-wasm`: host tests, the wasm32 compile, a
  `wasm-pack --target nodejs` build, and the Node smoke test.
- Coverage close-out tests (PLAN 18): twelve tests across six crates
  closing every reachable coverage gap — wasm public-boundary success
  paths, registry decode refusals and score instantiation, graph
  knob/kind validation, embedding-backend failure and miscount
  surfacing through node and reranker seams, filtered-survivor
  semantic scoring, HTTP malformed batch/graph-run bodies, recipe
  install failure arms. Lines 98.97 % on the lcov `DA` basis; the 125
  remaining unexecuted lines are each classified with an enforcing
  fact in `docs/COVERAGE.md`.
- Static-embedding survey arm: VTX-JEV-3 (`VTXAI/VTX-JEV-3`,
  Model2Vec-class 255,753×256 2-bit table with a position-gated
  attention pooler, Apache-2.0) on both boards — `runner/run_vtx.py`
  (suite: LF2 0.242 / FP32 0.317, the 7.5 pp quantization cost
  measured) and `runner/run_jevbench.py --arm vtx` (public 231:
  0.4113 accuracy, 0.4318 family-macro, ECE 0.126, 7.9 ms p50,
  deterministic, native probabilities only) through the vendor
  `inference.py` `JevClient`, whose pooler plain Model2Vec loading
  does not reproduce. The vendor sub-millisecond latency claim
  reproduces; the accuracy sits at the zero-ML engine's level, not the
  Jev-class field's — not a new tier (D16 unchanged).
- `docs/BENCHMARKS.md`: the complete measured comparison in one page —
  both boards (JevBench public 231 and the locked 120-item suite), the
  full 53-run model board, the D16 tiers, and every external row with
  provenance: the official TypeSafe Jev 1.13 hosted-API numbers, the
  autotrust/JEV-27B six-benchmark table (their runs, target bar 84.07
  mean), the JevBench authors' board, and the
  `SargeDev/jev-distill-corpus(-v3)` training-corpus landscape.
  Written website-liftable.
- Fusion study (`runner/fusion_study.py` + `results/fusion-*.md`): the
  confidence-gated ladder simulated post-hoc over measured per-item arm
  rows — suite blend engine → gte-ONNX → Qwen3.5-2B scores **0.867 @
  753 ms** vs 0.725 best single (F23), while the same gate on JevBench
  falls below the bridge alone (0.632 vs 0.6494) because the engine's
  out-of-domain ECE is 0.798: gates inherit their rung's calibration
  (F24).
- Resource accounting in every runner (`runner/resources.py`): `/proc`-
  based monitor recording peak RSS (`VmHWM`), CPU-seconds, IO bytes and
  wall time per arm into run JSONs (`resources`) and summary.md's new
  peak-RSS column; prior runs show `—`.
- `docs/TRAINING.md`: adapter-training research record — the JEV-27B
  recipe class (frozen base + small trained decision block), LoRA
  provenance and serving economics, what a 0.8 B decision-head/LoRA run
  costs on this GPU-less host (1–2 CPU-days first run), the community
  checkpoint it must beat (Jev-Style-0.8B 0.6494), and a costed E0–E3
  experiment ladder.
- ONNX runtime decision arm (`runner/run_onnx.py`): the optimum
  `onnx-community/Qwen3.5-2B-ONNX` q4 export driven by onnxruntime with
  the fork's exact tree scoring ported to Python (greedy-parity
  validated at the probe stage; hybrid qwen3_5 state threaded manually,
  including the asymmetric `present.`→`past_key_values.` output naming).
  Same weights as the GGUF arm: acc **0.658** / ECE 0.064 vs 0.725,
  prefill-bound ≥9× slower, 2.6× the peak RSS (F25) — llama.cpp keeps
  the decision rung; ONNX stays the product's portability format.
- Native verdict-slot readout arm (`runner/run_jevbench.py --arm
  jev_native` + suite driver): Jev-Style-0.8B-Decision-v3 through its
  own `macjev` render/readout scores **0.8083** on the locked suite
  (F26) — the same weights that scored 0.217 tree / 0.000 chat; the
  F10 interface-mismatch row now has its native control, and the
  comparability bridge has a suite-side anchor (0.8083 home vs 0.6494
  JevBench, both with the authors' shipped temperatures).
- Rank/margin gate study (`runner/margin_gate_study.py` +
  `results/embed-margin-study.md`): the embedding rung's probability is
  uncalibratable (degenerate fit), but its top1−top2 margin gates
  cleanly — margin ≥ 0.0183 accepts 20.8 % of suite items at 0.880
  accuracy, monotone to 1.000 at 0.0283 (CALIBRATION finding 3 closed;
  live profile `min_confidence: 1.0` + `min_margin ≈ 0.018`).
- Exact d15 calibration refit: the parallel-decision fork now emits
  full per-candidate distributions, and the Qwen3.5-2B artifact of
  record refits from the full board (reverse-softmax temperature,
  T 0.8426 → 0.9317, ECE 0.0621 → 0.0588) — the margin proxy
  over-sharpened on multi-way boards, as its own synthetic validation
  predicted. `crates/opencodifier-engine/tests/artifact_schema_check.rs`
  pins every artifact of record to the engine's `CalibrationArtifact`
  schema.
- Vulkan device A/B (suite, Qwen3.5-2B Q4_K_M): the Vega 8 iGPU leg
  matches the CPU leg's accuracy (0.725 / ECE 0.063) at 2.23× the speed
  (p50 1.35 s vs 2.93 s) for 1/43rd the host CPU-seconds and 26 % less
  peak RSS — the model rung no longer assumes a CPU-only host
  (REPORT.md "Device A/B").
- `docs/RESEARCH.md`, the standing frontier record (B0): the ladder
  design validated against 2025 cascade literature (Gatekeeper loss,
  early abstention, rational cascades, per-class isotonic
  calibration); logprob-confidence and constrained-decoding findings
  for the single-token readout; the System-One board sweep; HRM's
  halt head read as B6-inside-the-model and the HRM-Text GGUF serving
  contracts (PrefixLM one-batch prefill, template-as-contract, expert
  routing never merged); prompting/template levers ranked with the
  measured diet A/B (prompt content is first-order). Includes the
  runtime correction — ONNX Runtime ships no Vulkan EP, so the model
  rung targets the llama.cpp fork, not ORT.
- D24 §58 distribution matrix closed: all five distribution targets
  verified at their named levels (overnight cross-build 4/4 at
  link+file level; no foreign-arch execution claimed), and the
  storm-recovery chain re-drove the measurement arms a load storm
  killed — t16 landed with every metric float-identical to t08
  (thread invariance; the latency row load-confounded, not of
  record). PLAN/SPEC_COVERAGE rows narrowed to the true remainder
  (windows link+smoke, registry publication on explicit user go).
- Vision capability probe (REPORT 18i): LFM2.5-VL-450M Q4_K_M +
  mmproj Q8_0 answers the rendered decision context identically to
  text (0.33 s vs 1.42 s, CPU) — a capability gate, not an accuracy
  row.

### Changed

- Platform seams on wasm32 (D23): `Clock`'s `Instant` is
  `std::time::Instant` natively and the `web-time` host-clock reading on
  wasm32 (`std::time::Instant::now()` traps there; the dependency is
  target-gated, the native tree is unchanged), and wave execution is
  sequential on wasm32 (`std::thread::spawn` traps there;
  `RunReport::parallel_waves` is 0 by construction). Native behavior
  is byte-identical.
- Constant-factor latency wins across the engine's hot paths (B2),
  every change byte-identical in output: the reranker's candidate
  index built by `enumerate()` instead of a linear `position()` scan
  per candidate (kills an O(n²)); narrowing's per-question id views
  through `HashSet` membership; the survive-filter's pruned sets
  built once per question and `surviving_ids` no longer cloning full
  candidates to throw the bodies away; the rerank-order sort on a
  prebuilt rank map with first-occurrence semantics; the embedding
  scorer's query norm hoisted out of the per-candidate cosine loop.
  `rules.rs` bucket membership inspected and declined — the buckets
  are tiny, conversion would be a pessimization.
- BM25 narrowing rebuilds once per question instead of rescanning
  (B3): per-document term-frequency maps built at index time (was
  O(documents × query terms × document length) per question), the
  query tokenized once per `score_all` sweep (was once per document),
  and `score()`/`score_all()` sharing one `score_terms` path summed
  in sorted-term order — scores bit-identical, verified by a
  contract test. Handing the classifier an already-built index was
  considered and rejected: the `Arc<dyn Classifier>` seam belongs to
  embedding/ONNX models, not BM25.
- Coverage-floor debt from the above landed with it (the CI coverage
  lane had dipped to 98.32 % against the 98.5 % floor): new tests
  pin the profile-artifact fit refusal naming its rung, the plain
  non-proof-aware rung calibration path, `LadderPolicy` equality
  semantics, and `EngineHandle::with_ladder` success plus
  `InvalidConfig` propagation. Floor re-measured at 98.55 %; the
  GitForge pipeline of record is green on the release commit (run
  `5635156b`).

### Fixed

- `yoke-derive` 0.8.3 → 0.8.4: the pinned version was yanked
  upstream mid-window, turning `cargo deny` advisories red on
  pipelines after the lockfile was last touched.
- `.aegis/baseline.json` restored to `--format json` after a
  regeneration had written aegis's default text report — the gate's
  JSON loader refused to parse it and every scan failed at baseline
  load. Regenerated in-tree from the committed tree.

### Deferred

- JevBench 4B fork arm rerun (fork4b-v2) and the diet100 ctx16k leg:
  in flight on the shared host at release time; results land in
  REPORT.md when done.
- Criterion before/after deltas for the B2/B3 changes: the committed
  d9-baseline was measured in a quiet window; the deltas need the
  same window class to be comparable.
- Windows link+smoke cross-build and registry publication: awaiting
  explicit operator go (D24 closure narrows these to the only §58
  items not verified).

## [0.3.0] — 2026-09-29

### Added

- Agent-facing skills (§33, `skills/`): the shared calling protocol
  plus all eight skill areas — routing, model-selection, tool-selection,
  tool-gating, context-pruning, escalation, verification,
  memory-selection — each grounded in the shipped `/v1/decide` shape.
- Spec coverage map (`docs/SPEC_COVERAGE.md`): every PLANNING section
  §§1–80 mapped to the implemented tree with an honest verdict and an
  ordered remaining-work list.
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
- Native verdict-slot arm (`benchmarks/decision-model/NATIVE_VERDICT_ARM.md`,
  `runner/run_jev_native.py`): Jev-Style-0.8B-Decision-v3 measured through
  its own trained interface (verdict-slot logits, shipped temperature
  artifact) instead of the D16 tree-mode mismatch rows (0.217/0.0);
  manifest-verified reference runtime, determinism replay, runner-side
  honesty rules.
- JevBench external-anchor methodology (`benchmarks/decision-model/JEVBENCH.md`,
  `runner/run_jevbench.py`): the 231-item public split through the
  authors' own harness (their runner, scoring, no-synthesized-probability
  rule), IR mapping, the 64.1 % comparability bridge, published anchor
  rows, and smoke-verified wire facts; PLAN.md Phase 17 opens the
  workstream.

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

[Unreleased]: https://github.com/aliasfoxkde/OpenCodifier/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/aliasfoxkde/OpenCodifier/releases/tag/v0.4.0
[0.3.0]: https://github.com/aliasfoxkde/OpenCodifier/releases/tag/v0.3.0
[0.2.0]: https://github.com/aliasfoxkde/OpenCodifier/releases/tag/v0.2.0
[0.1.1]: https://github.com/aliasfoxkde/OpenCodifier/releases/tag/v0.1.1
[0.1.0]: https://github.com/aliasfoxkde/OpenCodifier/releases/tag/v0.1.0
