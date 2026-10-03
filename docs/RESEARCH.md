# Research Frontier

Standing record of external research that grounds the engine-enhancement
program (PLAN.md Phase 18 / D9 budgets / the B-series steps). Every claim
carries its source; every entry is dated. Append new findings at the bottom
of a section — never rewrite history. This page is the watch-list: each
phase sweep re-checks it and adds dated deltas.

Validated: 2026-09-30 / 2026-10-01.

## 1. Cascade & ladder literature (validates the B-series design)

The core architecture — cheap rungs first, confidence-gated escalation,
abstention as success — is not a local invention; the 2025 cascade
literature converged on the same shape:

- **Gatekeeper** (arXiv:2502.19335): calibrating the *gating* decision (not
  just the model's output distribution) is where cascade loss lives. Direct
  support for B4's per-rung `Calibration` seam + margin gates: we calibrate
  the accept/escalate decision per rung, exactly the Gatekeeper move.
- **Early abstention in cascades** (arXiv:2502.09054): abstaining early —
  before spending the expensive rung — reduced expected loss ~2.2% on
  average across tasks. Supports §38 (abstention is a successful outcome)
  as a *measured* win, not just a posture.
- **Rational cascades** (arXiv:2501.09345): tuning cascade thresholds via a
  probabilistic model of calibrated confidences beats hand-tuned fixed
  thresholds. Feeds B6: the cross-rung escalation table should be fit from
  the calibration artifacts, not hand-picked per rung.
- **Per-class isotonic calibration as a cascade component** (CEUR Vol-4250):
  per-class (here: per-outcome — accept/verify/abstain) isotonic fits beat
  a single global calibrator in cascade settings. Supports D15's
  full-distribution calibration record and the B4 per-rung artifact split.

Watch: whether "rational cascade" style joint threshold fitting is worth a
tool once B4/B5 land both real calibration artifacts and the model rung.

## 2. Confidence signals for the decision model

- **Logprob-confidence for classification** (arXiv:2512.03816): tracked
  logprobs are a usable calibration signal for small classifiers; supports
  the fork's native verdict-slot readout (F26: 0.8083 with decision-seqs).
- **Token probabilities → calibrated class probabilities** (Fireworks AI
  engineering blog): constrained single-token readout + normalization is a
  standard industrial path — the same shape as the llama.cpp fork's
  verdict-slot logits → Rust-side f64 softmax.
- **Constrained-decoding caution** (ArXiv, Apr 2026): "formatting traps" —
  a constrained readout can collapse probability mass onto the *format*
  token rather than the decision, especially when the prompt template
  shifts. This is exactly why calibration must be refit on any
  prompt/template change (D15 refit discipline) and why cache keys include
  template/policy versions.

Watch: HuggingFace for small decision/classifier models (≤4B) with clean
single-token readouts; gte-modernbert-style embedders with official ONNX
exports for the embedding rung.

## 3. Runtime correction: ONNX Runtime has no Vulkan EP

Key finding (2026-09-30): **ONNX Runtime ships no official Vulkan
execution provider** — CUDA, TensorRT, DirectML, CoreML, ROCm (WebGPU
in preview), QNN only. `ort` on this host (Linux, AMD Vega 8 iGPU, no
CUDA) therefore runs the CPU EP, silently.

Consequences:

- The `opencodifier-model` rung cannot get GPU acceleration through
  ORT+Vulkan; any claim otherwise would be fabricated.
- The pragmatic fast model path on this host is the **llama.cpp Vulkan
  build of the decision fork** (measured 2.23× vs CPU in the Device A/B,
  BENCHMARKS.md) → B5 targets the llama.cpp parallel-decision HTTP backend
  behind `InferenceBackend`, not ORT.
- ORT stays the D11 path for the embedding rung and small exported models
  on CPU; the hybrid conv+recurrent qwen3.5 arch measured ≥9× worse
  latency under ORT (F25), confirming ORT is *not* the first model rung.
- Community `onnx-vulkan-rs` exists; noted, not adopted (unaudited surface
  behind a security-sensitive seam).

## 4. OpenCodifier-relevant repos / implementations to track

- `lawrence3699/jev-style` + chaoliangUNSW 0.8B/2B decision models — the
  Jev-class reference line; guard.py is the #40 blueprint,
  readout_config the D15 artifact reference (see TRAINING.md).
- Gatekeeper / rational-cascade reference implementations on GitHub —
  mine for B6 threshold-fitting shape when the artifacts exist.
- llama.cpp upstream `--parallel` + speculative decisions work — the fork
  tracks this; rebase discipline recorded in BENCHMARKS.md.

## 5. Sweep protocol

Each phase: (a) re-check arXiv listings for "cascade", "early exit",
"abstention", "calibration" (cs.LG) deltas; (b) HuggingFace sweep for
small decision models and ONNX-exported embedders; (c) GitHub sweep of
the repos above; (d) append dated entries here with source IDs — no
undated claims, no citation-free numbers.

## 6. The System-One model board: CLM, Laya, Julia, Strands Decider (2026-10-01 sweep)

Operator-directed sweep of four model families plus the Strands Decider
system. Every source below was fetched and read in full on 2026-10-01
(HuggingFace model cards + `/api/models` metadata, the strandsagents.com
and AWS blog posts, the strands-labs GitHub README). Central result:
**the typed-decision IR is converging across the industry** — all four
families speak state + typed question (`choice` / `noul`≈boolean /
`score`) + per-option probabilities + calibrated confidence, with no
generation. OpenCodifier's IR is not idiosyncratic; it is the emerging
wire shape of the class.

### 6.1 Contrastive-LM CLM-v0.1-8B (and czl GGUF export)

Sources: `huggingface.co/Contrastive-LM/CLM-v0.1-8B`,
`huggingface.co/czl/CLM-v0.1-8B-GGUF` (both read 2026-10-01).

- **Architecture**: frozen Qwen3-8B encoder with last-token pooling +
  two small projection heads (state head, action head), trained with
  bidirectional InfoNCE. Training: ~60M Nemotron QA pairs, ~30M synthetic
  hard negatives, ~1M agentic trajectories. Apache-2.0 (weights and base).
- **It is a System One model in OpenCodifier's exact sense**: "CLM only
  scores the candidates you give it, and its probabilities are relative
  to that set" — candidate-conditioned, no generation. Zero-shot claimed
  ~Jev parity on computer-use/gaming/tool-calling at up to 9× lower
  latency; fine-tuned verifier heads reach SOTA DeepSWE 81.6% and
  Terminal-Bench 2.1 87.6%, 4–6× faster than Jev. API exposes `Choice`,
  `Noul` (boolean), `Score` — our three IR primitives by name.
- **State/action caching lever** (adopt-worthy for B3/B5): states and
  actions are encoded separately, so action (candidate) embeddings are
  reusable across questions — "with ~1k candidates, CLM is 13× faster
  than Jev." Our candidate narrowing already builds per-question
  structures; hoisting candidate representation out of the per-question
  loop is the same shape of win.
- **Encoder-locked heads** (their stated limitation): heads require
  Qwen3-8B last-token-pooled embeddings — the artifact-coupling problem
  our cache keys already model (model/calibration versions fold into the
  key; a head swap must re-key).
- **GGUF export** (czl, community): encoder-only quants with last-token
  pooling baked in (`pooling_type = 3`); heads stay unquantized in
  `.pt`. `clm_config.json` carries "the pooling / embedding / scale
  contract, which no GGUF key expresses on its own" — a manifest
  beside the artifact because the container cannot express the
  contract. Same lesson as our model manifests (D14).

### 6.2 Quantization acceptance gates (czl CLM-GGUF — methodology to copy)

The czl card is a model of quantization honesty; its gates transfer
directly to any model rung we quantize:

- Corpus: 23,926 scored questions; every variant scored **against a
  same-runtime bf16 reference of the same encoder**, never against the
  publisher's vLLM numbers ("otherwise a runtime delta would be reported
  as a quantisation result").
- Pass line: top-1 ≥ the measured bf16-vs-bf16 noise floor of the corpus
  in that runtime, **and** top-1 on *decisive* decisions (reference top-1
  lead > 1 nat) ≥ 0.995, **and** end-task accuracy delta within a 3.0
  point budget. Variants holding all decisive decisions but losing
  end-task accuracy are "usable", not recommended.
- Result: Q8_0 yes; Q6_K/Q5_K_M usable (−2.88/−2.05 pts); **Q4_K_M
  rejected (−6.63 pts)** for this ranking workload.
- **Mechanism worth internalizing**: the head computes
  `argmax(scale · cos)` with `scale = exp(logit_scale) = 100` — cosine
  error is amplified 100× before softmax, so a 0.001 cosine error costs
  0.10 nats and flips "concentrate on near-ties"; mean cosine similarity
  is close to useless as a quality signal. Our verdict-slot readout is a
  different head, but the lesson holds: validate quantized rungs on
  decisive-decision retention and end-task delta, not on embedding-space
  similarity.
- Cross-runtime numerics: llama.cpp bf16 reproduces vLLM argmax but not
  exact margins; two independent bf16 implementations agree to cosine
  0.9987 minimum over 4,448 texts. Bit-identical decisions across
  runtimes is not a real standard; decisive-retention is.

### 6.3 Laya / Mattepiu laya-onnx

Sources: `huggingface.co/Mattepiu/laya-onnx`,
`huggingface.co/convaiinnovations/laya` (referenced card, read
2026-10-01).

- **Architecture**: ModernBERT-large (395M, fully fine-tuned,
  bidirectional) + a from-scratch decision head (2 transformer layers,
  an option-marker scorer, and an **act/escalate head**) = 421M total.
  **Option markers**: every option is scored at its own `[MASK]` marker
  token, softmax over that question's options — the same
  per-candidate-slot readout our decision fork uses (F26 verdict slots;
  the #40 guard.py blueprint). Sequence layout
  `[CLS] <qtype> question [SEP] [MASK] opt0 [MASK] opt1 [SEP] state [SEP]`,
  512-token budget per question, all questions of a state in one forward
  pass (~33–38 ms GPU, ~15 ms CPU claimed).
- **Trained with RLCD** (Reinforcement Learning for Calibrated
  Decisions): the policy outputs a distribution, exploration adds
  zero-mean Gaussian noise to logits, reward is a strictly proper
  scoring rule (log + spherical, ranked probability score for ordinal)
  — maximum reward only at true calibrated probabilities. Plus TD(λ=1)
  over dialogue prefix slices. **Fitted per-cardinality temperature
  calibration** `[1.637, 1.251, 1.983]` with per-option-count scaling —
  independent confirmation of our per-class (D15) and per-rung (B4)
  calibration seams: calibration must condition on the distribution's
  shape, not just the question kind.
- **Numbers** (their table vs published Jev): p50 38.4 ms; in-task macro
  accuracy 0.838, macro ECE 0.060; **zero-shot 0.651 acc / ECE 0.207** —
  calibration and accuracy both collapse off-distribution (matches our
  OOD gate motivation; their eval publishes reliability diagrams and
  risk–coverage curves). Selective automation at 50% coverage with
  confidence ≥ 0.85: 92.2% accuracy — a risk–coverage operating point we
  should replicate in JevBench reporting.
- **The escalate head is B6 inside the model**: the architecture itself
  carries an act/escalate output — a model-native signal for the
  cross-rung escalation our ladder does in configuration. When the ONNX
  rung lands, exposing that head as an OOD/escalation feature (folded
  into `ConfidenceReport::ood_score` or a verifier-agreement channel)
  is a candidate design.
- **ONNX artifact**: `laya.onnx` + `laya_int8.onnx` (421M, CPU-feasible,
  single forward pass) — the strongest ONNX-native model-rung candidate
  found so far for `opencodifier-runtime`'s `InferenceBackend` (the
  qwen3.5 hybrid measured ≥9× worse under ORT, F25; a ModernBERT-class
  bidirectional encoder is the ORT-friendly shape). Marker positions are
  runtime inputs (`marker_pos`, `marker_mask`, `qtype`), i.e. the
  candidate-conditioning is explicit in the graph — mappable to
  `CandidateConditionedModel`. Note: a `run_laya.py` arm exists in
  `benchmarks/decision-model/runner/` (in flight, sibling work).
- Their limitation list independently restates our §73: "Arithmetic,
  counting, date comparisons, and multi-hop index lookups should be kept
  in deterministic code."

### 6.4 SupersonicLabs Julia-1-ONNX

Source: `huggingface.co/SupersonicLabs/Julia-1-ONNX` (read 2026-10-01).

- Same typed surface: state + question + 2–20 options, types
  `choice`/`score`/`noul`; "it is not a text generation model".
  ONNX export with 551 MB external weights, **Rust WASM tokenizer**,
  WebGPU (WGSL) browser inference; original runtime is Python CPU/CUDA.
- **Parity methodology** (copy into our rung validation): 100 real
  validation requests, batches of 4, 5 measured runs after warmup;
  published median 75.47 ms/decision in-browser, **100/100 prediction
  match vs the original runtime, max |Δlogit| 0.00225**. Our
  double-replay bit-determinism check should add a cross-runtime
  max-|Δlogit| metric — same spirit as the czl gates, bounded and
  reported rather than hand-waved.
- Accuracy is reported only for the original runtime (73.15% typed
  decisions, 94% AG News pilot, 86% Emotion pilot) and explicitly **not
  rerun** on the WebGPU export — they do not let a runtime change borrow
  another runtime's accuracy claim.
- "Strict encoding is enabled by default" — schema-strict request
  encoding mirrors `unsupported_generation_field` posture.

### 6.5 Strands Decider 2B (hobson v19) — the head-to-head reference

Sources: `strandsagents.com/blog/introducing-strands-decider/`,
`github.com/strands-labs/strands-decider`,
`huggingface.co/StrandsAgents/strands-decider-2B-hobson-v19` (all read
2026-10-01). The AWS blog post (`aws.amazon.com/blogs/opensource/...`)
does not mention Decider; it covers the wider Strands labs experimental
fleet (robots/simulation) — context only.

- **Architecture (v19)**: Qwen3.5-2B-Base torso, **LM head discarded**,
  replaced by a ~1M-parameter **pointer head**: each option is scored by
  comparing the hidden state at an `<answer>` position against the
  hidden state at that option's last token; one forward pass, no
  decoding loop. Torso adapted with rank-16 LoRA; head in fp32.
  "Because the head holds no per-option parameters, nothing can learn
  that 'the first option is usually right', nothing caps how many
  options a question may carry, and label sets are defined by the
  request rather than baked into the weights" — the strongest published
  statement of why candidate-conditioning beats fixed-label heads.
  v1's slot head (final hidden state → fixed slots) "performed
  significantly worse" — evidence for pointer/marker readouts over slot
  readouts, matching F26's verdict-slot result direction.
- **JevBench public numbers, published**: accuracy **0.723 (167/231)**
  at the 3072 window, 168 at 4096; Brier 0.342–0.349; **ECE 0.050**;
  their tier split easy/standard/hard **1.000 / 0.875 / 0.505**;
  latency median 115 ms / p95 299 ms on RTX 3090, 153 ms warm median on
  M3 Pro. Claims "3rd of 33 in the 2B class".
  **This is the same 231-task benchmark our fork arms run** — the
  fork4b (Qwen3.8-4B-Distill) arm lands directly against a published
  0.723 row, and our earlier Qwen3.5-2B 0.725 ties it. Their tier
  split vs ours (0.95/1.0/0.45 easy/original/hard) shows the same
  hard-tier cliff; tier definitions differ, so compare per-tier
  shapes, not labels.
- **Measurement-noise discipline** (adopt into JevBench reporting):
  six retrains of their v17 recipe gave σ = 3.2 tasks of 231, so they
  treat single-run differences under ~10 tasks as unresolved. Our
  REPORT.md should carry the same caveat on every fork comparison.
- **Calibration posture**: "one temperature per primitive, fitted on
  held-out short classification. The confidence bands are established
  there only: measure on your own traffic before you trust a
  threshold." Their headline reliability claim: at confidence ≥ 0.9,
  answers are right ~95% of the time on held-out short classification —
  a risk–coverage operating point, again matching the selective-
  automation reporting style. Score/noul "transfer poorly" to rubrics
  unlike the training mix.
- **Distillation shape**: teacher = frozen Qwen3.5-4B output
  distributions used as training targets, directly or through a parent
  model trained first — same 4B→2B distillation axis as our Qwen3.5/3.8
  arms. Reproducibility contract: `provenance.json` + `MANIFEST.sha256`
  + `python -m strands_decider.hf_export verify <folder>`; preregistered
  run records ("every training run states its predictions and its
  failure conditions before training... a run that misses its bar does
  not replace the reference model") — a governance pattern for our
  TRAINING.md record.
- **Integration pattern**: a `before_tool_call` intervention handler
  gates a tool call on two cheap noul decisions (argument-grounding,
  premature-call) with typed actions Proceed/Deny/Confirm/Guide — "a
  decision this cheap can sit in a path where an LLM call never could."
  This is the ladder's cheapest-first principle, implemented as
  intervention hooks; the typed-action vocabulary (proceed/deny/
  confirm/guide) is a candidate shape for B6 escalation outcomes.
  Server surface: `POST /v1/systemone`, loopback bind, no auth —
  same endpoint vocabulary the Jev bridge already targets.

### 6.6 Actionable deltas for the B-series

1. **B5 model rung** — the board now has two CPU-feasible ONNX-native
   candidates with marker/pointer readouts (Laya 421M; the fork's
   verdict-slot line) and one llama.cpp-native pointer head (Strands
   v19). Keep B5 on the llama.cpp fork path (this host's measured
   fastest lane) and evaluate the Laya ONNX export as the
   `InferenceBackend` second arm; adopt the czl decisive-retention +
   end-task-delta gates and the Julia max-|Δlogit| parity metric for
   whichever lands first.
2. **B3/B5 caching** — CLM's state/action split (13× at 1k candidates)
   argues for hoisting candidate-side representation out of
   per-question loops in the narrowing and model rungs.
3. **B6 escalation** — three concrete shapes to mine: Laya's act/
   escalate head (model-native), Strands' typed intervention actions
   (proceed/deny/confirm/guide), and the risk–coverage operating points
   (accuracy at fixed coverage/confidence floor) both competitors
   publish. Report JevBench rows with a coverage column so operating
   points are comparable.
4. **Reporting discipline** — carry the σ ≈ 3.2/231 retrain-noise
   caveat on all single-run fork comparisons; never compare accuracy
   across runtimes; publish ECE and Brier beside accuracy (both
   competitors do; our D15 already requires it).
5. **Watch** — CLM-35B "early October"; Strands v20+ iterations; the
   `contrastive-lm` head-training recipe (cheap per-task verifier
   heads) as a candidate for OpenCodifier's own rung fine-tuning
   story (TRAINING.md).

## 7. Hierarchical Reasoning Model and its text adaptations (2026-10-01 sweep)

Sources (all read 2026-10-01): `github.com/sapientinc/HRM` (paper:
arXiv:2506.21734), `huggingface.co/sinimiini/HRM-Text-1B-GGUF`,
`huggingface.co/vonjack/hrm-text-agent-gguf`.

### 7.1 HRM itself — architecture ideas, not a rung

- 27M parameters, two interdependent recurrent modules (high-level slow
  abstract planning; low-level fast detailed computation), deep
  effective computation in **one forward pass** with no CoT supervision,
  trained from ~1,000 examples without pre-training: near-perfect
  Sudoku-Extreme and maze optima, and ARC-AGI results ahead of far
  larger CoT models. Checkpoints are task-specific (ARC-2 / Sudoku /
  Maze) — a puzzle solver family, **not** a general decision model;
  the transferable content is architectural.
- **The halt head is B6 inside the model**: HRM trains an adaptive
  halting policy (`arch.halt_max_steps=8`, Q-learning halting) — the
  model itself decides whether to keep computing or to stop. That is
  the abstain/escalate decision as a learned signal, the same role
  Laya's act/escalate head plays (§6.3). Cross-rung escalation (B6)
  should treat the halt/continue probability as a first-class
  confidence feature, not just post-hoc thresholds.
- Recurrent depth is the latency dial: `L_cycles` sets compute depth
  per question — a rung whose cost is a knob between "one lexical
  pass" and "an external model". If a small recurrent decision model
  ever lands in `opencodifier-runtime`, its depth setting is exactly
  the kind of per-rung cost/gate trade the ladder exists to govern.
- Small-sample discipline: "accuracy variance of around ±2 points"
  across small-sample runs, and a documented late-stage-overfitting
  instability with an early-stop recommendation — the same
  retrain-noise honesty Strands publishes (§6.5).

### 7.2 HRM-Text GGUF exports — serving-contract lessons for B5

`sinimiini/HRM-Text-1B-GGUF` and `vonjack/hrm-text-agent-gguf` adapt
HRM to text as a 1B **PrefixLM** (bidirectional attention over the
prompt via `token_type_ids`), non-chat, non-instruction-tuned. Neither
loads in stock llama.cpp: a custom `hrm_text` GGUF architecture, a
patch pinned to one validated upstream commit, and the recurrent latent
tensor carried in F32.

- **Serving constraints that will bind any PrefixLM decision rung**:
  keep the complete prompt in one physical batch
  (`--batch-size`/`--ubatch-size` ≥ prompt tokens); keep
  `cache_prompt` disabled — no KV reuse from a shorter prompt, no
  speculative decoding. Our fork's parallel-decision server already
  has the right shape (single-pass verdict slots, no decode loop);
  the lesson is that a PrefixLM/encoder-class rung must refuse KV
  reuse by contract, which is a `--no-cache-*`-style flag pairing plus
  a manifest statement, not a hope.
- **The template is part of the model contract**: the agent export
  embeds each expert's required Jinja condition in its GGUF
  (`tokenizer.chat_template`) — the Code expert only runs in its
  `synth,cot` condition, Agent v2 in `direct`. Running a model in a
  prompt shape it was not conditioned on is out-of-distribution by
  construction; the honest deployment reads the embedded template and
  refuses a mismatch, the way czl's `clm_config.json` carries the
  pooling contract the GGUF keys cannot express (§6.1).
- **Expert routing, never merging**: "The code model and tool model
  are separate experts. Route tasks between them; the author found
  that merging their weights destroys one skill or the other."
  Independent confirmation of the per-rung/per-domain artifact model
  (D25 rungs, F24 per-domain calibration): capability lives in
  separate versioned artifacts selected by a deterministic router.
- **Validation methodology to copy for our future model exports**:
  tensor-level comparison (259/259), prompt token-ID equality,
  next-token top-1 and top-10 overlap per prompt, full-vocab
  mean-absolute-logit-error per prompt, and the explicit rule that
  "repetition by itself is not a conversion failure unless it is
  newly introduced by the runtime" — every quant validated against
  the BF16 conversion, and the BF16 conversion against the source
  F32, never a quant against a different runtime's numbers (same
  discipline as §6.2).
- **Honesty case study**: the vonjack card dissects an older
  community checkpoint whose "38k examples" was a one-epoch pilot of
  a code corpus with zero tool-call markers, and labels it
  "unevaluated Stage A code pilot with unstable quality" instead of
  letting the number stand. Model-card claims are audited against
  training records before becoming comparison rows — the standard our
  own BENCHMARKS.md provenance column already applies to incoming
  rows.
- License note: the Jason (jasoncarreira) HRM-Text code/agent
  checkpoints are **CC-BY-NC-4.0** — study and benchmark-compare, do
  not ship or redistribute weights (D14's discipline, extended to
  licensing).

### 7.3 Prompt and template levers — what would actually move results

Operator question (2026-10-01): can chat-template changes, better
prompting, or similar mechanisms improve the model arms' scores?
Grounded answer, separating what is measured from what is plausible:

- **The decision prompt is already the lever with the largest measured
  effect in-house.** The params A/B (REPORT.md, 2026-10-01) stripped
  prompt evidence (diet200) and accuracy fell 0.800 → 0.633 with
  metadata_match 1.0 → 0.525 — prompt *content* is first-order, worth
  more than any threshold tuning. The general form of the lever:
  more/better-ordered evidence in context, not different phrasing of
  the instruction.
- **Condition/template fidelity is an OOD guard, not a tune.** For
  instruct/distill arms (fork4b = Qwen3.8-4B-Distill), serving through
  the GGUF's embedded `--jinja` template versus a raw-completion
  envelope are different prompt distributions; the model was trained
  for one of them. A/B-ing template-on vs template-off on the current
  chain's arm is a cheap, legitimate experiment — as is a minimal
  system message ("decide; answer with one option") which instruct
  models often need to stop from narrating. Base-model arms (the 2B
  fork) have no template by construction; for them the decision
  envelope itself is the contract (§7.2).
- **Question phrasing is a weak channel — with a published warning.**
  Strands' own limitation section (§6.5): "questions are read less
  than documents; with the state and options fixed, a changed question
  often gets the same answer." Prompt-engineering the question text is
  therefore unlikely to buy much for pointer-head models, and tuning
  phrasing *against JevBench* would be benchmark contamination.
  Anything in this direction tunes on a held-out slice only (the
  internal boardgame/musique-style sets), never on the scored set.
- **Candidate order is the untested structural lever.** Pointer/marker
  heads score each option at its own slot; position bias is plausible
  and our rerank seam is explicitly "permute, never remove." A cheap
  study: replay one arm's run JSONs with deterministic candidate-order
  permutations and measure answer-flip rate; if flips are non-trivial,
  adopt a deterministic canonical order (and record it in the rung's
  manifest, since it changes the prompt contract and therefore the
  cache key).
  **Post-hoc half done (2026-10-02, `results/position-bias.md`,
  `runner/position_bias.py` — zero inference):** indexing each arm's
  winners against its tasks' ordered candidate lists (shares normalized
  per cardinality against the benchmark's own gold-position
  distribution) shows the prior is rung-specific, not universal. The
  decision fork is nearly position-neutral — position 0 captures 0.289
  of normalized share vs 0.262 gold (+2.7 pp), and its accuracy-by-gold
  -position curve has no primacy slope. The embedding rung (vtx) has a
  strong primacy prior — position 0 takes 0.413 vs 0.262 gold
  (+15.1 pp), starving position 2 (0.112 vs 0.216) — so for THAT rung
  the permutation replay is worth a server slot, and a deterministic
  canonical order (or order-permutation TTA) is a real candidate fix.
  The engine rung shows the third shape: no position-0 excess
  (0.255 vs 0.262) but it *never* selects position 5 (0.000 vs 0.025)
  and under-selects position 2 — an evaluation-order artifact of
  rule/lexical matching, i.e. the bias lives in the mechanism, not a
  learned slot preference. The permutation replay (flip-rate under
  canonical reordering) remains queued for a server slot; its value is
  now concentrated on the vtx rung.
