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

E0 status (2026-10-05): blocked on artifact availability, not on
code — the fork's `--lora`/`--lora-scaled`/`--lora-init-without-apply`
flags are present in the `build-pd` server, but no ready-made
(base, adapter) GGUF pair exists for any small base on the compute
host (the one local adapter, bonsai2-27b, already failed arch
validation in r20b and rides a ternary base with collapsed prefill).
Downloading a 27B base to probe is disproportionate.

**E0 EXECUTED 2026-10-06 — PASS.** Unblocked by the E1 smoke adapter
(§9.5): our own LoRA converted with the fork's
`convert_lora_to_gguf.py --base …Qwen3.5-0.8B --outtype f16` (48
tensors, 2.2 MB — converting our own trained adapter is the documented
serving path, not a third-party re-quant). Serving chain proven on
CPU (`-ngl 0 -t 6`):

```
llama-server -m qwen35-08b-qat-Q4_0.gguf \
    --lora qwen35-08b-e1-smoke-lora-F16.gguf \
    --decision-seqs 24 --port 8095
```

- Adapter applies cleanly; the `CPU_REPACK → CPU` buft fallback
  warnings are benign on CPU builds.
- `/v1/decision` tree mode returns a well-formed constrained
  distribution (4 candidates summing to 1.0, `scored_nodes: 1`,
  6 scored rows), argmax = gold on the probe item,
  p ≈ 0.923, 570 ms at 245 prompt tokens.
- **Load-bearing discovery: the route is off unless the server
  starts with `--decision-seqs N` (N ≥ 3)** — the runner default is
  24. Any hand-rolled server launch for the JevBench leg must pass it;
  without it the route 400s with `decisions are disabled`.

This closes the last unvalidated piece of the training bundle
(prep §9.5 → trainer → readout eval → adapter-GGUF serving). The
JevBench leg for the tuned arm is now a mechanical replay of this
chain: convert the r2 adapter, boot with `--lora --decision-seqs 24`,
then `run_jevbench.py --arm fork_4b --port <p>`.

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
   `8135922b…6632f4` (compute host).

   **Score gap-fill measured (score-v1, 2026-10-06).** 96,000 prompts
   (16,000 merged-v2 states × 2 rubric families × 3 votes, same E2B
   donor), 98.1 % parse (424 first-attempt JSON errors all recovered
   by re-ask). Over 30,246 complete (state, family) sets: **27,157
   hard labels (84.9 %)**, unanimous 3/3 only — 3,089 splits counted,
   never smoothed. Hard-label level distribution L0 26,027 / L1 139 /
   L2 576 / L3 415 — the L0 skew is the rubrics' honest read of these
   states and is reported as measured, not rebalanced. Report of
   record: `benchmarks/decision-model/results/score-v1.gap-report.md`.

   **Choice gap-fill measured (choice-v1, 2026-10-06).** 48,000
   prompts (16,000 jev-distill states × 3 votes). Each question is
   candidate-conditioned relevance: the state's own true query (gold
   by corpus pairing — nothing invented) plus three deterministic
   prime-stride distractor queries from other records; the true
   candidate's position rotates per vote, and unanimity is required
   on the mapped QUERY, not the letter — a position-biased donor
   cannot pass. 47,895/48,000 parseable (99.8 %; the donor run's
   parse_status column was invalidated by a lane-parser case bug —
   extract_local lowercased verdict values so the A–D space failed
   membership — and was recovered offline from the kept raw text via
   `runner/repair_parse_status.py`; lane fixed + regression-tested in
   the VIVERE tree). Over 15,898 complete sets: **12,074 hard labels
   (75.5 %)** — 3,266 splits and 558 unanimous-wrong counted, never
   smoothed. Rotation-robustness readout: hard-label yield is flat
   across true positions (A 73.6 %, B 80.7 %, C 74.9 %, D 72.7 %) and
   the donor's letter distribution is spread (D 14,018 … C 10,086) —
   no degenerate position lock. Report of record:
   `benchmarks/decision-model/results/choice-v1.gap-report.md`.

   **merged-v3 pinned (2026-10-06).** 173,452 records = merged-v2
   train (105,284) + recovered (28,937) + score (27,157) + choice
   (12,074), emitted by `runner/emit_merged_v3.py` (all four inputs,
   manifest `opencodifier.distill-manifest/3`), SHA-256
   `7e86c550…847b4f` (compute host). Question mix: noul 134,581 /
   score 29,557 / choice 14,114 — non-relevance coverage 4.2 % →
   **25.2 %**. Gates all zero: recovered-contract violations 0,
   score/choice contract violations 0, recovered (state, query)
   duplicates of train 0, train/suite state collisions 0, suite rows
   unchanged (240). This is the E1 training corpus (item 4).

### 9.5 E1 staging complete (2026-10-06) — prep, trainer, eval, smoke

The full train→serve loop is staged and every leg is executed, not
described:

- **SFT prep** (`runner/decision_sft_prep.py`, commit c7e7964):
  merged-v3 → 178,012 segmented verdict-slot rows + 240 suite rows
  excluded = exactly all 178,252 corpus questions (choice 13,874 /
  noul 134,581 / score 29,557), sha256 `4750dd8d…7253f6a`. The render
  is byte-matched to the serving adapter's layout
  (macjev-render-v1); structured states serialize with `json.dumps`
  exactly as `run_jevbench.py` does.
- **Trainer** (`runner/e1_train.py`, commit 4f5c268): LoRA r16 /
  alpha 32 / dropout 0.05 on q,k,v,o; loss on verdict tokens + EOS
  only, context masked -100. Two bugs the smoke caught, both fixed:
  the model was never moved to GPU (trained on CPU silently), and
  the fp16 path (Volta plan) had no GradScaler — unscaled fp16
  gradients underflow to zero and the run trains nothing.
- **Smoke run** (fedora, Qwen3.5-0.8B body, 400 rows, 1 epoch,
  bf16, ~2 min on GPU): loss 1.2002 → 0.7304 → 0.6321 over 50
  steps, adapter + `opencodifier.e1-train/1` manifest written, data
  sha matches the pinned corpus.
- **Eval hookup** (`runner/e1_eval.py`, commit 4f5c268, readout
  `oc-readout-v1`): render with verdicts absent, one forward pass,
  logit(" yes") − logit(" no") at each ` ->` slot, softmax over
  slots. Suite run, 120/120 items scored, zero slot-position
  failures: **untuned base 0.3333** (chance ≈ 0.20–0.25 for 4–6
  options — the readout interface is learned, not innate, per D16),
  **smoke adapter 0.3583** (+2.5 pp from 400 rows; per-class swings
  at n=40 are noise). This is the plumbing proof; the pre-registered
  E1 gates below are what the real arms must clear.

**E1 arms — pre-registered gates** (registered before any real arm
runs): each arm must beat BOTH the untuned base AND the best
community checkpoint through this same readout/serving path — suite
≥ 0.68 AND JevBench ≥ 0.6494 (Jev-Style-0.8B). Larger arms (2B/4B)
additionally beat the best same-size arm on composite. Anchor: Qwen
4B untuned 76.6 % → ~80.5 % tuned (community-measured). The fusion
ladder 0.867 is an ensemble ceiling, never a single-head promise.

Non-goals carried from §8: no non-Apache-2.0 corpora, no cloud
training by default; the burst node (T5500 2×V100) or Kaggle/Modal
remain the GPU paths, user-gated.

### 9.6 E1 r2 executed (2026-10-06 night chain) — GATE_FAIL, recorded

The night chain ran to completion after the real arm finished
(fedora, `logs/e1-night-chain.log`): trainer exit → gate eval →
comparator arm → gate check. Every number below is from the artifacts.

- **Training** (`runs/e1-a-qwen08b-r2`): 178,012 rows, 11,125 steps,
  1 epoch, bf16 LoRA r16, final loss **0.0517** — plateaued (lr→0 by
  schedule, loss flat over the last ~200 steps). 86/178,012 rows
  truncated at max_len. Manifest `opencodifier.e1-train/1`, data sha
  matches the pinned corpus. The arm is trained, not undertrained.
- **Gate eval** (`results/e1-readout-q08b-r2`, suite sha
  `9f0afaf6…`, 120/120 scored): **0.5667 vs the pre-registered 0.68
  gate — GATE_FAIL.** Per-class: lexical_semantic 0.70, metadata_match
  0.575, relational_compositional **0.425** (untuned base: 0.3333 at
  0.25/0.40/0.35). +23.3 pp over base, but the gain is carried almost
  entirely by lexical + metadata; relational is near-flat vs base.
- **Comparator arm** (`results/e1-readout-qwen38-08b`): community
  Qwen3.8-0.8B-Agentic-distill through the same readout: **0.35** ≈
  untuned base. Nothing transfers to this readout without training on
  it — sets expectations for the #125 imajev-2B probe the same way.
- **JevBench leg correctly NOT run** (`e1-jevbench-leg-r2.log`:
  `GATE_FAIL: suite 0.5667 < 0.68 — JevBench leg correctly NOT run
  (pre-registered)`). No benchmark number may enter the record from a
  sub-gate arm (release policy, HF gate).

Diagnosis — measured, not guessed (corpus `e1-sft-v1.rows.jsonl`,
178,012 rows): **74.6 % of the corpus is one question family** —
`relevance` noul rows from `SargeDev/jev-distill-corpus` (132,781),
vs choice 13,874 (7.8 %) and score 29,557 (16.6 %). Within the score
rows, **89 % carry label `0`** (26,329/29,557) — a near-constant
target teaches nothing. The suite's relational_compositional class
has no training analog at all, which is exactly where the arm stayed
at base level. Loss 0.0517 says the mix was fit; it does not say the
mix was informative for the suite's shape.

Next levers, in order (each cheap relative to a re-run):
1. **r3 = rebalanced mix, same trainer**: cap the relevance family
   (target ≤ 40 %), diversify templates inside every family, and
   repair the score-label degeneracy (audit whether the vivere rows
   are genuinely zero-heavy or the prep mapping collapsed them).
   Relational-compositional training rows may be generated from the
   engine's relational solver as exact-proof-verified items — with
   the standing guardrail that no suite item (all 240) or a
   re-rendering of one may enter the corpus (same exclusion rule the
   prep already enforces).
2. **Capacity, only if r3 still misses relational**: the 2 B tier
   (§7 E3) — the Jev-Style reference shows 2 B ≈ 0.725 is attainable,
   and #125's imajev-2B arm feeds the same question.
3. **Not levers**: more epochs (loss plateaued at 0.0517 — more
   training memorizes a skewed mix), readout redesign (F26: the same
   verdict-slot readout reaches 0.8083 on Jev-Style weights — the
   interface is fine, the data mix is not).

Post-hoc confidence read of the r2 rows (`rows.jsonl`, no re-run
needed — margins and ECE are recoverable from the recorded slot
scores): lexical top-prob 0.813 / margin 0.660; metadata 0.525 /
0.246; relational **0.502 / 0.226 — near-uniform**. Wrong answers
carry mean margin 0.237 vs 0.485 for correct. Two consequences: (a)
the failure is absent knowledge, not readout noise — and the model
*knows where it doesn't know*, so even this arm is ladder-usable as
an escalation trigger (low margin → next rung), which is the
runtime's own thesis; (b) calibration is decent (ECE 0.10–0.13 per
class) — preserve it: any contrastive/weighting fix must re-check
ECE, not just accuracy.

### 9.7 r3 protocol (2026-10-07) — systematic plan from the audit

