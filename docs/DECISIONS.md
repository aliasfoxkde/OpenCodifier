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

**Amendment (2026-09-28, relational solver):** the model id in the key
is the **live classifier's** id — `DecisionEngine::new` derives
`identity.model_id` from `Classifier::model_id()` at construction, and
wrapper classifiers compose their ids (`relational-v1|builtin-lexical-v1`),
so a swapped or decorated classifier invalidates cached decisions
without caller bookkeeping. A hand-set `EngineIdentity::model_id` is
never trusted over the decider that actually runs.

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

**Amendment (2026-09-28, implemented as Phase 14).** The first shipped
scheme is temperature scaling, not isotonic or Platt: the benchmark run
JSONs record winner probability + correctness only, which fits a
temperature exactly (winner-vs-rest margin fit) and nothing else.
Concretely: `Calibration` trait in the engine (`calibrate(class,
distribution) -> f64` + `version()`); `IdentityCalibration` = raw with
version 0 ("none"); `TemperatureCalibration` from a validated JSON
artifact (`format_version 1`, `scheme "temperature"`, temperatures
keyed by question kind, `deny_unknown_fields`, `calibration_version ≥
1` — 0 stays reserved for "none"). Artifact versions fold into cache
keys via `EngineIdentity`, so adopting a refit invalidates cached
decisions. Fitting is offline Python over benchmark runs
(`benchmarks/decision-model/runner/fit_calibration.py`); the fitted
artifacts for the D16 tier arms are committed under
`benchmarks/decision-model/results/calibration/` with measured
before/after ECE in `results/CALIBRATION.md`. The embedding rung's fit
is degenerate (no finite temperature helps; scores are ordering-only),
so no artifact is shipped for it — the per-model opt-in model means the
engine refuses to fake confidence where the data says none exists.
Per-class artifacts beyond the question-kind keys (task-level classes)
wait on the IR carrying class tags; the fitter already prints the
per-class spread, and it is large (0.5–13 across suite classes on 4B).

**Amendment (2026-09-28, relational solver):** an artifact ships only if
the fit does not worsen ECE — in-sample NLL improves on nearly any
1-parameter fit, so ECE is the shape check. The engine arm of record is
now the relational solver's bimodal proof/delegate stack, whose global
fit (T = 0.658) worsens ECE (0.094 → 0.097): no artifact ships, the
arm keeps identity calibration, and the retired `builtin-lexical-v1`
artifact was removed with the bare-lexical stack it described. The
fitter enforces the gate.

## D16 — Decision-model picks: tier scheme (measured, Phase 13; amended ×8)

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

