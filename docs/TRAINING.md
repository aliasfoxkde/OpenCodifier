# Training on this host: LoRA adapters and our own decision model

Research record, 2026-09-29. Question: can adapter training (LoRA-class) lift
our board numbers — especially the small/fast tiers — without re-training a
base model, and could we train it **on this host** (AMD Renoir APU, 16 cores,
30 GB RAM, no discrete GPU)?

Companion to `docs/BENCHMARKS.md` (the measured boards this analysis draws on).

## 1. Why the question is live

The bar we set for ourselves — autotrust/JEV-27B, 84.07 mean across six public
decision benchmarks — was itself built with this technique class: a **frozen**
Qwen3.8-27B backbone plus a **108.9 M-parameter decision block (≈0.4 % of the
model)**, initialized from the backbone's own `lm_head` rows for the answer
tokens, one forward pass per decision (~137 ms), trained **9.2 hours on a
single B200** on the public `SargeDev/jev-distill-corpus-v3` (Apache-2.0,
distilled from Jev 1.13 outputs; train split + 877 held-out test rows).
Nothing on our board is within 25 points of that number today.

We already exploit the *free* version of the same idea: D16's distill rows
(Qwen3.8-Distill 2B/4B at 0.725/0.767 suite) are other people's
decision-tuned checkpoints, picked by measurement. The open question is
whether *we* can produce one, at the size we want, tuned to **our** readout.

## 2. Provenance, corrected

- **LoRA itself is Hu et al., 2021 (Microsoft)** — rank-decomposed low-rank
  updates on frozen weights. NVIDIA's contribution is the **serving** story:
  TensorRT-LLM serves many LoRA adapters against one resident base engine,
  and NIM/Dynamo route requests to per-tenant adapters (base loads once,
  each adapter is a few hundred MB). That pattern is GPU-fleet economics;
  it does not apply to our single-host, local-first posture.
