# MiniCPM5 → GGUF model rung: plan

Opened 2026-10-12 while the v2 0.8B arms train (GPU window busy, CPU
lanes free). Operator hypothesis, falsifiable: *quantizing
MiniCPM5(-2B-Jev) to GGUF is the best size/intelligence/speed option
for the model rung.* This plan turns that into a measured verdict and,
if it survives, the concrete B5 model-rung candidate.

Context: the MiniCPM5-2B-Jev analysis (2026-10-12, `docs/BENCHMARKS.md`
footnote 8) measured the authors' artifact at 0.7792 acc / 0.7816 macro
/ ECE 0.054 / auto@5% 0.524 on public-231 through their fp32 PyTorch
CPU path (3.06 s mean, 35.5 s p95, fedora 16-thread). Board A leader.
Our best own arm: e1-e 0.6840 @ 0.58 s (V100). GGUF fork 2B arms:
decider-2b 0.7576 public-231.

## Recon results (2026-10-12, all verified)

1. **Base arch is stock Llama.** `openbmb/MiniCPM5-2B` config:
   `LlamaForCausalLM` / model_type `llama`, 42 layers, hidden 2048,
   GQA 16/2 heads, vocab 130560, rope_theta 5e6, bf16, untied
   embeddings, 131k ctx. llama.cpp serves it with **zero arch work**;
   OpenBMB publishes official GGUFs (F16/Q8_0/Q4_K_M), and
   `MiniCPM5-2B-Q4_K_M.gguf` is already on fedora
   (`~/Models/minicpm5-2b/`).
2. **The readout head is not a task head.** Their
   `LetterReadoutHead` is a slice of the model's *own* `lm_head`
   restricted to letter token ids (`model.py:342`,
   `letter_weight = full_lm.lm_head.weight[label_ids]`; their tag:
   `native_lm_head_letters_pride`) plus PriDe prior debias and
   per-type/global temperatures. Nothing in it is foreign to the
   transformer — so a merged GGUF + the fork's letters/verdict-slot
   readout (vocab logits at the decide position, restricted to letter
   ids) reproduces the mechanism exactly, and the debias/temperatures
   port into the Rust `Calibration` seam as f64 (the D15 machinery).
3. **Merge is faithful by construction.** Adapter (PEFT LoRA r=16/α32)
   targets only `q/k/v/o/gate/up/down_proj`; `lm_head` and embeddings
   untouched, `modules_to_save: null`. `merge_and_unload()` yields
   precisely the model their serving path sliced.
4. **Their calibration floats are on disk**
   (`checkpoint_meta.json`): global T=1.1314; per-type choice 1.1314 /
   noul 1.2338 / score 0.4213; their val-297 raw ECE 0.0355 →
   calibrated 0.0242. These are the fit to *reproduce-or-beat* — refit
   post-quant on our side, never import blindly (quant shifts logits).
5. **Licenses clean on both layers**: adapter repo Apache-2.0, base
   Apache-2.0. Serving and derivative weights permitted.
6. **No ready-made merged GGUF exists** (checked HF 2026-10-12; only
   base-model quants). Conversion-from-HF is the only path — this is
   not the forbidden re-convert case (that rule targets re-converting
   existing quantized arms / unloadable arms); there is nothing
   ready-made to prefer.
7. Toolchain on fedora: base HF snapshot + adapter + `model.py` at
   `~/oc-model-eval/models/minicpm5/`; sft venv (torch 2.14.0, peft
   0.21.0, transformers 5.17.0); converters in `~/Repos/llama.cpp` and
   `~/llama.cpp-tq3` (built: llama-cli b10369). Fork build of record
   resolved and sha-pinned at execution time.

## Arms matrix

| Arm | Weights | Purpose |
|---|---|---|
| `mc5-base-q4` | official base Q4_K_M, no adapter | control: how much of 0.78 is base capability vs decision tuning (stock base letters historically mid-0.4s on this suite) |
| `mc5-jev-f16` | merge(base+LoRA) → f16 GGUF | conversion-parity anchor, quant reference |
| `mc5-jev-q8` | ↑ quantized Q8_0 | primary candidate |
| `mc5-jev-q4` | ↑ quantized Q4_K_M | size/latency frontier point |
| `mc5-ours-*` (later) | our merged-v4 LoRA on this base, same path | base-swap door (e1-08b-next-arms.md) converges here |

