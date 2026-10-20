# Reference: NCM — Next-Concept Modeling (arXiv 2610.14162)

**Assessment date:** 2026-10-20
**Verdict:** **Do not adopt.** Harvest four ideas; none of them depend on NCM itself.
**Sources:** arXiv 2610.14162 (v1 HTML), `github.com/HazyResearch/ncm` (MIT),
`huggingface.co/hazyresearch/ncm-checkpoints` (MIT),
`huggingface.co/hazyresearch/ncm-datasets` (MIT),
`huggingface.co/hazyresearch/ncm-vqvae-checkpoints` (Apache-2.0),
`huggingface.co/hazyresearch/ncm-synthetics-checkpoints` (Apache-2.0).

---

## 1. What it actually is

NCM is **not** a diffusion model. There is no noise schedule, no denoising, no
iterative resampling of a masked sequence. It is a **discrete, coarse-to-fine,
multi-resolution generative model** — the text analogue of VAR / next-scale
prediction for images (the paper cites VAR-style next-scale prediction as its
direct lineage).

Two stages:

1. **Multiscale residual VQ-VAE tokenizer** (frozen for stage 2). Text → a
   hierarchy of K discrete code levels, each with its own codebook; later
   levels encode the *residual* of earlier ones. Nested dropout makes early
   levels independently meaningful.
2. **Next-concept generator**: a decoder-only transformer (12 blocks, d=768,
   12 heads, RoPE, QK-norm, block-diagonal attention) that emits **one level
   per forward pass**, predicting all positions within a level in parallel.
   **NFE = K levels.** Training is a single forward pass over all K levels
   using block-diagonal attention, with conditioning-code corruption as
   robustness augmentation.

Released configurations (paper Table 5 + released `config.yaml`s):

| Task | Target len L | Levels K | Codebook/level | Latent d | Tokenizer params | Generator params | Prefix P |
|---|---|---|---|---|---|---|---|
| OpenWebText | 1024 | 32 | 4,096 | 384 | 111.2M | 94.65M | 1 |
| LM1B | 128 | 12 | 8,192 | 384 | 111.40M | 97.07M | 1 |
| TinyStories | 128 | 12 | 4,096 | 384 | 36.43M | 88.61M | 1 |
| XSum | 48 | 8 | 4,096 | 384 | 60.63M | 88.02M | 512 |
| WMT14 De→En | 64 | 11 | 4,096 | 384 | 144.00M | 119.96M | 96 |
| Sudoku | 89 | 12 | 16 | 256 | 14.90M | 25.46M | 91 |

CFG doubles NFE (XSum 8→16, WMT14 11→22).

## 2. The speed claim, corrected

The paper's own benchmark: **1,024 sequences at batch 16 on one H100 SXM 80GB**.

| Benchmark | NCM | Compared to | NFE | Wall-clock speedup |
|---|---|---|---|---|
| LM1B | 97M, 12 steps | DUO (strongest DLM) | 12 vs 128 | **21×** |
| OpenWebText | ~95M, 32 steps | strongest DLM baseline | 32 vs 1024 | **10×** |
| XSum | 88M, 8 steps | AURORA-LM (130M) | 8 vs 128 | **31×** |

The GitHub README range is **9–31×**. The frequently-quoted "32×" is wrong —
**31×** is the paper number, and the comparison is against **diffusion LMs**,
not autoregressive LLMs.

**There is no AR-LLM wall-clock comparison anywhere in the paper.** The
Scaling Intelligence blog states explicitly that no numerical AR speedup is
reported. The only AR-vs-NCM number is NFE (8 vs 64 on XSum, 11 vs 64 on
WMT14) against ~99M-parameter AR baselines — i.e. **6–8× fewer generator
calls**, on models roughly 20–40× smaller than anything we would use.

## 3. Why the headline does not transfer to OpenCodifier

Our decision is **one verdict read from a single forward pass** over a prompt
containing the question plus all N options. We are already at **1 generator
call per decision**. NCM would be **K = 8–32 calls per decision** — strictly
slower than today's arm.

- NCM's advantage compresses **sequential token decode** into parallel
  passes. We do not decode tokens; we read verdict slots.
- Our remaining cost driver is **prefill** (prompt length). NCM cannot reduce
  prefill — it reduces decode calls.
- NCM's own limitation bites directly: *"NCM fixes both the hierarchy depth K
  and the per-level lengths ℓ_k, so even short continuations traverse the full
  hierarchy."* A decision is 1–2 tokens, so a decision-specific NCM would need
  a **retrained tokenizer and level schedule per task**.

### 3.1 Generate-only — there is no readout

The released code exposes `generate()`, `_sample_level()`, `_sample_level_raw()`,
`_decode_level_logits()`, `_encode_context()`, `_run_stack()`. There is **no
scoring, likelihood, or log-probability function anywhere**. The paper
describes no downstream scoring/classification application.