- **Per-cardinality calibration is prompt-shape-aware calibration.**
  Laya fits temperatures per option-count (§6.3) — the number of
  candidates is a prompt-shape variable that shifts distribution
  spread. Our artifacts key on question class; adding option-count
  buckets to the fit script (`fit_calibration.py`) is a small,
  measurable refinement consistent with D15's schema (class keys are
  arbitrary strings).
  **Done as analysis evidence (2026-10-02, CALIBRATION.md):** the
  fitter joins suite cardinalities from the seed-deterministic
  generator and fits per bucket. Confirmed on our own data — the
  margin-fit 4B fork of record wants T 1.44 at k=4 vs **0.15 at k=5**
  (raw winner mass dilutes across options, so the winner-gate channel
  reads badly underconfident on wide boards); the exact-fit 2B board
  is flatter (k5 1.49 around a global 0.93) and is the shape to trust,
  since the margin proxy over-sharpens multi-way boards (the
  synthetic-grid finding). Shipped artifacts unchanged — buckets ship
  as engine keys only when the engine can address them.
- **What prompting cannot fix here**: the fork's single-token
  verdict-slot readout has no generation, so the CoT-prompting axis
  and the constrained-decoding "formatting traps" failure mode
  (§1, §2 of this file) do not exist on the fork path by construction
  — that is the readout's whole point. If an arm ever needs
  generation-shaped prompting to score well, that is evidence the
  *rung is wrong* (the model is being asked to be a generator), not
  that the prompt needs work.

