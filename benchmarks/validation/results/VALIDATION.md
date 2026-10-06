# Real-world validation campaign — results of record

Class-by-class results for the real-world validation campaign
(`docs/VALIDATION.md` is the spec). Findings are numbered `RV-##` per
the findings-artifact contract (spec §9). Raw run JSONs live on the
compute host (`fedora:~/oc-model-eval/runs/…`) and are never committed;
every number here is traceable to one.

Manifest: repo commit at run time `8caf34f` (release build,
`target/release/opencodifier`); host fedora, 16 cores; serve on
`127.0.0.1:8092` with default policy, no ladder, no model rung. Host
load was hostile for every window tried (sibling agent sessions hold
the 16-core floor at ~17–21 and spike to 30+; gate policy v2 =
proceed-dirty with load recorded per run). Latency gates carry 5–9×
headroom over budget, so load contamination does not flip any verdict
recorded here.

## Class B — burst (levels 1–5), run `burst-r2` (2026-10-05)

Five-level traffic matrix against `/v1/decide`: 8/32/128 concurrent
clients × 1,024 unique bodies (level-4: 8 clients, 128 bodies × 2 cache
passes; level-5: 128 clients, 1,024 unique + 51 malformed over 8 F2
shapes). Uniqueness by zero-padded counter in a marker sentence asserted
grammar-inert and vocabulary-disjoint per the spec. Load 29–31 for the
whole run (1-min avg), recorded per level.

| level | clients | requests | throughput | p50 | p99 | valid-leg success | peak RSS |
|---|---:|---:|---:|---:|---:|---:|---:|
| 1 | 8 | 1,024 | 1,022 rps | 1.38 ms | 2.69 ms | 1,024/1,024 | 31.2 MiB |
| 2 | 32 | 1,024 | 968 rps | 4.91 ms | 15.60 ms | 1,024/1,024 | 31.2 MiB |
| 3 | 128 | 1,024 | 826 rps | 8.13 ms | 27.00 ms | 1,024/1,024 | 31.4 MiB |
| 4 | 8 | 256 (128×2) | 255 rps | 1.21 ms | 2.66 ms | 256/256 | 31.5 MiB |
| 5 | 128 | 1,024 + 51 malformed | 913 rps | 5.70 ms | 22.73 ms | 1,024/1,024 | 34.7 MiB |

healthz sampled at 10 Hz through every level: worst p99 14.71 ms
(level 3, budget ≤ 50 ms).

### Gates (spec §4.4)

| gate | budget | observed | verdict |
|---|---|---|---|
| op-success (valid legs) | 100 % every level | 100 % at all five levels; zero 5xx, zero timeouts, zero resets | pass |
| latency p99 | ≤ 25 / 75 / 250 ms @ 8 / 32 / 128 | 2.69 / 15.60 / 27.00 ms | pass (9× / 5× / 9× headroom, under load 29–31) |
| liveness | healthz p99 ≤ 50 ms @ L3 | 14.71 ms | pass |
| memory | L3 peak RSS ≤ 2× L1 | ratio 1.01 (31.4 vs 31.2 MiB) | pass |
| error discipline | every malformed body → 4xx + stable code, 0 × 5xx | 51/51 rejected: 44 × 400, 7 × 413 (>1 MiB); zero 5xx, zero panics | pass |
| no collateral damage | post-burst control decision identical | identical at all five levels (`answers`/`outcome`/`confidence`) | pass |
| absence recorded | — | no 429/503/backpressure exists in the response vocabulary at any level → RV-005 | recorded |

### Findings opened by class B

- **RV-001 (runner defect, fixed+re-verified)** — the burst runner
  recorded all-zero server resources when handed a dead `--server-pid`
  (burst-r1 passed 3743795; the live serve was 3069208): the tree walker
  unconditionally includes the root, every `/proc` read failed, and the
  zeros landed in the JSON as if they were measurements. The memory gate
  was *unmeasured* in burst-r1, not passing. Fix: refuse to start the
  monitor without a live `/proc/<pid>` entry and say so in the log;
  re-verified by burst-r2 recording 31.2–34.7 MiB.
- **RV-002 (spec reclassification, waived:accepted-by-design)** — two of
  the ten planned F2 shapes are not malformed on this surface: omitting
  `state.facts` returns 200 (`State.facts` is `#[serde(default)]`;
  omitted means "no typed facts" and the engine extracts from text) and
  duplicate JSON keys return 200 (serde last-key-wins). Reclassified out
  of the malformed set; spec §6 amended with the rationale.