Code audit of `e1_train.py` / `decision_sft_prep.py` / `e1_eval.py`
plus the r2 artifacts produced the findings; this section is the
work-through order. Sound and unchanged: seeded shuffle, verdict-only
masking, EOS supervision, fp16 GradScaler, clip 1.0, suite exclusion,
train/eval tokenization identity (Qwen adds no BOS — verified).
Gaps, mapped to phases: cumulative-mean loss logging (A1), no val
split or checkpoint selection (A2/A3), silent truncated rows (A4),
`--limit-rows` first-N skew (A5), unrecorded optimizer/library
versions (A3), equal-weight micro-batch loss averaging (A4),
family-skewed mix with degenerate score labels and zero relational
analog (B), padding waste + eager attention (A7, perf pilots).

**Phase A — instrument before training again.** Trainer v2 + eval v2;
pure code, no GPU needed beyond a smoke.
- A1 Windowed loss (last ~100 micro-batches), grad-norm, tokens/s,
  LR — the cumulative mean stays in the manifest only.
- A2 Stratified val slice held out from the CORPUS families (never
  the suite): ~2k rows, per-family val loss every 500 steps.
- A3 Save best-val AND last adapter; manifest records weight_decay,
  betas, torch/transformers/peft versions.
- A4 Loud-skip truncated rows at encode (count, never silently
  train nothing); loss accumulated token-count-weighted.
- A5 Shuffle (seeded) BEFORE `--limit-rows`; tokenized-feature cache
  keyed by data sha (arrow/npz) so pilots skip re-tokenization.
- A6 eval v2: margin, entropy, top-prob, ECE per class into
  `summary.json` (proved recoverable post-hoc on r2 — bake it in).
- A7 One `torch.profiler` pass on a pilot; apply the cheap wins it
  names (predicted: sdpa attention, length-bucketed batching).

**Phase B — data v2 (`e1-sft-v2`), the root-cause fix.**
- B1 Rebalance: relevance family capped ≤ 40 %; template-diversity
  audit per family (prefix census in the manifest).
- B2 Score-degeneracy repair: audit vivere score rows vs the prep
  mapping; degenerate (single-label) rows down-weighted, counted in
  the manifest.
- B3 Engine-generated relational items (Rust, workspace crate): the
  relational solver emits dependency-chain restoration items — the
  exact suite shape — each with exact-proof-verified gold AND
  graph-plausible provably-wrong distractors (hard negatives from
  the same fact graph). Guardrail in code: no suite item, prefix,
  or near-dup render may enter the corpus (suite sha list embedded;
  collision count asserted 0).
- B4 Contrastive minimal pairs: one-fact corruptions that flip the
  gold verdict (provably both sides); target ~1:1
  negative-enriched:plain ratio on relational rows, ≤ 1:3 overall.
- B5 Family weighting knobs in the trainer (per-family loss weight
  or controlled resampling — one knob, manifest-recorded).
- B6 Corpus contract gates extended in the prep manifest: family
  share caps, label-entropy floor per family, pair counts,
  suite-collision = 0.

**Phase C — pilot campaign (30–45 min each on the 5060 Ti).**
Pilots are 20–40 k rows through the instrumented trainer; the suite
is scored only as the pre-registered judge (never for selection —
per-family VAL loss selects).
- P0 current mix, instrumented — validates the instrumentation
  itself (windowed vs cumulative, val curve shape).
- P1 rebalanced mix (B1+B2+B5) — rebalance effect alone.
- P2 rebalanced + generated relational + hard negatives (B3+B4) —
  the full fix candidate.
- P3 ablation: P2 minus negatives — isolates the contrastive
  contribution (the weighted-theory question, answered with a
  number).
- P4 capacity/config: MLP LoRA targets, r32, sdpa — only if P2
  still moves relational weakly.
- Gate to Phase D: pilot P2 relational ≥ 0.60 on a pilot-suite
  slice with ECE ≤ 0.15, val relational curve still descending at
  end. Otherwise iterate B, not D.

**Phase D — full r3.** 178k-class v2 corpus, 1 epoch (2 only if the
val curve is still descending at end — r2's "plateau" was a logging
artifact, so this decision is now evidence-based). Pre-registered
gates unchanged: suite ≥ 0.68 AND JevBench ≥ 0.6494 (beats base AND
best community checkpoint); margins/ECE recorded post-hoc. Gate
holds → JevBench leg (§7 E0 mechanics). Gate misses → E.

**Phase E — escalation: 2 B tier on the T5500 (2×V100, fp16 +
GradScaler — path already in the trainer).** Same protocol, same
gates at the 2 B pre-registration (§7 E3: ≥ 0.75). Staged-bundle
rule stands: data + pilot protocol + eval hookup validated BEFORE
the node powers on, so powered-on time is pure training.

**Phase F — Rust contributions (parallel track).**
- F1 The relational/contrastive item generator lives in the
  workspace (deterministic, exact proofs, unit-tested in CI) — it
  is B3's implementation home, not a loose python script.
- F2 Shared byte-exact macjev renderer in the workspace: python prep
  and Rust serving adapter stop being two hand-synced
  implementations; property test renders a corpus sample through
  both and asserts equality.
  **Shipped (2026-10-07):**
  `opencodifier_schema::macjev` (`macjev-render-v1`) is now the single
  workspace-owned definition — order-preserving JSON DOM, python
  `strip()` whitespace class, python `json.dumps` object spacing,
  typed skips mirroring the prep's count buckets. Parity lane:
  `tests/macjev_parity.rs` pins a real merged-v3 corpus sample
  against python-derived expected bytes in plain CI; `just
  check-macjev` re-derives from the live python implementation on the
  host lane (prep changes must re-run it). Documented boundary:
  float rendering in structured states (Rust shortest-form vs python
  repr exponents) — never occurs in corpora; non-string
  criteria/instructions are typed skips where python would `str()`-
  coerce.
- F3 Profiler-informed Python config wins (sdpa, bucketing) stay
  Python — Rust goes where determinism and proof live.

**Phase G — research integration.** Three standing research tracks
(small-LM LoRA knobs; hard negatives/contrastive/calibration; perf +
instrumentation) run in parallel; their findings fold into the A/B
knob manifests and cite sources in this file's Sources section.

Standing gates carried: no suite item in training data (all 240,
checked mechanically); no non-Apache-2.0 corpora; ECE re-checked on
every arm (accuracy alone is not acceptance).

#### 9.7.1 E1-C pilot campaign executed (2026-10-07) — GATE FAIL, iterate B

All six arms ran on the 5060 Ti in one chained night run
(`e1c-chain`, ~30 min each). Suite = 120 rows (40 per class);
relational column is the §9.7 Phase-C gate metric.

| arm | mix | lr | suite acc | ECE | relational |
|---|---|---|---|---|---|
| p0-lr1e4 | r2 mix, instrumented | 1e-4 | 0.5417 | 0.167 | 0.300 |
| p1-lr1e4 | +rebalance (B1/B2/B5) | 1e-4 | 0.6000 | 0.131 | 0.275 |
| **p2-lr1e4** | +gen-relational+hard negs | 1e-4 | **0.6583** | 0.170 | **0.550** |
| p3-lr1e4 | P2 minus negatives | 1e-4 | 0.5583 | 0.278 | 0.400 |
| p2-lr5e5 | P2 | 5e-5 | 0.6167 | 0.243 | 0.375 |
| p2-lr2e4 | P2 | 2e-4 | 0.6417 | 0.160 | 0.475 |

**Gate verdict: FAIL on all three criteria for every P2 arm** —
relational < 0.60, ECE > 0.15, per-class val curves absent from the
manifests. Phase D does not open; iterate B.

Lever readings, in order of confidence:

1. **Hard negatives + generated relational items are the relational
   lever.** P2 doubled P1's relational accuracy (0.275 → 0.550) and
   the ablation isolates the contribution: P3 (no negatives) sits at
   0.400. B3+B4 stay; the volume knob is what moves next.
2. **lr 1e-4 is the sweet spot.** 5e-5 under-trains the relational
   slice (0.375); 2e-4 produces the best training loss but worse
   relational accuracy than 1e-4 (0.475 vs 0.550) — early overfit,
   consistent with r2.
3. **Rebalance alone does not touch relational** (P1 0.275 ≈ P0
   0.300); it buys general suite accuracy (0.60) and the best ECE
   (0.131). Mix effects are separable, as the phase design assumed.

**Temperature probe — ECE is a training problem, not a calibration
problem.** Post-hoc temperature on the top-2 pair
(p1' = p1^(1/T)/(p1^(1/T)+p2^(1/T)), T fit on 20 relational rows
minimizing 10-bin ECE, evaluated on the held 20): the fitted T pins
at the search boundary (9.0–9.9 of a 0.1–9.9 grid) for the strong
arms — the optimizer wants maximal flattening, i.e. it cannot find a
temperature that keeps confidence on right answers while fixing the
wrong ones. The slice's emblematic failure is p = 0.9585 on a wrong
prediction (hC-0000, `catalog` vs `billing`); mean top-prob 0.779
against 0.550 accuracy. The only arms where temperature "rescues"
held-out ECE to ≤ 0.15 (p3, p2-lr5e5: 0.340→0.090, 0.305→0.090) are
the near-chance ones — that is calibrated *ignorance* (the gate
abstains everything), not calibrated knowledge. Held-half swings
(±0.15 on 20 rows, sometimes worse after fitting: p2-lr2e4
0.100→0.150) also settle a secondary point: 40 relational eval rows
cannot support per-class temperature fitting. The existing D15 /
ladder machinery remains the right home for calibration, but it gets
engaged only after relational *accuracy* rises — more B3/B4 volume
is the fix, on the eval slice as well as the corpus.

**Instrumentation gap to close in the next pilot round:** per-family
val curves (A2) were not present in the manifests, so the third gate
criterion ("val relational curve still descending") was unjudgeable.
Trainer v2 already computes them; the manifest writer must persist
them. Fix before any further arm runs.

Iterate-B actions, in order: (1) scale B3/B4 relational volume in
the corpus (target ≥ 3× the pilot share) and regenerate the eval
slice at ≥ 200 relational rows so ECE and gates are estimable;
(2) persist per-family val curves to the manifest (A2 writer fix);
(3) hold lr at 1e-4, drop the lr axis from future arms; (4) P4
capacity knobs stay deferred until the volume lever saturates.

**Iterate-B repo edits landed (2026-10-08).** Actions (1) and (2)
are in the tree before any new arm runs:

- **Trainer v3 (`opencodifier.e1-train/3`)**: every `run_val` call
  appends `{step, loss, tok_acc, entropy, per_family}` to
  `manifest["val_history"]` — the third gate criterion is now
  judgeable from the manifest alone. The `VAL_LINE` log-scrape
  fallback in `e1c_pilot_table.py` remains for v2 manifests; the
  table prefers the manifest.
- **`--holdout-rel200` suite slice** (`suite_holdout_rel200.json`,
  seed 20261008, suite_version 1, ids prefixed `r`): 280 items =
  40 metadata + 40 lexical + **200 relational**, ids disjoint from
  suite/suite_holdout. `build_suite` gained a `class_c_count` param;
  the default path reproduces suite.json and suite_holdout.json
  byte-for-byte (verified pre-commit). Corpus share target for the
  matching itemgen build: ~26% relational (3.0× the p2 share).

#### 9.7.2 Gate reader defect found and fixed (2026-10-08) — rel-acc verdict revised

The rel200 rows exposed a failure shape the pilot table could not
explain: accuracy by gold slot position 0.435 / **0.055** / 0.413 /
0.711 / 1.000, mean slot logit climbing monotonically −1.13 → +4.28,
105/200 predictions piling onto slot 3 — and wrong-prob mean 0.7516 ≈
right-prob mean 0.7563 (confidently wrong under this reader).