**Actionable deltas, resolved**: (1) template A/B **landed 2026-10-02**
(`fork_4b-instr-*` on the anchor board): decide-template 0.7662 ==
anchor default, rules-template 0.7749 and task-template 0.7576 — all
within single-board noise of each other; nothing adopted, default
instructions retained (BENCHMARKS.md table). (2) candidate-order —
post-hoc position-prior half DONE (fork +2.7 pp / vtx +15.1 pp at
position 0 / engine never-position-5); the flip-rate replay needs
re-inference under permuted orders (stored rows are label-only, and
even probability-bearing rows hold logits computed under the original
order, so no post-hoc remap can measure it) — handed to the new-arm
queue, value concentrated on vtx. (3) per-cardinality buckets
**implemented** in `fit_calibration.py` (2026-10-02, committed
6f90bfd, CALIBRATION.md): the d15 re-run provides the first
JevBench-native probability channel to exercise the exact-fit path
with buckets. (4) when a PrefixLM/encoder rung lands, encode the
no-KV-reuse and one-batch-prefill constraints as flags + manifest
statements (§7.2).

## §9 — Known-knowns audit + external sweep (2026-10-02)

Three parallel sweeps (arXiv cascade/abstention/calibration; HF/GitHub
watch-list; implementation-focused GitHub) against our measured pain
points. Headline convergence: **our four failure modes are one defect
seen from four angles** — the readout has no null mass, no per-slice
certificate, no cardinality invariance, no rotation invariance.

