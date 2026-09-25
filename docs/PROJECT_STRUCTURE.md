# Project Structure

Living document — updated as phases land. The planned layout comes from
PLANNING.md §39; crates marked *(planned)* do not exist yet.

## Workspace layout

```text
opencodifier/
├── Cargo.toml               # workspace: shared lints, deps, release profile
├── rust-toolchain.toml      # local dev channel (stable)
├── rustfmt.toml             # stable-only settings
├── deny.toml                # cargo-deny: licenses/advisories/bans/sources
├── justfile                 # task runner; `just ci` = the pre-push contract
├── .gitforge.yml            # GitForge CI — the pipeline of record
├── .github/workflows/ci.yml # GitHub mirror config (not a failure signal)
├── .aegis/baseline.json     # aegis pattern-scan baseline (new findings = failures)
│
├── crates/
│   ├── opencodifier-core/      # canonical decision IR (done)
│   ├── opencodifier-schema/    # native/OpenAI/Anthropic/Jev adapters (done)
│   ├── opencodifier-engine/    # DAG executor, rules, caches, narrowing, BM25 (done)
│   ├── opencodifier-runtime/   # InferenceBackend traits, mocks; `onnx` deferred by gate (done, D2)
│   ├── opencodifier-model/     # decision serving contract + embedding classifier + manifests (done)
│   ├── opencodifier-http/      # axum 0.8 /v1 API (done)
│   ├── opencodifier-cli/       # clap CLI binary `opencodifier` (done)
│   ├── opencodifier-mcp/       # rmcp server (planned)
│   └── opencodifier-wasm/      # browser target (planned)
│
├── fixtures/                # wire-format fixtures per adapter (done, byte-locked)
├── recipes/                 # runnable decision graphs + captured responses (done)
├── skills/                  # agent-facing usage skills (planned)
├── models/                  # model artifacts (never committed; SHA-256 verified)
├── benchmarks/              # per-release criterion baselines (done for v0.1.0)
└── docs/                    # PLANNING.md (founding), PLAN.md (live plan), …
```

## Dependency direction (binding)

```text
cli ──► http ──┐
               ├──► engine ──► core ◄── schema
model ──► runtime ──► core
        └────────► engine
(mcp/wasm, when they land, take the same engine-facade edge as cli/http)
```

`opencodifier-core` must never depend on HTTP, MCP, CLI, Tokio, or any ML
runtime (PLANNING.md §39). Heavyweight dependencies are each confined to
exactly one crate — see `docs/DECISIONS.md` (feature-flag matrix).

## In-crate conventions

- Modules: `snake_case.rs`; one concern per module; module docs cite the
  PLANNING.md § they implement.
- Tests: unit tests in `#[cfg(test)]` modules (allow list:
  `clippy::unwrap_used, clippy::expect_used, clippy::panic,
  clippy::float_cmp`), integration tests in `tests/`, property tests with
  `proptest`, benchmarks in `benches/` (`criterion`, `harness = false`).
- Errors: one error enum per crate, `thiserror`-derived, with a stable
  `code()` string (see `crates/opencodifier-core/src/error.rs` for the
  pattern). `anyhow` is banned from public APIs.
- Every public item is rustdoc-documented; `missing_docs` + CI
  `-D warnings` make that enforceable.
