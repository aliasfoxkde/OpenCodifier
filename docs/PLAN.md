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
| 10 | `opencodifier-mcp` | **done** (2026-09-28) — rmcp **2.2.0** stdio server, §55 tool set, 9 client-driven e2e + 2 CLI stdio sessions (D17) |
| 11 | E2E recipes + docs book + examples | **done** (85cd2ec) — `recipes/` 3 runnable graphs + captured responses; docs set + accessibility checked |
| 12 | Release engineering (tag, release, GitForge pipeline green) | **done** (49bc224) — tag v0.1.0; pipeline of record green through GitForge (fe4b0871); v0.1.1 CI hardening (3bad2bb, run 8e8401f0); v0.2.0 with Phases 10/13/14/15 (c1c8166, tag v0.2.0, run 0ebd7318; CI-toolchain clippy fix c09e39d after run aaa2eefa) |
| 13 | Decision-model benchmark: pick the model | **done** — `benchmarks/decision-model/`, thirteen arms; Qwen3.8-4B-Distill reference pick, tiered alternatives (D16, amended ×2) |
| 14 | Confidence truthfulness: D15 calibration + §19 gates | **done** (2026-09-28) — `Calibration` trait + fitted temperature artifacts for the tier arms; the dead OOD channel is live and policy-gated |
| 15 | §43 relational solver: exact proofs over extracted facts | **done** (2026-09-28) — `facts` + `relational` in the engine, default zero-ML stack; engine arm 0.483 → 0.683 @ 1.3 ms, relational 0.950 (D16 ×8) |
| 16 | §45 focused-question extraction + reverse escalation | **done** (2026-09-28) — `focus` in the engine, `--focus-budget` on the CLI; long-suite A/B answer-identical on 120/120, views p50 93 tokens (D18) |

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
- Phase 10 (MCP) shipped against D1's `rmcp 2.2` pin as written
  (2.2.0); rmcp 3.x is a major-version line and is *not* adopted here —
  the upgrade, if ever, is its own decision record (D17).

## Phase 11 — docs + recipes (done)

- README current; `docs/` set complete (this file, ARCHITECTURE,
  DECISIONS, PROJECT_STRUCTURE, SECURITY, CONTRIBUTING); rustdoc
  exported.
- `recipes/` (done): three runnable decision graphs with their paired
  requests and captured responses — `minimal-choice` (smallest graph
  that can decide a choice question; the `filter` node is mandatory
  because choice questions are decided only over narrowed candidates),
  `strict-verify` (full ladder, 0.95 gate → the refusal is the
  product), `boolean-score` (no choice node; lexical evidence below
  `abstain_below` → abstain as HTTP 200). All responses are captured
  by execution, byte-reproducible across fresh serves (the cache node
  makes a *second* identical request report `cache_hit: true`, so
  comparisons use a fresh process — documented in `recipes/README.md`).
- Accessibility (checked 2026-09-25): CLI `--help` is plain text with
  zero ANSI escapes when piped, structured exit-code and diagnostics
  sections, and every subcommand's `--help` exits 0. Rustdoc heading
  order verified across all 67 generated pages: no authored content
  skips; the single h1→h3 jump is in rustdoc's own upstream `all.html`
  template (not crate-controlled).

## Phase 12 — release

- `just ci` green locally is the validation of record for this build.
- **Harness-jobs finding (2026-09-25) — RESOLVED at the source:** the
  queue rejected this cwd outright (`submit_job` accepted only
  backend-fixed, GitForge, dsc, BigData, Amortyx*). Fixed in the
  harness itself (backend-fixed `053264d`): `/nas/Temp/repos/OpenCodifier`
  is now an allowed root, with allowlist tests extended (the suite also
  had a stale assertion claiming `dsc` is refused after DSC_ROOT became
  an allowed root). The four profiles (`cargo-fmt`, `cargo-clippy`,
  `cargo-test`, `cargo-coverage`) are now the validated path for gate
  evidence; results recorded below as they complete.