*Amendment history:* **2026-09-28 eighth pass (relational solver, PLAN
Phase 15) — the zero-ML floor is re-tiered.** The engine's default stack
became the relational solver over the lexical classifier (exact proofs
over extracted facts, delegating everything it cannot prove); the
re-run engine arm on the byte-locked suite scores **0.683 blended
(0.88 / 0.23 / 0.950) at 1.3 ms p50** (REPORT F22). Consequences: (a)
**the fast tier is undercut on both axes** — Qwen3.5-0.8B (0.650 @
613 ms, 537 MiB) is retained as the fast *model* tier for lexical-heavy
residuals, but any deployment whose residual stream is metadata +
relational structure should ride the engine floor instead: 0.683 @
1.3 ms, zero MiB, zero download, bit-deterministic; (b) **the 0.50
relational ceiling is now known to bind likelihoods, not proofs** —
MiMo-9B keeps the model-arm crown (0.525 @ 14.3 s), while the runtime
proves 0.950 where models guess; (c) the calibration of record for the
engine arm stays identity — the global temperature fit worsens ECE on
the bimodal proof/delegate distribution (CALIBRATION.md), and the old
`builtin-lexical-v1` artifact was retired with the stack it described.
Board stands at 49 runs. 2026-09-28 seventh pass (blockwise int4 arm) — no
tier changes; the Q4_0-analog question is measured closed. ORT 1.30's
`MatMulNBits` quantizer (block 32, asymmetric, 4-bit) on the gte fp32
graph yields a 226 MB build scoring **0.575 / ECE 0.330 — blended,
per-class (0.47-0.75-0.50), and ECE identical to fp32 to the digit — at
815.7 ms/item vs 811.4: no speedup** (REPORT F21). Blockwise 4-bit is
therefore a *free memory fallback* (2.6× smaller at zero quality cost),
not a speed play — the CPU MatMulNBits kernel does not beat fp32 GEMM
at batch-1 encoder shapes; the fp32 runtime stands. Contrast F20: the
int8 collapse was the *scheme* (per-channel dynamic), not quantization
itself. q4f16 (int4 weights, fp16 activations) is untestable on this
host — ORT's 4-bit op has no fp16-compute CPU path; that format belongs
to QNN/CoreML-class EPs. Board stands at 48 runs. 2026-09-28 sixth pass (embedding-rung bake-off) —
no tier changes; the **embedding rung's runtime moves to ONNX fp32**:
the same gte-modernbert-base encoder on onnxruntime scores identically
to the torch row to the digit (0.575 / ECE 0.330 / 0.45-0.78-0.50) at
**811.4 ms/item vs 3374.8 — 4.2× faster on CPU** (REPORT F20). Dynamic
int8 is rejected (0.442 — metadata 0.45→0.28, relational 0.50→0.28 —
for −17% latency); an fp16-weights export is unloadable by onnxruntime
(mixed-dtype LayerNormalization from torch 2.14's exporter) and
skipped; EmbeddingGemma-300M Q8_0 via llama.cpp (319 MiB, new
`llamacpp` backend in run_embed.py) records 0.500 / ECE 0.256 /
634.9 ms — below the gte rung reference on accuracy, better
calibrated, no tier. The board stands at 47 runs; gte (any runtime)
remains embedding-rung reference only — ECE ≥ 0.330 keeps it behind
the D15 calibration gate. 2026-09-28 fifth pass (ternary second pass) — no
tier changes, no new completed runs; two ternary candidates resolved as
measured negatives. **Bonsai-8B** (Q1_0, 1105 MiB; `Bonsai-8B.gguf` and
`Bonsai-8B-Q1_0.gguf` are byte-identical artifacts under two names)
loads and answers a trivial prompt in 5.4 s, then prefills at ≈1.5
s/token beyond a ~30-token knee (196 s @ 130 tokens, 405 s @ 260, >420 s
@ ~520) — ≈26× Bonsai-4B per-token at matched length despite shipping
the *same* Q1_0+F32 type set (verified from the GGUF headers); the
120-item suite projects to ≈9–13 h of pure prefill, so the arm is out
of campaign budget (REPORT F18). **Ternary-Bonsai-2-27B** (PTQ1_0,
5.67 GiB, qwen3, multimodal per its mmproj files) is unloadable in the
ad129b0 build: `output.weight has invalid ggml type 143. should be in
[0, 43)` — prism-ml's packed type requires their own fork, and no other
file in the repo is host-viable (F16 = 51 GB; PQ2_0 = the unreadable
Ternary-8B family) (REPORT F19). Net: on-CPU ternary viability in this
build is 4B-and-below; the #35 accelerator question now spans
Q2_0_g64-at-8B, Q1_0-at-8B, and type-143 PTQ1_0-at-27B. 2026-09-28
fourth pass (extension) — no tier changes;
three arms added. **Bonsai-4B** (ternary, 546 MiB) posts the board's best
accuracy-per-byte (0.650 — the 0.8B tier's accuracy at a quarter of the
size) but at 4B-class latency (4.86 s p50), so it wins no tier; recorded
as a measured record and an accelerator-tier candidate, not a pick.
**LFM2.5-2.6B** Q4_K_M (0.608 @ 2.55 s — first hybrid-conv architecture
tested) is dominated by Qwen3.5-2B at the same latency; the DavidAU
"X12 NEO MAX" merge (0.667 @ 8.35 s, best sub-3B lexical at 0.775) is
latency-toxic and wins no tier. **Ternary-Bonsai-8B is not measurable on
the CPU host** (g64 quant >10 CPU-hours per decision request; plain
Q2_0/PQ2_0 tensors unreadable by the ad129b0 build; F16 exceeds host
memory) — ternary-class weights are an accelerator question (feeds the
#35 Vulkan A/B). 2026-09-27 third pass — tier scheme gains the
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
  across the forty-four runs; even the best, MiMo-9B Q3_K_S at 0.048, has
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

## D17 — MCP surface: rmcp 2.2 stdio, six decision tools (2026-09-28)

`opencodifier-mcp` (PLAN Phase 10, §31/§55) serves the runtime to MCP
hosts over **stdio only** — the transport is the local process's
stdin/stdout, so the local-first posture is structural: there is no
socket to bind and nothing remote to refuse (unlike HTTP's loopback
gate, there is no flag that opens this surface outward).

- **SDK pin stays at rmcp `=2.2` (D1).** rmcp 3.x exists (major line,
  breaking API) and is **not** adopted; upgrading would be a new decision
  with its own migration evidence. Phase 15's tool set, verbatim:
  `codify_decide`, `codify_batch`, `codify_graph`, `codify_validate`,
  `codify_verify`, `codify_explain`. §31's two model-dependent tools
  (`classify`/`score`-shaped) wait for the model rungs, as planned.
- **Thin surface, one pipeline.** Tools normalize through the schema
  adapters and execute through `EngineHandle` — the identical stack HTTP
  drives. The engine is sync (D5); the HTTP shell off-loads to blocking
  threads because a concurrent server must protect unrelated
  connections, while a stdio MCP session is strictly sequential, so MCP
  calls the engine inline. That is a deliberate asymmetry, not an
  inconsistency.
- **Batch bound.** `codify_batch` accepts at most 16 requests
  (`mcp.batch_too_large` above it) and decides items independently: one
  item's refusal is that item's error envelope, never a failed batch.
  The bound is transport hygiene, not a pipeline limit.
- **Error discipline.** Tool failures are tool-level results
  (`is_error: true`) carrying HTTP's exact envelope
  (`{"error":{"code","message"}}`) under the same stable codes; the
  crate's only own code is `mcp.batch_too_large`. Abstention is a
  successful result with the typed outcome — §73's rule survives the
  transport.
- **Shared surfaces, one definition each.** The engine's graph-document
  shim is public (`GraphDocument`) and HTTP's private mirror is deleted;
  the execution-report JSON projection lives in the engine
  (`opencodifier_engine::report::execution_json`) and both the CLI's
  `--trace` and MCP `codify_explain` call it.
- **`codify_graph` exposes the active graph document** (via the new
  `EngineHandle::graph`): the identity decisions are cached under, node
  count, parallelism, cache state, and the validated DAG itself. This is
  local introspection of a local runtime, consistent with §31's
  explainability stance; §32's deeper tool introspection remains future
  work.

## D18 — Focused-question extraction: recall-oriented views + reverse escalation (2026-09-28)

PLAN Phase 16 (§45): a decision-model rung pays per token while every
cheaper rung reads whole state. The engine may therefore decide a
question on a **per-question view** of long state — but only under
rules that make the view strictly safer than truncation:

- **Extraction is deterministic, model-free, and recall-oriented.**
  Sentences split with delimiters attached (kept views are the original
  prose, byte for byte, in original order); BM25 over sentence
  documents with question text + candidate descriptions as the query;
  verbatim candidate-id mentions are unconditional evidence. Zero-
  evidence sentences are never selected, so the *median* view sits near
  the budget while every kept sentence earned its place. Token
  accounting is the engine's bytes-over-4 estimate — a budget, not a
  measurement (no tokenizer exists; D2).
- **Blind extraction declines; decisive sentences are never amputated.**
  When no sentence shows positive evidence the view is the full state;
  when one sentence exceeds the whole budget it is kept whole. A view
  that saves nothing is the full state. Dropping the one decisive
  sentence is strictly worse than an over-long view, so recall beats
  the budget everywhere they conflict.
- **Reverse escalation, once, pre-gate.** If extraction engaged and the
  focused top probability is below the request's `min_confidence`, or
  entropy trips the §19 ceiling (default off), the engine re-decides on
  the full state before any gate or verifier sees an answer. The
  fallback cannot produce a *degraded accepted* answer; its cost is one
  extra classify, and it is visible in the trace
  (`focus_escalated`) and the report counts (`decided/engaged/
  escalated`).
- **Facts are never touched.** Only the text view shrinks; structural
  facts pass through to every view. Hostile state text is content, not
  instruction — a view feeds a classifier and nothing else, and no
  policy/threshold/graph decision is ever read from state text
  (§73, unchanged).
- **Cache identity (D6 extension).** The focus policy folds into the
  model-id component as `|focused-v1@<budget_tokens>`, so budget
  changes and focus on/off invalidate cached decisions without caller
  bookkeeping.
- **Byte-stability for existing consumers.** Unfocused engines have no
  focus surface at all: trace keys appear only when a policy is
  configured, `RunReport::focus` is all zero without one, and
  `execution_json` inserts `"focus"` only when `decided > 0`.
- **Evidence (Phase 16).** Long-suite A/B through the real binary
  (`suite_long.json`, ~3,681-token states): answer-identical to the
  full-state run on 120/120 items at `--focus-budget 512`, engaged
  views p50 93 tokens, 36 blind declines, 42 escalations (none
  answer-changing), ~2.5 ms/item extraction overhead at this scale
  (REPORT.md "Long-context A/B"). Measured limits: the A/B exercises
  benign dilution, not adversarial distractors; a distractor that
  out-scores the decisive sentence under BM25 remains the known failure
  mode, caught (not prevented) by escalation — recorded as the
  extractor's standing threat model.

## D19 — `/v1/graph/run`: client-supplied graphs are ephemeral and content-addressed (2026-09-29)

§36 lists `POST /v1/graph/run` among the primary endpoints: a client
submits a graph document plus a canonical request, and the runtime
decides through *that* graph instead of the built-in pipeline. That is
the one surface where the client controls pipeline structure, so it
gets the tightest rules in the runtime:

- **Body and response.** `{"graph": <GraphDocument>, "request":
  <canonical request>}`; the response is the standard native decision
  envelope plus an `"identity"` object reporting the identity the run
  actually executed under. Shape errors are `schema.*` (400); structural
  graph failures are the engine's own `graph.*` codes, identical to
  `/v1/graph/validate`. Abstention stays a `200` — §73's posture does
  not bend for ad-hoc graphs.
- **Content-addressed identity, never client-asserted.** The scoped
  engine's `graph_version` is the first 8 bytes (big-endian) of
  SHA-256 over the graph's canonical serialization (`version` + node
  specs, serde field order). A client naming `graph_version: 1` gets a
  hash instead; two runs of the same graph share one identity, and any
  node edit changes the hash. This keeps §73's rule (cache keys fold
  graph identity) true by construction: a decision made under graph A
  can never be served for graph B.