**Root cause: the oc-readout-v1 anchor was one token past the trained
context.** The SFT stream (`decision_sft_prep.segments_for`) renders
`"{opt} ->"` then the verdict `" yes"`/`" no"` as the very next tokens —
the newline comes *after* the verdict, never between `"->"` and it.
`e1_eval.py` rendered slot lines joined by `"\n"` and anchored each
readout at the end of `" ->\n"` (`SLOT_MARKER` included the newline), so
every slot ≥ 2 was scored under a bigram (`"->\n"`) that never occurs in
training. The corpus was never the problem (itemgen items match the eval
template) and volume was never falsified — the reader was.

**Proof (three-mode probe, p2b-lr1e4 adapter-best, same 200 rows):**
re-anchored at `" ->"` — the position whose logits predict the verdict in
training — allslot reads **0.61**, the training-faithful causal
interleaving **0.62**, per-candidate slot0 **0.63**, with a flat position
profile (pos1 0.5455–0.5636 vs the collapsed 0.055). Reader mode is a
±0.02 effect; the anchor was the whole defect.

**Canonical re-run with the fixed `e1_eval.py`** (committed alongside
this section; output `runs/e1c-p2b-lr1e4/eval-rel200-fixed-best`,
full 280-item slice): overall acc **0.6679**, ECE **0.2411**;
per class — metadata 0.900, lexical 0.725, **relational 0.610 @ n=200**
(matches the probe's allslot exactly, validating the probe), relational
mean top-prob **0.9167** against 0.610 accuracy.

**Revised iterate-B verdict:**

- The §9.7 Phase-C *accuracy* gate (rel ≥ 0.60) **passes marginally at
  0.610**; the 0.41 FAIL that motivated "volume falsified" was a reader
  artifact.
- The *calibration* gate **still fails decisively** (ECE 0.2411 > 0.15;
  relational top-prob 0.917 vs acc 0.610). Confidently-wrong remains the
  failure mode, consistent with the §9.7.1 temperature probe: this is a
  training-side problem, and B3/B4 volume stays the lever.
- Per-family val curves remain unjudgeable for the pilot arms
  (trainer v2 manifests); trainer v3 fixes this for iterate-B arms.
- **Scope:** every class accuracy and ECE e1_eval has ever reported is
  suspect by the same off-by-one until re-read (relative arm ordering
  may shift). The llama.cpp fork's native verdict-slot readout — the
  board numbers (F26) — was validated separately and is unaffected.
  Iterate-B arms run exclusively on the fixed reader, which re-baselines
  the program; the six pilot arms are not re-read.

### 9.8 Research integration (2026-10-07) — Phase G findings, dispositioned

Three research tracks ran against the §9.7 plan before Phase B/C
started. Adopted items are folded into the B/C knob manifests; the
rest are recorded as rejected-with-reason so later phases do not
re-litigate them.

**Perf + instrumentation (track 3) — mostly adopted into trainer v2.**
- FlashAttention-2 is Ampere+ only; NOT available on the T5500's
  Volta V100s. PyTorch SDPA silently dispatches to the
  memory-efficient CUTLASS backend below Ampere — functional and
  fused, and the correct target for both hosts. Adopted: `--attn
  sdpa` is now the trainer default with loud eager fallback (v2
  A7); eager remains the r2-parity arm.
- fp16 + GradScaler is mandatory on Volta (bf16 architecturally
  absent, cc≥8.0 required). Already in the trainer; T5500 plan
  confirmed fp16.
- Plain DDP for 2×V100 at 0.8–2 B + LoRA; no FSDP/device_mesh. No
  torch.compile/Triton on Volta. `output_attentions=True` silently
  forces eager — never set it in this program.
- Gradient checkpointing is an inherited TRL default that costs
  ~20 % throughput; measure `max_memory_allocated()` and disable
  where peak stays under ~70 % VRAM. Trainer v2 exposes
  `--no-grad-checkpoint`; the decision is per-pilot, measured.
- `pad_to_multiple_of=8` for fp16 tensor-core tile width — already
  the PadCollate behavior.
- Length-grouped batching: 1.5–2.5× throughput on high-variance
  corpora but a documented eval-loss degradation risk (loss spikes
  at megabatch boundaries; HF forum report). Adopted as
  `--length-bucket`, DEFAULT OFF, and any use must A/B against
  shuffle order on val loss — accuracy is judged only on eval loss
  plus the suite, never on throughput.
- TRL/Axolotl log schema adopted: windowed loss, pre-clip grad-norm
  (alarm band 0.1–10), LR, tokens/s, entropy, memory peak,
  per-family eval loss — trainer v2 emits all of these.
- **Measured addition from smoke v2d (not from research):** the
  dominant OOM at batch 4 × 2048 on 16 GB was the full-sequence
  logits + grad pair (~4.9 GiB) — the loss only ever reads
  supervised verdict positions, so trainer v2 now computes trunk
  hidden states, gathers supervised positions, and projects only
  those through lm_head (exact same CE, megabyte-scale logits).
  Apply this pattern unchanged on the T5500.
- **Measured addition (host):** the hybrid conv+recurrent Qwen3.5
  arch requires `causal_conv1d` + `flash-linear-attention` for
  fused kernels; r2 trained entirely on the slow reference
  fallbacks without anyone noticing. Verify the fused import in
  every new environment before judging throughput.

**Hard negatives / data mix (track 2) — adopted into Phase B/C.**
- Kabra et al. (ICLR 2026): SFT with plain CE on 100 % synthetic
  multi-hop data raised in-distribution accuracy 0.024 → 0.774 but
  FAILED to transfer to HotpotQA, while RL on the same data
  transferred. Generator structural diversity (≥ 2 independent
  template families) is the actual transfer fix; hard negatives are
  the shortcut-killer, not the reasoning teacher. Adopted: B3
  requires ≥ 2 independent relational template families (not one
  templated shape), and the P2-vs-P3 ablation is interpreted
  against this — if negatives lift in-distribution but the suite
  does not move, the generator is too narrow, not the negatives too
  few.
- Solver-verified negatives only — never render an unverified
  negative (RocketQA denoising; Zhan et al.: ~10 negatives optimal
  in retrieval, deeper sampling hurts). Adopted: B3 negatives are
  exact-proof-verified wrong answers over the same fact graph;
  ≥ 3 near-misses per ≥ 5 options; hard:positive 1:2–1:4.
- ANLI-style model-relative re-mining across 2–3 rounds; track the
  mined-negative survival rate as the primary early
  memorization-vs-generalization signal. Adopted as a P2/P3 metric.
- In-domain val margin/ECE CANNOT distinguish shortcut from
  rule-learning — need a held-out generator probe set plus an
  AFLITE check (drop anything a weak lexical baseline solves;
  > 70 % baseline accuracy = lexically separable negatives).
  Adopted: B3 emits a held-out generator probe; the prep manifest
  gains an AFLITE-style lexical-solvability number per family.
- Temperature sampling p ∝ N^(1/T), T=2.5, 40 % per-family cap,
  4× oversampling cap — adopted for B1 rebalancing. Minority-cell
  manufacture from the symbolic generator (not class weights) for
  the 89 %-constant score family — adopted into B2 (target ≥ 35 %
  non-constant score rows... measured before committing the number).
- DPO/ORPO rejected for this program (calibration tax survives
  post-hoc recalibration) — CE + optional label smoothing ε=0.05–0.1
  over wrong slots is the contrastive mechanism if P3 shows
  negatives need a sharper loss than plain CE.
- Single-softmax-over-slots head re-confirmed as the
  calibration-correct form — oc-readout-v1 already is this.

**LoRA knobs (track 1) — adopted as pilot arms, mostly rejected as
defaults.**
- r=16/α=32 kept as control; add r=8/α=16 parity arm (α=2r holds
  output scale). r ≥ 32 rejected (rank-collapse evidence: vanilla
  α/r scaling fails to improve beyond low rank — rsLoRA paper;
  full-FT learns 10–100× LoRA rank — Biderman et al. 2405.09673).
- LR is the real lever: the 2026 PEFT re-evaluation (2602.04998)
  found four LoRA variants within 1–2 % of each other once LR is
  tuned, optimal LR differing 10× between variants, and < 30 % of
  surveyed PEFT papers tune LR at all. Adopted: pilot LR grid
  {5e-5, 1e-4 (control), 2e-4} at fixed batch; LoRA+ λ=16 only as
  an optional grid slot. NeFTune rejected (loss sits on 1–2 verdict
  tokens; prompt-embedding noise has almost no loss surface).
  rsLoRA rejected at r=16 (it matters at high rank only).
- Epochs: 1 epoch default stands (repetition ceiling ≈ 4 epochs in
  the literature; 173k rows is not data-limited). A 2-epoch arm
  scored on ECE/Brier + abstention rate (not accuracy) is the
  highest-novelty cheap measurement — the literature has no
  repeated-epoch calibration-drift study for classification SFT.
- Checkpoint selection: select on val LOSS (never accuracy — 2 k
  rows ⇒ ~1.1 pt standard error, so sub-2-point "wins" are
  selection noise); trainer v2 saves best-val AND last — the last
  adapter is the zero-selection-bias reference the val sweep bets
  against, and both get suite-scored before either is called the
  result.

### 9.8.1 Unsloth assessment (2026-10-07, user-directed)

User asked whether Unsloth's claims ("LoRA, full fine-tuning,
pretraining. All 2x faster, 70% less VRAM, no accuracy loss") can be
adopted into our pipeline. Researched the Desktop docs, the GitHub
README, and the requirements page; dispositioned against our two
training hosts (fedora CPU; T5500 2×V100 Volta) and our arch
(Qwen3.5 hybrid Gated-DeltaNet).

**What Unsloth actually is**: a Triton-kernel patch layer over
HuggingFace transformers + TRL (Unsloth Core, pip-installable) plus a
no-code Desktop app. The headline numbers are measured against stock
HF+TRL with full-sequence logits, no packing, no fused kernels.

**Convergent findings — we already independently landed the two big
memory levers**: (a) their fused cross-entropy does not materialize
full-sequence logits — our supervised-position CE (v2e) is the same
mechanism, measured 13 GiB → 3.03 GB peak on the 0.8 B; (b) fp16 +
GradScaler on Volta is already our posture (§9.8 perf findings). The
"70 % less VRAM" baseline includes losses we already took.

**Adopt (measured, per-lever, into the pilot arms)**:
1. *Length bucketing / padding-free batching* — our rows are short
   and variable; padding waste is real compute. Plain
   group-by-length bucketing is Volta-safe (no Triton needed).
2. *8-bit optimizer (bnb adamw_8bit)* — optimizer state is a real
   VRAM chunk on the 2 B tier; sm_70-supported.
3. *All-module LoRA targets* (q,k,v,o,gate,up,down) — their ablation
   claim; one pilot arm against our current target set costs nothing.
4. *QLoRA NF4 arm for the 2 B tier* — weights 2 B fp16 ≈ 4 GB →
   ~1.3 GB, buys batch/context headroom on 16 GB cards. The "no
   accuracy loss" claim is OUR gate question: the arm must pass
   suite ≥ 0.68 AND JevBench ≥ 0.6494 like everything else; quantized
   base + LoRA changes the loss surface, so it is an arm, not a
   default.

**Reject**:
- *Unsloth Desktop* — a no-code wrapper around the same library; our
  trainer's manifest/trace determinism and supervised-position CE are
  not expressible there.
- *Wholesale adoption for the 0.8 B arm* — 3.03 GB / 16 GB is not a
  VRAM-bound rung; their win over our current posture would be the
  Triton fused-kernel share, which is the Ampere-preferring part.

**Empirical probe (RESULT, 2026-10-07)** — scratch venv on fedora
(`unsloth-probe-venv`, training env untouched; log:
`fedora:~/oc-model-eval/logs/unsloth-probe-20261007.log`):

1. *Import*: PASS — unsloth 2026.10.2 + unsloth_zoo 2026.10.2.
2. *Arch recognition*: PASS — our exact base resolves as
   `model_type=qwen3_5`, `Qwen3_5ForConditionalGeneration`; their
   patcher knows the hybrid class.
3. *Construction*: PASS — `FastLanguageModel.from_pretrained` builds
   our 853 M-param weights on CPU (dtype resolved to bf16 on CPU; on
   Volta the fp16 forcing would come from our trainer, as today).
4. *LoRA forward/backward*: FAIL — **upstream bug in their text
   path**: for this multimodal-aware class their patched tokenizer
   routes plain text through the image processor
   (`prepare_inputs_layout` → treats our training string as a
   base64/URL image candidate and raises). Text-only SFT on
   qwen3_5 is broken in their 2026.10.2 release, independent of our
   stack.

**Verdict**: the arch is supported; the drop-in is not — fighting
their monkey-patched processor inside our deterministic trainer
would buy a rung (0.8 B @ 3.03 GB / 16 GB) that is not VRAM-bound
anyway. Unsloth-the-library is dropped; Unsloth-the-techniques are
adopted directly into `e1_train.py` (the four adopt items above are
all ordinary PyTorch/PEFT features and need no dependency): a
group-by-length bucketing arm, an adamw_8bit arm, an all-module
LoRA-target arm (pilot P-series), and a QLoRA NF4 arm reserved for
the 2 B tier where VRAM actually binds. Every adopted arm passes the
same suite ≥ 0.68 AND JevBench ≥ 0.6494 gates as everything else —
"no accuracy loss" is a claim we measure, never inherit.

Sources added this pass: arXiv:2405.09673 (LoRA rank vs full FT);
arXiv:2312.03732 (rsLoRA); arXiv:2602.04998 + 2602.09492 (PEFT
re-evaluations — LR dominance, batch size first-order); arXiv:
2402.12354 (LoRA+); arXiv:2310.05914 (NeFTune); Thinking Machines
"LoRA Without Regret"; Unsloth LoRA hyperparameters guide; Kabra
et al. ICLR 2026 (synthetic multi-hop SFT vs RL transfer);
RocketQA / Zhan et al. (denoised negatives, ~10 optimal); ANLI
(re-mining protocol); AFLITE (lexical-solvability filter);
NeurIPS 2023 "To Repeat or Not To Repeat" (arXiv:2305.13230);
Muennighoff et al. data-constrained scaling (arXiv:2305.16264);
"Unveiling Over-Memorization in Finetuning LLMs for Reasoning
Tasks" (2025); arXiv:2602.22107 (validation criteria study);
PyTorch SDPA / Volta capability matrix (FA2 Ampere+; no bf16, no
compile, no Triton).

### 9.8.2 #124 quarantine arm B: treatment, knob, build (2026-10-09)

The measured-A/B directive (2026-10-06) executes as a **one-variable
arm B** against the pinned arm A (r2): identical recipe and row order,
plus the merged-v2 quarantine rows arm A actually DROPPED, re-admitted
at `_train_weight = 0.05` through the new `--row-weight-field` knob.

**Pool accounting (why the remainder, not the whole file).** The
quarantine pool holds 38,747 mixed-label cluster members; merged-v3
already re-admitted 28,937 of them at weight 1.0 (qradj-v1). Re-adding
those at 0.05 would up-weight them to an effective 1.05 — a second,
unintended variable. Arm B therefore admits only pool records whose
`record_id` is absent from the arm-A rows; overlaps are skipped and
counted in the build manifest.

**The knob** (`e1_train.py --row-weight-field`): per-row loss weight
read from a jsonl field; the micro-batch loss scales by the mean row
weight (`loss = raw · fw · rw`). **Val loss stays unweighted**, so
checkpoint selection and cross-arm val comparison are unaffected (same
contract as fam_weights). The cache split tag carries the field name
(fam2 → fam3), so the same file read with and without the flag cannot
serve one cache. Flag empty ⇒ every row weighs 1.0 and the loss
op-order is unchanged.

**Build** (`build_quarantine_arm.py`, manifest version
`opencodifier.e1-quarx-build/1`): gates input shas against the pinned
arm-A rows (4750dd8d…) and pool (289e97c3…); segments pool records with
the SAME prep code (`decision_sft_prep`) and skip counters; excludes
pool states colliding with any suite state in the parent corpus (the
v2 gate covered surviving train rows only — the pool was never
suite-checked); asserts arm-A rows pass through untouched and carry no
`_train_weight`. Output `e1-quarx005.rows.jsonl` sha
`13968f047da29169209914b812c64a05b7abb877255be3a294b1b6b63a251d85`.

**Chain** (`e1-quarx.sh`, fedora): stage 0 build → stage 1a
knob-inertness smoke → stage 1b weighted-path smoke (`train_nonunit` >
0 asserted) → stage 2 full train → stage 3 pinned-suite evals
(holdout + rel200, best/last, fixed reader) → stage 4 pre-registered
gate line (suite ≥ 0.68 ⇒ JevBench leg next; < 0.68 ⇒ correctly not
run).

### 9.8.3 Torch-RNG seeding defect caught by the inertness gate (2026-10-09)

The first inertness run **failed correctly**: flag-off vs
field-named-but-absent diverged from the first logged step (step-20 win
0.5247 vs 0.4959; final cum 0.3398 vs 0.3343) with identical LR
schedules and identical seeded data selection. Root cause: `--seed`
governed only the data pipeline (`random.Random` for shuffle/split/
bucketing) — **torch's RNG was never seeded**, so LoRA init (random
Gaussian) and dropout (0.05) drew fresh weights in every process. Every
manifest to date records `seed: 42` that truthfully covered the row
order only. Runs before this fix keep their recorded numbers; the
noise this injects into any same-recipe re-run is now measured: ~1.6 %
final loss at smoke scale (the failed pair itself is the measurement).