- **RV-003 (spec amendment, waived:unsatisfiable-as-stated)** — the
  post-burst poison check cannot be raw byte-identity on any server with
  the exact-decision cache: the second identical send legitimately
  returns `cache_hit: true` and a rewritten trace. Amended to
  decision-core identity (passes everywhere) with raw byte-identity kept
  as information; spec §4.4 amended.
- **RV-004 (cache behavior, note)** — the exact-decision cache is a
  1,024-entry LRU (`CacheConfig::default`, ttl 300 s). Level 1/2/3/5
  flood exactly ≥ 1,024 unique bodies, evicting the pre-burst control
  entry, so the post-burst control is a miss; level 4's 128-body pool
  keeps it and the control replays verbatim (raw bytes identical). Not a
  defect — bounded caches evict — but any consumer that assumes
  "repeat ⇒ cache_hit" across a large unique-body flood will be wrong,
  and this campaign's fixtures now record the boundary.
- **RV-005 (admission control, open)** — zero 429/503/backpressure
  signals at any level (predicted as §9.3 row 2, now measured): every
  request is a `spawn_blocking` task with no bound. Throughput plateaus
  rather than scales (1,022 → 968 → 826 rps across 8 → 32 → 128 clients,
  a mild decline, under load 29–31) while latency budgets hold with 5–9×
  headroom; at ~1 ms server-side per request the tier saturates its
  pipeline near 1k rps and extra clients only queue. This is the
  evidence base §10.3 (admission-control decision) wanted; the verdict
  stays open there.
- **RV-006 (flaky client reset under load, note)** — burst-r1 saw 1 of 5
  `nested_5000` legs end in a client-side `BrokenPipeError` (server
  closed mid-reject under load 24); burst-r2 saw 6/6 clean 400s for the
  same shape. Not reproducible as a server defect; recorded for
  completeness.

Class B verdict: **all §4.4 gates pass**; the four note-grade findings
above are recorded, one runner defect fixed and re-verified.

## Class A — agentic multi-turn loops, run `classA-r2` (2026-10-05)

The 24-session × 50-turn incident-triage plan (1,152 decisions + 48
repeat probes per run), four runs against the campaign binary
(`opencodifier-default`, sha256 `d09ecbba…`, repo rev `2d3f81f`, plan
fixture sha256 `276327ac…` byte-identical across hosts). Load 28–29
throughout. Each run is hermetic: the runner spawns its own serve, so
cache state never leaks between runs.

| run | requests | correctness | probes | probe cache-hit | p50 | p99 | slope | abstain |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| A-serial | 1,200 | 0.9583 | 48/48 | 48/48 | 0.614 ms | 0.944 ms | +0.00166 ms/turn | 0.0 % |
| A-interleaved (8×) | 1,200 | 0.9583 | 48/48 | 48/48 | 1.838 ms | 3.390 ms | −0.00024 ms/turn | 0.0 % |
| A-focused (`--focus-budget 512`) | 1,200 | 0.9583 | 48/48 | 48/48 | 0.595 ms | 0.956 ms | +0.00172 ms/turn | 0.0 % |
| A-serial-replay (`--compare`) | 1,200 | 0.9583 | 48/48 | 48/48 | 0.640 ms | 0.904 ms | +0.00191 ms/turn | 0.0 % |