- **The shared serving cache is never consulted.** The scoped engine is
  assembled fresh per request — ad-hoc graph runs are an
  evaluation/inspection surface, not the serving path — so any cache it
  has is created and dropped with the handle: a one-slot, one-second
  private cache that serves at most a repeated question inside one
  `decide` call. (The cache rejects a zero-capacity config by contract,
  so "never the serving cache" is expressed as throwaway assembly, not
  as a disabled flag.) Assembly is cheap (validation + rule
  bookkeeping; no model loading under the lexical posture), so
  per-request engines cost microseconds and no state survives the
  request.
- **What the client does not control.** Calibration, parallelism caps,
  execution-time ceilings, and body limits are the host's; the scoped
  engine carries no rule pack and no verifier — graph nodes plus the
  built-in classifier are the whole pipeline. Node count is capped at
  `limits.max_graph_nodes` (128) *before* construction, and the DAG
  contract (ids, edges, cycles, single output) is enforced by the same
  `DecisionGraph` constructor `/v1/graph/validate` uses — never a weaker
  local copy.
- **Engine surface, one definition.** `EngineHandle::ephemeral(graph)`
  is the only constructor for scoped runs; it derives the
  content-addressed identity and disables the shared cache internally.
  HTTP composes it with the ordinary decode/decide/encode path — no
  endpoint-local pipeline logic (D5 sync core; the async shell stays in
  the interface crate).
- **MCP parity deferred.** MCP `codify_graph` remains introspection
  only; an ad-hoc execution tool would need the same identity rules and
  is future work, not an omission.

## D20 — Decision Registry: definitions are content-hashed artifacts, requests stay canonical (2026-09-29)

§63 wants decision definitions to become reusable artifacts: a named,
versioned document carrying the question shape and the policy, from
which a runtime builds a real request. The registry is bookkeeping over
artifacts; it is never a second decision path.

- **One type, one crate.** `DecisionDefinition` lives in
  `opencodifier-schema` (`registry.rs`) next to the wire adapters: the
  document is a wire format, and instantiation goes through the same
  core constructors (`ChoiceQuestion::new`, `DecisionPolicy`'s
  validating conversion) every other ingress uses — never a weaker
  copy. Fields: `id` (dotted, non-empty), `version` (u64, ≥ 1),
  `question` (`type` + `text`), `candidates` (a choice question's
  static candidate list, or `{"dynamic": true}` for caller-supplied
  ones), and an optional `policy` (absent = `DecisionPolicy::default`).
