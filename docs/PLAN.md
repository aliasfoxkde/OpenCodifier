# Execution Plan (live)

The phased execution contract for reaching a usable OpenCodifier v0.1.
PLANNING.md is the founding specification (§§1–80); this document is the
**live, expanded** plan that turns it into ordered, verifiable phases and
records honest status. Where the two disagree, this file and
`docs/DECISIONS.md` win.

## 0. Honest assessment (audit result)

A critical audit of PLANNING.md against the 2026-Q3 ecosystem produced 34
findings; the material ones, resolved here:

1. **Dependency reality** — fastembed/tantivy are unusable for this
   project (D4); ort/rmcp need exact pins (D1/D2). PLANNING.md's dep
   mentions were directional, not versions.
2. **§70 perf budget was untestable** — a single end-to-end number can't
   localize a regression. Replaced by per-stage criterion budgets (D9).
3. **Async was a hidden coupling** — §44 implied Tokio everywhere. The
   core is now explicitly sync with trait-based time/cancellation (D5).
4. **Cache key had no normative home** — every crate would invent one.
   Now single-site (D6).
5. **Contract freezing had no schedule** — versioning rules existed but
   nothing forced them before consumers exist. trace_version,
   non_exhaustive, and error codes shipped in Phase 1 (D8).
6. **Calibration was underspecified** — where parameters live, how they
   version, what un-calibrated reports look like (D15).
7. **No wire fixtures** — adapter correctness needs byte-locked
   fixtures, not just round-trip property tests (D10, Phase 2).
8. **Security posture was prose, not config** — local-bind default,
   `--allow-remote` gate, model SHA-256 manifests are now concrete
   (D12/D14, SECURITY.md).
9. **WCAG 2.1 AAA applies only where there is UI** — for v0.1 that is
   the CLI (screen-reader-clean `--help`, no color-only signals) and the
   HTML rustdoc; the browser target inherits it when it lands.

Remaining risks tracked in §6 below.

## Phase status

| Phase | Scope | Status |
|-------|-------|--------|
| 0 | Workspace scaffold, governance, CI configs | **done** (37b8b63) |
| 1 | `opencodifier-core` canonical IR | **done** (4f8900c) |
| 2 | `opencodifier-schema` adapters + fixtures | **done** (5952769) |
| 3 | `opencodifier-engine` graphs + rules + caches | **done** (5952769) |
| 4 | Engine narrowing (metadata + BM25) | **done** (5952769) |
| 5 | Deterministic decision model + calibration interface | **done** (5952769) |
| 6 | `opencodifier-runtime` trait + honest ONNX feasibility | **done** (5146c34) — gate measured: a FAIL / b FAIL / c PASS → `onnx` deferred (D2) |
| 7 | `opencodifier-model` (candidate-conditioned scoring) | **done** (d3a5aa5) — `EmbeddingClassifier`, `ModelManifest`, serving contract; no weights in V1 |
| 8 | `opencodifier-cli` | **done** — `decide`/`graph validate`/`serve`/`models verify`, D13 exit codes, 20 e2e tests |
| 9 | `opencodifier-http` | **done** — axum **0.8** `/v1` (D12 amended), loopback gate, 26 tests incl. real-socket e2e |
| 10 | `opencodifier-mcp` | pending |
| 11 | E2E recipes + docs book + examples | pending |
| 12 | Release engineering (tag, release, GitForge pipeline green) | pending |

## Phase 2 — schema adapters + fixtures (done, 5952769)

- `opencodifier-schema` crate: `native` (identity), `openai`
  (strict structured outputs — enum / anyOf / minimum-maximum only;
  oneOf/const treated as unsupported), `anthropic` (tool input_schema),
  `jev` (System One: `state`/`model`/`questions`; score answer =
  probability-weighted mean index with `"0".."n-1"` keys; `noul` ≥ 0.5 →
  boolean true).
- Error enum `SchemaError` with `code()` (`schema.*`), unknown-field
  tolerance for wire formats.
- Fixtures committed under `fixtures/`:
  `jev/systemone-{choice,score,noul}-request.json`,
  `openai/{choice-enum,score-bounded-integer,boolean}.format.json`,
  plus Anthropic tool-schema equivalents.
- **Accept:** property round-trip native⇄wire for each format; fixtures
  byte-locked; adapters total (no panics on any input ≤ limits);
  clippy/fmt/test/deny clean; coverage of the crate ≥ 99% lines.

## Phase 3–5 — engine (done, 5952769)

- DAG executor over `max_graph_nodes 128`, topological, cycle-checked.
- Deterministic rules: fact equality/range/set membership, text
  contains/regex (bounded), producing full distributions or hard
  filters; every rule execution traced.