### §9.1 Internal known-knowns (verified in code/data this pass)

- **The OOD confidence channel is entropy-only.**
  `distributional_ood` (executor.rs) returns normalized entropy of the
  answer distribution — self-referential; it carries zero
  input-likeness evidence. The policy seam (`Policy::with_ood_ceiling`,
  core/policy.rs) is wired and waiting. The JevBench cascade preview
  proves the cost: a 3-rung engine→vtx→fork_4b fusion tops out at
  **0.636 — below fork-alone 0.667** (oracle 0.823) because the engine
  rung admits 81% of adversarial items at t=0.50 with a 0.377-accuracy
  rung and ECE 0.402; its confidence cannot rank its own answers
  off-distribution. **Routing, not rungs, is the unlock on adversarial
  prose.** Candidate deterministic input-likeness features: BM25
  best-match score mass vs the calibration corpus band; candidate- vs
  category-level marginal disagreement (TV > τ, label-free, §9.2-3);
  conformal prediction-set size (§9.2-7).
- **BM25 is built 4× per question on the default path**
  (narrowing.rs once; classifier.rs three more times: query scoring,
  support scoring, label scoring). B3's build-once handoff never
  reached the classifier path. Fedora re-baseline (in flight) gives
  the fair per-stage numbers; build-once is the follow-up.
