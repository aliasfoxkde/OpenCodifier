# Spec Coverage Map — PLANNING.md §§1–80 vs the Implemented Tree

Status: audit record (2026-09-28). PLANNING.md is the source of truth for
intent; `docs/PLAN.md` tracked its own 17 phases (0–16) to done — this
document is the check that every *section* of the spec has a landing
place, not just every tracked phase. Verdicts: **done** (implemented,
tested, in the default build), **partial** (some of the section shipped;
the rest named), **designed** (design of record exists, no code),
**planned** (spec'd, nothing yet).

## Foundation (§§1–8)

| § | Section | Verdict | Evidence |
|---|---|---|---|
| 1 | Mission | done | the shipped ladder; `docs/ARCHITECTURE.md` |
| 2 | Product Definition | done | all four interface crates; v0.2.0 released |
| 3 | Core Design Principle | done | cheapest-reliable-mechanism rungs, `opencodifier-engine/src/lib.rs` crate docs |
| 4 | Terminology | done | IR naming matches the spec terms |
| 5 | Canonical Decision Types | done | `Choice`/`Boolean`/`Score` in `opencodifier-core` (Score keeps distribution + EV) |
| 6 | Native Internal Representation | done | validating constructors, `ir.*` error codes, `trace_version` |
| 7 | Compatibility Architecture | done | adapters normalize into the IR; wire formats only in `opencodifier-schema` |
| 8 | V1 JSON Schema Support | done | native / OpenAI / Anthropic / Jev codecs + byte-locked fixtures |

## Engine and runtime (§§9–30)

| § | Section | Verdict | Evidence |
|---|---|---|---|
| 9 | Decision Graph | done | declarative serializable DAGs, `graph validate` |
| 10 | Deterministic Execution Engine | done | DAG executor, `Clock`/`Deadline` traits |
| 11 | Deterministic Rules | done | `RuleEngine`, facts grammar, relational solver (Phase 15) |
| 12 | Candidate Narrowing | done | metadata filter + hand-written BM25 |
| 13 | Candidate-Conditioned Classification | done | `EmbeddingClassifier` + D16 decision-model tiers behind the runtime trait |
| 14 | First ML Model | partial | candidate-conditioned scoring shipped; no trained in-house weights yet (D16 picked general small LLMs; §15/§62 are the training path) |
| 15 | Model Training Architecture | planned | benchmark harness + dataset discipline exist (Phase 13); no trainer |
| 16 | Training Data Format | planned | — |
| 17 | Calibration | done | `Calibration` trait + fitted temperature artifacts (Phase 14, D15) |
| 18 | Multi-Dimensional Confidence | done | top/margin/entropy + distributional OOD + verifier agreement |
| 19 | Two-Model Verification | done | confidence-gated verifier cascade; never two classifiers per request |
| 20 | Risk-Aware Decisions | done | `DecisionPolicy` gates (min_confidence/entropy ceiling/margin/OOD ceiling) |
| 21 | Abstention | done | successful outcome, never an error; D13 exit codes |
| 22 | Batch Inference | partial | multi-question requests + benchmark batched mode; the `/v1/batch` HTTP endpoint is not implemented (§36) |
| 23 | Cache Architecture | done | exact-decision cache, normative `CacheKeyBuilder`, version-folded keys (D6/§64) |
| 24 | Embeddings | partial | `EmbeddingBackend` trait + `EmbeddingClassifier` + benchmark bake-off (F20: ORT fp32 rung); no `embedding` graph node yet (declared, not built) |
| 25 | No Mandatory Vector Search | done | lexical path complete with zero ML |
| 26 | Reranking | planned | node kind declared in the engine docs; no implementation |
| 27 | Model Runtime | done | `InferenceBackend`/`EmbeddingBackend`; `ort` behind the `onnx` feature (D2) |
| 28 | Why ONNX + Burn | done-as-decided | D2; Burn unexamined until ONNX fails a need |
| 29 | WASM | planned | §57; no `opencodifier-wasm` crate yet |
| 30 | Tokenization | partial | deterministic bytes-over-4 estimate (D2) everywhere a budget needs one; no tokenizer — deliberate while no model arm runs in-process |

## Interfaces and delivery (§§31–39)

| § | Section | Verdict | Evidence |
|---|---|---|---|
| 31 | MCP | done | rmcp 2.2 stdio server, six `codify_*` tools (D17) |
| 32 | MCP Tool Introspection | done | rmcp schema introspection |
| 33 | Skills | done | `skills/` — README protocol + all eight §33 areas (routing, model-selection, tool-selection, tool-gating, context-pruning, escalation, verification, memory-selection) |
| 34 | Built-in Recipes | partial | `recipes/` ships 3 runnable graphs + captured responses (Phase 11); §34 lists 12 recipe areas and an `opencodifier recipe install` command that don't exist |
| 35 | CLI | done | `decide`/`graph validate`/`serve`/`models verify`/`mcp serve`, D13 exit codes, `--focus-budget` |
| 36 | HTTP API | partial | `POST /v1/decide`, `/v1/graph/validate`, `GET /v1/healthz` shipped; **not shipped:** `/v1/batch`, `/v1/graph/run`, `POST /v1/validate`, `/v1/systemone` compat, `GET /v1/models`, `/v1/capabilities` |
| 37 | Local Server | done | loopback default, explicit `--host` flag, gate-tested |
| 38 | Privacy | done | no telemetry/cloud/accounts; explicit downloads; SHA-256 manifests (D14) |
| 39 | Project Layout | done | `docs/PROJECT_STRUCTURE.md` matches the tree |

## Spec phases (§§40–59)

| § | Phase | Verdict | Evidence |
|---|---|---|---|
| 40–50 | Phases 0–10 | done | `docs/PLAN.md` rows 0–10 |
| 51 | Phase 11 — Graph Optimization | planned | no optimizer; graphs are hand-authored DAGs |
| 52 | Phase 12 — Embeddings | partial | see §24 |
| 53 | Phase 13 — Retrieval | planned | no `retrieve` node; `max_retrieval_results` policy limit exists |
| 54 | Phase 14 — Reranking | planned | see §26 |
| 55 | Phase 15 — MCP | done | Phase 10 in PLAN numbering (D17) |
| 56 | Phase 16 — Recipes and Skills | done | `recipes/` (3 runnable graphs, Phase 11) + `skills/` (§33, 2026-09-28) |
| 57 | Phase 17 — WASM | planned | crate absent; §68 (WASM security) rides on it |
| 58 | Phase 18 — CLI / Distribution | partial | CLI done; release matrix (Win/macOS/ARM64), Homebrew/winget/cargo-publish unstarted (GitForge releases exist for Linux x86_64) |
| 59 | Phase 19 — Benchmarking | done | criterion baselines per release + the Phase 13 decision-model benchmark (D16) |

## Integration, governance, examples (§§60–80)

| § | Section | Verdict | Evidence |
|---|---|---|---|
| 60–62 | Amortyx integration / shadow / training | designed | `docs/INTEGRATION_AMORTYX.md` — implementation follows its shadow-evidence promotion gate |
| 63 | Decision Registry | done | `DecisionDefinition`/`Registry` in `opencodifier-schema/registry.rs` (D20); format doc `docs/REGISTRY.md`; recipe `recipes/registry/` with a derivation-pinned request |
| 64 | Graph Versioning | done | version-folded cache keys (D6) |
| 65 | Explainability | done | deterministic execution trace, `trace_version`, no chain-of-thought |
| 66 | Security | done | hostile-input rule, loopback gate, aegis gate in CI |
| 67 | Resource Limits | done | `RequestMetadata.limits` enforced (input bytes/questions/candidates/graph nodes/time/retrieval) |
| 68 | WASM Security | planned | with §57 |
| 69 | V1 Acceptance Criteria | done | per-phase acceptance in `docs/PLAN.md`; v0.2.0 cut |
| 70 | V1 Performance Goals | done | D9 per-stage budgets, criterion-measured |
| 71 | V1.1 | planned | semantic cache, reranker, hybrid retrieval, multilingual, graph optimizer |
| 72 | V2 | planned | adaptive graphs, online calibration, distributed cache, GPU batching |
| 73 | Critical Implementation Rules | done | enforced in code + CI (see CLAUDE.md list) |
| 74 | Amortyx Router Example | designed | §74's walkthrough is the worked example inside `docs/INTEGRATION_AMORTYX.md` §3–§5 |
| 75–78 | Examples: tool selection / context pruning / skill selection / escalation | done | the four worked examples are the calling discipline of `skills/tool-selection.md`, `context-pruning.md`, `memory-selection.md` (the §77 skill-selection shape), and `escalation.md`/`verification.md` |
| 79 | Architectural Relationship | done-as-documented | the semantic-plane / routing-plane / execution-plane separation, restated in `docs/INTEGRATION_AMORTYX.md` §1 |
| 80 | Core Thesis | done | realized |

## What remains, ordered (the actionable distillation)

**CPU-light (doable on a busy host) — consumed 2026-09-29:**
2. ~~D15 full-distribution refit prep~~ **done** — the fork emits the full
   per-choice softmax (`fields.<f>.distribution`, branch
   `d15-full-distribution` in the out-of-tree llama.cpp checkout) and
   `run_llama.py` records it as per-item `probs`; rebuild + validation wait
   for a quiet-host window, after which calibration refits beyond
   winner-vs-rest and the fork_4b JevBench arm upgrades to native Brier/ECE.
3. ~~#25 MBLI arm design~~ **done** — `benchmarks/decision-model/NATIVE_VERDICT_ARM.md`
   + `runner/run_jev_native.py` (verdict-slot readout, manifest-verified
   reference runtime); first run queued behind the params A/B.
4. ~~#39 JevBench methodology~~ **done** — `benchmarks/decision-model/JEVBENCH.md`
   + `runner/run_jevbench.py`; engine arm complete (231/231, acc 0.3766,
   p50 2.0 ms, det 231/231 — `results/REPORT.md` external-anchor section),
   bridge arm in replay, fork_4b pending.

**Needs the host quiet (cargo builds/tests):**
5. **§36 HTTP surface completion** — `/v1/batch`, `/v1/graph/run`, `/v1/validate`, `/v1/systemone`, `/v1/models`, `/v1/capabilities`.
6. **§34 recipe fleet + `recipe install`** — the twelve recipe areas.
7. **§63 Decision Registry** — artifact format + loader.
8. **§51 graph optimizer**, **§53 retrieval node**, **§26/§54 rerank node** (with §24's `embedding` node).

**Larger efforts (own phases):**
9. **§57+§68 WASM** (`opencodifier-wasm`, browser demo, WASM security).
10. **§58 distribution matrix** (cross-platform releases, Homebrew/winget/crates.io).
11. **§15/§16 training pipeline** (with §62's shadow-ledger dataset discipline).
12. **§60 implementation** — Amortyx integration per the promotion gate.