- Our serving path is **llama.cpp `--lora`**: GGUF-format adapters applied at
  load time (optionally scaled) over a base GGUF; HF's GGUF-my-LoRA Space
  converts PEFT adapters. Two measured-by-others caveats we must respect:
  runtime adapters are **slower than merged** weights (extra low-rank matmuls
  on the same memory bus), while **merged→GGUF conversion can lose fine-tune
  fidelity** (llama.cpp issue #7062) — so the adapter file, not the merge, is
  the fidelity-preserving artifact. Upstream llama-server has no first-class
  runtime adapter hot-swap yet (discussion #7850); restart-per-adapter is
  acceptable for benchmarking.
- Our benchmark fork (`parallel-decision`) has **unknown** `--lora` +
  `/v1/decision` tree-mode interaction — that is a 10-minute probe before
  any training investment.

## 3. The fact that prices this experiment

We already have a data point for "corpus-distilled 0.8 B decision model" on
our own board: **Jev-Style-0.8B (Q4_K_M) scores 0.6494 on JevBench-231**
(bridge arm, +0.9 pp vs its published 64.1 %). A community member already
ran this recipe at 0.8 B and the result is known. Training our own adapter
is therefore not a shot in the dark — it must **beat or justify itself
against 0.6494** (JevBench) and **0.650** (Qwen3.5-0.8B suite baseline),
with one real edge the community checkpoints don't have:

- tuned for **our readout** — one-token constrained decision completions /
  tree-mode scoring — rather than prose chat, which is exactly the failure
  mode we measured in the writing-distills (Qwen3.8-2B: chat 0.700 > decision
  0.617; MiniCPM5 chat collapse 0.092);
- wired into **our calibration seam** (D15: per-kind temperature, fitted
  runner already exists);
- exportable as **the product's decision rung**: a decision-head model reads
  out in one forward pass and exports to ONNX — which is literally what
  `opencodifier-model` + `opencodifier-runtime`'s `ort` feature consume.

## 4. Can this host train it? (the arithmetic)

Training-flop reality check: LoRA cuts **optimizer/gradient memory**, not
compute — frozen layers still run forward *and* backward (activation
gradients), so the classic ≈6·N·D FLOP/epoch estimate still applies.

| Step | Estimate on 16 cores (~300 GFLOPS effective, bf16) |
|---|---|
| Qwen3.5-0.8B LoRA (r=16), 1 epoch × 10 M tokens | ≈ 4.8·10¹⁶ FLOP ≈ **44 h** |
| Same, 3 M-token subset (Jev-distill decision rows are short) | ≈ **13 h/epoch** |
| First viable run: 0.8 B, r=16, 1–2 epochs, 3–5 M tokens | **≈ 1–2 CPU-days** |
| Same recipe at 2 B | ≈ 3–6 CPU-days (poor first target) |

RAM fits easily: bf16 weights ≈ 1.6 GB + activations for seq 512 / batch 8 +
LoRA-only gradients/optimizer state → ~10–15 GB, inside 30 GB. Tooling:
`conv-venv` already has CPU torch + transformers; `peft` is a small pure
addition. What does **not** work here: bitsandbytes/QLoRA (CUDA-only), MLX
(Apple-only), any NF4 path. llama.cpp's `finetune` example is effectively
dead upstream; PyTorch+PEFT is the training path, llama.cpp the serving path.

So: **yes, a first 0.8 B LoRA is a 1–2 day nice-level run on this host**,
with the harness measuring the trainer itself (the resource monitor now in
the runners records trainer CPU/RAM/IO too). GPU-class results in GPU-class
time would still need rented hardware (JEV-27B's 9.2 B200-hours; a 0.8 B
version is roughly two orders below that) — out of scope unless that changes.

## 5. Two variants, in order of architectural fit

1. **Decision-head adapter (JEV recipe shrunk to 0.8 B)** — freeze the base,
   train a small decision block initialized from the base's `lm_head` rows
   for our candidate/answer token ids, read out one forward pass of logits
   over candidate tokens. Same training cost class as LoRA (the frozen body's
   forward+backward dominates), but the *inference* artifact is exactly our
   decision rung: small, ONNX-exportable, calibrated through D15, cache keys
   fold its artifact version (§73 rule). This is the target.
2. **Classic LoRA (r=16, q/k/v/o + gate/up/down)** — the general-purpose
   fallback; serving cost measured by the ± adapter A/B in the same arm.

## 6. Expected gain, honestly bounded

Anchors: Jev-Style-0.8B **0.6494** (JevBench), Qwen3.5-0.8B base **0.650**
(suite), Qwen3.5-2B **0.725**. A decision-tuned 0.8 B plausibly lands
**0.65–0.75 suite / 0.65–0.72 JevBench** — competitive with our 2 B balanced
tier at 0.8 B cost, and it does not threaten the 4 B/9 B tiers. The failure
mode to watch is the measured one: decision-shaped training data only, no
prose completions, or we reproduce the writing-distill degradation.
Calibration expectations: per-kind temperature after training, not before.

## 7. Experiment ladder (costed, gated)

| # | Step | Cost | Gate to proceed |
|---|---|---|---|
| E0 | `--lora` + `/v1/decision` probe on the benchmark fork (any matching small adapter) | minutes | adapter applies and tree-mode scores |
| E1 | Corpus prep (v3 → decision-completion rows) + 0.8 B decision-head/LoRA training, 1–2 epochs | 1–2 CPU-days, nice'd | suite ≥ 0.68 **and** JevBench ≥ 0.6494 (i.e. beats base and the community checkpoint) |
| E2 | Winner → ONNX export → `opencodifier-model` decision rung integration | 1 day | logits parity vs trainer (torch-to-the-digit rule), calibration fitted |
| E3 | Recipe → 2 B balanced tier, only if E1/E2 hold | 3–6 CPU-days | ≥ 0.75 suite at 2 B |

Every run lands through the standard arm discipline: quiet host (load < 8),
sha256s into `models.manifest.json`, raw JSONs out-of-tree, REPORT/
BENCHMARKS/PLAN/CHANGELOG updated together, replay determinism, native
probabilities only.

E0 status (2026-10-05): **blocked on artifact availability**, not on
code — the fork's `--lora`/`--lora-scaled`/`--lora-init-without-apply`
flags are present in the `build-pd` server, but no ready-made
(base, adapter) GGUF pair exists for any small base on the compute
host (the one local adapter, bonsai2-27b, already failed arch
validation in r20b and rides a ternary base with collapsed prefill).
Downloading a 27B base to probe is disproportionate. The probe runs
when E1 produces our own adapter on the GPU node — E0 executes there
as the load-back gate.

## 8. Non-goals

No cloud APIs, no GPU rental by default, no training on non-Apache-2.0
corpora, no ternary/Q1_0 training, no prose-generation fine-tunes (the
runtime refuses free-form generation fields by contract).

## Sources

- Hu et al., *LoRA: Low-Rank Adaptation of Large Language Models*, 2021.
- autotrust, *JEV-27B: fast, calibrated decisions* (HF blog + model card),
  accessed 2026-09-29 — recipe numbers in §1.
- SargeDev/jev-distill-corpus (v3), Apache-2.0; JEV-27B's training corpus.
- llama.cpp: GGUF-my-LoRA (HF blog, ngxson); issue #7062 (merged-adapter
  fidelity loss); discussion #7850 (runtime adapter swap) — accessed 2026-09-29.
- NVIDIA TensorRT-LLM key features (multi-LoRA in one engine); NIM/Dynamo
  multi-adapter serving — accessed 2026-09-29.
- Arabpour et al., *LoRA Fine-Tuning Without GPUs: A CPU-Efficient
  Meta-Learning Algorithm*, 2025 — CPU-only LoRA as an active (niche) area.

## 9. Own-model distill program (opened 2026-10-03, task #88)

Goal: best-of-all-worlds on the composite (accuracy you can act on,
speed, footprint) rather than the best single arm. The board says the
pieces exist separately — Winnow-E4B owns accuracy (0.842) and vision
(mmproj), jebadiah-4b-v2 Q3_K_S owns the size/accuracy knee (1.98 GiB
at 0.792), the engine owns latency — no single artifact owns all three.

Plan, in dependency order:

1. **Dataset merge — DONE (2026-10-05, pinned).** `runner/merge_distill_corpus.py`
   produced `merged-v1` on the compute host
   (`~/oc-model-eval/corpora/merged-v1.jsonl`, SHA-256
   `03706a9a850d84fe3bc6cf750d78665c2406dcbb16c67f0d79e78a108d612cbf`):
   149,600 records — 148,160 noul + 2,040 choice + 2,400 score; splits
   train 149,360 / suite 120 / suite_holdout 120; zero contamination
   hits against either eval suite, zero parse skips. Sources:
   - **`LocalLLaMA/typed-decisions`** (Apache-2.0, 1,200 train rows) —
     the decisive find: rows are already in the IR request shape
     (`state` JSON + `questions` `{name: {type: choice|noul|score,
     instructions, criteria}}` + `gold` carrying per-option
     **probabilities**, confidence, and a scalar `noul` score) — native
     soft labels, emitted verbatim, zero-loss. Also disclosed (with
     HelpSteer2) as jebadiah-4b-v2's own training data, which both
     explains that arm's decision formatting and gives the distill the
     same training distribution as a board arm.
   - **`SargeDev/jev-distill-corpus`** (Apache-2.0, 148,160 rows) —
     relevance noul; normalized to IR noul with fixed criteria; hard
     label from `label_binary`, teacher values (`label_32b`, `jev`)
     carried raw — no probability transform invented at merge time.
   - Suites emitted tagged as eval surfaces, never deduped away.
   License rulings: `nvidia/HelpSteer2` (CC-BY-4.0, disclosed jebadiah
   source) **excluded** — Apache-2.0-only rule. The three merge-v2 open
   items are resolved by measurement (2026-10-05,
   `runner/analyze_merged_dups.py`, report on the compute host at
   `~/oc-model-eval/corpora/merged-v1-dupreport.json`):
   - **Near-dup**: v1's exact dedup held at the canonical level (zero
     duplicate `(state, question)` pairs), but MinHash LSH over
     canonicalized state text (5-gram shingles, 64 permutations, 16×4
     banding, Jaccard ≥ 0.8, zero buckets hit the pair-gen cap) found
     35,359 verified pairs in 12,971 clusters covering 48,330 records
     (32.3 % of the corpus) — all in `jev-distill-corpus`, Jaccard
     range 0.8–1.0: byte-exact v1 missed punctuation/case variants.
   - **Label conflict inside the redundancy**: 8,957 of the 12,971
     clusters (69 %) carry mixed gold labels — near-identical passages
     labelled both relevant and irrelevant. Merge-v2 policy:
     uniform-label clusters (4,014) dedup to one representative;
     mixed-label clusters are **quarantined** (separate file, excluded
     from train, cluster id + conflicting labels carried) — picking a
     winner inside a label fight invents supervision, keeping the fight
     poisons it. Quarantined rows are the VIVERE re-adjudication pool.
   - **Fourth source (fork prompt/response logs): rejected.** The arms'
     `raw/` captures store model *predictions* on eval-suite items — no
     gold labels, and folding eval surfaces into train is contamination.
   - **suite_long**: confirmed 120 items with `derived_from` recorded —
     rides tagged as eval only.
   - **Kind census** (per question — the training unit): 2,040 choice /
     149,960 noul / 2,400 score, a 33.8× imbalance; near-dup removal
     leaves ~113k unique noul against ~4.4k choice+score — the gap the
     VIVERE extraction lane fills (item 4 below).

   **merged-v2 emitted and pinned (2026-10-05).**
   `runner/emit_merged_v2.py` applied the policy on the compute host:
   4,014 uniform clusters deduped (5,569 records dropped; surviving
   representative = highest `teacher.jev`, tie-break lowest input line
   — deterministic), 8,957 mixed clusters fully quarantined (38,747
   rows → `merged-v2.quarantine.jsonl`, each carrying `_quarantine`
   cluster id + the cluster's label set — the VIVERE re-adjudication
   pool). Output `merged-v2.jsonl`: **105,284 records**
   (typed-decisions 1,200 / jev-distill 103,844 / suites 240), question
   kinds 2,040 choice / 105,644 noul / 2,400 score. Gates green: zero
   train↔suite state collisions, all 240 suite rows through untouched.
   Manifest `opencodifier.distill-manifest/2` pins both SHA-256s
   (`merged-v2.jsonl` `cb6b42cb…2915`, quarantine `289e97c3…d0ee`)
   with the policy parameters. Cluster identity reconciled exactly with
   the dupreport: 12,971 = 4,014 uniform + 8,957 mixed;
   5,569 dropped + 38,747 quarantined + 4,014 representatives = 48,330
   in-cluster records.