- **Never-abstain-at-bottom is unclaimed value**: 19% of JevBench
  engine abstentions scored incorrect while a 0.667-accuracy rung sat
  below. Escalation should be exhausted before abstention (B6 config
  values, not code).
- **Fusion of record stops at 2B** (suite ladder 0.867 @ 753 ms);
  adding the 4B fork rung and per-family coverage columns
  (§6.6-3) is a zero-inference re-run once today's arms land.

### §9.2 External findings (each: claim → verdict for us)

1. **Mainline llama.cpp merged decision-model support — PR #29818
   `/v1/systemone`, merged 2026-10-02** (`server_decision_context`,
   `{arch}.decision.type` GGUF metadata; choice/noul/score types;
   parallel shared prompt prefix; noul = P(yes); score = level
   probabilities + expected score). Ten ggml-org GGUFs landed Oct 1–2
   (Laya, Kev-0.8B/4B/9B, OpenJev, Clef, lev, Julia-1,
   Bespoke-Nimble-9B-v3). Also: native `/completion` with
   `temperature < 0` (greedy) + `logit_bias` on choice tokens +
   `n_probs = K` + `min_keep = K` yields a **full choice distribution
   in one decode step on stock llama.cpp** — no tree, no fork.
   ACTIONABLE: (a) build stock master, run Kev-4B / OpenJev /
   Laya-GGUF through the 231 harness via `/v1/systemone`; (b) diff
   biased-greedy single-step distributions against the fork on 231 —
   parity would drop the fork from our build entirely (encoder-class
   immediately; causal readouts after the diff).