Fix: `torch.manual_seed(seed)` (+ `manual_seed_all`) at trainer start,
recorded as `"torch_seeded": true` in the manifest. Re-run result:
**INERT OK — flag-off == field-with-unit-weights, bit-for-bit after
stripping tok/s** (07:12:53Z). One gate now proves two things: the row
weighting is math-inert when unset, and torch-seeded runs are
bit-reproducible end-to-end. Consequence for arm B vs arm A: the init
draw differs (arm A predates seeding) exactly as it would between any
two unseeded runs — the paired suite analysis remains the decision
instrument, and the 1.6 % smoke-scale noise floor is its context.

### 9.8.4 Arm B executed: gate result and paired-analysis caveat (2026-10-09)

The chain restarted 02:16 local (post-reboot) and **completed**: full
train 02:16–09:14 (11,615/11,615 steps, final cum 0.0511, val loss
0.0296, tok_acc 0.9880), evals 09:14–09:19. Manifest checks: corpus
sha `13968f04…` matches the pinned build; `torch_seeded: true` (§9.8.3
fix active); row-weight contract verified live — 9,718 non-unit train
rows, mean 0.9503 (the dropped-pool remainder at 0.05).

**Gate result (pre-registered):** suite holdout **0.65** (best; ECE
0.1512; last identical 0.65, ECE 0.1617) **< 0.68 → GATE_FAIL —
JevBench leg correctly NOT run.** rel200: 0.55 both variants (ECE
0.2066 / 0.1921). Notably, holdout ECE 0.1512 is the closest any arm
has come to the ≤ 0.15 calibration line (E1-C pilot blocker: 0.2411).

**Instrument caveat before any A/B verdict.** Arm A r2's recorded gate
number (0.5667, §9.7) was scored on suite sha `9f0afaf6…` — the
instrument changed when the readout fix + rel200 slice landed (current
shas `6e9b6e0f…` / `afe6770c…`). Comparing 0.65 against 0.5667 across
instruments is invalid. The pre-registered decision instrument is the
**paired suite analysis**: arm A's adapter (still on disk at
`runs/e1-a-qwen08b-r2`) re-scored on the CURRENT suites, same reader,
same base.

**Paired result (arm A `eval-curinst-*`, 12:38–12:39 local): arm B
wins on both slices.**

| slice | arm A (curinst, last) | arm B (last) | Δ |
|---|---|---|---|
| suite_holdout (120) | 0.6333 (ECE 0.0761) | 0.65 (ECE 0.1617) | **+1.7** |
| rel200 (280) | 0.5036 (ECE 0.1246) | 0.55 (ECE 0.1921) | **+4.6** |

Arm A had no `adapter-best` artifact (best-checkpointing postdates r2),
so the comparison is last-vs-last on both sides — and arm B's
best≈last (0.65/0.65, 0.55/0.55) makes it insensitive to checkpoint
choice anyway. The rel200 delta (+4.6, on the weakest slice) is well
above the §9.8.3 noise floor (~1.6 % at smoke scale); the holdout delta
(+1.7) is marginal. **Verdict: re-admitting the dropped quarantine
rows at weight 0.05 is net-positive — the pool carries real signal.**
Design consequence for the merged-v4 corpus (#124): keep pool rows in
at low weight rather than dropping them.

The trade is calibration: arm A ECE 0.0761 vs arm B 0.1512 on holdout.
Arm B buys accuracy and spends calibration — acceptable for the ladder
(post-hoc per-rung calibration is the planned B3/B4 lever), but it
means the pool re-admission is not free. Gate stays **GATE_FAIL**
either way — 0.65 < 0.68, JevBench leg correctly withheld.

### 9.9 E1-E staged bundle for the T5500 (2026-10-09)

Per the staged-bundle rule: **data + pilot protocol + eval hookup
validated BEFORE the node powers on**, so powered-on time is pure
training. Bundle: `/nas/Temp/work/oc-model-eval/t5500-bundle/`
(transfer hub; self-contained — data, suites, reader, trainer, base
model, integrity manifest).

- **Arm**: Qwen3.5-2B, e1-iterb corpus (177,999 rows — the exact 0.8B
  iterate-B corpus, so capacity is the only variable), r2 recipe knobs
  otherwise (lr 1e-4, LoRA r16/α32/drop 0.05 q,k,v,o, batch 2 × accum
  8, max-len 2048, warmup 0.03, seed 42, 1 epoch), **fp16 + GradScaler
  (Volta has no bf16; scaler path pre-existing and smoke-exercised),
  sdpa default** (CUTLASS mem-efficient below Ampere; FA2 is Ampere+
  only), grad checkpointing on.
- **stage0_env.sh**: bundle sha verification → venv → torch from the
  cu126 channel → **Volta probe** (CUDA 13 dropped sm_70; a cu13x wheel
  fails loudly with the fix in the error text) → 400-row fp16 GPU smoke
  (NaN gate, steps > 0, peak-memory + attn recorded from the manifest).
- **train_e1e_2b.sh**: 2k smoke → full train (~11k steps, 8–16 h
  single-GPU estimate) → holdout + rel200 evals (best/last, fixed
  reader) → pre-registered **2B gate ≥ 0.75** (§7 E3). GATE_PASS →
  JevBench leg next (wired after review); GATE_FAIL → correctly not
  run; no benchmark number from a sub-gate arm.
- **Honest staging state**: pipeline end-to-end validated on the
  staging host (0.8B); actual V100 execution, fp16 loss-scale behavior
  at 2B, and wall-clock step time validate at power-on — that is what
  stage 0 is for. **DDP deliberately absent** (single-GPU-first; plain
  DDP only on Volta, no FSDP/device_mesh — §9.8) and recorded as the
  first power-on validation item.

#### 9.9.1 E1-E executed: both arms GATE_PASS (2026-10-11)

- **Execution**: restart after the strata kill — smoke 2k rows
  rc=0 (23:35Z→01:24Z Oct 10), full arm `e1e2b` 01:24:18Z Oct 10 →
  **06:27:08Z Oct 11 rc=0** (29.0 h wall, 11,009 steps, ~9.5 s/step —
  on the stage-0 projection). Replica (seed 7) trained concurrently on
  GPU1, closed 05:59Z.
- **Co-tenant incident (recorded as an ops finding)**: the chain's
  rel200 eval leg OOM'd — `t5500-llama.service` (a Qwen3.8-27B
  llama-server deploy, 14.16 GiB on GPU0) started at 06:28:45Z, two
  seconds into the leg, and the chain driver died before the stage-4
  GATE line. No process was killed; the remaining legs (gate rel200 +
  all four replica evals) were completed manually on the free GPU1 via
  `e1e-finish.sh` — same eval commands, same suites, same stage-4 gate
  rule; artifacts `runs/*/eval-*`, log `logs/e1e-finish.log`. T5500
  effectively has one usable GPU while that unit holds GPU0.
