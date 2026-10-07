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
- F3 Profiler-informed Python config wins (sdpa, bucketing) stay
  Python — Rust goes where determinism and proof live.

**Phase G — research integration.** Three standing research tracks
(small-LM LoRA knobs; hard negatives/contrastive/calibration; perf +
instrumentation) run in parallel; their findings fold into the A/B
knob manifests and cite sources in this file's Sources section.

Standing gates carried: no suite item in training data (all 240,
checked mechanically); no non-Apache-2.0 corpora; ECE re-checked on
every arm (accuracy alone is not acceptance).

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
