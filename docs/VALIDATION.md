# Real-World Validation Campaign — design + runbook

The design of record for the real-world validation campaign (three workload
classes against the shipped surfaces). Design and runbook only: nothing in
this document is a measured result. Measured numbers quoted here are
committed anchors from earlier phases, cited so the acceptance gates are
traceable — every gate below is either derived from `docs/DECISIONS.md` D9
(per-stage budgets) and D32 (the 120 s execution ceiling) or named as an
open question.

Method of record follows the house validation discipline: enumerate the
feature surface from the binary before driving it; test real workload
classes, never happy-path one-shots; dual-arm every claim (direct engine vs
through-product); store findings as artifacts before the task closes;
silence is a finding.

**Status:** designed 2026-10-05, not executed. The campaign's evidence
lands in `benchmarks/validation/results/VALIDATION.md` (committed) with raw
runs out of tree; defect rows are filed into `docs/PLAN.md` before the task
closes.

---

## 1. Scope

### 1.1 What "real-world validation" means here

OpenCodifier is a local-first decision runtime with three shipped
interfaces over one engine (`opencodifier_engine::EngineHandle`): HTTP
(axum 0.8, loopback), MCP (rmcp 2.2, stdio), and the CLI. Everything
measured so far about it is single-shot and serial: one request, one
client, one process, one decision (`benchmarks/decision-model/` — 49 board
runs; `benchmarks/baselines/criterion/` — the D9 stage estimates). None of
that exercises the three properties a deployment actually depends on:

1. **Chains** — a caller that keeps state across decisions and re-enters
   the runtime dozens of times, with the state growing each time.
2. **Concurrency** — many clients at once, where the interface crate owns
   threading (`spawn_blocking` off the async workers, D5) and the engine is
   sync single-threaded per request.
3. **Scale of input** — states far larger than the benchmark suite's ~20
   words, where the deterministic layers still have to decide and where a
   configured model rung starts to cost real seconds (D32's measured ~22 s
   anchor).

"Real-world validation" is therefore: drive the shipped binary through
those three workload shapes, on the compute host, with the same evidence
discipline as the model-pick benchmark, and record what breaks. It is not
an accuracy campaign — the board already owns accuracy. Correctness appears
only as a *guard* (a workload that silently changes answers is a defect, not
a benchmark row).

### 1.2 The three workload classes

| class | shape | primary question |
|---|---|---|
| **A. Agentic multi-turn loop** | 24 sessions × 50 turns; each turn's decision feeds the next turn's state; state grows monotonically | does the runtime hold up as a *loop component* — no drift, no unbounded growth, no cross-session bleed? |
| **B. Concurrent burst / queue** | 8 / 32 / 128 concurrent clients against `opencodifier serve` | what does the HTTP tier do at saturation — latency, error discipline, memory, liveness? |
| **C. Long context** | states at 4k / 32k / 128k estimated tokens through every pipeline tier | where do the deterministic layers stop being cheap, where does focus extraction engage, and where would escalation fire? |

### 1.3 Evidence discipline (binding)

- Every runner imports the shared helpers — `ResourceMonitor`
  (`benchmarks/decision-model/runner/resources.py`) and `load_now()` /
  `wait_health()` / `post()` (`benchmarks/decision-model/runner/run_llama.py`,
  `run_engine.py`) — via `sys.path.insert`, the pattern `run_stock.py`
  already uses to import from `run_llama.py`. No copies, no forks.
- Every run JSON carries: `suite_sha256`, `binary_sha256`, `git_rev`,
  `config.host`, `load_avg {start, end}`, `resources` (peak RSS, CPU-s,
  IO, wall), and a `determinism` block from a full replay.
- Latency is only comparable within a recorded load regime (gate policy
  v2, §8.4); accuracy and determinism are load-invariant and are the
  gate.
- Raw run JSONs stay out of tree. The repo commits the merged report and
  the byte-locked generators, exactly as `benchmarks/decision-model/`
  does (`results/summary.md` committed, run JSONs regenerable).
- `aegis --format json scan --file . --baseline .aegis/baseline.json`
  runs after every content edit of this campaign — including this
  document and the new runner files — and any new findings are triaged
  into the baseline in the same commit (the Phase 12 discipline
  correction). Measured at design time: this document alone carries **95
  new findings, all detector misfires on prose** (33 `magic-number` —
  ports, sizes, counts; 16 `excess-line-length`; 11
  `interpretability-tool`; 10 each `hardcoded-ip`/`ssrf-localhost` on
  `127.0.0.1:<port>` loopback documentation; 9 `markdown-code-fence`;
  the rest singletons) — the same misfire class PLAN Phase 12 triaged
  for the CI docs. The baseline is **not** regenerated in this commit:
  the working tree carries unrelated WIP (`docs/RESEARCH.md`,
  `scripts/generate_attestation.py`) that a directory regen would bake
  in. Triage happens in the commit that lands this document.

