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
├── infrastructure/
│   └── docker/ci-rust.Dockerfile  # runner-local CI image recipe (`just ci-image`)
│
├── scripts/                 # single-purpose validation + release tooling
│   ├── e2e_validate.py          # release-binary e2e over the real wire (PLAN 18d)
│   ├── coverage_floor.py        # lcov DA-basis floor assertion (CI coverage lane)
│   ├── generate_attestation.py  # verified per-artifact build attestation (schema opencodifier.attestation/1)
│   └── generate_release_notes.py# renders notes from attestations + docs/RELEASE_NOTES_TEMPLATE.md
│
├── dist/                    # release staging (gitignored): matrix artifacts,
│                            # attestations, rendered notes (`just release-matrix`)
│
├── crates/
│   ├── opencodifier-core/      # canonical decision IR (done)
│   ├── opencodifier-schema/    # native/OpenAI/Anthropic/Jev adapters (done)
│   ├── opencodifier-engine/    # DAG executor, rules, caches, narrowing, BM25,
│   │                           #   relational solver, ladder + calibration seams (done)
│   │                           #   facts.rs = the extraction grammar itemgen renders against
│   ├── opencodifier-itemgen/   # D37 relational/contrastive item generator:
│   │                           #   sample → render → solver-verify (binary `itemgen`;
│   │                           #   corpus emission only, never a serving dependency)
│   ├── opencodifier-runtime/   # InferenceBackend traits, mocks; `onnx` deferred by gate (done, D2)
│   ├── opencodifier-model/     # decision serving contract + embedding classifier + manifests (done)
│   │     src/decision.rs, embedding.rs, manifest.rs — serving contract
│   │     src/kai.rs + kai/     — kai contract + committed fixtures
│   │     src/julia.rs + julia/ — julia contract + committed fixtures
│   │     src/nli.rs + nli/     — NLI verbalization verifier contract + fixture (PLAN 24j, `nli` feature serves it)
│   │     src/render.rs         — shared answer-key/question-text rendering (llamacpp + nli)
│   │     src/llamacpp.rs       — llama.cpp loopback backend (D26, `llamacpp` feature)
│   ├── opencodifier-http/      # axum 0.8 /v1 API (done)
│   ├── opencodifier-cli/       # clap CLI binary `opencodifier` (done)
│   ├── opencodifier-mcp/       # rmcp 2.2 stdio server, six codify_* tools (done)
│   ├── opencodifier-wasm/      # wasm32 binding: WasmEngine over the zero-ML stack (done; D23)
│   └── opencodifier-ffi/       # C-ABI staticlib over the same wire module (done; D24)
│
├── fixtures/                # wire-format fixtures per adapter (done, byte-locked)
├── recipes/                 # runnable decision graphs + captured responses (done)
├── skills/                  # agent-facing usage skills: 8 §33 areas (done)
├── models/                  # model artifacts (never committed; SHA-256 verified;
│                            #   created on demand by the models/CLI path)
├── benchmarks/              # per-release criterion baselines (first committed: D9, 2026-10-01)
│   ├── baselines/criterion/ # engine + http d9-baseline estimates (committed from B1 on)
│   ├── decision-model/      # Phase 13 model-pick benchmark: byte-locked suite, arms, results
│   │     results/REPORT.md  # comparison page of record; docs/BENCHMARKS.md links here
│   └── validation/          # validation-campaign record (#104; VALIDATION.md summary)
│       └── runner/          # compute-host parity runners (own workspaces; ORT via
│                            #   runtime-onnx-dynamic + ORT_DYLIB_PATH): kai-parity-rs,
│                            #   julia-parity-rs, nli-parity-rs (PLAN 24j)
├── site/                    # static marketing page (Cloudflare Pages direct-upload;
│                            #   Tailwind runs as a dev-time pipeline only —
│                            #   `npm run build:css` writes the committed tw.css)
│   ├── maze-core.js         # maze demo pure logic (seeded RNG, generation,
│   │                        #   Bresenham FOV, bot ladder, entities, rubric
│   │                        #   scoring) — DOM-free, node-testable
│   ├── maze.js              # maze demo shell: DPR canvas renderer, input,
│   │                        #   rAF loop, modes (bot/play/versus), session
│   │                        #   leaderboard, rubric editor, WASM tie-breaks
│   └── tests/               # node --test suites (board page + maze core/shell/deep-link)
└── docs/                    # PLANNING.md (founding), PLAN.md (live plan), …
                             #   RELEASE_NOTES_TEMPLATE.md is the release-notes
                             #   skeleton; `just release-notes` fills its
                             #   mechanical slots from the dist/ attestations
```

## Dependency direction (binding)

```text
cli ──► http ──┐
               ├──► engine ──► core ◄── schema
model ──► runtime ──► core
        └────────► engine
itemgen ──► engine ──► core   (itemgen also declares the direct core edge)
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