- **Results** (fixed reader; acc / ECE):

  | arm | holdout best | holdout last | rel200 best | rel200 last |
  |---|---|---|---|---|
  | gate s42 | **0.8083** / 0.179 | 0.8083 / 0.176 | 0.7321 / 0.262 | 0.7321 / 0.261 |
  | replica s7 | **0.8000** / 0.172 | 0.8083 / 0.174 | 0.7179 / 0.259 | 0.7250 / 0.252 |

- **Pre-registered gate (§7 E3: ≥ 0.75 holdout-best)**:
  **GATE_PASS × 2** (0.8083, 0.8000). Seed delta 0.0083 — replication
  holds; on the replica the last adapter matches the best (0.8083), so
  the endpoint is not a best-checkpoint artifact.
- **rel200 is the binding slice**: 0.718–0.732 across all four
  variants vs 0.80–0.81 holdout — the relational/compositional
  weakness persists at 2B (0.8B lane showed relational val 0.0),
  capacity alone did not close it. Candidate lever for the soft-label
  v2 arm (#143): relational-heavy data, not more parameters.
- **JevBench 231 leg (GATE_PASS precondition met; executed 2026-10-11)**:
  adapter-best, fp16, T5500 GPU1, harness `9ec6f15a` (same as every
  Board A row). **Accuracy 0.6840** (158/231), family-macro 0.6832,
  raw ECE 0.2067, Brier 0.5035, p50 0.58 s, replay determinism
  231/231 (labels + probabilities). **Release gate: both legs pass**
  — suite 0.8083 ≥ 0.68 AND JevBench 0.6840 ≥ 0.6494 (the Jev-Style
  0.8B bridge) — the first own-trained arm to clear both. Per-family
  shape matches the rel200 diagnosis: extraction 0.958, routing 1.00,
  tool_selection 1.00, ordinal 1.00 vs multi_hop 0.389, judge_hard
  0.412, long_policy 0.368, temporal_numeric 0.333 — the
  relational/compositional families are exactly where the arm loses
  the board.
- **Calibration fit (D15 fitter, in-sample winner-vs-rest margin)**:
  shipped `results/calibration/e1e-qwen3.5-2b-holdout.json`
  (T=3.362, ECE 0.179 → 0.117, AURC 0.146 unchanged — both artifact
  gates pass) and `e1e-qwen3.5-2b-rel200.json` (T=5.100,
  ECE 0.262 → 0.169). Applied to the external JevBench board
  analysis-only (no test-set fit): ECE 0.2067 → 0.1269. The arm is
  systematically overconfident by ~3.4× in probability — consistent
  with verdict-slot training on hard labels.

### 9.10 v2-0.8B arms executed on fedora: hard control clears the suite
gate, soft-label distillation is a wash (2026-10-12)

Both v2 arms ran to completion on the fedora 5060 Ti (trainer v5 with
resume; sequential hard → soft under `runs/arms-fed.sh`; 11,615 steps
each; hard 4 h 51 m, soft 4 h 35 m; both rc=0, zero resume events
needed). Recipe: E1-E byte-identical (b2×a8, max_len 2048, fp16+sdpa,
LoRA r16/α32/d0.05, lr 1e-4, warmup 0.03, seed 42, val_every 500,
ckpt_every 100) over `corpora/e1-softlabel-v1` (187,822 rows, sha
`2f51072f…`); soft arm adds `--soft-labels --teacher-temp 1` (facts as
recorded). Same-platform A/B satisfied.

**Suite legs** (e1_eval.py, E1-E protocol — readout oc-readout-v1,
defaults bf16+eager exactly as `train_e1e_2b.sh` stage 3; suite shas
pinned in each summary.json; run dirs on NAS
`runs/v2-08b-{hard,soft}-fed/`):

| arm | holdout-120 acc (ECE) | rel200-280 acc (ECE) | val floor (fit) |
|---|---|---|---|
| hard control | **0.7000** (0.1406) | **0.5214** (0.1580) | 0.0292 (= fit L∞ 0.0292) |
| soft-label | 0.6750 (0.1601) | 0.5179 (**0.0923**) | 0.1477 (fit L∞ 0.1541, still descending at 8k) |

**Gate verdicts (pre-registered §7 E3 / §9.12 rules):**
- **Hard control: suite 0.7000 ≥ 0.68 GATE_PASS**, JevBench 231 leg
  executed per protocol (`runs/jevbench-v2-hard.sh`, r2 shape: fork
  server, qat-Q4_0 base + LoRA F16, CPU-only, port 8095, harness
  `fork_4b`, complete=True, operational_success=1.0):
  **accuracy 0.5584 (129/231), macro 0.5458, Brier 0.6304, ECE
  0.1952 — JevBench < 0.6494 GATE_FAIL. Release gate: FAIL overall**
  (needs both legs). The dual gate did its job: the suite leg alone
  would have waved this arm through; the board leg caught the
  transfer gap. Suite→board deltas: v2-0.8B 0.700→0.558 (−14.2),
  E1-E-2B 0.808→0.684 (−12.4), Jev-Style bridge 0.8B row 0.6494 —
  the transfer discount is ~12-14 pp at every rung, and the 0.8B
  starts too low to survive it. The 2B (E1-E) remains the only
  own-trained arm through both legs.
  (Harness note: the leg crashed at the post-scoring provenance
  merge — `ForkAdapter` lacked the `ladder` class attr the fusion
  summary reads; scoring artifacts were already written and valid
  (`complete=true`). Fixed in-repo + patched at the pinned harness
  on fedora; scoring path untouched.)
- **Soft arm: suite 0.6750 < 0.68 GATE_FAIL — JevBench correctly NOT
  run** (arm-B precedent).
- **"Soft must beat hard": FAILED, and honestly a statistical wash,
  not a loss.** Δholdout = 2.5 pp on 120 rows (84 vs 81 correct —
  inside binomial noise at this n); Δrel200 = 0.35 pp on 280 rows
  (146 vs 145). **The distillation premise (teacher distributions >
  hard labels) showed no accuracy gain at 0.8B with T=1, one epoch.**
  What soft DID buy: calibration — rel200 ECE 0.0923 vs hard 0.1580,
  with the expected mechanism (softer confidence: mean top-prob 0.728
  vs 0.877 on holdout). A candidate pivot is soft-as-calibrator, not
  soft-as-accuracy: distill for the confidence profile, take argmax
  accuracy from hard labels.

**Reading the val floors (§12.3 protocol):** hard sat exactly on its
fitted floor at the end (last val 0.0292 = predicted L∞ — the
extrapolator called it from 17 points at step 8500). Soft was still
descending at 8k past its early-curve fit (precrash L∞ 0.1541 from 7
noisy points vs actual 0.1477 at 11,615 — extrapolators predict the
run, gates decide truth, §12.3 rule vindicated).

**Program implications:**
1. The **relational slice stays binding**: hard rel200 0.5214 vs the
   2B's 0.718–0.732 — a +5 pp suite gain over arm B moved rel200 by
   <2 pp. Data shape (relational-heavy rows), not parameters or label
   mode, is the 0.8B lever (§9.9 diagnosis unchanged).
2. **VIVERE soft-label transfer: premise falsified at this config** for
   accuracy. T=1/1-epoch is one point in config space (T sweep + more
   epochs queued in §12.4), but the default posture for 0.8B v3 should
   be hard labels unless the T sweep flips the sign.
3. **Calibration-by-distillation is a real observed effect** (ECE
   table above) and feeds the D15 seam: a soft-trained head's
   probabilities may need less temperature correction than a
   hard-trained head's (hard is systematically overconfident — §9.9
   fit T=3.362 on the 2B).
4. Holdout-120 vs "231 split" naming: the suite gate leg is the pinned
   **120-row holdout** (`suite_holdout.json`, sha in summaries); the
   231-row surface is the JevBench public split (separate leg). Never
   conflate the two when comparing arms.

## 10. Transfer pathways into a small decision model (taxonomy, 2026-10-06)

Recorded because it disciplines which experiment buys what. Four distinct
methods, often conflated:

1. **VIVERE extraction (ours) — labels, not weights.** Teacher judgment
   is captured as verdicts on our states; unanimity-gated hard labels
   become supervised rows for OUR head on OUR architecture. No teacher
   weights at runtime and no student-architecture constraint — the
   student can be anything, including a future pruned model. Cheapest
   per unit of capability moved; proven at scale (merged-v3: three
   donor legs, 173,452 records). Not classical distillation (no
   soft-logit transfer, no teacher–student coupling); closest family is
   label/synthetic-data distillation.
2. **Weight distillation into an existing small arch** — what the
   empero Qwen3.8 arms are (Qwen3.8 2.4T A95B → Qwen3.5-2B/4B arch).
   Keeps the small arch, moves capability. Measured (r22, REPORT):
   **no letters-lane lift over untuned Qwen3.5 at either size**
   (2B 0.725 = base's 0.725; 4B 0.792 < base's 0.800). Capability
   transfer into an arch does not automatically beat that arch's own
   untuned decision behavior on a typed-decision readout; the transfer
   showed up as emission cleanliness instead (0 invalid dists at 4B —
   the cleanest on the board).
3. **Prune-then-recover** — shrink a larger-arch model to <1–4 B and
   recovery-train. The only path that changes the small model's
   ARCHITECTURE (it inherits the larger family's improvements — the
   Qwen3.8-vs-Qwen3.5 point). Most expensive and least proven for
   decision tasks; bounded by recovery-training compute (full-rigor
   budgets are pretraining-scale; burst-node-feasible only at
   LoRA-recovery depth). Held as option C, triggered when a uniquely
   strong teacher has no small variant AND evidence shows an
   arch-shaped gap the data program cannot close. Middle path if
   triggered: structured-prune an existing 4 B (e.g. the r22 4B) to
   ~1–2 B + LoRA recovery on merged-v4 — tests shrink-large vs
   train-small inside one arch at burst scale.
4. **QAT last** — quantization-aware training is the deployment step on
   whichever compressed model wins, never a substitute for 1–3. Our own
   quant evidence says why: requant provenance dominates bit-width
   (r11b) and QAT-sourced arms hold accuracy at 4 bit where post-hoc
   quants collapse (r13/r15).

Decision rule going forward: for each new capability need, exhaust (1)
first → adopt ready-made (2) arms when they measure well → escalate to
(3) only on a demonstrated arch-shaped gap → always finish with (4) at
deployment.

## 11. Corpus-prep gates for merged-v4 (§15.6 item 9, 2026-10-08)

`runner/corpus_gates.py` is the gate front-end for ingesting
`jev-distill-corpus-v3` (and any future corpus) into a merged-v4 arm:
schema validation, teacher-noise scoring, per-class/per-family caps,
and byte-exact label-fight majority — every row routed to kept or
quarantined **with its reason recorded**, never silently deleted.
Quarantine keeps the row verbatim plus a `_gate` sidecar; kept rows
carry a `_gates` facts sidecar (family, kind, argmax, top, margin);
a manifest pins input SHA-256s, per-gate counts, and the noise
histograms. Gate semantics: caps are per-class/per-family ceilings on
the file (`floor(share × clean rows)`, first-come keep, share ≥ 1
disabled); the noise floor is inclusive (a row at exactly the floor
passes); fights group on byte-exact (state, question, options) and
quarantine the minority label, everything on an exact tie.