- **Identity is content, labels are labels.** A definition's identity is
  the SHA-256 hex of its canonical serialization (`content_hash()`).
  `id` and `version` are labels for humans and tooling — naming a
  version does not make two different documents the same artifact, and
  editing a document changes its identity even if the labels do not
  move. This is D6's discipline applied one level up.
- **Definition identity never enters cache keys.** The instantiated
  `DecisionRequest` is canonical and cache keys fold the request content
  plus engine identity (D6), nothing else. Two definitions that
  instantiate to byte-identical requests legitimately share a cached
  decision — the decision depends on the request, not on which artifact
  produced it. Provenance stays artifact-side; instantiation does not
  smuggle the definition id into `RequestMetadata`, where it would
  perturb keys.
- **Instantiation is total or refuses.** A choice definition with
  dynamic candidates needs the caller's candidate list at
  instantiation; a static one refuses extra candidates rather than
  silently merging. Question text and candidates go through the same
  validating constructors as any wire request, so an invalid definition
  fails at load with the adapter's own `schema.*` codes — a registry
  cannot hold a definition the engine would refuse.
- **The registry itself is boring on purpose.** `Registry` indexes
  definitions by `id` and rejects duplicate ids at construction —
  lookup, not discovery; no filesystem, no network, no reload. Loading
  documents from disk is the caller's job (CLI, tests, recipes), which
  keeps the crate sync and local-first.
- **Recipe, not ceremony.** One recipe demonstrates the path:
  definition → instantiate → decide through a committed graph, with the
  response captured by the same procedure as every other recipe — the
  registry has to earn its place in the fleet, not ship as an unused
  library.

## D21 — Retrieval and rerank are explicit, disclosed narrowing stages (2026-09-29)

PLANNING §24/§52 (embeddings), §53 (retrieval), §26/§54 (reranking) land
as three graph node kinds — `embedding`, `retrieve`, `rerank` — plus two
engine seams. The design keeps every §73 rule intact while making the
spec's `50 → metadata → BM25/embedding → 10 → reranker → 3 → decision`
pipeline expressible as a plain DAG.

- **Three nodes, three honest jobs.** `embedding` only *annotates*: it
  embeds the state text once per run and scores every surviving
  candidate description by cosine, recording the scores — it never
  removes or reorders. `retrieve` narrows: per choice question it keeps
  the top `top_n` candidates whose semantic score clears `floor`, and
  every dropped candidate is named in the trace with its score — never
  silent. `rerank` reorders: it rebuilds each choice question's
  candidate list in the reranker's score order — it never removes.
- **A floor that cannot starve.** If the floor would eliminate every
  surviving candidate, `retrieve` keeps the top-1 and lets the
  confidence gate — not a narrowing artifact — decide the question's
  fate. Narrowing stages never manufacture an abstention.
- **The Reranker seam is a trait with two implementations.**
  `Reranker::rerank(query, candidates) → Vec<ScoredCandidate>` exactly
  as §26 sketches it. `LexicalReranker` is the zero-ML default (BM25
  over candidate descriptions, the same primitive as the `lexical`
  node); `EmbeddingReranker` is cosine over an `EmbeddingBackend`
  (PLANNING §24). The node spec names its reranker (`"lexical"` |
  `"embedding"`); the engine never silently substitutes one for the
  other.
- **Missing backend ⇒ refuse at assembly, never at request time.** The
  handle carries the optional embedding backend; a graph naming
  `embedding`/`retrieve`/embedding-`rerank` nodes on a handle without
  one fails assembly with `engine.missing_backend` before a socket
  opens. The zero-ML base binary stays useful with no model — by
  refusing graphs that need one, not by degrading them into lies.
- **Safe mode refuses retrieval pruning, same rule as the prune knob.**
  `retrieve` is semantic pruning with extra disclosure. Safe mode (the
  default) rejects any graph containing a `retrieve` node at assembly,
  exactly as it rejects the `lexical_prune_limit` knob; the opt-out is
  explicit (`with_safe_mode(false)`), per-engine, and never per-request.
  Hostile input cannot switch it.
- **The embedding backend is part of engine identity.** `EngineIdentity`
  gains `embedding_model` (default `"none"`), folded into every cache
  key by the single normative `CacheKeyBuilder`. An embedding swap or
  upgrade invalidates cached decisions exactly as a classifier swap
  does (§73, §64) — semantic scores feed narrowing, so they are decision
  inputs and get identity like any other.
- **Node validation is additive and strict.** `retrieve` requires
  `top_n ≥ 1`; `floor` only means something on `retrieve`;
  `reranker` only on `rerank`; unknown knobs on other kinds are ignored
  by serde but rejected by the validator, because a graph that carries
  dead knobs is a graph the author did not mean.

## D22 — The optimizer only removes the unobservable (2026-09-29)

PLANNING §51 wants a decision compiler. V1.1 ships the two passes whose
correctness is provable over the DAG's actual semantics — and refuses
the passes whose "optimizations" would be guesses.

- **Equivalence is over decisions, not traces.** Two graphs are
  equivalent when every request produces identical answers, outcome,
  and confidence. Traces legitimately differ (fewer nodes executed);
  they are instrumentation, not output. Everything below is measured
  against that definition, fleet-wide: every recipe graph decides
  byte-identically (answers, outcome, confidence, identity's model
  fields) optimized and unoptimized.
- **Pass 1 — dead-node elimination is reachability from `output`,
  minus the cache.** A node no `output`-reachable path contains can
  never contribute to a response — with one exception: `cache` is
  observable across requests (it probes/reports the key and a later
  request reads the same cache), so cache nodes are never eliminated,
  however unreachable. Everything else is a pure function of its
  dependencies.