2. **Quantization floor first (r11, in flight).** Winnow-E4B Q4_K_M /
   Q4_K_S / Q4_0 / IQ4_XS / IQ3_M / Q3_K_S, letters readout, scored by
   the same composite as every arm. The quant curve tells the distill
   program what accuracy a 4-bit E4B-class body can carry before any
   training run is bought. Working floor per the board rule
   (BENCHMARKS.md, 2026-10-03): **4-bit K-quant or higher** — sub-3-bit
   collapses and odd-bit i-quants carry a measured speed penalty
   (UD-IQ3_XXS: 3.2× slower than UD-Q4_K_XL for −0.8 pp), so the 3-bit
   arms run as confirmation only; distill outputs target the 4-bit-and-
   up range.
3. **Distill candidates** (each benchmarked through the standard arms
   before any tier claim — same harness, same honesty rules):
   - **Winnow-E4B base + LoRA** on the merged corpus: keeps vision and
     the measured accuracy leader; Q8_0 teacher outputs from the
     quant-bake-off arms provide soft labels for free.
   - **Qwen3.5-2B/4B + LoRA**: the readout path (fork tree mode) is
     the most measured; a decision tune closes the gap its own JevBench
     anchor showed (76.6 % untuned vs 80.5 % tuned at 4B).
   - Acceptance bar: beat the best same-size arm on the composite, not
     accuracy alone — a tie on accuracy at half the size wins.
