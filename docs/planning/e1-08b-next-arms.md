# 0.8B next arms: soft-label distillation (v2) + the noise-gate A/B grounding

Status: DESIGN / staged-pending-operator (2026-10-10, during E1-E).
Research basis: operator question "can we improve the 0.8B, or are 2B
models the best bet?" — answered: 0.8B ceiling ~0.69-0.72 via the levers
below; 2B stays the model rung; 0.8B becomes the calibrated cheap rung.

## Measured starting point

- arm B (e1-quarx005, 0.8B, merged-v3+pool@0.05, 187,822 rows,
  11,615 steps, best val 0.0294 @ step 11500): suite 0.65 GATE_FAIL.
- Third-party 0.8B (Jev-Style-v3): 0.6494 — two-team capacity
  signature at this tier.
- Public 2B arms: decider-2b 0.7576, imajev-2b 0.7359 (public-231).

## Levers (ranked, expected gain)

1. **Soft-label distillation from a 2B teacher** (+2-5 pp): GKD/on-policy
   literature (Agarwal et al.; Thinking Machines 2025) — students reach
   ~90% of teacher; reverse-KL beats offline KD by +1-2 pp absolute.
   Trainer extension: replace hard CE with teacher KL over the CLOSED
   verdict-token set in the `sup_logits` path (k-vectors, not
   vocab-sized). Corpus `e1-softlabel-v1` = merged-v4 + teacher
   distributions, one measured change per version.
2. Readout/serving shape parity (0-5 pp, measured at 2B: 0.808
   letters vs 0.758 served tree).
3. Seed soup of LoRA adapters (+0.5-1.5 pp; trainer is seeded — two
   runs suffice).
4. On-policy refinement phase 2 (+1-2 pp over #1; needs a generation
   loop).
5. Per-rung calibration (no accuracy delta; makes a 0.68-0.70 model a
   GOOD rung; abroad worth ~+16 pp composite per F24).

## Teacher contingency (E1-E decides)

- GATE_PASS (suite >= 0.75): teacher = our E1-E arm — license-clean,
  publishable lineage.
- GATE_FAIL: teacher = decider-2b-v11 (0.7576, benchmarked in our
  harness) — but distillate carries provenance constraints on
  open-weights release (release policy applies to derived weights).

## Noise-gate A/B grounding correction (2026-10-10 recon)

The recorded pool (81,740 rows @ top<0.5, manifest
`results/corpus_v3_train.gates-manifest.json`) was measured on
`jev-distill-corpus-v3/train.jsonl` (655,806 rows, sha `71e3ac50…`,
byte-identical copy verified on NAS at
`/nas/Temp/work/oc-model-eval/corpora/jev-distill-corpus-v3/`).
**That file's ids are synthetic `v3_*`; every e1 arm trained on the
merged line (`typed-decisions:*` record ids from dataset.jsonl).** The
pool therefore does NOT map onto any trained arm's rows, and merged-v4
records carry only scalar teacher metadata (`teacher.label_32b`,
`teacher.jev`) — no per-option distribution for corpus_gates' noise
gate. Consequences:

- The pool-as-measured A/B would be a new corpus line (v3 train split),
  not an A/B on the program's main line — parked unless that line is
  ever trained.
- A merged-line noise gate requires teacher distributions over
  merged-v4 — i.e. the same prerequisite as lever 1. One
  **teacher-scoring campaign** (VIVERE extraction over 183,262 records)
  produces both `e1-softlabel-v1` AND the `_gates` facts that let
  `corpus_gates.py` define a merged-line pool reproducibly.

## Sequencing (gpuq; both GPUs busy until E1-E lands ~07Z Oct 11)

1. E1-E verdicts (pre-registered; JevBench leg if PASS).
2. Teacher-scoring campaign on merged-v4 (staged-bundle rule: validate
   on a 1k-record smoke before the full run; teacher per contingency;
   both V100s shardable by record range).
3. 0.8B-v2 arm: e1-softlabel-v1 + shape-parity readout + seed 42;
   second seed for the soup when a GPU frees.
4. Merged-line noise gate A/B (from the campaign's facts) — after v2,
   one measured change per corpus version.

## Platform control rule

T5500 arms are fp16+sdpa (no bf16 on Volta); fedora arms are
bf16+eager. Any T5500 0.8B comparison against arm B's 0.65 needs a
same-platform control (arm B's corpus re-run on T5500) or is recorded
as a two-delta result. E1-E precedent: platform delta recorded, one
intended variable per arm.

## Operator gates

- VIVERE lane re-open for the teacher-scoring campaign (operator
  paused it 2026-10-09; re-asked 2026-10-10 — staged on go).
- Teacher choice contingent on E1-E verdict (above).

## Trainer optimization backlog (2026-10-12, from the v2 launch)

Measured during the concurrency sweep before the v2 arms (7 pilots,
4000-row samples, b2/b4/b8/b16 × ckpt ±, bucketed ±):

- **Resumability (before the next long arm)**: the trainer keeps only
  `adapter-best` + final; no optimizer state — a death mid-arm means a
  full restart. Add checkpoint triplet (optimizer + GradScaler + step +
  RNG + dataloader position) and `--resume <run-dir>`; verify a
  resume-continuation reproduces the uninterrupted loss curve on a 1k
  pilot. Enables GPU time-slicing too.
- **Sweep verdict (measured)**: b2×a8 fastest (7.51 s/step); larger
  micro-batches lose monotonically to padding waste (8.58 / 10.49 /
  12.19); no-checkpoint OOMs MID-RUN on long-sequence batches —
  peak-case, not average-case; VRAM never binding (≤6.3 of 15.77 GiB).
  0.8B is only 1.26× faster/step than the 2B — shape/overhead-bound.
- **Batch-size ranking is not stable**: it was measured with
  `--length-bucket` broken (inert flat-shuffle, fixed d989c9d). Redo
  the micro-batch sweep with working bucketing before believing b2 —
  the padding tax was the reason big batches lost.
- **causal_conv1d canary** (post-arms): F.conv1d is in the hot path
  (visible in the OOM traces). Same-seed loss-trajectory parity probe
  required — a kernel swap may not change numerics silently. Keep if
  ≥1.3×; applies to 2B arms too.
- **Falsified non-levers** (do not rediscover): two arms on one GPU
  (throughput is not VRAM-bound); DDP for the A/B (two runs needed
  anyway; all-reduce changes numerics); tokenizer parallelism (cache
  builds in seconds); torch.compile (low expected gain on V100 fp16).
- **Sequence packing** (design option): concat-to-budget kills padding
  but needs cross-row masking and its own A/B — trainer-versioned
  change, not a knob.
