# Decision Records

Binding architecture and dependency decisions, with the reasoning that
produced them. Each entry supersedes anything contradictory in
PLANNING.md. Ordered by decision number; new decisions append.

## D1 — MCP SDK: `rmcp = "=2.2"`, stable MCP `2025-11-25`

PLANNING.md §47 named rmcp loosely. `rmcp` 3.0.0-beta is in flight but
beta; the stable line is 2.2, which speaks the MCP `2025-11-25` stable
revision. Exact-pin (`=`) because rmcp 2.x → 3.x is a protocol-surface
change, not a semver-compatible bump. Revisit when 3.0 reaches stable.

## D2 — ONNX runtime: `ort = "=2.0.0-rc.13"` — DEFERRED by measured gate

`ort` rc.12 → rc.13 changed the API and rc.14 may again. fastembed pins
rc.12, which is why fastembed is **not** used (D4). Pin exactly, keep
behind the `onnx` feature, and bump only as a deliberate PR that re-runs
the model round-trip tests. `default-features = false`, plus the
`load-dynamic` feature packaged as the `runtime-onnx-dynamic` feature so
distros can point at a system ONNX runtime.

**Feasibility-gate result (measured 2026-09-24, scratch probe
`/nas/Temp/tmp/oc-ort-probe`): the `onnx` feature does NOT ship in
V0.1. The dependency is not added; the deterministic path is the only
path.** Per PLAN.md Phase 6 the gate had three legs:

- **(a) reference model loads with api-27 — FAIL.** ort rc.13 defaults
  to `api-27`, which demands ONNX Runtime 1.27.x. Nothing on the
  reference machine provides it (system dylib: 1.21.0 / API 21;
  Python-wheel copies: 1.22.1, 1.24.1, 1.24.2 / API 22-24). Loading at
  the default API level fails with a clean typed error for every
  available dylib. It would pass only compiled down to `api-24` (needs
  a 1.24.x dylib) or `api-21` (all four load). Numerics are identical
  across API levels (measured).
- **(b) bit-identical logits round-trip vs Rust softmax — FAIL.** Over
  10,000 seeded random logit rows plus extremes: ONNX `Softmax` op,
  ONNX Div-chain, and Rust f32 softmax agree pairwise within **max 5
  ULP (~2.4e-7 abs, ~3.3e-7 rel)** but no pair is bit-identical —
  including the two ONNX formulations with each other. ORT additionally
  flushes subnormal probabilities to 0 below ~1e-38 where Rust keeps
  them. Run-to-run ORT output **is** deterministic (0/640,000 values
  differed across 5 repeats) — the requirement that fails is
  *cross-implementation* bit-identity. Consequence for later: cache
  keys already avoid this (cached responses are stored, not recomputed
  in a second implementation), but verifier agreement and calibration
  comparisons must carry a 5-ULP tolerance, never `==` on f32.
- **(c) p99 single-decision latency ≤ budget — PASS.** Single-row
  [1,8] p99: 30.8-61.0 us; batch [64,8] p99: 39.7-60.1 us across all
  four dylibs — two orders of magnitude inside the 5 ms budget (D9).

**Re-entry conditions for the `onnx` feature:** compile at `api-21` or
`api-24` (not the default `api-27`); require an explicit
`ORT_DYLIB_PATH` (no unversioned `libonnxruntime.so` resolves on this
platform, and ort panics via dlopen without one); document the
user-supplied dylib requirement; and carry the 5-ULP tolerance rule
into verifier/calibration comparisons. Also: `Session::run` takes
`&mut self` at rc.13, so a concurrent server needs a session pool or
mutex.

## D3 — MSRV: 1.90, stable channel

Edition 2024. Every dependency must support 1.90 (checked: criterion
0.8 needs 1.86, burn 0.20 needs 1.89 — both fine). `rust-toolchain.toml`
pins `stable` locally; CI images use `rust:1.90`. MSRV bumps are
release-note items.

## D4 — No fastembed, no tantivy: hand-written BM25 + adapter-supplied
embeddings

- fastembed → hard-pinned to ort rc.12, incompatible with D2, and drags
  hf-hub/model download into a privacy-first local crate. Rejected.
- tantivy → full search engine (index files, concurrency, git dep for a
  fix) for what is an in-memory top-k over ≤256 candidates. Rejected.
- Instead: ~200 lines of hand-written BM25 (`opencodifier-engine`),
  property-tested against hand-computed IDF/tf values, and an
  `EmbeddingBackend` trait so embedding models are supplied by the
  runtime crate — nothing downloads at runtime, ever.

## D5 — Synchronous core, async at the edges