---

## 2. Step 0 — feature-surface enumeration

The campaign starts by enumerating what the binary actually exposes, from
the binary and the route table — not from the docs. Any feature found in
code but absent from the enumerated surface, or on the surface but absent
from code, is finding RV-000 (§9.3).

### 2.1 CLI surface (clap walk)

```bash
cd /nas/Temp/repos/OpenCodifier   # fedora: ~/repos/OpenCodifier (§8.1)
B=target/release/opencodifier
cargo build --release -p opencodifier-cli
$B --help            > /tmp/rv-surface-cli-root.txt
$B decide --help     > /tmp/rv-surface-cli-decide.txt
$B graph --help      > /tmp/rv-surface-cli-graph.txt
$B graph validate --help > /tmp/rv-surface-cli-graph-validate.txt
$B serve --help      > /tmp/rv-surface-cli-serve.txt
$B mcp --help        > /tmp/rv-surface-cli-mcp.txt
$B mcp serve --help  > /tmp/rv-surface-cli-mcp-serve.txt
$B models verify --help > /tmp/rv-surface-cli-models.txt
$B recipe list       > /tmp/rv-surface-cli-recipe-list.txt
$B recipe install --help > /tmp/rv-surface-cli-recipe-install.txt
```

Expected (from `crates/opencodifier-cli/src/args.rs`): seven leaf commands
(`decide`, `graph validate`, `serve`, `mcp serve`, `models verify`,
`recipe list`, `recipe install`); `decide` flags `--input`, `--format
native|openai|anthropic|jev`, `--policy`, `--trace`,
`--abstain-is-success`, `--focus-budget`, `--ladder`,
`--llama/--llama-model-id/--llama-timeout-ms`; exit codes 0 / 1 / 2 / 3
per D13. Every `--help` must exit 0 and contain no ANSI escapes when
piped (the Phase 11 accessibility check, re-proven here).

### 2.2 HTTP surface (route table)

```bash
grep -n '\.route(' crates/opencodifier-http/src/routes.rs
grep -n 'MAX_BODY_BYTES\|FORMAT_HEADER' crates/opencodifier-http/src/routes.rs | head
```

Expected — the nine routes registered by `opencodifier_http::router`
(`routes.rs:105`):

| method | path | notes |
|---|---|---|
| POST | `/v1/decide` | `x-opencodifier-format` selects the adapter (D30) |
| POST | `/v1/batch` | `MAX_BATCH = 16`, shared with MCP `codify_batch` |
| POST | `/v1/graph/validate` | |
| POST | `/v1/graph/run` | client graph, content-addressed identity (D19) |
| POST | `/v1/validate` | decode-only preflight |
| GET | `/v1/models` | |
| GET | `/v1/capabilities` | |
| GET | `/v1/healthz` | |
| POST | `/v1/chat/completions` | V1 chat surface (Phase 19e) |

Body cap `MAX_BODY_BYTES = 1_048_576` (413 above it). Absent from the
surface by decision, and the enumeration asserts their absence:
`/v1/systemone` (D31), any Anthropic/OpenAI-shaped route (those are
library adapters), any engine-configuration route on `/v1` (D25).

### 2.3 MCP surface

```bash
grep -n 'name = "codify_' crates/opencodifier-mcp/src/lib.rs
```

Expected: `codify_decide`, `codify_batch`, `codify_graph`,
`codify_validate`, `codify_verify`, `codify_explain` (D17). Stdio only;
`mcp serve --graph|--focus-budget|--ladder|--llama…`.

### 2.4 Runtime self-declaration vs code

```bash
$B serve --bind 127.0.0.1:8092 &   # pre-flight port check first (§8.3)
curl -s http://127.0.0.1:8092/v1/healthz | python3 -m json.tool
curl -s http://127.0.0.1:8092/v1/models    | python3 -m json.tool
curl -s http://127.0.0.1:8092/v1/capabilities | python3 -m json.tool
```

The `identity` block (`model_id`, `graph_version`, `calibration_version`,
`engine_semver`, `embedding_model`) is what every cache key folds (D6); it is recorded into
every run JSON and must be identical across all arms of the campaign.

**What the enumeration is for:** the campaign then drives exactly this
surface, no more. Features that exist in code with no surface path (e.g.
`NodeSpec::threshold`, validated and fingerprinted but unwired per D25)
are not validated by this campaign and are recorded as such — an
unwired feature is a code fact, not a validated capability.

---

## 3. Workload class A — agentic multi-turn loops

### 3.1 Generator

New file `benchmarks/validation/suite/generate_workloads.py`, stdlib-only,
seeded, byte-locked (`--check` re-derives and compares bytes). It emits
`session_plan.json`: 24 sessions × 50 turns.