### First measurement (train.jsonl, 655,806 rows, permissive defaults)

Ran 2026-10-08; row conservation verified (kept + quarantined = input,
both in the 655,806 smoke run and the 13-row four-gate micro-fixture,
each gate's firing hand-checked):

- **top-prob < 0.5: 81,740 rows (12.5%)** — the default noise gate
  quarantines exactly these. Distribution by decile (d = floor(10·p)):
  d0 0, d1 80, d2 4,464, d3 27,787, d4 49,409 — the mass sits just
  under the floor; only 80 rows are below 0.2 and none below 0.1.
- **margin < 0.1: 207,461 rows (31.6%)** — the bottom margin decile is
  the real ambiguity pool, 2.5× the top-prob pool. The margin gate
  defaults OFF; these histograms are why thresholds must be chosen
  from the measurement, not assumed.
- Zero schema violations, zero label fights, zero cap pressure at
  defaults — corpus-v3 is structurally clean; its noise is purely
  teacher-uncertainty mass.

**Implication for the #124 A/B:** the "38,747-row quarantine pool"
framing came from merged-v2 near-dup analysis; corpus-v3's own noise
pool is a different, measured quantity (81,740 at the 0.5 floor). The
A/B arms should define the pool as `corpus_gates.py` quarantines it at
a recorded threshold — the manifest makes the pool a reproducible
artifact instead of a per-campaign judgement call.

### merged-v4 built and verified (2026-10-09)

**Upstream accounting, measured** (a recon pass had assumed the HF
dataset grew past what the lineage ingested — it did not):
`corpora/src/dataset.jsonl` is byte-identical (sha `7c7a8806…`) to the
file merged-v1 ingested, so upstream never grew. The 148,160 rows
partition exactly: **132,781** survive into merged-v3, **+28,937**
quarantine rows were re-admitted by the qradj-v1 recovery (verified by
record-id intersection, not inferred from counts), **+9,810** remain
quarantined (the remainder), and **+5,569** were deduped at the v2
merge (exact duplicates of retained rows; they stay out — re-adding
them would double-weight identical content).

**Build** (`fedora:~/oc-model-eval/build_merged_v4.py` →
`corpora/merged-v4.jsonl`, manifest `merged-v4.manifest.json`):
merged-v3 (pinned `7e86c550…`, 173,452 records, byte-verbatim) + the
9,810-record quarantine remainder, each marked
`_pool: {origin, quarantine_reason, train_weight: 0.05}` — the §9.8.4
verdict applied at the record level. 183,262 records out; sha
asserted on both inputs; suite state-collision gate reproduced arm B's
build numbers exactly (28,937 recovered-skip, 0 collisions, 9,810
admitted). `_pool.train_weight` is a record-level marker consumed by
row prep (applied per emitted segment row, arm-B precedent);
`decision_sft_prep` itself does not read it.

**Equivalence verification**: `decision_sft_prep` on merged-v4 emits
187,822 rows that are **sequence-identical** to arm B's trained rows
(`e1-quarx005.rows.jsonl`, sha `13968f04…`) after stripping
`_train_weight` — the record-level corpus and the arm that was trained
are two representations of one composition, and the pin is now
checkable rather than asserted.

**Not folded in**: the `corpus_gates.py` noise-pool threshold A/B
(81,740 rows at the 0.5 floor) stays open for a future arm — one
measured change per corpus version. merged-v4's only treatment vs v3
is the A/B-verified pool re-admission. First consumer per §10: the
prune-then-recover middle path (structured-prune the r22 4B to
~1–2 B + LoRA recovery on merged-v4), burst-scale.

## 12. Configuration ledger, epochs, and plateau policy (2026-10-12)

Opened after the operator's plateau question ("if training plateaus do
we 'know' it's good? can accuracy rise again?"). This section is the
audit trail for **which parts of the current recipe are measured and
which are inherited assumptions** — the difference between a tuned
pipeline and a lucky one.

### 12.1 What a plateau means — and what it does not

A flat validation curve means exactly one thing: **this recipe — this
data, this schedule, this loss mode, this rank — has extracted what it
can.** It is a statement about the recipe, not about the arm's
quality, and not about the task's ceiling.

Three rules follow, each with a recorded reason:

1. **Plateau ≠ good.** "Good" is decided only by the held-out gates
   (suite ≥ 0.68, JevBench ≥ 0.6494 — §9.12 release policy). The
   E1-C pilot is the standing counterexample: its val loss plateaued
   beautifully while the reader defect meant accuracy sat *below
   chance* (0.2251). A converged curve will converge to garbage just
   as readily as to a good model. Never read a gate verdict off a
   loss curve.
2. **Within a run, a plateau is flat — accuracy does not rise again
   on its own.** Nothing arrives during training to change the
   outcome: same data, same schedule, same init. Waiting longer on a
   plateaued arm buys noise, not progress. (The cosine tail is the
   one schedule event that still moves late-training loss — see
   rule 3's other side.)
3. **Across recipes, a plateau is a floor for THIS recipe only, and
   it moves under nameable interventions.** Accuracy comes back and
   goes higher when something material changes:
   - **schedule events** — the LR tail (cosine decay → 0), an LR
     restart, or a longer warmup;
   - **capacity** — higher LoRA rank, full-FT, a bigger base;
   - **data** — more or cleaner rows (VIVERE gap-fill, the
     merged-line noise gate A/B), a different mix;
   - **loss mode** — hard-CE → soft-KL distillation is not a
     third recipe on the same curve, it is a different surface
     (hard-CE val ~0.03 vs soft-KL val ~0.16 is definitional, not
     comparable — never compare across loss modes, only within one);
   - **optimization machinery** — bucketing/packing (affects which
     gradients the padding tax leaves on the table), kernel swaps.
   Without one of these, "it will go up if we just keep training" is
   not a hypothesis, it is a wish.

**Epochs are a stopping point, not a property of the data.** The
honest framing: train longer than you expect to need with
`adapter-best` checkpointing on, and *choose* the epoch count on the
observed curve. Because `--ckpt-every` now persists the best
checkpoint (trainer v5, commit c44ad6a), choosing too many epochs
costs only compute, never quality — the wrong-epoch-count failure
mode is retired. Current arms inherit `epochs=1` from E1-E
(**assumed, not measured** — flagged in the ledger below); the
3-epoch × 4k-row pilot (~15 min on the 5060 Ti) is queued to convert
that assumption into a measurement.

### 12.2 Configuration ledger — measured vs assumed vs falsified

Every knob in the v2-0.8B recipe, with its evidentiary status. Anything
marked ASSUMED is inherited (E1-E / Jev-style reference) and untested
in this configuration; marked FALSIFIED means measured and rejected.

| Knob | Status | Evidence |
|---|---|---|
| `batch 2 × accum 8` | **MEASURED fastest** | 7-pilot sweep 2026-10-12: b2×a8 7.51 s/step (V100) vs b4 8.58, b8 10.49, b16 12.19 — monotonic loss to padding waste; VRAM never binding (2–6.3 GiB) |
| throughput on Ada | **MEASURED** | 5060 Ti 1.06 s/step = 7.1× V100; workload is shape/overhead-bound (0.8B only 1.26× faster/step than 2B), not param-bound |
| `fp16 + sdpa` platform rule | **MEASURED** | E1-E trained and evaluated with it (train-manifest `"attn": "sdpa"`); the "eager on t5500" note was stale. Same-platform A/B rule stands |
| grad checkpointing ON | **MEASURED mandatory** | no-ckpt OOMs mid-run even at b4 (hybrid conv+recurrent activations; long-batch peak, not average — average-case VRAM fits lie) |
| `epochs 1` | **MEASURED** (pilot + full-scale) | v2 arms: hard landed exactly on its fitted floor at the budget end (§9.10); soft hit its floor by ~8k of 11,615 steps. Pilot E3 (3 epochs × 4k rows, below): val flat from step ~400 of 741, eval acc 0.6333 vs 0.6583–0.6667 for the 1-epoch arms — epochs 2–3 bought nothing at pilot scale. 1 epoch stands |
| `lr 1e-4` | **MEASURED directional** | pilot L-sweep (below): 1e-4 > 5e-5 > 2e-4 on holdout acc (0.6333 / 0.5917 / 0.5583, E3 vs L1 vs L2). Inherited default wins both directions |
| LoRA `r16 / α32 / dropout 0.05` | **MEASURED directional** | pilot R-sweep: r8a16 0.6667 > r16a32 > r32a64 0.5833 — smaller rank won at 4k rows (overfit direction). r16 stays the default at full scale; r8 is the named candidate for small-data arms |
| soft-label temperature `T` | **MEASURED directional (new row)** | pilot T-sweep: T=0.5 0.6583 acc / **0.068 ECE** (best calibration of the window) vs T=2.0 0.5417 / 0.105 — accuracy and calibration both fall monotonically in T. v2 soft used T=1. Sharpened (T<1) soft targets are the soft-as-calibrator operating point, not softened ones |
| `warmup 0.03` | **ASSUMED** | untested |
| `val_frac 0.01 / cap 2000, val_every 500` | **MEASURED adequate** | val noise ~±0.005 on hard (rmse of fit 0.0050 vs trend 0.037→0.030); 17 points over an arm is enough for floor estimation |
| `--length-bucket` | **FALSIFIED as shipped; fixed forward** | inert defect (flat-shuffle destroyed grouping, d989c9d); all recorded arms ran with it OFF, so nothing shipped is affected. Re-sweep batching AFTER the fix |
| trainer resume | **MEASURED** | v5 parity pilot: SIGKILL mid-val → worst val delta 0.0003 vs uninterrupted; guard refuses fingerprint mismatch (negative control tested) |
| two arms on one GPU / DDP / IO parallelism / torch.compile | **FALSIFIED non-levers** | timesharing adds nothing (not VRAM-bound); DDP changes numerics for no gain; tokenize/cache is seconds; compile = low expected gain + graph breaks |

**The soft-arm step budget finding (first extrapolator catch):** the
pre-crash soft arm's curve fits L∞ = 0.1541 (rmse 0.0019, b = 0.94)
— effectively converged by ~val2500 of 11,615 steps. The E1-E step
budget is ~4× longer than the soft arm needed. Candidate ledger
entries if it reproduces on the fedora soft arm: stop early (adapter-best
already holds the floor), or spend the surplus steps on a lower final
LR rather than more plateau steps.

**Resolution on the fedora soft arm:** partially confirmed — its floor
was lower than the early fit predicted (0.1477 actual vs 0.1541
predicted; the 7-point fit underestimated the descent, §9.10) but it
still reached ~floor by ~8k of 11,615 steps. Net: the soft arm wastes
~30% of its budget; the hard arm uses all of it. Lesson: fits from
few noisy early points UNDERESTIMATE the remaining descent — refit at
each val point (the tool does) and treat the floor as provisional
until two consecutive fits agree.

**The pilot window (2026-10-12 22:11–23:00, fedora 5060 Ti):** seven
4k-row arms (`--limit-rows 4000`, seed 42, b2a8 fp16+sdpa, ckpt/val
every 100), each with a holdout-120 eval of adapter-best. Known
confound: the cosine schedule compresses to 741 steps at this scale,
so arms read against a different LR trajectory than full runs.

| arm | knobs | holdout-120 acc | ECE |
|---|---|---:|---:|
| E3 | 3 epochs (base lr/r) | 0.6333 | 0.1355 |
| L1 | lr 5e-5 | 0.5917 | 0.1074 |
| L2 | lr 2e-4 | 0.5583 | 0.1000 |
| R1 | r8/α16 | **0.6667** | 0.0950 |
| R2 | r32/α64 | 0.5833 | 0.1012 |
| T1 | soft T=0.5 | 0.6583 | **0.0680** |
| T2 | soft T=2.0 | 0.5417 | 0.1050 |

Reads, all directional (n=120 → ±8.5 pp two-σ on any single arm; only
the R1–T2 spread ~12.5 pp even approaches significance): (1) the
inherited defaults win the lr axis in both directions; (2) LoRA
capacity prefers SMALLER at pilot data volume — consistent with
overfitting, and the reason r8 is only a small-data candidate, not a
full-scale change; (3) the epoch answer repeats the §9.10 doctrine —
E3's val was flat from step ~400 (loss 0.1298 → 0.057 by step 400,
then 0.0566–0.0601 for the remaining 341 steps across epochs 2–3) and
its eval acc sits below both 1-epoch arms; (4) the temperature axis is
the one genuinely new measurement: soft T=0.5 matches the best arm's
accuracy while posting the window's best ECE (0.068 vs 0.095–0.136
hard) — the soft-as-calibrator pivot (§9.10) now has an operating
point, T<1 sharpened, to test at full scale. 4k rows underscales the
corpus (full-data hard arm: 0.7000), so absolute values read low; the
pilot ranks knobs, it does not gate anything. Run tree archived:
`runs/pilot-window-1012/` on both hosts.