No Tokio (or any async runtime) in core/engine/schema/runtime. The
engine is a sync library; `Clock`, `Deadline`, and
`CancellationToken` are traits implemented by the caller. Interface
crates (axum, rmcp, wasm-bindgen) adapt. Rationale: deterministic
tests, no runtime contagion, trivial embedding in non-Tokio hosts.

## D6 — Single normative cache-key site

`CacheKeyBuilder` in `opencodifier-engine` is the only code that
computes a decision-cache key: SHA-256 (sha2 0.10) over length-prefixed
fields — canonical request with sorted candidates, graph version, model
id, calibration version, engine semver. Other crates reference
`CacheKey`, never re-derive it.

## D7 — Float policy: decision math in Rust, f64

All decision math is Rust-side f64 (softmax, calibration, margins,
entropy, score means). ONNX graphs produce logits only. Guarantees the
same decision on any backend and keeps quantization out of the
decision path. `float_cmp` lint is denied outside test modules.

## D8 — Trace contract frozen now

`trace_version = 1` on every trace, `#[non_exhaustive]` on public
enums, stable `code()` strings (`ir.*` prefix in core). Applied in
Phase 1 — before any consumer existed — so no break can ever be
"grandfathered".

## D9 — Benchmarks: criterion from Phase 3, per-stage budgets replace §70

PLANNING.md §70's single whole-pipeline budget was too coarse to act
on. Replaced by per-stage budgets (below), measured with criterion
(harness = false) committed under `benchmarks/` per release. Regression
= p99 over budget on the reference machine class (4-core laptop).

| Stage | Budget (p99) |
|-------|--------------|
| Normalize (schema → IR) | 1 ms |
| Rule match + cache hit path | 100 µs |
| BM25 narrowing, 256 candidates | 1 ms |
| Embedding scoring (supplied backend) | backend-declared |
| Deterministic decision path end-to-end | 10 ms |
| HTTP overhead added by `opencodifier-http` | 2 ms |

## D10 — Property testing: proptest + round-trip fixtures

Invariants that must never break, encoded as proptest cases: adapter
round-trips (native ⇄ wire), distribution normalization under
arbitrary probability vectors, cache-key stability under map
re-ordering, candidate narrowing subset-safety. Fixtures under
`fixtures/` lock every wire format byte-for-byte.

## D11 — Feature-flag matrix (heavyweight deps confined to one crate)

| Feature | Crate | Pulls in |
|---------|-------|----------|
| *(default)* | core, engine, schema | serde, thiserror, sha2 — nothing heavy |
| `onnx` | opencodifier-runtime | ort rc.13 (D2) |
| `runtime-onnx-dynamic` | opencodifier-runtime | ort `load-dynamic` |
| `http` | opencodifier-http | axum 0.9, tokio |
| `mcp` | opencodifier-mcp | rmcp 2.2 (D1) |
| `cli` | opencodifier-cli | clap v4 |
| `wasm` | opencodifier-wasm | wasm-bindgen, getrandom js |

No crate may enable another crate's heavyweight feature transitively;
the default build of the whole workspace stays ML-free and runtime-free.

## D12 — HTTP surface: axum 0.8, `/v1` prefix

Endpoints: `POST /v1/decide`, `POST /v1/graph/validate`,
`GET /v1/healthz`. Errors map `code()` → stable strings in the body
(`{"error": {"code", "message"}}`), HTTP status by error class (4xx
input, 5xx internal). Binding default `127.0.0.1:8177`; non-loopback
binds require `--allow-remote` (documented as a real risk in
SECURITY.md).

*Amended 2026-09-24: the original pin said "axum 0.9" — that version
does not exist (latest published is 0.8.9); the pin was aspirational
and never checked, the same failure mode as the ONNX api pin in D2.
Corrected to the measured reality: **axum 0.8**.*

## D13 — CLI: clap v4 with an exit-code table

`decide`, `graph validate`, `serve`, `models verify` subcommands. Exit
codes: 0 success/abstain-with-flag, 1 input error, 2 policy gate
escalation, 3 internal error — documented in `--help` and the book so
scripts can branch on them.

*Implemented notes (2026-09-25): exit 2 covers **every non-decisive
outcome** (`abstain`, `escalate`, `verify`, `no_valid_candidate`) —
`is_decisive()` is the only honest split the `DecisionOutcome` enum
supports. clap's own default exit 2 for bad arguments is normalized to
1 so scripts can trust the table. `serve --policy` validates the file
then reports `cli.policy_inapplicable`: policy lives on the request in
this IR, and a pretended server-wide override would be a lie.*

## D14 — Model integrity: SHA-256 manifests, never commit weights

