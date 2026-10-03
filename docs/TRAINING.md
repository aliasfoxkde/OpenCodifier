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

1. **Dataset merge.** Inventory + license-audit the trainable corpora:
   SargeDev/jev-distill-corpus-v3 (Apache-2.0, JEV-27B's corpus), the
   120-item suite + `suite_long` (ours, Apache-2.0), the fork arms'
   prompt/response logs (ours), and any jebadiah training corpus only
   if its license permits. Merge = dedupe (exact + near-dup on
   question id), normalize to the IR request/response shape, and pin
   the merged artifact (SHA-256, source window) per §5.
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
4. **VIVERE knowledge extraction** stays experimental-labeled and
   outside the promotion gate (INTEGRATION_AMORTYX §8): corpus runs
   consume shadow-ledger exports, never live traffic, and any
   VIVERE-derived claim is labeled experimental in both trees.

Non-goals carried from §8: no non-Apache-2.0 corpora, no cloud
training by default; the burst node (T5500 2×V100) or Kaggle/Modal
remain the GPU paths, user-gated.