- **Pass 2 — CSE over identical pure specs.** Two nodes with equal
  `(kind, depends_on, knobs)` in the pure set (`normalize`, `rule`,
  `filter`, `lexical`, `embedding`, `retrieve`, `rerank`, `choice`,
  `boolean`, `score`, `branch`) compute the same value against the same
  wave snapshot, so dependents re-point to the first and the duplicate
  disappears. `cache` is excluded (side-effecting), `threshold` and
  `output` excluded (unique, response-shaping). Duplicate entries in a
  `depends_on` list are dropped as part of normalization.
- **What V1.1 refuses, and why.** Constant folding has no constants
  worth folding (rule conditions read hostile input; folding one input
  path specializes the graph to a benchmark). Early exit and
  parallelization hints change wave structure and therefore traces and
  timing contracts for gains the per-stage budgets (D9) have not asked
  for. Candidate pruning is not a graph transform at all — it is
  narrowing, and D21 already owns its disclosure rules. The optimizer
  that ships is small enough to prove.
- **Surface.** `opencodifier_engine::optimize(graph) → DecisionGraph`
  (pure, returns a fresh graph; `graph_version` is preserved — D6 keys
  on the author's version, and optimization must not silently change
  cache identity). Equivalence proofs: per-pass unit tests plus a
  fleet-wide test that optimizes every committed recipe graph and
  asserts identical decisions.

## D23 — The WASM boundary is the zero-ML decision runtime (2026-09-29)

PLANNING §57 asks for `opencodifier-wasm` (CPU/SIMD/threads/WebGPU) with
a browser demo; §68 fixes the security posture. V1.1 lands the boundary,
not the model path:

