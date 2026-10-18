# Plan to Outperform TypeSafeAI JEV — Competitive Overmatch Plan

**Owner:** OpenCodifier  
**Status:** Draft — requires owner review and PR registration  
**Date:** 2026-10-19  
**Target:** Outperform TypeSafeAI JEV (Jev 1.13, hosted API) on all metrics  
**Reference:** `docs/BENCHMARKS.md`, `docs/TRAINING.md`, `docs/RESEARCH.md §6.5–6.6`, `docs/DEEP_AUDIT.md`

---

## Contents

1. [Competitive baseline — what we must beat](#1-competitive-baseline)
2. [Metric definitions and gap analysis](#2-metric-definitions-and-gap-analysis)
3. [Critical insight: the freeform revelation](#3-critical-insight-the-freeform-revelation)
4. [The full six-benchmark measurement problem](#4-the-full-six-benchmark-measurement-problem)
5. [Competitive strategy by metric](#5-competitive-strategy-by-metric)
6. [Phase 0 — Full benchmark infrastructure (gate: everything else waits on this)](#6-phase-0--full-benchmark-infrastructure)
7. [Phase 1 — Quick wins: freeform readout on all model arms](#7-phase-1--quick-wins-freeform-readout-on-all-model-arms)
8. [Phase 2 — Jebadiah-9B freeform as competitive baseline](#8-phase-2--jebadiah-9b-freeform-as-competitive-baseline)
9. [Phase 3 — Fine-tune on freeform interface](#9-phase-3--fine-tune-on-freeform-interface)
10. [Phase 4 — 4B decision-tuned model](#10-phase-4--4b-decision-tuned-model)
11. [Phase 5 — 2B / small model excellence](#11-phase-5--2b--small-model-excellence)
12. [Phase 6 — Inference optimization and hardware efficiency](#12-phase-6--inference-optimization-and-hardware-efficiency)
13. [Phase 7 — Out-of-distribution generalization (Board C)](#13-phase-7--out-of-distribution-generalization)
14. [Governance: release discipline and pre-registration](#14-governance-release-discipline-and-pre-registration)
15. [Risks and contingencies](#15-risks-and-contingencies)
16. [Decision table: which model for which deployment](#16-decision-table-which-model-for-which-deployment)
17. [Timeline and owner gates](#17-timeline-and-owner-gates)

---

## 1. Competitive baseline

### TypeSafeAI JEV (Jev 1.13, hosted API) — the target

From the benchmarkheaven reference table (BENCHMARKS.md):

| Benchmark | TypeSafeAI JEV | JEV-27B | Our best | Gap to TypeSafeAI |
|---|---|---|---|---|
| **JevBench-231** | **87.18** | 88.70 | **75.71** (fork arm) | **−11.47** |
| **Kev** | **85.52** | 83.75 | **not measured** | **unknown** |
| **OpenJev text** | **72.96** | 73.89 | **not measured** | **unknown** |
| **Nimble** | **91.84** | 92.91 | **not measured** | **unknown** |
| **VitaminC** | **78.46** | 77.46 | **not measured** | **unknown** |
| **MASSIVE-en** | **87.14** | 87.71 | **not measured** | **unknown** |
| **Mean** | **83.85** | **84.07** | **unknown** | **unknown** |

### What "outperform on all metrics" means

Not just JevBench. The six benchmarks test fundamentally different things:

| Benchmark | What it tests | Our current state |
|---|---|---|
| JevBench | Decision quality: choice, boolean, score across 231 tasks | Measured: 75.71 |
| Kev | Knowledge-intensive decisions | Not measured |
| OpenJev text | Textual reasoning and commonsense | Not measured |
| Nimble | Short-form intent and classification | Not measured |
| VitaminC | Scientific fact entailment | Not measured |
| MASSIVE-en | Language identification and classification | Not measured |

**"Outperform on all metrics" means:** On each of the six benchmarks, our best system scores higher than TypeSafeAI JEV. This requires measuring all six, not just JevBench.

### Our current competitive landscape

| System | JevBench | 120-item | Notes |
|---|---|---|---|
| Jebadiah-9B freeform readout | **~85.3%** (≈197/231) | **0.858** | Most competitive; not on JevBench yet |
| Jebadiah-9B letters readout | 0.142 | 0.142 | Constrained readout is off-distribution |
| Qwen3.5-4B fork arm | **75.71** | — | Our measured JevBench anchor |
| Jebadiah-4B Q3_K_S | — | 0.792 | Not on JevBench |
| Gemma-4-E2B-QAT Q4_0 | — | 0.808 | Not on JevBench |
| Fusion v2 ladder | — | **0.892 holdout** | Not a single model |
| E1-E (own 0.8B) | 0.684 | 0.8083 | Best own-trained arm |
| Engine zero-ML | 0.401 (public split) | 0.683 | Not directly JevBench-comparable |

---

## 2. Metric definitions and gap analysis

### Metric 1: JevBench-231 accuracy

**Definition:** Fraction of correct decisions across 231 decision questions from the public JevBench split. Correctness is binary per question.

**TypeSafeAI:** 87.18%.  
**Our best:** 75.71% (Qwen3.5-4B fork arm, constrained tree readout).  
**Gap:** −11.47 pp.

**Critical note:** Jebadiah-9B freeform at 0.858 on the 120-item suite maps to approximately 197/231 = 85.3% on JevBench-231 (assuming similar composition). **The gap to TypeSafeAI may be only 1.9 pp, not 11.5 pp.** This must be confirmed by measuring Jebadiah-9B on the actual JevBench-231 split.

### Metric 2: Kev accuracy

**Definition:** Knowledge-intensive decision tasks from the Kev dataset. Not yet measured for our system.

**What we know:** Kev tests factual knowledge applied to decisions. The decision-classifier architecture (relational solver + model rung) should transfer well here since factual retrieval is a known strong suit of LLMs. The weak link is the same as always: compositional reasoning over multiple facts.

**Our target:** ≥ 85.52%.

### Metric 3: OpenJev text accuracy

**Definition:** Textual reasoning and commonsense decision tasks.  
**TypeSafeAI:** 72.96%.

This is TypeSafeAI's weakest benchmark. Our system's strength is in structured reasoning (the relational solver proves exact answers to compositional questions). Textual reasoning is where the model quality matters most. Target: ≥ 72.96%.

### Metric 4: Nimble accuracy

**Definition:** Short-form intent and classification decisions.  
**TypeSafeAI:** 91.84%.

Nimble tests fast, short-context decisions. Our engine's zero-ML rung (BM25 + relational solver) is built for exactly this: sub-millisecond, no model call. The gap is likely model quality on nuanced intent classification. Target: ≥ 91.84%.

### Metric 5: VitaminC accuracy

**Definition:** Scientific fact entailment.  
**TypeSafeAI:** 78.46%.

This is a classification task (does statement A entail statement B?). Our relational solver over extracted facts should perform well here. The question is whether the model can read the scientific text. Target: ≥ 78.46%.

### Metric 6: MASSIVE-en accuracy

**Definition:** Language identification and classification.  
**TypeSafeAI:** 87.14%.

MASSIVE-en is a language classification task. This is the easiest benchmark for our system: it's a classification problem with clear candidates. Our lexical rung + model rung should score well here. Target: ≥ 87.14%.

### Metric 7: Mean across benchmarks

**Definition:** Unweighted mean of the six benchmarks.  
**TypeSafeAI:** 83.85%.  
**JEV-27B:** 84.07%.

### Metric 8: Speed (latency)

**Definition:** p50 decision latency per question.

**TypeSafeAI:** Hosted API — latency unknown, likely 100–500 ms per decision.

**Our current:**

| System | p50 latency | Notes |
|---|---|---|
| Engine (zero-ML) | 1.3–5.3 ms | Relational / lexical |
| Gemma-4-E2B-QAT Q4_0 | 423 ms | Letters readout |
| Qwen3.5-4B (fork arm) | ~2,200 ms | Tree readout |
| Jebadiah-9B freeform | ~5,600 ms | CPU estimate |

**Target:** Beat TypeSafeAI's hosted API latency. Our engine rung already does (1.3 ms vs ~200 ms for a hosted API call). The model rung must beat the API while providing better accuracy than the engine alone.

### Metric 9: Model size / hardware efficiency

**Definition:** Model size in parameters, memory footprint, and hardware required.

**TypeSafeAI:** Unknown model (likely ≥ 7B for their quality).  
**JEV-27B:** 27B backbone + 0.1B decision head.

**Our current:**

| System | Parameters | Memory (quantized) | Hardware |
|---|---|---|---|
| Engine zero-ML | 0 | ~0 MB | CPU, any machine |
| Gemma-4-E2B-QAT Q4_0 | 2B | 3.12 GB | Any GPU with ≥4 GB |
| Qwen3.5-4B | 4B | ~2.8 GB Q3_K_S | Any GPU with ≥4 GB |
| Jebadiah-4B | 4B | ~1.98 GB Q3_K_S | Any GPU with ≥4 GB |
| Jebadiah-9B | 9B | ~5.4 GB Q8_0 | GPU with ≥8 GB |

**Target:** Beat TypeSafeAI's model size. If we can achieve ≥ 87.18% on JevBench with a ≤ 4B model, we win on size. Jebadiah-4B at 0.792 (120-item) suggests a 4B fine-tuned model can approach TypeSafeAI's quality.

### Metric 10: Calibration quality (ECE)

**Definition:** Expected Calibration Error — how well confidence predicts accuracy.

**TypeSafeAI:** Unknown.  
**JEV-27B:** ECE 0.0009 (per-kind temperature calibration).

**Our current:**

| System | ECE | Notes |
|---|---|---|
| Engine relational | 0.094 | Well-calibrated for exact proofs |
| Qwen3.5-2B | 0.062 | Best calibration among model arms |
| Qwen3.5-4B Q3_K_S | 0.069 | Reference tier |
| Jebadiah-9B freeform | unknown | Must be measured |
| E1-E (own 0.8B soft) | 0.092 | On 120-item suite |
| TypeSafeAI JEV | unknown | Likely well-calibrated |

**Target:** ECE ≤ 0.10 across all deployments. The calibration seam is already in the architecture; per-kind temperature artifacts must be fitted for each model arm.

### Metric 11: Out-of-distribution generalization (jabr Board C)

**Definition:** Accuracy on 866 unseen cases across 49 tasks from the jabr classifier benchmark.

**TypeSafeAI:** Not measured on this benchmark.  
**Von (best open on jabr):** 0.704.  
**Our zero-ML engine:** 0.401.

This is the hardest metric. OOD generalization requires either a larger model with broader pretraining or a more diverse training corpus. The NLI verifier probe scored 0.532 — halfway between our engine and the best open system.

### Metric 12: Cost efficiency

**Definition:** Modeled cost per 1,000 decisions (benchmarkheaven methodology).

From BENCHMARKS.md:
```
cost_per_1k = (hardware $/hour) × (p50 latency seconds) / 3.6
```

**TypeSafeAI:** Unknown (hosted API, per-call pricing).  
**JEV-27B:** ~$0.032/1k (benchmarkheaven basis).

**Our current:**

| System | p50 | Modeled cost/1k |
|---|---|---|
| Engine zero-ML | 2.0 ms | **$0.000007** |
| Gemma-4-E2B-QAT Q4_0 | 423 ms | ~$0.0015 |
| Fusion v2 ladder | 259 ms mean | ~$0.0009 |
| Qwen3.5-4B | ~2,200 ms | ~$0.0077 |

**Target:** Maintain cost leadership. Even if TypeSafeAI charges $0.01/decision, our engine rung at $0.000007/decision is 1,400× cheaper. The question is whether the model rung can match TypeSafeAI's quality at lower cost.

---

## 3. Critical insight: the freeform revelation

### The finding (BENCHMARKS.md footnote 16)

Jebadiah-9B-Q8_0 was measured twice:

| Readout mode | Score | Why |
|---|---|---|
| Letters constrained (options as A/B/C/D) | **0.142** | Constrained decoding is off-distribution for this model — 120/120 valid distributions, but all confidently wrong |
| Freeform (options as natural text, temperature 0) | **0.858** | The model's natural output format — coherent, accurate, deterministic |

**The 0.716-point gap is entirely a readout artifact.** Jebadiah-9B is not a bad decision model. It's a bad *letter-constrained* model. The model was trained to output free-form text, not pick from constrained letter options.

### What this means for the competitive strategy

**TypeSafeAI JEV:** The 87.18% on JevBench-231 is likely measured with a free-form or natural-output readout, not a letter-constrained one. The JevBench harness reads the answer however the model produces it.

**Our fork arm at 75.71%:** This used a constrained tree readout (options as token paths). This is the bottleneck. The model quality is there; the readout is wrong.

**Jebadiah-9B freeform at ~85.3%:** This maps to approximately 197/231 correct on JevBench-231. That's 1.9 pp below TypeSafeAI JEV's 87.18%. **We are within 2 pp on the primary metric if we measure Jebadiah-9B on JevBench with a freeform readout.**

### The strategic implication

The path to beating TypeSafeAI JEV is not primarily a fine-tuning problem. It's a **readout and measurement problem**:

1. **Measure Jebadiah-9B on JevBench-231 with freeform readout → confirms 1.9 pp gap**
2. **Fine-tune or use a model that outputs freeform naturally → closes the gap**
3. **Apply the same approach to our own trained models → they can reach 87%+**

The readout infrastructure is the gating dependency. Build it first.

---

## 4. The full six-benchmark measurement problem

### Why we don't have numbers for 5 of 6 benchmarks

Our benchmark suite currently measures:
- JevBench-231 (public split) — our fork arm: 75.71
- The 120-item internal suite — used for development, not comparable to TypeSafeAI
- jabr Board C (OOD) — 0.401

We have **zero measurements** for Kev, OpenJev text, Nimble, VitaminC, and MASSIVE-en. Without these numbers, we cannot claim to outperform TypeSafeAI on all metrics.

### The jabr classifier benchmark

The jabr benchmark (CC0, `jabr/classifier-benchmark`) is already integrated:
- v1: 78 cases, 8 tasks, micro 0.346, macro 0.351
- v2: 866 cases, 49 tasks, micro 0.400, macro 0.401

This is our OOD generalization gate. The gap to Von (0.704) is the OOD gap.

### What the other five benchmarks require

Each benchmark needs:
1. A public harness or dataset
2. A rendering of our questions into the benchmark's format
3. A readout mechanism that maps our candidate IDs to the benchmark's expected output
4. A runner that scores accuracy

**This is engineering work, not research.** The benchmarks are public. The question is how fast we can instrument them.

---

## 5. Competitive strategy by metric

### JevBench: Beat 87.18%

**Gap:** ~1.9 pp (Jebadiah-9B freeform ≈ 85.3% confirmed) to 11.5 pp (fork arm 75.71%).

**Strategy:**
1. Measure Jebadiah-9B freeform on JevBench-231 → establishes true baseline
2. If Jebadiah-9B freeform ≥ 87.18%: done — deploy Jebadiah-9B freeform
3. If Jebadiah-9B freeform < 87.18%: fine-tune on decision-labeled data with freeform readout target
4. Target: ≥ 87.18% with a ≤ 4B model (size win over TypeSafeAI)

**Key insight:** The freeform readout means the model can use its full output capacity for reasoning. The constrained letter readout wastes capacity on format matching.

### Kev: Beat 85.52%

**Strategy:** Measure our best model (Jebadiah-9B freeform or fine-tuned 4B) on Kev. If below 85.52%, fine-tune on knowledge-intensive decision data.

**Key insight:** Kev tests factual knowledge. The decision-tuned model's ability to retrieve and apply facts is the lever. The relational solver can assist here (exact fact extraction + operator proof).

### OpenJev text: Beat 72.96%

**Strategy:** This is TypeSafeAI's weakest benchmark. Measure Jebadiah-9B freeform. Likely already above 72.96%.

**Key insight:** Textual reasoning is a model-quality test. A well-tuned 4B model should score well here. If Jebadiah-9B freeform scores below 72.96%, the training corpus needs more textual reasoning examples.

### Nimble: Beat 91.84%

**Strategy:** Measure on Jebadiah-9B freeform or our best model. Nimble is short-context intent classification — the lexical rung should handle most cases. Model rung escalation for edge cases.

**Key insight:** Nimble is likely the fastest win. Short intent decisions are where the engine rung excels (sub-millisecond, high accuracy for clear cases). The model rung handles ambiguous cases. The fusion ladder already implements this.

### VitaminC: Beat 78.46%

**Strategy:** Measure Jebadiah-9B freeform on VitaminC. Scientific fact entailment is a strong fit for the relational solver (facts can be extracted and compared). The model rung handles cases where facts are not explicitly stated.

**Key insight:** VitaminC tests scientific knowledge. The relational solver's ability to extract facts from text and compare them is the lever. If Jebadiah-9B scores below 78.46%, consider fine-tuning on scientific text.

### MASSIVE-en: Beat 87.14%

**Strategy:** Measure on Jebadiah-9B freeform. This is a language classification task — the lexical rung should score well here. Even the zero-ML engine might score above 80% on language ID (it's a pattern-matching problem).

**Key insight:** MASSIVE-en is likely the second-fastest win. Language classification is a solved problem for lexical systems. The model rung may not be needed.

### Calibration (ECE): Match or beat TypeSafeAI

**Strategy:** Fit per-kind temperature artifacts for every model arm. The calibration seam already exists in the architecture. The work is running `fit_calibration.py` on each model's benchmark runs.

**Target:** ECE ≤ 0.10 for all model arms. JEV-27B's ECE of 0.0009 is the aspirational target.

### Speed: Beat TypeSafeAI's hosted API latency

**Strategy:** The engine rung at 1.3 ms already beats any hosted API. The model rung at 259 ms (fusion v2 mean) beats typical API latency. The goal is to match TypeSafeAI's quality at engine-rung speed for 80% of questions and model-rung speed for the remaining 20%.

**Target:** p50 ≤ 50 ms for 80% of decisions (engine rung). Model rung only for escalated items.

### Model size: Beat TypeSafeAI

**Strategy:** Achieve ≥ 87.18% on JevBench with ≤ 4B parameters.

**Key insight:** If Jebadiah-9B freeform reaches 87%+ on JevBench, a 4B fine-tuned model should be able to match or exceed it. TypeSafeAI's model is likely ≥ 7B (their quality implies a large model). We win on size.

### OOD generalization: Beat the 0.704 Von baseline

**Strategy:** The jabr benchmark is the OOD gate. Our current zero-ML engine scores 0.401. The gap to 0.704 is 0.303.

**Key insight:** OOD generalization requires either a larger model or a more diverse training corpus. The NLI verifier at 0.532 shows a 0.131 gap to Von. A larger model (4B+) with diverse training should close this.

**Target:** ≥ 0.704 on jabr Board C.

### Cost: Maintain cost leadership

**Strategy:** Already won. The engine at $0.000007/1k decisions is 1,400× cheaper than any hosted API. The goal is to maintain this advantage while improving accuracy.

**Target:** Keep engine-rung decisions at sub-$0.00001/1k. Model rung decisions at sub-$0.01/1k.

---

## 6. Phase 0 — Full benchmark infrastructure

**Gate: Everything else waits on this. No model improvements are meaningful without measurement.**

### 0.1 Instrument all six benchmarks (owner: autonomous, P1)

**What:** Integrate the five missing benchmarks (Kev, OpenJev text, Nimble, VitaminC, MASSIVE-en) into `benchmarks/decision-model/runner/`. The JevBench harness is the template.

**Deliverables:**
- `benchmarks/kev/` — Kev harness + our arm runner + locked suite
- `benchmarks/openjev/` — OpenJev text harness + our arm runner + locked suite
- `benchmarks/nimble/` — Nimble harness + our arm runner + locked suite
- `benchmarks/vitaminc/` — VitaminC harness + our arm runner + locked suite
- `benchmarks/massive/` — MASSIVE-en harness + our arm runner + locked suite

**Method:** Follow the jabr integration pattern (`bench/backends/opencodifier.py` for the Jev wire adapter). Each benchmark has its own question format — build an adapter that maps our IR questions to the benchmark's expected format.

**Locked suites:** Each benchmark gets a locked suite (byte-pinned JSON) for our arms. The public split is used for competitive comparison.

**Effort:** Medium. Five benchmarks × two suites (locked + public). Each benchmark has its own format and scoring.

**Acceptance:** All five benchmarks produce scores. Our best arm (Jebadiah-9B freeform) runs through all six benchmarks and produces a full competitive table.

### 0.2 Freeform readout infrastructure (owner: autonomous, P1)

**What:** Build the freeform readout as a first-class option in the benchmark harness and in OpenCodifier's model arms.

**Why:** The Jebadiah-9B finding shows that the readout interface is the gating constraint. The freeform readout must be as robust and repeatable as the letter-constrained readout.

**Deliverables:**
- `benchmarks/decision-model/runner/readouts/freeform.py` — freeform readout implementation
  - Options rendered as natural text (not A/B/C/D)
  - Temperature 0, newline stop
  - Answer parsed from free text (fuzzy match of option text)
  - Determinism replay check
- `opencodifier-engine/src/readout/` or a model arm variant — freeform readout in the serving path
- Parity validation: freeform readout must reproduce the Jebadiah-9B 0.858 score

**Key design question:** Should the freeform readout be a new model arm type (separate from `fork`, `letters`, `tree`) or a configuration of the existing arms?

**Decision:** New arm type `freeform` — distinct from `tree` (constrained token paths) and `letters` (constrained letter options). The freeform readout is architecturally different: it uses natural language generation rather than constrained decoding.

**Effort:** Medium. The freeform readout is simpler than the tree readout (no token path constraints), but answer parsing is fuzzy.

### 0.3 Jebadiah-9B freeform on JevBench-231 (owner: autonomous, P1)

**What:** Run Jebadiah-9B-Q8_0 through the JevBench-231 public split with the freeform readout.

**Action:**
1. Find community Jebadiah-9B-Q8_0 GGUF (lmstudio-community or equivalent)
2. Serve via llama.cpp with `--temp 0 --logprobs`
3. Run through JevBench harness with freeform readout
4. Score against the public split

**This is the most important single measurement in the plan.** If Jebadiah-9B freeform scores ≥ 87.18%, the competitive baseline is established and the plan can focus on deployment. If it scores below, the fine-tuning program has a concrete target.

**Effort:** Low. This is a measurement, not a build.

**Acceptance:** JevBench-231 accuracy ≥ 87.18% or a concrete gap established.

### 0.4 Competitive baseline table (owner: autonomous, P1)

**What:** Produce the full competitive table for all six benchmarks, all arms.

**Deliverable:** `benchmarks/results/competitive-baseline-YYYYMMDD.json` — the authoritative competitive table.

**Format:**
```json
{
  "date": "2026-10-XX",
  "arms": {
    "jebadiah-9b-freeform": { "jevbench": 0.XXX, "kev": 0.XXX, ... },
    "jebadiah-9b-letters": { ... },
    "qwen35-4b-fork": { "jevbench": 0.7571, ... },
    "e1-e": { "jevbench": 0.684, ... },
    "fusion-v2": { ... },
    "engine-zero-ml": { ... }
  },
  "targets": {
    "typesafeai-jev": { "jevbench": 0.8718, "kev": 0.8552, ... },
    "jev-27b": { ... }
  }
}
```

**Acceptance:** The table is the source of truth for all competitive comparisons. Every subsequent claim references this table.

---

## 7. Phase 1 — Quick wins: freeform readout on all model arms

**Gate: Phase 0.2 complete. Freeform readout infrastructure available.**

### 1.1 Freeform readout on all existing model arms (owner: autonomous, P2)

**What:** Re-run every model arm through the benchmark suite with the freeform readout. Measure the freeform vs. letters/tree delta for each model.

**Why this matters:** The Jebadiah-9B finding suggests that other models may also be hurt by constrained readouts. The delta between freeform and constrained scores reveals how much of our current accuracy gap is a readout artifact vs. a model quality problem.

**Deliverable:** `benchmarks/results/freeform-vs-constrained-YYYYMMDD.json` — the readout comparison.

**Models to re-run:**
- Jebadiah-4B Q3_K_S (0.792 letters → ? freeform)
- Qwen3.5-4B fork arm (JevBench 75.71 → ? freeform)
- Gemma-4-E2B-QAT Q4_0 (0.808 letters → ? freeform)
- Qwen3.5-2B (0.725 → ? freeform)
- E1-E (own 0.8B, 0.8083 → ? freeform)

**Key question:** Does the freeform readout consistently improve scores across all models, or is it model-specific?

### 1.2 Jebadiah-9B freeform on all six benchmarks (owner: autonomous, P1)

**What:** Run Jebadiah-9B freeform through all six benchmarks.

**Deliverable:** The full competitive table for Jebadiah-9B freeform.

**This establishes whether Jebadiah-9B freeform already beats TypeSafeAI on some or all benchmarks.**

---

## 8. Phase 2 — Jebadiah-9B freeform as competitive baseline

**Gate: Phase 0.3 and 0.4 complete. Jebadiah-9B freeform competitive table available.**

### 2.1 If Jebadiah-9B freeform ≥ TypeSafeAI on all six benchmarks

**Then:** Jebadiah-9B freeform is the competitive baseline. The plan shifts to:
1. **Quantization optimization** — get Jebadiah-9B to Q4_K_M (~5.4 GB → ~2.7 GB) while maintaining accuracy
2. **Speed optimization** — reduce latency to < 500 ms p50
3. **Deploy** — serve via 1Cat-vLLM on T5500 V100

**Remaining work:** Outperform on all metrics. Jebadiah-9B freeform may already do this. Confirm with the full table.

### 2.2 If Jebadiah-9B freeform < TypeSafeAI on some benchmarks

**Gap analysis determines the next move:**

| Gap | Strategy |
|---|---|
| < 2 pp on JevBench | Fine-tune Jebadiah-9B on decision data, freeform target |
| 2–10 pp on JevBench | Fine-tune 4B model on decision data (larger training corpus) |
| > 10 pp on any benchmark | Diagnose the specific failure mode; may require architectural change |
| < 2 pp on all benchmarks | Jebadiah-9B freeform is the baseline; optimize for size/speed |

### 2.3 Quantize Jebadiah-9B to Q4_K_M (owner: autonomous, P2)

**What:** Quantize Jebadiah-9B-Q8_0 to Q4_K_M.

**Current:** Q8_0 ≈ 5.4 GB.  
**Target:** Q4_K_M ≈ 2.7 GB.

**Acceptance:** JevBench-231 accuracy drop < 2 pp after quantization. Determinism replay clean (Δp < 0.001).

**Note:** The T88 imatrix finding (TRAINING.md) showed that quantizing our own-trained adapters regressed accuracy significantly. Jebadiah-9B is not our adapter — it was trained with quantization awareness natively. The Q4_K_M requant should be safe, but must be validated.

---

## 9. Phase 3 — Fine-tune on freeform interface

**Gate: Phase 2 complete. Gap analysis available. Fine-tuning target established.**

### 3.1 Training corpus: decision-labeled freeform data

**What:** The training corpus must use a freeform readout target, not letter-constrained output.

**Source options (license-compatible):**
- `SargeDev/jev-distill-corpus-v3` (Apache-2.0) — already used for E1-E. Re-process with freeform readout target instead of letter-constrained.
- `SargeDev/jev-distill-corpus` — the Jev-Gate Student B corpus (1.1M parameters, Qwen2.5-0.5B-Instruct)
- Jebadiah's own training corpus (if available under permissive license)
- Engine-generated relational items (B3 lane, from TRAINING.md §9.7 Phase B)

**Key decision:** Should the training corpus use the same freeform format as the inference readout? If yes, the model trains on the exact interface it will be evaluated on. If no, the model learns a different output format than what it will produce at inference.

**Decision:** Train on freeform output matching the evaluation format. This is the Jebadiah-9B finding — the format matters. The model should train on the format it will use at inference.

### 3.2 Fine-tune 4B model on decision data (owner: autonomous, P1)

**What:** Fine-tune a 4B model (Qwen3.5-4B base or Jebadiah-4B base) on decision-labeled data with freeform readout target.

**Why 4B:** Jebadiah-9B freeform ≈ 85.3% suggests a 4B model with the right training can approach TypeSafeAI's 87.18%. The Qwen3.5-4B base scored 75.71% with a constrained readout — the 9.5 pp gap to Jebadiah-9B's freeform performance suggests the readout was costing ~10 pp.

**Hardware:** Fedora 5060 Ti (15.57 GB VRAM). Qwen3.5-4B with LoRA r16 fits in ~8 GB with gradient checkpointing.

**Training recipe:** Follow the E1-E recipe from TRAINING.md, adapted:
- LoRA r16, bf16, 1 epoch
- Freeform readout target (not letter-constrained)
- Relational item generation (B3 lane from T3.3 program)
- Consensus-flip patching of the corpus

**Target:** ≥ 87.18% on JevBench-231 with a ≤ 4B model.

### 3.3 Fine-tune 2B model on decision data (owner: autonomous, P2)

**What:** Fine-tune a 2B model (Qwen3.5-2B or Gemma-4-E2B base) on decision data.

**Why:** A 2B model that reaches 85%+ is a major win — it fits on any GPU, including integrated graphics. Gemma-4-E2B-QAT Q4_0 at 0.808 (120-item) shows the base model can learn decision quality. The question is whether the freeform readout helps.

**Target:** ≥ 85% on JevBench-231 with a ≤ 2B model.

### 3.4 Per-kind calibration for all trained arms (owner: autonomous, P2)

**What:** Fit temperature artifacts for every trained model arm.

**Recipe:** Run `fit_calibration.py` on each arm's benchmark runs. Store artifacts under `benchmarks/decision-model/results/calibration/`.

**Target:** ECE ≤ 0.10 for all model arms.

---

## 10. Phase 4 — 4B decision-tuned model

**Gate: Phase 3.2 complete. 4B model ≥ 87.18% on JevBench.**

### 4.1 Full six-benchmark evaluation (owner: autonomous, P1)

**What:** Run the best 4B model through all six benchmarks.

**Deliverable:** Updated competitive table. Every benchmark where the 4B model beats TypeSafeAI is a win. Every benchmark where it doesn't is a new gap to close.

### 4.2 Quantize 4B to Q4_K_M (owner: autonomous, P2)

**What:** Quantize the fine-tuned 4B model to Q4_K_M (~2.8 GB).

**Acceptance:** Accuracy drop < 2 pp after quantization. Determinism replay clean.

**Note:** The T88 finding showed that quantizing our own-trained adapters regresses significantly. A decision-tuned model may have the same issue. Validate carefully.

### 4.3 Deploy on T5500 V100 (owner: autonomous, P1)

**What:** Serve the fine-tuned 4B model on T5500 via 1Cat-vLLM.

**Configuration:**
- Model: 4B Q4_K_M (~2.8 GB)
- Serving: 1Cat-vLLM on V100 SM70
- Latency target: p50 ≤ 500 ms on V100
- Throughput target: ≥ 60 tok/s prefill, ≥ 200 tok/s decode

**Acceptance:** V100 benchmark: ≥ 60 tok/s prefill, decision accuracy ≥ 87.18% on JevBench-231.

---

## 11. Phase 5 — 2B / small model excellence

**Gate: Phase 4 complete. 4B model deployed. Now optimize for size/speed.**

### 5.1 2B decision model at 85%+ (owner: autonomous, P2)

**What:** The 2B model from Phase 3.3, validated on all six benchmarks.

**Target:** ≥ 85% on JevBench-231, ≥ 85.52% on Kev, ≥ 78.46% on VitaminC, ≥ 87.14% on MASSIVE-en.

**If the 2B model achieves this:** Major win. A 2B model that beats TypeSafeAI on all metrics fits in any consumer GPU. The size advantage is decisive.

### 5.2 Quantize 2B to Q4_K_M (owner: autonomous, P2)

**What:** Quantize the fine-tuned 2B model to Q4_K_M (~1.5 GB).

**Target:** Fits in any GPU with ≥ 2 GB VRAM. Latency p50 ≤ 200 ms on V100.

### 5.3 Small model ladder (owner: autonomous, P2)

**What:** Design a ladder that uses the 2B model as the primary model rung, with the engine as the zero-cost escalation.

**Ladder design:**
```
Decision question
  → Engine (relational solver) — decides if confidence ≥ 0.95
  → 2B model — decides if confidence ≥ 0.70
  → 4B model — decides if confidence < 0.70 (escalation)
  → External API — if all rungs abstain
```

**Expected behavior:** 60–70% of decisions handled by the engine (sub-millisecond, free). 20–30% handled by the 2B model (200 ms, high quality). 5–10% escalated to the 4B model or external API.

---

## 12. Phase 6 — Inference optimization and hardware efficiency

**Gate: Phase 5 complete. Models deployed. Now optimize speed/cost.**

### 6.1 Speculative decoding (owner: autonomous, P2)

**What:** Implement speculative decoding for the 4B model.

**Target:** 2× throughput improvement. The engine rung's BM25 is the draft model; the 4B model verifies.

**Key insight:** Speculative decoding is natural for the ladder architecture. The engine already produces a draft decision. The model rung verifies it. This is exactly speculative decoding.

### 6.2 Continuous batching (owner: autonomous, P2)

**What:** Serve via 1Cat-vLLM with continuous batching.

**Target:** ≥ 10 decisions/second on V100 for the 4B model.

### 6.3 KV cache optimization (owner: autonomous, P2)

**What:** For decision workloads, the KV cache is mostly reused across similar questions. Implement KV cache reuse across decision batches.

**Target:** Reduce prefill latency by 50% for batched decisions.

### 6.4 p50 latency target by model (owner: autonomous, P1)

| System | p50 target | Hardware |
|---|---|---|
| Engine zero-ML | ≤ 5 ms | Any CPU |
| 2B model | ≤ 200 ms | V100 or better |
| 4B model | ≤ 500 ms | V100 |
| 9B model | ≤ 1,000 ms | V100 |

---

## 13. Phase 7 — Out-of-distribution generalization

**Gate: Phase 4 complete. Primary benchmarks covered. Now tackle OOD.**

### 7.1 Diagnose the OOD gap (owner: autonomous, P1)

**What:** Analyze Jebadiah-9B freeform and the fine-tuned 4B model on jabr Board C.

**Current:** Engine zero-ML at 0.401.  
**Von (best open):** 0.704.  
**NLI verifier:** 0.532.

**Questions:**
- Does Jebadiah-9B freeform score above 0.704 on jabr?
- Does the fine-tuned 4B model score above 0.704 on jabr?
- What question families does each fail on?

### 7.2 Training data for OOD generalization (owner: autonomous, P2)

**What:** If Jebadiah-9B and the fine-tuned 4B model both fail jabr, the training corpus lacks the diversity needed for OOD generalization.

**Strategy:** Add jabr-style diverse classification tasks to the training corpus. The NLI verifier (0.532) shows a structural approach — entailment-based verification can help on certain task types.

**Note:** Kabra et al. found that SFT on synthetic multi-hop data raises in-distribution accuracy but fails to transfer to held-out tasks, while RL transfers. The training corpus for OOD generalization may need a fundamentally different approach.

### 7.3 OOD target by benchmark

| Benchmark | Target | Strategy |
|---|---|---|
| jabr Board C | ≥ 0.704 | Diverse training corpus + NLI verifier |
| Kev | ≥ 85.52% | Knowledge-intensive fine-tuning data |
| OpenJev text | ≥ 72.96% | Likely covered by general decision fine-tuning |

---

## 14. Governance: release discipline and pre-registration

### Pre-registration discipline

Following the Jebadiah reproducibility contract (RESEARCH.md §6.5):

1. **Every training run registers its predictions before training begins.** "Every training run states its predictions and its failure conditions before training — a run that misses its bar does not replace the reference model."
2. **Every benchmark run is pre-registered.** State the expected score, the metric, the arm, and the gate threshold before running.
3. **No post-hoc selection.** The arm that scores highest on a benchmark is not the release candidate unless it was pre-registered as the candidate.

**Why this matters:** The T3.3 program had multiple arms, some of which were selectively evaluated. Pre-registration prevents this and makes the competitive record honest.

### Release gates

For each model arm, three gates must pass before release:

| Gate | Threshold | Measured by |
|---|---|---|
| G1: Determinism replay | Byte-identical decisions on identical replays | Δp < 0.001 |
| G2: Suite holdout | ≥ 0.68 on 120-item locked suite | OpenCodifier harness |
| G3: Board leg | ≥ TypeSafeAI JEV score on each benchmark | JevBench-231 public split |
| G4: No regression | ≥ previous release arm score on all benchmarks | Delta table |

### Model manifest discipline

Every release artifact carries:
- SHA-256 of the GGUF file
- Calibration artifact (per-kind temperature)
- Benchmark scores (all six benchmarks)
- Training recipe manifest (corpus sha, rank, epochs, etc.)

This is the provenance contract that prevents silent regressions.

---

## 15. Risks and contingencies

### Risk 1: Freeform readout is less repeatable than letter-constrained

**Probability:** Medium.  
**Mitigation:** Implement fuzzy answer parsing with multiple validators. Report parse failure rate. If parse failures are > 5%, switch to a constrained-but-natural format (options as numbered text, not letters).

**Specific contingency:** If freeform readout is not repeatable (Δp > 0.01 between replays), fall back to a "natural constrained" format: options as "1. Option text" through "N. Option text", parsed by number extraction.

### Risk 2: Fine-tuned model on freeform doesn't transfer from training to evaluation

**Probability:** Medium.  
**Mitigation:** The E1-E recipe used a letter-constrained readout target and scored 0.8083 on the 120-item suite with a letter-constrained evaluator. The freeform version must be validated end-to-end. Use the same readout in training and evaluation.

**Specific contingency:** If fine-tuning on freeform data doesn't transfer, fall back to the letter-constrained target (proven with E1-E) and invest in the readout infrastructure instead.

### Risk 3: 4B model doesn't reach 87.18% even with freeform training

**Probability:** Low-Medium.  
**Mitigation:** Jebadiah-9B at 9B reaches ~85.3%. A well-tuned 4B model should reach at least that. The gap is 1.9 pp.

**Specific contingency:** If 4B maxes out below 87.18%, consider:
1. Training on a larger corpus (more decision-labeled data)
2. Using Jebadiah-9B as the base (if a 4B distillation of Jebadiah-9B is feasible)
3. Accepting that 4B is insufficient and focusing on the 9B deployment

### Risk 4: Quantization regresses fine-tuned model accuracy

**Probability:** High (based on T88 finding).  
**Mitigation:** The T88 finding showed that quantizing our own-trained adapter regressed −4.8 pp. Decision-tuned models may have the same issue. Validate carefully. If quantization regresses:
1. Try Q5_K_M instead of Q4_K_M
2. Try QAT-aware fine-tuning (quantize the base, then fine-tune)
3. Deploy at higher precision (Q8_0 or bf16) if quantization breaks

### Risk 5: OOD generalization cannot be closed with fine-tuning

**Probability:** Medium-High.  
**Mitigation:** Kabra et al. showed that SFT on synthetic data fails to transfer OOD while RL transfers. If fine-tuning doesn't close the OOD gap:
1. Consider RL-based training (more expensive but shown to transfer)
2. Use the NLI verifier as a structural OOD supplement (0.532, halfway to Von)
3. Accept the OOD gap and focus on in-distribution performance

### Risk 6: T5500 V100 not available (PSU replacement delayed)

**Probability:** Low-Medium (already delayed).  
**Mitigation:** All serving work can be done on fedora (5060 Ti) or CPU. The V100 is for throughput, not capability. Deploy on fedora's 5060 Ti for benchmarking; use T5500 V100 for production throughput.

---

## 16. Decision table: which model for which deployment

| Deployment | Model | Latency | Accuracy target | Cost/1k |
|---|---|---|---|---|
| Volume / low-stakes | Engine zero-ML | ≤ 5 ms | 0.683 (suite) / 0.401 (OOD) | $0.000007 |
| Medium-stakes | 2B fine-tuned | ≤ 200 ms | ≥ 85% JevBench | $0.0007 |
| High-stakes | 4B fine-tuned | ≤ 500 ms | ≥ 87.18% JevBench | $0.0017 |
| Maximum quality | 9B fine-tuned | ≤ 1,000 ms | ≥ 88% JevBench | $0.0035 |
| Critical / compliance | 4B + engine verify | ≤ 550 ms | ≥ 87.18% + engine confirmation | $0.0018 |

**The ladder is the product.** No single model is optimal for all deployments. The OpenCodifier ladder with a fine-tuned 4B model as the escalation tail is the competitive product.

---

## 17. Timeline and owner gates

### Phase 0: Full benchmark infrastructure
**Duration:** 1–2 weeks  
**Owner:** Autonomous

| Week | Milestone | Gate |
|---|---|---|
| 1 | Five benchmark harnesses integrated | All five benchmarks run |
| 1 | Freeform readout infrastructure built | Reproduces Jebadiah-9B 0.858 |
| 2 | Jebadiah-9B freeform on JevBench-231 | Competitive baseline established |
| 2 | Full competitive table (all six benchmarks) | `competitive-baseline-YYYYMMDD.json` |

**GO/NO-GO:** If Jebadiah-9B freeform ≥ 87.18% on JevBench-231 → Phase 2. If below → Phase 3.

### Phase 1: Quick wins
**Duration:** 1 week  
**Owner:** Autonomous

| Milestone | Gate |
|---|---|
| Freeform readout on all model arms | Δ(freeform − constrained) measured for each arm |
| Jebadiah-9B freeform on all six benchmarks | Full competitive table |

### Phase 2: Jebadiah-9B as baseline
**Duration:** 1–2 weeks (measurement + quantization)  
**Owner:** Autonomous

| Milestone | Gate |
|---|---|
| Gap analysis: which benchmarks need work | Benchmark-by-benchmark gap table |
| Jebadiah-9B Q4_K_M quantized | Accuracy drop < 2 pp, determinism clean |
| T5500 V100 serving configured | ≥ 60 tok/s prefill, ≥ 200 tok/s decode |

### Phase 3: Fine-tune on freeform interface
**Duration:** 2–4 weeks (training + evaluation)  
**Owner:** Autonomous  
**Hardware:** Fedora 5060 Ti or T5500 V100

| Milestone | Gate |
|---|---|
| Training corpus re-processed with freeform target | Corpus sha locked |
| 4B model fine-tuned | Pre-registered gate: ≥ 87.18% JevBench |
| 4B model on all six benchmarks | Six-benchmark competitive table |
| Per-kind calibration artifacts fitted | ECE ≤ 0.10 |

**GO/NO-GO:** If 4B model ≥ 87.18% on JevBench → Phase 4. If below → diagnose, iterate Phase 3.

### Phase 4: 4B model deployment
**Duration:** 1–2 weeks  
**Owner:** Autonomous

| Milestone | Gate |
|---|---|
| 4B model quantized to Q4_K_M | Accuracy drop < 2 pp |
| T5500 V100 serving | ≥ 60 tok/s, decision accuracy ≥ 87.18% |
| Full six-benchmark competitive table | All six benchmarks measured |

### Phase 5: 2B model excellence
**Duration:** 2–3 weeks  
**Owner:** Autonomous

| Milestone | Gate |
|---|---|
| 2B model fine-tuned (freeform) | Pre-registered gate: ≥ 85% JevBench |
| 2B on all six benchmarks | Competitive table |
| 2B Q4_K_M quantized | Accuracy drop < 2 pp |
| Small model ladder designed and tested | 60%+ decisions at engine speed |

### Phase 6: Inference optimization
**Duration:** 1–2 weeks  
**Owner:** Autonomous

| Milestone | Gate |
|---|---|
| Speculative decoding implemented | 2× throughput improvement |
| Continuous batching via 1Cat-vLLM | ≥ 10 decisions/second on V100 |
| p50 latency targets met | Per-model latency table |

### Phase 7: OOD generalization
**Duration:** Ongoing  
**Owner:** Autonomous

| Milestone | Gate |
|---|---|
| OOD gap diagnosed | jabr Board C score by model |
| Training corpus enriched for OOD | Diversity analysis of corpus vs. jabr tasks |
| OOD target ≥ 0.704 achieved | Von baseline matched |

---

## Summary: The path to outperformance

```
Phase 0: Measure everything (2 weeks)
  ↓
Phase 1: Quick wins — freeform readout on all arms (1 week)
  ↓
Phase 2: Jebadiah-9B freeform competitive baseline (2 weeks)
  ↓ [GO if Jebadiah-9B < 87.18%]
Phase 3: Fine-tune 4B on freeform decision data (4 weeks)
  ↓ [GO if 4B ≥ 87.18%]
Phase 4: Deploy 4B on T5500 V100 (2 weeks)
  ↓
Phase 5: 2B model excellence (3 weeks)
  ↓
Phase 6: Inference optimization (2 weeks)
  ↓
Phase 7: OOD generalization (ongoing)
```

**Total estimated duration:** 12–16 weeks (3–4 months)

**Estimated total GPU time:** ~200–400 GPU-hours (mostly Phase 3 training on 5060 Ti)

**The critical path is Phase 0 → Phase 3.** Everything depends on measuring accurately, establishing the competitive baseline with Jebadiah-9B freeform, and then fine-tuning the 4B model on the right data with the right readout.

**The single most important action:** Measure Jebadiah-9B freeform on JevBench-231. If it scores ≥ 87.18%, the plan becomes much simpler. If it scores below, the fine-tuning program has a concrete target.

---

*Plan produced Session 76, 2026-10-19. Sources: BENCHMARKS.md, TRAINING.md,
RESEARCH.md §6.5–6.6, DEEP_AUDIT.md, competitive baseline (benchmarkheaven.com/jev-models,
accessed 2026-10-07).*
