# Offline campaign — 2026-10-23

**Context.** T5500 (V100 pair) and fedora (5060 Ti / i5-13600K) are powered
down for AC work. Everything below was chosen to run on **this host only**:
CPU-only Linux, no NVIDIA GPU (AMD Lucienne iGPU), 2.9 TB free on `/nas`,
Python 3.13.5 + `requests`, gitforge CI at `127.0.0.1:42782` reachable.

Research basis: RESEARCH.md §27–§30, PLAN.md Phase 25, the full
`docs/ESCALATION_LANES_PLAN.md` (468 lines, read in full), OPERATOR_ACTIONS,
a TODO/FIXME sweep of all Rust sources (result: **zero markers — the gap is
missing programs, not deferred edits**), one `#[ignore]` test
(`macjev_parity`, host-lane gated), and an inventory of local assets.

## Local asset inventory (what makes offline work possible)

| asset | path | state |
|---|---|---|
| engine binary | `target/release/opencodifier` | present, stale (Oct 19 — rebuild before measuring; cheap) |
| fork llama-server (CPU, 18 MB) | `OpenCodifier-fork/forks/llama.cpp/build/bin/llama-server` | proven 2026-10-22: `/v1/decision` tree mode, full `distribution` map, ~984 ms with 2B |
| base GGUFs | `/nas/Temp/work/oc-model-eval/models/` | Qwen3.5-2B-Q4_K_M, -4B-Q4_K_M, full UD quant ladder, decider-2b-v11, +20 others |
| locked suite | `benchmarks/decision-model/suite/suite_holdout_rel200.json` | 280 items, 3 classes, answer key inline |
| JevBench harness clone | `/nas/Temp/work/oc-model-eval/jevbench-ref` | datasets public/easy,original,hard — engine-arm legs run locally |
| CI | `.gitforge.yml` (lint/test/doc/e2e/supply-chain) + GitHub mirror | gates all green as of `24989e3` |

**Blocked-class work** (do not attempt until remotes return or upstream
ships): `BACKEND=cuda` fork build (no nvcc here; V100s are on T5500),
LoRA throughput A/B (adapters live on t5500), E1-E HF upload (adapter on
t5500 + operator approval), D42 per-family profile thresholds (need the
adapter for slot legs), v3 family-remap *run* (corpus on t5500 — the
*script* can be built and tested here), fast-LoRA kernels (NVIDIA port
does not exist upstream, §28), Windows/macOS publication legs.

## Gap and improvement inventory

| # | area | evidence | offline? |
|---|---|---|---|
| G1 | **Phase 25 #188 premise validation** — the gate-vs-bare-LLM A/B | PLAN.md: "runs **first** — if the gate does not beat a bare LLM, the rest is decoration"; no harness exists (the harness *is* the deliverable) | **yes — flagship** |
| G2 | **#189 escalation contract** | `DecisionOutcome::Escalate` declared (`core/src/response.rs:28`) but **never constructed**; no `EscalationReport`, no `escalate_below`, zero HTTP/MCP exposure. Absent ⇒ byte-identical behaviour already holds trivially | yes |
| G3 | **#190 `opencodifier-config`** | "the repo has **no config file at all**, which is the structural gap under every one of these" (PLAN.md) | yes |
| G4 | **#191 lanes / #192 init+doctor** | 8 named presets designed, none implemented; no `lane`/`init`/`doctor` subcommands in the CLI | yes (after G3) |
| G5 | **#193 feedback loop / #194 `codify_gate` / #195 CI recipes / #196 playground QA** | MCP has 6 tools (no `codify_gate`); HTTP has 9 routes (no `/v1/feedback`); playground QA tool absent | yes (after G2/G3) |
| G6 | **§27 design docs** — semantic cache ("No design"), hybrid retrieval ("No design") | RESEARCH §27 marks both "actionable now … don't need GPU"; reranker semantic half likewise undesigned | yes |
| G7 | **Fork build-smoke CI job** | fork branch has `smoke.sh` (proven) but no CI job builds the fork | yes (write; heavy to run) |
| G8 | **`scripts/upload_e1e_to_hf.sh` hardening** | script is complete (fixed `d75df6d`) but assumes t5500 artifact layout; can take a path argument + `--dry-run` verification offline | yes (partial) |
| G9 | **Stale bookkeeping** | OPERATOR_ACTIONS git-state says "main → gitforge @ `d75df6d` (pushed)"; actually gitforge/main is **3 behind, 0 ahead** (fast-forwardable), origin/main is 390 behind (deliberately unsynced — operator call) | yes |
| G10 | **Stale worktree** | `/home/mkinney/agent-work/opencodifier-correctness-20261020`: 390 commits behind, clean, one duplicate PR#7 commit. Cleanup needs operator go-ahead — flagged, not deleted | operator |
| G11 | **Engine-arm JevBench reproduction locally** | harness clone + engine binary are both local; Board A engine row (0.3766) and per-family engine breakdowns for the §29 routing design can be re-measured without t5500 | yes |
| G12 | X/Discord handles; EROFS sandbox unit; E1-E HF approval + org name | OPERATOR_ACTIONS #3/#4/#6 | operator |

