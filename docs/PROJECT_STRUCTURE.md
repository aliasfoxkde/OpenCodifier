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
│   ├── opencodifier-engine/    # DAG executor, rules, caches, narrowing, BM25
│   ├── opencodifier-schema/    # native/OpenAI/Anthropic/Jev adapters (planned)
│   ├── opencodifier-runtime/   # InferenceBackend trait, ONNX backend (planned)
│   ├── opencodifier-model/     # candidate-conditioned decision model (planned)
│   ├── opencodifier-http/      # axum /v1 API (planned)
│   ├── opencodifier-mcp/       # rmcp server (planned)
│   ├── opencodifier-cli/       # clap CLI (planned)
│   └── opencodifier-wasm/      # browser target (planned)
│
├── fixtures/                # wire-format fixtures per adapter (planned, Phase 2)
├── recipes/                 # ready-to-use decision graphs (planned)
├── skills/                  # agent-facing usage skills (planned)
├── models/                  # model artifacts (never committed; SHA-256 verified)
├── benchmarks/              # criterion output kept per release (planned)
└── docs/                    # PLANNING.md (founding), PLAN.md (live plan), …
```

## Dependency direction (binding)

```text
cli/http/mcp/wasm ──► engine ──► core ◄── schema
                       │
                       └──► runtime ──► core      (planned)
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
