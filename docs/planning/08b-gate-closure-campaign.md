# Campaign plan: 0.8B gate closure + corpus noise grounding (2026-10-13)

Status: PLANNED (not started). Owner: agent, operator-gated items marked
[OP]. Supersedes nothing; feeds docs/PLAN.md and TRAINING.md §12.4.

## The binding constraint, stated plainly

The release gate for an own arm is **suite ≥ 0.68 AND JevBench ≥ 0.6494**
(docs/BENCHMARKS.md, docs/TRAINING.md). e1-e (2B) passes both. The 0.8B
class fails the board leg: v2-hard holdout 0.7000 → board 0.5584
(−11.4 pp suite→board discount), **9.1 pp short of the 0.6494 bar**.

The pilot window measured the training knobs; none of them is an
accuracy lever (lr 1e-4 directional, r8 small-data candidate, epochs
2–3 flat, soft T = calibration only). The soft arm was an accuracy
wash at full scale (0.675 vs 0.700 holdout). **Conclusion: no
configuration of the current corpus credibly closes the 0.8B board
leg. The binding constraint is data — label quality and relational
density — not the training recipe.** strands-decider-E2B's independent
adjudication (§15.10) confirms the corollary: every 2B arm loses the
same hard-compositional families, and the one axis Amazon's arm wins
on (ECE 0.045) came from baked per-kind calibration, not capacity.

## Gap register

| # | Gap | Evidence | Closed by |
|---|---|---|---|
| G1 | 0.8B board leg −9.1 pp; no current candidate closes it | v2 board 0.5584 vs bar 0.6494; soft wash | T1 → T3.3 |
| G2 | Teacher-gold is self-referential (94–96 % assumed, not measured); quarantine pool 69 % mixed-gold clusters | TRAINING.md; merged-v2 quarantine analysis | T1.4 |
| G3 | No trainer resumability — every long arm risks a torn run | trainer backlog; relaunch-campaign.sh precedent | T2.1 |
| G4 | Calibration pipeline: single post-hoc T vs per-kind training-time fit | strands board-best ECE 0.045 with baked per-kind T vs our e1-e post-hoc T=3.362 | T3.2 |
| G5 | Engine model rung absent in Rust: simulated ladder 0.867 vs engine 0.683 | F23 fusion study; §15 enhancement program | T4 |
| G6 | Board submission package not assembled end-to-end | #107; draft + 8 operator questions exist | T5 [OP] |
| G7 | Batch-sweep evidence invalid (length-bucketing defect) | trainer backlog `--length-bucket` inert | T2.3 |
| G8 | causal_conv1d parity unverified | trainer backlog canary | T2.2 |
| G9 | §15 items 24j/24k sized both-GO, queued | RESEARCH.md §15.6.2 | T5 |
| G10 | strands runtime use blocked on an official ONNX export | §15.10: torch-only serving path | watch only |

## Tracks

### T1 — Corpus noise grounding: strands as second teacher

G2 first; G1's data lever depends on its verdict. strands-decider-E2B
is architecturally decorrelated from our qwen/minicpm teachers
(gemma base, different training line), Apache-2.0, speaks the same
SystemOne wire contract, adjudicated live at 0.7706/ECE 0.045.

- **T1.1 Host prep (T5500)**: power-cap systemd unit active (mandatory,
  t5500-power-cap regime); checkpoint + venv staged (286 M copy from
  fedora; cu126 supports sm_70 natively — the sm_120 trap is
  5060 Ti-specific); bridge duty untouched on fedora.
  **Contingency (2026-10-13): T5500 is DOWN awaiting a replacement
  PSU** — T1.2 starts on fedora CPU instead: bf16 CPU serve measured
  ~1.7 s/task steady in the §15.10 adjudication, so a 1,000-row probe
  is ~30 min single-threaded and needs no GPU. Full pass on CPU is
  3–5 days — defer T1.3 to the V100 pair or accept CPU days as a
  conscious trade; do not block T2 on it either way.
- **T1.2 Slice probe (no gate)**: 1,000 corpus rows through
  `strands-decider serve` batched on V100 (or fedora CPU per the
  T1.1 contingency). Verify: window-4096 truncation rate on real
  corpus lengths, p50/p95 latency, response schema parity, agreement
  spot-check vs teacher-gold on 100 rows. Fixture-first per the
  model-import parity discipline.
