# Burst-compute task queue — T5500 down (2026-10-26)

**Trigger:** T5500 powered off at operator direction 2026-10-26 ~1:20 PM
Central (idle 27B server killed with the box; no training jobs were
running). Turn-on is operator action, on request.

**Platform policy (binding, Oct 10):** local hosts first; training
free-tier-first (Kaggle T4); Modal burst ≤ $50 authorized. Cloud
training stays user-gated per TRAINING.md §8 non-goals.

## What burst compute is NOT needed for (resolved by evidence)

- **Strands consensus pass** — DONE on fedora's 5060 Ti (cu130 venv,
  0.23 s/row, T1.3 full pass 183,262 rows; T1.4 analysis + T1.5 gate
  FIRED; merged-v5 built). The 42 GPU-h figure predates the 5060 Ti
  unlock. Not a burst item.
- **2B training arms** — T3.3/T3.3' complete (r16/r8 falsified v5;
  t33p passed both gates). fedora ran these; no burst.
- **24j 231-split leg** — ONNX verifier, ~49 ms/pass ⇒ minutes on CPU.
  Engineering item, not burst.

## Burst-relevant queue (ranked; timeframes are wall-clock estimates)

### B1. T3.4 — 4B own-tuned capacity arm [FIRED, top of queue]
- **Trigger check: FIRED.** Pre-registered trigger = "2B arm plateaus
  below 0.75 board after v5 + per-kind T." Champion 2B (E1-E) sits at
  0.6840 board; v5/v6 corpus arms (t33p 0.6580) confirm the plateau.
  The named contingency applies.
- **Spec:** Qwen3.8-4B-Distill, r48 LoRA, corpus = e1-iterb scaffold
  (the only corpus that ever produced a 0.8083-suite arm — v5/v6
  volume falsified) + T3.2 per-kind calibration; fork-anchor floor
  0.7662; same release gate (suite ≥ 0.68 AND JevBench ≥ 0.6494).
- **Host:** Kaggle T4×2 (free, 16 GB each) — 4B r48 LoRA at b4-bucket
  ckpt ON per the T2.3 sweet spot. Sessions ~9-10 h each (Kaggle
  12 h cap): epoch 1 ≈ 19k steps × 4B ≈ 30-40 GPU-h total ⇒ **2-3
  Kaggle sessions across 3-4 days** (weekly 30 h quota). Resume
  support exists (trainer `--resume`, c44ad6a).
- **Timeline:** prep (env + corpus upload + smoke) 1 session (day 1);
  train 2-3 sessions (days 2-5); eval legs + gate 0.5 day (day 6);
  conditional JevBench leg +0.5 day. **Total ≈ 1 week.**
- **Session log (updated 2026-10-28):** session 1 ran to the 12 h cap
  (kill at step ~2220/11009, upload cell never fired; trainer state
  step=2000, val 0.0161 / tok_acc 0.9938, train loss 0.077 falling).
  Recovery via kernel-output harvest + manual HF upload worked; run
  rate ~193 steps/h ⇒ full epoch ≈ 57 h ≈ 5 sessions if unmodified.
  Trainer **v6** shipped into the pipeline before session 3: (a)
  resume-state is uploaded to the HF repo from inside the trainer
  after every ckpt (`--hf-upload-repo/--hf-upload-token`, background
  thread, upload failure never kills training) — a 12 h kill now
  loses at most one upload interval; (b) val-plateau early stop
  (`--early-stop-patience 6` = 3k steps without best-val gain at
  val_every 500), plateau counter survives resume. Fingerprint
  unchanged, so the step-2000 state resumes under v6. Expected: 2-3
  sessions total instead of 5, depending on plateau onset.

### B2. JEV-27B contamination-audit probe [STAGED, blocked on Modal spend limit — 2026-10-28]
- **Staged:** runner arm `jev27` shipped (`run_jevbench.py --arm jev27`
  — autotrust's own serve_decide.py POST /v1/decide, thinking off,
  probabilities native); Modal burst app
  `benchmarks/decision-model/modal/run_b2_modal.py` (H100, weights in
  the `jev27-weights` volume, in-process 231 replay); `huggingface`
  Modal secret created. JEVBENCH.md arm entry 7 records the contract.
- **Blocker:** first launch refused — "Workspace has exceeded its
  spend limit" (no cost incurred). Operator action: raise the Modal
  spend cap; the burst fires unchanged afterward.
- **Host note:** T5500 ruled out — 2×V100 32 GB cannot hold 27B bf16
  + decision head, and the head has no llama.cpp path.
- **Ask:** run `autotrust/JEV-27B` over JevBench-231 through our
  engine bridge to establish the board-topper's true number on our
  lane (contamination audit; §14 dominance question).
- **Host:** 27B inference — NOT Kaggle-T4-able (88 GB class weights
  at Q3 ≈ 15-16 GB actually; possible on a single T4 at Q3_K but
  slow ~2-4 s/item ⇒ 231 items ≈ 15 min inference + setup).
  Realistically: **Modal H100 burst, ~$2-5 of the $50 budget**, 1-2 h
  wall-clock including image build; or 30 min on T5500 when next
  powered (the 27B already served there: IQ3_XXS bridge, 2×V100).
- **Timeline:** T5500-on path = **30 min**; Modal path = **half a day**
  (image + harness port + run).

### B3. Bridge MTP A/B [T5500-dependent — parked]
- Draft-mtp is the measured 2× lever on the 27B bridge (fedora memory).
  The A/B stages (a)/(b) from TRAINING §9 need the T5500 V100 pair.
- **Timeline when T5500 returns:** half a day.

### B4. Clock-lock sweep + power soak [T5500-dependent — parked]
- {unlocked,1300,1200,1100} × {120,110,100 W}; PSU-swap-dependent,
  operator-gated. Half a day when the box returns.

### B5. E1-F/G follow-ons (v3 family remap run) [parked — evidence says no]
- D42 mechanism shipped; per-family profile thresholds need
  re-measurement, but the v6 verdict ("confidence-gated volume ≠
  transfer") downweights any further v3-volume arm. Revisit only if
  the board submission demands a per-family row.

## Standing next levers NOT in burst scope (engineering, local)

- consensus-flip patch arm — evidence-ranked #1 in TRAINING tail, but
  it's a 2B fedora arm (9.7 h class), not burst.
- #150 eval-lane v2 (CPU fork lane determinism tolerance) — local.
- 24j 231-split leg — local CPU, minutes.

## Turn-on checklist for T5500 (operator)

1. Power on; boot systemd unit applies the 120 W caps (mandatory —
   PSU OCP regime; `relaunch-campaign.sh` sanitizes torn resume files
   if any).
2. Parked items resume: B2 (30 min), B3, B4, and task #18 (Vulkan
   probe if the V620 has arrived).
