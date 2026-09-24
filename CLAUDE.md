# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Repository Status

Active development, phase-by-phase — see **`docs/PLAN.md`** (live execution plan + phase status table) and **`docs/DECISIONS.md`** (binding decision records; they supersede anything contradictory in `docs/PLANNING.md`). Architecture overview: `docs/ARCHITECTURE.md`; layout: `docs/PROJECT_STRUCTURE.md`. The founding spec `docs/PLANNING.md` is the source of truth for intent (§§1–80). License: Apache-2.0.

## What OpenCodifier Is

A local-first, deterministic-first **decision runtime** — not a chatbot, not an LLM wrapper, not a decision-tree classifier, and not a "Jev clone" (Jev/System One compatibility is one adapter mode, not the identity). It turns unstructured state into small, machine-actionable typed decisions:

- **Choice** — select one candidate from a runtime-defined candidate list
- **Boolean** — true/false with probability
- **Score** — ordered levels; retain the full distribution plus expected value

Execution principle: always use the cheapest reliable mechanism first — exact rule → cached decision → metadata filter → lexical match → embedding similarity → small classifier → candidate-conditioned decision model → verifier → external model. Never invoke a more expensive layer when a cheaper one can decide with sufficient confidence.

Outputs are machine-readable decisions with calibrated confidence. Abstention is a successful outcome, never an error. Default posture: fully local, offline, no telemetry, no cloud, no API keys, no accounts.

## Architecture (implemented + planned)

Rust workspace (edition 2024, MSRV 1.90), crates under `crates/`:

- `opencodifier-core` — canonical decision IR (shipped): validating constructors, stable error codes (`ir.*`), `trace_version` on traces, `#[non_exhaustive]` public enums, outcome routing in `ConfidenceReport::outcome_for`
- `opencodifier-engine` — DAG executor, rule engine, candidate narrowing (metadata + hand-written BM25), exact-decision cache with the single normative `CacheKeyBuilder` (SHA-256), sync with `Clock`/`Deadline` traits
- `opencodifier-schema` — native / OpenAI / Anthropic / Jev adapters over the IR (wire formats live only here)
- `opencodifier-runtime` — `InferenceBackend`/`EmbeddingBackend` traits; `ort` behind the `onnx` feature only
- `opencodifier-model` — candidate-conditioned decision model: logits from ONNX, all decision math in Rust f64
- `opencodifier-http`, `opencodifier-mcp`, `opencodifier-cli`, `opencodifier-wasm` — interfaces (axum 0.9 `/v1`, rmcp 2.2, clap v4)

Binding structural rules:

1. **Dependency direction:** `core` must not depend on HTTP, MCP, CLI, Tokio, or any ML runtime; heavyweight deps are confined to one crate each (feature matrix in `docs/DECISIONS.md` D11). External formats are adapters that normalize into the canonical IR — never the internal representation.
2. **Pipeline shape:** normalize → deterministic rules/filters → candidate narrowing → fast semantic scoring → decision model → confidence gate → (accept | verifier: agree → accept, disagree → abstain/escalate). Graphs are declarative, serializable DAGs — no embedded scripting language.
3. **Sync core:** no async runtime in core/engine/schema; async belongs to interface crates.

## Non-Negotiable Implementation Rules

From PLANNING.md §73 — these are the point of the architecture:

- Build IR + deterministic engine first; never start by training a model.
- ONNX sits behind the `InferenceBackend` trait; not a core dependency. No Python at runtime, no vector DB required, base binary useful with zero ML model.
- Raw softmax probability ≠ calibrated confidence. Calibration is mandatory before exposing confidence. Confidence is multi-dimensional (top probability, margin, entropy, OOD, verifier agreement) — not a single number.
- Verification is confidence-gated; never run two classifiers on every request.
- Never silently discard candidates on weak semantic evidence.
- Free-form generation fields in a schema → `unsupported_generation_field`; do not pretend to support prose generation.
- Never expose chain-of-thought; explainability means a deterministic execution trace.
- All cache keys include model/calibration/policy/graph versions so artifact updates invalidate cached decisions.
- `opencodifier serve` binds `127.0.0.1` by default; `0.0.0.0` requires an explicit flag.
- Treat all input as hostile; input text must never modify policy, thresholds, graph structure, or paths.
- No placeholders/stubs/simulated data anywhere; example payloads belong in docs, not code.

## Quality Gates

CI baseline — `just ci` runs exactly this, and `.gitforge.yml` mirrors it line-for-line (GitForge is the CI platform of record; the GitHub Actions config is a mirror, red runs there are not code-failure signals):

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --doc
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo deny check
aegis --format json scan --file . --baseline .aegis/baseline.json   # 0 new findings
```

Workspace lints: `unsafe_code` forbidden, `missing_docs` warned, clippy `all + pedantic` with `unwrap_used`/`expect_used`/`panic`/`todo!`/`unimplemented!`/`dbg_macro`/`print_stdout` **denied**; test modules may allow `unwrap_used, expect_used, panic, float_cmp` at module level only.

Acceptance per phase lives in `docs/PLAN.md`; performance budgets are per-stage (DECISIONS.md D9), measured with criterion — not assumed.