### 12.3 The extrapolator — `benchmarks/decision-model/runner/arm_curve_fit.py`

Stdlib-only tool that reads a run dir's `train-manifest.json`
`val_history` (record of record) — or, for mid-run dirs, the
`val step=N loss=X` log lines — and fits two plateau forms
(L∞ + a·s^−b and L∞ + a·e^(−bs); grid over b + closed-form 2-var
least squares, negative loss floors rejected). Reports the floor
L∞, projected loss, `plateaued` (<1% improvement on the best of the
last three preceding points), and `val_turning_up` (past the val
minimum — `adapter-best` holds the right checkpoint from here).
`--self-test` runs synthetic-curve assertions (power-law and exp
ground truths recovered within tolerance; flat, upturn, and
insufficient-data flags). `--warn-above X` flags `abort_candidate`
for projected-final above a gate-implying threshold.

Protocol (binding):

- **Review-only, never auto-kill.** The E1-C lesson: every
  extrapolator predicts the run, not the truth. An `abort_candidate`
  flag means a human compares the projection to the gate threshold
  before anything is stopped.
- **Judge floors, not single flags.** On noisy val (hard arm:
  ±0.005 noise vs 0.037→0.030 trend), `plateaued` flickers; the fit's
  L∞ with its rmse is the stable read. `val_turning_up` + a flat
  `plateaued` together = stop gracefully.
- **Refit, don't rescale.** A projection is valid at the last fitted
  step; reaching a distant horizon by rescaling is directional at
  best. The tool says so in its output.
- **First catches (2026-10-12):** soft-precrash floor 0.1541
  converged-by-~2500 (§12.2 ledger finding); live hard-fed floor
  0.0292 with last 0.0298 — on the floor, still nominally improving.

### 12.4 Process-audit queue (work done while arms run)

Standing answer to "can we improve the pipeline while we wait":

1. **Epoch pilot — DONE 2026-10-12** (with the LR and LoRA-r legs folded
   into the same window; see the pilot table in §12.2). Follow-through:
   a full-scale soft T=0.5 arm is the one candidate the pilot names.
2. **LR × LoRA-r mini-sweep — DONE at pilot scale** (same window);
   full-scale confirmation only if a future recipe change reopens the
   axis.
3. **causal_conv1d canary** — install → 4k-row b2a8 pilot → same-seed
   loss parity within fp16 noise → keep only if ≥1.3× s/step.
4. **Bridge flag stages (a)/(b)** — at the post-arms restart (§9
   serving notes; MTP as a measured A/B).
5. **Clock-lock sweep on t5500** {unlocked,1300,1200,1100} ×
   {120,110,100 W} — after the PSU swap, never mid-run.
6. **Corpus ceiling/noise estimators** — proposed lane (gate-transfer
   prior from teacher facts; VIVERE gain-guided extraction) —
   awaiting operator go.

### 2026-10-14 addendum: corpus-v3 is the board-topper's training corpus — and the ingest half of the #124 plan never shipped

`autotrust/JEV-27B` (88.70 family-macro on public-231, above its own
closed teacher) trained on exactly the `SargeDev/jev-distill-corpus-v3`
whose ingestion §11's gates were built for: 740,957 rows = `yuri_v3`
498,010 (synthetic operational scenarios, 53 domains, labels distilled
from TypeSafe Jev 1.13 via OpenRouter) + `yuri_v1` 148,154 (the stream
our lineage DID ingest — 148,160 at our snapshot) + `openjev_v2` 94,801
(CC0, ZefanCai/Open-Jev reschema'd). Measured facts:

- **merged-v5 contains ZERO `yuri_v3` rows** (grep-verified). merged-v4
  was built as merged-v3 + the 9,810 quarantine remainder; the §11
  gates ran their first measurement over all 655,806 train rows
  (2026-10-08: 81,740 rows below the 0.5 top-prob floor, 207,461
  margin<0.1, zero schema violations, zero label fights) but the
  gated ingest itself never became an arm. The earlier "upstream never
  grew" recon compared `corpora/src/dataset.jsonl` against what
  merged-v1 ate — a local-vs-local check that missed the v3 revision
  entirely.
- Their card's own limitations (mirrors teacher's mistakes, ~7 %
  option-order flip, placeholder rows) are the yuri_v1 relevance
  stream's known properties — the consensus/noise machinery we built
  for v5 attacks exactly this and was never pointed at yuri_v3.
- License clean at every layer: corpus Apache-2.0, openjev stream CC0.
  Whether SargeDev's OpenRouter/TypeSafe terms permitted their
  distillation is their exposure; the published artifact is
  Apache-2.0 and downstream-clean for us. Their `calibration` /
  `test` / `test_set_30k` splits stay out of any train use (their
  internal metrics are in-sample on test_set_30k).

**merged-v6 candidate (queued behind T3.3's verdict):** ingest
corpus-v3 train (655,806) through `corpus_gates.py` at recorded
thresholds + suite decontamination, union merged-v5. Optional
consensus pass over the new rows (strands judge, ~42 GPU-h at
0.23 s/row) only if T3.3 shows consensus labels move the gate. This
is the largest single data-side lever available and the exact corpus
the open board-topper trained on — at 2B, with per-kind calibration,
on consensus-cleaned labels.

## T3.3 r16 arm: merged-v5 corpus FALSIFIED (2026-10-14)

Fedora recovery pipeline (co-tenant 27B OOM killed with operator
authorization; `runs/t33-recover.sh`, real-rc discipline) ran the r16
arm's eval legs. Training itself was healthy: 11,615 steps, final val
0.0168 / tok_acc 0.9943, adapter-best 5.9 MB. Then, through the pinned
harness lane (same suite sha, same readout as e1-e):

- **holdout 120: 0.5167** (e1-e on the identical suite: 0.8083) —
  **GATE_FAIL** vs 0.68; board leg correctly skipped by the gate
  (0.5167 < 0.68). No board number exists for this arm; none may.
- **rel200: 0.4071** (e1-e: 0.718–0.732).
- Per-class: lexical 0.725 / metadata 0.675 / **relational 0.15**.
- ECE raw 0.3022 → per-kind T=2.7974 → 0.1128 (same hot-logits shape
  as every arm; artifact baked to `adapter-best/perkind_calibration.json`).

Reading: recipe byte-identical, corpus the only delta (rows sha
`018f6a1d…`), so this is a **−0.29 holdout / −0.31 rel200 corpus-only
regression**. Train fit near-perfect (0.0168) means optimization
succeeded — the *training distribution* is the defect: merged-v5
(SargeDev 142,591 + vivere 39,231 + typed-decisions 1,200 + suite 240)
regressed the relational/compositional slice catastrophically vs the
E1 corpus. Note the paradox recorded at T3.3 launch: per-kind
teacher-agreement ROSE in v5 (noul 0.9721 / choice 0.9765 /
score 0.9608) while transfer collapsed — teacher-agreement is not the
binding constraint; family mix/relational density is.

Consequences:

1. **merged-v6 promoted from candidate to the named next data lever**
   (corpus-v3 ingest, 655,806 rows — see preceding section). It is the
   only untested corpus-scale move, and it is the corpus the open
   board-topper actually trained on.
2. The relational-density program re-opens with evidence: negative-
   enriched ratio ≤ 1:3 on relational rows, engine-generated
   relational items (B3 lane).
3. r8/a16 (rank A/B) trains on the same corpus as the within-corpus
   control; it is expected to fail the gate for the same reason and
   is informative for shape/rank only.

### Root cause closed: the A/B pair is e1-iterb vs e1-softlabel-v2 (2026-10-14, later same day)

Zero-GPU probes settled the mechanism. First correction to the reading
above: e1-e did not train on e1-softlabel-v1 — TRAINING.md §9.9.1 pins
e1-e to **e1-iterb (177,999 rows)**. The honest A/B pair is therefore
e1-iterb vs e1-softlabel-v2, and the recipe is verified byte-identical
(`e1_train.py:443` — `target_modules=["q_proj","k_proj","v_proj","o_proj"]`
on both; rank/hardware were the only recorded deltas).

Census of the two corpora (kind mix / engineered content):

| | e1-iterb (e1-e) | e1-softlabel-v2 (r16) |
|---|---|---|
| rows | 177,999 | 187,822 |
| choice | 76,411 (42.9%) | 13,874 (7.4%) |
| score | 53,077 (29.8%) | 29,557 (15.7%) |
| noul (boolean) | 48,511 (27.3%) | 144,391 (76.9%) |
| itemgen streams | **67,619 (38.0%)** | **0** |
| top sources | vivere 50,841, SargeDev 47,854, itemgen/score-board 23,520 | SargeDev 142,591 (75.9%) |

The itemgen streams in iterb (score-board 23,520, root-cause-agreeing
15,300, first-restored-chain 15,300, root-cause-chain 13,499) are the
engine-generated relational/compositional items — exactly the slice
that collapsed to 0.15 in r16. Two alternative mechanisms were probed
and falsified on the way:

- **"the family mix changed v1→v2"** — wrong pair, and moot: the
  e1-softlabel-v1 vs v2 census shows compositionally identical corpora
  (only 3,652 consensus-flip labels differ, 1.9%).
- **"the consensus flip pass injected contradictions"** — flip probe
  (merged-v4 vs merged-v5, grouped by normalized instructions):
  3,652 flips, **0 injected into previously-consistent groups**; v5 is
  slightly MORE consistent than v4 (37 vs 42 mixed sibling groups).

Verdict: the softlabel stream structurally lacks what the iterb
program engineered in — choice/score mass (23.1% vs 72.8%) and the
itemgen relational chains (0% vs 38%). Training on a 77%-boolean
corpus shifts the prior hard toward boolean; the suite's choice-heavy
relational slice collapses. Simple distribution shift, no exotic
explanation. **merged-v6 design constraint updated: it must carry
iterb's engineered content (itemgen streams or equivalent relational
regeneration) with corpus-v3 as the volume lever — a raw-stream union
alone is now falsified.** Throughput note: the staged T2.2/T2.3 pilots
now run `--no-grad-checkpoint` (A7 flag, bit-identical math, ~20%
throughput) on fedora where peak memory allows.

## T3.3 r8 rank control: GATE_FAIL, and the corpus verdict gets a second angle (2026-10-14, 4:23 PM Central)

The staged chain's first leg — `runs/t33-recover.sh` r8/a16 (half the
rank of the falsified r16 arm, same merged-v5 corpus, same recipe
otherwise). Training healthy to the end (cum 0.0505, LR annealed to
~4e-07, gn clean throughout; tok/s 22 at --no-grad-checkpoint... the
r8 leg predates the flag, tok/s 21).