Replay determinism: 0 mismatches — answers, outcomes, and confidences
byte-identical across the full 1,200-decision chain (D7). Focus A/B:
**1,200/1,200 answers identical** to the full-state run (gate ≥ 99 %;
D18's own anchor was 120/120). Cross-session bleed: none — the
interleaved run's correctness and probe integrity match the serial run
exactly. Outcomes: every decision `accept`, zero verify/abstain — the
deterministic stack is fully confident on its own proof-shaped input.

### Gates (spec §3.4)

| gate | budget | observed | verdict |
|---|---|---|---|
| op-success | 100 % × 4 runs | 4,800/4,800 HTTP 200 | pass |
| answer correctness | ≥ 0.95 | 0.9583 every run | pass |
| probe correctness + cache-hit | 100 % | 100 % every run | pass |
| replay determinism | byte-identical | 0 mismatches | pass |
| per-turn latency | p99 ≤ 12 ms | 0.90–3.39 ms | pass (13× headroom at p99, under load 28) |
| growth slope | ≤ 0.05 ms/turn | +0.0017 max | pass (29× headroom; no BM25-rebuild blowup) |
| abstain rate | ≤ 20 % | 0.0 % | pass |
| focus A/B | ≥ 99 % answer-identical | 100 % | pass |

### Findings opened by class A

- **RV-007 (engine, major, open)** — the 48 correctness misses are not
  noise: they are exactly turns 29–30 of every session (2 × 24), one
  deterministic shape. At those turns `catalog` transitions `down` while
  its dependency `gateway` is healthy; `catalog` is the provable root
  cause (it has dependents, nothing upstream of it is failing), but the
  engine accepts `gateway` — the dependency hub — at calibrated
  confidence 0.948, above the 0.8 accept gate. The trace chain is
  `normalize → cache → rule → filter → lexical → choice → threshold`:
  no relational node appears; the lexical/rule layer decided a question
  the relational solver was built to prove. Repro: `A-serial.json`
  session 0 turns 29–30; request body = session_plan session 0
  `request_bodies["29"]`. Every session misses identically, so the fix
  has a free regression fixture. Class-A correctness passes *with* this
  defect (0.9583 vs the 0.95 floor is 1 pp of margin); the relational
  rung not being consulted on its own question shape is the deeper
  issue.

Class A verdict: **all §3.4 gates pass**, one major engine finding
(RV-007) filed with a deterministic repro anchor.


## Class C — long-context tiers, run `classC-r2` (2026-10-05)

Three tiers × 120 items at 4,096 / 32,768 / 131,072 estimated tokens
(`longctx_tiers.json`, generator `generate_workloads.py`, seed 20261005,
suite sha256 `fe96c1d3…`) plus one 262,144-token probe item exercised as
a raw-socket oversized-body probe. Four default-build arms (full state,
`--focus-budget 512`, `--focus-budget 4096`, and a full replay of each)
against `opencodifier-default` (sha256 `d09ecbba…`, the classes A/B
binary, repo rev `2d3f81f`), then the escalate arm against
`opencodifier-llamacpp` (sha256 `1947aaf7…`) with the fusion-v2 ladder
and a live llama.cpp decision rung. One serve configuration per run,
sequential, fresh port each arm.

| arm | L1 p50 | L2 p50 | L3 p50 | correctness (every tier) | wall | load |
|---|---:|---:|---:|---:|---:|---:|
| full | 2.03 ms | 7.68 ms | 28.27 ms | 0.675 | 4.6 s | 36.8 |
| focus512 | 8.8 ms | 26.3 ms | 79.5 ms | 0.675 | 14.9 s | 76.5 |
| focus4096 | 6.4 ms | 26.2 ms | 80.7 ms | 0.675 | 14.5 s | 81.8 |
| replays ×3 | — | — | — | 0.675 | — | — |

Correctness is *identical* across every tier and every arm — 0.675 with
per-class 0.850 metadata / 0.225 lexical-semantic / 0.950 relational,
deltas vs the committed short-suite anchor (PLAN Phase 15:
0.88 / 0.23 / 0.95) of −0.03 / −0.005 / 0.0. The metadata delta sits
exactly on the ±3 pp stability gate and passes; nothing degrades with
document length. p50 latency scales 2.03 → 7.68 → 28.27 ms across ×8
document growths — an exponent of ~0.76, comfortably sub-linear, with
p99 43.2 ms at the 128 k-token tier. Outcome mix is stable per tier
(50 abstain / 42 accept / 28 verify): the lexical-semantic abstentions
are the engine refusing below-confidence guesses, and that posture does
not erode as context grows.

Focus A/B: the extractor engaged on 252/360 items (84 per tier — the
items whose answer-bearing region sits outside the fixed head window)
and reversed its decision on 126/360 (42 per tier) when escalation
asked for a wider view; **answers were 360/360 identical** to the
full-state arm under both budgets. The extraction pass costs 4.4× at
L1 (2.03 → 8.8 ms) and 2.8× at L3 (28.3 → 79.5 ms) — the overhead
shrinks in relative terms as documents grow — and the 512 vs 4096
budget choice is statistically indistinguishable on every tier.

Determinism: all three arms replayed against a fresh serve with
answers, confidences, and outcomes **byte-identical** across all 360
decisions (D7; the class-C discipline of one serve per arm means the
replays also prove cache-cold determinism, not just repeat hits).

Oversized-body probe: the 262,144-token item arrives as a body past
the 1 MiB wire limit and is answered with a **clean 413** in 0.611 ms
("Failed to buffer the request body: length limit exceeded") — never a
partial parse, never a 200 with truncated state (spec §5.4).

### Gates (spec §5.4)

| gate | budget | observed | verdict |
|---|---|---|---|
| per-class stability vs short | ±3 pp | −3.0 / −0.5 / 0.0 pp | pass (metadata exactly at bound) |
| cross-arm parity vs full | ≤ 3 pp | 0.0 pp (identical) | pass |
| replay determinism | byte-identical | 0 mismatches ×3 arms | pass |
| deadline honesty | client 30 s | worst arm p99 43.2 ms (default arms) | pass |
| oversized body | clean 413 | 413, envelope error, 0.611 ms | pass |

### Escalate arm — fusion-v2 ladder + a live model rung

Three runs because the rung had to be sized (RV-009): rerun-1 at
`-c 8192` (every L2/L3 escalation 500ed fast with `no free KV cache
space`), rerun-2 at `-c 24576` (L2 still overflows — an L2 decision
prompt is ~32k tokens before the question), and the final rerun-3 at
`-c 40960` (2026-10-05 20:05–21:11 CDT, load 9–15). Rung:
`Jev-Style-0.8B-Decision-v3-Q4_K_M` (sha `0a19bc29…`, RV-008 chain) on
the parallel-decision fork, `-t 8 -ngl 0 --decision-seqs 8`; ladder
`fusion-v2.json`; serve binary sha `1947aaf7…`; runner rev `2d3f81f` +
escalate-arm edits.

| tier | items | correct | outcomes | p50 | p95 | p99 | escalated |
|---|---|---|---|---|---|---|---|
| L1 | 120 | 0.650 | 96 accept / 23 verify / 1 http_500 | 2.1 ms | 16.5 s | 23.9 s | 45 % |
| L2 | 120 | 0.533 | 65 accept / 55 http_500 | 12.7 ms | 30.2 s | 30.2 s | 55 attempts |
| L3 | 120 | 0.533 | 65 accept / 55 http_500 | 53.5 ms | 30.2 s | 30.4 s | 55 attempts |
| probe (262k) | 1 | — | clean 413 | — | — | — | — |

What the final run establishes:

- **L1 escalations are servable and reproducible.** 54 escalations
  (45 %), every chain recording `jev-style-0.8b-decision-v3(top=…)`.
  The quality split reproduces rerun-1 *exactly* — non-escalated items
  **0.970** (64/66), escalated items **0.259** (14/54), net 0.650 —
  across a rebuilt rung process, a different KV size, and a different
  day. The out-of-distribution rung finding (RV-010) is not noise.
- **KV sizing to the workload works** (RV-009): exactly one L2 item
  logged a memory-slot error at the 40960 margin (its prompt plus the
  decision suffix tipped over); the other 239 L1+L2 escalations
  allocated cleanly. Only **one** rung-side hard error (`send_error`)
  occurred in the entire run.
- **The binding constraint above L1 is CPU prefill time, not capacity.**
  With KV no longer refusing, a 32k- or 131k-token escalation prompt
  still cannot be served inside the engine's ~30 s rung-client timeout
  on 8 CPU threads: every L2/L3 escalation burns a bounded ~30.1–30.3 s
  and comes back as a typed http_500 — **never a hang** (all 111 500s
  in the run share that one wall-clock signature, including the single
  L1 case `B-0808` where a slow rung overran the same timeout), the
  rung stayed healthy (200) throughout, and the client deadline
  contract held (escalate arm's client timeout 130 s > engine 30 s, so
  the typed error is visible, not a client-side kill). On this
  hardware the model rung contributes nothing above L1 — the
  deterministic stack answers its 65 confident L2/L3 items at 0.985
  while all 55 low-confidence items it would escalate die at the rung.
- **Gates for this arm fail honestly.** Per-class parity vs the full
  arm: L1 −13/+7/−5 pp, L2/L3 −15.5/−23/−7.5 pp (budget ±3). The
  p99-deadline gate (written for the deterministic arms, ≤5 s) reads
  23.9 s / 30.2 s — that *is* the rung tail, the finding, not a
  measurement defect. Probe gate passes (clean 413, unchanged).

The escalate arm's campaign verdict: the ladder **routes** correctly
and its engagement is traceable and deterministic, but until the model
rung is in-distribution *and* fast enough for the tier it is handed
(RV-010, RV-011), fusion-v2 with a live model rung is net-negative
above L1-tier states. The zero-ML fusion profile remains the shipped
posture; the model rung is validated as plumbing (routing, typed
failure, bounded latency), not as a quality win.

### Findings opened by class C

- **RV-008 (rung environment, note)** — the decision-rung endpoint is
  fork-specific: upstream llama.cpp builds answer 404 on `/v1/decision`
  (the parallel-decision fork's route), so a rung arm is only as
  reproducible as its *build + weights* pair. The runbook's original
  arm (`pd-fork-38de7eb__qate2b-q4_0__tree-v2`) is unreproducible as
  specified — the qate2b GGUF is purged from storage. Substitution
  chain, recorded and sha-pinned: runbook arm (purged) →
  `Julia-1-Q8_0.gguf` (sha256 `1ea6a7e8…`; unloadable in the decision
  fork — mmbert pre-tokenizer — and `/v1/decision` absent from the
  upstream build) → **`Jev-Style-0.8B-Decision-v3-Q4_K_M.gguf`**
  (sha256 `0a19bc29…`, decision-trained qwen-arch model the fork loads
  and the rung's purpose wants). Rule going forward: every model-rung
  arm pins build provenance + weights sha in the run directory.
- **RV-009 (rung capacity, sized-to-workload after two failures)** —
  escalate rerun-1's L2/L3 legs returned 55 × HTTP 500 each
  (`outcome: null`, ~15–30 s per item) while L1 worked: the rung was
  launched with `-c 8192`, and every L2/L3-sized state overflows it —
  the fork logs `no free KV cache space for the decision prompt`. The
  first correction was wrong: rerun-2 relaunched at `-c 24576` and L2
  *still* failed with the same error, because the binding quantity is
  the decision **prompt** size (the fork's decision seqs share the
  prompt KV — rerun-1's L1 4k-token prompts × 8 seqs fit in 8192), and
  an L2 state is ~32k tokens before the question is appended. The rule
  that actually holds: **size rung capacity to the largest prompt the
  workload will hand it, not to a round number** — rerun-3 launches at
  `-c 40960` (L1 + L2 covered with headroom) and the failure-mode
  finder (`run_inject.py`) launches its rungs at the same capacity by
  default. L3 (~131k-token states) is *out of the CPU rung's envelope
  by measurement*, not configuration: it would need ~14 GB of KV on
  this host and a prefill far beyond the client's 30 s deadline, so
  its escalations fail honestly (typed rung error → null outcome) and
  that boundary is the recorded result (see RV-011).
- **RV-010 (model rung quality, open)** — the escalation ladder routes
  correctly but the substitute model decides poorly on exactly the
  items it is handed: non-escalated L1 items run at **0.970**
  correctness while escalated items land at **0.259**, and the net
  effect of the model rung on L1 was *negative* (0.675 → 0.650, three
  answers lost). 45 % of L1 fired the rung (54 items; 30 accept / 24
  verify outcomes, every chain recording
  `jev-style-0.8b-decision-v3(top=…)`), so engagement is real and
  traceable — the confidence gate is doing its job — but a
  0.8 B decision model substituted out of its training distribution
  (this suite's `target-*` candidate vocabulary is not the model's)
  answers hard items worse than the cheap rungs' residual. The verify
  path did not rescue them (0.208 on verify-outcome items). This is
  the campaign's cleanest evidence for the D15/fusion thesis: the
  ladder's value depends on the *top* rung's in-distribution
  competence, and an out-of-domain rung converts a 0.97 stack into a
  0.65 one on the items it escalates. Escalate-arm numbers above carry
  this caveat, and rerun-3 reproduces the split *exactly* (0.970 / 0.259
  across a rebuilt rung and a different day) — the substitution effect
  is deterministic, not noise. On L2/L3 the rung contributes nothing at
  all: the deterministic stack answers its 65 confident items at 0.985
  while every one of the 55 items it would escalate dies at the rung's
  prefill-bound timeout.
- **RV-011 (long-context escalation ceiling, open)** — a CPU rung
  cannot serve L3-sized (131,072 est-token) escalations under the
  deadline contract, and rerun-3 sharpened the boundary: it is not
  memory (40960 tokens of KV serves every L2 prompt but one) but
  **prefill throughput vs the engine's ~30 s rung-client timeout** —
  a 32k/131k-token CPU prefill takes longer than the timeout, so L2/L3
  escalations resolve as bounded typed http_500s at ~30.2 s, never a
  hang, with the rung healthy throughout. Raising the timeout would
  buy minutes per item at CPU prefill rates and is tuning-to-the-arm,
  not a fix. This is the D32 revisit condition made concrete with
  numbers: either the rung moves to accelerated inference, or the
  *engine* must narrow/focus the state it hands the rung (the class-C
  focus view is the natural mechanism) — escalating a full L3 state
  verbatim is not a servable configuration on CPU.

## Failure injections (class F), run `finject-r1` + `f1b-r4` (2026-10-05)

All injections ran on the compute host via `runner/run_inject.py`
against the same fusion-v2 + Jev-Style-0.8B configuration as the
escalate arm (rung `-c 40960 --decision-seqs 8`, model sha `0a19bc29…`).
The first full sequence crashed in its last phase and lost the earlier
phases' evidence with it — the runner now persists each phase's records
the moment it finishes, and records a phase error instead of dying, so
one phase's bug can no longer destroy another phase's measurements.

| injection | contract | observed | verdict |
|---|---|---|---|
| F1 (rung killed mid-session) | healthz ≤ 1 s; no hang across kill + 30 s outage + restart | healthz **0.41 ms**; 50/50 turns resolved, **0 errors**; restart ok; retry of the killed turn 200 | pass — but see RV-012: vacuous for the rung |
| F1b (rung killed mid-**escalation**) | in-flight rung-bound request resolves ≤ timeout + 5 s; healthz ≤ 1 s; restart; failed walk serves **no cached decision** on retry | in-flight request died typed (500) **0.05 s** after the kill (wall 2.05 s); healthz **0.61 ms**; restart ok; retry `cache_hit: false` → fresh walk → verify outcome | pass on every clause |
| F3a (121 s declared) | refused at the wire | **400 `schema.limit_exceeded`** — "limit `max_execution_time` exceeded: 121 > 120" | pass |
| F3c (rung behind +60 s proxy) | typed timeout inside deadline + 5 s; server healthy after | typed `TimeoutError` at **30.03 s** (the engine's rung-client bound fires long before the 120 s deadline); server healthy | pass — the 120 s deadline itself was not reached (RV-011's bound again) |
| F4 (proofs-only ladder, class A) | verifier seam never errors; absent verifier visible in traces | 20/20 requests: outcome **accept**, `threshold_verifier: "none"` in every trace, **0 errors** | pass; the pure-`verify` shape did not fire — class A accepts above threshold, and the absent verifier degrades to a visible `"none"`, not an error |
| F2 (malformed burst) | covered by class B level 5 | 51/51 rejected: 44 × 400, 7 × 413, zero 5xx (class B table) | pass |

F1b exists because F1 as specified could not bind: the 24×50 class-A
session plan is fully rule-decidable, so all 50 turns through the
fusion ladder resolve at the exact-rule layer (`policy_source:
"kind:choice"`, confidence 1.0) and the killed rung had zero traffic in
flight. F1b drives the lexical_semantic class the escalate arm showed
fires the rung, warms a *different* item than it kills (the
exact-decision cache is content-keyed — replaying the warmup's body
just hits the cache at 0.6 ms, which the first f1b run measured), and
exercises the kill exactly where the component is load-bearing.

### Findings opened by class F

- **RV-012 (fault-injection targeting, closed with F1b)** — a fault
  injected into a component the workload never uses is a green
  scoreboard over an untested path: F1 passed every clause while
  killing an idle rung (0/50 turns engaged it). Rule going forward: a
  fault-injection phase must first *prove engagement* (a warmup that
  demonstrably reaches the target — here: rung evidence in the decision
  chain) and must dodge the content-keyed cache when it wants a second
  live walk. The vacuous pass is the dangerous kind: it reads exactly
  like a real one in the gate table.
- **RV-013 (deadline layers, note)** — the D32 120 s execution ceiling
  is guarded by a much tighter inner bound: the engine's ~30 s
  rung-client timeout fires first in every late-rung scenario measured
  (F3c's +60 s proxy resolves as a typed `TimeoutError` at 30.03 s;
  class C's L2/L3 escalations resolve at ~30.2 s). Typed, healthy, and
  bounded — the deadline contract holds — but any future claim about
  "120 s worst-case behavior" must be re-measured against the *inner*
  bound, which is the one that actually binds.