- **GitForge finding (2026-09-24) — RESOLVED at the source:** pipeline
  **creation** was a stub (`pipeline --create` printed "not yet
  implemented"; `PipelineQueries::create` had no API caller). Also
  discovered: GitForge's rename commit (acc45f49) missed the pipeline
  filename, so CI read `.gitforce.yml` while repositories (including
  OpenCodifier) commit `.gitforge.yml`. Fixed in GitForge: the gateway
  now exposes `POST /api/pipelines` (validated registration with the
  single-active-version invariant), `POST /api/pipelines/{id}/runs`
  (revision-resolved hand-off to the CI orchestrator), and
  `DELETE /api/pipelines/{id}` (delete when runless, deactivate when
  history exists); the CLI `pipeline create/run/watch/delete` verbs are
  implemented against them; and the committed-definition loader accepts
  `.gitforge.yml` first with the legacy `.gitforce.yml` spelling as a
  fallback. Covered by gateway route tests + ci-service loader tests.
- **Aegis finding (2026-09-25) — RESOLVED at the source:** baseline
  refreshes self-polluted because the scanner read `.aegis/baseline.json`
  during the scan and baked its own matches into the fresh artifact
  (2,721 → 13,143 entries, ~80% self-referential). Fixed in aegis
  itself (aegis `fix/state-dir-self-scan`, 445cf4c, 0.6.3): the
  `.aegis` state directory is now a built-in exclusion like
  `node_modules`/`target`/`.git`, across directory, single-file, and
  `--staged` scans (via the new `Scanner::should_ignore`). The
  out-of-tree regeneration dance and its memory note are retired. The
  committed baseline was regenerated in tree with the fixed binary:
  **1,708 entries, zero self-referential** (was 1,707 mostly polluted),
  and the documented gate command reports **0 new findings**. One
  config lesson recorded: generation and gate must use the same
  `--config` (this repo's gate runs the default profile — production
  would report ~1,667 phantom findings).
- **CI-image finding (2026-09-25) — RESOLVED:** the first real pipeline
  run (8185900a) failed the supply-chain lane at `cargo install --git
  http://127.0.0.1:42782/...aegis.git` — connection refused. The root
  cause is host networking, not a wrong address: job AND build containers
  cannot reach host services at all (the host firewall INPUT chain is
  default-DROP for docker-sourced traffic — only DNS/53 is accepted;
  verified from a bridge-network container against the 172.17.0.1 gateway
  too), so no URL to the git-server can work from inside a job. Fixing it
  exposed two further defects: every lane compiled its tools at job
  runtime (against the runner's own F25 baked-in-tools contract), and
  `rust-toolchain.toml` (`channel = "stable"`) made every job silently
  download the latest stable on the stock `rust:1.90` image (measured:
  1.98.1) — CI never ran the pinned 1.90. The fix follows the GitForge
  house pattern (`dsc-ci-rust`): a committed image recipe
  (`infrastructure/docker/ci-rust.Dockerfile`, built by `just ci-image`,
  runner-local tag `opencodifier-ci-rust:1`) bakes the toolchain
  (`RUSTUP_TOOLCHAIN` — verified to override the rust-toolchain.toml
  file), rustfmt/clippy (absent from the stock rust image), cargo-deny
  0.20.2, cargo-audit 0.22.2, aegis at pinned rev `445cf4c` (the exact
  binary the committed baseline was generated with, vendored as
  build-context source because it is not on crates.io), and a warm crate
  registry. All four jobs now run with zero runtime installs; the full
  supply-chain lane was rehearsed inside the image against the working
  tree (deny / audit / aegis / fmt all exit 0) before the pipeline ran
  it. The scan of the new files triaged 24 findings into the baseline
  (1,709 → 1,729 entries; 4 dead entries removed): 3 line shifts + 1
  fence-content change in PROJECT_STRUCTURE.md, and 20 detector misfires
  on CI prose (loopback topology documentation, `timeout:` fields,
  tool/file-name mentions) — each appended with a triage note.
  First pipeline run on the image (f1153de0): lint, test, and doc
  succeeded on the baked image; supply-chain failed — correctly. The
  paragraph recording this finding had been added to this file after the
  local gate last ran, and its loopback/IP documentation tripped five
  detector patterns plus one line-shifted fence: the pipeline caught
  what a post-edit local gate would have caught. Discipline correction
  recorded: the aegis gate runs after every content edit, not only
  before "the final" one. The six findings were triaged into the
  baseline (1,729 → 1,734; dead :259 fence entry removed).
- **Pipeline of record green (2026-09-25, run fe4b0871 on 3f38113):**
  lint, test, doc, and supply-chain (cargo-deny, cargo-audit, aegis
  baseline gate) all succeeded inside the baked image. Getting there
  surfaced one more platform finding: a newer platform release deployed
  from un-merged main silently dropped the pipeline-management endpoints
  (PR #236 still open), so the CLI's run-trigger route reports not-found
  against it; the push-webhook and internal trigger paths were
  unaffected, and manual runs went through the CI service's own
  push-event endpoint (the same typed path webhooks use). Durable fix:
  merge PR #236 to main before the next platform release cut.
  Harness-jobs: the fmt profile ran green through the queue (first
  OpenCodifier gate validated there); clippy/test/coverage remain
  queue-blocked by box load — local `just ci` and the GitForge pipeline
  cover the same gates meanwhile.
- cargo-deny + cargo-audit clean; aegis scan: 0 new findings vs
  baseline; coverage ≥ 99% lines workspace-wide.
- SemVer tag `v0.1.0`, GitHub release notes from the changelog, push
  origin + gitforge.

## Phase 13 — decision-model benchmark (done)

Question: before any weights are trained, can a small local general model
serve the candidate-conditioned decision layer — and which one? Answered
with measurement, not opinion.

- **Harness** (`benchmarks/decision-model/`): a committed byte-locked
  suite (`suite/suite.json`, generator `generate_suite.py`, seed
  20260926, 120 Choice questions in canonical IR shape with ground
  truth by construction — metadata_match / lexical_semantic /
  relational_compositional, 40 each), three arms, and a summarizer.
  Weights never enter the repo; `results/models.manifest.json` pins
  every artifact's SHA-256 (D14 practice).
- **Design sources** (operator-supplied): the Jev analysis at
  `docs/References/Can Your GPU Hit Jev's Milliseconds Mark.txt` and
  thecodacus/llama.cpp branch `parallel-decision` (reviewed at
  `ad129b0`). The llama arm drives its `POST /v1/decision`: candidate
  ids scored as token paths forked from one cached prefix, tree mode =
  log-softmax at each divergence node, product along the path,
  renormalized over candidates — the exact constrained distribution,
  nothing sampled. Distinct-token-path candidates are hard-rejected by
  the branch (throws), duplicate-id suites would not pass silently.
- **Results** (CPU-only host — no GPU here, so absolute latency is
  CPU-scale; ranking and calibration are the deliverable; full table in
  `benchmarks/decision-model/results/summary.md`):

  | arm | acc (meta/lex/rel) | acc | ECE | p50 |
  |---|---|---|---|---|
  | engine builtin-lexical | 0.88 / 0.23 / 0.35 | 0.483 | 0.115 | 5.3 ms |
  | MiniLM-L6 zero-shot | 0.30 / 0.62 / 0.35 | 0.425 | 0.172 | 107 ms |
  | Qwen2.5-0.5B | 0.60 / 0.33 / 0.25 | 0.392 | 0.218 | 695 ms |
  | Llama-3.2-1B | 0.48 / 0.35 / 0.35 | 0.392 | 0.265 | 997 ms |
  | Qwen2.5-1.5B | 0.90 / 0.45 / 0.40 | 0.583 | 0.191 | 1.25 s |
  | Qwen2.5-3B | 0.90 / 0.75 / 0.38 | 0.675 | 0.263 | 2.58 s |
  | Gemma-3-4b-it | 0.95 / 0.85 / 0.45 | 0.750 | 0.236 | 3.04 s |
  | Qwen3.5-0.8B | 0.93 / 0.68 / 0.35 | 0.650 | 0.074 | 613 ms |
  | Qwen3.5-2B | 0.95 / 0.72 / 0.50 | 0.725 | 0.062 | 1.73 s |
  | MiniCPM5-1B | 0.95 / 0.35 / 0.15 | 0.483 | 0.232 | 694 ms |
  | **Qwen3.8-0.8B-Distill** | 0.85 / 0.55 / 0.30 | 0.567 | 0.098 | 808 ms |
  | Qwen3.8-2B-Distill | 0.90 / 0.62 / 0.33 | 0.617 | 0.095 | 1.34 s |
  | **Qwen3.8-4B-Distill** | **1.00 / 0.95 / 0.35** | **0.767** | **0.057** | 3.76 s |

- **Extension (2026-09-26).** Three newer models typed through the
  identical harness moved the pick (D16 amended): **Qwen3.5-2B** —
  within noise of Gemma's accuracy (0.725 vs 0.750 = 3 items of 120) but
  decisively better where the runtime consumes: best-calibrated of all
  ten arms (ECE 0.062 vs 0.236), best relational score (0.50), half the
  parameters, ~57% of the latency. **Qwen3.5-0.8B** is the measured fast
  tier: 0.650 @ 613 ms — more accurate than every Qwen2.5 model at or
  below their latency. **MiniCPM5-1B** is rejected: 0.483 ties the
  zero-ML engine at ~130× its latency and its chat baseline collapses
  (0.092) — metadata-tilted (0.95) but no semantic or relational skill.
  The chat-arm re-measurement also surfaced a thinking-mode pitfall now
  fixed in the runner: Qwen3.5/MiniCPM5 chat templates spend the token
  budget on `reasoning_content` and return empty `content` unless
  `enable_thinking` is disabled; with the fix, Qwen3.5-2B's chat
  baseline (0.717) lands on its decision-arm accuracy (0.725).

- **Extension 2 (2026-09-26, Qwen3.8 distills).** Four community distill
  arms typed through the identical harness moved the pick again (D16
  amended ×2, now a tier scheme): **Qwen3.8-4B-Distill** takes the
  reference slot on both axes that matter — best accuracy measured
  (0.767; metadata 1.00 is the first perfect class score) and best
  calibration (ECE 0.057) — strictly superseding Gemma-3-4b-it. Its one
  weak class is relational (0.35 vs Qwen3.5-2B's 0.50), which is why
  D16 now names **Qwen3.5-2B as the balanced alternative** (relational
  crown, half the latency) and keeps the Qwen3.5-0.8B fast tier. The two
  community "Qwen3.8-0.8B" repos (gatilin, Atomic-Germ) are **the same
  weights** — bit-identical `model.safetensors` (sha `a84cd623…`), one
  model under two names, caught by SHA manifest discipline — and score
  0.567, dominated by Qwen3.5-0.8B. Their uploads also omit the MTP
  block their configs declare; GGUF conversion needs llama.cpp's
  `--no-mtp` flag (converted locally to bf16, 1.5 GB, clean export).

- **Extension 3 (2026-09-27, full sweep — 41 runs).** Quant curves
  (2B and 4B UD/k-quant ladders), a 9B MoE, tiny decoders, embedding and
  decision-model references, a fork without a decision arm, and an
  external Jev-class tune moved the tier scheme again (D16 amended ×3):
  **MiMo-V2.6-9B Q3_K_S is the new frontier** (0.817, ECE 0.048, and the
  first arm over the 0.50 relational ceiling at 0.525 — 14.3 s p50, MoE
  so ~2× 4B latency), the **interactive reference moves to
  Qwen3.5-4B** (Q3_K_S 0.800 / ECE 0.069 @ 6.8 s; UD-Q4_K_XL 0.800 @
  4.9 s), and the balanced/fast tiers stand. The **2-bit cliff** is
  0.617 at 4B and 0.383–0.450 at 2B; every ≥3-bit 4B quant is ≥ 0.775.
  Tiny decoders (≤350M: Granite 0.342, Falcon-90M 0.275/0.267, Gemma
  0.200, glm5.1-distill 0.250) all land below the 5.3 ms engine layer —
  never ship them as decision arms. gte-modernbert-base zero-shot
  (0.575) ties the engine blend with the best non-model relational score
  (0.50) but ECE 0.330 — the embedding rung cannot gate anything before
  D15 calibration. **Jev-Style-0.8B-Decision-v3 is an
  interface-mismatch row, not a model-quality row** (0.217 / ECE 0.408 /
  chat 0.0): its trained readout is per-option verdict slots
  (`h·(w_yes − w_no)` at `->` positions with shipped group temperatures),
  so both of this harness's readouts sit outside its trained interface —
  zero transfer measured, native-readout arm is future work. Its
  ecosystem (lawrence3699/jev-style) is nonetheless the closest external
  analog to this codebase: decision-gated PreToolUse guard,
  artifact-embedded group temperatures, automation-at-error-budget eval.

- **Extension 4 (2026-09-28 — 44 runs).** Four operator-requested arms,
  no tier changes (D16 amended ×4): **Bonsai-4B** (ternary, 546 MiB) is
  the board's best accuracy-per-byte — 0.650, the 0.8B tier's accuracy
  at a quarter of the size — but its 4.86 s p50 is 4B-class, so it wins
  no tier (ternary packing collapses size and CPU prefill in equal
  measure); **Ternary-Bonsai-8B is unmeasurable on this host** (g64
  quant >10 CPU-hours per decision request; plain Q2_0/PQ2_0 tensors
  unreadable by the ad129b0 build — recorded as a finding, no row);
  **LFM2.5-2.6B** (0.608 @ 2.55 s, first hybrid-conv architecture
  tested) is dominated by Qwen3.5-2B; the **DavidAU X12 NEO MAX merge**
  (0.667 @ 8.35 s, best sub-3B lexical at 0.775) is latency-toxic.
  Full narrative record now committed: `results/REPORT.md` +
  `results/charts/` (rendered by `runner/plot.py` from the run JSONs).

- **Extension 5 (2026-09-28, ternary second pass — still 44 runs).** Two
  operator-requested probes, both measured negatives, no tier changes
  (D16 amended ×5): **Bonsai-8B** (Q1_0, 1105 MiB) loads and answers a
  trivial prompt in 5.4 s but prefills at ≈1.5 s/token beyond a
  ~30-token knee (196 s @ 130 tok, 405 s @ 260 tok, >420 s @ ~520 tok) —
  ≈26× Bonsai-4B per-token at matched length with the identical
  Q1_0+F32 type set (verified from the GGUF headers); the suite projects
  to ≈9–13 h of pure prefill, out of campaign budget (REPORT F18).
  **Ternary-Bonsai-2-27B** (PTQ1_0, 5.67 GiB, multimodal per its mmproj
  files) is unloadable in the ad129b0 build — ggml type 143 outside
  `[0, 43)`, prism-ml-fork-only (REPORT F19). On-CPU ternary viability
  in this build is 4B-and-below; backlog #35 (Vulkan A/B) now owns the
  whole ternary ladder.
  **Scope decision (2026-10-05, operator): the Bonsai arm is dropped
  from campaign scope, future-scope under #35.** r20b bounded the
  question instead of running the arm: the PTQ1_0 27B loads on the
  PrismML fork and answers the probed suite item correctly at p≈0.995
  (retention-consistent with the vendor's 98.2 % claim, one item), at
  73.8 s/readout on 12 P-cores — accuracy-plausible but
  speed-nonviable on CPU even if the full arm confirmed the claim
  (~10 h of readouts here); only a Vulkan/GPU rung makes the class
  worth measuring, so #35 stays its sole owner. The AtomicChat
  abliterate LoRA is unmeasurable on the fork build (`--lora` rejected
  at parse) — re-evaluate only if #35 reopens the fork.

- **Extension 6 (2026-09-28, embedding-rung bake-off — 47 runs).**
  Backlog #37 executed: the ONNX CPU-speed question answered with a
  controlled arm set (same encoder, same suite, same math, same 4
  threads). **gte ONNX-fp32 ties the torch row to the digit
  (0.575 / ECE 0.330) at 811.4 ms/item vs 3374.8 — 4.2× faster — and
  becomes the rung's runtime** (D16 amended ×6, REPORT F20); int8
  dynamic quant rejected (0.442 for −17% latency); fp16 export
  unloadable (torch 2.14 exporter emits a mixed-dtype LayerNorm);
  EmbeddingGemma-300M Q8_0 recorded via a new `llamacpp` backend in
  run_embed.py (0.500 / ECE 0.256 / 634.9 ms). run_embed.py now carries
  three backends (onnx / torch / llamacpp) with identical downstream
  math.

- **Extension 7 (2026-09-28, blockwise int4 arm — 48 runs).** The
  operator's "try ONNX at Q4_0/q4f16" follow-up, measured: ORT 1.30's
  `MatMulNBits` quantizer (block 32, asymmetric, 4-bit — the mechanical
  Q4_0 analog; q4f16 has no CPU path in ORT's 4-bit op and is
  untestable here) on the gte fp32 graph gives a 226 MB build scoring
  **0.575 / ECE 0.330 — identical to fp32 to the digit — at 815.7 ms vs
  811.4 ms: zero speedup** (D16 amended ×7, REPORT F21). Verdict: fp32
  stays the rung's runtime; q4-b32 is the RAM-bound fallback. Two
  lessons recorded: the F20 int8 collapse was the *scheme* (per-channel
  dynamic), not quantization itself — blockwise affine scales survive
  where dynamic int8 died; and "4-bit is faster" is LLM-decode
  intuition that does not transfer to batch-1 encoder GEMM shapes on
  AVX2.

- **Findings.** (1) The task-dependent floor from the reference video
  reproduces exactly: relational_compositional never exceeds 0.50 for
  any cheap arm (the ceiling held across the whole sweep until
  MiMo-9B Q3_K_S reached 0.525 at 14.3 s/decision — a verifier-tier
  exception, not an interactive one), so the confidence gate +
  escalation is mandatory, not decorative. (2) The ladder's ordering is
  validated per class: the
  lexical engine nearly solves attribute matching at 1/500th the
  latency, embeddings own paraphrase, and only the model adds the
  relational headroom it can. (3) Raw winner probability is NOT
  calibrated anywhere (ECE 0.048–0.626 across the full sweep, and even
  the best arm has no
  post-hoc calibration fitted) — §73's calibration mandate and D15 are
  confirmed by measurement before any confidence is exposed. (4) The
  constrained decision arm is bit-deterministic run-to-run on every
  model; the chat JSON-writing baseline is not (identical reruns of
  Qwen-0.5B scored 0.400 then 0.433 — continuous-batching composition
  changes greedy decode) — the scoring arm is the deterministic one.
  (5) Batched contexts cut per-decision cost 5–8x (Gemma lexical class:
  565 ms bulk vs 3.04 s single) — prefill dominates on CPU. (6) The pick
  is a tier scheme (D16 amended ×2): Qwen3.8-4B-Distill reference,
  Qwen3.5-2B balanced/relational alternative, Qwen3.5-0.8B fast tier;
  Gemma-3-4b-it, Qwen2.5-3B, Llama-3.2-1B, Qwen2.5-0.5B, MiniCPM5-1B and
  all three Qwen3.8 small distills are rejected on measurement. (7) The
  decision arm's accuracy dominance is model-dependent: the Qwen3.8-2B
  distill is the first arm whose chat baseline **beats** its decision
  score (0.700 vs 0.617) — distills trained to reason by writing lose
  under forced token-path commitment — while the 4B distill keeps the
  normal order (decision 0.767 > chat 0.742). (8) Community model repos
  repackage identical weights under different names; the SHA-256
  manifest (D14) is what catches it.
- **Accept:** suite byte-lock verified (generator rerun is
  byte-identical); every arm replays the suite twice with
  `determinism.predictions_match` recorded in its result file;
  `results/summary.md` and `results/models.manifest.json` are
  committed — raw run JSONs stay out of tree (regenerable from the
  pinned suite + manifest on deterministic arms, and pure measurement
  data the scanner gate should not have to triage).

## Phase 14 — confidence truthfulness: D15 calibration + §19 gates (done, 2026-09-28)

The benchmark proved raw winner probability is uncalibrated everywhere
(ECE 0.048–0.626) and the executor's OOD channel was dead. Both fixed,
without changing behavior for engines that do not opt in:

- **Calibration seam (`opencodifier-engine/src/calibration.rs`).**
  `Calibration` trait (`calibrate(class, distribution) -> f64` +
  `version()`), `IdentityCalibration` (raw, version 0 — D15's
  "none"), and `TemperatureCalibration` loaded from a validated,
  versioned artifact (`format_version 1`, `scheme "temperature"`,
  per-class temperatures keyed by question kind, `deny_unknown_fields`,
  `calibration_version >= 1`). Fitting is offline Python
  (`runner/fit_calibration.py`: winner-vs-rest margin temperature fit,
  golden-section NLL, no deps) — runtime stays Rust and deterministic.
  `EngineConfig::calibration` defaults to identity; a fitted artifact's
  `calibration_version` folds into the cache key (D6), so adopting one
  invalidates cached decisions exactly as it must.
- **Fitted artifacts committed** (`benchmarks/decision-model/results/
  calibration/*.json`, six arms; evidence + method + limits in
  `results/CALIBRATION.md`). All four LLM tiers are overconfident
  (T 0.84–1.67; frontier MiMo-9B ECE 0.048 → 0.026). The embedding
  rung's fit is **degenerate** (NLL falls to its T→∞ limit): gte
  zero-shot scores are ordering evidence only — no artifact emitted,
  and its scores must never gate by raw probability.
- **§19 uncertainty gates live (`opencodifier-core/src/policy.rs`).**
  `DecisionPolicy` gains `entropy_ceiling` / `min_margin` /
  `ood_ceiling` (defaults disabled: ∞ / 0.0 / ∞), validated like the
  confidence gates; `uncertainty_gate_trips` demotes an
  otherwise-accepted decision to verification. The executor now
  computes the deterministic distributional OOD proxy
  (`entropy / log2(k)`, clamped to [0,1]; density-ratio detectors come
  with model rungs, D2) and both `calibrated` and `ood` are trace
  facts. `ConfidenceReport::outcome_for` applies the full cascade:
  confidence, risk, then uncertainty gates. Disabled gates serialize as
  absent (a `RawDecisionPolicy` mirror with `skip_serializing_if`), so
  canonical policy bytes — and therefore cache keys — are unchanged for
  engines that have not opted in; the byte-locked wire fixtures pass
  untouched.
- **Accept (met):** 12 calibration unit tests + 4 core gate tests + 2
  engine integration tests (an artifact swap changes confidence AND
  invalidates the cache; the OOD channel gates acceptance end-to-end
  and is observable in the trace); fmt/clippy/test/doc/deny/aegis green.

## Phase 15 — §43 relational solver: exact proofs over extracted facts (done, 2026-09-28)

The benchmark's relational_compositional class exposed the gap between
"BM25 over prose" and "knowing the state": every model arm plateaued at
≤ 0.53 and the engine at 0.350. The cheapest-reliable-rung principle
(§43) applied literally says the runtime should *prove* relational
answers before it scores them:

- **Exact-grammar fact extraction (`facts.rs`).** Sentences split on
  `.`; each must match one pattern in full — `X depends on Y`,
  `X is healthy|degraded|down|failing`, `X: s1, s2, …`,
  `X comes back online only after Y` — into typed `RelationalFact`s.
  Total on any input, conservative by construction; entity names are
  `[A-Za-z0-9_-]` runs ≤ 64 chars, so hostile text cannot smuggle
  structure.
- **The relational solver (`relational.rs`), a Classifier decorator.**
  Three general operators — root cause (transitive dependency closure),
  healthiest group (strict argmax), first restored (gates-something,
  waits-for-nothing) — compute over the *whole* extracted structure,
  not the candidates. A proof stands only if it is unique, inside the
  candidate set, and agreed by every operator; anything else delegates
  to the inner classifier untouched. No question-text keywords: the
  facts decide what fires. `model_id` composes
  (`relational-v1|builtin-lexical-v1`).
- **Cache-key honesty (D6/§64).** `DecisionEngine::new` now derives
  `identity.model_id` from the live classifier's `model_id()`, so a
  wrapped or swapped classifier changes keys without caller
  bookkeeping; a hand-set identity model_id is never trusted. This was
  a real (if latent) correctness gap: the composed id existed for cache
  composition but nothing consumed it.
- **Default stack.** `EngineHandle::lexical` (CLI/HTTP/MCP all route
  through it) assembles solver-over-lexical: the base binary is more
  useful with still zero ML.
- **Measured (engine arm re-run on the byte-locked suite).**
  relational_compositional 0.350 → **0.950**, blended 0.483 → **0.683**,
  p50 5.3 ms → **1.3 ms**; metadata/lexical classes bit-identical (no
  regression); determinism replay exact. Proofs leave at p = 1.0 and
  were right 26/26; the two relational misses were delegated hedges.
  D16 amended (×8): the zero-ML floor undercuts the fast tier
  (0.683 @ 1.3 ms vs Qwen3.5-0.8B 0.650 @ 613 ms). Calibration: the
  global temperature fit worsens ECE on the bimodal proof/delegate
  stack, so no artifact ships (CALIBRATION.md finding 2); the retired
  `builtin-lexical-v1` artifact was removed with the stack it
  described.
- **Accept (met):** 23 new engine unit tests (grammar totals/hostility,
  per-operator proofs and abstentions, delegation, model-id
  composition, solver-vs-bare regression) + 2 pipeline tests
  (classifier-swap invalidation, hand-set-identity override) + http e2e
  identity check updated; fmt/clippy/test/doc/deny/aegis green; engine
  arm re-measured, board at 49 runs.

## Phase 10 — `opencodifier-mcp`: the Model Context Protocol surface (done, 2026-09-28)

§31/§55: the runtime is driveable by MCP hosts as tools, over stdio, with
no chain-of-thought anywhere. The crate is thin by the same rule the HTTP
surface is thin: every tool normalizes through [`opencodifier_schema`] and
executes through [`EngineHandle`] — there is no MCP-specific pipeline to
drift.

- **The server (`OpenCodifierServer`).** rmcp 2.2.0 (D1's pin, honored
  verbatim; the 3.x line is a major version and is not adopted — D17)
  with `#[tool_router]`/`#[tool]` over an `Arc<EngineHandle>`. The
  engine is sync (D5); a stdio session is strictly sequential, so tools
  call it inline (no `spawn_blocking`) — there is no concurrency here to
  starve, unlike the concurrent HTTP server.
- **The Phase-15 tool set (§55).** `codify_decide` (native request →
  native response, `POST /v1/decide` as a tool), `codify_batch`
  (independent per-item verdicts, in order, capped at
  [`MAX_BATCH`] = 16 with `mcp.batch_too_large`), `codify_graph` (the
  identity + the full validated graph document actually running, via the
  new `EngineHandle::graph`), `codify_validate` (the engine's own
  decode-and-validate path, `graph.*` codes verbatim), `codify_verify`
  (the confidence-gate verdict: outcome, decisive flag, the full
  multi-dimensional confidence report), and `codify_explain` (the
  response's deterministic trace plus the executor run report — the
  machine's record, never generated reasoning).
- **One error envelope, shared with HTTP.** Tool failures are
  tool-level results (`is_error: true`) carrying the same
  `{"error":{"code","message"}}` document under the same stable codes
  (`schema.*`, `graph.*`, `engine.*`); the crate's only own code is
  `mcp.batch_too_large`. Abstention is `is_error: false` with the typed
  outcome.
- **Two shared-surface folds (no duplicate implementations).** The
  engine's private `GraphRepr` serialization shim became the public
  `GraphDocument` and the HTTP surface's private mirror of it was
  deleted — one decode-and-validate path for both ingresses. The CLI's
  `--trace` projection moved into the engine as
  `opencodifier_engine::report::execution_json`, so CLI and MCP project
  the run report through one function.
- **CLI.** `opencodifier mcp serve [--graph PATH]` (clap v4, D13):
  assembles the same engine as `serve`, serves stdio until the client
  disconnects; session failure is exit 3 under `mcp.session_failed`.
- **Accept (met):** 4 envelope unit tests + 10 server unit tests +
  12 client-driven e2e tests (real rmcp `ClientHandler` over a duplex
  transport: handshake, exact tool list, decide/batch/graph/validate/
  verify/explain, hostile payload, oversize batch, abstention-is-
  success, unknown-tool protocol error) + 2 CLI stdio e2e (a real
  JSON-RPC session through the binary; `--graph` refusal before any
  session); fmt/clippy/test/doc/deny/aegis green.

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

## Phase 16 — §45 focused-question extraction + reverse escalation (done, 2026-09-28)

A decision-model rung pays per token, and the cheap rungs before it read
whole state text. When a state is much longer than the decision needs,
the engine now builds a per-question **view**: the sentences that carry
lexical evidence for the question and its candidates, in original order,
within a token budget — and escalates back to the full state when the
focused decision is weak.

- **The extractor (`focus.rs`), deterministic and model-free.** Sentence
  split on `.`/`!`/`?`/newline with delimiters attached (a kept view is
  the original prose byte for byte); BM25 over sentence documents with
  the question text plus every candidate description as the query, plus
  a verbatim candidate-id mention bonus (the strongest relevance signal
  there is). Sentences with zero evidence are never selected; facts are
  structural and are never touched — only the text view shrinks.
- **Recall-oriented budget.** The budget bounds the *median* view, not
  every view: a decisive sentence longer than the whole budget is kept
  whole rather than amputated, and when no sentence shows positive
  evidence the extraction is blind and **declines** — the view is the
  full state. Token estimate is the engine's deterministic bytes-over-4
  (no tokenizer exists; D2); the estimate only has to be stable and
  monotone, because it is a budget, not a measurement.
- **Reverse escalation, at most once, before any gate.** When extraction
  engaged and the focused distribution's top probability is below the
  request policy's `min_confidence`, or its entropy trips the §19
  entropy ceiling (disabled by default), the engine re-decides on the
  full state *before* the confidence gate or the verifier sees an
  answer. The fallback is therefore invisible to the gate but visible
  in the trace.
- **Cache honesty (D6/§64).** A focus policy folds into the cache
  identity as `|focused-v1@<budget_tokens>` on the classifier's model
  id, so changing the budget or turning focusing off invalidates cached
  decisions without caller bookkeeping.
- **Surface.** `FocusPolicy`/`FocusSummary`/`FocusView` +
  `engine::EngineConfig::with_focus`; `--focus-budget TOKENS` on
  `opencodifier decide`/`serve`/`mcp serve` (one shared
  `runtime::engine_config` assembly). Trace detail keys
  `focus_engaged/kept/total/tokens/escalated` appear only when a policy
  is configured; `RunReport::focus` counts (`decided/engaged/escalated`)
  are all zero without one, and `execution_json` grows its `"focus"`
  key only when `decided > 0` — unfocused projections are byte-identical
  to their pre-focus form.
- **Measured (long-suite A/B through the real binary).**
  `suite/suite_long.json` (derived, deterministic, ~3,681-token states)
  run twice through `opencodifier serve` — full state vs
  `--focus-budget 512`: **answer-identical on all 120 items**
  (0.683 blended, ECE 0.094, same per-class, determinism replay exact on
  both sides). 84/120 items extracted (views p50 **93 tokens**, ≤ 511,
  out of ~145-sentence states); 36/120 declined blind (contexts with no
  query vocabulary) and still matched; 42/84 engaged views escalated and
  changed nothing. Extraction costs ~2.5 ms/item p50 at this scale
  while deciding on ~4% of the state. REPORT.md carries the full A/B.
- **Accept (met):** 12 `focus` unit tests (budget semantics, recall
  rules, determinism, hostility, decline paths) + 7 engine integration
  tests (engagement, no-escalation, escalation to the full state, cache
  identity separation, projection byte-stability) + 1 CLI e2e
  (long-state `decide --trace --focus-budget`); fmt/clippy (1.98 +
  CI-exact 1.90)/test/doc/deny green; generator + suite + REPORT
  recorded in the benchmark.

## Phase 17 — external anchor: JevBench public split (engine + bridge + vtx done)

Every REPORT.md number so far is internal (our suite, our seeds). Phase 17
measures OpenCodifier on a third-party suite with published rows for
comparable systems. Methodology of record:
`benchmarks/decision-model/JEVBENCH.md` (dataset and harness verified
against the official repo, `fstandhartinger/jevbench` @ `9ec6f15a`, MIT;
231 public items; choice 139 / noul 74 / score 18; mean chance 0.3176).

- **Harness discipline.** The authors' own harness runs our arms — their
  runner, their scoring (argmax for choice/noul, expected value for
  score, multi-class Brier, top-label ECE), their serial no-retry budget
  semantics, raw evidence outside both repos. Zero scoring drift against
  every published row is the whole point.
- **The bridge row.** Jev-Style-0.8B-Decision-v3 Q4_K_M through its
  native verdict-slot readout first: its authors self-ran exactly these
  weights on exactly these 231 items with the official harness and
  published **64.1 % (148/231)**. Reproducing that within a few points
  validates adapter and environment; every other row we produce then
  transfers to the published context.
- **Arms.** engine (default deterministic stack over `/v1/decide`;
  abstain → incorrect, winner-only → label-only — never an invented
  tail), fork_4b (tree mode, label-only per D15), jev_native (native
  probs), vtx (VTX-JEV-3 through its vendor `JevClient` with the
  jev_native rendering). `runner/run_jevbench.py`, wire facts
  smoke-verified and recorded in JEVBENCH.md.
- **Sub-steps.** (1) ✅ engine arm complete run — 231/231 in one
  `run_all`, accuracy **0.3766** (macro 0.4011, chance 0.3176), ECE
  0.402, Brier 0.890, client-wall p50 **2.0 ms**; replay determinism
  231/231; zero synthesized probabilities (coverage 1.0, abstain ~19 %
  scored incorrect). Getting here killed two misdiagnoses: the v1/v2
  stops at item 186 were the engine's deterministic 3-consecutive-abstain
  stretch tripping the infra stop rule (abstain now maps to the 422
  refusal bucket, still incorrect), and the hours-long "load wedge" was
  the runner's per-row fsync stalling on a contended ext4 journal —
  the same 231 items take ~60 s on tmpfs. (2) ✅ native bridge run —
  **0.6494 (150/231)** vs the published 64.1 % (148/231), +0.9 pp on the
  authors' own harness; ECE 0.080 (their stack is calibrated here, ours
  is not), Brier 0.425, p50 6.72 s, replay determinism 231/231 on
  labels *and* probability vectors, `probability_sources: ["native"]`
  only; archived `runs/jevbench/bridge-v1`. (3) fork_4b run on a quiet-host
  window (D16 config). (4) ✅ REPORT.md JevBench section + JEVBENCH.md
  results table. (5) optional: the fork-side full-distribution patch
  (D15) upgrades the fork arm from label-only to native Brier/ECE
  (patch written both sides, inert until fork rebuild).
  (6) ✅ static-embedding survey arm (vtx) — VTX-JEV-3 (Model2Vec-class
  255,753×256 table, 20.5 MB 2-bit LF2, position-gated pooler in the
  vendor `inference.py` client) on the public 231: accuracy **0.4113**
  (macro 0.4318), ECE 0.126, Brier 0.684, p50 **7.9 ms**, replay
  determinism 231/231, native probabilities only; 3.5 pp above the
  engine, ~23 pp below the 0.8B bridge, ~45 pp below the JEV-27B
  anchor — a data point the field lacked, not a new tier (D16
  unchanged). Suite counterpart rows `run_vtx.py`: LF2 0.242, FP32
  0.317 (quantization costs 7.5 pp); archived `runs/jevbench/vtx-v1`.
  (7) ✅ fusion study (post-hoc ladder over measured rows,
  `runner/fusion_study.py` + `results/fusion-*.md`): suite
  engine→gte→Qwen3.5-2B blend **0.867 @ 753 ms** vs 0.725 best single
  (F23; relational 0.88, oracle 0.975, blended ECE 0.127 → per-rung D15
  calibration is the follow-up); JevBench engine+vtx 0.455 (+4.4 pp over
  vtx-only) but engine+bridge **0.632 < 0.6494 bridge-only** — the
  engine's out-of-domain ECE 0.402 poisons the gate (F24; the study's
  single-arm ECE line was recomputed 2026-10-02). Live-ladder
  **wiring landed (2026-09-30, D25)**: `LadderPolicy` per-node/per-kind
  gate overrides resolved at the existing confidence gate
  (`opencodifier-engine/src/ladder.rs`), identity-decorated cache keys,
  byte-identical when empty; thresholds become config once per-domain
  calibration exists (opt-in ship posture). **Model rung wired
  (2026-10-05, D26/D27)**: `LlamaDecisionClassifier`
  (`opencodifier-model`, `llamacpp` feature — ureq loopback client over
  the fork's `POST /v1/decision`, Rust-side softmax, no synthesized
  probability) + cross-rung escalation tail (`with_rungs`, fired only
  by the gate, `|rungs-v1@…` cache re-key) + CLI surfacing
  (`--llama/--llama-model-id/--llama-timeout-ms` on `decide`, `serve`,
  `mcp serve`; `runtime::handle` assembles lexical-primary +
  model-rung; `ladders/README.md` "The model rung"; default build
  stays dependency-identical and refuses `--llama` with
  `cli.model_rung_unavailable`). **D26 acceptance MET (2026-10-05,
  live run)**: locked suite through `opencodifier serve --ladder
  ladders/fusion-v1.json --llama …` against the pd-fork `38de7eb`
  `llama-server` — blended **0.842 ≥ 0.80** at **117.2 ms mean ≤ 1 s**
  (p50 0.54 ms; 37/120 items escalate; replay bit-exact; board row
  `engine__rung-qate2b-q4_0-fusion.json`, BENCHMARKS.md footnote ¹⁴).
  **F28 gate refit landed (2026-10-05)**: contract-faithful tree
  collection (`runner/collect_tree.py`) showed the model rung alone at
  **0.758** under the engine's served shape (letters-era 0.808 does not
  transfer); the offline sweep (`runner/tree_margin_refit.py`) found
  the margin axis flat on lexical confidences — shipped
  `ladders/fusion-v2.json` (accept at confidence ≥ 0.56, margin 0.0) —
  engine-validated at **0.933 @ 259.8 ms** mean (56/120 escalations,
  replay bit-exact, in-sample; held-out estimate ~0.90), board row
  `engine__rung-qate2b-q4_0-fusion-v2.json`, footnote ¹⁵; run JSONs now
  stamp `config.host` provenance. **F29 held-out validation (2026-10-05,
  `suite_holdout.json` v3, 120 disjoint items)**: fusion-v2 **0.892** vs
  fusion-v1 **0.800** under the identical stack and host — the refit
  advantage transfers out-of-sample undiminished (+9.2 pp vs +9.1
  in-sample) and the honest discount from the in-sample 0.933 lands
  inside the predicted ~0.90 band; the holdout's own optimum gate
  plateau is **0.56–0.59**, containing the shipped 0.56 with zero drift
  (margin axis flat on holdout too); lexical-only fell to 0.700 once
  the audit-hardened class B lost its keyword hooks — a genuine
  transfer test. **F30 integrity audit (same day)**:
  `runner/audit_suite.py` re-derives every class-A/C answer from its
  own context (80/80 exact on both suites), enforces class-B
  structural validity + a keyword-leak test, checks main/holdout
  disjointness, byte-lock, and stdlib-only provenance; one v1 leak
  disclosed (frozen suite), six holdout leaks reworded before any F29
  measurement, cross-suite id collision fixed (`h*` namespacing).
  Board rows `engine__holdout-*`, REPORT.md F29/F30 entries,
  BENCHMARKS.md held-out section.
  The embedding rung's margin
  gate is measured (`results/embed-margin-study.md`: margin ≥ 0.0183 →
  coverage 0.208 at acc 0.880, monotone to 1.000 at 0.0283), closing
  CALIBRATION finding 3. (8) ✅ resource accounting: `runner/resources.py`
  `/proc` monitor in every runner (peak RSS / CPU-s / IO / wall per arm
  into run JSONs + summary.md peak-RSS column). (9) ✅ adapter-training
  research record `docs/TRAINING.md` (E0–E3 experiment ladder; first
  0.8 B decision-head/LoRA run ≈ 1–2 CPU-days on this host). (10) ✅
  ONNX runtime arm (`runner/run_onnx.py`): the optimum q4 export of
  Qwen3.5-2B driven by onnxruntime with the fork's tree scoring ported
  exactly (greedy parity validated at the probe stage) — acc **0.658** /
  ECE 0.064 vs 0.725 for llama.cpp on the *same weights*, prefill-bound
  ≥9× slower, 2.6× the peak RSS (F25; the run carries a host-contention
  caveat that cannot move the verdict — accuracy and determinism are
  load-independent). Ladder unchanged; ONNX stays the portability
  story.
- **Accept (partial):** engine arm: zero synthesized probabilities ✅,
  replay determinism ✅, all 231 in one run ✅. Bridge reproduction ✅
  (+0.9 pp), bridge schema discipline ✅, bridge determinism ✅,
  REPORT/JEVBENCH sections ✅, vtx survey arm ✅ (native probs,
  deterministic, documented in `docs/BENCHMARKS.md`). fork_4b arm
  pending quiet host.

## Phase 18 — hardening pass (coverage, lints, e2e, remaining §-items)

The 99 % test/code/doc-coverage target and the strictest-lint posture are
their own phase, not an afterthought of feature phases. Ground rules: the
measurement arms of Phase 17 share this host with CI; cargo-heavy steps
(census runs, e2e suites) need the same quiet-host window discipline as
the benchmark arms — plan them for windows, author the code/tests CPU-light
in between. Ordered so each step's output feeds the next.

- **18a — Coverage census (needs quiet host, ~one `just test-cov` run).**
  ✅ Done 2026-09-29: `docs/COVERAGE.md` — baseline **97.47 % lines /
  95.56 % functions / 98.20 % regions** (19 230 lines), per-crate table,
  every gap ≥ 5 lines classified `testable-now` / `needs-harness` /
  `mixed` with the 18b plan or waiver named. ~425 of 487 missed lines
  are reachable by normal tests; the embedding-model-file, cache-
  concurrency, and executor-deadline slices carry the waiver plans.
  The first census attempt was contaminated (stale `llvm-cov-target`
  instrumentation reported ghost files at 0 %); the purge recipe and
  the ghost-file check are recorded in COVERAGE.md.
- **18b — Gap tests (CPU-light authoring, gated by 18a's table).**
  ✅ Done 2026-09-29: 18 tests across five crates closed every
  reachable gap — line-truth misses went 121 → 78, and all 78
  remaining lines are classified unreachable-by-construction in
  `docs/COVERAGE.md`'s waiver table (`#[non_exhaustive]` future-
  variant arms, `Distribution::from_pairs` arms fed by data the
  constructors preclude, guards implied by an earlier check, serde
  infallibility). Highlights: mutex-poisoning fault injection for the
  cache and clock (real panic under the held lock, degraded-but-correct
  service asserted), the IR's last-line confidence validation pinned
  via a public-seam `Calibration`, and real-SIGINT tests for `serve`
  and the HTTP shutdown signal. Acceptance is via PLAN's explicit
  per-gap waiver clause: the text-column numbers (98.69 % lines /
  96.02 % functions / 97.91 % regions) cannot reach 99 % without
  covering construction-unreachable code, and COVERAGE.md documents
  the three measurement views so the residual is understood rather
  than chased. (The 18a census had the text table's Lines and Regions
  columns swapped; COVERAGE.md carries the correction.)
- **18c — Strict lints.** ✅ Done 2026-09-29: the census collapsed to a
  proof rather than a backlog — every crate wires workspace lints and
  CI's `-D warnings` lint lane was green, so `missing_docs` (plus
  `rust_2018_idioms`, `clippy::all`, `clippy::pedantic`) had zero
  violations and were flipped warn → deny directly. A bare local
  `cargo clippy --workspace --all-targets` is now as strict as CI.
  Accept: workspace lints all deny-level (the four pedantic waivers
  remain, documented with reasons), clippy and rustdoc green — CI
  proves it per push.
- **18d — E2E validation suite.** ✅ One scripted pass (`scripts/e2e_validate.py`,
  `just e2e`, GitForge `e2e` stage + GitHub mirror) against the release
  binary: healthz identity, native `/v1/decide` choice/boolean/score,
  cache-engage identity (identical decision content, `cache_hit` false→
  true), abstain-as-200, malformed state → 400 `schema.*`, oversized
  body → 413, graph validate (clean DAG + cycle → `graph.cycle`), and
  the MCP stdio handshake with the full tool list. Amended while
  building it: the shipped HTTP surface is exactly the three routes in
  `routes.rs` — `/v1/systemone` and Anthropic/OpenAI HTTP shapes do not
  exist (those wire formats are `opencodifier-schema` library adapters,
  covered by crate tests; §36 endpoints are 18e). The suite also
  caught two real facts: `policy` is a required request field, and the
  first run's leaked serve process silently served later runs from warm
  cache (now guarded by a pre-flight port check). Accept: 13/13 checks
  green on the release binary; CI stage proves it per push.
- **18e — §36 HTTP surface completion.** `/v1/batch`, `/v1/graph/run`,
  `/v1/validate`, `/v1/models`, `/v1/capabilities` (semantic-cache probe
  too), each with contract tests from 18d's harness. Accept: every
  documented endpoint returns a spec-shaped answer, contract tests in CI.
  Status: complete — `/v1/batch` (engine-owned `MAX_BATCH=16`
  shared with MCP `codify_batch`, per-item error envelopes, 200 on any
  well-formed batch), `/v1/validate` (decode-only preflight), `/v1/models`
  + `/v1/capabilities` (single active lane; `decision_kinds` pinned to the
  IR by test), and `/v1/graph/run` (D19: client graphs are validated
  through the engine's own constructor, node-capped pre-construction, and
  executed via `EngineHandle::ephemeral` — content-addressed identity,
  per-request throwaway cache, so ad-hoc runs never touch the serving
  cache). 20 Rust contract tests + 8 new checks in
  `scripts/e2e_validate.py` (21/21).
- **18f — §63 Decision Registry.** Artifact format (versioned, hashed,
  D6-style identity) + loader + a registry-backed routing example; the
  seam noted in `codec.rs` becomes the format. Accept: format doc in
  `docs/`, round-trip tests, one recipe using it.
  Status: complete (D20). `DecisionDefinition`/`Registry` in
  `opencodifier-schema/registry.rs`: content-hashed identity (labels are
  not identity), validating decode through the wire types' own
  constructors, dynamic-vs-static candidate discipline, duplicate-id
  refusal. Format doc `docs/REGISTRY.md`; 8 unit tests including a
  derivation test that pins `recipes/registry/requests/model-selection.json`
  to `instantiate()`'s exact output; the recipe's expected response was
  captured against `minimal-choice.json` (`verify`, top 0.687).
  SPEC_COVERAGE §63 → done.
- **18g — §34 recipe fleet + `recipe install`.** Status: complete. The
  twelve §34 decision areas are runnable recipes under `recipes/` —
  graph + request + captured response each (5 verify, 7 abstain outcomes;
  abstention is the honest zero-ML posture, not a gap).
  `opencodifier recipe list` names the fleet; `recipe install <name>
  [--dest DIR] [--force]` writes the three files into a project-local
  directory (default `./recipes`), refusing to clobber without `--force`
  (`cli.recipe_exists`) and refusing unknown names (`cli.unknown_recipe`).
  The fleet ships inside the binary via `include_str!`
  (`crates/opencodifier-cli/src/recipes.rs`), so installed bytes are
  byte-identical to the committed copies — asserted by unit and
  integration tests and by the e2e suite. 18d's suite grew 21 → 30
  checks (list coverage, install byte-identity, overwrite guard, and
  decide-through-install matching the captured outcome on a fresh
  server). SPEC_COVERAGE §34 → done.
- **18h — Graph nodes (§53 retrieval, §26/§54 rerank, §51 optimizer).
  DONE (2026-09-29).** D21/D22 records first, then code: `embedding`
  (annotation), `retrieve` (`top_n`/`floor`, floor-never-starves keep-top-1,
  every drop trace-named), `rerank` (permute never remove;
  `LexicalReranker`/`EmbeddingReranker`); missing backend refused at
  assembly (`engine.missing_backend`), safe mode refuses `retrieve`;
  `embedding_model` rides engine identity and every cache key.
  `optimize()` = dead-node elimination + pure-node CSE only (D22 bars
  folding/early-exit; cache/threshold/output excluded from CSE; merges
  dedupe rewritten deps), graph_version preserved, result revalidated via
  `DecisionGraph::new`. Acceptance: 10-test `tests/semantic_nodes.rs`
  integration suite + 9 optimizer tests incl. the fleet equivalence proof
  (all 15 committed graphs decide identically optimized vs not); HTTP
  identity JSONs carry `embedding_model` additively.
- **18i — Measurement-queue completion (storm-gated; runs in quiet
  windows).** Landing in order of information value: bridge ✓
  (0.6494), **native verdict-slot ✓ (0.8083, F26 — #25 closed)**,
  **margin gate study ✓ (CALIBRATION 3 closed)**, **d15 exact refit ✓
  (T 0.8426 → 0.9317 exact-fit artifact shipped)**, **Vulkan APU ✓
  (0.725 @ 2.23× the CPU leg's speed, 1/43rd the host CPU-seconds —
  REPORT Device A/B)**, **vision probe ✓ (#42: LFM2.5-VL-450M answers
  a rendered decision context identically to text, 0.33 s vs 1.42 s —
  capability gate passed)**, **#62 cross-builds ✓ (4/4 linked
  2026-10-01, D24 status)**. Storm-recovery chain (2026-10-01, REPORT):
  the storm-killed #32 arms are re-driven under gate policy v2 —
  **t16 ✓**
  (every metric float-identical to the surviving t08 arm; its latency
  row is load-confounded and not of record), fork_4b's recovery arm
  rc=1 on a driver dataset path (re-queued via follower with corrected
  paths), diet100–ctx16k still in flight (days at storm pace — the
  driver's own header). Accept: each lands in REPORT.md with its
  honesty rules. **Relocated (2026-10-02, operator decision): the
  remaining queue no longer runs on this box** — it is
  infrastructure-only from here on (services/CI/storage; its load
  floor ~17–31 makes quiet windows unreachable). The in-flight params
  long arm (~9 h banked) was killed and the whole measurement payload
  moved to the Fedora compute node (i5-13600K, idle by role;
  `fedora:~/oc-model-eval`, driver script `fedora_queue.sh`): builds
  (pre-d15 `ad129b0` + d15 `38de7eb`), a 3-item toolchain smoke
  (passed, python 3.14), then fork_4b-fedora baseline re-run
  (same-host anchor + cross-host determinism vs fork_4b-v1), the
  instruction A/B (#76), the d15 arm (#61), and the long-arm re-run
  (#32) whose result becomes the clean latency of record. Local
  partials: `fork_4b-instr-decide-v1-local-aborted` (55/231).
- **18j — Distribution + WASM (§57, §58, §68).** Own phases after the
  core is hard: cross-platform release matrix, Homebrew/winget/crates.io;
  `opencodifier-wasm` with the §68 security posture. Accept: per §58/§57
  rows in SPEC_COVERAGE flipping to done with evidence.
- **18j (WASM half) — DONE (2026-09-29); distribution (§58) remains.**
  D23 record first, then `crates/opencodifier-wasm`: `WasmEngine`
  (`decide`/`validate_graph`/`run_graph`/`identity`) over
  `EngineHandle::lexical` — the base zero-ML posture in the browser —
  with typed JSON errors (`{"code","message"}`, same codes as HTTP) at
  every entry point. The first wasm-pack artifact trapped in Node, which
  forced the platform seams D23 now records: `clock.rs`'s `Instant`
  (std natively, `web-time` on wasm32, target-gated so the native tree
  gains nothing) and sequential wave execution on wasm32
  (`parallel_waves` = 0 by construction). Acceptance: `just check-wasm`
  (host tests, `cargo check --target wasm32-unknown-unknown`,
  `wasm-pack --target nodejs`, Node smoke over the real artifact —
  decide round trip, determinism, hostile refusals, graph cycle,
  missing-backend refusal; NOT part of `just ci`), 7 host tests incl.
  a manifest scan asserting no tokio/axum/reqwest/ort/libloading/dlopen,
  §68 posture structural. Browser demo rides §58.
- **18j (§58 remainder) — builds recorded (D24 status 2026-10-02).**
  All five targets verified at their named level: linux-x86_64
  run-tested + e2e (platform of record), windows-x86_64-gnu **linked
  2026-10-02** (`opencodifier.exe` PE32+ console x86-64 +
  `opencodifier_wasm.dll` cdylib, zero warnings, 2m44s at -j4 beside
  the measurement chain; smoke is structurally out of scope on this
  host — no wine, no Windows machine), and the overnight cross-build
  linked the remaining four —
  aarch64-linux-gnu, aarch64-linux-musl (static), aarch64-darwin,
  x86_64-darwin — at link + `file`-magic level (no foreign-arch
  execution claimed; no qemu, no macOS host). macOS artifacts state the
  no-run/no-signing caveat in release notes. Registry publication
  (crates.io/Homebrew/winget) is an outward action awaiting explicit
  user go — never automatic.
- **Coverage close-out — DONE (2026-09-29).** The 18b gap table driven
  to its floor: 203 → 125 `DA`-missed lines by twelve tests across six
  crates (wasm public-boundary success paths, registry decode-refusal
  matrix + score round trip, D21 graph knob validation, miscounting/
  failing embedding backends through node and reranker seams, filter→
  embedding→retrieve composition, HTTP malformed batch/graph-run
  bodies, recipe install failure arms). Headline: 98.97 % lines on the
  lcov `DA` basis (12 007/12 132), 98.34 % text-basis lines, 95.61 %
  functions, 97.68 % regions. Every one of the 125 survivors is
  classified with its enforcing fact in `docs/COVERAGE.md` — no
  reachable line remains untested; the rest are `#[non_exhaustive]`
  future-variant arms, construction-implied guards, server-fault
  encodes, the wasm error boundary (Node-smoke-owned), and
  test-support panics. Discovery recorded: a host test cannot build
  any `JsValue` (`__wbindgen_string_new` has no host shim → SIGABRT),
  so wasm error paths are provable only in Node over `pkg/`. All seven
  `just ci` gates green; aegis baseline regenerated (fmt column shifts
  + this ledger's own prose → 3 027 findings, 0 new).

**Sequencing.** 18a/18b/18c are the coverage spine and gate everything
else (no new surface lands untested after 18b). 18d precedes 18e–18g (the
suite is their harness). 18h is decision-heavy — DECISIONS.md first. 18i
fills quiet windows throughout. 18j last. WGCA/web-standards: no web
frontend exists — the clause maps to 18d's HTTP contract suite and the
API reference; noted here so it is not silently dropped.

## Phase 19 — adoption + integration documentation (opened 2026-10-02)

User directive (2026-10-02): "add docs and clear instructions on how to
integrate with various coding tools, harnesses, and so on that people
use … build out solid integration, usage documentation … for ease of
adoption" — with the D29 constraint that first-party integrations
(Amortyx et al.) are worked examples over public surfaces, never a
privileged stack.

- **Doc of record: `docs/INTEGRATIONS.md`.** One entry point for every
  integration path, with the honesty rule that every documented
  endpoint, flag, and tool name is verified against the shipped tree
  (the same discipline SPEC_COVERAGE applies to §-items). Covers: the
  HTTP surface (native `/v1/*`), the six `codify_*` MCP tools with
  client registration, the CLI (`--format native|openai|anthropic|jev`),
  Rust library consumption under the dependency-layering rules, the
  WASM zero-ML runtime (D23), CI decision-gate patterns (GitForge
  pipeline of record), and first-party integrations as examples
  (Amortyx → `docs/INTEGRATION_AMORTYX.md`).
- **19a — wire-format selection over HTTP (gap, code). — DONE
  (2026-10-02, D30).** The `x-opencodifier-format` request header
  (`native` default | `openai` | `anthropic` | `jev`) on
  `/v1/decide`, `/v1/batch`, `/v1/validate`; decode and response
  projection symmetric per request, native error envelope everywhere,
  unknown names refused `schema.invalid_value`, absent header
  byte-identical to pre-D30 behavior, cache identity unchanged
  (keyed post-decode). Recorded as D30; route-level round-trip,
  unknown-format, batch, and validate tests in `routes.rs`.
- **19b — Jev-ecosystem callers. — DECIDED (2026-10-02, D31): no
  dedicated route.** The `/v1/systemone` convention is NOT an
  OpenCodifier route and stays unclaimed. D30's
  `x-opencodifier-format: jev` header carries the whole capability
  (same adapter, same cache identity); a route would be a pure alias
  with zero new capability, and promoting a Jev convention to a
  top-level route would invert the identity doctrine (Jev
  compatibility is one adapter mode, not the product's shape).
  Revisit condition recorded in D31: a named consumer that cannot set
  a request header.
- **19c — quickstart + copy-paste examples. — DONE (2026-10-02).**
  The §2.3 quickstart now works as printed (core: `state.facts`,
  `policy`, `metadata` are optional on the wire, defaulted; a
  hand-written minimal payload never invents housekeeping fields) and
  is pinned by `the_documented_quickstart_round_trips_verbatim` in
  `tests/http_e2e.rs` — the test pastes the doc's exact JSON. The §2.4
  format-header curl is pinned over the wire too
  (`the_format_header_selects_the_adapter_over_the_wire`: openai
  projection + unknown-value refusal). Doc fixes found by writing the
  test: the payload's `state` is `{"text": …}` not a bare string, the
  question discriminator is `type` not `kind`, the error-envelope
  example now names a real code (`schema.invalid_value` — the old
  example's `schema.unknown_field` never existed), and the outcome
  list names all six IR outcomes. Choice/Boolean/Score example
  requests, MCP registration snippets, and the abstention envelope
  were already exercised by the `http_e2e` suite (choice round-trip,
  `policy_with_high_threshold_abstains_with_a_200`, score levels,
  batch per-item errors).
- **19d — publish path (blocked on user go, unchanged).** crates.io /
  Homebrew / winget / §58 publication remain explicitly
  user-gated (§58 note below); this phase only prepares the docs
  those channels need.
- **19e — Amortyx live integration E2E (task #86). — DONE
  (2026-10-04).** V1 chat surface validated end-to-end through the
  deployed router on this host. Two fixes landed to make the path
  work: Amortyx `38daffa9` (`feat/forward-client-response-format`)
  forwards the client's `response_format` to the provider wire
  (released routers drop it at three ingress layers, so every
  strict-schema decision request died as a 502-relayed
  `schema.unsupported_generation_field`), and the OC chat envelope
  now carries an honest zeroed `usage` object (OpenAI-shaped clients
  decode it unconditionally; pinned in `http_e2e`). Validation:
  dual-arm parity (content identical direct vs router, 5/5 fresh
  bodies), 12-way burst (12/12, single distinct content),
  abstention pass-through, and the full breaker lifecycle with
  unique bodies (5 failures → open → 30 s → half-open → 3 successes
  → closed; provider restart does not reset it). Full record +
  findings F-1..F-4 in `docs/INTEGRATION_AMORTYX.md` §13. Both code
  gaps found by the validation were fixed, deployed, and re-verified
  the same day: request-side `response_format` ingress (Amortyx
  `38daffa9`) and response-side extension passthrough (`d182a126` —
  through-router responses now carry
  `opencodifier: {outcome, calibrated_confidence}`). F-3 (exact-match
  cache serves 200s during outages — vary bodies in drills) and F-4
  (doc drift, fixed) recorded; F-2 (intermittent silent 404s during
  restart churn, proven not-the-router) remains open as
  environmental.

Acceptance: every INTEGRATIONS.md claim mechanically checkable (route
exists in `routes.rs`, tool registered in `opencodifier-mcp`, flag in
`args.rs`); no aspirational endpoint anywhere; D29 respected in every
example.

## Spec coverage map (audit, 2026-09-28; refreshed 2026-09-30)

All 17 tracked phases are done; the §-by-§ check that every PLANNING
section has a landing place lives in `docs/SPEC_COVERAGE.md`. The gaps
the 2026-09-28 audit surfaced have since closed: §36 HTTP surface (18e),
§34 recipe fleet + `recipe install` (18g), §63 decision registry (D20,
18f), and the §26/§53/§54 ML graph nodes (18h) are done; §57 WASM shipped
runtime-first (D23, 18j). The §58 build matrix is now recorded at its
named verification levels (D24 status 2026-10-01: all five targets
linked or run-tested; no foreign-arch execution claimed). What remains:
**§58 publication** (windows link+smoke quiet-window; crates.io/Homebrew/
winget await explicit user go), the **§15/§16 training path**, and
**§60 Amortyx integration** (designs exist; implementation is its own
promotion-gated effort).
SPEC_COVERAGE's table was also corrected 2026-09-30 against `routes.rs`:
no `/v1/systemone` HTTP route exists (the Jev shape is a library
adapter), and §22 batch inference is done end to end.

## Integration design notes (designs, not phases)

Backlog items that produce designs rather than code land here; each names
its doc of record.

- **Hosted API tier (task #41)** — designed, 2026-09-28:
  `docs/HOSTED_TIER.md`. A deployment mode, not a product mode: the
  identical binary, edge-owned auth/tenancy/quotas, VPS phases H0
  (single-principal loopback behind an authenticated edge) → H1
  (multi-principal, model lanes as workers) → H2 (deferred, saturation-
  gated), and a cost story priced only against measured anchors (1.3 ms
  engine / 613 ms fast / 4.9 s reference; $0.028–0.17/Mtok value anchor
  from the Amortyx receipts).
- **Amortyx integration (PLANNING.md §60–§62, task #40)** — designed,
  2026-09-28: `docs/INTEGRATION_AMORTYX.md`. Router decisions as typed IR
  (`amortyx.task_complexity` et al., §63 registry shape), ladder/latency
  lane mapping (deterministic rungs only on the hot path), loopback
  `/v1/decide` wire contract with abstain-degrades-to-heuristic, shadow
  mode riding Amortyx's existing session-sticky holdout, the VIVERE
  corpus path as read-only ledger consumer with no production claims,
  and §62 training with versioned datasets. No code yet by design —
  implementation follows the shadow-evidence promotion gate.