- **T1.3 Full pass [OP]**: merged-v4 states → second-opinion column.
  At ~0.1–0.2 s/task batched: ~3–6 h per 100 k rows on one V100.
  No API spend (local weights) — this is the noise half of the
  operator-gated "corpus ceiling/noise estimators" item.
- **T1.4 Analysis**: agreement matrix (teacher vs strands vs gold),
  per-family disagreement density, noise bounds → **replaces the
  assumed 94–96 % with a measurement**; disagreement pool = the
  relabel-candidate set by construction (max-information rows).
- **T1.5 Decision gate [pre-registered]**: threshold fixed BEFORE
  running T1.4 — if teacher-vs-strands disagreement exceeds 5 % of
  rows, build merged-v5 with consensus labels / ensembled soft labels;
  below 5 %, v4 stands and the noise hypothesis is dead (record
  either way; a null result falsifies G1's data-quality theory too).

### T2 — Trainer readiness (blocks all long arms)

- **T2.1 Resumability**: step-checkpoint save/load; resume-parity probe
  (kill at step k, resume, loss curve must rejoin); torn-file
  sanitizer (relaunch-campaign.sh pattern).
- **T2.2 causal_conv1d canary**: same-seed parity with/without the
  kernel; one 30-min run settles a standing foot-gun.
- **T2.3 Batch-sweep redo** with fixed bucketing (retires G7; the D9
  shape evidence stays invalid until this runs).

### T1 results (2026-10-13, complete)

- **T1.2 slice probe DONE** (1,000-row stride sample, GPU): teacher vs
  strands agreement **0.8748** over 1,030 questions (noul 0.908
  n=790 — dominated by 1-question SargeDev relevance rows; choice
  0.949 n=78; score 0.679 n=162). Zero request errors; max state
  5,934 chars — **no window-4096 truncation risk anywhere**. Runner:
  `benchmarks/decision-model/runner/strands2b_slice_probe.py`;
  artifacts on fedora `~/oc-model-eval/runs/strands2b-slice-v1/`.
- **GPU unlocked on fedora**: strands' torch (cu126) has no sm_120
  kernels, but a cu130 venv (`~/venvs/strands-gpu`) serves the same
  checkpoint on the 5060 Ti at **0.23 s/row vs 20.3 s CPU (~88×)**,
  device parity verified (max prob delta 0.007, zero argmax flips vs
  the CPU fixture). Full-pass economics collapse from CPU-days to
  ~½–1 GPU-day.
- **T1.3 full pass DONE** (operator go 2026-10-13; resumed once after
  an unrelated ENOSPC on the host, zero rows lost): all **183,262
  merged-v4 rows → 188,061 questions**, 1 error question,
  `~/oc-model-eval/runs/strands2b-full-v1/`.
- **T1.4 analysis DONE** — teacher (gold) vs strands agreement:
  **0.8715 overall → 12.85% disagreement** (noul 0.9081 n=144,391 /
  choice 0.9251 n=14,113 / **score 0.6667 n=29,557**). By source:
  SargeDev relevance 0.911, vivere 0.769, **typed-decisions 0.610**.
- **T1.5 gate: FIRED** (pre-registered threshold 5%; measured 12.85%,
  2.6× over; score kind alone is 33.3% contested). **merged-v5 BUILT**
  (2026-10-13, `runner/build_merged_v5.py`, policy fixed in the script
  docstring before any flip count was observed): flip a label iff
  teacher-facts-v1 and strands BOTH disagree with gold; everything
  else byte-identical; suite rows untouched; flipped distributions =
  strands' native probs with recomputed EV (v4's `score` field verified
  = EV over prob keys, 2400/2400) and confidence = mean(strands,
  teacher tops). Results: **3,652 flips / 188,062 questions (1.94%)**
  (noul 2,843 / score 551 / choice 258; SargeDev 2,564 /
  typed-decisions 891 / vivere 197); agree 158,655;
  keep-2v1 25,037; split3 477. Internal check: teacher dissent 4.88%
  ≈ the recorded 94.9–95.1% teacher-gold agreement. Score
  disagreements are 74% adjacent-bucket (ordinal adjacency, not
  random flips). Raw disagreement (12.85%) ≠ flip pool (1.94%): the
  disagreement map lives in `merged-v5.disputes.jsonl` for any
  downstream filtering; the corpus is the consensus-label variant of
  the pre-registered "consensus labels / ensembled soft labels".
  sha256 `b04f87fc…`; manifest + disputes alongside at
  `corpora/merged-v5/` (NAS + fedora).

