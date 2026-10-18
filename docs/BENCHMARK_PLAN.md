# Plan to Outperform JEV-27B — Competitive Overmatch Plan (v2)

**Owner:** OpenCodifier
**Status:** Draft v2 — substantially revised after BENCHMARKS.md, DEEP_AUDIT.md, TRAINING.md audit
**Date:** 2026-10-18
**Target:** Outperform autotrust/JEV-27B (88.70 on JevBench-231) on JevBench accuracy; close the OOD gap on jabr
**Reference:** `docs/BENCHMARKS.md`, `docs/TRAINING.md`, `docs/RESEARCH.md`, `docs/DEEP_AUDIT.md`

---

## Changelog v2

v1 had four critical errors, all corrected here:

1. **Wrong target**: v1 aimed to beat TypeSafeAI JEV (87.18) — the *teacher* model. The correct target is autotrust/JEV-27B (88.70) — the open distillation that beat it. The gap to JEV-27B is 13 pp, not 1.9 pp.

2. **Stale Jebadiah number**: v1 cited "Jebadiah-9B freeform at 85.3% on JevBench-231" as established fact. This is **not confirmed on JevBench-231**. The 0.858 number is from the **120-item locked suite** (BENCHMARKS.md footnote 16). JevBench-231 performance is unknown. The "1.9 pp gap" is therefore a phantom.

3. **Fork arm is uncalibrated on freeform**: The fork arm (75.71 on JevBench-231) uses a constrained tree readout. The freeform improvement claim (~10 pp) is inferred from Jebadiah, not measured from our own arm. Applying Jebadiah's freeform delta to our fork arm is unsupported.

4. **E1-E is not the limiting factor on JevBench**: E1-E scores 0.6840 on JevBench-231. The per-family breakdown shows the gap is concentrated in `multi_hop` (0.389), `judge_hard` (0.412), `long_policy` (0.368) — compositional and long-context failures, not readout failures. Freeform readout won't fix those families.

---

## Contents

