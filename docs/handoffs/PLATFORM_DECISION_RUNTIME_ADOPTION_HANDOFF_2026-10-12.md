# Platform Decision Runtime Adoption Handoff — 2026-10-12

**Status:** handoff document. This is a plan for adopting the OpenCode
decision runtime on the platform — **not** a claim that any integration is
complete, and not a claim of test, build, or pipeline success. No build,
test, scan, or CI command has been executed while writing it; the only
commands executed during its production are the read-only HTTP GET probes
(`/v1/healthz`, `/v1/capabilities`, `/v1/models`) and `/proc` provenance
reads recorded in §3.1 and in the revision note below. Every acceptance
criterion below is something a future worker must make true and then prove.

**Revision — 2026-10-12 (post-initial-write, same day).** This revision
incorporates newer authoritative **read-only** runtime evidence for the
pre-existing `127.0.0.1:8177` service: `GET /v1/healthz`, `/v1/capabilities`,
and `/v1/models` probed at **2026-10-12 05:12:37–05:12:44 UTC**, all HTTP 200,
plus `/proc` provenance reads — full record in §3.1. The prior E3 state
("process observed, `/v1/healthz` NOT probed") is stale; it is preserved in
§3.1 as history only. Headline finding: **deployment/source drift** — the
live identity is `engine_semver` **0.4.0** against source **0.5.0**
(HEAD `b890edb`) — recorded as a §10 stop condition, **not** as success. No
test, build, scan, GitForge contact, or service mutation was performed for
this revision; the probes were read-only, and they are the only commands
executed while producing this document (the status statement above is
scoped accordingly).

**Worktree of record:** `/home/mkinney/worktrees/opencode-platform-adoption-20261012`
(branch `docs/platform-adoption-20261012`, isolated worktree; upstream repo of
record per `docs/PLAN.md` Phase 12: `/nas/Temp/repos/OpenCode`).

**Authoritative sources used:** repository `CLAUDE.md` (no `AGENTS.md` exists
in this tree), `README.md`, `SECURITY.md`, `docs/PLAN.md`, `docs/PLANNING.md`
(§§1–80; where PLANNING and PLAN/DECISIONS disagree, PLAN + DECISIONS win),
`docs/DECISIONS.md` (D1–D37, binding), `docs/INTEGRATIONS.md`,
`docs/INTEGRATION_AMORTYX.md`, and `.gitforge.yml`.

---

## 1. Objective

Adopt the OpenCode decision runtime as a working decision service on the
platform — through its existing public surfaces only — by (a) establishing a
fresh, verified source/runtime/CI baseline, (b) freezing the API contract and
selecting exactly one first consumer, (c) running an isolated canary, (d)
proving deterministic/fallback/abstention correctness, (e) wiring
observability, (f) releasing and promoting only on evidence, and (g) keeping
rollback available at every step.

OpenCode is a local-first, deterministic-first **decision runtime**: it
turns unstructured state into small typed decisions (choice / boolean / score)
with calibrated confidence, an abstention outcome, and a deterministic
execution trace. It is not a chatbot, not an LLM wrapper, and never writes
prose (README; PLANNING §73 Rule 10).

## 2. Scope and non-goals

**In scope:** verifying the runtime baseline; documenting the API contract of
record; selecting one consumer; canary operation on loopback; correctness and
observability evidence for the canary; the release/promotion path; the
rollback path; the boundaries in §8.

**Non-goals (explicit):**

- **No modification of Amortyx.** Amortyx is separate middleware/router and is
  out of scope for modification by this handoff. It appears only as a
  potential *consumer pattern* (worked example, D29). The 2026-10-04 Phase 19e
  Amortyx E2E was a historical exception with its own record
  (`docs/INTEGRATION_AMORTYX.md` §13); it grants no standing permission.
- **No VIVERE work.** VIVERE is experimental and distinct (see §8). No
  coupling, no corpus wiring, no claims.
- **No code moves, merges, or new coupling** between the runtime and any
  sibling service without an explicit, separately recorded decision.
- **No external publication.** crates.io / Homebrew / winget remain
  explicitly user-gated (D24; INTEGRATIONS.md §9 "19d").
- **No telemetry, no cloud calls, no API keys, no accounts** — ever
  (SECURITY.md invariant 1; PLANNING §73 Rules 13–14).
- **No unsupported savings or quality claims.** Nothing in this document
  asserts cost savings, latency wins, or accuracy improvements for the
  adoption; all such numbers must be measured in the target environment under
  the evidence rules in §4 before they exist as claims.

## 3. Current evidence confidence

Everything a future worker acts on falls into one of four evidence classes.
Do not upgrade a claim's class without doing the verification named in the
ledger.