### T3 — Arms

- **T3.3 LAUNCHED (2026-10-13 ~23:05 local, fedora)**: pipeline
  `runs/t33-v5.sh` — r16/a32 then r8/a16, both 1 epoch (the measured
  standing config; "2 epochs" in the earlier text is superseded by
  TRAINING.md's "1 epoch stands"), E1-E recipe otherwise byte-identical,
  on the merged-v5 rows corpus `e1-softlabel-v2` (rows sha
  `018f6a1d…`, built from merged-v5 through the byte-identical
  build_softlabel_corpus path; per-kind teacher-agreement rose to
  noul 0.9721 / choice 0.9765 / score 0.9608 from v4's
  0.9524/0.9579/0.9422 — the flip arithmetic checks per kind). Each
  arm: holdout + rel200 readout evals, per-kind T baked to
  `adapter-best/perkind_calibration.json` (T3.2 export step), then a
  gate-gated JevBench board leg (holdout ≥ 0.68) on CPU llama-server.
- **T3.3 r16 VERDICT: GATE_FAIL — merged-v5 falsified (2026-10-14)**.
  Training healthy (val 0.0168, tok_acc 0.9943); holdout **0.5167**
  vs e1-e's 0.8083 on the same suite (rel200 0.4071 vs 0.718–0.732;
  relational class 0.15). Board skipped by the gate. Corpus-only
  delta — recipe byte-identical — so merged-v5's mix is the defect
  (teacher-agreement rose yet transfer collapsed: agreement ≠ the
  binding constraint). merged-v6 promoted to the named next lever;
  full record in TRAINING.md "T3.3 r16 arm" (2026-10-14). Recovery
  ran via `runs/t33-recover.sh` after the co-tenant-27B OOM (fixed:
  real rc capture, perkind argv paths, GPU pre/mid-flight guards).
- **T3.1 Full-scale soft T=0.5** (after T2.1): completes the pilot's
  named follow-through. Pre-registered expectation: calibration win,
  accuracy wash — it is a record-completing arm, not a gate attempt.
- **T3.2 Per-kind temperature fit at export** (G4): fit per-kind T on
  holdout at end of training, bake into the exported config (strands
  pattern). Evaluated on rel200 + board; new calibration artifact
  shape; benefits every future arm including e1-e's successor.
  **Pre-test DONE (2026-10-13, in-sample, disclosed — sizing probe,
  not a shipped artifact)**, tool `runner/perkind_holdout_fit.py`
  (validates against the strands record: reproduces ECE 0.0451 /
  auto@5% 122/231 exactly):
  - v2-08b board rows (231): raw ECE 0.1952 → single global T 0.1126
    → **per-kind T\* 0.0847** (bool 4.55 / choice 2.93 / score 2.12).
  - v2-08b holdout (120): raw 0.1406 → own fit T\*=1.64 → 0.1175;
    the e1-e global 3.362 applied to this arm **worsens** it to
    0.2428 — one global T does not transfer across arms.
  - strands rows with our global 3.362: ECE 0.045 → 0.257 — same
    non-transfer, their edge is per-arm + per-kind.
  Verdict: the axis is real and cheap; bake per-kind T at export per
  arm. auto@5% does not move at 0.8B accuracy (nothing to auto-accept
  at 55%); it moves once accuracy does.
- **T3.3 The gate-closing arm** (after T1.5-if-v5): v5 data +
  measured knobs (lr 1e-4, 2 epochs, r8/r16 A/B at full volume) +
  T3.2 export calibration. Success criterion: board ≥ 0.6494 with
  suite ≥ 0.68. If T1.4 kills the data hypothesis, T3.3 is replaced
  by the documented pivot: 0.8B is a calibration/cheap-arm only, and
  the release effort moves to e1-e successors at 2B.
