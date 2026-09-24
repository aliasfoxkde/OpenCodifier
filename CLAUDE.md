# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Repository Status

Greenfield — no implementation code yet. `docs/PLANNING.md` is the canonical V1 implementation plan (Phases 0–20) and the source of truth for architecture and sequencing. Read it before implementing anything; this file summarizes its binding constraints. License: Apache-2.0.

## What OpenCodifier Is

A local-first, deterministic-first **decision runtime** — not a chatbot, not an LLM wrapper, not a decision-tree classifier, and not a "Jev clone" (Jev/System One compatibility is one adapter mode, not the identity). It turns unstructured state into small, machine-actionable typed decisions:

- **Choice** — select one candidate from a runtime-defined candidate list
- **Boolean** — true/false with probability
- **Score** — ordered levels; retain the full distribution plus expected value

Execution principle: always use the cheapest reliable mechanism first — exact rule → cached decision → metadata filter → lexical match → embedding similarity → small classifier → candidate-conditioned decision model → verifier → external model. Never invoke a more expensive layer when a cheaper one can decide with sufficient confidence.

Outputs are machine-readable decisions with calibrated confidence. Abstention is a successful outcome, never an error. Default posture: fully local, offline, no telemetry, no cloud, no API keys, no accounts.

## Planned Architecture

Rust workspace, crates under `crates/`:

- `opencodifier-core` — canonical decision IR (`DecisionRequest`, `DecisionQuestion`, `DecisionAnswer`, `Candidate`, `DecisionPolicy`, `Confidence`, `DecisionTrace`)
- `opencodifier-schema` — normalization of native OC / OpenAI / Anthropic / Jev schemas into the IR
- `opencodifier-engine` — decision graph (DAG) executor, rule engine, candidate narrowing, caches
- `opencodifier-runtime` — `InferenceBackend` trait (ONNX Runtime in V1, Burn later)
- `opencodifier-model` — candidate-conditioned decision model: context encoder + candidate encoder → per-candidate scalar logit → softmax; never generates text
- `opencodifier-http`, `opencodifier-mcp`, `opencodifier-cli`, `opencodifier-wasm` — interfaces
- Also: `adapters/` (openai, anthropic, jev), `recipes/`, `skills/`, `models/`, `benchmarks/`, `fixtures/`, `tests/`

Two binding structural rules:

1. **Dependency direction:** `core` must not depend on HTTP, MCP, CLI, Tokio, or any specific ML runtime. External formats are adapters that normalize into the canonical IR — they are never the internal representation and never executed directly.
2. **Pipeline shape:** every request flows normalize → deterministic rules/filters → candidate narrowing → fast semantic scoring → decision model → confidence gate → (accept | verifier: agree → accept, disagree → abstain/escalate). Decision graphs are declarative, serializable DAGs with topological execution, parallel independent nodes, short-circuiting, and trace generation — no embedded scripting language.

Primary integration is Amortyx (the user's routing/gateway system): OpenCodifier is the *semantic decision plane*; Amortyx remains the *economic/routing plane*. Neither absorbs the other's responsibilities.

## Non-Negotiable Implementation Rules

From PLANNING.md §73 — these are the point of the architecture:

- Build IR + deterministic engine first; never start by training a model (Phases 1–5 precede any ML).
- ONNX sits behind the `InferenceBackend` trait; not a core dependency. No Python at runtime, no vector DB required, base binary useful with zero ML model.
- Raw softmax probability ≠ calibrated confidence. Calibration (temperature scaling first; measure ECE/Brier/NLL) is mandatory before exposing confidence. Confidence is multi-dimensional (top probability, margin, entropy, OOD, verifier agreement) — not a single number.
- Verification is confidence-gated; never run two classifiers on every request.
- Never silently discard candidates on weak semantic evidence (safe mode: elimination requires deterministic or strong evidence).
- Free-form generation fields in a schema → `unsupported_generation_field`; do not pretend to support prose generation.
- Never expose chain-of-thought; explainability means a deterministic execution trace (node, candidate counts, thresholds, latency, cache hit).
- All cache keys include model/calibration/policy/graph versions so artifact updates invalidate cached decisions.
- `opencodifier serve` binds `127.0.0.1` by default; `0.0.0.0` requires an explicit flag.
- Treat all input as hostile (candidate descriptions, schemas, state text, graph files, model manifests); input text must never modify policy, thresholds, graph structure, or paths.

## Quality Gates

CI baseline (PLANNING.md §40):

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
cargo test --doc
cargo deny check
```

Phase gates worth remembering:

- **Phase 1** (IR) is accepted only when round-trip/negative tests pass (Choice/Boolean/Score round trips; empty and duplicate candidates; invalid score ordering; malformed JSON) with **no ML dependency**.
- **Phase 2** (schemas): every fixture in `fixtures/<format>/` must normalize to identical canonical IR where semantics are equivalent.
- **Phase 3** (engine): complete decision-graph engine running against a `MockClassifier`, no ML.
- Performance numbers in §70 (<100 µs deterministic, sub-ms cached) are targets to benchmark, not claims to assume.