2. **New local-arm candidates above our best (0.667)**:
   strands-decider-2B-hobson-v19 — **0.723 on the same 231 split**,
   Brier 0.348, **ECE 0.050**, with an onnx-community export shipping
   a conversion/parity harness (drops into `opencodifier-runtime`
   behind `onnx`); jebadiah-4b-v2 (LoRA on Qwen3.5-4B-chat, same
   base + same verdict-slot readout as our fork — cleanest
   apples-to-apples against our recipe; per-type temperatures
   published: choice 1.12 / noul 1.33 / score 0.83) and
   jebadiah-27b (0.866 on our split, maintainer-reported — ties
   hosted Jev); Winnow-E4B (claims **80.52% on the 231 public
   subset**, 2.2× throughput of its 12B sibling). CLM-35B still
   unreleased (org holds CLM-8B only; re-check ~Oct 6–10); CLM-8B
   itself is weak on decision accuracy (30.0 Cap) but the staged
   hard-negative contrastive-head recipe (hard-neg top-1 52.1→69.2
   only when staged; replay holds teacher 68.5 vs 56.2) is the best
   leverage on our 41% vtx rung. ACTIONABLE: four benchmark arms
   queued behind tonight's measurement queue; ONNX parity fixtures
   from the strands export double as IR/adapter conformance tests.