- **T3.4 Capacity contingency: 4B own-tuned** [CONTINGENCY, not
  primary — operator direction 2026-10-13]: the board leader is a 2B
  (minicpm 0.7792) and strands is a 2B (0.7706), so 2B capacity is
  proven sufficient to top this board; smaller = faster = better
  product fit. Primary path is the 2B on v5 (T3.3). T3.4 (4B r48 LoRA
  on v5 + T3.2 calibration, fork-anchor floor 0.7662) triggers only
  on a named trigger: **the 2B arm plateaus below 0.75 board after
  v5 + per-kind T** — evidence the residual gap is base-model
  capacity, not data. Host: T5500 V100 pair (PSU) or QLoRA on fedora.
- **Base selection is judged on through-our-lane substrate, not board
  numbers (2026-10-14, evidence recorded)**: the minicpm 0.7792 is
  **lane-bound** — their prompt format + native letter-readout head
  (BENCHMARKS.md ⁸/136); through OUR fork lane the same adapter
  scores **0.6407** with its control base refitting to the same
  0.6407 (⁹) — the adapter adds ~nothing off its home lane. Measured
  2B substrates through our lane: Qwen3.5-2B **0.725** vs MiniCPM5-2B
  **~0.64**, and the qwen3.5 toolchain (trainer, verdict-slot readout,
  D15 calibration, kai/julia contracts, serving path) is fully reused
  by every qwen arm while MiniCPM5 needs new arch support (its GGUF
  rung lane CLOSED on exactly that friction). Hence: qwen3.5-2B primary
  (T3.3), qwen3.5-4B contingency (T3.4); a MiniCPM5-base arm is a
  [CANDIDATE] family-axis experiment only if T3.3's recipe transfers
  and the family axis (RESEARCH §16) becomes a funded question.

## What "beyond strands" means (targets, three legs)

strands-decider-E2B is the bar to beat on BOTH axes: board accuracy
> 0.7792 (the minicpm leader) and raw ECE < 0.045 / auto@5% > 0.528
(the strands calibration). The evidence says no single training-knob
axis gets there; the three legs compose:

1. **Data** (T1): consensus labels + relational density — attacks the
   hard-compositional wall every 2B arm hits (the 53-miss tier).
2. **Calibration** (T3.2): per-kind T baked at export — attacks the
   axis strands won; the pilot's soft-T=0.5 ECE 0.068 at pilot scale
   says the axis is real.
3. **Capacity** (T3.3, T3.4): the 2B-on-v5 gate/leader arm is the
   primary; the 4B is a contingency behind a named plateau trigger.

### T4 — Engine product track (parallel, engineering-side)

- **T4.1 Model rung wiring (B5)**: llama.cpp `InferenceBackend` +
  `EngineHandle` decision lane, so the engine can reach the fork
  lane's measured quality (sim ladder 0.867 vs engine 0.683 is the
  largest product-side gap). Constraint set unchanged: ONNX/llama
  behind traits, no heavyweight deps in core, bit-determinism proof
  or a DECISIONS-recorded re-key.
  **Scoping DONE (2026-10-13, fork source read at
  `/nas/Temp/work/oc-model-eval/llama.cpp`)** — one correction to the
  plan's shape: the fork's `POST /decision` is *not* a tensor-in /
  tensor-out model, so it does not fit [`InferenceBackend`]
  (DenseTensor maps, `crates/opencodifier-runtime/src/backend.rs:31`).
  It is a **decision backend**: one shared prefix (system +
  instructions, chat-template-rendered server-side — all contexts must
  share it) plus 1–256 context strings and a finite JSON schema; the
  server scores every schema row in one batched pass. Per-context
  result: `decision.<field>` (selected value) + `fields.<field>` with
  `value`, `probability`, full `distribution` {choice → p} when the
  field is enum/numeric-with-probs, `interval_p10_p90` for numeric
  fields, `scored_nodes`, plus usage/timings. Design that follows:
  1. `opencodifier-runtime`, `llamacpp` feature: new `DecisionBackend`
     trait (contexts + finite option sets → per-field distributions;
     blocking HTTP, no async in runtime) with the fork client as first
     impl; heavyweight dep (HTTP client) confined to this crate/feature
     per D11.
  2. `opencodifier-model`: a `fork` contract module alongside
     `kai`/`julia` — schema per kind (bool = 2-value enum; choice =
     candidate-id enum; score = numeric levels), question rendering
     shares `KaiQuestion`-style payloads; fork `distribution` maps
     straight onto the IR's outcome distribution (calibration seam
     applies downstream — the T3.2 per-kind T artifact becomes the
     engine lane's calibration config).
  3. `opencodifier-engine`: ladder rung / `EngineHandle` lane over the
     trait. Determinism: batch-invariance must be *proven* (same
     request in different batch compositions → byte-identical
     distributions) before batching ships; until then the engine lane
     pins batch = 1 (trivially deterministic).
  kai.rs is the in-repo template for contracts 1–2 (render → batch →
  tensors → backend → f64 softmax); here the server does the scoring
  and returns probabilities, so the module is render + wire + validate
  only.