1. [What the benchmarks actually say](#1-what-the-benchmarks-actually-say)
2. [The gap that is real and the gaps that are phantom](#2-the-gap-that-is-real-and-the-gaps-that-are-phantom)
3. [The single most important action: measure Jebadiah-9B on JevBench-231](#3-the-single-most-important-action-measure-jebadiah-9b-on-jevbench-231)
4. [Root-cause analysis: where the fork arm actually loses](#4-root-cause-analysis-where-the-fork-arm-actually-loses)
5. [The correct competitive strategy](#5-the-correct-competitive-strategy)
6. [Phase 0 — Measurement infrastructure (gate for everything else)](#6-phase-0--measurement-infrastructure-gate-for-everything-else)
7. [Phase 1 — Freeform readout on our own arms](#7-phase-1--freeform-readout-on-our-own-arms)
8. [Phase 2 — Fine-tuning strategy corrections](#8-phase-2--fine-tuning-strategy-corrections)
9. [Phase 3 — The OOD generalization problem](#9-phase-3--the-ood-generalization-problem)
10. [Governance and pre-registration](#10-governance-and-pre-registration)
11. [Decision table](#11-decision-table)
12. [Risks and contingencies](#12-risks-and-contingencies)
13. [What this plan does NOT cover](#13-what-this-plan-does-not-cover)

---

## 1. What the benchmarks actually say

### Board A — JevBench-231 (the only comparable public row)

Source: BENCHMARKS.md Board A, JEVBENCH.md method of record.

| system | acc | family-macro | ECE | auto@5% | p50 | determinism |
|---|---:|---:|---:|---:|---:|---|
| **autotrust/JEV-27B** (open, Apache-2.0; Qwen3.8-27B + 109 M-param decision block) | **88.70** | — | 0.0009 | — | — | — |
| **TypeSafe Jev 1.13** (hosted API, closed; the teacher) | 87.18 | — | — | — | — | — |
| **strands: amazon/strands-decider-E2B-gemma4-v1-2610** (2B Gemma-4 + pointer head, our run) | **0.7706** | 0.7612 | 0.045 | 0.528 | 1.75 s | 231/231 |
| **minicpm: MiniCPM5-2B-Jev** (ytbai LoRA + letter readout, our run) | **0.7792** | 0.7816 | 0.054 | 0.524 | 3.06 s | 231/231 |
| **fork: Qwen3.5-4B UD-Q4_K_XL** (tree mode, base model, no decision tune) | **0.7662** | 0.7571 | 0.070 | — | 2.28 s | 231/231 |
| **gguf: mc5-jev MiniCPM5-2B-Jev Q8_0** (our GGUF conversion, fork tree mode) | 0.6407 | 0.6061 | 0.058 | 0.299 | 1.30 s | 231/231 |
| **e1-e: Qwen3.5-2B + own LoRA** (verdict-slot, our run) | 0.6840 | 0.6832 | 0.207 | 0.100 | 0.58 s | 231/231 |
| OpenCodifier engine (relational solver over lexical) | 0.3766 | 0.4011 | 0.402 | 0.000 | 2.0 ms | 231/231 |

Key observations:
- **strands (0.7706)** and **minicpm (0.7792)** are the strongest open arms we've measured on JevBench-231, both ≈11–12 pp above our fork arm.
- **JEV-27B at 88.70** is 10.7 pp above strands.
- **fork arm (0.7662)** is our strongest unassisted open-model arm — no decision tuning, base model only.
- **E1-E (0.6840)** is our own-trained arm: strong on the relational slice (fact 1.00, tool_selection 0.917) but collapses on multi-hop (0.389) and judge_hard (0.412).
- **mc5-jev GGUF (0.6407)** — our own GGUF conversion of MiniCPM5-Jev — is 14 pp below the authors' path. The adapter loses its decision capability through the conversion lane. This is a warning: our GGUF conversion pipeline destroys adapter quality.

### Board B — 120-item locked suite (not comparable to JevBench)

The 120-item suite tests three classes (`metadata_match`, `lexical_semantic`, `relational_compositional`) not represented in the JevBench composition. Numbers on this suite **cannot be compared to JevBench-231 accuracy** (footnote 16 of BENCHMARKS.md confirms this explicitly). The Jebadiah-9B "0.858" number is from this suite, not JevBench-231.

### Board C — jabr OOD (866 cases, 49 tasks, CC0)

Our engine scores 0.401. Von (best open) is 0.704. This is the hardest benchmark and requires a fundamentally different approach.

---

## 2. The gap that is real and the gaps that are phantom

### Real gap: 13 pp to JEV-27B on JevBench-231

Our best measured arm (fork Qwen3.5-4B at 0.7662) is 12 pp below JEV-27B (88.70). This is the primary metric. Every other gap analysis in v1 was contaminated by the JevBench/suite conflation.

### Phantom gap: "Jebadiah-9B freeform at 85.3%"

This number does not exist on JevBench-231. It is from the 120-item suite. The plan cannot use it as a baseline until Jebadiah-9B is measured on the actual JevBench-231 split.

### Real gap: E1-E loses in compositional families

E1-E (0.684 on JevBench-231) is 8 pp below the fork arm (0.7662) despite being our own-trained arm with a verdict-slot readout. The per-family breakdown (BENCHMARKS.md footnote ⁶) shows: `multi_hop` 0.389, `judge_hard` 0.412, `long_policy` 0.368, `temporal_numeric` 0.133 — all near zero. This is a **training data composition problem**, not a readout problem. Freeform won't fix it.

### Real gap: OOD generalization

jabr Board C at 0.401 vs Von 0.704. This is a 0.303 gap that no fine-tuning on in-distribution data will close.

---

## 3. The single most important action: measure Jebadiah-9B on JevBench-231

Before any training investment, we need the actual number. The Jebadiah-9B freeform claim (≈85% on JevBench-231 estimated from 0.858 on the 120-item suite) is the most important unmeasured claim in the plan.

**Action:** Find a community Jebadiah-9B-Q8_0 GGUF, serve with llama.cpp, run through the authors' JevBench harness with freeform readout (temperature 0, newline stop, fuzzy answer extraction).

**Three possible outcomes:**

| Outcome | JevBench-231 | Implication |
|---|---|---|
| Jebadiah-9B freeform ≥ 88.70 | ≥ 88.70 | JEV-27B is the target; we need a comparable arm. A 4B decision-tuned model trained on freeform data should approach this. |
| Jebadiah-9B freeform 85–88.70 | 85–88 | TypeSafeAI's 87.18 is within reach with a 4B fine-tune. The fine-tuning path is confirmed. |
| Jebadiah-9B freeform < 85 | < 85 | The freeform interface alone does not close the gap. The problem is model quality and/or training data. Larger model or different architecture needed. |

**This measurement gates the entire fine-tuning strategy.** Do it first.

---

## 4. Root-cause analysis: where the fork arm actually loses

The fork arm (0.7662 on JevBench-231) uses a constrained tree readout on a base Qwen3.5-4B with no decision tuning. Its per-family profile (inferred from strands/minicpm patterns):

| Family type | What it tests | Likely fork arm score | Root cause |
|---|---|---|---|
| fact / extraction | Single-step factual retrieval | High (0.8+) | Model knows facts |
| tool_selection / routing | Tool/API selection from descriptions | High (0.8+) | Exact match on known patterns |
| intent / paraphrase | Lexical overlap with training data | Moderate (0.6–0.8) | Partial alignment |
| multi-hop | Compositional reasoning across facts | Low (0.3–0.5) | Base model struggles with multi-step composition |
| judge_hard | Policy/norm evaluation requiring nuance | Low (0.3–0.5) | Requires world knowledge the base lacks |
| long_policy | Long-context policy reasoning | Low (0.2–0.4) | Context length and attention |
| temporal_numeric | Time-aware numerical reasoning | Low (0.1–0.3) | Numerical reasoning weakness |

**The fork arm's ceiling is not the readout.** The base Qwen3.5-4B has known weaknesses in multi-hop, temporal reasoning, and policy evaluation. A decision-tuned variant trained on the right data can improve on these families, but the base model's ceiling constrains how much.

**minicpm (0.7792)** and **strands (0.7706)** are both stronger than the fork arm. Examining their per-family profiles: minicpm is strong on `judge_hard` (0.765) and `trap` (0.875) but weak on `temporal_numeric` (0.133) and `tradeoff` (0.333). strands is strong on every easy family (1.00) but weak on `temporal_numeric` (0.267), `probability` (0.400), and `ambiguous` (0.429). These are **complementary failures** — a model that combines minicpm's judge_hard strength with strands's broad easy-family coverage could approach 0.85+.

---

## 5. The correct competitive strategy

Given the above analysis, the correct strategy is:

1. **Measure first** — Jebadiah-9B on JevBench-231 is the single most important data point.
2. **If Jebadiah-9B freeform ≥ 87%**: Use it as the deployment arm. Fine-tune a 4B model on freeform decision data to approach 88.70.
3. **If Jebadiah-9B freeform 85–87%**: Fine-tune a 4B model (Qwen3.5-4B or Gemma-4-E2B base) on decision data with freeform readout target. The goal is to close the 1–4 pp gap.
4. **If Jebadiah-9B freeform < 85%**: The problem is model quality. Consider a larger base model or a fundamentally different training approach (RL instead of SFT).
5. **Always**: Address the compositional families (multi-hop, judge_hard, long_policy) through training data composition, not through readout changes.

**The ladder is still the product.** The fusion v2 ladder (0.892 holdout on the 120-item suite) handles the majority of decisions at engine speed. The model rung fills the gaps the engine cannot solve.

---

## 6. Phase 0 — Measurement infrastructure (gate for everything else)

**Gate: Everything else waits on this. No training decision is valid without accurate measurements.**

### 0.1 Jebadiah-9B freeform on JevBench-231 (owner: autonomous, P0 — highest priority)

**What:** Run Jebadiah-9B-Q8_0 through the official JevBench harness with freeform readout.

**Deliverable:** `benchmarks/decision-model/results/jebadiah-9b-jevbench-231-freeform-YYYYMMDD.json`

**Method:**
1. Find community Jebadiah-9B-Q8_0 GGUF (lmstudio-community or equivalent)
2. Serve via llama.cpp with `--temp 0 --logprobs`
3. Render JevBench questions with options as natural text (not A/B/C/D)
4. Extract answer via fuzzy text match (the option text appears verbatim in the response)
5. Score against the public split using the authors' harness

**Acceptance:** A JevBench-231 accuracy number for Jebadiah-9B freeform. This is the most important single measurement in the plan.

### 0.2 All five missing benchmarks (Kev, OpenJev, Nimble, VitaminC, MASSIVE-en)

**What:** Integrate the five unmeasured benchmarks into the runner framework.

**Why:** We cannot claim to outperform JEV-27B on "all metrics" if we only measure one benchmark. The six-benchmark table must exist.

**Effort:** Engineering, not research. Each benchmark has a public harness and dataset. The work is format mapping and scoring plumbing.

**Deliverable:** `benchmarks/kev/`, `benchmarks/openjev/`, `benchmarks/nimble/`, `benchmarks/vitaminc/`, `benchmarks/massive/` — each with a runner that produces a score.

### 0.3 Fork arm freeform readout (owner: autonomous, P1)

**What:** Run our fork Qwen3.5-4B arm (currently at 0.7662 with tree readout) through JevBench-231 with freeform readout.

**This directly measures the freeform delta on our own arm**, separating the readout effect from the model quality effect.

**Acceptance:** A delta `(fork freeform) − (fork tree)` that quantifies how much the readout costs on our model.

### 0.4 Competitive baseline table (owner: autonomous, P1)

**What:** Produce the authoritative competitive table with all measured arms on all six benchmarks.

**Deliverable:** `benchmarks/results/competitive-baseline-YYYYMMDD.json`

---

## 7. Phase 1 — Freeform readout on our own arms

**Gate: Phase 0.3 complete. Fork freeform delta measured.**

### 1.1 Freeform readout on E1-E

**What:** E1-E at 0.684 with verdict-slot readout. Measure with freeform readout.

**Hypothesis:** The freeform delta for E1-E is smaller than for the fork arm, because E1-E's failures are in compositional families (multi-hop, judge_hard, long_policy) that freeform won't fix.

**If freeform E1-E > 0.70**: Freeform readout is a cheap win. Ship it.
**If freeform E1-E ≈ 0.68**: The gap is training data composition, not readout. Focus on Phase 2.

### 1.2 Freeform readout on strands and minicpm

**What:** strands (0.7706) and minicpm (0.7792) are the strongest arms we've measured. Neither has been tested with freeform readout.

**If freeform strands ≥ 0.80**: strands + freeform is the deployment arm. Fine-tune a 4B model to match it.
**If freeform minicpm ≥ 0.80**: Same.

### 1.3 Per-family delta analysis

**What:** For each arm tested in Phase 1, compute per-family accuracy with freeform readout.

**This is the diagnostic that tells us where freeform helps and where it doesn't.** Families where freeform improves accuracy are families where the constrained readout was the bottleneck. Families where freeform doesn't improve are families where the base model or training data is the bottleneck.

---

## 8. Phase 2 — Fine-tuning strategy corrections

v1's fine-tuning strategy was based on the phantom "Jebadiah-9B at 85.3% on JevBench-231" claim. With real measurements in hand, we correct the strategy.

### 8.1 Training data composition is the primary lever

E1-E's per-family failure profile (multi_hop 0.389, judge_hard 0.412, long_policy 0.368, temporal_numeric 0.133) is a **training data composition problem, not a size or readout problem.**

The merged-v3 corpus (173,452 rows, 25.2% non-relevance) and merged-v4 corpus (with quarantine pool) are the training data. The composition of these corpora on the hard families determines where E1-E can learn.

**Action:** Audit the training corpus by family. What fraction of multi-hop, judge_hard, and long_policy examples are in the corpus? If these families are underrepresented, the model cannot learn them regardless of size or readout.

**This is the VIVERE lane's purpose** (VIVERE = primary cheap knowledge-extraction/transfer method for the distill program). The VIVERE lane extracts decision-relevant knowledge from text and generates training items for the underrepresented families. This is the correct lever for the hard families.

### 8.2 Fine-tune on freeform interface, not letter-constrained

v1 correctly identified that the readout interface matters. The Jebadiah finding (0.142 letters vs 0.858 freeform on the 120-item suite) is real — it's just on the wrong benchmark. The principle transfers: **train on the format the model will be evaluated on.**

**Decision:** Fine-tuning uses freeform readout as the training target. Evaluation uses freeform readout.

### 8.3 Base model selection

**If Jebadiah-9B freeform ≥ 88.70 on JevBench-231**: Jebadiah-9B or a 4B distillation of it is the target. A 4B model trained on Jebadiah-quality freeform data should reach ≥ 87%.

**If Jebadiah-9B freeform 85–88%**: The base model matters. Qwen3.5-4B is the proven base (fork arm 0.7662 with no fine-tune). A decision-tuned Qwen3.5-4B with freeform readout should improve on the fork arm by 2–5 pp.

**If Jebadiah-9B freeform < 85%**: The base model is insufficient for this task. Consider:
- Gemma-4-E2B as base (strands uses it successfully)
- A larger base (Qwen3.5-8B or 14B)
- RL-based training instead of SFT

### 8.4 The mc5-jev GGUF failure is a warning

Our own GGUF conversion of MiniCPM5-Jev (0.6407) is 14 pp below the authors' path (0.7792). This means our GGUF conversion pipeline destroys adapter quality.

**If we train our own adapter and serve it via GGUF, we must validate that the GGUF conversion does not regress accuracy by more than 2 pp.** The imajev (0.7359) and decider (0.7576) GGUF paths did not have this problem because they are standalone models, not adapter-over-base.

**Decision:** Own-trained adapters should be served via llama.cpp `--lora` at inference time, not merged into the base GGUF. This preserves adapter fidelity (TRAINING.md §2 caveat about merge quality).

### 8.5 Per-kind calibration remains mandatory

Every trained arm gets per-kind temperature artifacts fitted via `fit_calibration.py`. The calibration seam is already in the architecture. The work is running it on each arm's benchmark runs.

**Target:** ECE ≤ 0.10 for all model arms. JEV-27B's ECE of 0.0009 (per-kind temperature) is the aspirational target.

---

## 9. Phase 3 — The OOD generalization problem

The jabr benchmark (866 cases, 49 tasks, CC0) is the OOD gate. Our engine scores 0.401. Von (best open) is 0.704. This is a fundamentally different problem from in-distribution accuracy.

### 9.1 Diagnose the OOD failure mode

**Before any training investment:** Run our best arms (fork, strands, minicpm, E1-E) on jabr. Compute per-task accuracy. Identify which task families we fail on and which task families Von succeeds on.

**The OOD gap has two possible root causes:**
1. **Coverage gap**: Our training data doesn't include jabr-style tasks. Adding them closes the gap.
2. **Compositional generalization failure**: Our training data includes similar tasks but the model can't generalize to new compositions. This requires a fundamentally different training approach (RL, not SFT).

**Kabra et al. found** that SFT on synthetic multi-hop data raises in-distribution accuracy but fails to transfer OOD, while RL transfers. This is the most important finding for the OOD problem.

### 9.2 If the gap is coverage

**Action:** Add jabr-style tasks to the training corpus. The VIVERE lane can generate decision items in the style of the tasks we fail on.

**Target:** ≥ 0.704 on jabr Board C.

### 9.3 If the gap is compositional generalization

**Action:** Consider RL-based training (PPO or GRPO) instead of SFT. This is more expensive but shown to transfer OOD where SFT fails.

**This is a research-level investment.** Pre-register the expected improvement before starting.

### 9.4 The NLI verifier as an OOD supplement

The NLI verifier probe scored 0.532 on jabr — halfway between our engine (0.401) and Von (0.704). An NLI-based entailment verification step could be added to the ladder as an OOD supplement: when the model rung is uncertain, an NLI entailment check over the candidate options provides a second opinion.

**This does not require retraining the model.** It is a ladder configuration change.

---

## 10. Governance and pre-registration

### Pre-registration discipline

Following the Jebadiah reproducibility contract (RESEARCH.md §6.5):

1. **Every training run registers its predictions before training begins.** State the expected JevBench-231 score, the arm, and the gate threshold before running.
2. **Every benchmark run is pre-registered.** State the expected score, the metric, the arm, and the gate threshold before running.
3. **No post-hoc selection.** The arm that scores highest on a benchmark is not the release candidate unless it was pre-registered as the candidate.

### Release gates

| Gate | Threshold | Measured by |
|---|---|---|
| G1: Determinism replay | Byte-identical decisions on identical replays | Δp < 0.001 |
| G2: Suite holdout | ≥ 0.68 on 120-item locked suite | OpenCodifier harness |
| G3: Board leg | ≥ JEV-27B score on each benchmark | JevBench-231 public split |
| G4: No regression | ≥ previous release arm score on all benchmarks | Delta table |

### Model manifest discipline

Every release artifact carries:
- SHA-256 of the GGUF file
- Calibration artifact (per-kind temperature)
- Benchmark scores (all six benchmarks)
- Training recipe manifest (corpus sha, rank, epochs, etc.)

---

## 11. Decision table

### Which model for which deployment

| Deployment | Model | Latency | Accuracy target | Cost/1k |
|---|---|---|---|---|
| Volume / low-stakes | Engine zero-ML | ≤ 5 ms | 0.683 (suite) / 0.401 (OOD) | $0.000007 |
| Medium-stakes | Decision-tuned 4B (freeform) | ≤ 500 ms | ≥ 85% JevBench | $0.0017 |
| High-stakes | JEV-27B-class arm | ≤ 1,000 ms | ≥ 88.70% JevBench | $0.0035 |
| OOD critical | 4B + NLI verifier supplement | ≤ 600 ms | ≥ 87% + jabr ≥ 0.70 | $0.0020 |

### Competitive gap by benchmark

| Benchmark | Our best (measured) | JEV-27B target | Gap | Strategy |
|---|---|---|---|---|
| JevBench-231 | 0.7706 (strands) | 88.70 | −11.7 pp | Fine-tune 4B on freeform decision data |
| Kev | not measured | ~85+ (estimated) | unknown | Measure first |
| OpenJev text | not measured | ~74+ (estimated) | unknown | Measure first |
| Nimble | not measured | ~92+ (estimated) | unknown | Measure first |
| VitaminC | not measured | ~78+ (estimated) | unknown | Measure first |
| MASSIVE-en | not measured | ~87+ (estimated) | unknown | Measure first |
| jabr Board C | 0.401 | 0.704 | −0.303 | VIVERE data + possible RL training |

---

## 12. Risks and contingencies

### Risk 1: Jebadiah-9B freeform is below 85% on JevBench-231

**Probability:** Medium. The 0.858 on the 120-item suite may not transfer.

**Mitigation:** If below 85%, the freeform readout is not the primary lever. The problem is model quality and training data. Reassess the fine-tuning strategy based on the per-family breakdown.

**Specific contingency:** If Jebadiah-9B freeform < 85%, the plan pivots to: (a) measure strands and minicpm freeform, (b) use the best of those as the deployment arm, (c) fine-tune a 4B model on decision data targeting the families where the best arm is weak.

### Risk 2: E1-E's failures are training data composition, not model size

**Probability:** High. The per-family profile (multi_hop 0.389, judge_hard 0.412) suggests the merged corpus lacks these family types.

**Mitigation:** Audit the corpus by family before any new training run. The VIVERE lane generates decision items for underrepresented families. This is the correct lever.

**Specific contingency:** If the corpus has near-zero multi-hop examples, the VIVERE lane must generate them before any fine-tuning can succeed on those families.

### Risk 3: GGUF conversion destroys our own adapter quality (like mc5-jev)

**Probability:** High. mc5-jev proves our conversion pipeline regresses adapter quality by 14 pp.

**Mitigation:** Serve own-trained adapters via llama.cpp `--lora` at inference time. Never merge into the base GGUF.

**Specific contingency:** If `--lora` serving is too slow, investigate why mc5-jev regressed while imajev (0.7359) and decider (0.7576) did not. The difference is that mc5-jev is an adapter-over-base while imajev/decider are standalone models.

### Risk 4: OOD generalization cannot be closed with fine-tuning

**Probability:** Medium-High. Kabra et al. showed SFT fails OOD transfer while RL succeeds.

**Mitigation:** Pre-register the expected improvement. If SFT doesn't close the gap after corpus enrichment, escalate to RL-based training.

### Risk 5: Quantization regresses our own-trained adapter accuracy

**Probability:** High (based on T88 finding in TRAINING.md: adapter quantization regressed −4.8 pp).

**Mitigation:** Validate quantization on each arm before release. If quantization regresses by more than 2 pp, try Q5_K_M instead of Q4_K_M, or deploy at higher precision.

### Risk 6: T5500 V100 not available (PSU replacement delayed)

**Probability:** Low-Medium. Already delayed.

**Mitigation:** All serving work can be done on fedora (5060 Ti) or CPU. The V100 is for throughput, not capability.

---

## 13. What this plan does NOT cover

This plan is specifically about **closing the JevBench-231 gap to JEV-27B** and **closing the jabr OOD gap**. It does not cover:

- **The Vision rung (Phase H)**: SigLIP2 embedder, image carriers, ladder integration. Phases I–J blocked on SigLIP2 ONNX weights.
- **The VIVERE distill program**: The training data composition work. This is tracked separately under Task #88.
- **Serving infrastructure**: The T5500 V100 plan for production throughput. This is tracked under the T5500 research record.
- **The MCP and CLI interfaces**: Already shipped, no changes planned.
- **The wasm and FFI interfaces**: Already shipped, no changes planned.

---

## Summary: The path to outperformance

```
Phase 0: Measure (1–2 weeks)
  ├── Measure Jebadiah-9B freeform on JevBench-231 ← THE GATE
  ├── Run fork arm with freeform readout (delta measurement)
  ├── Integrate five missing benchmarks
  └── Produce competitive baseline table
  ↓
Phase 1: Freeform readout on all arms (1 week)
  ├── Fork freeform delta confirmed
  ├── strands / minicpm freeform measured
  └── Per-family delta analysis
  ↓ [Based on Jebadiah-9B result]
Phase 2: Fine-tuning (2–6 weeks)
  ├── If Jebadiah ≥ 87%: deploy Jebadiah or fine-tune 4B on freeform data
  ├── If Jebadiah 85–87%: fine-tune 4B on freeform decision data
  ├── If Jebadiah < 85%: reassess — model quality is the bottleneck
  └── Per-kind calibration fitted for every arm
  ↓
Phase 3: OOD generalization (ongoing)
  ├── Diagnose jabr failure mode
  ├── If coverage gap: VIVERE lane generates missing task types
  ├── If compositional failure: consider RL-based training
  └── NLI verifier as OOD ladder supplement
```

**The single most important action is measuring Jebadiah-9B on JevBench-231.** Every decision in this plan depends on that number.

---

*Plan produced Session 76 v1, revised v2 2026-10-18. Sources: BENCHMARKS.md, DEEP_AUDIT.md, TRAINING.md, RESEARCH.md, competitive baseline (benchmarkheaven.com/jev-models, accessed 2026-10-07).*
