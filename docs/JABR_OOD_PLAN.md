# Jabr OOD Plan — Out-of-Distribution Generalization on Board C

**Date:** 2026-10-18
**Branch:** `vision/vision-rung-2026-10`
**Status:** Draft — for task #166 (benchmark dominance)
**Reference:** `docs/RESEARCH.md §14`, `docs/BENCHMARKS.md Board C`, `docs/BENCHMARK_PLAN.md §9`

---

## Context

**The jabr benchmark** (866 cases, 49 tasks, CC0) is the OOD gate. Our zero-ML engine scores **0.401 macro**. The best open system (Von) scores **0.704**. The gap is **30 pp** — the largest single gap between our system and the field.

This is **not** a readout or calibration problem. It is a fundamental OOD generalization failure: our training data does not cover the task distributions jabr tests.

**Why this matters:** JevBench-231 accuracy is an in-distribution metric. We can close the JevBench gap through better training data composition. But jabr tests generalization to tasks we've never seen — which requires a different approach.

---

## What We Know

### From RESEARCH.md §14 + BENCHMARKS.md Board C

1. **The jabr harness is public** (CC0, `jabr/classifier-benchmark`). We can run ourselves on it without restriction.
2. **Zero-ML engine: 0.401 macro** (866 cases, self-run 2026-10-06). The engine's relational solver handles the tasks it recognizes; it fails on everything else.
3. **Von (best open): 0.704** — uses a purpose-trained classifier. GLiNER2: 0.698. Laya: 0.583.
4. **Jev (teacher): 0.966** — the ceiling. This is a fine-tuned system, not a zero-shot one.
5. **E1-E: 0.684 on JevBench** — our best arm on in-distribution tasks. Unmeasured on jabr.
6. **The mc5-jev GGUF failure** (0.6407 on JevBench vs authors' 0.7792) is a warning: our GGUF conversion pipeline destroys adapter quality. We must not assume a GGUF-served adapter retains its in-distribution ability OOD.
7. **Kabra et al. found**: SFT on synthetic multi-hop data raises in-distribution accuracy but fails to transfer OOD. RL transfers. This is the most important finding for this plan.

### The Two Root Causes

| Root cause | Symptom | Solution |
|---|---|---|
| **Coverage gap** | Our training data doesn't include jabr-style tasks | Add jabr-style tasks to corpus; VIVERE lane generates decision items in the style of tasks we fail |
| **Compositional generalization failure** | Our training data includes similar tasks but the model can't generalize to new compositions | RL-based training (PPO/GRPO) instead of SFT |

---

## Phase 0 — Diagnose: Run All Arms on Jabr

**Gate: Everything else waits on this. We cannot design a fix without knowing which failure mode we have.**

### 0.1 Run zero-ML engine on jabr (baseline — already done)

- **Score:** 0.401 macro
- **Per-task breakdown needed:** Which tasks does the engine handle? Which does it fail on?
- **Deliverable:** Per-task accuracy table, identifying the hardest task clusters

### 0.2 Run E1-E on jabr

- E1-E (0.684 JevBench) has never been measured on jabr.
- If E1-E ≥ 0.60 on jabr: the LoRA generalizes partially. The gap to Von (0.704) is closing.
- If E1-E < 0.50 on jabr: the LoRA collapses OOD. This confirms the coverage gap is the primary problem.
- **Deliverable:** E1-E jabr score + per-task breakdown

### 0.3 Run strands and minicpm on jabr

- strands (0.7706 JevBench) and minicpm (0.7792 JevBench) are the strongest arms we've measured.
- If either ≥ 0.65 on jabr: that arm + freeform readout is the OOD deployment candidate.
- **Deliverable:** strands + minicpm jabr scores

### 0.4 Per-task failure mode analysis

For every task where all arms fail:
1. Is the task format covered by our corpus? (metadata_match, relational_compositional, etc.)
2. Is the failure one of: (a) novel domain, (b) novel task structure, (c) long-context, (d) multi-hop composition?
3. This determines whether VIVERE generation or RL training is the right lever.

---

## Phase 1 — If the Gap Is Coverage (most likely)

**Trigger: E1-E < 0.50 on jabr, or per-task analysis shows novel domain coverage gaps**

### 1.1 Map jabr task taxonomy to corpus gaps

Cross-reference the 49 jabr task families against our training corpus. Identify which families are:
- **Missing entirely** — no items of this type in corpus
- **Present but underrepresented** — few examples relative to JevBench composition
- **Present with wrong structure** — e.g., our multi-hop items use a different composition pattern than jabr's

**Deliverable:** Gap analysis table per task family

### 1.2 VIVERE generation for missing task families

The VIVERE lane generates decision items from text. For each missing/underrepresented family:
1. Feed task description + few-shot examples into VIVERE prompt
2. Generate 500–2000 items per family
3. Validate: each generated item must have a verifiably correct answer
4. Add to training corpus

**Target:** Cover all 49 jabr task families with ≥ 500 items each.

### 1.3 Retrain on expanded corpus

Fine-tune E1-E base (Qwen3.5-2B) on corpus + VIVERE-generated items.

**Gate:** ≥ 0.60 macro on jabr (the RL-free path)

---

## Phase 2 — If the Gap Is Compositional Generalization

**Trigger: E1-E ≥ 0.50 on jabr but < 0.60, or VIVERE generation doesn't close the gap**

### 2.1 RL training assessment

Kabra et al. (arXiv reference in RESEARCH.md): RL (PPO/GRPO) transfers OOD where SFT fails.

**Options:**
- **GRPO** (SimPO, ORPO): Cheaper than PPO, shown effective for reasoning tasks
- **PPO**: More expensive but more studied

**This is a T5500 GPU investment.** Pre-register expected improvement before starting.

### 2.2 Pre-registration required

Before any RL run:
- State the expected jabr score
- State the arm and base model
- State the gate threshold
- No post-hoc selection of the best run

---

## Phase 3 — NLI Verifier as OOD Supplement

**No retraining required — ladder configuration change**

The NLI verifier probe scored **0.532 on jabr** — halfway between our engine (0.401) and Von (0.704).

When the model rung is uncertain, an NLI entailment check over the candidate options provides a second opinion.

### 3.1 Wire NLI verifier into the ladder

Add an NLI entailment step as an OOD supplement:
- For each candidate option, compute NLI(evidence, option_text) → entailment score
- Combine with model confidence → calibrated OOD decision
- **Target:** ≥ 0.55 on jabr from NLI alone; ≥ 0.70 combined with model rung

### 3.2 Validate NLI supplement

Run jabr with and without NLI supplement. Report the delta.

**This is the fastest path** to an OOD improvement — no retraining required.

---

## Phase 4 — Von-style Approach (long-term)

Von (0.704) is a purpose-trained classifier. Its architecture:
- Base: a strong encoder (DeBERTa-v3)
- Training: on-domain fine-tuning for each jabr task family
- Inference: per-task specialized heads

### 4.1 Task-specialized heads

If our generalist approach caps at ~0.65, consider task-specialized heads:
- One lightweight classifier per task family
- Gated by task-type metadata
- Combined with the generalist ladder

### 4.2 Engineering cost estimate

- Per-task head training: ~2 hours each × 49 tasks = ~100 hours
- Inference cost: 49 extra models in the serving stack
- **This is only worth it if Phase 1–2 don't reach 0.70**

---

## Decision Tree

```
Run all arms on jabr (Phase 0)
        |
        v
E1-E >= 0.60? ──yes──> NLI supplement (Phase 3) ──> target reached? ──yes──> DONE
        |
       no
        v
E1-E >= 0.50? ──yes──> VIVERE generation (Phase 1) ──> retrain ──> target reached?
        |                                                       |
       no                                                      no
        v                                                       v
   RL training (Phase 2) ──> pre-register ──> T5500 run ──> target reached?
                                                           |
                                                          no
                                                           v
                                             Task-specialized heads (Phase 4)
```

---

## Success Metrics

| Phase | Metric | Gate |
|---|---|---|
| Phase 0 | Per-task jabr scores for all arms | Must complete before any training |
| Phase 1 | VIVERE coverage of all 49 task families | ≥ 500 items per family |
| Phase 1 | Retrain jabr score | ≥ 0.60 macro |
| Phase 3 | NLI supplement delta on jabr | ≥ +0.05 over no-supplement baseline |
| Phase 2 | RL jabr score | ≥ 0.70 macro (pre-registered) |
| Phase 4 | Task-specialized heads jabr score | ≥ 0.70 macro |

**Target:** ≥ 0.70 macro on jabr (Von-class open systems)

---

## Risk Register

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| E1-E completely collapses on jabr (< 0.30) | Medium | High | VIVERE generation is the recovery path |
| VIVERE items are low-quality (hallucinated answers) | Medium | High | Validate each item programmatically before adding to corpus |
| RL training is required but T5500 PSU is unavailable | High | High | NLI supplement is the fallback; publish results with honest "GPU-limited" caveat |
| GGUF conversion destroys OOD quality (mc5-jev pattern) | Medium | Medium | Serve adapters via `--lora` at inference time, not merged |
| Von-style task heads require too much engineering | Low | Low | Only pursued if Phases 1–2 don't reach target |