3. **Sys1Cal (arXiv:2609.35342) — suppressed null mass is the
   overconfidence mechanism**: Choice-style readouts force
   P(A)+P(¬A)=1 with nowhere for P(U); recovering the null lifts
   soft accuracy 0.771 → 0.978. ACTIONABLE: k+1 candidate
   construction (explicit abstain candidate) whose mass feeds the
   confidence gate as a calibrated OOD feature — touches IR
   candidate construction, not the ladder.
4. **Conformal cascade (arXiv:2607.25018) + UCCI (arXiv:2605.18796)
   — the gate upgrade**: accept iff the calibrated prediction set
   collapses to one (distribution-free coverage ≥ 1−Kα per tier);
   UCCI adds isotonic margin→error-probability with
   cost-constrained thresholds (−31% cost at F1 0.91, ECE
   0.12 → 0.03, production-scale). Both map 1:1 onto `LadderPolicy`
   per-(kind, cardinality) overrides. Caveat (arXiv:2506.18162):
   conformal miscoverage under covariate shift — calibrate the
   nonconformity score on a held-out adversarial slice, never the
   pooled suite. Pairs with arXiv:2608.05064: **8/22 model-task
   pairs hit the temperature-scaling infeasibility floor** — our
   T=0.15 @ k5 margin fit is that signature, and the fix is
   per-(arm × cardinality) Platt/isotonic, not a better T.
5. **Rotation ensembling (arXiv:2609.30454)**: averaging choice
   probabilities over k cyclic option rotations gains +3.8 pp
   (37.4% of one MC set flips answer under rotation!), and applying
   it only to low-margin items recovers most of the gain at a
   fraction of the cost. Our `--decision-seqs 24` batch absorbs the
   rotations — test-time compute *within* a rung, no rung jump,
   doubles as the position-bias fix. Slot into `LadderPolicy` as a
   per-node override. Boundary condition from arXiv:2607.20864:
   position bias is measurable only in the 60–95% base-accuracy
   band — measure the +15.1 pp vtx effect inside that band.
6. **model2vec-rs + self-distillation (MinishLab)**: official Rust
   crate (int8, from_bytes, 8k samples/s/thread); distill() runs
   ~30 s on CPU over our own vocabulary; potion-base-32M *beats its
   own teacher* on classification (71.70 vs 69.25). Repairs the
   vtx supply-chain gap (vendor position-gated pooler is
   irreproducible from the shipped table) with an artifact we own,
   version into cache keys. MRL-truncated scoring
   (static-retrieval-mrl-en-v1) = score at 64–128 dims, escalate on
   thin margins.
7. **Coherence TV (arXiv:2609.33971)**: candidate-level vs
   category-level total-variation disagreement is a label-free OOD
   trigger; and their decoupling result (Laya +9.2 acc while ECE
   0.046 → 0.124) is a standing warning — never transfer a
   calibration fit across arms (already D15 doctrine).
8. **Bongard (arXiv:2609.39111)** — the training-time fix for
   adversarial-prose robustness: joint-embedding post-training
   lifts rephrasing robustness 75.7 → 85.9. Adopt as a fine-tune
   stage (KL between option distributions under paraphrase, head
   frozen) for whichever decision model wins item 2's bake-off.
9. **Fastino GLiNER2.5-Decide family (2026-09-24, Apache-2.0) —
   third-party validation of the ladder's model rung shape, and a
   real encoder-rung candidate.** `fastino/GLiNER2.5-Decide` is a
   340M DeBERTa-v3-large *encoder* decision model: label set passed
   at call time (schema-driven, §63-shaped), several heads scored in
   one forward pass, zero generated tokens, ordinal scales as string
   levels ("0".."10") and described labels supported; explicitly not
   general-purpose (no reasoning/explanations). Their
   `fastino/fast-decisions` leaderboard (held-out exact match, 17
   domains × 300): GLiNER2.5-Decide **60.2** > GLiNER2 XL 1B 59.6 >
   JevK5 57.6 > multi-Decide (287M mDeBERTa, "boundary"
   propose-then-rerank arch) 56.7 > SemIf (Qwen3.5-4B) 56.4 >
   GLiFormer 49.0 > Laya Router 46.6 — our fleet overlaps theirs
   (Laya is a queued arm; Qwen3.5-4B is our distill base), so the
   suites cross-check. The GGUF port
   (`3ntr0py-t4m3r/GLiNER2.5-Decide-GGUF`, f32/Q8_0 465 MB/Q4_K) is
   **not** a drop-in: parity vs torch fails (cos 0.87–0.93 vs gate
   >0.9999; the disentangled-attention relative-position term is
   ~3 orders under-weighted; 1–3-token inputs exact, divergence from
   4+ tokens), the 2.1M-param decision head ships separately (run
   host-side, keep f32), and the patch targets the kade/llama.cpp
   fork (`spec-multi-output`), not upstream. Latency is flat across
   quants (activation-bound; ~33 ms/req on one 4090). Two things to
   steal: (a) **calibration doctrine, independently restated** — the
   card measures its own ship state as uncalibrated (T=1.0, argmax
   correct on 6/6 with mean top-prob 0.628), rejects a single global
   temperature (per-task logit spreads differ ~10×), and gates
   adoption on **mean ECE < 0.15 per (task type, label count,
   activation) bucket plus non-inverted reliability curves** — the
   same per-(kind, cardinality) bucketing as §9.3-3; (b) **the
   encoder rung**: a plain transformer encoder is the opposite ORT
   failure mode from the qwen3.5 hybrid conv+recurrent arch
   (F25: ≥9× latency) — encoder + Rust-f64 head (4 tensors) behind
   the existing `onnx` feature is a credible sub-100 ms CPU model
   rung. The dataset (17 × 100 dev rows public, test 300/domain held
   out; `{"input", "classifications": [{task, true_label, labels,
   multi_label}]}`) maps 1:1 onto Choice IR (task→question,
   labels→candidates, true_label→gold; multi-label compared as sets;
   ordinals → Score ordered levels) — an extra bake-off slice, with
   the standing disclosure that dev-split numbers are not comparable
   to the published leaderboard. Company context: Fastino (Palo
   Alto, $25M raise) sells this exact shape as "TLMs" — task-specific
   scored decisions, "<100 ms", "route on confidence rather than
   parse prose" (GLiDE demo), exposed through a Jev-style
   `POST /v1/systemone`. ACTIONABLE: (a) eval-only reference arm via
   the `gliner2` pip package on Fedora scoring JevBench-231 +
   fast-decisions-dev; (b) ONNX export study (encoder under `ort`,
   head in Rust f64) as the candidate CPU model rung; (c) adopt the
   ECE-per-bucket + non-inverted-reliability gate wording for any
   adopted arm; (d) re-check the GGUF port after an upstream parity
   fix.