## Phases and gates

**Phase 1 — merge + convert** (fedora CPU, ~15 min): PEFT
`merge_and_unload` in bf16 → `save_pretrained` → `convert_hf_to_gguf.py
--outtype f16` → `llama-quantize` q8_0 and q4_k_m. Gate: converter
emits the llama arch without warnings; each GGUF loads in the fork
build; manifest records shas + build sha.

**Phase 2 — parity fixtures BEFORE any benchmark** (model-import
discipline, quality.md):
  a. **Tokenization**: HF token ids == GGUF token ids on ~50 rendered
     wire prompts (catches a `tokenizer.ggml.pre` regex fallback, the
     classic silent corruptor for a fresh vocab).
  b. **Letter-logit agreement** at the decide position, authors'
     priors/temps OFF: fp32 PyTorch vs f16 GGUF vs Q8 vs Q4 — argmax
     agreement + mean KL over the letter slice. Gate: f16-vs-fp32
     ≈ zero (conversion parity); Q8 tight; Q4 recorded honestly even
     if looser.
  c. **Sub-chance fixture accuracy = halt-and-diagnose**, never
     proceed to a benchmark (the 0.2251 mis-anchored-index lesson).

  *Execution log (2026-10-12):* tool = `runner/gguf_parity.py`
  (Layer-1 tokenization exact-match + Layer-2 fp32-HF vs server
  next-token top-K on 48 suite fixtures). First f16 leg: tokenize
  1.000, argmax 0.833, mean|Δlp| ≈ 1.0 nat → gate fail, chain halted.
  Diagnosis (fingerprint probes, never a blind rerun): **not a
  conversion defect** — the GGUF inherited `add_bos_token=true,
  bos_token_id=0` from MiniCPM5's tokenizer config, so llama-server
  prepended `[0]` to every *string* prompt (109→110 tokens), while the
  reference path (ytbai `model.py`) tokenizes `add_special_tokens=False`
  — no BOS ever. Proof: explicit-id prompts agree to 3 decimals
  (f16 ≈ fp32 exact); `HF([0]+seq)` fingerprints the string-prompt
  distribution on all top-5 tokens to ≤0.01 nat. Fix at the artifact
  level (`flip_add_bos.py`): `add_bos_token=false` in all three GGUFs,
  re-pinned — f16 `94dd64a6…`, q8_0 `fc61a377…`, q4_k_m `b24efa5c…`.
  Lesson: `/tokenize` does not apply the BOS prepend that string
  `/completion` does; sequence truth for parity must come from the
  serving path, not the tokenize endpoint.

**Phase 3 — public-231 frontier**: letters readout per arm →
acc / macro / ECE / auto@5% + latency mean/p95 + file size, fedora CPU
(bounded threads; parity environment with the receipt). Then the
post-hoc fit — per-type temperatures + prior debias, D15 style, fitted
on our calibration split — with the standing gate: calibration must
not worsen ECE. V100 latency numbers after the t5500 frees (~Oct 13,
post-v2-arms).

**Phase 4 — decision**: frontier table vs the receipt rows (PyTorch
path, decider-2b GGUF, e1-e). Go/no-go for B5 integration (llama.cpp
backend + verdict-slot + Rust calibration). The winning arm(s) get the
JevBench leg — required before any release/teacher-lineage claim
(both legs rule); schedule when the t5500 frees or fedora off-peak.

## Verdict (2026-10-12, phases 1–3 executed; board row + fn9 in
`docs/BENCHMARKS.md`)

Frontier (public-231, fork tree-mode lane, T=14 fedora CPU; raw →
post-D15 in-sample refit):