Each session is an incident-triage loop over a fixed entity set (12
entities, names from the same `[A-Za-z0-9_-]` alphabet the relational fact
grammar accepts, ≤ 64 chars). Turn *t* of a session is:

1. **Mutate** — one entity transitions health (`healthy → degraded → down`
   or back), or one dependency edge is added, deterministically from the
   seeded plan.
2. **Render state** — the session's accumulated prose: a header line, then
   one line per event so far (so state grows by ~1–3 sentences per turn;
   the 50-turn terminal state is ~2k estimated tokens by the engine's
   bytes-over-4 accounting), plus `state.facts` carrying the same
   mutations as typed facts.
3. **Ask** — one Choice question over 4 candidates drawn from the current
   entity set: "which single entity is the root cause" (transitive
   dependency closure) or "which entity must be restored first" (gates
   something, waits for nothing). Both operators are the Phase 15
   relational solver's exact proofs, so the correct candidate is derivable
   from the state by construction and is recorded as the gold answer.
4. **Repeat probe** — every 20th turn re-issues turn *t−5*'s exact request
   body unchanged (an idempotent re-ask, the common agent-loop pattern).
   It must come back from the exact-decision cache.

Ground truth is by construction, like the Phase 13 suite; the generator
carries the same self-audit as `runner/audit_suite.py` (a sibling
`audit_workloads.py` re-derives every gold answer from the rendered state
and refuses on any mismatch).

### 3.2 Traffic

| parameter | value | rationale |
|---|---|---|
| sessions | 24 (run 1), 8 concurrently interleaved (run 2) | run 2 is a cross-session-bleed check, not the burst class |
| turns / session | 50 | 1,200 decisions per run; the loop shape, not volume, is the subject |
| client timeout | 30 s | below the D32 ceiling by 4×; a hang is an observable |
| endpoint | `POST /v1/decide`, native | the interface real integrations use |
| policy | `min_confidence 0.8 / verify_below 0.65 / abstain_below 0.5 / risk low` | the request policy every recorded engine arm uses |
| limits | D32 ceiling: `max_execution_time 120 s`, `max_input_bytes 1 MiB`, `max_candidates 256` | the wire contract as shipped |

### 3.3 Metrics per request

- client wall (ms); HTTP status; typed `outcome` (`accept`/`verified`/
  `verify`/`abstain`/`escalate`/`no_valid_candidate`).
- the response `metrics` object, exactly its four wire fields:
  `cache_hit`, `candidates_in`, `candidates_out`, `verification_triggered`
  (`crates/opencodifier-core/src/trace.rs`). Narrowing efficiency is
  computed client-side as `candidates_out / candidates_in`
  (`DecisionMetrics::reduction_ratio` exists in Rust but is not on the
  wire).
- deciding-node evidence from the trace: the threshold entry's
  `outcome`/`verifier` facts, `policy_source` when a ladder override
  fired, `rungs_fired` + `rung_chain` when the escalation walk ran, and
  the per-node facts each stage records (`normalize`: `state_bytes`/
  `facts`; `rule`: `fired`/`facts_set`/`excluded`/`pinned`; `cache`:
  `hit`/`key`; `focus_*` when a focus budget is configured). The node
  that produced the distribution is *inferable* from these entries —
  `decided_by` is `pub(crate)` and never serialized (§9.3).
- per-turn session index, so latency can be plotted against turn number.

### 3.4 Acceptance