### §9.3 Priority queue (replaces the §6.6 deltas' ordering)

1. Benchmark arms: Winnow-E4B, Kev-4B, strands-2B ONNX,
   jebadiah-4b-v2 on the 231 harness (Fedora queue, after tonight).
2. Stock-llama.cpp parity diff (biased-greedy single-step vs fork)
   — potential fork retirement.
3. Gate upgrade: conformal set-size acceptance per
   (kind, cardinality) + k+1 null candidate + isotonic margin
   table; calibrate on a held-out adversarial slice.
4. Rotation ensembling behind a low-margin per-node override.
5. Self-distilled model2vec rung (model2vec-rs, int8) replacing
   vtx; own pooler spec; version into cache keys.
6. Engine internals: wire a real input-likeness OOD feature
   (lexical-band / TV / set-size) through `ood_ceiling`; BM25
   build-once completion. **LANDED** (2026-10-02): D28 lexical-band
   OOD channel (d7eefdf); BM25 build-once handoff to the deciding
   classifier (0bfcf33).
7. Fastino GLiNER2.5-Decide arms (§9-9): `gliner2` reference arm on
   the 231 harness + fast-decisions-dev slice (eval-only, Fedora);
   ONNX encoder + Rust-f64-head rung study behind the `onnx`
   feature; ECE-per-bucket adoption gate. GGUF port deferred until
   upstream parity passes.
8. Vision path (2026-10-03, extended after the SigLIP 2 + OCR survey):
   - **V1 = Winnow-E4B's own mmproj**, at the best 4-bit the quantize
     tool accepts (r11 arms); fallback if llama-quantize cannot
     quantize the tower is stock mmproj. External evidence backs the
     fallback as the likely pick: the Unlimited-OCR GGUF repo keeps
     its vision encoder at **F16 (774 MiB) deliberately** — "it is
     small and quantizing it hurts OCR accuracy" — and its own
     limitations section says prefer Q4_K_M or higher, converging
     with the 2026-10-03 operator quant floor. Tower-quant stays an
     arm, never the default; the tower is small next to the body.
   - **OCR ≠ vision understanding — two different rungs.** OCR is
     text extraction: a *preprocessing* rung that turns image state
     into text state, after which the existing deterministic ladder
     decides. Visual scene understanding ("is this a failing-test
     screenshot?") is mmproj-class. SigLIP2-class encoders do the
     second; only OCR models do the first.
   - **Unlimited-OCR-GGUF** (sahilchachra; baidu/Unlimited-OCR, MIT;
     DeepSeek-OCR arch: SAM-ViT-B + CLIP-L/14 DeepEncoder → linear
     projector → DeepSeek-V2 MoE 3B): strong shape — grounded
     markdown with `<|det|>` boxes, `--temp 0` deterministic,
     Q4_K_M 1.82 GiB recommended, serves via llama-server
     OpenAI-compatible image_url (matches the Amortyx registration
     pattern). **Parked, two blockers:** (a) requires the PR #17400
     llama.cpp branch — not merged to upstream main, stock llama.cpp
     will not load these files, and a second llama.cpp build breaks
     our single-runtime discipline; (b) a second resident 3B model
     doubles footprint for a rung that only fires on image input.
     Revisit when DeepSeek-OCR lands in upstream main.
   - **SigLIP2 NaFlex** as a standalone ONNX vision-embedding rung:
     the naming is Base 86M / Large 303M / so400m ("shape-optimized")
     400M / Giant 1B, with **NaFlex = dynamic-resolution variants**
     (FlexiViT/NaViT lineage) of Base and so400m — one model for
     aspect-sensitive inputs (native aspect ratio, little distortion)
     and document resolution. Beats SigLIP 1 at all scales on
     zero-shot classification, retrieval, and VLM transfer. It is an
     **encoder**: embeddings + zero-shot label matching only — no
     text extraction, no captioning; the text side is trained at a
     64-token max_length, so longer candidate labels must be chunked
     (author guidance: chunk to 64 tokens, average/max the scores).
     Fit: ~86M tower ≈ 90 MB int8 ONNX behind `EmbeddingBackend`,
     feeding the embedding→lexical ladder for image triage — NOT a
     drop-in for the Winnow mmproj (its projector is trained to
     Winnow's space; a swap means projector training, a
     §9-distill-program task). Gemma-4's stock tower is SigLIP-family
     so the lineage fits; embedding spaces do not transfer zero-shot.