| arm | size | acc | ECE | p50 |
|---|---|---|---|---|
| mc5-jev-f16 | 5.0 GB | 0.6364 → 0.6450 | 0.263 → 0.060 | 1.22 s |
| mc5-jev-q8_0 | 2.7 GB | 0.6364 → 0.6407 | 0.269 → 0.058 | 1.30 s |
| mc5-jev-q4_k_m | 1.6 GB | 0.6320 → 0.6407 | 0.246 → 0.082 | 1.47 s |
| mc5-base-q4 (control) | 1.6 GB | 0.6320 → 0.6407 | 0.246 → 0.082 | 1.29 s |

- **Size/speed/fidelity legs: thesis WINS.** Parity-exact conversion
  (f16 ≡ fp32 to 3 decimals; q8 argmax 1.000, 0.126-nat mean delta),
  quant cost ≤0.4 pt at q4, 1.6–2.7 GB served vs ~10 GB fp32 runtime,
  CPU p50 1.2–1.5 s vs the authors' 3.06 s.
- **Intelligence leg: thesis FAILS the pre-registered bars.** Every
  arm caps ≈0.64; the Jev adapter adds ≤0.4 pt over the base-model
  control through this lane (base-q4 and jev-q4 are statistically
  identical). Rung bar (beat e1-e 0.6840 at lower latency): FAIL on
  both legs. Publish bar (JevBench ≥ 0.6494): FAIL (0.6407). No
  JevBench leg scheduled — the holdout gate would not clear.
- **Attribution**: the authors' 0.7792 is lane-bound — their prompt
  format ("Answer: (" cue), native letter readout, and PriDe
  machinery do not transfer through the fork tree-mode lane. The
  adapter's skill is real (their receipt replicates) but bound to
  their serving path.
- **D15 refit mechanics validated**: raw ECE 0.25–0.27 with ~70% of
  rows at conf 0.99 is fully treatable by 4-parameter per-type
  temperature + 0.1 prior debias (post ECE 0.058–0.082, in line with
  the d15-4B lane's 0.0696). `mc5_refit.py` is arm-generic.
- **B5 go/no-go: NO-GO for this arm.** e1-e stays the release line.
  The lane's durable deliverables: `runner/gguf_parity.py` (fixture
  gates), the BOS-prepend serving-path rule, `flip_add_bos.py`,
  sha-pinned GGUF ladder, and the refit tool. V100 numbers moot (gate
  failed on CPU parity env; latency would not flip the verdict).

## Decision criteria for the hypothesis

- **Rung bar** (integration candidate): merged arm beats e1-e 0.6840
  at materially lower latency than the PyTorch path, with calibration
  fit passing the ECE gate.
- **Publish bar** (anything attributed to OpenCode): existing
  release gates — suite ≥ 0.68 AND JevBench ≥ 0.6494. Nothing here
  publishes without both legs.
- **Reference to stay near**: 0.7792 fp32. Q8 within ~1 pt is a
  quantization win; Q4 traded honestly if the frontier table says the
  size/latency buys more than the points cost.
- GGUF-vs-PyTorch verdict is about the *serving path*, not just the
  model: same weights, fork readout + Rust calibration vs their
  `model.py` fp32 loop.

## Risks

- Tokenizer pre-tokenizer regex fallback in conversion → silent id
  mismatch (Phase 2a exists for exactly this).
- Quant cost concentrates in small letter-logit margins; ppl-style
  intuitions understate it. Fixtures + 231 runs answer, not vibes.
- Their head.pt floats are fitted to THEIR fp32 readout; refit
  post-quant, then compare fits (a large drift is itself a finding).
- 5.5 h-class PyTorch JevBench leg — run the GGUF-served variant
  instead once Phase 2 passes (same harness, faster arm); keep the
  PyTorch number as the already-recorded cross-check on public-231.
- CI co-tenancy on fedora: bounded threads, short runs, observe load
  before each phase.

## Costs

Phases 1–3 ≈ 1–2 h wall on fedora CPU, zero GPU, zero spend. Phase 4
is a decision doc; B5 integration remains the pre-scoped L-sized item.

## Non-goals

- No ONNX export in this lane (B5 picks the backend once the frontier
  is known).
- No modification of the authors' adapter; we merge and serve, credit
  stays with ytbai, lineage recorded in manifests.
- No teacher-lineage decision until both gate legs exist for this arm.