| arm | rank | holdout | rel200 | board |
|---|---|---|---|---|
| t33-2b-v5-r16 | r16/a32 | 0.5167 | 0.4071 | skipped (< 0.68) |
| t33-2b-v5-r8 | r8/a16 | **0.6250** | **0.4393** | skipped (< 0.68) |

Expected GATE_FAIL, delivered — but the rank control did more than
confirm: **halving the rank IMPROVED holdout by +10.8 and rel200 by
+3.2 while still failing the gate.** On a healthy corpus, more rank
never hurts holdout this way; on v5 it does. The reading: r16 had
enough capacity to memorize the 77%-boolean weak-label mix, and
memorization displaced transfer — r8 memorizes less and generalizes
better, yet still cannot clear 0.68 because the composition deficit
(the closed root cause above) binds before capacity does. Two
independent angles now agree: merged-v5 fails on composition, not on
recipe or capacity. No further v5 arms warranted at any rank.

Recovery calibration fit ran as designed (per-kind T: choice=2.169);
artifacts pulled to /nas/Temp/work/oc-model-eval/runs/t33-2b-v5-r8
(adapter, adapter-best, holdout/rel200 evals, manifest). Chain
proceeded to t22-t23 at 4:26 PM Central (T2.2 canary + T2.3 sweep,
then the t66 corpus bake-off decides the T3.3' corpus slot).

## T2.2/T2.3 first pass: one banked lever, two defects, rerun running (2026-10-14, 5:12 PM Central)

First-pass outcomes (details + defect table in
`planning/08b-gate-closure-campaign.md`):

- **Banked — length-bucketing +31.3% tok/s** at batch2/accum8 under
  `--no-grad-checkpoint` (58.8 → 77.2 tok/s, t23-a vs t23-b, paired
  same-condition). #147's fix is measured-effective.
- **T2.3 b4/b8 arms + all four t66 bake-off arms OOM** — the
  `--no-grad-checkpoint` pilot config sat at 12.7–13.2 GiB on the
  15.57 GB card; any bigger micro-batch or transient co-tenant draw
  tipped it over (failed allocs 28–96 MiB).
- **T2.2 canary FAIL was structural** — comparator wanted n≥100 common
  windows; the trainer emits one win-line per 20 steps, so 400 steps =
  exactly 20 windows. Kernel means matched within 0.36%; the 16.6%
  worst delta was early-window warmup noise, not divergence.

Rerun (`runs/chain-rerun.sh`, launched 5:10 PM Central): every arm
trains WITH grad checkpointing — recipe-faithful (the champion recipe
uses ckpt) and ckpt is math-transparent, so the bake-off corpus ranking
is unaffected — plus `expandable_segments:True` to kill the
fragmentation co-factor. Canary comparator fixed (n≥15 windows, worst
rel delta over step≥100 windows ≤ 2%, overall mean ≤ 1%). Order:
bake-off → sweep → canary; results land here + the campaign doc.

## T2.3 sweep + T2.2 canary rerun verdicts (2026-10-14, 6:52 PM Central)

**T2.3 sweep (all ckpt ON, softlabel-v1, batch×accum=16 throughout) —
#147 closes with a clean throughput table:**

| arm | config | mean win (post-100) | tok/s | Δ vs a |
|---|---|---|---|---|
| t23r-a | b2/a8, no bucket | 0.0716 | 32.9 | — |
| t23r-b | b2/a8 + bucket | 0.0690 | 43.2 | +31.3% |
| t23r-c | b4/a4 + bucket | 0.0703 | **47.5** | +44.4% |
| t23r-d | b8/a2 + bucket | 0.0723 | 44.5 | +35.3% |

Readings: bucketing replicates its banked +31.3% exactly under ckpt ON
(config-independent lever); b4 is the throughput sweet spot; b8
regresses; loss trajectory is flat across configs (mean win
0.069–0.072) — pure throughput lever, ordering-neutral. vs first pass:
`--no-grad-checkpoint` was worth +79% tok/s (58.8/32.9 and 77.2/43.2,
consistent both pairs) — the speed is real, and so was the peak-memory
bill that killed six arms. Memory-constrained sweet spot: **b4-bucket
ckpt ON**.

**T2.2 canary with the fixed comparator (n≥15, worst post-100 ≤ 2%,
mean ≤ 1%): FAIL — and this time it's a measurement, not a structure.**
CANARY_WINDOWS=20, WORST_POST100=0.0678, MEAN_DELTA=0.0165. causal_conv1d
is NOT trajectory-transparent vs sdpa under fp16: same-seed windows
drift up to 6.8% relative. It does buy **+29.2% tok/s** (42.5 vs 32.9).
Verdict banked: kernel = throughput lever with a numerics cost — fine
for pilots/sweeps where trajectory identity doesn't matter, OFF for
gate arms (one variable at a time). Env restored after the canary.

**T3.3' LAUNCHED 6:47 PM Central** (runs/t33p-v6full.sh →
runs/t33p-2b-v6full): Qwen3.5-2B, r16/a32 champion recipe, sdpa,
ckpt ON, full merged-v6-full (306,999 rows: 151,436 choice / 81,537
score / 72,026 noul; train 304,999 / val 2,000), ~19.2k steps ≈ 19 h →
train ETA tomorrow early afternoon Central; holdout/rel200/perkind legs
+ 0.68 gate + conditional JevBench board leg are chained in the same
script. One change vs the T3.3 arms: the corpus (census verdict above).

## T3.3' verdict: GATE_PASS ×2 — and the corpus-volume lever closes (2026-10-15, 5:20 AM Central)

`t33p-2b-v6full` finished clean (train rc=0, 19,058 steps, 9.7 h,
gn max 7.81, no val divergence; artifacts pulled to
`/nas/Temp/work/oc-model-eval/runs/t33p-2b-v6full`).

| leg | E1-E (iterb scaffold, 178k) | t33p (merged-v6-full, 307k) | release gate |
|---|---|---|---|
| suite holdout (120) | **0.8083** | 0.6917 | ≥ 0.68 — **PASS** |
| rel200 (200) | **0.718–0.732** | 0.6607 | — |
| JevBench board (231) | **0.6840** | 0.6580 | ≥ 0.6494 — **PASS** |
| suite→board drop | −12.4 | **−3.4** | — |
| ECE (per-kind T fit) | 0.127 (T=3.362) | 0.1313 (T=5.225) | — |

**GATE_PASS × 2 — the second own arm through the release gate, and the
first via the corpus-composition route.** But the E1-E comparison on
the identical suite/readout is the real result: **E1-E remains strictly
better on every leg.** The 129k confidence-gated v3 additions cost
~12 holdout / ~6 rel200 / ~2.6 board points at fixed recipe. What v5
proved catastrophically (ungated weak-label volume destroys transfer),
v6 proves precisely (gated + deduped volume still does not add
transfer): **confidence-gated volume ≠ transfer.** The census gates
measure label quality; they do not measure marginal training value.

The composition arc is now complete at gate scale: v5 catastrophic →
v6 recovers to gate-passing → scaffold alone best. No
merged-v7-at-volume arms warranted; the corpus-volume lever is closed.

Silver lining worth keeping: t33p's suite→board lane drop is −3.4 vs
E1-E's −12.4 — the v6-trained arm agrees across lanes far better. If
part of the suite number is lane-specific, v6-style data makes the two
legs honest with each other; relevant to #150 (eval lane v2).

Release candidate: unchanged — E1-E. t33p is evidence, not a
candidate. Next levers, evidence-ranked: consensus-flip patch /
strands consensus pass (quality-first), T3.4 4B contingency, #150
eval lane.

## T88 imatrix Q4_K_M arm: FAIL on all three gates (2026-10-15, 10:20 AM Central)

The #88 quant bake-off imatrix leg, executed end-to-end on fedora
(T5500 still down): peft `merge_and_unload` of t33p onto the Qwen3.5-2B
base (F16) → fork converter `--no-mtp` → stride-25 in-domain corpus
(≈6,900 rows rendered) → `llama-imatrix --chunks 1500` →
`llama-quantize --imatrix … Q4_K_M` (1.19 GB) → two fork-lane JevBench
passes (port 8097, `--decision-seqs 24`, CPU).

| gate | threshold | measured | verdict |
|---|---|---|---|
| G1 determinism replay | byte-identical rows | runner's own `results.jsonl` vs `results-replay.jsonl` differ in **both** runs, and run A ≠ run B | **FAIL** |
| G2 board leg | ≥ 0.6494 | **0.6104** | **FAIL** (−3.9 pts) |
| G3 no-regression | ≥ 0.648 (F16-LoRA 0.6580 − 1 pt) | **0.6104** | **FAIL** (−4.8 pts) |

Two process notes recorded honestly: (1) the arm script's G1 check
diffed a `rows.jsonl` path the runner does not write — that FAIL line
was a script bug — but the runner's **built-in** replay comparison
fails on its own, so the determinism verdict stands regardless;
(2) the imatrix pass needed `--chunks 1500` (whole-file default ≈ 9 h
under CI co-tenancy; chunk-capped finished in < 1 min once load
dropped).

This arm is triple-dead: it violates the standing quant floor
(every deployment quant passes determinism replay), the release gate,
and the no-regression line. **Nothing publishes from this arm; E1-E
remains the only release candidate** — the quant bake-off now has a
verdict for the imatrix leg matching the historical pattern (4 of 6
requants flipped; quantization of this adapter is not safe at Q4_K_M).

Attribution run in flight at write time: unquantized merged-F16
through the identical fork lane (`runs/jevbench/fork-t88diag`,
started 10:24 AM Central). Reading: ≈ 0.658 ⇒ the Q4_K_M quantization
is the culprit — next levers Q5_K_M, more imatrix chunks, base-model
imatrix, or a separately-trained adapter; ≈ 0.61 ⇒ the
`merge_and_unload` → `convert_hf_to_gguf` path itself regresses —
audit LoRA scaling (B-application) against `convert_lora_to_gguf`
before any further quant work. Verdict to be appended.

### T88 attribution verdict (2026-10-15, 11:05 AM Central)

The diagnostic arm (unquantized merged-F16 through the identical fork
lane, 231/231 scored) landed at **0.6494**:

| lane | accuracy | delta vs F16-LoRA 0.6580 |
|---|---|---|
| F16-LoRA (t33p, reference) | 0.6580 | — |
| merged-F16, fork lane (diagnostic) | **0.6494** | −0.9 pts |
| merged + imatrix Q4_K_M (T88 arm) | 0.6104 | −4.8 pts |

**The quantization is the culprit, and the bake-off closes no-go.**
The merge+convert path costs only ~0.9 pts (lane-level; the F16-LoRA
reference serves via `convert_lora_to_gguf` + adapter application, a
different path), while Q4_K_M imatrix quantization costs a further
~3.9 pts. Decisive detail: even the UNQUANTIZED merge path sits
exactly at the release-gate line (0.6494) with zero headroom — so no
quant tier of this arm can clear the board leg. Q5_K_M / more-chunks /
base-imatrix levers are moot for this adapter; a separately-trained
quantization-aware adapter would be a new arm, not a repair. E1-E
remains the only release candidate.

Second finding, now measured twice (T88 run A/B and the diagnostic):
**the CPU fork lane does not replay byte-identical under host load**
(`results.jsonl` vs `results-replay.jsonl` differ in every pass
today). The determinism-replay gate as written (byte-identical) is
stricter than this lane can satisfy; determinism claims for quants on
this lane need either a load-pinned window or a tolerance definition
(same verdict set, identical accuracy) — flagged for #150 (eval lane
v2). The T88 G1 FAIL therefore reflects lane behavior, not an
imatrix-specific defect; the accuracy gates alone already killed the
arm.