| Class | What it covers | Confidence | Rule for workers |
|---|---|---|---|
| **E1 — Code-graph verified** | Structural facts from the code graph of project `nas-Temp-repos-OpenCode`, indexed 2026-10-12 (8,982 nodes, 35,751 edges; Rust packages — all 11 workspace members per
root `Cargo.toml`: `opencode-engine`, `-schema`, `-core`, `-model`,
`-runtime`, `-cli`, `-http`, `-mcp`, `-wasm`, `-ffi`, `-itemgen`);
re-confirmed by direct file reads on 2026-10-12 (anchors in §9.1) | High | Cite the anchor; re-verify only if the file changed |
| **E2 — Documented historical claims** | Phase statuses and measurements recorded in repo docs (e.g. Phase 19 done 2026-10-04; Phase 20 measured 98.39 % on 2026-10-08; Phases 21/22 open; Phase 24 in progress) | Medium — these are records, not fresh observations | Treat as history until re-verified at baseline (Task A2/A3); never cite as current state without the re-check |
| **E3 — Read-only endpoint-probed; drift-flagged** | The live 8177 service was probed read-only at **2026-10-12 05:12:37–05:12:44 UTC**: `GET /v1/healthz`, `/v1/capabilities`, `/v1/models` all returned **HTTP 200**; healthz reports `engine_semver` **0.4.0** against source workspace version **0.5.0** (HEAD `b890edb`) — **deployment/source drift** (full record: §3.1; the earlier process-only observation is preserved there as history) | Medium for liveness; **not fitness** | 200 is liveness only, never readiness or fitness; drift is a §10 stop condition; require source/build provenance + runtime identity match before any consumer traffic (A3 provenance gate; C1; C3); never restart or replace the running service outside the §6/§10 rollback/canary gates |
| **E4 — Operator-reported** | The current platform-side CI block designated **G7** (reported 2026-10-12). Not verifiable from this worktree — no network/GitForge contact was permitted while writing | Low | Re-verify at Task A1; until then, make no CI-evidence claims |

**E2 items this handoff specifically relies on (all documented historical
claims, not freshly verified):** Phase 19 adoption + integration documentation
done 2026-10-04 (`docs/INTEGRATIONS.md`, D30/D31, 19e Amortyx E2E); Phase 20
coverage restoration done with **98.39 %** (18 187/18 485, in-image lcov-DA
basis) measured 2026-10-08, floor fixed at 97.5, D36 ratchet retired; Phase 21
site WCAG/deployment **open** (opened 2026-10-06); Phase 22 docs-freshness
sweep **open** (status table 2026-10-06; the section body additionally records
a 2026-10-08 pass that has not been reflected in the table — reconcile in Task
A4); Phase 24 research enhancement **in progress** (2026-10-07; items 1–4,
8–10 and §15.8 fold-ins done; items 5/6 sized and deferred).

### 3.1 E3 record — read-only runtime probes, 2026-10-12 (supersedes the process-only state)

**History (superseded; preserved for the record).** Earlier on 2026-10-12, a
local process `opencode serve --bind 127.0.0.1:8177` was observed in the
host process list, and at that time **`/v1/healthz` had not been probed** —
the observation proved only that a binary had been started. That process-only
state is now historical: the read-only probes below supersede it. It is kept
here so the evidence trail shows exactly what was and was not known, and
when.

**Read-only probe evidence (GET only; 2026-10-12 05:12:37–05:12:44 UTC; no
service mutation of any kind):**

- `GET /v1/healthz`, `GET /v1/capabilities`, `GET /v1/models` on
  `http://127.0.0.1:8177` — all returned **HTTP 200** within that window.
- healthz body: `status=ok`; identity `engine_semver=0.4.0`;
  `model_id=relational-v1|builtin-lexical-v2|focused-v1@512`; `nodes=10`;
  `parallelism=16`; `cache=true`.
- capabilities: `max_batch=16`; `max_body_bytes=1048576` — consistent with
  the E1 anchors (`MAX_BATCH = 16`; 1 MiB body cap; §9.1).
- `/v1/models`: exactly one active decision model listed.

**Live-process provenance:**

- PID 2467422, started 2026-10-11 21:35:11 (host local time);
  `/proc/2467422/exe` resolves to
  `/nas/Temp/repos/OpenCode/target/release/opencode`; binary file
  mtime 2026-10-06 13:18:40 (host local time).
- Source of record at check time: HEAD `b890edb`, `Cargo.toml` workspace
  version **0.5.0**.

