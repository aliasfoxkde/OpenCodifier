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
- **T1.2 Slice probe (no gate)**: 1,000 corpus rows through
  `strands-decider serve` batched on V100. Verify: window-4096
  truncation rate on real corpus lengths, p50/p95 latency, response
  schema parity, agreement spot-check vs teacher-gold on 100 rows.
  Fixture-first per the model-import parity discipline.
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

### T3 — Arms

- **T3.1 Full-scale soft T=0.5** (after T2.1): completes the pilot's
  named follow-through. Pre-registered expectation: calibration win,
  accuracy wash — it is a record-completing arm, not a gate attempt.
- **T3.2 Per-kind temperature fit at export** (G4): fit per-kind T on
  holdout at end of training, bake into the exported config (strands
  pattern). Evaluated on rel200 + board; new calibration artifact
  shape; benefits every future arm including e1-e's successor.
- **T3.3 The gate-closing arm** (after T1.5-if-v5): v5 data +
  measured knobs (lr 1e-4, 2 epochs, r8/r16 A/B at full volume) +
  T3.2 export calibration. Success criterion: board ≥ 0.6494 with
  suite ≥ 0.68. If T1.4 kills the data hypothesis, T3.3 is replaced
  by the documented pivot: 0.8B is a calibration/cheap-arm only, and
  the release effort moves to e1-e successors at 2B.

### T4 — Engine product track (parallel, engineering-side)

- **T4.1 Model rung wiring (B5)**: llama.cpp `InferenceBackend` +
  `EngineHandle` decision lane, so the engine can reach the fork
  lane's measured quality (sim ladder 0.867 vs engine 0.683 is the
  largest product-side gap). Constraint set unchanged: ONNX/llama
  behind traits, no heavyweight deps in core, bit-determinism proof
  or a DECISIONS-recorded re-key.

### T5 — Operator-gated queue [OP]

Board submission go (assemble receipts index first — now includes the
strands independent replication), bridge MTP A/B, corpus ceiling
estimators (accuracy half; T1.4 is the noise half), T5500 power soak,
24j/24k, strands-ONNX watch.

## Sequencing

```
now ──► T2.1 T2.2 (engineering, no gate) ──► T3.1
    └─► T1.1 T1.2 (slice probe, no gate) ──► [OP] T1.3 ──► T1.4 ──► T1.5
                                                              │
                              v5 ──► T3.3 ◄── T3.2 (either way)
T4.1 interleaves on engineering time; T5 waits for operator.
```

Hosts: T5500 = training + T1 (V100 pair, power-cap regime); fedora =
bridge duty + CPU eval legs; Kaggle T4 free tier / Modal ≤ $50 burst =
overflow training per platform policy.

## Discipline

Every number in this plan cites a shipped record (BENCHMARKS.md,
TRAINING.md, RESEARCH.md §15.10). Pre-registrations: T1.5 threshold,
T3.1 expectation. No sub-gate arm numbers reach the board. Findings
land in docs + commit + push before the thread closes. Slice-first
for any new model contact (model-import parity discipline).
