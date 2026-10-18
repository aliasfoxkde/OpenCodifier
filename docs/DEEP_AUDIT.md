# OpenCodifier System Audit — Session 76 (Corrected)
**Date:** 2026-10-19  
**Auditor:** pi coding agent  
**Scope:** Architecture · Decision Rungs · Engine · Quantization · Fine-tuning · Documentation · Roadmap  
**Sources:** Full codebase · BENCHMARKS.md · PLAN.md · TRAINING.md · DECISIONS.md  
**Correction:** An earlier version of this document compared JevBench-231 (231 items) against the
120-item harness suite. Those benchmarks are not comparable. This version corrects that and
re-scopes the audit to the actual state of the system.

---

## Contents

1. [What the benchmarks actually say](#1-what-the-benchmarks-actually-say)
2. [Where the system actually stands today](#2-where-the-system-actually-stands-today)
3. [Fine-tuning: what was tried and what the results mean](#3-fine-tuning-what-was-tried-and-what-the-results-mean)
4. [Training corpus and data program findings](#4-training-corpus-and-data-program-findings)
5. [NLI verifier probe result](#5-nli-verifier-probe-result)
6. [Quantization findings](#6-quantization-findings)
7. [Engine architecture audit](#7-engine-architecture-audit)
8. [Vision integration (Phase H)](#8-vision-integration-phase-h)
9. [Composite deployment score — the real question](#9-composite-deployment-score)
10. [What the T5500 V100 plan actually addresses](#10-what-the-t5500-v100-plan-actually-addresses)
11. [Honest remaining gaps](#11-honest-remaining-gaps)

---

## 1. What the benchmarks actually say

### The JevBench comparison is not apples-to-apples

My earlier version of this document claimed Strands Decider Gemma 4 E4B at JevBench 187/231 (80.9%)
was "25–40 pp more accurate" than OpenCodifier's 0.767 on the 120-item harness suite. This is
wrong. JevBench-231 and the 120-item locked harness suite are different question sets with
different compositions. You cannot compare them directly.

The correct comparison is against the same benchmark:

| System | JevBench-231 (public split) | Source |
|---|---|---|
| JEV-27B (autotrust) | **88.70** | benchmarkheaven, 2026-09 |
| **Our fork arm** (Qwen3.5-4B, constrained tree readout) | **75.71** | BENCHMARKS.md F24/fedora anchor |
| Strands Decider E4B Gemma 4 (JevBench 187/231) | **~80.9%** (187/231) | RESEARCH.md §21 |
| Zero-ML engine (JevBench public split) | **40.11** | BENCHMARKS.md Board A |
| Jebadiah 9B freeform readout | **85.8%** | BENCHMARKS.md footnote 16 |

The honest gap to the target bar is **13 pp** (75.71 → 88.70). Strands Decider E4B Gemma 4
at ~80.9% sits about 5 pp above our fork arm on the same benchmark.

### The 120-item suite tells a different story

| System | 120-item suite | Source |
|---|---|---|
| **Fusion v2 ladder** (lexical + relational + Gemma-4-E2B-QAT Q4_0) | **0.933** (in-sample), **0.892** (holdout) | BENCHMARKS.md §¹⁵, F29 |
| Gemma-4-E2B-QAT Q4_0 (model rung alone) | **0.758** (served shape), **0.808** (letters readout) | BENCHMARKS.md §¹⁵ |
| Engine relational solver | **0.683** | BENCHMARKS.md parity table |
| Engine lexical rung | **0.483** | BENCHMARKS.md parity table |
| Zero-ML engine (public split) | **0.401** (jabr Board C OOD) | BENCHMARKS.md Board C |

The fusion ladder at 0.892 holdout is the correct number for our system. The 0.933 is in-sample
(gate fitted on the suite it was scored on); the out-of-sample holdout confirms the gain (+9.2 pp
over fusion-v1 on the held-out suite, consistent with the in-sample +9.1 pp).

### Board C — out-of-distribution (jabr)

The jabr classifier benchmark (866 cases across 49 tasks, CC0) is the OOD gate. The zero-ML
engine scores 0.401 macro — the honest floor. The best open system on that board (Von) is 0.704.
There is a large OOD gap to close.

---

## 2. Where the system actually stands today

### Shipped: Phases 0–20, v0.5.0 (2026-10-08)

All phases through Phase 20 are complete. The system ships as a deterministic decision runtime
with a five-rung escalation ladder (exact rules → relational solver → lexical → model →
external), calibration seam, confidence-gated verification, and an MCP interface.

Key shipped capabilities:
- **Decision IR** (Phase 1): canonical typed IR, non_exhaustive enums, trace_version on traces
- **DAG executor** (Phase 3): topological, cycle-checked, LRU cache with SHA-256 key builder
- **Relational solver** (Phase 15): grammar-extracted facts → closed-form operators, 0.950 @ 1.3 ms
- **Fusion v2 ladder** (Phase 13/14/15): lexical + relational + Gemma-4-E2B-QAT Q4_0, 0.892 holdout
- **Temperature calibration** (Phase 14): fitted artifacts, proof-aware wrapper, versioned
- **Focus extraction** (Phase 16): reverse escalation, long-suite A/B answer-identical 120/120
- **Vision modality** (Phase H): SigLIP2 embedder, image carriers through IR and adapters, image CLI

### Open: Phases 21 and 22

- **Phase 21** (WCAG 2.1 AAA + progressive enhancement): opened 2026-10-06, fixes landed, deploy pending
- **Phase 22** (docs freshness sweep): opened 2026-10-06

### T5500 V100: Task T-047, blocked by PSU

The T5500 V100 research and optimization plan is documented in
`docs/T5500_V100_OPTIMIZATION_PLAN.md`. The plan covers serving Strands Decider Gemma 4 E4B
on 4× V100 via 1Cat-vLLM. This is not yet a system capability — it is a future serving
configuration once hardware is available.

---

## 3. Fine-tuning: what was tried and what the results mean

### The training program summary (TRAINING.md §9, T3.3)

The system ran an extensive own-training program over October 2026. The findings are recorded
in detail in TRAINING.md. The key results:

**r2 (first attempt, 2026-10-06):** Qwen3.5-0.8B LoRA r16 on 178k rows from SargeDev/jev-distill-corpus-v1.
GATE_FAIL at 0.5667 vs 0.68 threshold. Root cause: 74.6% of the corpus was one question family
(relevance/noul), score labels were 89% label-0 (near-constant), and zero relational/compositional
analogs existed. The relational class stayed at base model level.

**r16 (merged-v5 corpus):** 2B LoRA r16. GATE_FAIL. Halving rank improved but still failed.
The merged-v5 corpus catastrophically regressed the relational/compositional slice vs the E1
corpus. Root cause: corpus volume ≠ transfer. More weakly-labeled data destroyed performance.

**t33p (merged-v6-full, 307k rows, 2B):** GATE_PASS × 2 (suite holdout ≥0.68, JevBench ≥0.6494).
But E1-E (the original iterb scaffold corpus, 178k rows) remained strictly better on every leg
despite being smaller. **Confidence-gated volume ≠ transfer.** The suite→board drop for t33p
was −3.4 pp vs E1-E's −12.4 pp (better cross-lane agreement), but the raw numbers are lower.

**T88 imatrix Q4_K_M quant arm:** Triple-FAIL. Determinism replay failed, board leg failed (0.6104),
no-regression failed. The Q4_K_M quantization of the adapter regressed −4.8 pp vs the unquantized
reference. The imatrix leg of the quant bake-off closes no-go.

**v2-0.8B own-trained arms (BENCHMARKS.md, 2026-10-12):**

| Arm | Holdout-120 | rel200 | ECE |
|---|---|---|---|
| v2-08b-hard | 0.700 | 0.521 | 0.158 |
| v2-08b-soft | 0.675 | 0.518 | **0.092** |

The soft distillation arm's key finding: **calibration improved at matched accuracy** (ECE 0.092
on the relational slice vs 0.158 for hard, at nearly identical accuracy). This is the soft-as-calibrator
pivot — distillation buys calibration, not accuracy.

**The release candidate remains E1-E** (Qwen3.5-0.8B LoRA r16 on iterb corpus, suite 0.8083,
JevBench 0.6840, relational 0.610). It is the only own-trained arm through the release gate.

### What the fine-tuning program proved

1. **Relational/compositional training data is the binding constraint.** Every arm that lacked
   engine-generated relational chains scored near-base-level on the relational class. The
   iterb scaffold corpus's 38%-itemgen-stream content was the active ingredient.

2. **Corpus volume does not substitute for data quality.** Adding weakly-labeled data (merged-v5,
   merged-v6) at scale did not transfer. The data mix matters more than the data volume.

3. **Soft distillation is a calibration lever, not an accuracy lever.** At matched accuracy,
   soft distillation improved ECE by 0.066 on the relational slice. This is real but modest.

4. **QAT is the single largest measured effect in the board (+45.8 pp for Gemma-4-E2B-QAT Q4_0)**
   — but this is a third-party trained model, not our own fine-tuning.

5. **The 13 pp gap to JEV-27B is real and is not closed by our fine-tuning program.** E1-E at
   0.684 on JevBench vs JEV-27B at 0.887 is still a 20 pp gap. The iterb scaffold is the best
   our training produced; the board's own Qwen3.5-4B fork arm at 0.757 is 13 pp below the target.

---

## 4. Training corpus and data program findings

### The critical corpus

**SargeDev/jev-distill-corpus-v3** (Apache-2.0) is the named training source. Both VTX-JEV-3
and autotrust/JEV-27B train on it. It contains paired typed judgments (Choice/Noul/Score with
full teacher distributions) across chemistry, fact-checking, safety, security, code, and dialogue
routing. The iterb scaffold's engine-generated relational items (root-cause, healthiest,
first-restored chains) are what made the E1-E arm transfer — they are not in the raw corpus.

### Data mix findings (TRAINING.md T3.3)

| Corpus | Rows | Relational analog | Result |
|---|---|---|---|
| iterb scaffold (E1-E) | 178k | 38% engine-generated chains | GATE_PASS, best on every leg |
| merged-v5 | ~182k | 0% | Catastrophic regression on relational |
| merged-v6-full | 307k | 0% (same issue) | GATE_PASS but lower than E1-E |
| e1-softlabel-v2 | 188k | 0%, 77% boolean | Relational collapsed to 0.15 |

**The next lever is consensus-flip patching and strands consensus pass** (quality-first),
not more corpus volume. The T3.3 program closed the corpus-volume lever.

### Contrastive / hard negative findings

Kabra et al. (ICLR 2026) found: SFT on synthetic multi-hop data raises in-distribution accuracy
but fails to transfer to held-out tasks, while RL on the same data transfers. Generator structural
diversity (≥2 independent template families) is the actual transfer fix. Hard negatives are the
shortcut-killer, not the reasoning teacher. The adopted B3 requirement: ≥2 independent relational
template families, and negatives must be exact-proof-verified wrong answers over the same fact graph.

---

## 5. NLI verifier probe result

**Item 6 of the §15.6 ranked program (DECISIONS.md):** DeBERTa-v3 NLI as a structurally
uncorrelated second opinion for the verification ladder.

Result (2026-10-08): `MoritzLaurer/deberta-v3-base-zeroshot-v2.0` (ONNX export) scored
**0.5325** on JevBench-231 public split — between VTX (0.411) and the 0.8B bridge (0.649).
The per-family split confirms the premise: entailment-shaped families near ceiling
(tool_selection 1.000, intent 0.875), composition-shaped families below chance
(multi_hop 0.111, trap 0.125). **NLI is not a general decider; it is the second opinion for
entailment-shaped decision classes.** The verify-band wiring stays an open rung-level decision.

---

## 6. Quantization findings

### QAT is the largest single effect measured (r16, 2026-10-04)

Gemma-4-E2B-QAT Q4_0: **0.808** at 423 ms p50. Non-QAT Q4_0 of the same weights: **0.350**.
**+45.8 pp from training-time quantization-aware fine-tuning**, with clean determinism replay.
The official-weights Q4_0 (lmstudio-community) beats unsloth's UD-Q4_K_XL (+4.1 pp, better
calibration, faster, provenance rule established).

### K-quants are safe ≥4-bit, i-quants fail under requant

From r11b and r19:
- K-quants (Q4_K_S through Q6_K): safe for requant from Q8_0, replay-clean above Q4_K_S
- Legacy Q4_0: collapses under requant (0.717, 16 invalid dists, flip)
- i-quants (IQ4_XS, IQ3_M): fail under requant (flips, invalid dists)
- Sub-3-bit: worthless on decision tasks

### Imatrix quantization of own-trained adapter: no-go (T88)

Q4_K_M imatrix quantization of the t33p adapter regressed −4.8 pp vs unquantized.
The merge_and_unload → convert_hf_to_gguf path itself costs ~0.9 pp. No quant tier
of this arm can clear the board leg. The quant bake-off is closed for this adapter.

### T5500 V100 quant options (T5500_V100_OPTIMIZATION_PLAN.md)

For V100 SM70 serving:
- **Gemma-4-E2B-QAT Q4_0**: 3.12 GB, 0.808 accuracy, 423 ms — the proven small-model operating point
- **Strands Decider E4B Gemma 4 (no official GGUF yet)**: JevBench ~80.9%, Q4_K_M ~4.6 GB
  — requires building the GGUF export pipeline
- **TurboQuant v3**: draft PR #779 pending SDPA merge into 1Cat-vLLM; not yet available
- **GSQ-RCO**: requires H100/Hopper for quantization compute; ISTA-DASLab model proves it at scale

---

## 7. Engine architecture audit

### DAG executor and confidence system

The five-rung escalation ladder is clean and well-tested. Key design properties:

1. **ProofAwareCalibration** correctly separates exact proofs (p=1.0 by construction) from
   tempered model outputs — the core insight of the calibration seam is implemented correctly.

2. **LadderPolicy** composition is clean: per-node overrides beat per-kind overrides, ladder
   identity in cache keys prevents silent re-gating. Non-empty ladders require distinct id
   (validation prevents silent cache-key collision).

3. **Relational solver** is underfed by the fact grammar. Every model scores ≤0.50 on the
   relational class; the engine's own relational solver proves 0.950 at 1.3 ms on the same
   class. The grammar expansion (≥2 independent relational template families, exact-proof-verified
   hard negatives) is the highest-leverage data-side move.

4. **The D35 spread bound** is correctly implemented with dedicated tests covering all three
   saturation failure modes.

5. **Cache key construction** follows D6/D14: graph version, model/calibration/policy versions,
   state hash, question fingerprint, ladder identity. The single-site SHA-256 CacheKeyBuilder
   prevents crate-level cache key proliferation.

### Calibration seam

`Calibration` trait + `TemperatureCalibration` + `ProofAwareCalibration` + `EnsembleCalibration`
(stub). The AURC gate (D15 amendment, 2026-10-07) refuses artifacts whose risk-coverage trade-off
worsens. The fit provenance carries `aurc_before`/`aurc_after`/`accuracy_at_coverage`. The
engine's load gate (`TemperatureCalibration::from_artifact`) enforces all invariants on
deserialization.

### ONNX status

ONNX runtime (ort = 2.0.0-rc.13) is **deferred** (D2). The feasibility gate found: (a) ort
demands API level 1.27 which needs a newer ONNX Runtime than available on the host; (b) bit
identity vs Rust softmax fails (max 5 ULP agreement); (c) latency passes (p99 31–61 μs vs
5 ms budget). The pragmatic path is llama.cpp Vulkan build, not ORT. `opencodifier-runtime`
defines the `InferenceBackend` trait; the ONNX path is deferred until the API-level constraint
is resolved.

---

## 8. Vision integration (Phase H)

Recent work (branch `vision/vision-rung-2026-10`, commits on top of v0.5.0/v0.6.1):

- **SigLIP2 embedder** (`d3db99f`): zero-shot vision-language contract in `opencodifier-model`
  via `onnx-community/siglip2-so400m-onnx`. Serves as a vision-language embedder behind
  `opencodifier-runtime`'s `EmbeddingBackend` trait.

- **Image modality in the escalation ladder** (`3677c15`): `RungModality::Image` in
  `opencodifier-engine/src/ladder.rs`. Image-bearing states skip `Text` rungs and route
  to image-capable rungs; image requests without image rungs are refused at assembly time.

- **Image carriers through wire adapters** (`1f53859`): `image_url` and `image_base64` fields
  in the IR, carried through OpenAI/Anthropic/Jev schema adapters.

- **Image CLI** (`8dc85c5`): `opencodifier decide --image <path>` surfaces through the CLI.
  Phase H audit (`8337864`): Aegis scan + WCAG accessibility check. Committed on
  `vision/vision-rung-2026-10`.

This is a significant new capability that extends the system beyond text-only decision routing.
The vision ladder is opt-in (empty by default, byte-identical to text-only for existing deployments).

---

## 9. Composite deployment score — the real question

The composite formula (`docs/BENCHMARKS.md`, "Composite deployment score"):

```
A_trust = clamp(accuracy − ECE)
Spd     = clamp(1 − log10(p50_ms) / 4)      # 1ms→1.00, 10ms→0.75, 100ms→0.50, 1s→0.25
Res     = clamp((4.4 − log10(size_MiB)) / 2.2)  # 20MiB→1.00, 2GiB→0.50, 9GiB→0.20
Overall = 100 × (0.45·A_trust + 0.30·Spd + 0.25·Res)
```

| System | Acc | ECE | A_trust | Spd | Res | Overall |
|---|---|---|---|---|---|---|
| OpenCodifier engine (relational over lexical) | 0.683 | 0.094 | 0.589 | 0.97 | 1.00 | **80.7** |
| VTX-JEV-3 lf2 | 0.242 | 0.136 | 0.106 | 0.99 | 1.00 | **59.5** |
| **OpenCodifier ladder v2** | 0.933 | 0.092 | 0.841 | 0.40 | 0.41 | **60.0** |
| Jebadiah-4B Q3_K_S | 0.792 | 0.087 | 0.705 | 0.15 | 0.50 | **48.7** |
| Winnow-E4B (vision, Q8_0) | 0.842 | 0.109 | 0.733 | 0.17 | 0.23 | **43.8** |

**The engine wins the composite by a large margin** (80.7 vs 60.0 for the fusion ladder).
This is not a bug — it is the architecture working as designed: the model rung is a
last-resort escalation, and the composite score weights speed and footprint heavily
because those affect every decision. The model's accuracy contribution is real but modest
on the composite.

**The decision for a deployment is**: how much accuracy do I need, and what am I willing
to pay in latency and footprint? For high-stakes decisions: the fusion ladder at 0.892
holdout. For volume processing: the zero-ML engine at 0.683 / 0.401 OOD.

**The composite score does not answer "which system is more accurate."** It answers
"which system is the best trade-off for my deployment." The gap between the engine's
80.7 and the ladder's 60.0 is the price of model quality.

---

## 10. What the T5500 V100 plan actually addresses

The T5500 plan (`docs/T5500_V100_OPTIMIZATION_PLAN.md`) is a **serving configuration plan**,
not a system capability. It documents how to serve Strands Decider Gemma 4 E4B on the
T5500's 4× V100 once the PSU arrives. The value is:

1. **Hardware utilization**: V100 SM70 can serve Gemma-4-E2B-QAT Q4_0 (3.12 GB) or
   Strands Decider E4B Gemma 4 Q4_K_M (~4.6 GB) with KV cache headroom in a single V100.
   TP1 is sufficient; TP2 for 12B models introduces Marlin kernel risks on V100.

2. **Throughput**: 1Cat-vLLM achieves 71.1 TFLOP/s attention on V100 (vs 36.2 stock llama.cpp).
   For batch decision workloads, this is significant.

3. **The GGUF export blocker**: No official GGUF export of `strands-decider-E4B-gemma4-v1-2610`
   exists. Building it requires `strands_decider.hf_export` from the Strands Decider source repo.
   This is a prerequisite that is not yet resolved.

4. **Community GGUF available now**: `fabricant451/strands-decider-2B-hobson-v19-GGUF` (526
   downloads) is testable immediately on V100 once the PSU arrives.

The T5500 plan does not close the 13 pp JevBench gap. It documents a V100-optimized
serving configuration for the Gemma 4 E2B QAT model (which already scores 0.808 on the suite)
and the Strands Decider E4B model (which scores ~80.9% on JevBench-231 — about 5 pp above
our fork arm on the same benchmark).

---

## 11. Honest remaining gaps

### 1. The 13 pp JevBench gap to the target bar

JEV-27B at 88.70 vs our fork arm at 75.71. The training program produced E1-E at 0.684 on
JevBench — 4 pp above our fork arm, but still 20 pp below the target. The iterb scaffold
was the best recipe found. The next levers are:
- Consensus-flip patch / strands consensus pass (quality-first)
- ≥2 independent relational template families for hard negatives
- Per-kind temperature calibration fitted into the artifact

### 2. OOD generalization (Board C: 0.401)

The jabr classifier benchmark at 0.401 macro is the honest floor. The zero-ML engine
has no transfer to OOD tasks. The NLI verifier probe (0.532) is not a general solution.
This requires either a larger training corpus with diverse task types, or a different
architecture for the verification rung.

### 3. The relational class ceiling (≤0.50 for every model ≤4B)

Every model arm ≤4B scores ≤0.50 on the relational/compositional class. Only MoE models
(9B class) cross this ceiling. The relational solver proves 0.950 at 1.3 ms on the same
class — so the engine covers it, but the model rung cannot. This is the binding constraint
on the fusion ladder's quality ceiling: every model rung error is on a relational item
that the solver already answers correctly.

### 4. No official GGUF export of Strands Decider Gemma 4

The highest-value import (Strands Decider E4B Gemma 4 at ~80.9% JevBench) is blocked on
building the GGUF export. The `strands_decider.hf_export` tool must be obtained from the
Strands Decider source. This is not a research gap — it is an engineering gap.

### 5. Per-quant determinism replay is not yet automated in CI

The benchmark harness computes Δp between identical replays, but this is not yet a CI gate.
Four of six requant quants flip predictions on identical replays (r11b). Every deployment
quant must pass a determinism replay test before shipping.

### 6. Eval lane determinism under host load (T88 finding)

The CPU fork lane does not replay byte-identical under co-tenant host load. The determinism
replay gate (byte-identical) is stricter than this lane can satisfy. A tolerance definition
is needed: same verdict set, identical accuracy — which is what the benchmark harness
already computes as Δp.

### 7. Phase 22: docs freshness sweep

Opened 2026-10-06. PLANNING.md, TRAINING.md, and BENCHMARKS.md are current through the
latest runs. The research frontier (RESEARCH.md) is current through §21. No specific gaps
identified beyond the normal incremental updates.

---

## Summary

OpenCodifier v0.5.0 is a mature, well-tested deterministic decision runtime. The fusion
ladder at 0.892 holdout accuracy represents real progress: it combines a 1.3 ms relational
proof layer with a QAT-trained 2B decision model to achieve near-90% accuracy on the
120-item suite. The zero-ML engine at 0.401 OOD is the honest deployment floor.

**The gaps are not architectural.** The engine, calibration, and ladder designs are sound.
The gaps are empirical:

1. The 13 pp JevBench gap to the target bar (training data quality, not architecture)
2. OOD generalization (0.401 on jabr Board C)
3. Relational class ceiling (every model ≤4B ≤0.50; engine covers it, model rung cannot)
4. No official GGUF export of Strands Decider Gemma 4 (engineering blocker)
5. Per-quant determinism replay not yet automated in CI
6. Eval lane determinism under host load needs a tolerance definition

**The T5500 V100 plan addresses serving configuration**, not the accuracy gap. Serving
Gemma-4-E2B-QAT Q4_0 (0.808) or Strands Decider E4B Gemma 4 (~80.9%) on V100
hardware is a throughput and cost optimization — real, but it does not close the
13 pp JevBench gap.

**The composite deployment score is the correct frame.** The engine at 80.7 composite
vs the ladder at 60.0 means: model quality gains are real but cost speed and footprint.
The deployment decision is a configuration question, not a correctness question.

---

*Corrected audit — Session 76, 2026-10-19. Sources: BENCHMARKS.md (full board, 113 runs),
PLAN.md (Phases 0–24), TRAINING.md (T3.3 program, T88 verdict), DECISIONS.md (D1–D31),
T5500_V100_OPTIMIZATION_PLAN.md, RESEARCH.md §21, git log.*