- Exact-decision cache via `CacheKeyBuilder` (D6) + LRU bounded by
  configured byte budget, hit recorded in `DecisionMetrics`.
- Narrowing: metadata filters first, BM25 (hand-written, D4) second,
  budget-driven stop; subset-safe under proptest.
- Calibration trait + identity calibration (D15).
- **Accept:** criterion benches for D9 table (rule/cache/BM25 rows);
  property tests pass 10k cases; full trace emitted per run.

## Phase 6 — runtime abstraction + honest ONNX feasibility (done)

- `InferenceBackend` / `EmbeddingBackend` traits in
  `opencodifier-runtime`, sync (D5), `Debug + Send + Sync`, returning
  validated `f32` `DenseTensor`s — logits/vectors only (D7). Mock
  backends (scripted inference, deterministic FNV-1a embedder) keep the
  whole product runnable with zero ML.
- **Honest feasibility gate — MEASURED 2026-09-24 (probe
  `/nas/Temp/tmp/oc-ort-probe`, four real dylibs, 10k seeded inputs):
  the `onnx` feature does not ship in V0.1.**
  - (a) `api-27` load — **FAIL**: ort rc.13 demands ONNX Runtime
    1.27.x; available dylibs are 1.21.0 (system) and 1.22.1/1.24.1/1.24.2
    (Python wheels). Loadable only at `api-21`/`api-24`.
  - (b) bit-identity vs Rust softmax — **FAIL**: max 5 ULP (~3.3e-7
    rel) agreement, never bit-identical; ORT flushes subnormals below
    ~1e-38. Run-to-run ORT is deterministic (0/640k differed).
  - (c) latency — **PASS**: p99 31-61 us vs the 5 ms budget (~100x
    headroom).
  - Consequence: no ort dependency is added; deterministic path is the
    only path; re-entry conditions recorded in DECISIONS.md D2
    (api-21/24 build, explicit `ORT_DYLIB_PATH`, 5-ULP tolerance in
    verifier/calibration comparisons, `Session::run` is `&mut self`).
  - **No simulated results were produced or needed: every number above
    is measured on this machine.**

## Phase 7 — model crate (done, this commit)

- `opencodifier-model`: candidate-conditioned scoring over
  `InferenceBackend` (`decision.rs`: named-tensor contract
  `context`/`candidates`/`logits`, shape-validated, never truncated;
  Rust-side softmax per D7) and embedding similarity classification
  (`embedding.rs`: any `EmbeddingBackend` → engine `Classifier`,
  cosine + stable softmax, temperature is scoring shape — not
  calibration; zero-vector → 0.0, never NaN; malformed backend answers
  are typed `engine.classifier_failed` failures, never guesses).
  SHA-256 `ModelManifest` + artifact verification (D14).
- **Coverage ledger (2026-09-24, clean instrumented re-measure):**
  7,604 instrumented lines, **73 uncovered = 99.04% line coverage**
  (lcov `DA:…,0` ground truth; the llvm-cov summary's 98.58% counts
  duplicate codegen regions and is not the ledger basis). Every
  uncovered line is in a documented-unreachable category with an
  in-source justification: 50 engine lines (mutex-poisoning arms,
  `#[non_exhaustive]` catch-alls, assertion-shadowed fallbacks), 10
  schema lines (`#[non_exhaustive]` catch-alls), 13 model lines (3
  defensive `Distribution::from_pairs` closures after softmax — output
  is normalized by construction — and the `#[non_exhaustive]`
  question-kind arm).

## Phases 8–9 — interfaces (done)

- `opencodifier-engine::EngineHandle` — the one facade (D5 sync):
  `lexical` zero-ML constructor, `decide`/`decide_with_report`,
  `validate_graph`, `health`. No interface reimplements pipeline logic.
- `opencodifier-cli` (D13): `decide`
  (`--input`/`--format native|openai|anthropic|jev`/`--policy`/`--trace`/
  `--abstain-is-success`), `graph validate`, `serve`
  (`--bind 127.0.0.1:8177` default, `--allow-remote` gate,
  `--graph`), `models verify` (D14 digest gate). Exit codes 0 accept /
  1 input / 2 policy-gate escalation / 3 internal — **any non-decisive
  outcome** is exit 2 (the honest split `DecisionOutcome` supports),
  and clap's own exit 2 for bad args is normalized to 1 so scripts can
  trust the table. Responses always canonical native JSON; `--trace`
  prints the CLI-owned execution-report projection (`RunReport` is not
  `Serialize`; recorded as a possible engine follow-up).