| gate | criterion | source |
|---|---|---|
| op-success | 100 % HTTP 200 in every run — 1,248 requests each (1,200 decisions + 48 repeat probes), 3,744 across the serial, interleaved, and focused runs. An abstain is a 200 + typed outcome, not an op failure. Any 5xx, connection error, or timeout fails the class. | §73 posture |
| answer correctness | ≥ 0.95 per class over gold answers (the engine's measured relational-class floor is 0.950, PLAN Phase 15); the repeat probes must be 100 % correct | PLAN Phase 15 anchor |
| replay determinism | full 1,200-decision chain re-run: answers, outcomes, and confidences byte-identical | D7 / Phase 13 practice |
| cache integrity | every repeat probe reports `cache_hit: true` with an answer identical to the original turn; zero cross-session wrong answers attributable to cache (would surface as a correctness miss) | D6 |
| per-turn latency | p99 ≤ 12 ms (D9 deterministic path 10 ms + HTTP overhead 2 ms) at the ≤ 2k-token turn shape | D9 |
| growth slope | linear fit of per-turn p50 against turn index: slope ≤ 0.05 ms/turn. Super-linear growth (BM25 index rebuild over ever-longer state) is the class-A failure mode this gate exists to catch | this campaign |
| abstain rate | ≤ 20 % of non-probe turns; above that the deterministic stack is degrading on its own proof-shaped input | PLAN Phase 15 (0.950 relational) |
| focus A/B | a third run at `--focus-budget 512` is answer-identical to the full-state run on ≥ 99 % of turns | D18 discipline (120/120 at ~3.7k tokens) |

---

## 4. Workload class B — concurrent bursts and queues

### 4.1 Generator

`benchmarks/validation/runner/run_burst.py`. Bodies are drawn from the
Phase 13 suite's item shapes (context + choice question) and are
**unique per request** — the exact-decision cache must not absorb the
burst (the F-3 lesson from the Phase 19e Amortyx drill: an exact-match
cache serves 200s through an outage if bodies repeat). Uniqueness comes
from a per-request counter folded into the state text as a trailing
sentence that names no candidate id and matches no relational-fact
pattern, the same ignorable-padding rule `make_long_suite.py` enforces
with an assertion.

A second pass at each level re-sends a fixed 128-body set twice to
measure cache-hit behaviour under concurrency (the only pass where hits
are expected).

### 4.2 Traffic matrix

| level | clients | requests | bodies | purpose |
|---|---:|---:|---|---|
| 1 | 8 | 1,024 | unique | baseline concurrency |
| 2 | 32 | 1,024 | unique | scaling |
| 3 | 128 | 1,024 | unique | saturation |
| 4 | 8 | 256 | 128 × 2 | cache-hit pass |
| 5 | 128 | 1,024 unique + 51 malformed (5 %) | mixed | error discipline under load |

Client: stdlib threads, one connection per client, per-request timing,
client timeout 30 s. Queue depth is *observed*, not configured: the
shipped server has no admission control — every request becomes one
`spawn_blocking` task (routes.rs `decide`) — so outstanding-request count
(client-side), server RSS, and `/proc/loadavg` are the queue evidence.

### 4.3 Metrics

p50/p95/p99 client wall, throughput (req/s), op-success rate, error
classes (4xx by code, 5xx by code, timeouts, connection resets), cache-hit
rate, server peak RSS / CPU-s / IO (`ResourceMonitor`), healthz latency
sampled during the burst, `load_avg` start/end.

### 4.4 Acceptance

| gate | criterion | source |
|---|---|---|
| op-success | 100 % at every level (no 5xx, no timeout, no reset) | §73 / D13 |
| latency | p99: ≤ 25 ms @ 8, ≤ 75 ms @ 32, ≤ 250 ms @ 128 (5×, 6×, 21× the 12 ms single-request through-HTTP budget — explicit contention allowances, recorded with the run) | D9 × contention headroom |
| liveness | healthz p99 ≤ 50 ms during the level-3 burst | the ops surface must survive saturation |
| scaling | throughput 32-clients ≥ 2× the 8-client level; the 128 level is recorded whether it scales or plateaus — a plateau is a finding (unbounded `spawn_blocking` pile), not a failure | this campaign |
| memory | level-3 peak RSS ≤ 2× level-1 peak | unbounded task spawning shows here first |
| error discipline | every malformed body in level 5 returns 4xx with a `schema.*` code; zero 5xx; zero panics | §73, ARCHITECTURE §7 |
| no collateral damage | a well-formed control request issued after level 5 returns the byte-identical answer it returned before it | cache/state poisoning check |
| absence recorded | no 429/503/backpressure signal exists in the response vocabulary at any level — recorded as a finding about the surface, not counted as a defect | silence-is-a-finding |

---

## 5. Workload class C — long context

### 5.1 Generator

`generate_workloads.py` also emits `longctx_tiers.json`: the committed
suite's three classes (metadata_match / lexical_semantic /
relational_compositional), 40 items each, 120 per tier, with each item's
state padded to a target estimated token count (bytes-over-4, the
engine's own budget accounting) using the same vocabulary-disjoint,
grammar-inert distractor prose `make_long_suite.py` uses — with its
disjointness assertion kept.

| tier | est. tokens | state bytes | fits `MAX_BODY_BYTES`? |
|---|---:|---:|---|
| L1 | 4,096 | ~16 KiB | yes |
| L2 | 32,768 | ~128 KiB | yes |
| L3 | 131,072 | ~512 KiB | yes — this is the structural maximum: the state alone fills half the 1 MiB cap, and the envelope plus question must fit in the remainder. A 256k-token tier is not designable against the shipped surface. |
| probe | 262,144 | ~1 MiB | refused at the socket (413) — recorded as the ceiling row |

### 5.2 Arms

| arm | configuration | question answered |
|---|---|---|
| C-full | `serve` default stack, no focus | what does the whole pipeline cost at scale |
| C-f512 | `serve --focus-budget 512` | does focused extraction hold answers at 32× the budget it was validated at (D18 validated ~3.7k-token states) |
| C-f4096 | `serve --focus-budget 4096` | the extraction/escalation crossover |
| C-escalate | `serve --ladder ladders/fusion-v2.json --llama …` (llamacpp feature build) | where escalation actually fires and what it costs — the D32 120 s ceiling's reason to exist |

### 5.3 Metrics

p50/p95/p99 client wall per tier × arm; outcome mix; escalation rate
(`rungs_fired > 0`); focus engagement/decline/escalation from the trace's
`focus_*` facts over HTTP (and `RunReport::focus` / the `"focus"` key in
`execution_json` on the CLI/MCP projection); view token p50; deciding-node
evidence (§3.3); projected model-rung tail from the measured escalation
count × the D32 anchor (~22 s per readout at 15 KB / 5 candidates —
recorded as a projection, never presented as a measurement).

### 5.4 Acceptance

| gate | criterion | source |
|---|---|---|
| answer parity (focus) | focus arms are answer-identical to the C-full arm on ≥ 99 % of items at every tier | D18 |
| answer stability | C-full correctness vs the suites' gold answers degrades ≤ 3 pp per class against the committed short-suite engine row (0.683 blended; per-class 0.88 / 0.23 / 0.950) | PLAN Phase 15 |
| determinism | full replay per tier × arm is byte-identical | D7 |
| scaling shape | p50 across 4k → 32k → 128k fits a power law with exponent ≤ 1.3 (BM25 index build is the dominant stage; super-quadratic growth would indicate a pathological scan) | this campaign |
| absolute ceiling | C-full p99 ≤ 2 s at 128k — 200× the D9 10 ms deterministic budget, the scaling allowance this campaign proposes as a D9 amendment. D9's budgets are stated at the stage reference shapes (256 candidates, short state) and do not cover long state; the measured curve is the artifact, the amendment is filed as a finding (§9.3), not assumed | D9 gap |
| deadline honesty | no request in C-full/C-f512/C-f4096 exceeds 5 s; the C-escalate arm's tail is bounded by the request deadline and reports a typed timeout rather than hanging | D32 |
| socket ceiling | the 256k probe returns 413 with the documented envelope; nothing partially parses | `MAX_BODY_BYTES` |

---

## 6. Failure-injection dimension (cross-cutting)

| injection | procedure | must hold |
|---|---|---|
| **F1 — model rung unavailable** | start `serve` with the llamacpp build + `--llama http://127.0.0.1:8094` and `--ladder ladders/fusion-v2.json`; drive class-A sessions; `kill -9` the llama-server mid-session; restart it 30 s later and continue | every in-flight request resolves (typed error or answer) within client timeout + 5 s — never a hang; `healthz` 200 within 1 s of the kill; the first request after restart succeeds; a request that failed mid-walk serves no cached decision on retry (the cache stores completed responses only) |
| **F2 — malformed-input burst** | 10 malformed shapes × 10 requests, level-3 concurrency: missing `state.facts`, wrong question discriminator, body > 1 MiB, unknown `x-opencodifier-format` value, non-JSON bytes, empty body, 5,000-deep nested JSON, `max_candidates: 300`, wrong `limits` types, duplicate JSON keys | 100 % 4xx with a stable code; 0 × 5xx; 0 panics; server process unchanged; the post-burst control request is byte-identical to pre-burst |
| **F3 — deadline pressure (D32)** | (a) request declaring `max_execution_time: 121 s` → refused at the wire; (b) C-full request at 128k declaring the full 120 s → completes inside the ceiling; (c) C-escalate with the rung behind a delay proxy (`8095`, +60 s per call) → typed timeout error inside deadline + 5 s, server healthy after | D32: the ceiling is finite, validated at the wire, min-ed with the server config |
| **F4 — verifier disagreement** | ladder `proofs-only-v1.json` on class A (forces non-proof questions down the verify path with no verifier configured) | outcome `verify` with `verifier: "none"` in the trace; never an error; counted, not failed |

**F1 is the campaign's highest-value injection.** The rung walk propagates
classifier errors with `?` (`executor.rs`, `resolve_threshold`), so a dead
model endpoint turns an *already-answered* question into a failed request —
the primary rung's distribution is discarded. Whether that is the correct
posture (D29 puts degrade-to-consumer on the consumer) or a defect (a
partial answer the engine had in hand is thrown away) is exactly what the
injection is designed to settle, with the expected/observed row filed under
§9.3 rather than decided here.

---

## 7. Dual-arm methodology

Same requests, same build, five arms (D0–D4); parity is the gate,
overhead is the measurement.

| arm | mechanism | what it isolates |
|---|---|---|
| **D0 — in-process engine** | `cargo bench -p opencodifier-engine --bench engine` (`decide_full_pipeline`, `decide_mock_classifier`, `normalize_native_decode`) | the engine alone, no transport — the D9 stage rows |
| **D1 — in-process router** | `cargo bench -p opencodifier-http --bench http` (`decide_round_trip_4_candidates` drives `opencodifier_http::router` over an ephemeral loopback listener) | decode + encode + hyper, no process boundary |
| **D2 — real socket** | `runner/run_engine.py`-style client against `opencodifier serve --bind 127.0.0.1:8092` | the deployment path |
| **D3 — MCP stdio** | JSON-RPC over stdin/stdout to `opencodifier mcp serve` (`codify_decide`), the raw-frame pattern `scripts/e2e_validate.py` already uses | the third interface's framing cost |
| **D4 — CLI** | `opencodifier decide --input <file> --trace` over a 120-item sample | per-invocation process cost, reported separately, never averaged into latency |

**Overhead = measured differences**, not assumptions: D1 − D0 is the
"D9 HTTP overhead 2 ms" check; D2 − D1 is the process/socket cost; D3 − D2
is the MCP framing cost. Every number is recorded with its `load_avg`
regime.

**Parity rule:** D1/D2/D3 must return byte-identical decisions — answer,
outcome, confidence, `identity`, `trace_version` — for the same payload on
the same build. The engine is one pipeline and three transports; a
divergence is a blocker-severity finding, whatever the latency numbers say.

**Scope limit:** the campaign does not add a new in-process harness binary.
D0/D1 are the committed criterion benches; the dual-arm claim rests on
them plus the socket arm, which is exactly the evidence the D9 table
already uses.

---

## 8. Runbook (compute host: fedora)

### 8.1 Host facts (verified 2026-10-05)

| item | value |
|---|---|
| host | `fedora` (i5-13600K, 20 threads, CPU-only — no GPU) |
| repo | `/home/mkinney/repos/OpenCodifier` (was 2 commits behind `main` at design time — sync first) |
| python | 3.14.6 |
| scratch | `/home/mkinney/oc-model-eval` (runs, builds, models) |
| ports in use | 8080 and 8091 both held by standing `llama-server` processes from the measurement chain. **The campaign block is 8092–8095**; 8080 is never used. |

Port allocation for the campaign: `8092` primary `serve`; `8093` second
`serve` instance; `8094` model-rung `llama-server`; `8095` delay proxy.

### 8.2 Sync, build, verify

```bash
ssh fedora
cd ~/repos/OpenCodifier
git fetch origin && git checkout main && git pull --ff-only
git rev-parse HEAD                 # becomes the commit of record in every manifest
export RUNS=$HOME/oc-model-eval/runs/realworld
export TMPDIR=$HOME/oc-model-eval/tmp
mkdir -p "$RUNS/binaries" "$TMPDIR"
cargo build --release -p opencodifier-cli
cp target/release/opencodifier "$RUNS/binaries/opencodifier-default"
sha256sum "$RUNS/binaries/opencodifier-default" | tee "$RUNS/binaries/default.sha256"
cargo build --release -p opencodifier-cli --features llamacpp   # C-escalate / F1 only
cp target/release/opencodifier "$RUNS/binaries/opencodifier-llamacpp"
sha256sum "$RUNS/binaries/opencodifier-llamacpp" | tee "$RUNS/binaries/llamacpp.sha256"
```

One build target, one binary: the feature build **overwrites**
`target/release/opencodifier`, so each build is copied aside and digested
immediately — the default-build arms run against
`$RUNS/binaries/opencodifier-default`, the C-escalate/F1 arms against
`$RUNS/binaries/opencodifier-llamacpp`, and the run JSON's
`binary_sha256` names which one actually served. The default build stays
dependency-identical; the `llamacpp` build is a separate artifact used
only by the arms that name it.

### 8.3 Pre-flight (every serve invocation)

```bash
port_free() { ! (exec 3<>/dev/tcp/127.0.0.1/$1) 2>/dev/null; }
port_free 8092 || { echo "8092 held — pick another"; exit 1; }
"$RUNS/binaries/opencodifier-default" serve --bind 127.0.0.1:8092 \
    > "$RUNS/serve-8092.log" 2>&1 &
curl -sf http://127.0.0.1:8092/v1/healthz   # gate: 200 before any traffic
```

A stale server silently answering on the port poisons every cache and
determinism assertion — the same trap `scripts/e2e_validate.py` guards
with its pre-flight `connect_ex` probe.

### 8.4 Load gate (policy v2: proceed-dirty, record)

```bash
cut -d' ' -f1 /proc/loadavg    # recorded at run start and end, per run
```

- **Behavioral rows** (op-success, error codes, answer parity, cache
  behaviour, determinism, memory ceilings) are load-invariant → run
  under whatever regime exists; `load_avg` travels with the result.
- **Latency rows** (the p50/p95/p99 and slope gates in §3.4, §4.4, §5.4)
  are comparable only within a regime → run, then re-check: if a
  latency-gated run's p99 exceeds 3× the class gate, re-run it in a
  better window and record both. The regime is never an excuse to drop a
  gate; it is the reason a re-run is honest.
- **Budget-of-record rows** (the D9 criterion comparisons in §7) keep the
  stricter convention the D9 baseline driver used: a Class L window
  (load < 30, bounded wait), because a committed baseline must be clean
  (`bench_baseline_d9_20261001.sh`: waits rather than proceeds).

### 8.5 Execution order

```bash
RUNS=~/oc-model-eval/runs/realworld
mkdir -p "$RUNS"/{surface,classA,classB,classC,failure,dualarm}

# 1. Step 0 surface enumeration (§2) → $RUNS/surface/
# 2. Generators, byte-locked + audited
python3 benchmarks/validation/suite/generate_workloads.py --check
python3 benchmarks/validation/runner/audit_workloads.py

# 3. Class A
python3 benchmarks/validation/runner/run_session.py --port 8092 \
    --plan "$RUNS/classA/session_plan.json" --out "$RUNS/classA/A-serial.json"
python3 benchmarks/validation/runner/run_session.py --port 8092 --interleave 8 \
    --plan "$RUNS/classA/session_plan.json" --out "$RUNS/classA/A-interleaved.json"
python3 benchmarks/validation/runner/run_session.py --port 8092 --focus-budget 512 \
    --plan "$RUNS/classA/session_plan.json" --out "$RUNS/classA/A-focused.json"

# 4. Class B
for lvl in 8 32 128; do
  python3 benchmarks/validation/runner/run_burst.py --port 8092 --clients "$lvl" \
      --requests 1024 --out "$RUNS/classB/B-c${lvl}.json"
done
python3 benchmarks/validation/runner/run_burst.py --port 8092 --clients 8 \
    --requests 256 --cache-probe --out "$RUNS/classB/B-cacheprobe.json"
python3 benchmarks/validation/runner/run_burst.py --port 8092 --clients 128 \
    --requests 1024 --malformed-rate 0.05 --out "$RUNS/classB/B-mixed.json"

# 5. Class C — one serve configuration per run, sequential, one port each
python3 benchmarks/validation/runner/run_longctx.py \
    --binary "$RUNS/binaries/opencodifier-default" --port 8092 --arm full \
    --suite benchmarks/validation/suite/longctx_tiers.json \
    --out "$RUNS/classC/C-full.json"
python3 benchmarks/validation/runner/run_longctx.py \
    --binary "$RUNS/binaries/opencodifier-default" --port 8093 --arm focus512 \
    --suite benchmarks/validation/suite/longctx_tiers.json \
    --out "$RUNS/classC/C-f512.json"
python3 benchmarks/validation/runner/run_longctx.py \
    --binary "$RUNS/binaries/opencodifier-default" --port 8093 --arm focus4096 \
    --suite benchmarks/validation/suite/longctx_tiers.json \
    --out "$RUNS/classC/C-f4096.json"
python3 benchmarks/validation/runner/run_longctx.py \
    --binary "$RUNS/binaries/opencodifier-llamacpp" --port 8093 --arm escalate \
    --ladder ladders/fusion-v2.json --llama http://127.0.0.1:8094 \
    --llama-model-id pd-fork-38de7eb__qate2b-q4_0__tree-v2 \
    --suite benchmarks/validation/suite/longctx_tiers.json \
    --out "$RUNS/classC/C-escalate.json"

# 6. Failure injection (F1–F4, §6)
python3 benchmarks/validation/runner/run_inject.py --port 8092 --rung 8094 \
    --out "$RUNS/failure/"

# 7. Dual-arm parity (D0/D1 via cargo, D2/D3/D4 via the runners)
cargo bench -p opencodifier-engine --bench engine -- --save-baseline realworld-baseline
cargo bench -p opencodifier-http    --bench http   -- --save-baseline realworld-baseline
python3 benchmarks/validation/runner/run_mcp_arm.py --port 8092 \
    --out "$RUNS/dualarm/D3-mcp.json"

# 8. Determinism replays (each runner re-drives its own suite; the
#    determinism block is mandatory, not optional)
# 9. Summarize → the committed report of record
mkdir -p benchmarks/validation/results
python3 benchmarks/validation/runner/summarize.py "$RUNS" \
    > benchmarks/validation/results/VALIDATION.md
```

### 8.6 Manifest, determinism, replay (identical to the existing runners)

Every run JSON, before it counts as evidence:

| field | rule |
|---|---|
| `git_rev` | `git rev-parse HEAD` at run time; all arms of one comparison carry the same rev |
| `binary_sha256` | digest of the exact binary that served |
| `suite_sha256` | SHA-256 of the byte-locked input fixture |
| `config.host` | `socket.gethostname()` — speed rows are only comparable within a host (the board's provenance rule) |
| `identity` | the `/v1/healthz` identity block, recorded before and after the run; mismatch = discard the run |
| `resources` | `ResourceMonitor` report — mandatory in every runner, as in every decision-model runner |
| `load_avg` | start/end 1-minute average |
| `determinism` | `predictions_match` + `max_prob_delta` from a full replay; a run without a replay is not of record |

Generators are byte-locked: `--check` re-derives the fixture and exits
non-zero on any byte difference, the Phase 13 suite discipline.

---

## 9. Findings-artifact contract

### 9.1 Where findings live

| artifact | path | rule |
|---|---|---|
| Committed report of record | `benchmarks/validation/results/VALIDATION.md` | the campaign's own page, written in the `results/REPORT.md` style: findings numbered `RV-F##`, each with its measured numbers and its honesty caveats |
| Raw runs | `fedora:~/oc-model-eval/runs/realworld/**` | never committed; regenerable from the pinned fixtures + binary |
| Defect rows | `docs/PLAN.md`, campaign task row | every row from §9.2 filed before the task closes |
| Board deltas | `docs/BENCHMARKS.md` + `benchmarks/decision-model/results/board.csv` | only if a gate result changes a published number (e.g. a new D9 amendment row) |

### 9.2 Defect row format

```
| id | severity | surface | repro anchor | expected | observed | status |
```

- **id** — `RV-001`, `RV-002`, … (`RV-000` is reserved for the step-0
  surface-enumeration finding, whatever it turns out to be).
- **severity** — `blocker` (a gate in §3–§6 fails, or a response is wrong /
  hung / untyped), `major` (a documented behaviour is not observable), 
  `minor` (latency gate missed with a recorded load-regime cause), `note`.
- **repro anchor** — the exact command and request id, sufficient for
  someone else to reproduce without reading the runner source.
- **status** — one of `open`, `fixed+re-verified`, `waived:<reason>`. The
  task does not close with a row in `open`.

### 9.3 Rows this design already predicts (filed at execution, not now)

1. **No per-stage timings exist in the trace.** `TraceEntry` is
   `{node, detail}` and `DecisionMetrics` is
   `{candidates_in, candidates_out, cache_hit, verification_triggered}`
   (`crates/opencodifier-core/src/trace.rs`). The deliverable's "per-stage
   stage timings from the deterministic trace" is not satisfiable as
   stated: stage evidence must come from the criterion benches (D0/D1) and
   client wall. Whether to add durations to the trace is a trace-contract
   change (D8 freezes `trace_version = 1`) — open question §10.2,
   not something this campaign can decide by measuring.
2. **No admission control on the HTTP tier.** Every request is one
   `spawn_blocking` task; there is no queue bound, no 429, no backpressure
   vocabulary. Class B measures the consequence.
3. **D9 has no long-context row.** The budgets are stated at reference
   shapes; §5.4 proposes the amendment and files it.
4. **Rung-walk failure discards the primary answer** (§6 F1).
5. **`serve --policy` is accepted and inert** (validated then reported
   `cli.policy_inapplicable`) — on the surface by design; the enumeration
   records it so no caller mistakes it for an engine override.
6. **No wire field names the deciding rung.** `QuestionDecision::decided_by`
   is `pub(crate)` and never serialized; the deciding node is inferred from
   trace facts (`policy_source`, `rung_chain`, the node entries) rather than
   read. Ladder-driven traffic can identify the rung only when a ladder is
   configured — a plain engine's trace names the nodes but not which one
   decided. Fine for this campaign (it infers, and the ladder arms are
   explicit); worth a record if a consumer ever needs it typed.

Silence-is-a-finding applies in both directions: a stage that engages and
produces no observable metric anywhere is blocker-grade, and so is a gate
this document cannot express because the runtime emits nothing to gate on.

---

## 10. Open questions

1. **No campaign task row exists in `docs/PLAN.md`.** Nothing in the plan
   or the repo names task #104 or a real-world validation phase; this
   document is the working spec until a row exists with its own acceptance
   criteria. If that row's criteria differ from §3–§6, the row wins and
   this document is amended, not the reverse.
2. **Stage timings in the trace** — add (trace_version bump, D8 record,
   wire-fixture churn) or keep stage evidence at the criterion layer? Not
   decided here (§9.3 row 1).
3. **Admission control** — whether the HTTP tier gains a bounded blocking
   pool / queue depth / 429 path is a decision record of its own; class B
   supplies the evidence, not the verdict.
4. **Commit of record on fedora** — the checkout there was 2 commits behind
   `main` at design time; §8.2 pins the sync step. Any arm run against a
   different rev than its comparison arm is void.
5. **Model-rung weights for the C-escalate arm** — the ladder
   (`ladders/fusion-v2.json`) is committed but the GGUF behind
   `--llama-model-id` is host-local and unpinned by this document; the run
   manifest must pin its SHA-256 (D14 practice) or the arm is recorded as
   unmeasured rather than approximated.
6. **Campaign duration budget** — class C's escalate arm and the F1
   restart window are the only unbounded-cost items; the campaign assumes
   a single host-day. If a budget constraint exists, it should be stated
   in the task row, because it decides whether C-escalate runs at all
   three tiers or only L3.