**Finding — deployment/source drift, not success.** A 200 is liveness, not
readiness or fitness; only liveness is demonstrated here. By the E1 code-graph
fact, `EngineIdentity.engine_semver` is derived automatically from the crate
version (§9.1). The builtin `model_id` is a versionless literal/composition
(`builtin-lexical-v1`, overwritten by the live classifier's composed id), not
a package-version string. A binary built from the current source (0.5.0)
therefore reports engine semver 0.5.0 — the live instance reports **0.4.0**,
so it is demonstrably not a build of the current source. This is recorded as
**deployment/source drift** and as a
§10 stop condition (item 4). No consumer traffic may route through this
instance (A3 provenance gate; C1; C3). Nothing in this handoff recommends
restarting or replacing the running service — any disposition goes through
the documented Phase G rollback/canary gates (§6, §10, D-O2); the probes
changed nothing on the service.

## 4. Falsifiability and evidence rules (apply to every task)

1. An acceptance criterion is **falsifiable** iff a worker can run a named
   command and get a pass/fail answer with no judgment call.
2. Every task names its **evidence artifacts** — files or captured outputs
   that exist after the task, independent of anyone's memory.
3. No claim of test/pipeline success exists in this handoff. "Green" is a
   property a worker demonstrates with captured exit codes, run ids, or
   byte-compared artifacts — after this document.
4. Performance or accuracy numbers produced under host contention are not of
   record (the repo's own quiet-host discipline, PLAN Phase 18i); reschedule
   the measurement, do not average the storm away.
5. Measurements are made in the target environment, against the pinned
   toolchain (`rust-toolchain.toml`; MSRV 1.90, D3). Local-toolchain coverage
   numbers never move floors or set expectations (D36 standing rule).

## 5. Phased task ledger (dependency-aware)

Dependency spine: **A → B → C → D → E → F**, with **G (rollback) prepared
before C carries any real traffic** and standing thereafter. Tasks inside a
phase may parallelize only where their inputs are disjoint. Do not start a
phase until the previous phase's acceptance artifacts exist.

### Phase A — Fresh source / runtime / CI baseline

Everything else depends on this phase. Its purpose is to replace every E2/E3/E4
item in §3 with an E1-class observation from today.

- **A1 — CI-of-record status (includes the G7 block).**
  Confirm the state of the GitForge pipeline (platform of record per
  `CLAUDE.md`; `.gitforge.yml` mirrors `just ci` line-for-line) at the
  baseline commit, and the scope/owner of the operator-reported **G7** block.
  Files: `.gitforge.yml` (six lanes: lint, test, doc, e2e, supply-chain,
  coverage), `justfile`.
  Acceptance: a written lane-status table (lane, run id, result, date) exists;
  if G7 still blocks, the table says so and Phase F promotion is explicitly
  gated on its resolution (see §7 D-O4, §10 stop condition).
  Evidence: the lane-status table; run ids from the GitForge UI/API of record.
  The GitHub Actions mirror is **not** a failure signal and never substitutes
  (repo policy, CLAUDE.md).
- **A2 — Source baseline.** On a clean checkout of the chosen baseline commit:
  run the documented gate (`just ci`). Its recipe chain `fmt-check lint test
  doc deny scan` expands to exactly these eight commands, in execution order:
  `cargo fmt --all -- --check`;
  `cargo clippy --workspace --all-targets -- -D warnings`;
  `cargo test --workspace`;
  `cargo test -p opencode-model --features llamacpp,tokenizers`;
  `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`;
  `cargo test --doc`;
  `cargo deny check`;
  `aegis --format json scan --file . --baseline .aegis/baseline.json --quiet`
  (expect 0 new findings).
  Files: `justfile`, `rust-toolchain.toml`, `.aegis/baseline.json`, `Cargo.toml`.
  Acceptance: `just ci` exits 0 — equivalently, all eight commands above exit
  0 on the pinned toolchain; the aegis JSON report shows 0 new findings.
  Evidence: captured exit codes for all eight commands (or the captured
  `just ci` transcript); the aegis JSON report saved out-of-tree.
- **A3 — Runtime baseline.** Build the release binary and validate it as the
  repo does: `cargo build --release`, then
  `python3 scripts/e2e_validate.py --binary <release-opencode> --port <port verified free>`
  (the CI lane uses port 8188; PLAN 18d/18e documents the check set). Probe
  `GET /v1/healthz`, `/v1/capabilities`, and `/v1/models` on a deliberately
  started instance and capture the bodies verbatim **with exact UTC
  timestamps**. The pre-existing 8177 process from §3 is already probed and
  recorded (§3.1: all three endpoints 200 at 2026-10-12 05:12 UTC, live
  identity **0.4.0** vs source **0.5.0** — deployment/source drift); that
  record stands as history, its disposition is owner decision D-O2, and it
  must never carry consumer traffic, nor be restarted or replaced outside the
  Phase G/§10 gates. **Provenance gate (binding before any consumer
  traffic):** the serving runtime's reported identity must match its
  source/build provenance of record — healthz `engine_semver` equals the
  workspace `Cargo.toml` version at the pinned baseline commit, and the
  binary's provenance (path, commit, file mtime/build time) is captured
  alongside the identity. The §3.1 drift instance fails this gate by
  definition; only a provenance-matched instance passes. Smoke the CLI
  contract:
  `cargo run -p opencode-cli -- decide --input fixtures/native/request.json`
  (documented in README; exit 2 / `verify` is the fixture's documented honest
  outcome, not a failure).
  Acceptance: e2e suite exits 0 with its full check count printed; healthz
  returns 200 with identity JSON captured verbatim **and** that identity
  matches the baseline source/build provenance (semver and model ids checked
  against the pinned commit's `Cargo.toml`); the fixture decide behaves
  exactly as documented; every endpoint capture carries its exact time.
  Evidence: suite stdout; healthz/capabilities/models bodies with UTC times;
  the provenance record (commit, workspace version, binary path + mtime);
  fixture command exit code + output.
- **A4 — Docs-freshness spot-check (feeds Phase B).** Reconcile the known
  discrepancies found while writing this handoff, each either fixed in the
  owning doc or recorded as an issue: (i) `INTEGRATIONS.md` §2.1 prints
  `opencode serve --addr 127.0.0.1:9200` while D12/D13 and PLAN record
  `serve` with default bind `127.0.0.1:8177` and a `--bind`-style flag — verify
  against `crates/opencode-cli/src/args.rs` and fix the doc or the flag
  usage text; (ii) the Phase 22 status-table/body mismatch in `docs/PLAN.md`
  (§3, E2). This task is part of the already-open Phase 22 sweep.
  Acceptance: each item has a one-line disposition (fixed-at-commit or issue
  recorded).
  Evidence: diffs or the issue notes.

### Phase B — API contract and consumer selection

- **B1 — Freeze the contract of record.** Produce a single contract sheet,
  mechanically checkable against the tree (the `INTEGRATIONS.md` honesty rule):
  the nine routes with methods from
  `crates/opencode-http/src/routes.rs:105-118` (`/v1/decide`,
  `/v1/batch`, `/v1/graph/validate`, `/v1/graph/run`, `/v1/validate`,
  `/v1/models`, `/v1/capabilities`, `/v1/healthz`, `/v1/chat/completions`),
  with the body limit applied at `routes.rs:116`
  (`DefaultBodyLimit::max(MAX_BODY_BYTES)`; documented as a 1 MiB socket cap,
  PLAN Phase 8–9); the six MCP tools (D17: `codify_decide`, `codify_batch`,
  `codify_graph`, `codify_validate`, `codify_verify`, `codify_explain`,
  stdio-only); CLI subcommands/flags from `crates/opencode-cli/src/args.rs`
  including the model-rung pairing rule (`--llama` requires
  `--llama-model-id`, `crates/opencode-cli/src/runtime.rs:86-100`,
  error `cli.model_rung_unavailable`); the D30 wire-format header
  (`x-opencode-format: native|openai|anthropic|jev`, native default,
  unknown → 400 `schema.invalid_value`); the error envelope
  `{"error":{"code","message"}}` with `ir.*` / `schema.*` / `engine.*`
  namespaces; the six IR outcomes (`accept`, `verified`, `verify`, `abstain`,
  `escalate`, `no_valid_candidate`); CLI exit codes 0/1/2/3 (D13).
  Acceptance: every route/tool/flag in the sheet carries a file anchor that a
  reviewer can open; zero aspirational entries.
  Evidence: the contract sheet (checked into docs or the consumer's harness
  config).
- **B2 — Select exactly one first consumer.** Candidate consumers, all over
  public surfaces (D29): a CI decision gate (INTEGRATIONS §7 pattern), an
  agent harness via MCP (§3), or an OpenAI-shaped router via
  `/v1/chat/completions` (§2.5). Selection criteria to apply and record:
  transport reachability (loopback-only), whether the consumer's payloads are
  already structured decisions, the explicit fail-open/fail-closed choice via
  `DecisionPolicy` gates, and the degrade path (connection refused, timeout,
  or `abstain` → the consumer's own prior behavior, byte-for-byte — D29).
  Confirming the pick is owner decision D-O1 (§7).
  Acceptance: a one-page consumer brief naming the chosen surface, the exact
  routes/tools/flags it will call (anchored per B1), its policy gates, and its
  degrade rule.
  Evidence: the consumer brief.

### Phase C — Isolated canary

- **C0 — Rollback readiness first.** Complete §6/§10 rollback preparation
  (Phase G tasks G1–G2) **before** the canary serves any consumer-shaped
  traffic.
- **C1 — Isolation envelope.** Start a fresh `opencode serve` on loopback
  at a port verified free at start (record it); do not share state with the
  pre-existing 8177 process; defaults for body/batch/limits (see §6); add
  `--graph` / `--ladder` / `--focus-budget` only if the B2 brief demands them
  — each decorates cache identity by construction (D6/D25/D27), which must be
  reflected in the recorded `/v1/models` identity. The canary instance must
  pass the A3 provenance gate (source/build provenance + runtime identity
  match) **before any consumer-shaped traffic**; the drifted 0.4.0 instance
  recorded in §3.1 is not eligible to serve the canary.
  Acceptance: server binds, healthz 200, the recorded identity matches the
  configured flags **and** the baseline build provenance, and all endpoint
  captures carry exact UTC times.
  Evidence: start command, healthz body, `/v1/models` + `/v1/capabilities`
  captures with timestamps, and the provenance record.
- **C2 — Canary checks (criteria to meet, not results).** All against the
  canary server: fixture-shaped decide round-trip; one committed recipe
  replayed and byte-compared against its captured response using the fresh-
  process discipline documented in `recipes/README.md`; a deliberate abstain
  (high-threshold policy in the request) returns HTTP 200 with a typed
  non-decisive outcome; an oversized body returns 413; a well-formed batch of
  ≤ `MAX_BATCH` (engine-owned, shared with MCP `codify_batch`; enforced at
  `routes.rs:350-352`) returns 200 with per-item results; a malformed body
  returns 400 with a `schema.*` code; a graph with a cycle refused
  `/v1/graph/validate` with `graph.cycle`; two identical decides through fresh
  processes produce identical decision content (determinism spot-check).
  Acceptance: every check passes with captured request/response pairs.
  Evidence: saved JSON pairs + a parity diff log.
- **C3 — Consumer shadow run.** Route the selected consumer's real decision
  questions through the canary in **record-only shadow mode** (the §61
  pattern: nothing acts on the outcomes). Log per item: request shape, outcome,
  calibrated confidence, trace id. **Pre-traffic gate:** no consumer question
  is routed until the A3 provenance gate and the C1 identity captures (with
  exact times) exist and match — a 200 from a drifted instance (§3.1) does
  not clear this gate.
  Acceptance: shadow log covers the agreed item count with zero behavior
  changes on the consumer side; the log records an outcome *distribution only*
  — no accuracy, savings, or latency claims (in-shadow, in-domain, small N);
  the pre-traffic gate evidence is attached.
  Evidence: the shadow log; the pre-traffic provenance/identity captures;
  the consumer-side config proving read-only mode.

### Phase D — Deterministic / fallback / abstention correctness

- **D1 — Determinism.** Double-replay the C3 corpus through fresh processes.
  Acceptance: decision content and `trace_version` identical on both passes,
  item for item.
  Evidence: both run logs + a byte-diff report.
- **D2 — Abstention semantics.** Verify across the surfaces the consumer
  uses: abstain is a typed outcome and HTTP 200 (never an error status); on
  the CLI it is exit 2 (D13) if the CLI path is in the brief.
  Evidence: captured abstain responses/exit codes.
- **D3 — Fallback and escalation honesty.** With a ladder configured
  (e.g. `--ladder ladders/fusion-v2.json`, plus the model rung flags only if
  the brief demands them): escalated items carry `rungs_fired` / `rung_chain`
  trace facts (D27) and `policy_source` where an override fires (D25). With
  nothing configured: behavior is byte-identical to the default engine — no
  ladder/focus/rung trace keys appear (D25/D27/D18 empty-default rule).
  Acceptance: both halves demonstrated on the canary corpus.
  Evidence: traces from a configured run vs an unconfigured run.
- **D4 — Cache honesty.** An identical repeat reports a cache hit with
  identical content; changing the ladder/model/calibration identity re-keys
  (spot-assert via the identity the server reports), so stale decisions cannot
  be served across artifact changes (D6; PLAN Phase 14).
  Evidence: before/after identity captures + the cache-hit pair.
- **D5 — Degrade path under failure.** Stop the canary server mid-exercise;
  the consumer falls back to its own prior behavior per its B2 brief (D29
  rule: no consumer availability may depend on the runtime's availability).
  Acceptance: the degrade is demonstrated, not argued — the consumer's fallback
  output for the stopped-server window is captured.
  Evidence: consumer-side log for the outage window.

### Phase E — Observability

- **E1 — Trace capture.** Every canary decision's deterministic execution
  trace retained (this is the only explanation surface — no chain-of-thought
  exists anywhere, PLANNING §65). Trace facts to expect per configuration:
  `focus_*` when a focus budget is set (D18), `policy_source` /
  `calibration_source` for ladder overrides (D25), `rungs_fired` /
  `rung_chain` for escalations (D27), `boolean_threshold` /
  `boolean_flipped` / `abstain_candidate` / `abstain_elicited` only when the
  corresponding policy fields are configured (D25 amendments), absent
  otherwise.
  Acceptance: the evidence bundle pairs every decision with its trace.
  Evidence: the trace store (files keyed by request id).
- **E2 — Identity at boundaries.** `/v1/models` and `/v1/capabilities`
  captured at canary start, after any reconfiguration, and at teardown —
  attached to every evidence bundle so any number produced is attributable to
  a named engine identity.
  Evidence: the identity captures.
- **E3 — Resource accounting.** Per-request wall time and process peak RSS
  recorded using the repo's own `/proc` monitor pattern
  (`benchmarks/decision-model/runner/resources.py`) or an equivalent; compare
  against the D9 per-stage budgets **as measurement only** — no budget claims
  until measured on quiet host (§4 rule 3/4).
  Evidence: the resource rows in the run log.
- **E4 — CI-side observability debt.** GitForge's jobs-level API returns
  empty for finished runs, so lane logs must carry their own numbers — the
  pattern is `scripts/coverage_floor.py`, which prints the measured value and
  floor on pass and fail (D36 decision 2; the API fix is tracked against
  GitForge itself, PLAN Phase 20h, not this repo). Any new lane or check added
  for adoption must print its own number the same way.
  Evidence: the lane-log snippets.
- **E5 — Coverage lane unchanged.** The coverage lane stays as-is: floor
  fixed at 97.5 (in-image lcov-DA basis; D36 update 2026-10-08 retired the
  ratchet at the reachable ceiling — the E2 98.39 % number is the documented
  measurement behind that decision). No adoption work may lower or route
  around the floor.
  Evidence: the lane definition in `.gitforge.yml` (unchanged) at the
  promotion commit.

### Phase F — Release / promotion

- **F1 — Promotion gate.** Promotion may start only when: A1's lane table is
  green at the candidate commit (or the G7 block is resolved and re-baselined,
  §7 D-O4); A2–A4, B, C, D, E artifacts all exist and pass their criteria.
- **F2 — Release recipe (documented path; do not improvise).** Follow the
  Phase 23 discipline in `docs/PLAN.md` and the justfile targets:
  `just release-preflight`, `just release-build <target>`,
  `just release-matrix`, `just release-checksums`, `just release-notes`
  (rendered from attestations over `docs/RELEASE_NOTES_TEMPLATE.md`;
  `scripts/generate_attestation.py`, schema `opencode.attestation/1`),
  then `just release-verify`. All legs from **one tree** (the
  `cargo_tree_sha256` single-tree rule; the v0.5.0 mixed-tree first pass was
  discarded for exactly this — PLAN Phase 23). Foreign-arch targets carry
  their named verification level (link + `file` magic), never a "supported"
  claim (D24).
  Acceptance: `just release-verify` passes on every attested leg; release
  notes quote only measured numbers with sources.
  Evidence: the attestation set, `sha256sums.txt`, rendered notes.
- **F3 — Tag and publish where policy allows.** SemVer tag; release on
  GitForge (platform of record); GitHub mirror release per the Phase 23 step
  list. Registry publication (crates.io / Homebrew / winget) stays
  **excluded** without an explicit user go (D24; INTEGRATIONS §9).
  Acceptance: `git describe` matches the release; pipeline of record green at
  the tag.
  Evidence: tag, release URLs, run id at the tag.
- **F4 — Consumer promotion.** Shadow → advisory → active only on the
  consumer side's own measured would-have-positive evidence, gathered over
  time (the §61/`INTEGRATION_AMORTYX.md` promotion pattern); the degrade path
  of D5 stays armed at every stage. This handoff does not schedule that
  promotion — it defines the gate.
  Evidence: the consumer's promotion memo when it eventually happens.

### Phase G — Rollback (prepared before the canary; standing thereafter)

- **G1 — Consumer rollback.** The consumer's adoption config carries an
  explicit kill switch (stop routing to the runtime). On activation, the
  consumer reverts to its prior behavior byte-for-byte (D29 §4 posture).
  Acceptance: the kill switch is demonstrated in D5's outage exercise, not
  merely documented.
  Evidence: D5's captured outage-window log.
- **G2 — Runtime rollback.** The server is stateless per request and caches
  are in-process, so rollback is: stop the canary process; restore the
  previous binary/tag. Cache keys fold graph/model/calibration/policy
  versions (D6), so a version change cannot serve stale decisions across a
  rollback.
  Acceptance: a rollback drill (stop → restore → healthz 200 → identity
  matches the previous tag's build) completed once before F3.
  Evidence: the drill log with identity captures.
- **G3 — Release rollback.** The previous tag remains the release of record
  until the new one passes F1–F3; attestations + `sha256sums.txt` make any
  artifact swap detectable (`generate_attestation.py --check`).
  Evidence: the retained previous-release artifacts.

## 6. Security, privacy, and resource/load controls

**Security/privacy boundaries (binding; violations are security bugs per
`SECURITY.md`):**

1. No telemetry, no cloud calls; the runtime opens no network connections
   except an explicitly flagged server bind or an explicit model install.
2. Data is not executable: state text, candidate/tool descriptions, schemas,
   and manifests never modify policy, thresholds, graph structure, paths, or
   permissions (PLANNING §66; enforced posture, not aspiration).
3. Input is hostile: parsers enforce resource limits and never panic on
   malformed input (SECURITY.md 3; §73).
4. Model integrity: SHA-256 manifests (D14); weights never enter the
   repository; a mismatch refuses before any tensor is read.
5. Local bind default: `127.0.0.1` unless an explicit non-loopback host flag
   is passed; MCP is stdio-only by construction (D17). The canary binds
   loopback — always.
6. One error contract (`{"error":{"code","message"}}`, D30) so consumers
   distinguish "your payload is wrong" (`schema.*`, 4xx) from "the runtime
   could not decide" (typed outcome, 200) and "engine fault" (5xx).
7. Safe mode refuses semantic pruning (`retrieve` nodes) unless explicitly
   disabled per engine (D21); input text cannot flip it.

**Resource/load controls (defaults; do not loosen for the canary):**

- Body cap at the socket: `DefaultBodyLimit::max(MAX_BODY_BYTES)` applied in
  the router, `routes.rs:116` (documented as 1 MiB, PLAN Phase 8–9).
- Batch bound: engine-owned `MAX_BATCH = 16`, enforced at
  `routes.rs:350-352`, shared with MCP `codify_batch`; per-item error
  envelopes; the transport stays 200 for a well-formed batch.
- Per-request execution ceiling: 120 s (D32), validated at the wire and
  min-ed with the server's own config; §67 Limits defaults (max input bytes,
  tokens, candidates, questions, graph nodes 128, execution time, memory,
  retrieval results) stay conservative.
- Sync CPU work off async workers: `spawn_blocking` in the batch path
  (`routes.rs:357-364`); the engine core itself is sync (D5).
- Host discipline: CI job containers are capped (4 GiB; `-j 4` sizing in
  `.gitforge.yml`) and measurement runs need quiet-host windows (PLAN 18i);
  load-confounded numbers are not of record.
- Supply-chain gates ride every change: `cargo deny check`, `cargo audit`,
  aegis 0-new-findings against `.aegis/baseline.json` (the `supply-chain`
  lane).

**GitForge-only validation policy and the current G7 block.** GitForge is the
CI platform of record; `.gitforge.yml` mirrors `just ci` line-for-line; the
GitHub Actions config is a mirror and a red run there is not a code-failure
signal (CLAUDE.md; `.gitforge.yml` header). The operator reported a current
platform-side block designated **G7** (2026-10-12, E4): until A1 re-verifies
and clears it, no CI evidence is claimable and Phase F is gated. There is no
approved alternative validation channel — inventing one (e.g. leaning on the
GitHub mirror) is a policy violation, not a workaround.

## 7. Owner decisions required (only these)

Plain language; each blocks exactly the tasks listed.

- **D-O1 — Which consumer goes first?** (blocks B2→C.) The candidates are the
  CI gate, the agent harness (MCP), or an OpenAI-shaped router client. The
  owner picks one; the others wait for a later cycle.
- **D-O2 — What happens to the already-running server on 127.0.0.1:8177?**
  (blocks A3 sign-off.) *Updated 2026-10-12:* the keep-and-probe half already
  happened — §3.1 records the read-only probes (all endpoints 200,
  05:12:37–05:12:44 UTC) and the finding: live identity **0.4.0** against
  source **0.5.0**, i.e. deployment/source drift, unfit for consumer traffic.
  The remaining decision is disposition only: leave it untouched as a
  recorded environment fact while the baseline/canary run on a separate,
  provenance-matched instance, or retire it — retirement itself goes through
  the documented Phase G/§10 rollback and canary gates, not an ad-hoc restart
  or replacement. Silently reusing it for traffic is not an option.
- **D-O3 — Is any external publication in scope this cycle?** (blocks nothing
  by default.) Default is **no** (it stays user-gated per D24). Only an
  explicit go opens it.
- **D-O4 — If the G7 block persists, what is the accepted way to proceed?**
  (blocks A1 sign-off and all of Phase F.) The options are: wait for GitForge,
  or record an explicit, time-boxed manual-gate protocol signed by the owner.
  Using the GitHub mirror as a substitute is not an option.

## 8. Architecture boundaries — explicit separation

- **OpenCode** is an independent, deterministic-first decision runtime.
  Its identity and its cheapest-first execution ladder (exact rule → cache →
  metadata filter → lexical → embedding → small classifier → decision model →
  verifier → external model) are the product (CLAUDE.md; README).
- **Amortyx** remains separate middleware/router, out of scope for
  modification here. The boundary is doctrinal and recorded (PLANNING §60;
  `INTEGRATION_AMORTYX.md` §1): OpenCode makes semantic decisions and
  never sees provider keys or prices; Amortyx makes economic/operational
  decisions and never guesses semantics; neither absorbs the other. First-party
  consumers get exactly the public surfaces third parties get (D29) — no
  privileged endpoints, no in-process back doors.
- **VIVERE** is experimental and distinct: in the recorded design notes it
  appears only as a read-only ledger consumer with no production claims
  (`docs/PLAN.md` integration design notes). It has no role in this adoption.
  Nothing here recommends coupling VIVERE, Amortyx, or OpenCode code, or
  moving code between them — that requires an explicit decision record of its
  own.

## 9. Precise pointers for future workers

### 9.1 Code-graph facts (E1; project `nas-Temp-repos-OpenCode`, indexed 2026-10-12; 8,982 nodes / 35,751 edges)

- Rust packages — all 11 workspace members (root `Cargo.toml`):
  `opencode-engine`, `opencode-schema`, `opencode-core`,
  `opencode-model`, `opencode-runtime`, `opencode-cli`,
  `opencode-http`, `opencode-mcp`, `opencode-wasm`,
  `opencode-ffi`, `opencode-itemgen`.
- HTTP router registration — `crates/opencode-http/src/routes.rs:105-118`
  (`pub fn router`): `/v1/decide`, `/v1/batch`, `/v1/graph/validate`,
  `/v1/graph/run`, `/v1/validate`, `/v1/models`, `/v1/capabilities`,
  `/v1/healthz`, `/v1/chat/completions`; the body limit is applied in the same
  builder (`routes.rs:116`).
- Batch handler — `routes.rs:339-367`: enforces `MAX_BATCH`
  (`routes.rs:350-352`, `schema.limit_exceeded` above it) and moves the sync
  CPU work off the async workers via `spawn_blocking`
  (`routes.rs:357-364`).
- Model-rung flag pairing — `crates/opencode-cli/src/runtime.rs:86-100`
  (`model_rung`): `--llama` without `--llama-model-id` is refused with
  `cli.model_rung_unavailable` (the model id discriminates decision cache
  keys, D6).
- Engine identity ↔ crate version — code-graph confirmed (2026-10-12,
  project `nas-Temp-repos-OpenCode`): `EngineIdentity.engine_semver` is
  "Bumped automatically from crate version" and uses
  `env!("CARGO_PKG_VERSION")` (`crates/opencode-engine/src/cache.rs:66-88`).
  Separately, the builtin `model_id` is the literal `builtin-lexical-v1`
  (`cache.rs:86`), overwritten by the live classifier's model id; classifier
  ids are literals/compositions (`classifier.rs:190`, `relational.rs:56`,
  `engine.rs:343`) and are not package-version-derived. Only `engine_semver`
  grounds the §3.1 drift finding: a binary built from workspace 0.5.0 reports
  semver 0.5.0; the live 8177 instance reports 0.4.0, so it is not a build of
  the current source.

### 9.2 Files and modules (read before working; none edited by this handoff)

`justfile` (gate + release + wasm recipes); `.gitforge.yml` (six lanes);
`.aegis/baseline.json`; `rust-toolchain.toml`; `crates/opencode-cli/src/args.rs`
(CLI surface); `crates/opencode-cli/src/runtime.rs`; `crates/opencode-http/src/routes.rs`;
`crates/opencode-mcp/src/lib.rs` (six tools); `crates/opencode-engine/src/ladder.rs`
and `ladder_fit.rs` (D25); `ladders/fusion-v1.json`, `ladders/fusion-v2.json`,
`ladders/proofs-only-v1.json` (shipped profiles); `recipes/` (fleet + captured
responses; fresh-process comparison rule in `recipes/README.md`);
`fixtures/native/request.json` (README quickstart fixture);
`scripts/e2e_validate.py` (release-binary suite); `scripts/coverage_floor.py`;
`scripts/generate_attestation.py`; `docs/{PLAN,PLANNING,DECISIONS,INTEGRATIONS,
INTEGRATION_AMORTYX,ARCHITECTURE,COVERAGE,BENCHMARKS,REGISTRY}.md`;
`docs/RELEASE_NOTES_TEMPLATE.md`.

### 9.3 Commands (documented, not executed — the §3.1 read-only probes excepted)

The build/test/scan commands below were **not** executed while writing this
handoff. The only commands executed during its production are the read-only
HTTP GET probes and `/proc` provenance reads recorded in §3.1; no build,
test, scan, or CI command was run, and the service was never mutated.

```bash
just ci                      # the eight-command local gate (= .gitforge.yml)
cargo build --release
python3 scripts/e2e_validate.py --binary <release-opencode> --port <free-port>
cargo run -p opencode-cli -- decide --input fixtures/native/request.json
cargo run -p opencode-cli -- graph validate <graph.json>
just check-wasm              # wasm32 check + Node smoke (not part of just ci)
just coverage                # line coverage (cargo llvm-cov)
just release-preflight && just release-matrix && just release-verify
curl -s http://127.0.0.1:<port>/v1/healthz
```

`<release-opencode>` / `<free-port>` / `<graph.json>` are command
parameters for the worker to fill from the actual build and environment —
the port must be verified free at use.

## 10. Stop and rollback conditions

**Stop conditions — any one halts forward progress until resolved:**

1. Any `just ci` gate red on the baseline commit (A2 fails).
2. aegis reports ≥ 1 new finding vs `.aegis/baseline.json`.
3. The e2e suite is not fully green on the release binary (A3 fails).
4. **Deployment/source drift:** the serving runtime's reported identity does
   not match its source/build provenance of record (the A3 provenance gate).
   Observed and standing as of 2026-10-12 (§3.1): the 8177 instance reports
   `engine_semver` **0.4.0** against source workspace **0.5.0** at HEAD
   `b890edb`. Its 200-responsiveness is liveness only and does not clear
   this condition; no consumer traffic routes to it.
5. A canary parity check fails (C2), or any accepted decision's trace
   contradicts the recorded engine identity (E2 mismatch).
6. Any SECURITY.md invariant (1–5) is violated or credibly suspected —
   telemetry attempt, non-loopback bind without the explicit flag, input
   text influencing policy/structure, an unverified model artifact.
7. GitForge remains blocked (G7) with no owner-signed path forward (D-O4).
8. Host contention storms measurement windows repeatedly (reschedule;
   do not record load-confounded numbers).

**Rollback:** per Phase G — G1 (consumer kill switch; degrade-to-prior-
behavior byte-for-byte), G2 (stop/restore the process; cache-key versioning
prevents stale decisions across versions), G3 (previous tag stays the release
of record until the new one fully passes). Abstention and outage are
non-failures by design: a runtime that abstains, or is down, hands the
decision back to the consumer's own prior behavior (D29) — that is the
rollback path working, not a defect. The §3.1 drifted instance (live 0.4.0 vs
source 0.5.0) is likewise handled only through these gates — e.g. a G2
stop/restore drill with identity captures, or an owner-signed D-O2
disposition; this handoff neither recommends nor authorizes an ad-hoc restart
or replacement of the running service.

---

*End of handoff. Next action for the adopting worker: Phase A, task A1 —
re-verify the GitForge/G7 state — then A2 on a clean checkout. Update §3's
evidence classes as claims are upgraded; do not cite E2/E3/E4 items as
current facts after better evidence exists. E3 is upgraded for liveness only
(§3.1, 2026-10-12 05:12 UTC): cite the 8177 service as "alive but drifted
(live 0.4.0 vs source 0.5.0)" — never as ready, fit, or cleared for consumer
traffic.*