- `opencodifier-http` (D12, **axum 0.8** — 0.9 does not exist; D2-style
  pin correction recorded): `POST /v1/decide`,
  `POST /v1/graph/validate`, `GET /v1/healthz`; `{"error":{code,message}}`
  envelope with owning-crate codes; 400 input vs 500 engine faults;
  1 MiB body cap at the socket; loopback-by-default bind with
  `allow_remote` opt-in; sync engine off the async workers via
  `spawn_blocking`; abstention is 200.
- `serve --policy` validates the file then reports
  `cli.policy_inapplicable`: policy lives on the request in this IR,
  and pretending a server-wide policy override existed would be a lie.
- **Accept (met):** choice + score + abstain e2e through both surfaces
  — CLI via `assert_cmd` on the real binary, HTTP via reqwest over a
  real socket; abstention forced through a high-threshold `DecisionPolicy`
  carried in the request (the IR's own mechanism, no special casing).
  The serve loop is additionally lifecycle-tested in-process:
  `serve_with_shutdown` takes the shutdown future as a parameter, so
  bind → accept → request → graceful shutdown runs under the test
  runtime (the spawned-binary e2e covers the same loop at process
  level, but a child's profraw never merges into the coverage report).
- **Coverage ledger (2026-09-25, clean instrumented measure):**
  8,340 instrumented lines, **83 uncovered = 99.00% line coverage**
  (lcov `DA:…,0` ground truth; the llvm-cov summary overcounts
  duplicate codegen regions and is not the ledger basis). Every
  uncovered line is in a documented-unreachable category with an
  in-source justification: engine 44 (mutex-poisoning arms,
  `#[non_exhaustive]` catch-alls, assertion-shadowed fallbacks,
  divide-by-zero guards), schema 10 (`#[non_exhaustive]`
  catch-alls), model 13 (defensive `Distribution::from_pairs`
  closures after normalized softmax + the non_exhaustive
  question-kind arm), interfaces 16 (CLI `run()` process shell and
  the `ctrl_c` await — process-lifetime lines covered by the e2e
  suites whose profraw files cannot merge — and the defensive
  encode-error closure on engine-produced responses).
- Phase 10 (MCP) remains planned; D1's `rmcp 2.2` pin is stale
  (latest is 3.4.x) and will be corrected when that phase starts.

## Phase 11 — docs + recipes

- README current; `docs/` set complete (this file, ARCHITECTURE,
  DECISIONS, PROJECT_STRUCTURE, SECURITY, CONTRIBUTING); rustdoc
  exported; `recipes/` with ≥3 runnable decision graphs and their
  fixtures-derived expected outputs.
- Accessibility: CLI `--help` clean under a screen reader, rustdoc
  structure passes heading-order checks.

## Phase 12 — release

- `just ci` green locally; the same gates validated through the
  harness-jobs queue (`cargo-fmt`, `cargo-clippy`, `cargo-test`,
  `cargo-coverage` profiles at this cwd) — that is the CI validation of
  record for this build.
- **GitForge finding (2026-09-24):** pipeline **creation** is a stub in
  the current GitForge build (`pipeline --create` reports "not yet
  implemented"; `PipelineQueries::create` has no API caller; pushes do
  not auto-create pipelines from `.gitforge.yml`). `.gitforge.yml`
  remains the pipeline definition of record for when the endpoint lands;
  until then, gate parity is maintained by keeping it line-for-line
  equivalent to `just ci`. GitHub mirror status ignored if red
  (billing-blocked account, per harness policy).
- cargo-deny + cargo-audit clean; aegis scan: 0 new findings vs
  baseline; coverage ≥ 99% lines workspace-wide.
- SemVer tag `v0.1.0`, GitHub release notes from the changelog, push
  origin + gitforge.

## Quality gates (every phase, no exceptions)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace && cargo test --doc
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo deny check
aegis --format json scan --file . --baseline .aegis/baseline.json   # 0 new
```

## 6. Risk register

| Risk | Impact | Mitigation |
|------|--------|------------|
| ort rc API churn | runtime crate rework | exact pin (D2), feature-gated, feasibility gate in Phase 6 |
| No embedding model without downloads | semantic layer weaker locally | `EmbeddingBackend` is caller-supplied; lexical path is complete without it |
| BM25 correctness drift | wrong narrowing | property tests vs hand-computed IDF/tf; fixtures |
| MCP beta churn | server rewrite | stable 2025-11-25 via rmcp 2.2 only (D1) |
| Coverage dip while scaffolding crates | gate failure | each phase lands with its tests; coverage task runs per phase, not once at the end |
| GitHub mirror red | noise, not signal | harness policy: GitForge is the pipeline of record |