## Prioritized tracks

**Track A — #188 premise validation (flagship).** The escalation plan
fixes the ordering ("188 first — it can invalidate the rest") and the
decision rule *before* the run: gate ships if `engine_wrong` < host LLM's
own error rate **and** `saved_calls` ≥ 20%. Design, adapted to what
exists today and stated honestly in the results:

- **Workload:** `suite_holdout_rel200.json` (280 items, answer key inline,
  the repo's locked suite). The plan asked for a multi-turn agentic set;
  none exists locally with an independent key, and the suite is the only
  real known-answer workload on this host. Recorded as a deviation; a
  multi-turn set can follow if the premise validates.
- **Arm A (bare LLM):** fork llama-server + Qwen3.5-2B-Q4_K_M,
  `--decision-seqs 3`, `/v1/decision` tree mode per item (~1 s/item,
  ~5 min total).
- **Arm B (gated):** engine gate first — `opencodifier serve` (zero
  stack) per item; `outcome == accept` ⇒ engine answers (saved call),
  `verify`/`abstain` ⇒ escalate to the same llama-server. **Zero engine
  changes needed** — the existing accept band *is* the gate; #189's
  `escalate_below` + payload refines it after the premise is proven.
- **Metrics:** `engine_sufficient`, `engine_wrong`, `saved_calls`,
  combined accuracy vs Arm A, tokens, latency. Result lands in
  `docs/BENCHMARKS.md` (the task's acceptance gate).

**Track B — #189 escalation contract.** `escalate_below: Option<f64>`
(default `None` ⇒ byte-identical), `Escalate` construction in the
executor, bounded deterministic `EscalationReport` (no chain-of-trace),
projections (native/OpenAI/Anthropic/Jev), HTTP `escalation` block, MCP
carriage. Gate: "absent ⇒ byte-identical responses; present ⇒ payload
shape + bounded size + determinism". Unblocks #193/#195/#196.

**Track C — #190 config crate.** `opencodifier-config` (core+engine
only), JSON, discovery `$OPENCODIFIER_CONFIG` → XDG → `./opencodifier.json`,
precedence "flag > env > project > user > default", strict `config.*`
errors. Gate: "no-config ⇒ byte-identical (fixture suite); every existing
flag is an override". Unblocks #191/#192.

**Track D — §27 design docs** (semantic cache, hybrid retrieval,
semantic reranker): pure writing, explicitly greenlit by §27.

**Track E — housekeeping:** push gitforge (fast-forward), refresh
OPERATOR_ACTIONS git-state, run `just check-macjev` (host lane available
here), harden `upload_e1e_to_hf.sh` (G8), draft fork CI smoke job (G7).

**Deferred to T5500's return** (prep only now): v3 remap script (build +
synthetic-row test here), E1-E release checklist verification, family
profile design from local engine-arm per-family data (G11 feeds it).

## Line that must not move

OpenCodifier stays the runtime: no provider SDK, no auto-download of
weights, no config file ⇒ byte-identical behaviour, and every Phase 25
task keeps its recorded acceptance gate — "nothing advances on 'it
compiles'".