The one conceivable path — feed `"<question>\n<option A> ->"` as a raw-token
prefix and read level-0 logits — fails because level 0 is a coarse semantic
sketch (level-0 length is **1 code** for LM1B/WMT14, 4 for XSum, 91 for
Sudoku), no trained "the answer is X" token exists, and candidate likelihood
across the hierarchy is not well defined (fixed per-level lengths, undefined
length normalization).

Adapting NCM to selection therefore requires: re-implementing the **unpublished
training code**, training a new multiscale tokenizer + generator with a
decision-specific level schedule, and inventing a readout. **A research
project, not an integration.**

## 4. Engineering cost and quality ceiling

- **No ONNX, no safetensors, no GGUF, no `transformers` modeling class.** The
  checkpoints are raw `torch.save` `.pt` files; loading requires the repo's own
  `models/` + `tokenization/` code. A faithful ORT port of the generator
  (per-level subgraphs, dynamic block-diagonal masks, per-level RoPE/level
  embeddings, per-level `post_upsample_conv`, optional tied codebook heads)
  **plus** the multiscale residual VQ tokenizer (encoder, residual quantizer,
  EMA codebook lookup, `learned_l1_cascade`, nearest-neighbor argmin, decoder)
  **plus** a parity harness is estimated at **3–6 engineer-weeks plus GPU
  time, before any training.**
- **Nothing above 725M.** Largest released generator is 725M (LM1B-large);
  the scaling study is 97M / 317M / 725M **on LM1B only**. Everything else is
  25M–120M. Context lengths 129–560 tokens. No instruction tuning, no MCQ
  supervision, no reasoning traces.
- **OpenWebText weights are not released** (`owt/TODO.md`, 5 bytes) despite OWT
  being a headline result.
- **Quality at scale is well below AR**: OWT Gen. PPL 83.12 vs AR sample 35.45
  (ground truth 14.15); TinyStories 15.41 vs 8.89. NCM's real win is
  **global-planning** tasks (Sudoku: 99.5/96.5/86.1 vs AR greedy
  14.6/5.1/1.0) — a regime we are not in.
- **The lossy VQ bottleneck penalizes our weak spot.** The paper's own words:
  one token error can "prevent code from running, or invalidate" the output.
  Our weak families are exactly composition-heavy multi-step reasoning
  (multi_hop 0.389, temporal_numeric 0.333, long_policy 0.368).

### 4.1 Licensing

| Artifact | License | Usable in our Apache-2.0 project? |
|---|---|---|
| `github.com/HazyResearch/ncm` code | MIT | yes |
| `ncm-checkpoints`, `ncm-datasets` | MIT | yes |
| `ncm-vqvae-checkpoints`, `ncm-synthetics-checkpoints` | Apache-2.0 | yes |
| **`hazyresearch/nsm`** | **none** | **no** — no tag, no model card; HF default is all-rights-reserved |
| **`hazyresearch/ncp`** | **none** | **no** |
| **`hazyresearch/ncm-neurips-owt`** | **none** | **no** |

The published MIT stack is clean and shippable with attribution. The NSM/NCP
internals are legally unusable — and those are the *only* artifacts large
enough to be interesting, because the published stack tops out at 725M and
120M respectively. NSM's checkpoints are additionally **aborted `.tmp` training
saves** (`checkpoint-iter-10000.pt.tmp.rank0/`), and the NSM generator carries
an extra `shared_head_mlp` head absent from the released NCM configs — i.e. it
is a different branch, not the paper weights.

## 5. Harvest — four transferable ideas (no NCM dependency)

### H1. Coarse-to-fine compute allocation → a new ladder rung

The transferable principle is **spend a cheap coarse pass to decide whether
the expensive fine pass is needed**. In our ladder this is a new rung between
embedding similarity and the decision model: a small concept-level predictor
that classifies question type, shortlists candidates, or abstains early.

Why it is the right lever for us: our cost driver is **prefill**, so anything
that shortens the decision prompt pays directly. Cost: one small model plus a
training-free or lightly-tuned head. Low risk, and it composes with the
existing `retrieve` node's `top_n`/`floor` and with the metadata/BM25 narrowing
already in the engine.

### H2. Entropy alongside accuracy on the choice distribution

NCM reports mean unigram entropy next to every generative perplexity precisely
to catch degenerate low-entropy samples (OWT AR greedy: PPL 1.07 but entropy
1.65 — collapsed). **Our analogue: report the entropy of the per-option choice
softmax alongside accuracy at every rung.** A rung at 0.80 accuracy with
near-zero choice entropy is overfitting a skewed eval; high entropy with low
accuracy is under-confident. We have a "silence is a finding" rule but no such
metric wired to the choice distribution. Cheap, and it turns the abstention /
calibration work into something measurable.

### H3. Cross-slice calibration transfer test