4. **VIVERE knowledge extraction is the program's primary
   dataset/knowledge lane** (owner directive, 2026-10-05 — was
   "experimental side-lane"). No expensive teacher-distillation is
   bought to extract knowledge or data: VIVERE extracts the corpus
   cheaply, improves the dataset (gap-fill, near-dup removal — the
   merge-v2 open items above), and transfers it onto the small model.
   LoRA fine-tuning remains the application step; **Unsloth is the
   named fallback tooling** if VIVERE-native transfer underperforms it
   — which method carries is measured, not assumed. The claims gate is
   unchanged: corpus runs consume shadow-ledger exports, never live
   traffic (INTEGRATION_AMORTYX §8), and every VIVERE-derived claim
   stays experimental-labeled in both trees until a measured arm clears
   the promotion gate. Running the program through VIVERE also makes it
   VIVERE's refine/validate loop — real corpus, real gaps, real
   readout delta. The concrete gap-fill spec lives in the VIVERE lane
   tree (`docs/extraction/OC_GAPFILL_SPEC.md`, VIVERE
   feat/local-extraction-lane @ 277e3ce): priority order is quarantine
   re-adjudication first (38,747 rows already on disk), then score
   questions over existing states, then claims-bridge choice; gold
   contract is raw-teacher values with k-vote hard labels, single-vote
   answers dropped, never smoothed.

   **Quarantine re-adjudication measured (qradj-v1, 2026-10-06).**
   The donor run (`qradj_full_v3`: 116,241 prompts, 99.95 % parse,
   zero truncated, zero transport errors) joined all 38,747
   quarantined records; **28,937 recovered (74.7 %)** under the
   confirm-only contract — 3/3 unanimous votes agreeing with the
   surviving teacher gold. 6,724 splits, 3,036 unanimous
   contradictions, and 50 incomplete sets stay quarantined. Cluster
   anatomy verified post-hoc: no cluster in the whole pool pairs one
   query with two labels — merge-v2's state-only grouping had flagged
   passage-QA structure (one passage, many queries) as label fights;
   every recovered row is a distinct (state, query) with zero
   same-query duplicates, so all 28,937 records are admissible with no
   dedup. Each recovered record carries a `_readjudication` block
   (raw donor votes, values + p), so the recovery is auditable from
   the record alone. Donor confidence is saturated (p50 1.0, 79.6 %
   extreme votes) and carried but not gated — the contract gates on
   vote values. Reports of record:
   `benchmarks/decision-model/results/qradj-v1-gap-report.{md,json}`;
   recovered pool `qradj-v1.recovered.jsonl` SHA-256
   `8135922b…6632f4` (compute host). merged-v3
   (105,284 + 28,937 = 134,221 records) emits after the score
   gap-fill lands; score gap-fill (item 2) is in flight — 96,000
   prompts = 16,000 merged-v2 states × 2 families × 3 votes.

Non-goals carried from §8: no non-Apache-2.0 corpora, no cloud
training by default; the burst node (T5500 2×V100) or Kaggle/Modal
remain the GPU paths, user-gated.