`models/*.json` manifests record {file, sha256, bytes, source, license};
the runtime refuses a mismatch before any tensor is read. `models/`
weights are gitignored; manifests are committed.

## D15 — Calibration storage

Per-question-class calibration parameters (isotonic or Platt) are
versioned artifacts alongside model manifests, referenced by
`calibration_version` in the cache key (D6). Un-calibrated runs report
`calibrated_confidence = raw` with `calibration_version = "none"`, so
the cache never conflates calibrated and uncalibrated results.

## D16 — Decision-model picks: tier scheme (measured, Phase 13; amended ×3)

**Decision (amended 2026-09-27, third pass — full-sweep frontier):** the
tier scheme gains a frontier tier and the interactive reference moves:

- **Frontier / verifier tier: MiMo-V2.6-Distill-Qwen-9B Q3_K_S** (sha256
  in the manifest) — **0.817 overall (1.00 / 0.93 / 0.525), ECE 0.048 —
  best calibrated measured, and the first arm over the 0.50 relational
  ceiling.** p50 14.3 s, 4.26 GB, bit-deterministic. MiMo-V2.6 is MoE
  (its RL trainer's own notes say so), which is why 9B runs at ~2× 4B
  latency, not dense-9B ~2.3×. Slot: verifier stage and bulk/offline
  decisions where 14 s/decision fits the D9 budget — never the
  interactive path. Q3_K_M ties its accuracy (0.817) but loses
  calibration (0.081) and latency (18.4 s); IQ3_XXS trades to 0.783 at
  10.6 s.
- **Reference (interactive): Qwen3.5-4B Q3_K_S** — 0.800 (1.00 / 0.95 /
  0.45), ECE 0.069, p50 6.8 s; **UD-Q4_K_XL is the fastest 0.800**
  (4.9 s, ECE 0.074). This supersedes Qwen3.8-4B-Distill (0.767) on
  accuracy at the same latency class. Every ≥3-bit 4B quant measured is
  ≥ 0.775; the **2-bit cliff is 0.617** (UD-IQ2_XXS; 0.383–0.450 at 2B)
  — quants at or below 2 bits are never shippable for decisions.
- **Balanced: Qwen3.5-2B** unchanged (0.725, best 2B-class ECE 0.062,
  relational 0.50, p50 1.7 s); its UD-Q6_K_XL variant is the
  accuracy-lead option (0.758, ECE 0.079, +25% latency) when a decision
  point needs it.
- **Fast: Qwen3.5-0.8B** unchanged (0.650 @ 613 ms, ECE 0.074).
- **Measured out:** every ≤350M decoder (Granite-350M 0.342, Falcon-90M
  0.275/0.267, Gemma-270M 0.200, glm5.1-distill 0.250 — all below the
  5.3 ms engine layer they would need to justify); gte-modernbert-base
  zero-shot (0.575, ties the engine blend, best relational 0.50 at
  3.4 s — but ECE 0.330, unusable as a gate before D15 calibration) and
  Laya-421M (0.475) stay embedding-rung references, not decision arms.
- **Interface-mismatch row (not a quality rejection):**
  Jev-Style-0.8B-Decision-v3 scores 0.217 / ECE 0.408 / chat 0.0 through
  this harness because its trained readout is per-option verdict slots
  (`h·(w_yes − w_no)` at each option's `->` position, shipped group
  temperatures) — candidate-id token paths and JSON answers are outside
  its trained interface, and zero transfer was measured. A native
  verdict-slot readout is the #25 arm type. (The surrounding jev-style
  project remains the closest external analog to this codebase: a
  decision-gated PreToolUse guard, artifact-embedded group temperatures
  with in-artifact fit quality, automation-at-error-budget eval —
  reference designs for #40 and D15/#34.)
- **Latency findings:** importance-matrix quants are not uniformly
  slower — at 9B, IQ3_XXS (10.6 s) beats Q3_K_S (14.3 s), inverting the
  4B pattern (IQ3_XXS 15.7 s vs K_S 6.8 s there). K2-Horizon (fork
  without `/v1/decision`) screens chat-only: 7B 0.800 / 4B 0.700 /
  1B 0.725 at 9.1 s sampled p50 — labeled non-comparable in the summary.

**Second pass (2026-09-26 — Qwen3.8 distills):**
**Qwen3.8-4B-Distill** (empero-ai, Q4_K_M GGUF, sha256 `dec96e8c…`, see
`benchmarks/decision-model/results/models.manifest.json`) is the reference
model for the candidate-conditioned decision layer: **0.767 overall
(1.00 metadata — perfect / 0.95 lexical-semantic — new crown / 0.35
relational), ECE 0.057 — best calibrated measured**, p50 3757 ms, under
the identical constrained-scoring conditions as every other arm
(thecodacus/llama.cpp `parallel-decision` `ad129b0`, `POST /v1/decision`
tree mode, CPU-only host). It strictly supersedes Gemma-3-4b-it (0.750,
ECE 0.236 — worse on both axes; Gemma is now fully superseded, kept only
as an arm in the record).

The pick is a **tier scheme**, because the leaders trade off:

- **Reference: Qwen3.8-4B-Distill** — accuracy + calibration leader. This
  is the model D15 calibration is fit against first and the verifier tier
  uses.
- **Balanced alternative: Qwen3.5-2B** (0.725, ECE 0.062, p50 1735 ms) —
  holds the relational crown (**0.50** vs the 4B distill's 0.35; its one
  weak class), half the latency. Preferred where the residual stream is
  relational-heavy or the latency budget bites.
- **Fast tier: Qwen3.5-0.8B** (0.650 @ 613 ms, ECE 0.074) — unchanged;
  neither new 0.8B distill comes close (0.567).

Rejected on measurement: Qwen3.8-2B-Distill (0.617 @ 1344 ms — below
Qwen3.5-2B everywhere that matters), Qwen3.8-0.8B (0.567 @ 808 ms bf16 —
dominated by Qwen3.5-0.8B on accuracy, latency, and calibration),
MiniCPM5-1B, Llama-3.2-1B, Qwen2.5-0.5B (at or below the engine baseline).
Provenance findings folded into the record: (a) both community
"Qwen3.8-0.8B" repos ship **bit-identical safetensors** (sha
`a84cd623…`) — one model, two packages, caught by manifest discipline
(D14); (b) their uploads omit the MTP block declared in config — GGUF
conversion requires llama.cpp's `--no-mtp`.

*Amendment history:* 2026-09-27 third pass — tier scheme gains the
MiMo-9B Q3_K_S frontier (first model over the relational ceiling), the
interactive reference moves from Qwen3.8-4B-Distill to Qwen3.5-4B
(K-quant or UD-Q4_K_XL), the 2-bit cliff and the 9B IQ-quant latency
reversal are recorded, and Jev-Style-0.8B is entered as an
interface-mismatch row. 2026-09-26 first pass — **Qwen3.5-2B**, moved from
Gemma because a 3-item gap was noise while calibration (0.062 vs 0.236),
relational (0.50 vs 0.45), half the parameters, and ~57% of the latency
were not. 2026-09-25 original — **gemma-3-4b-it** (Q4_K_M GGUF, sha256
`882e8d2d…`), led every class (0.95 / 0.85 / 0.45, 0.750 overall) over
the original five typed models (Qwen2.5-{0.5,1.5,3}B, Llama-3.2-1B;
Qwen2.5-3B runner-up at 0.675).*

**Serving mechanism is NOT the decision.** The measurement went through
llama.cpp's `/v1/decision`; the integration seam in this codebase remains
the `opencodifier-runtime` `InferenceBackend` trait (D7, logits-only). The
ONNX re-entry gate is D2. Any backend that can produce candidate-conditioned
logits is admissible; the pick names the weights, not the server.

**Conditions carried forward (blocking exposure, not the pick):**
- Raw winner probability is uncalibrated at every model (ECE 0.048–0.626
  across the forty-one runs; even the best, MiMo-9B Q3_K_S at 0.048, has
  had no post-hoc calibration fitted). D15 calibration must be fit per
  class before any confidence leaves the runtime (§73 confirmed by
  measurement).
- relational_compositional held at ≤ 0.50 for every arm until MiMo-9B
  Q3_K_S reached 0.525 at 14.3 s/decision; every interactive-tier pick
  scores 0.35–0.50 there — that class stays escalation/verifier
  territory for anything latency-bounded, which is precisely why a
  relational-weak interactive reference is tolerable.
- The ladder holds: the lexical engine at 5.3 ms keeps metadata_match
  (0.88 vs 1.00 for the best model at ~700× the latency); a model call is
  only bought when cheaper layers abstain.
- Single-shot, chat-JSON and decision-arm latencies are comparable on CPU
  (prefill-dominated); the decision arm earns its slot on determinism
  (bit-exact replay vs batch-composition-sensitive generation), exact
  candidate distributions, and 5–8× bulk throughput — not on single-shot
  wall clock. Note the arm's accuracy dominance is model-dependent: the
  Qwen3.8-2B distill is the first arm whose chat baseline beats its
  decision score (0.700 vs 0.617) — reasoning-by-writing distills lose
  under forced token-path commitment.