NCM Table 7 exists because the LM1B comparison must not depend on the scoring
model — the authors re-ran everything under Mistral-7B, Llama-3.1-8B,
Gemma-2-9B and Qwen-3-14B because generative perplexity is reference-model
dependent.

**Our analogue:** our calibration temperature is fit against one surface. If it
does not transfer across our JevBench-231 surface and our jabr board surface,
we have a **calibration defect, not a model defect**. Fit temperature on one
slice, evaluate on the other, report the delta. This directly addresses our
known ECE pain (0.2411 on E1-C, 0.2067 raw / 0.1269 calibrated on E1-E, vs
JEV's 0.08) and it is a **cheap, high-value check**.

### H4. Conditioning-corruption augmentation (training-time)

NCM trains the generator with randomly corrupted conditioning levels so it
stays sane when a conditioning part of the input is missing or wrong. Our
analogue: train the decision model so it remains correct when **one option's
context is missing or corrupted**. This maps onto our non-negotiable rule
*"never silently discard candidates on weak semantic evidence"* and onto
candidate-list perturbation robustness. Cheap to try on the existing arm.

### Considered and deprioritized: MBR-style agreement aggregation

NCM+CFG+MBR lifts XSum ROUGE-L 28.2 → 29.6 and WMT14 BLEU 23.9 → 25.2 by
generating K candidates and selecting the one most similar to the others. Our
analogue: sample K verdict-slot reads with **K fixed seeds** and aggregate by
agreement. Deterministic by construction, so it does not violate our
replay-determinism requirement, and it needs no new backend. But our F26
baseline is already 0.8083 and latency is our product — this costs K× latency
for a small expected gain. **Low priority.**

### Also worth knowing: the diffusion-LM landscape

LLaDA (8B) and Dream (7B) are the canonical open masked diffusion LMs;
practical decode rates are ~20 tok/s once refinement steps are counted, i.e.
slower than AR once steps are included — every step reprocesses the full
length-L sequence. The speed leaders are distilled/flow models (Mercury Coder
Mini ~1,109 tok/s on H100; quality still trails AR by ~5–15 points).
NVIDIA's Fast-dLLM gets up to 27.6× throughput on LLaDA/Dream via adaptive
parallel decoding.

**The generalizable lesson:** diffusion's parallelism advantage **shrinks as the
output gets shorter and the step count grows**, and it is **entirely absent
when the output is a single typed decision**. Diffusion buys no determinism
(noise injection) and no calibration. **Nothing in the diffusion literature
reports a multiple-choice selection benchmark** — that is our niche, and it is
currently uncontested.

## 6. What actually applies to our "fixed context" question

The intuition that a **static-token model with a tournament bracket** could
scale to larger decision work is sound *as an architecture direction*, and it
does not require NCM. Our engine already has the shape:

```
normalize → rules/filters → candidate narrowing → semantic scoring
          → decision model → confidence gate → verify / abstain
```

Candidate narrowing is currently **metadata + hand-written BM25** only; the
decision model is a **single-shot argmax over the narrowed set**. A
model-driven narrowing rung — the model scores the candidate set coarsely, a
tournament keeps the top-k at each round, and the final survivors get the
expensive pass — is exactly NCM's coarse-to-fine idea applied to *options*
instead of tokens, and it is the same shape as H1 above.

Two cautions before building it:

1. **A tournament costs K model passes.** NCM's speedup came from *compressing*
   passes, not multiplying them. A bracket that runs 4 rounds of narrowing plus
   a final decision is 5 calls, not 1 — strictly worse than today unless the
   narrow rounds are dramatically cheaper than the decision pass (e.g. a tiny
   concept model over option *text only*, with no question context).
2. **The verdict-slot readout already evaluates all options in one pass** (the
   parallel-decision fork returns the full per-choice softmax distribution;
   F26: 0.8083 via the slot readout vs 0.217 for the scorer-style mismatch).
   NCM's per-level parallelism is the *same idea applied to tokens*; we already
   apply it to options. A tournament bracket would have to beat a single pass
   that already sees every option simultaneously — a high bar, and it is
   exactly the bar a **coarse concept rung** (H1) attacks from the prefill side
   instead.

Recommendation: **implement H1 as a rung, not NCM as a model.** If the coarse
rung shortens the decision prompt measurably, the same mechanism scales to
arbitrarily many candidates by construction.

## 7. Related OpenCodifier findings

- The engine's relational solver fired on **0 of 231** JevBench items
  (`benchmarks/decision-model/runner/proof_fusion.py`, 2026-10-20). The
  solver's fact extractor accepts only exact grammar patterns
  (`X depends on Y`, `X is healthy|degraded|down|failing`, `X: s1, s2, …`,
  `X comes back online only after Y`); JevBench states are free prose. A
  proof-first fusion therefore measured **0.6840 — identical to E1-E alone**.
  See `docs/PLAN.md` for the consequences.