- **What compiles to `wasm32-unknown-unknown`:** `core` + `engine` +
  `schema` (and `runtime`'s traits). These are sync, allocation-only,
  IO-free crates — verified by `cargo check --target
  wasm32-unknown-unknown`, not assumed. The WASM crate depends on
  nothing else: no tokio, no axum, no `ort`, no filesystem, no
  networking, no environment access, no storage API of any kind (§68's
  IndexedDB clause is satisfied vacuously until a storage adapter
  exists, and the crate is the only place one may ever live).
- **The exposed engine is the base posture.** `WasmEngine` assembles
  `EngineHandle::lexical` over the default pipeline — relational solver
  over BM25, exactly the zero-ML stack the native base binary ships.
  Wire formats go through the `schema` native adapter only; the browser
  never sees IR internals unvalidated.
- **The model rung stays native for now — stated, not hidden.** §57's
  WebGPU/ONNX-Web path would put `ort` in the browser; that is a
  runtime choice (D2 territory) with its own supply-chain and
  integrity story (§68's "model downloads explicit and integrity
  checked"). No browser model path ships until that record exists.
  The WASM build's usefulness does not depend on it: deterministic
  decisions, rules, narrowing, and traces run fully client-side.
- **Security posture is structural, not promised (§68).** No dynamic
  native code: the artifact is pure wasm, no `-sys` crates, no JS
  shims beyond `wasm-bindgen`'s glue. No telemetry: there is no
  output channel. All input is hostile: every entry point decodes
  through the validating schema adapters and returns typed errors;
  nothing from a request can touch policy, thresholds, graph
  structure, or paths (PLANNING §73 carries over unchanged).
- **Verification without a browser.** The artifact is built with
  `wasm-pack --target nodejs` and exercised in Node — decide,
  validate, hostile-input refusals — because the same wasm module the
  browser loads is the module the harness runs. The browser demo
  (§57's drag/drop page) is presentation on top of the exact artifact
  the Node harness proves; it stays out of V1.1 and rides the
  distribution phase (§58) rather than blocking the runtime.
- **The platform seam the Node harness forced into the open.** The
  first artifact trapped: `std::time::Instant::now()` has no source on
  `wasm32-unknown-unknown` (a wasm module reads the *host's* monotonic
  clock), and `std::thread::spawn` is unavailable there. Both are now
  declared seams, not runtime surprises: `clock.rs` re-exports
  `std::time::Instant` natively and the `web-time` reading on wasm32
  (target-gated dependency — the native tree gains nothing), and the
  executor's parallel-wave path is compile-time gated off on wasm32, so
  `RunReport::parallel_waves` is 0 there by construction. Deadline
  math, cache TTLs, and the sequential wave path are the same code on
  every target; only the reading and the thread count are the
  platform's.

## D24 — The distribution matrix is what this host can honestly verify (2026-09-29)

PLANNING §58 asks for cross-platform release distribution. The matrix is
set by verification honesty, not aspiration — every column names how it
is proven, and nothing ships a claim its evidence cannot carry:

- **linux-x86_64-gnu** — the platform of record: release-built,
  run-tested, e2e-proven (30/30) on this host; GitForge releases carry
  it today.
- **windows-x86_64-gnu** — the full workspace (all ten crates incl.
  `opencodifier-wasm`) compiles clean for
  `x86_64-pc-windows-gnu` with the mingw linker present. **Linked
  (2026-10-02, 2m44s at -j4 alongside the measurement chain):**
  `opencodifier.exe` (PE32+ console, x86-64) and
  `opencodifier_wasm.dll` (cdylib), zero warnings. Verification level
  is link success + `file` PE magic + a stated not-executed-here
  caveat — and that is this host's terminal level for the target: no
  wine and no Windows machine exists here, so the smoke run is not
  "pending a quiet window", it is structurally out of scope for this
  host (a Windows box or CI runner with wine would be the venue).
- **linux-aarch64** — `aarch64-unknown-linux-{gnu,musl}` targets
  installed, `cargo-zigbuild`/`zig` present for linking; both linked
  2026-10-01 (the gnu one dynamically, the musl one static). A smoke
  run is structurally out of scope on this host for the same reason
  as windows: no qemu-user (and no arm hardware) exists here — an
  arm box or a qemu-capable runner would be the venue.
- **darwin x86_64/aarch64** — rust targets installed; `cargo-zigbuild`
  can link Mach-O without an Apple SDK. Verification is link success +
  `file` magic only: no macOS host runs it here, no codesigning or
  notarization exists, and the artifact states that in its release
  notes rather than implying support.
- **Package-manager publication (crates.io, Homebrew, winget) is an
  outward action, never an automatic one.** Each needs an account,
  credentials outside the repo, and an explicit user go — the release
  flow of record is GitForge releases; external registries are opt-in
  follow-ups, listed in PLAN 18j's remainder.
- **The WASM artifact rides releases** as `wasm-pack` output (built by
  `just check-wasm`, gitignored in-tree): `pkg/` attached when a
  release is cut, never committed.

Rule the matrix enforces: a target either has named verification
evidence or is named as pending — the release page never says
"supported" where the truth is "compiles".

**Status 2026-10-01 — the staged cells now carry their named
evidence.** The overnight cross-build run (recorded matrix:
`runs/crossbuild__matrix.md`, rustc 1.98.1 / zig 0.16.0, from the repo
root) links all four remaining targets: `aarch64-unknown-linux-gnu`
(dynamically linked ELF aarch64, 4.4 MB),
`aarch64-unknown-linux-musl` (statically linked ELF aarch64),
`aarch64-apple-darwin` and `x86_64-apple-darwin` (Mach-O arm64 /
x86_64). Per this record, that is link + `file`-magic verification
only — no foreign-arch execution is claimed (no qemu on the host; no
macOS host exists here). `windows-x86_64-gnu` **joined the linked set
2026-10-02** (`opencodifier.exe` PE32+ + `opencodifier_wasm.dll`,
`file`-magic verified, not executed — no wine/Windows on this host, so
smoke stays structurally out of scope here, not merely quiet-window
pending). What remains open in the matrix is unchanged: registry
publication is an explicit user go, and native smoke exists only for
linux-x86_64.

## D25 — The escalation ladder is engine-internal per-node policy, never IR (2026-09-30)

The fusion study (F23: engine→gte→2B blend 0.867 vs 0.725 best single)
bounds what a confidence-gated ladder can score, and the rungs'
confidences are not commensurable — a relational proof is p = 1.0 by
construction, the embedding rung's scores are ordering-only (D15
finding 3: the probability fit is degenerate; the measured gate is
`min_margin ≈ 0.018`, `results/embed-margin-study.md`), and the model
rung carries a fitted temperature. One request-level policy cannot gate
all of them. The wiring decision:

- **`LadderPolicy` composes the existing gate type; the IR does not
  move.** Engine config gains optional per-node / per-kind
  `DecisionPolicy` overrides (`opencodifier-engine/src/ladder.rs`),
  resolved at the existing confidence gate: `per_node[id]` →
  `per_kind[kind]` → request policy. No wire change, no schema change —
  `pub(crate)`-level plumbing only (`QuestionDecision::decided_by`
  records which node decided, so the threshold node re-resolves the
  same rung).
- **The cache stores completed (gated) responses**, so the stale-gate
  hazard is real; the protection is identity: a configured ladder
  decorates the engine's model id as `|ladder-v1@<id>` (same rule as
  focused extraction, D6) and rides every cache key. A changed ladder
  re-keys; it never re-gates old responses. Assembly refuses a
  non-empty ladder whose `id` is empty or `"none"` — an anonymous
  ladder is a correctness bug, not a style choice.
- **Empty ladder is byte-identical to no ladder** — no decoration, no
  `policy_source` trace facts, locked wire fixtures. The ladder ships
  opt-in (F24: gates inherit their rung's calibration; per-domain
  calibration precedes a default-on ladder).
- **Traces explain the gate**: when an override fires, the decide and
  threshold entries record `policy_source` (`node:<id>` /
  `kind:<name>`) — the no-hidden-thresholds rule applies to the ladder
  like every other gate.
- **`NodeSpec::threshold` remains unwired** (validated and
  fingerprinted since introduction, never consumed by the executor).
  The ladder composes alongside it; wiring the graph scalar to the
  gate is a separate, deliberate change with its own record.

**Amendment (2026-10-01 — per-rung calibration and shipped profiles).**
The ladder's remaining D25 scope is wired and the profile documents
ship:

- **Per-rung calibration.** `LadderPolicy` carries
  `calibrations: BTreeMap<String, Arc<dyn Calibration>>` keyed by the
  same `node:<id>` / `kind:<name>` strings the trace records; the
  executor resolves it beside the policy and a rung calibration
  replaces the engine-level calibration for that rung's questions
  (`None` = unchanged, byte-identical). The trace records
  `calibration_source` when one fires. A calibration-only ladder is
  non-empty — validate() demands the identity for it too, so an
  artifact swap can never re-gate decisions under single-policy cache
  keys.
- **`ProofAwareCalibration`** (CALIBRATION finding 2, no artifact
  shipped): exact proofs arrive as single-entry distributions at
  p = 1.0 and pass through raw; multi-entry distributions delegate to
  the inner calibration. Its `version()` is the inner version — safe
  because the wrapper only changes semantics inside a ladder whose id
  decorates the cache identity; bump the ladder id when wrapper
  behavior changes, exactly like an artifact swap.
- **Profiles are data.** `LadderProfile` (serde, `deny_unknown_fields`)
  loads a JSON document — per-kind/per-node policies through
  `DecisionPolicy`'s validated mirror, rung calibrations as D15
  artifacts with an optional `proof_aware` wrapper — and refuses an
  empty profile. The CLI surfaces it as `--ladder <PATH>` on `decide`,
  `serve`, and `mcp serve` through the one shared `runtime::engine_config`,
  so the three interfaces cannot drift. HTTP config surfacing stays
  deferred: no engine-configuration surface exists on `/v1` yet
  (D12), and a flag that silently re-gates a running server would
  need its own record.
- **Shipped profiles** (`ladders/`, kept loadable by test):
  `fusion-v1` — rule rung accepts proofs at 1.0; classifier kinds run
  the measured accept-on-margin shape (`min_confidence 0.0` +
  `min_margin 0.0183`, `results/embed-margin-study.md` — the gate
  cascade is demote-only, so accept-on-margin is expressed by a
  probability gate that never fires and a margin floor that demotes
  near-ties). `proofs-only-v1` — only proofs accept outright.
  Ladders remain opt-in (F24: per-domain calibration precedes any
  default-on ladder).

## D26 — The model rung is a prompt-shaped Classifier over llama.cpp, not a tensor backend (2026-10-02)

The escalation ladder (D25) has one missing rung: the decision model
the measured board says is worth escalating to (F23: the simulated
engine→gte→2B blend scores 0.867 @ 753 ms vs 0.725 best single). The
serving reality on this host is the llama.cpp `parallel-decision`
fork — its native verdict-slot readout is the only interface that ever
scored the Jev-Style weights honestly (F26: 0.8083 native vs 0.217
tree-mode mismatch, F10), and ONNX Runtime is not the path (F25: ≥9×
prefill-bound latency; no Vulkan EP exists to fix it). The wiring
decision:

- **The rung implements `Classifier`, not `InferenceBackend`.**
  `InferenceBackend` is the tensor-in/tensor-out contract of the ONNX
  serving path (D7: named `DenseTensor`s). The fork's readout is
  prompt-shaped — a rendered candidate-conditioned prompt in,
  verdict-slot logprobs out. Forcing one shape onto the other would
  fabricate tensors and re-create the F10 interface mismatch this
  board spent a phase correcting. `LlamaDecisionClassifier`
  implements `opencodifier_engine::Classifier` (decide →
  `Distribution`) and assembles through `EngineHandle::new` /
  `with_ladder` as decider or verifier — zero engine changes, the
  same seam every other rung uses.
- **Rust owns the decision math.** The server returns logprobs; the
  softmax over the verdict slot and any calibration happen in Rust
  f64 (D7), and confidence is calibrated through the existing D15
  seam before any gate reads it. Server-reported probabilities are
  never exposed raw.
- **One crate, one feature, one new dependency.** The client lives in
  `opencodifier-model` behind a `llamacpp` feature; the HTTP client
  is `ureq` (sync, `default-features = false`, `json` only) — blocking
  matches the engine's sync core (D5), and a leaf crate gains no async
  runtime. Plain HTTP, deliberately: the endpoint is loopback by the
  bind rule, and every Rust TLS backend (`ring`, `aws-lc-rs`) drags in
  either an OpenSSL-derived license outside `deny.toml`'s allowlist or
  a C toolchain build for a hop TLS cannot help. Without the feature
  the default build is dependency-identical; with it, cargo-deny
  licenses and advisories gate the addition like any other.
- **No live server in the test suite.** A minimal `Transport` trait
  (JSON in, JSON out) is the injection seam: unit tests script
  verdict-slot responses, timeouts, malformed payloads, and non-200s
  through a mock. Real-server integration is opt-in against a
  loopback endpoint (the bind rule — a default must never point at a
  remote host) and skips when absent.
- **Determinism is a contract, not a hope.** Sampling is pinned
  greedy server-side; `model_id()` composes build + GGUF + readout
  config and feeds cache keys like any classifier's (D6); timeouts
  ride the engine's `Deadline` seam; retries are bounded and
  explicit, never ambient. Double-replay of a suite arm must be
  bit-identical before any number is of record.
- **Weights never ship (D14).** The backend addresses a
  user-provided llama-server; no GGUF in the repository, no
  download-at-runtime, no vendored model artifacts.

Acceptance is the F23 target the ladder has been simulating: blended
accuracy ≥ 0.80 at ≤ 1 s mean on the locked suite, measured
in-process through the real engine, not through the benchmark
harness's post-hoc blend.

## D27 — Cross-rung escalation: a configured rung list, fired only by the gate (2026-10-02)

D25 gave every rung its own gate; D26 gave the ladder its model rung.
What the F23 cascade actually does is still missing: when a rung's
gate does not accept, the question moves to the *next* rung. Today a
non-accepting outcome dead-ends in `Verify`/`Abstain` against a single
optional verifier. The decision:

- **Fallbacks are an engine-level ordered rung list, not graph nodes.**
  `Rung { classifier, calibration, policy }` (all optional beyond the
  classifier) lives in `ladder.rs` because it *is* a ladder rung — the
  same escalation vocabulary, now with an ordered tail. The graph stays
  a declarative DAG of pipeline stages; model routing is engine
  configuration, and no IR or wire shape changes. Engines assemble
  through `DecisionEngine::new_with_rungs` /
  `EngineHandle::with_rungs`; every existing constructor passes an
  empty list and is unaffected.
- **The gate is the only trigger — never two classifiers on a happy
  path.** At the threshold node the primary rung is gated exactly as
  today. Only a non-`Accept` outcome (`Verify` *or* `Abstain` — both
  mean "this rung could not decide") fires the next rung, which is
  gated by *its own* policy when it carries one, else the deciding
  node's ladder-resolved policy. The list walks until a rung accepts
  or is exhausted; the last rung's outcome stands and the existing
  verifier cascade (agree → `Verified`, disagree → `Abstain`) applies
  to the *final* distribution. Abstention at the end of the list
  remains a successful outcome, never an error.
- **Rung overrides are honest about scope.** A rung's calibration
  replaces the engine-level calibration for its own distribution (the
  D15 seam, per rung, as B4 did for gates); a rung's policy replaces
  the node-resolved policy the same way. Fallbacks decide on the full
  state over the already-narrowed question — the same inputs the
  verifier gets today — and their distributions are clipped to the
  surviving candidate set like every other classifier's.
- **Cache identity carries the rung list.** A non-empty list decorates
  the model id `|rungs-v1@<model>@<cal-ver>+…` in list order, so a
  changed composition (model swap, calibration refit, list reorder)
  re-keys every cached decision mechanically. A rung's *policy* has no
  version of its own: changing one is an artifact change under the
  same discipline D25 set for calibrations — express it by bumping the
  ladder id (or altering the composition the decoration already sees).
- **The trace explains the walk.** A question that escalated gains
  `rungs_fired` and a `rung_chain` fact naming each rung's model, top
  key, and gate outcome in order — the deterministic-explainability
  rule (§71) applied to routing, no chain-of-thought involved. An
  engine without fallbacks emits byte-identical traces, which the
  fixture suite asserts.
- **Confidence-gated, budget-aware.** Each rung firing passes the
  wave guard (deadline + cancellation) before the call, so a rung
  walk cannot outrun the request's budget.

## D28 — The OOD channel gains a policy-selected input-likeness mode (2026-10-02)

Measured motivation (REPORT, JevBench engine row; RESEARCH §9.1): the
zero-ML ladder admits 81% of adversarial items with a 0.377-accuracy
rung, and a 3-rung fusion tops out *below* its best single arm —
because the OOD channel feeding `ood_ceiling` is **normalized answer
entropy**, which measures the shape of the distribution, not whether
the input resembles anything the rung was calibrated on. The policy
seam (`Policy::with_ood_ceiling`, §19) has existed since Phase 14; it
has been receiving the wrong signal. The decision:

- **Two deterministic OOD modes, selected by policy, never fused.**
  `OodMode::AnswerEntropy` (the default — today's behavior,
  byte-identical) and `OodMode::LexicalBand`. In lexical-band mode the
  OOD score is `1 − lexical_coverage`, where coverage is the fraction
  of non-stop query tokens (the closed BM25 stop-word list, same
  tokenizer) present in the best-scoring document's term set —
  computable from the index the rung already builds, zero new
  inference, fully deterministic. Answer-entropy stays the fallback
  wherever a rung exposes no lexical evidence (relational proofs,
  the model rung), so every rung has a defined OOD value in both
  modes.
- **No-evidence is maximal OOD.** A query whose informative tokens are
  all stop words yields coverage 0 → OOD 1. A lexical rung that cannot
  lexically ground its input must escalate, not trust a softmax over
  nothing — the §73 posture, made mechanical.
- **Policy selects; the ladder composes.** `DecisionPolicy` gains
  `ood_mode` (serde default `answer_entropy`; old payloads
  deserialize unchanged), so the D25/D27 machinery overrides it per
  node, per kind, and per fallback rung like every other gate. The
  gate relationship check is unchanged (mode is orthogonal to
  `abstain_below <= verify_below <= min_confidence`). Cache identity
  is carried by the existing ladder decoration, and a policy change
  re-keys mechanically through it.
- **Raw lexical evidence leaves the classifier through the deciding
  path only.** `Classifier::decide_extended(state, question, index)`
  returns `(Distribution, Option<f64>)` — the coverage of the best
  document — with a default impl delegating to `decide` and returning
  `None`. The `index` parameter is the build-once handoff (task #80):
  the executor may pass the narrowing node's already-built index for
  the identical candidate set; classifiers remain free to ignore it
  and build their own (bit-identical either way — same documents,
  same index, same scores).
- **What this is not.** Not a trained density model, not a learned
  projector — a deterministic lexical grounding signal, the
  input-unlikeness evidence available before any model exists (the
  same doctrine the entropy placeholder's own comment states). It
  also deliberately does not change what `ood_score` means under the
  default mode: existing traces, artifacts, and calibration cells
  remain valid; only policies that opt into `lexical_band` get the
  new semantics, and they get them spelled out here.

## D29 — First-party integrations are adapters over the public surfaces, never a privileged stack (2026-10-02)

User directive (2026-10-02): OpenCodifier will integrate with Amortyx
and the other in-house services "for full stack and compatibility …
but we don't want to force people to use one stack, workflow, pipeline,
or another."

- **The public surfaces are the only surfaces.** Everything an
  in-house consumer can do — HTTP (`/v1/decide`, `/v1/batch`,
  `/v1/graph/*`, `/v1/validate`, `/v1/models`, `/v1/capabilities`),
  MCP (the six `codify_*` tools), the CLI, the Rust crates, the WASM
  runtime — is what any third party gets. A first-party integration
  may not take a back door: no in-process linkage beyond the public
  crate layering, no endpoint that only Amortyx/Control-Center can
  call, no feature that ships only through a first-party path.
- **First-party consumers are examples, not requirements.** The
  default posture stays exactly what PLANNING.md §1 states:
  fully local, offline, no accounts. Installing Amortyx, the harness,
  or any sibling service is never a prerequisite for any OpenCodifier
  capability, and no OpenCodifier capability may detect or prefer a
  first-party consumer.
- **Each side keeps its own responsibilities** (INTEGRATION_AMORTYX.md
  §1, unchanged): OpenCodifier makes semantic decisions; Amortyx makes
  economic/operational ones; neither absorbs the other. The Jev
  adapter doctrine generalizes: compatibility with any ecosystem —
  Jev/System-One, Fastino-style `systemone` callers, OpenAI or
  Anthropic clients — is an adapter mode over the IR, never the
  identity of the runtime.
- **Failure posture stays degrade-to-consumer.** A first-party
  consumer must treat the decision runtime as optional: connection
  refused, timeout, or `abstain` falls back to that consumer's own
  prior behavior (the INTEGRATION_AMORTYX.md §4 rule). No consumer's
  availability may depend on the decision runtime's availability.
- **Docs follow the same rule.** `docs/INTEGRATIONS.md` documents
  third-party integration paths as the primary story; first-party
  integrations appear in it as worked examples of the same public
  surfaces, pointing at their own design docs of record.