### T5 — Operator-gated queue [OP]

Board submission go (assemble receipts index first — now includes the
strands independent replication), bridge MTP A/B, corpus ceiling
estimators (accuracy half; T1.4 is the noise half), T5500 power soak,
24j/24k, strands-ONNX watch.

### The white paper (credibility program — outline, 2026-10-14)

Working thesis: **multi-family consensus labels + per-kind calibration
lift small-model typed-decision accuracy and honest confidence** —
every claim below has, or is generated by, a shipped record:

1. **C1 Label noise is real and measurable** — T1.4/T1.5: 94–96 %
   teacher-gold agreement was assumed; cross-family measurement found
   12.85 % disagreement; the pre-registered consensus rule flips
   1.94 %. Methodology contribution: the disputes ledger.
2. **C2 Consensus data moves the gate** — T3.3 A/B (v4 vs v5, same
   recipe, both arms + legs). The paper's central experiment.
3. **C3 Calibration is a per-kind, per-arm property** — T3.2: global T
   does not transfer across arms (0.045→0.257 on strands with ours);
   per-kind fit at export does. Board-ECE deltas as receipts.
4. **C4 The system beats its best model in-domain** — 0.933 vs 0.8083
   (fusion-v2 vs e1-e alone, F28) + the honest out-of-domain caveat
   (0.558 as-shipped on the 231) and the gate-transfer fix.
5. **C5 Determinism + latency are engineering contributions** —
   bit-exact replay, 1.3 ms zero-ML floor, per-tier table (D16).
6. **C6 (only if arms run)** the transfer-vs-size curve (§16).

Venue: arXiv + the board + HF model cards. Non-negotiables per brand:
one number per surface, no universal claim, no "fully open" claim
(RELEASE_POLICY.md), limitations section states the in-domain gate
fitting and the hard-compositional tier wall. Drafting starts after
T3.3's verdict (C2 is its result).

## Sequencing

```
now ──► T2.1 T2.2 (engineering, no gate — code + CPU smoke on fedora) ──► T3.1
    └─► T1.2 DONE ──► T1.3 RUNNING (fedora GPU, overnight) ──► T1.4 ──► T1.5
                                                                     │
                                    v5 ──► T3.3 (2B primary) ◄── T3.2 (either way)
T3.4 fires only on the 2B-plateau trigger. T4.1 interleaves on
engineering time; T5 waits for operator.
```

Hosts: T5500 = training + T1 (V100 pair, power-cap regime) — **DOWN
2026-10-13, replacement PSU on order; all T2/T3 arms queue behind it**;
fedora = bridge duty + CPU eval legs — root/home are one fs (nvme0n1p3,
1.9 T) and hit 8 G free on 2026-10-13; remediated same day (~7 G caches
reclaimed, the 2.4 T stale gitforge-audit-20260822 snapshot + 76 G of
superseded eval models archiving copy-first to /nas/Temp/archive/
before delete-verify, spin-down ops staged for the 16 T sda);
Kaggle T4 free tier / Modal ≤ $50 burst = overflow training per
platform policy.

## Discipline

Every number in this plan cites a shipped record (BENCHMARKS.md,
TRAINING.md, RESEARCH.md §15.10). Pre-registrations: T1.5 threshold,
T3.1 expectation. No sub-gate arm numbers reach the board. Findings
land in docs + commit + push before the thread closes. Slice-first
for any new model contact (model-import parity discipline).
