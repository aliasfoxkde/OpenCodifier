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

**Verified 2026-10-05 (#92): the full encoding contract is reproduced.**
The ONNX repo's own `parity.py` points at
`julia.data.sequence(tokenizer, row, 1024, 256, strict=True)` in the
parent checkpoint repo; that function, reimplemented with plain
`tokenizers` + numpy (no torch), reproduces the published parity exactly:

- Sequence: `[CLS] head[:budget] [SEP] ([MASK] " "+opt[:48])* [SEP]
  state[:room] [SEP]` — CLS/SEP/MASK are `<bos>`(2)/`<eos>`(1)/`<mask>`(4);
  head is plain text `"{type} question: {question}"` (the qtype rides BOTH
  on that text prefix and on the int input — there is no qtype marker
  token); head budget 256 with `max(8, ·)` floor; `max_length` 1024;
  state raw-encoded, no leading space, room = `max_length − len − 1`.
- Graph inputs: `marker_pos` = 0-indexed positions of each option's
  `[MASK]`, `marker_mask` all-true over K options, `qtype` int
  `{choice: 0, score: 1, noul: 2}` (embedding table size 3).
- Reproduction: **100/100 prediction match, max |Δlogit| 7.82013e-05**
  vs their published `parity-cpu.json` 7.82012939453125e-05 — exact.

**Viability on the JevBench public 231** (zero-shot probe, criteria
rendered `"label: desc"`, noul as `[no, yes]`): **acc 0.4416, macro
0.4190, Brier 0.888, ECE 0.3915, p50 40.9 ms** CPU (ORT 1.30,
`runs/onnx-julia1-jevbench-v1/` on the eval host). Above the engine-only
row (0.3766) and vtx (0.4113), far below the bridge (0.6494), Laya's
published 58.4 %, and Julia's own original-runtime numbers (73.15 %
typed decisions) — our zero-shot rendering is unvalidated against
theirs, so read it as a floor, not a model verdict. Decision recorded
for #92: **not competitive as a standalone arm, and ECE 0.39 disqualifies
it as a gate candidate**; the exact encoding contract above is the
durable artifact (it is the Laya-family runtime-input convention,
working reference for any marker-encoder rung we build or import).

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

- **Verified locally (2026-10-05, contract + corpus).** The full
  upstream contract was extracted from their own `prompting.py` /
  `infer.py` / `data.py` and reproduced with plain `tokenizers` +
  numpy against the shipped q8 ONNX:
  - **Rendering**: `<state>\n{content}\n</state>\n` +
    `<question type="{kind}">` with headers ("Decide whether the
    statement is true of the state." / "Select exactly one option." /
    "Rate the state against the ordered levels below (lowest first)."),
    numbered one-line options `N. name — desc` (em-dash, desc
    space-collapsed), `<answer>` tail; JSON state via
    `json.dumps(indent=2, sort_keys=False)`; no BOS; `pad_token_id`
    248044.
  - **Packing**: question-first reserve `min(len(q), 0.75·4096)`;
    question front-truncated with offsets AND pointer indices shifted
    by the cut; state budget `4096 − reserve`.
  - **Readout**: `option_pos` = LAST token index of each option's
    char-span **plus `len(state_ids)`**; `answer_pos` = L−1; logits
    divided by per-kind temperature (noul 0.9107, choice 0.7342,
    score 1.3278) then softmax.
  - **Fixture parity**: 11 recorded cases, worst Δprob 0.0255, zero
    argmax flips — inside their published q8 tolerance band
    (0.016–0.026). Full-pipeline e2e (own render → own tokenize → own
    index math → graph): same 0.0255 / 0.
  - **JevBench 231 probe (corrected run, `runs/strands2b-jevbench-v1/`)**:
    **acc 0.7273, macro 0.7266, Brier 0.3449, ECE 0.0628**, p50 4.71 s
    under host load (~2.8 s unloaded, ORT CPU) — against their
    published 0.723 / Brier 0.342–0.349 / ECE 0.050: **reproduced**.
    ECE 0.063 passes where Julia-1's 0.39 failed; latency makes this
    an escalation/verifier rung candidate, never a per-request decider
    (the deterministic engine serves the same split at 5.3 ms p50).
    Family shape: extraction/fact/intent/ordinal/tool_selection 1.0,
    temporal_numeric 0.20, long_policy 0.32, probability 0.40 — the
    long-context and numeric families are where the 2B torso gives out.
  - **Token distribution over the 231** (recorded `prompt_tokens`,
    first datapoint for adaptive context allocation): p50 143, mean
    661, p95 2717, max 3936 against the 4096 ceiling; 82.3 % of items
    fit under 1024 tokens, 3.5 % exceed 3072. Per-question-type p50s
    are indistinguishable (143/213/137); per-family p50s span 101
    (extraction) to 2642 (long_policy, multi_hop) — the allocation
    signal is the deterministic question class, not the wire type.
    A static 4096 window buys the tail at ~28× the median cost on
    every fused forward pass.
  - **The v1 probe incident (0.2251, documented per request)**: the
    first run passed question-relative `option_pos` without the
    `len(state_ids)` base, so the pointer head read hidden states
    inside the state text — **0.2251 accuracy, below chance, at mean
    top-prob 0.558**: pointer readouts fail non-gracefully, producing
    confident nonsense, not errors. A second latent bug
    (truncated-question pointers not shifted by the cut) was fixed in
    the same pass. Neither was caught by anything until the number
    itself was flagged as weird; the fix is four-layer validation
    (graph invocation → tokenization → span math → composition),
    in-pipeline pointer assertions (count == options, base ≤ p < L,
    strictly increasing), and the standing rule: sub-chance accuracy
    is a bug alarm, never a model verdict. Codified in the harness
    rules (backend-fixed `.claude/rules/quality.md`, 2026-10-05).
    Broken runs archived as `results.wrongbase.jsonl` next to the
    corrected artifacts; the corrected run reproduces the published
    row.

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

---

## §10 — Performance & accuracy methods sweep (2026-10-04)

Three parallel research sweeps, each source-verified (agent reports persisted at
`/nas/Temp/work/oc-model-eval/runs/research-{llamacpp-cpu-perf,selective-conformal,
jointhead-cascade}-20261004.md` — 98 / 7 / 56 tool calls respectively). Scope:
(a) CPU inference performance for the prefill-dominated letters lane, (b)
selective prediction / conformal / calibration for the confidence gate (verified
against our local `policy.rs` / `confidence.rs` / `calibration.rs` integration
surface), (c) family-A joint-head + cascade/routing code references. Only the
load-bearing findings are recorded here; the work-dir reports carry the full
detail with per-claim sources.

### §10.1 Thread sweep on the 13600K: -t 12 wins prefill (corrected finding)

CORRECTION (same day, 2026-10-04 — recorded because the error is
instructive). The first version of this section claimed an "AVX-VNNI defect":
that our bench binaries lacked VNNI because the trees were configured on the
NAS (Zen2, no VNNI) and shipped to fedora. That claim was a verification
artifact — `objdump` was run on `bin/llama-server`, which in this build
layout is a thin wrapper; the quant kernels live in `bin/libggml-cpu.so`.
Corrected proof: `vpdpbusd` sites are present in ALL builds (build-stock 357,
build-pd 393, fresh build-stock-r18 357) — **VNNI was always engaged; no
rebuild is warranted**. The upstream measurements (PR #25346: +32% pp512
i5-13420H; PR #27851: +9.1% pp2048 9950X3D) remain valid context but describe
a failure mode we do not have. Lesson: verify ISA claims against the shared
object that carries the kernels, and let the runtime's own `system_info`
banner be ground truth (`llama-bench -o csv` suppresses it).

What r18 did establish, with measurement (`runs/r18_bench_vnni_threads.csv`,
gemma-4-E2B QAT Q4_0, `-fa on`, pp400/n1, 3 reps):

- **`-t 12` beats `-t 6` on prefill: 229.6 ± 7.8 vs 203.0 ± 3.8 t/s
  (+13.1%)**; decode (tg1) marginally favors t=6 (21.8 vs 20.6 t/s) —
  irrelevant to single-token readout. The runner's hardcoded `-t 12` is
  validated by measurement; llama.cpp's hybrid default (6 P-cores, E-cores
  and HT skipped) does not transfer to this prefill-dominated workload. No
  published 13600K sweep existed; this closes it for our config.
- Absolute prefill anchor for the 0.808 operating point: **~230 t/s at
  pp400** on the E2B QAT Q4_0 (D9 context).

### §10.2 Thread count: the runner's `-t 12` overrides llama.cpp's own hybrid default

`common/common.cpp` skips E-cores ("efficiency cores harm lockstep threading")
and HT siblings ("hyperthreading isn't useful for linear algebra") → the
13600K's llama.cpp default is **-t 6** (6 P-cores, no HT, no E); our runner
hardcodes 12. Upstream measured: P-core 3.8× E-core single-thread prefill (PR
#23309); SMT oversubscription 3.6× decode regression (#19110/#19260); +3.8%
pp64 from 8→16 threads while tg16 *drops* 8.3% (llama-bench README table). No
published -t sweep for a 13600K exists — r18's sweep (`-t 6,12 -p 400 -n 1`
micro bench + a `-t 6` parity arm) closes that gap on our hardware. Note:
pinning is NOT automatic (default = inherited affinity); `--cpu-mask` +
`--cpu-strict 1` exist but #26997 reports a no-op case — verify before relying.

### §10.3 Do-not-do list (each verified against source, not folklore)

- **Speculative decoding: never enable.** At single-token readout
  `n_draft_max` computes to 0 (no drafts accepted), yet the drafter still
  prefills the prompt unconditionally every batch (upstream open work item
  `TAG_SPEC_AVOID_DRAFT_REEVAL`) — a second full forward pass that buys
  nothing. Worse: draft-model speculation **diverges from vanilla under
  temperature=0/top_k=1 on quantized targets** (open issue #25618) —
  disqualifying for a bit-reproducible decision runtime. Unsloth's MTP
  numbers are decode-only GPU figures; no prompt-processing data anywhere.
- **KV-cache quantization is not a speed tool on CPU.** pp flat within noise
  (#20969); with CPU flash-attn, quantized KV takes the `one_chunk` fallback
  and is **5.0× slower** at 8k ctx (#26948). Accuracy on constrained
  short answers collapses without rotation (AIME25: Q4_0 2.0% → 21.7% with
  rotation, #21038). Legitimate only for KV capacity (raising `-c`).
  Correction to our earlier note: #21332 is a merged revert PR — on current
  master the SWA cache *is* quantized; there is no f16/f32 SWA special case.
- **`-b`/`-ub` tuning is a no-op for our prompt length** (200–400 tok = one
  ubatch); the only CPU sweep shows pp *halving* above ub 512 (#18725).
- **OpenBLAS: 3× worse** on pp (#25565, libgomp spinlock contention).
  `--defrag-thold` deprecated. AVX512 (fused off on 13th-gen desktop) and
  AMX (absent on consumer Raptor Lake) are non-options.
- **`--cache-reuse N`** (non-contiguous KV chunk relocation — the mechanism
  that fits our permutation suite, same tokens different order) is
  mechanism-verified but **gain-unmeasured** upstream, and **blocked for
  gemma** (iSWA caches are not shiftable) — a dense-model lever only.
- `--no-mmap` is gone: `-lm/--load-mode mmap+mlock` is the
  memory-pressure fix (latency variance under co-tenant PSI storms, not
  throughput).

### §10.4 Certified thresholds for the gate (grounds §9.3 item 3)

Verified against our integration surface: new artifacts slot in as
`CalibrationArtifact` `scheme` variants (existing version-tagged cache keys
auto-invalidate); `ConfidencePolicy::{verify_below, abstain_below,
min_confidence}` are the bands; `ProofAwareCalibration` is precedent for
pre-calibration signals.

- **Split-conformal abstention (highest fit):** score items `s_i = 1 −
  p̂(correct_i)` (or any fixed scorer), `k = (n+1) − floor(α(n+1))` in exact
  integer arithmetic, accept iff `score ≤ s_{k−1}` — marginal accepted-accuracy
  ≥ 1−α, distribution-free. Rust artifact: `{scores, k, qhat, scorer_id,
  alpha}` — serializable, hashable. Edge case NumPy gets wrong: `k > n` ⟹
  `q̂ = +∞` (always abstain), not an error. **Critical correction:** our 3
  permutations are exchangeable at the *question-cluster* level only — the
  calibration count is 120 clusters (351 pooled with JevBench), not 360 rows,
  or the +1/(n+1) slack quietly overstates the guarantee by ~√3. JevBench is
  validation-only, never in the quantile (cross-suite = the shift case,
  arXiv:2006.09462). Per-`question_class` conformalization is the right
  granularity (key already exists) with the `n ≥ 30`-per-class guard.
- **Chow's rule** as the derived default (reject iff `max_k p_k < 1 − ε`,
  ε = cost ratio): one interpretable knob for arms too small to
  conformalize; needs calibrated posteriors to be optimal → composes with
  temperature (arXiv:2405.05160; 0/1 reduction is our derivation).
- **Margin ≡ MSP at K=2** (`MSP = (1+margin)/2` — identical risk-coverage
  curves; computing both is redundant). They diverge only at K ≥ 3, where
  margin is invariant to tail mass and entropy is not — exactly our
  Choice/Score arms, where **no paper answers the scorer question** (verified
  negative: Galil et al. evaluate only MSP/MC-dropout/temperature) → capture
  margin + entropy + MSP in the trace and let per-arm calibration select
  empirically. Under coexisting shifts margins beat MSP/entropy (arXiv:2405.05160).
- **Conformal risk control** for expected-error framing (matches "abstention
  is a successful outcome" better than miscoverage): the official `get_lhat`
  is nine lines (arXiv:2208.02814).
- **Geifman SGR** gives the PAC form our gate actually poses ("accepted
  error ≤ r* w.p. 1−δ") — needs a ~40-line regularized incomplete beta
  (Lentz), shared with rational-cascade tuning. Must surface infeasible
  (r*, δ, data) triples as hard artifact errors, never empty accepts.
- **Energy-based OOD has a hard boundary:** `MSP = E + f_max`, so energy is
  exactly what softmax discards — computed on a *normalized* distribution it
  is **identically zero for every input**. Usable only on raw-logit rungs
  (decision-model, BM25-as-heuristic), computed **before** our calibration
  seam (which normalizes in log-prob space and would destroy it).
- **Calibration: keep temperature, add Platt (Laplace-smoothed targets), do
  NOT add isotonic** — sklearn's own threshold is ~1000 samples; we have
  360–591. Newton temperature fits need a multi-restart + NLL-improvement
  guard (the "convexity" claim is not provable for K>2).
- **Framing correction to §1:** Gatekeeper (arXiv:2502.19335) is a
  fine-tuning loss (misclassified → uniform), with no public code and no
  guarantee — its value is the calibration-target insight, not inference
  math. The only cascade paper with public code is rational cascades
  (arXiv:2501.09345 → `mzelling/rational-llm-cascades`); early-abstention
  cascades (arXiv:2502.09054) have none (constraint removable via
  `φ_i = ξ_i + softplus(δ_i)` if we port it).

Port order (from the sweep): score capture → conformal → Chow → CRC → SGR →
energy → joint cascade tuning.

### §10.5 Family-A execution path: Kai-0.6B-ONNX contract (grounds #92)

Source-verified from `onnx-community/Decision-2.0-Kai-0.6B-ONNX` (conversion
scripts + fixtures) and `vllm-sr/Decision-2.0-Kai-0.6B` (runtime package):

- **Graph I/O:** `input_ids [B,L]`, `attention_mask`, `answer_pos [B]`,
  `option_pos [B,K]` → **one fp32 `logits [B,K]`** — a single forward pass
  scores all K candidates (no KV cache in the fused graph; positions built
  in-graph). This is the family-A form our candidate-conditioned model
  crate mirrors.
- **The head is 5 matrices of 1024×256 (~1.05M params, 4.21 MB):** bilinear
  `(key(c)·query(q))/√256` + `scalar(gelu(candidate_mlp(c)+query_mlp(q)))` —
  a 40-line Rust f64 test oracle for the ONNX path.
- **Quantization is MatMulNBits block-32** (8-bit decoder, 4-bit embeddings,
  fp32 activations) — a built-in `com.microsoft` contrib op stock `ort`
  loads with default features; not dynamic int8. Ops are standard
  (LayerNormalization/Gelu/GatherND/Range…); no hybrid conv/recurrent ops
  (plain Qwen3 — F25's slow arch is not present here). ort-crate loading
  itself is untested — first task of the arm. Local corroboration: our
  gte-modernbert MatMulNBits b32 arm already ran through the eval harness
  (815.7 ms/item @ max_len 256, 4 threads) — **cap Kai prompt length well
  below its 8192 budget and measure before assuming the rung is affordable.**
- **Free parity target:** `conversion/fixtures` + their measured deltas (q8
  vs fp32 export: max prob diff 0.017, 0 argmax flips) — quantization is not
  the accuracy risk; latency is.
- **`score_bias.json` is a fully specified, model-free calibration recipe:**
  per-level additive logit offsets fitted by float64 Newton on held-out rows
  minimizing `−log softmax(z+b)` with stratum weights `n^−0.5`, mean-zero
  shift — a direct port into our `Calibration` seam targeting Score-rung ECE.
- **`shared_ctx.py` shared-prefix prefill** (prefix capped at the shortest
  candidate position, suffix re-based per question) is the mechanism behind
  their 4.9 ms/question median — the multi-question latency lever if the
  rung is adopted.
- Provenance note: Kai's weights are a uniform 6-checkpoint soup
  (`m6-mxcx-soup`) — model-averaging as a training-time ensemble, relevant
  to #88's distill recipe.

### §10.6 Clef head internals + routing mechanisms (grounds B6)

- **clef-flash `JointSchemaHead`** (source read): six 4096→1024 projections,
  type embedding per question kind (noul/choice/score), 2 evidence-routing
  cross-attention layers, residual scorer over `[field, option, field·option,
  |field−option|]` — and a **lexical prior fused inside the head** (scaled
  cosine of the option's raw embedding rows vs the question vector) with a
  learned sigmoid residual gate. Our "cheapest mechanism first" instinct,
  trained in as a prior. Not portable (244 MB head, 19 GB vision backbone);
  **no training code public** — no RLCD-for-calibrated-decisions
  implementation exists anywhere (facebookresearch/RLCD is text alignment,
  archived). Their score answers use raw expected value with **no** fitted
  bias (contrast Kai's Newton offsets).
- **RouteLLM's threshold calibration is the pattern D25 wants:**
  `threshold = quantile(1 − strong_model_pct)` of the router-score
  distribution — operating points derived from the distribution instead of
  hand constants, making "accept-rate p @ accuracy a" a first-class reported
  number. FrugalGPT adds per-link gates (`accept iff score > 1 − thres`,
  deterministic by construction).
- **JEV-as-a-Judge (arXiv:2609.26550)** is the closest published analogue of
  our ladder: label-probability judge, threshold frozen in advance,
  accept-or-escalate; reported +0.9 points over the strong model at 41% of
  its cost, live-test replicated. Stated failure modes: style-perturbed
  inputs and reference-free prose — the classes to watch in our own
  abstention audits. Semantic-entropy uncertainty (Nature 2024,
  `jlko/semantic_uncertainty`) needs k samples per item — wrong trade at
  our budget, skip.
- clef-evals is API-only (no joint-head code) but ships
  `published_reference.json` (Clef/Clef-flash/Jev/Laya across 5 suites,
  median latencies) usable as cross-model reporting context.

### §10.7 Priority addendum (2026-10-04) — merges with §9.3

1. ~~r18 VNNI rebuild~~ **Corrected 2026-10-04: no VNNI defect existed**
   (§10.1). r18's lasting outputs: the `-t 6/12` thread sweep (t=12 wins
   prefill +13.1% — runner config validated) and a cross-build determinism
   check (two independently-built binaries, same commit + seed — integrates
   via the r18b followup).
2. **Conformal gate port** (§10.4) — supersedes §9.3 item 3 with concrete
   algorithms and the cluster-level-n correction; port order above.
3. **Kai-0.6B-ONNX arm** (#92) — contract verified in §10.5; ort-crate
   smoke test first, prompt-length-capped latency measurement second,
   parity fixtures as the accuracy gate.
4. **`score_bias` Newton fit** into the Calibration seam (Score-rung ECE).
5. **Quantile-derived thresholds** for LadderPolicy/D25 overrides (§10.6).

## §11 — Inference-engine repos: Strata, flash-attention, thecodacus/llama.cpp (2026-10-04)

Trigger: user-directed sweep — "could the code from either of these provide
insights to enhancing our core system?" Sources read in full: Strata
README + `docs/HOW_IT_WORKS.md` + `docs/DETAILS.md`; flash-attention
README; thecodacus/llama.cpp repo metadata + `perf`-branch commit
history vs upstream `ggml-org/llama.cpp`.

**Headline verdicts: no code from any of the three enters the workspace.
Strata contributes five practice-level transfers and one external
validation of our posture; flash-attention has no integration path;
thecodacus/llama.cpp is a watch-list item for a class of models we don't
run.**

### §11.1 Niko1221/Strata — practices transfer, code does not

What it is (MIT, engine 0.1.3x, ships a paper + per-engine-version bench
results): a token-generation inference engine that runs Qwen3.8-Flash-Next
(125B MoE, 24,576 experts, 10 active/token) on 12-24 GB consumer GPUs by
tiering the model across the whole PC — GPU holds attention/DeltaNet
mixers, routers, shared experts, the MTP draft layer, KV, and an adaptive
hot-expert cache; RAM holds all 24,576 experts pinned (CPU computes misses
in place, concurrent with GPU work); SSD holds a 28.8 GB n-gram table. MTP
speculative drafting reaches 2.4-3.2 tokens/pass with byte-identical
output (big model checks every draft) for 1.6-1.8× speedup.

Every technique that makes Strata work is a *decode-time* technique for a
generating MoE — expert tiering, KV streaming/quantization, 8K-chunk
prefill with PCIe overlap, MTP drafting. OpenCodifier emits one decision
per request from dense gemma-family readouts; none of those apply. What
transfers is how they run and document the engine:

- **Determinism engineering as a shipped feature (their #152 → #410
  chain).** Adaptive expert tier + multi-token kernel rounding differences
  + draft-window grouping + prompt-cache decode-path resumption + PCIe
  share of missed experts ⇒ the same prompt at temperature 0 could end in
  a *different, equally good* answer depending on cache/conversation
  state. Their fix is one opt-out switch per state source
  (`STRATA_IQ_MT_MIN=1`, `--prompt-cache 0 --adapt-swaps 0
  --pcie-frac 0`), with a measured repeat-identity rate and per-switch
  cost ("4 repeats → 1 distinct answer with all three switches; 2
  distinct without `--pcie-frac 0`; 2 distinct at defaults"). This is the
  strongest external parallel to our cache-key versioning + identity
  replay, and it is a **checklist for B5**: llama-server has its own
  state sources (prompt cache, KV reuse, slot count, batch composition)
  and our backend-arm protocol should enumerate and pin every one.
- **Measured acceptance gating for speculative paths.** Prompt-lookup
  drafting is enabled "only where its measured acceptance and cost say it
  pays: code edits 6-11% faster, other text unchanged", and `/metrics`
  exposes `drafts_offered`/`drafts_accepted` per request. Direct design
  input for **B6**: an escalation link exists in a policy only where its
  measured agree-rate × cost justifies it, and escalation counters are
  first-class trace/metrics output, not an afterthought.
- **Calibrate-keep-if-better, per host** (`--calibrate`: measures
  candidate engine settings, keeps one only if >3% faster, remembered per
  PC+model). Adopt as a *bench-selection* pattern for D9 (criterion
  profiles measured per host class, kept only if the delta clears noise),
  never as runtime self-modification — our gates are policy, and policy
  must never adapt at request time.
- **Structured output = validate, don't constrain.** Their
  `response_format: json_schema` is "schema prompting followed by server
  validation, one generation per request, with no hidden retry";
  violations return 502 with `error.code: structured_output_failed`;
  streaming buffers until validation passes. Independent confirmation of
  our `unsupported_generation_field` + abstention + strict `ir.*` error
  codes posture. Nothing to change; cite it.
- **Resource-arbitration semantics.** idle-unload / min-free-VRAM gate /
  `before_load` hook / explicit 503 "the GPU is in use by another
  program" / 409-while-busy on control endpoints / lazy load on first
  request. Concrete small candidate for **opencodifier-http**: structured
  busy/backpressure codes (503 with machine-readable reason, 409 on
  control ops while a decision is in flight) instead of silent queueing.
- Same refuse-loudly ethos elsewhere: rope scaling past the trained range
  refuses `--rope-scaling none` with a 400 rather than silently
  degrading — matches our no-silent-fallback rule.

### §11.2 Dao-AILab/flash-attention — no integration path

Exact (not approximate) fused attention kernels; the memory saving comes
from never materializing the attention matrix (IO-aware tiling). OpenCodifier
computes no attention anywhere: core/engine/schema are sync Rust, and the
model rung delegates to llama.cpp/ONNX behind `InferenceBackend` (D11).
flash-attention is a CUDA library; our base posture is CPU-only. If a GPU
tier is ever added, the benefit arrives *inside* llama.cpp's CUDA/Vulkan
backends — nothing for us to integrate. Two portable principles, both
already embodied: (a) exact-but-IO-aware restructuring — avoid
materializing O(n²) intermediates, the same spirit as the B2/B3
constant-factor work; (b) determinism is a configurable, testable
property (their split-K reduction notes) — supports the identity-replay
stance. **Closed: revisit only if we ever implement our own kernels.**

### §11.3 thecodacus/llama.cpp — inert for our arms; MoE watch item

Fork of `ggml-org/llama.cpp`, default branch `perf`, last push 2026-09-30.
Commit-level delta vs upstream (verified): a MoE expert cache
(`--moe-cache-profile` / `--moe-cache-slots`) wired into the qwen4exp
architecture, an MTP draft head (upstream PR #28243), `--sched-async-cpu`,
`--cpu-tp`, TurboQuant KV (CUDA; the Metal variant was dropped after an
upstream backend restructure), a `llama-moe-trace` tool, and one real
bugfix (`mul_mat_id` sync predicate vs the expert-pack split) — maintained
by periodic large upstream syncs (677 commits merged at once). Same
idea-family as Strata (expert cache + speculative drafting) targeting the
same model family. Our decision arms are dense gemma-family — no MoE, no
MTP — and our pinned builds (commit 1537a0a, upstream) are untouched by
any of it. **Watch trigger:** an MoE decision-model candidate would make
the expert cache relevant to the B5 backend.

Independent evidence for #88 from this ecosystem: Strata's "Coder" release
(ISTA-DASLab expert-pruned Qwen3.8-Flash-Next, 256 of 512 experts kept,
91.3% of the full model's SWE-bench Verified, fits 32 GB) — the
"smaller-but-targeted beats uniformly-small" distill thesis, consistent
with our r19 finding that capability is bought at training time (QAT Q4_0
0.808 vs the 0.50 non-QAT ceiling, §12), not by inference-side tricks.

### §11.4 Plan deltas (merges with §9.3 / §10.7)

1. **B6 design inputs (from Strata):** per-link acceptance telemetry
   (`escalations_offered`/`escalations_accepted` analog of their
   draft counters) and "enable only where measured acceptance × cost says
   it pays" as the stated semantics of LadderPolicy overrides.
2. **B5 backend determinism audit:** enumerate and pin llama-server's
   state sources (prompt cache, KV reuse, slots, batch composition)
   before accepting any backend-arm number as identity-replay-clean —
   Strata's #410 is the cautionary tale.
3. **New small queue item — opencodifier-http backpressure codes:**
   structured 503 (busy, machine-readable reason) and 409-on-control-
   while-in-flight, mirroring the arbitration semantics above.
4. **#88 evidence addendum:** Coder release (expert-pruned MoE, 91.3%
   SWE-bench retained) alongside QAT as the two validated
   capability-per-byte levers.
5. No source from Strata, flash-attention, or thecodacus/llama.cpp is
   vendored, ported, or depended on. flash-attention: closed. thecodacus:
   watch-list. Strata: practices only.
6. All §9.3 items keep their existing rank beneath these.

## §12 — r19 quantization ladder: where decision capability lives (2026-10-05)

Headline: **on the decision readout, inference-side representation changes
do nothing and training-side changes do everything.** 17 non-QAT quants of
gemma-4-E2B-it spanning 2.7→16 bpw score 0.300–0.550 with BF16 itself at
0.500, ECE uniformly bad (0.226–0.490) — while the QAT arms at the *same*
bits score 0.767–0.808 (ECE 0.102) and Google's mixed-precision mobile QAT
at ~2.5 average bits holds 0.800. Board rows in BENCHMARKS.md footnote 11;
speed reference alongside. Written for the VIVERE rollover: each finding
below is stated as a training-side conclusion (method → measured readout
delta), because VIVERE will run the training side itself.

### §12.1 The flat ladder (the null result that anchors everything)

- 17 rungs — plain K-quants (Q3–Q8), legacy (Q4_0/Q4_1), i-quants
  (IQ2_M…IQ4_NL), unsloth UD-* dynamic imatrix mixes, BF16 — form one
  statistically indistinguishable band. Spread ≈ ±0.1, which is what
  60–120 items produce by sampling noise alone.
- Therefore the earlier per-rung findings (r15 "QAT is carrying the
  number", r16's +45.8 pp matched-format delta) generalize: **no quant
  choice matters below the training floor.** Quantization experiments on
  an untrained-for-task base measure the base, not the quant.
- Corollary for future arms: a new quant format only demonstrates value
  on this suite if the base already has decision capability to preserve.
  This is why the Bonsai-2-27B claim (PTQ ternary, 98.2% FP16 retention)
  is worth one bounded probe (r20b) but not a full CPU arm — the claim is
  about *retention on a capable base*, and our suite can test retention
  only where capability exists.

### §12.2 Training-time effects (the only things that moved the number)

| method | bits | acc | ECE | readout delta |
|---|---|---:|---:|---|
| non-QAT BF16 (ceiling reference) | 16 | 0.500 | 0.355 | — |
| QAT Q4_0 (official export) | 4 | 0.808 | 0.102 | +30.8 pp over the base's own ceiling |
| QAT UD-Q4_K_XL (unsloth mix of same QAT ckpt) | ~4.5 | 0.767 | 0.137 | +26.7 pp |
| mobile mixed-precision QAT (2/4/2/4/8 by tensor class) | ~2.5 | 0.800 | 0.102 | +30.0 pp at 1/6 the bits |
| QAT UD-Q2_K_XL (pure 2-bit) | ~2.5 | 0.483 | 0.170 | back inside the noise band |

Training-side conclusions:

1. **QAT is not a compression technique here — it is the capability
   transfer itself.** The +30 pp over BF16 means the QAT training pass
   taught the model the readout better than the base model knows it,
   while simultaneously cutting bits 4×. For VIVERE: the distill/train
   stage is where decision-readout fidelity is purchased; a teacher-grade
   base merely bounds it. (Boundary, resolved same day in §12.6: the
   transfer needs latent capability in the base to elicit — LiquidAI's
   quantization-aware *distillation* at 350M replicates nothing, and
   Google's QAT lift has no 350M-scale analog in our data.)
2. **Bit placement beats bit budget.** Pure 2-bit QAT fails where the
   same average bits, placed 2/4/2/4/8 across lm_head/MLP/attn/PLE,
   scores 0.800 with perfect emission. If VIVERE ships small models,
   per-tensor-class mixed precision is a first-class training decision,
   not a post-hoc packing choice.
3. **Emission stability is also trained in.** 14 of 17 non-QAT quants
   flip predictions between identical replays; every QAT arm above 2 bits
   replays clean. A determinism replay must gate any shipped artifact
   (r11b said this for requants; r19 says it is a training property, not
   a quantizer property).
4. **Calibration follows the same split**: non-QAT ECE is uniformly bad
   across all bit-widths; QAT ECE lands at ~0.10 without any calibration
   fitting. Whatever produces the capability produces calibrated
   confidence with it.

### §12.3 The UD-imatrix null result

Unsloth's importance-matrix dynamic mixes — the community default for
"better low-bit quants" — do not beat plain K-quants on the decision
readout at matched size (§12.1 board rows: UD-Q3_K_XL < Q3_K_M,
UD-Q6_K_XL < Q6_K, UD-IQ2_M worst-in-ladder). Conclusion for VIVERE
data-collection: an imatrix fitted on generic text does not represent a
decision-distribution's importance structure. If importance weighting
matters for distill artifacts, the calibration corpus must be drawn from
the decision suite's distribution — a testable follow-up (requant the
QAT checkpoint with a suite-derived imatrix).

**Resolved (r21a, 2026-10-05): the matched-corpus control is also a
null.** Requants of the official QAT-Q4_0 checkpoint with an imatrix
fitted on the suite's own rendered prompts (120 items × 3 perms,
answer closers included): Q4_K_M ties the control digit-for-digit on
accuracy *and* every per-class accuracy (0.808 / lex 0.975 / meta 1.00
/ rel 0.45) at +17 % p50 and +0.010 ECE; Q3_K_M 0.767 and IQ3_XXS
0.792 land below the control, non-monotonically (the 2.5-bit grid beat
the 5.5-bit one — one measurement, unreplicated). Combined with the
r19 UD null this closes the imatrix question for the decision readout
at both corpus extremes, generic and exactly-matched: **importance
reweighting is not a lever on a QAT checkpoint** — the training-time
recipe pins the readout, and inference-side representation changes
neither add to nor rescue it. For VIVERE data-collection the
conclusion stands but sharpens: spend on the training recipe and the
corpus, not on PTQ side-recipes. The one imatrix case still open sits
on *distill* (non-QAT) weights, where §12.6.1's 0.683 Gemini distill
at plain Q4_K_M is the natural test subject.

### §12.4 Speed side (unchanged by the accuracy collapse)

Prefill t/s is flat 190–230 from 2.7 to 6 bpw (compute-bound at 5B
class); Q8/BF16 halve it; the QAT-Q4_0 arm is the fastest full-size row
(230.8 t/s) *and* the most accurate. The operating point costs nothing
on either axis. Full table: BENCHMARKS.md "r19 E2B quant-ladder speed
reference".

### §12.5 Open items this section spawns (status: resolved same day, §12.6)

1. **Format A/B (safetensors-vs-GGUF)** — resolved: GGUF side ran
   without the template override and sits below the readout floor at
   both precisions (§12.6.3); combined with the torch side
   (fp32 0.200 / bf16 0.300) the A/B concludes *consistent*: the tiny
   0.8B base is ≈chance under both engines, and the lanes differ only
   in how far below chance they can see.
2. **Suite-derived imatrix requant** of the QAT checkpoint (§12.3) —
   still open; cheap, one arm, directly VIVERE-actionable.
3. **Third-party teacher-distill arms** — resolved, and the segment's
   headline: transfer is real but teacher/recipe-dependent (§12.6.1).
4. **Bonsai-2-27B probe** — resolved: retention-consistent at one item;
   `--lora` untestable on the vendor fork; full arm remains with #35
   (§12.6.4).
5. **MoE + QAD datapoints** — resolved: QAD is a null at 350M; the MoE
   arm is template-locked out of the readout rather than incapable
   (§12.6.2/§12.6.3).

### §12.6 r20 same-day resolutions (2026-10-05)

**§12.6.1 Distillation transfers; recipe quality decides.** Same base
(gemma-4-E2B), same quant (plain Q4_K_M, no QAT): the Gemini-3.1-Pro
reasoning distill scores **0.683** — +18.3 pp over the base's own BF16
ceiling, metadata-match 0.95, ECE 0.212, clean replay (Δp 0.242) — the
first non-QAT arm above the r19 band. The abliterated-Opus distill of
the same base scores 0.350 (in-band) with a replay flip (Δp 1.000,
valid_rows_match false). Training-side conclusions for VIVERE:

- Teacher distillation buys decision-readout capability at 4-bit
  *without* quantization-aware training — the transfer path and the
  compression path are separable.
- Teacher/recipe choice is decisive: one recipe transfers +18 pp, the
  other (abliterated) lands in-band and destroys replay stability.
  Abliteration (safety-direction ablation) is a capability hazard, not
  a neutral edit.
- Replay determinism rode with the winning distill and flipped with
  the damaged one — the third independent replication (after r11b
  requants and r19 QAT arms) that emission stability is a training
  property.

**§12.6.2 QAD is a null at 350M.** LiquidAI's quantization-aware
distillation (QAD) Q4_0 vs the plain-Q4_0 control of the same
LFM2.5-350M: 0.208 vs 0.217, ECE 0.551 vs 0.587 — nothing. The
r16/r19 QAT lift does **not** generalize as "training for quantization
elicits capability". Two non-exclusive readings: (a) scale floor —
the 350M dense base has no latent decision capability to elicit
(itself at 0.217, and the 230M sibling can't even hold the emission
format: 65/120 invalid); (b) teacher/task mismatch — QAD's distillation
corpus is generic text, not decision-shaped. Either way the VIVERE
rule sharpens: elicitation-style training needs a base above the floor,
and the training corpus must carry the readout structure.

**§12.6.3 The readout floor (protocol finding).** Four arms recorded
120/120 invalid distributions: tiny1b f16/Q4_K_M and LFM2.5-8B-A1B
Q4_K_M/UD. Post-queue raw probes (one item, stock build, same payload
as the runner) established the mechanism: llama-server applies
`logit_bias` to the *sample* but computes `top_logprobs` from the
*unbiased* distribution — the probe's biased sample emitted a letter
while its reported top-5 held none. So the letters protocol measures
"letter mass inside the model's natural top-20", and a model below
that horizon reads as invalid regardless of what the biased argmax
would say. The four arms sit below the floor for two different
reasons:

- tiny 0.8B: natural distribution degenerate (torch lane's
  full-letter-softmax readout puts the same weights at ≈chance) —
  genuinely at/below chance.
- LFM2.5-8B-A1B (MoE): logits healthy, but its chat template opens
  the assistant turn with `<think>` at p≈1.0; `enable_thinking:
  False` is silently unsupported by LiquidAI's template, so letter
  mass lands at ≈e⁻¹²⁶. A template-rescued variant could measure the
  capability behind the format lock, but that changes the prompt
  contract mid-board — out of protocol scope, noted as an option.

Consequence: "below floor" is a real, reportable measurement — these
models cannot answer this readout unassisted — but it is not the same
quantity as a low accuracy on surviving distributions.

**§12.6.4 Bonsai-2-27B probe.** The PrismML fork shallow-clones and
builds llama-server in 79 s; PTQ1_0 loads where stock 1537a0a refuses.
One-item probe: first-token top-1 is the correct letter at p≈0.995,
identical with and without `--reasoning-budget 0` (the flag is inert
for this model). Retention-consistent with the vendor's 98.2 % claim,
anecdote-level by construction. Two operational notes: the fork
inherits the help-listed-but-rejected flag bug class (`--lora` rejected
at parse — the AtomicChat abliterate LoRA is unmeasurable on this
build), and F18's 27B CPU extrapolation was ~20× pessimistic for this
host (74 s/item actual; a full 480-readout arm ≈ 10 h). **Scope
decision (operator, 2026-10-05): the Bonsai arm is dropped from
campaign scope — even if the retention claim holds, ~74 s/readout is
deployment-nonviable on CPU; the class returns only under #35's Vulkan
rung.**

### §12.7 Operating point: what beats QAT-E2B-Q4_0, and what's next (2026-10-05)

The official QAT Q4_0 export (0.808 / ECE 0.102 / 423 ms p50, board row
`stock__e2bqat-official-q4_0-letters.json`) is the campaign's reference
operating point. Four arms beat it on accuracy — every one pays ≥2.3×
the latency:

| arm | acc | ECE | p50 | note |
|---|---:|---:|---:|---|
| **Winnow-E4B (own model)** | **0.842** | 0.109 | 2171 ms | clean replay; the #88 program's product |
| Winnow-E4B Q4_K_S (requant of Q8_0) | 0.825 | 0.101 | 951 ms | r11b's operating pick |
| jebadiah-9b-v2 Q8_0 | 0.833 | 0.075 | 5626 ms | 9B class |
| MiMo-V2.6-Distill-9B Q3_K_S | 0.817 | 0.048 | 14307 ms | best ECE on the board, 34× slower |

**Nothing beats 0.808 inside its ~423 ms class** — the QAT export is
the Pareto frontier of the small-fast lane, which is why it is the
escalation target. The one number that beats *every* single-model arm
is not a model at all: the simulated ladder/cascade (REPORT F23,
**0.867 @ 753 ms**) — cheap rungs decide the easy majority, the model
rung sees only what escalates.

Candidate paths above 0.808, ranked by measured evidence:

1. **Winnow lineage (own model, #88).** Already 0.842; the Q4_K_M
   requant's determinism flip (Δp 0.998) is the known defect, and a
   QAT-in-loop export (Google's recipe applied to our checkpoint)
   would plausibly hold ~0.84 at ~400-500 ms — the most direct route
   to a faster-better point. The r16/r19 lesson applies: QAT must be
   in the training loop, not post-hoc.
2. **Suite-derived imatrix — resolved null (r21a, 2026-10-05, §12.3).**
   The matched-corpus control did not hold 0.808 through requant: the
   Q4_K_M requant tied the control exactly, lower rungs degraded
   non-monotonically, and every arm cost more latency than the Q4_0
   original. The trick does not transfer to Winnow or future distills
   *as a PTQ step on a QAT checkpoint*; the open form is an imatrix
   requant of a *distill* (non-QAT) artifact, if one is ever needed.
3. **Own-teacher distill of the E2B base.** The third-party Gemini
   distill reached 0.683 with someone else's recipe and no access to
   our suite (§12.6.1). Distilling from a teacher *on suite-shaped
   data* targets the readout directly — the VIVERE extraction path;
   expected to clear 0.683 by a wide margin and contest 0.808 at
   plain Q4 bit-rates.
4. **Not a candidate: bigger gemma-QAT.** The official E4B-QAT export
   underperformed its sibling badly (0.558, 29 invalid dists, board
   row `stock__e4bqat-official-q4_0`) — the QAT recipe did not
   transfer across sizes within the family, so "scale up the same
   QAT artifact" is falsified for now.

Escalation architecture (how the runtime uses the operating point):
the pipeline contract is cheapest-reliable-rung-first — exact rule →
cached decision → metadata filter → lexical (0.683 @ 1.3 ms
relational) → embedding → **the QAT model rung only on escalation** →
verifier. The model rung is candidate-conditioned and
confidence-gated; it never runs twice per request (verification is
gated, D-series rule), and B6 gives `Verify`/`Abstain` outcomes a
configured next rung. The runtime does not improve raw model
capability — that is the offline program's job (#88 dataset merge +
fine-tune, VIVERE extraction/distillation, QAT-in-loop, suite-shaped
corpora); the runtime's lever is to never pay model cost when a
cheaper rung can decide confidently, and to abstain or escalate when
the model is unsure. Raw-capability work lands as new board rows; the
ladder converts whichever arm is best into *outcomes* beyond that
arm's standalone score.

## §13 — Deep-research round: readout-floor mechanics, the Jebadiah v2 replication, cascade attack surface (2026-10-05)

Research round run while CI held the floor; four fronts (readout
floor, QAT recipe, new arms, cascade literature). Everything below is
documented-before-use: no arm has been re-run on these findings yet.

### §13.1 The readout floor is an upstream logprobs-reporting behavior — and the bypass is known

An upstream llama.cpp issue (filed 2026-02-17: "`-l 2742-100` does not
have any effect on reported logprobs") confirms, independently of our
r20 protocol, that **reported logprobs do not reflect logit_bias** —
a deviation from OpenAI API semantics. Mechanism, corroborated across
sources: llama.cpp captures the reported probs vector from the raw
logits; grammar constraints (GBNF, whether via `--grammar` or
`response_format: json_schema` on the chat endpoint) act as masks in
the sampler chain *after* raw-logit computation (rejection-sampling
order), so they change which token is *sampled* but not which
logprobs are *reported*. Known-adjacent bugs: garbled logprobs
(#11561, 2025-01), grammar/prompt-interaction reports through 2026.

Consequences for the letters protocol:

- The floor's mechanical form is now explained: under `logit_bias`
  the reported top-20 is the *raw* top-20; when letter mass sits below
  the raw horizon the renormalized letter distribution is built from
  whatever letters happen to appear — a distorted readout, exactly the
  §12.6 observation.
- Grammar/json_schema constraints would buy **format compliance**
  (every sampled token is a letter) but *not* calibrated
  distributions. Useful, not sufficient.
- The robust bypass is the one Jebadiah ships as its standard readout
  (§13.2): **raw `/completion` with a self-rendered prompt** — the
  server's chat template is never invoked, the thinking structure is
  never entered (pre-fill/close it in the rendered prompt), and label
  logprobs are read at the answer position. This is the untested fix
  for the template-locked LFM2.5 arm (§12.6's `enable_thinking` was
  silently unsupported by LiquidAI's template; `--reasoning-budget 0`
  is parsed by our fork but untested on this model). Feeds #93: the
  runner needs a `--raw-prompt`/`--template` lane, not just a grammar
  flag.
- Cheap settling probe (r22 candidate, one item, minutes): three
  request shapes on one below-floor arm — bias+top20 (current
  protocol), no-bias top20 (raw truth), json_schema+logprobs — and
  inspect which tokens each reports. This pins whether reported
  probs are sampler-independent on our fork build.

### §13.2 Jebadiah v2: an independent replication of this campaign, with recipe conclusions

The `frontier-infra` org (non-profit, Texas) ships **Jebadiah** —
"open System One decision model": typed questions (choice / noul
[yes-no] / score), a probability per option from **one forward pass,
nothing generated**. Same problem shape as OpenCodifier's IR and the
D26 model rung; their v2 cards (2026-09-26..29) are the closest
public replication of this program found to date.

Recipe conclusions that transfer to #88:

- **v2 switched base checkpoints**: v0/v1 fine-tuned the Qwen3.5
  `-Base` checkpoints; v2 is merged bf16 on the **chat** checkpoints
  (thinking off), same data otherwise. Their words: "Training on the
  chat checkpoint was worth more than every data change we tried:
  synthetic pools, a stronger teacher's labels, human-labelled
  yes/no." Checkpoint choice dominated data composition — #88 should
  distill from the chat checkpoint, not base.
- **Public Apache-2.0 training pool**: `frontier-infra/jebadiah-synth-v2`
  — 14,714 synthetic typed-decision questions across 24 families,
  teacher-labelled. A ready-made seed for #88's dataset merge; the 24
  families are also a checklist against our suite's family coverage.
- **Independent quant-stability table for a decision model** (260
  held-out items vs merged bf16): Q8_0 changes 4/260 answers (max Δp
  0.053), Q5_K_M 21/260 (0.119), Q4_K_M 24/260 (**0.355**); bf16 vs
  the training run itself drifts 1/260. **Score questions are ~2×
  more fragile than choice** (13–14 of the flips). Cross-check
  against r21a: our Q4_K_M requant of the *QAT* checkpoint moved
  accuracy zero and Δp ≤ 0.063 — roughly 6–10× more stable than
  their full-FT (non-QAT) weights at the same nominal rate. Both
  facts support §12.3: importance reweighting is a lever on non-QAT
  weights; QAT checkpoints carry their own quant robustness. And the
  score/noul class is the fragile one on both sides — our suite's
  rel/meta classes deserve the same suspicion.
- **Readout**: option-label logprobs at the answer position from
  `/completion`, renormalized over labels, then per-route fitted
  temperatures (choice 1.1167, noul 1.3319, score 0.8312,
  `temperatures.json`). Same math as our letters protocol; they keep
  temperature at the serving layer, ours lives in the engine's
  Calibration seam — same idea, cleaner layering on our side.
- **Calibration does not transfer** (their own warning, mirrors our
  ECE gate): on held-out HelpSteer2-like traffic the fitted score
  temperature wants 1.26, not 0.83 — "refit on your own labels."
  Validates calibrating on the deployment distribution per rung
  rather than trusting any shipped temperature.
- **The top-20 logprob cap is industry-wide**: LM Studio and AINode
  both cap at 20; on 77-option Banking77 questions the top pick
  survived but probabilities moved up to 0.16 past the cap, and they
  refuse >20-option questions outright. Our
  renormalize-over-available-letters choice is a valid variant, but
  per-option probabilities beyond ~20 candidates are not trustworthy
  on any stack.
- **Board context**: Decision Index 0.2.1 (67 open models,
  maintainer-validated runs) puts Jeb-27B #5 at 54.67. On JevBench's
  231 public items: 27B 0.866 (= Jev 1.13.0), 9B v2 0.818, 4B v2
  0.758. Not comparable to our suite (different items), but #78's
  jebadiah-4b-v2 arm is live and now has family context — the arm
  measures it on *our* suite. Their GGUF agreement numbers cover only
  the official repo files; mradermacher's community quants are
  explicitly not validated builds (download official for the arm).
  llama.cpp ≥ v0.5.0 needed for the `qwen35` arch; our fork
  (2026-09-29) qualifies.

### §13.3 strands-2B located: an ONNX artifact, not a GGUF arm

#78's "strands-2B" resolves to `onnx-community/strands-decider-2B-hobson-v19-ONNX`
— a decision model shipped as ONNX. That places it in #92's ONNX-rung
line (Kai-0.6B-ONNX, Julia-1), not the GGUF bake-off; no official
GGUF surfaced. #78 scope adjusted accordingly. **Resolution
(2026-10-05)**: contract verified and corpus-run — §6.5 "Verified
locally" holds the numbers (0.7273 / ECE 0.0628, published row
reproduced; fixture parity 0.0255 / 0 flips) and the v1 pointer-base
incident write-up. Verdict for #92: the strands ONNX backend is the
one arm whose calibration clears a gate, and its `InferenceBackend`
trait fit is clean (fused q8 graph, `input_ids`/`attention_mask`/
`answer_pos`/`option_pos` → logits `[B,K]`, temperature + softmax in
Rust f64 above the line per D7); CPU latency (~2.8 s p50) caps it at
escalation/verifier duty. Build is justified; the portability win
(pure Rust + `ort`, no llama.cpp) is the reason, speed is not.

### §13.4 r78: the 9B letters collapse is readout transfer, not the model

The r10 letters-parity anomaly (9B at 0.142, below the 0.25 chance
floor) closed with a three-probe diagnosis on the same stock build and
artifact: (1) raw completion is coherent — weights fine; (2) chat-mode
empty completions were *our probe's* bug — the model emits thinking
tokens that llama.cpp routes to `reasoning_content`, so `content` is
empty at small `max_tokens`; with `enable_thinking: false` the sanity
exchange is clean; (3) free-form answering in the model's trained
option-label format scores **0.858** (metadata 1.00 / lexical 0.975 /
relational 0.600), deterministic, 1/120 unparseable — while the
letters interface on the identical artifact stays at 0.142. The
letters arm's distributions were well-formed but content-free:
constrained-letter logits renormalize into position-flavored noise
(permutation analysis: zero content-following; by-position accuracy
0.00–0.33). The 4B tolerates letters (0.833/0.850) — interface
tolerance is per-model, not a family property. Board rows + footnote
¹⁶ in `docs/BENCHMARKS.md` carry the numbers; procedural rule (Model-
Import Parity Discipline, backend-fixed quality.md): a below-chance
constrained-readout arm on a competent base is a readout bug alarm —
the discriminating test is a free-form readout probe, run before any
"broken model" verdict. Also resolved here: llama.cpp's
`reasoning_content` split must be checked in any probe that reads
`message.content` from a thinking-capable model — an empty visible
completion is ambiguous between "refusal/EOS" and "all budget consumed
by reasoning".

### §13.5 Gemma 3n QAT: checkpoints are public, the recipe is not

Google released the int4 QAT **checkpoints** (TorchAO-produced) for
Gemma 3 / 3n but has **not** open-sourced the training code that
produced them — no `google-ai-edge` repo carries the recipe. What is
public: TorchAO's reproducible quantization recipes
(pytorch.org, 2025-09-19) and Unsloth's QAT lane (~70 % PTQ-loss
recovery claim). Therefore §12.7's candidate 1 (Winnow QAT-in-loop)
means **building our own fake-quant + LoRA loop on TorchAO**
(fake-quantized int4 in forward *and* backward during a short
fine-tune; QA-LoRA/LoftQ family), not lifting Google's recipe. Open
feasibility question: CPU-only fedora training throughput for a 4B
fake-quant loop; the T5500 V100s remain the burst option.

### §13.6 Cascade attack surface: confidence gates are adversarially steerable

Forced Deferral Attack (arXiv:2606.15308, 2026-05): an adversarial
input that *suppresses the weak model's confidence* forces a
confidence-gated cascade to escalate — cost inflation and routing
manipulation without touching any policy file. Companion result:
"When Efficiency Backfires: Cascading LLMs Trigger Cascade Failure
under Adversarial Attack" (arXiv:2605.17288). The binding rule
"input text never modifies policy/thresholds" does not cover this:
FDA manipulates the *confidence the gate reads*, which is exactly the
quantity D27's escalation consumes. Mitigations worth recording for
any multi-tenant deployment (low severity for local-first single-user
posture): abstention/escalation-rate monitoring (an FDA shows as a
rate spike), per-source rate limiting, and treating model-rung
escalations as a metered resource. Also noted in passing: conformal
risk-controlled routing ("Conformal Arbitrage") and
cost-expectation gating (CascadeDebate: execute iff
E[cost|execute] < E[cost|refuse]) as calibration-adjacent routing
literature for the standing watch-list.

## §14 — The public board: leaderboards, the alternatives press, and the dominance question (2026-10-05)

Operator-directed sweep: five "open Jev alternatives" articles plus
three benchmark surfaces — all eight URLs fetched and read in full on
2026-10-05 (none missing; the Hugging Face Space
`benchmarkheaven/JevBench` renders as an app shell only, its data
lives on the benchmarkheaven site). Trigger: the operator's question —
can OpenCodifier officially top the public benchmarks on **all**
metrics (accuracy, error rate, latency, speed, cost) without a large
LLM underneath?

### §14.1 The evaluation landscape is four surfaces, not one

| surface | scale | who runs it | headline rows |
|---|---|---|---|
| JevBench public 231 (`../JEVBENCH.md`) | 231 items | upstream authors' harness | hosted Jev 86.6 % / JEV-27B 88.70 (family-macro) / **fork_4b (ours) 76.62** / Kev-4B 73.71 / Laya 55.8–58.4 |
| Benchmark Heaven "JevBench v1.6.0" | 1,500 decisions self-hosted / 600 hosted; 92 systems | third party, author-submitted | Quyet-1.0-Large Cap 81.7 (31B decoder), Jev Cap 76.5 (Intel 62.4), best ≤4B Cap 65.4 |
| jabr classifier benchmark | 49 tasks / 869 cases | third party | Jev **0.966** vs best open **0.704** (Von); GLiNER2 0.698; Laya 0.583; **zero-ML engine (our self-run, 2026-10-06) 0.401** |
| jevbench.xyz | first-party run archive | JevBench itself | Banking77 80.3 % (19.7 % err); phishing recall 85.7 → 98.4 from one prompt change |

Cross-surface divergence is itself the finding. Laya: own README
0.766 (fine-tuned **on the benchmark's own training split** —
in-sample), 0.583 on jabr, 46.6 on Fastino's 17-domain eval, Intel
0.3–5.5 on Benchmark Heaven, 55.8–58.4 published on the 231. Kev-4B:
73.71 family-macro on the 231, Intel 19.5 on Benchmark Heaven (their
note: an older Qwen3-4B variant). Every system wins on a suite it
chose — pinggy's own conclusion — so no number is portable and the
only cross-suite currency is a composite. The board's official
composite is a **harmonic mean** over Intelligence / Calibration /
Speed / Cost (25:25:25:25, adjustable) — harmonic means punish the
weakest axis hardest.

### §14.2 The Benchmark Heaven board map (92 systems)

Structure: Capability Score = mean(Intelligence, Calibration);
eligibility for the Jev-class table = cost ≤ 2× Jev (≤ $0.065/1k) and
median latency ≤ 2× Jev (≤ 1.23 s) — 63/92 qualify.

- **The top of the board is large-model territory.** #1 Cap
  Quyet-1.0-Large (81.7) runs an undisclosed decoder over
  Gemma-4-31B; #2 deck-31B (frozen Gemma-4-31B-it FP8); #3 is Jev
  itself (76.5). The best fully-≤4B row is Plumb-4B at Cap 65.4
  (Intel 43.0). Beating the board *without a large LLM* therefore
  means beating the 31B class on composite — that is the open
  position.
- Intelligence inside the Jev-class table: ~59 tops the ≤12B class
  (Winnow-12B Q8), ~43 the ≤4B class. Calibration tops at 90.6 (Jev).
  Reranker rows (bge-reranker, mxbai, GTE, zerank) score Intel 0–16
  but Calib 73–88 — **the Calib column pays for abstention-aware
  posture even at near-zero intelligence**; our calibrated
  verify/abstain machinery monetizes directly on their metric.
- Cost and latency correlate weakly (their Spearman ρ = 0.18,
  n = 92): fast-and-cheap is genuinely unclaimed territory on the
  board.
- Submissions are author-reported (repo/model-card links; costs
  marked \* are estimated) — self-attestation. jevbench.xyz marks its
  own first-party runs "carries no evidence tier … artifacts
  published in full" — the protocol-honesty language our REPORT.md
  already practices; our provenance discipline (F30 audit, byte-lock,
  determinism columns) is differentiating currency on a
  self-attested board.

Where we would land today (no cross-suite mapping assumed):
eligibility is trivial (local CPU; fusion p50 0.94 ms in-suite /
136.7 ms held-out vs the 1.23 s cap; marginal cost $0). Speed and
cost: at/near the floor of anything on the board — the fastest board
p50 is 0.18 s, and no row is both local and sub-second. Calibration:
measured ECE 0.070 (fork_4b, 231) / 0.103 (fusion-v2, held-out suite)
with abstention sits in board-top company. Intelligence on THEIR
suite: unknown — the binding constraint, and the harmonic mean
punishes it hardest. On the one third-party split we have run (231):
fork_4b 0.7662 is above every published ≤4B row we know (Kev-4B
73.71, decider-2b 71.0, Jev-Style-2B 73.6) and below the
9B/27B/hosted class (Open-Jev-9B 77.13, hosted Jev 87.18, JEV-27B
88.70).

### §14.3 The dominance question, answered

**Top-of-composite: plausible. Top-of-every-axis including accuracy
over the hosted 27B class: not currently evidenced — the two goals
differ by exactly the Intelligence axis.**

- **Already won, today, on the board's own terms: cost and speed.**
  Local-zero-cost and sub-150 ms p50 through the fusion gate vs a
  0.18 s board-best — orders of magnitude, not margins. Error rate on
  the 231: fork_4b 23.4 % vs Kev-4B 26.3 % / decider-2b 29 % —
  best-in-class ≤4B; the hosted tier sits at ~13 %.
- **The path runs through measurements we already queued.** #105
  (fusion-v2 on the 231) is the single highest-value number left:
  F29's +9.2 pp held-out lift, transferred at even half strength,
  puts the full system near 0.81–0.82 on the 231 — past every open
  ≤9B row, still short of hosted Jev. The official self-hosted
  submission (1,500 decisions, their protocol) then buys the real
  Intel/Calib read and tells us whether #88 (own distill) + #92
  (ONNX rungs) + the §10.4 conformal gate can close on the 31B class.
- **The "unique approach, no large LLM" thesis is the only unclaimed
  position on the board**: the current #1 has a 31B underneath;
  nobody combines local-free + sub-second + calibrated abstention +
  ≤4B-class accuracy. That combination is what the composite rewards
  (three of its four axes).
- **Caveats.** (i) Suite sensitivity (§14.1) means "top of the
  benchmarks" must be claimed per-benchmark with artifacts, never as
  a universal number. (ii) jabr is the OOD gate the field flunks
  (0.966 vs 0.704) — our abstention posture should be *presented*
  there, where automation-share-at-error-budget is the honest metric
  a ladder wins. (iii) The board is self-attested; independent
  replication (their harness, our artifacts) is the credibility
  moat, the same way the 231 split's author-harness runs are ours.

### §14.4 Transferable insights, per source

1. **pinggy** (survey + M3 Pro hands-on): the jabr failure-mode map —
   Von collapses to one mode on unfamiliar domains, GLiNER2
   over-triggers on keywords (the failure class our F30 leak test
   audits for), Laya compresses rating scales. Laya's candid README:
   base checkpoints near chance zero-shot (0.362/0.342 vs 0.318
   random) and the 0.766 headline is trained on the evaluated
   benchmark's own split — the in-sample contamination pattern F30
   exists to catch; also weak on large option sets (Banking77 0.425
   vs Jev 0.870) from a fixed per-option token budget — our
   candidate-conditioned parallel slots have no such cap, and a
   large-candidate-set robustness claim is testable with the D9
   256-candidate bench. NanoJev: a 0.6B purpose-trained model beats
   Jev 128/128 vs 56/128 **inside its training domain** (ViZDoom) and
   fails outside — the domain-distill thesis behind #88, plus
   evidence that "beats Jev at one thing" is a publicly holdable
   position. jevlike's **shuffled-context control** (pair each menu
   with the wrong context) is a cheap label-free OOD probe worth
   adding to suite reporting. And: stars ≠ quality (Von, 546 stars,
   leads the open field on jabr over 19.3k-star Laya).
2. **Kev's evaluation discipline** (pinggy/datacamp): automation
   share at a fixed error budget as a first-class published metric
   (Kev-9B 0.45–0.57 vs Jev 0.70 at 5 %) — directly computable from
   our gate-coverage rows; adopt in BENCHMARKS.md beside ECE. Their
   held-out-new-sources split (0.822 vs Jev 0.857) is the same move
   as our F29.
3. **SemIf** (pinggy/datacamp): direct option-logit readout is 5.21×
   faster than JSON generation; reusing one long state across 21
   criteria lifts 2.33 → 20.03 decisions/s — third-party replication
   of the single-pass readout and shared-prefill levers (Kai's
   `shared_ctx.py`, §10.5, same shape).
4. **apidog**: mini-jev's 6,750-observation paired test — JSON
   generation 0.909 vs letter-logit 0.907 (CI [−1.44, +1.04]) — the
   readout mode costs no accuracy at ~4× latency: independent
   replication of the F26 verdict-slot thesis with published CIs.
   Jev pricing ($0.042/M input tokens, output free, 70–500 ms) is
   the cost bar; their closing line — local wins privacy, not cost,
   "once GPU time is counted" — is precisely the claim our ladder
   rebuts: ~1 ms CPU gate decisions amortize to ~free.
5. **scriptbyai**: Cloudflare ships Clef/Clef-flash (27B/9B,
   Apache-2.0) — the hosted-tier Intel ceiling we do not chase;
   Kev-27B 0.889 on its New Sources set (80 GB-GPU class).
   **jevos-v3** (CPU-only single native binary, no Python, 80.8 % on
   999 hand-written yes/no) is our closest philosophical competitor
   and is *not* on the Benchmark Heaven board — watch item + possible
   231 arm.
6. **geeky-gadgets**: 20–30 projects in under two weeks; "cascade
   modeling — lightweight models for simple tasks, reasoning models
   for complex ones" is now mainstream advice: the ladder's thesis,
   independently arrived at. Decider's 32k context — context length
   is a board-visible axis.
7. **benchmarkheaven**: the board structure itself (§14.2) — the
   eligibility line, the harmonic composite, the weak cost/latency
   correlation, and rerankers earning Calib points with zero Intel.
8. **jevbench.xyz**: protocol sensitivity as a published finding
   (one prompt change moved recall 85.7 → 98.4, "same model, same
   rows") — grounds D15's refit-on-change discipline and §7.2's
   template-contract doctrine; also offers private evaluation
   (register interest, no data upload) — a possible official-number
   path that does not require publishing artifacts.

### §14.5 Actionable deltas + watch-list (merges under §9.3/§10.7/§13)

1. **#105 reframed**: the fusion-v2 231 re-run is the
   pre-submission measurement for the dominance program.
2. **NEW #107 — benchmark-dominance program**: official Benchmark
   Heaven self-hosted submission (1,500 decisions, their protocol,
   artifacts published); ~~jabr run if the harness is public~~ — DONE
   2026-10-06: the harness is public (CC0,
   `jabr/classifier-benchmark`); zero-ML engine self-run scored
   **0.401 macro on v2** (866 cases, 0 errors, 1.6 ms/case;
   BENCHMARKS.md Board C); the adapter is the artifact an upstream
   backend PR would carry, if the owner wants one. ~~the
   automation-share-at-5 %-error column~~ — DONE 2026-10-06 (Board A
   `auto@5%` column): engine 0/231, vtx 0/231, Jev-Style bridge
   **84/231 (0.364)**, fork label-only (—). The zeros are findings:
   the engine's p≥0.999 single-entry (proof/relational) rows run at
   34% precision on the OOD 231 — below base rate — which is B4's
   per-mode calibration wrapper quantified; the confidence gate
   refusing full automation at 38% accuracy is the design working.
   Still open: the
   board submission and the
   large-candidate-set (77-label-class) robustness probe via the D9
   256-candidate bench (needs a fork/GPU window; post-training).
   ~~the shuffled-context OOD probe~~ — DONE 2026-10-06 (Board B
   "Shuffled-context OOD probe"): derived suite shuffles each item's
   context chunks (80/120 reordered, 40 lexical controls unchanged);
   **zero decision flips, per-class accuracy byte-identical**, 11/120
   confidence moves all downward and all ≤0.026 — the engine is
   order-invariant by construction (facts + BM25 term sets, not
   positions), the order-invariance third of the OOD picture beside
   jabr's distribution-shift direction and the p≥0.999
   overconfidence direction.
3. Watch: jevos-v3 (CPU-binary posture twin; candidate 231 arm); Von
   (jabr open-field leader, 395M ModernBERT — same family as our
   model2vec/vtx line); Clef-flash (Apache-2.0 9B, possible rung
   candidate); CLM-35B (§6.6, ~Oct 6–10); Fastino GLiNER2.5-Decide
   arms (§9.3-7 unchanged).
4. Reporting rule: whenever an external number is quoted in
   marketing material, carry the §14.1 divergence table with it —
   one number per surface, never a universal claim.


### §14.6 The imajev / Jev-Omni family: the ecosystem converges on our serving contract (2026-10-06, user-directed)

Six repos scanned (HF API metadata + full cards): `mohit67890/imajev-{4b,9b}`
(the family's authors), `mindchain/imajev-{2b,4b}-GGUF` (third-party runnable
GGUFs; an `imajev-9b-GGUF` also exists), `akhilaaa3/Jev-Omni` (12B merged
multimodal classifier), `Reza2kn/Jev-Omni-Q4_K_M-GGUF` (its independent
quantization). All Apache-2.0.

1. **The de-facto ecosystem serving shape is ours.** Three independent
   projects ship the same architecture OC's model crate already pins:
   typed decisions over a candidate list, probabilities per option, no
   generation; a small separate decision head/readout artifact (imajev
   `decision_readout.safetensors` 2.6 MB; Jev-Omni `head.pt` →
   `decision-head-f32.npz` 3.78 MiB; our kai/julia contracts with committed
   fixtures); and a client-side temperature JSON shipped next to the weights
   (imajev `calibration.json`, JEV-27B §11's L-BFGS fits, our `Calibration`
   seam + D15). The Jev `POST /v1/systemone` wire shape (plus `images`,
   `unknown_probability`, `abstained`) is the contract third parties build
   against — our `opencodifier-schema` Jev adapter already speaks it.
2. **imajev-2B readout mechanics confirmed from the artifacts** (downloaded
   `decision_readout.json` + `calibration.json`): 255 option codes mapped to
   fixed token ids (A–Z = ids 32–57; AA–JT = single vocab tokens), readout =
   **vocab logits at the answer position restricted to those ids, ÷ T=1.646,
   softmax** — the lm_head IS the readout, so a plain llama.cpp GGUF needs no
   head file (contrast Jev-Omni, whose separate 256×hidden head must be
   applied client-side over `llama-server --embedding --pooling none` hidden
   states). Directly harnessable by `run_jev_native.py`'s single-position
   readout machinery.
3. **Set the prior before the run: the 2B is a previous-generation adapter**
   (the 4B card says so explicitly) and its own calibration fit note records
   the served number: **60.4 / ECE 0.123 on JevBench hard (rot4+cal)**, with
   in-distribution T≈1. Our fork 2B's recorded 0.725 on the same leg already
   beats it. The measured value of the #125 run is protocol-consistency on
   OUR harness, not a leaderboard upset. The real open-model bar is
   **imajev-4B**: JevBench hard 72.1% single-pass (+0.9 at 4 rotations),
   ECE 0.082; board JevBench v1.4.2.2 #1/91 (composite 67.37); Image JevBench
   76.4 (#2/50, best open); DecisionBench #3/56 (79.65). Q4_K_M 2.42 GB is
   the follow-up arm if the 2B leg is worth extending.
4. **Calibration policy evidence, now two-sided.** JEV-27B (§11) fits
   per-kind temperatures; imajev-4b TRIED a per-type fit and REJECTED it —
   it lowered hard-tier ECE but pushed pooled ECE past their 0.03 guard, so
   they ship a single T=1.305. Resolution: the pooled-ECE guard decides,
   which is exactly our ECE-worsening gate. #124's calibration A/B
   (single-T vs per-kind on the same checkpoints, eval-only) is the local
   measurement of this.
5. **`unknown` as a trained readout class.** imajev trains the 256th readout
   code as `unknown` with unknown targets at 17.15% of stage-1 data (capped
   15% on teacher-labelled), 22% in the phase-3 hard stage; the wire response
   carries `unknown_probability` + `abstained`. Our abstention lives at the
   POLICY layer only; the head cannot express "can't tell" distributionally.
   Concrete r3+ training delta → queued as #124 arm D (one variable per arm;
   never confounded into the dedupe A/B).
6. **Hard-question mining curriculum** (imajev phase 3): keep a question only
   when every teacher agrees with intent, mine what the current release gets
   WRONG from a 210k pool (58k text + 32k image kept), relabel failures with
   teacher distribution targets, replay, and repeat one round (round 2 = the
   13,247 still-failed + equal replay). Their shipped adapter is a
   weight-space average of the hard adapter and the soft-target continuation.
   A directly adoptable shape for our r3 data program; note it keeps hard
   rows with distribution targets rather than dropping them — relevant
   context for #124's drop-vs-down-weight measurement.
7. **Reporting artifact worth adopting:** the automation/threshold trade
   table — at ≥80/90/99% confidence: 63%/58%/40% automated at
   94.9%/97.5%/100% correct. The same animal as our Board A `auto@5%`
   column; add the confidence-threshold sweep to BENCHMARKS.md when the
   post-training arms land.
8. **Rotation averaging at serve time**: 4 option orders averaged, +0.9 hard
   (350 ms vs 96 ms p50 on H100) — the serving-side twin of JEV-27B's 30%
   permutation-training augmentation and our position-bias record. Cheap,
   calibration-compatible (a separate rot4 calibration file), K× cost — a
   verify-band/escalation lever, not a default-path lever.
9. **Jev-Omni**: Gemma-4-12B-it merged + separate 256-way head; claims
   86.15% micro on the 231 public JevBench decisions, ECE 0.040 (Medium);
   sweet spot ≤20 options; audio ≤30 s, video 16 frames. 123 likes, 0
   downloads — visibility is not adoption. Its Q4 quantization report is
   honest about limits: 3/4 published text examples reproduced, max
   probability delta 0.210, "do not assume source-equivalent probabilities
   or calibration"; Q4-vs-BF16 benchmark deltas (choice 98.75% vs 95.42%)
   are cross-protocol, not quantization effect sizes. Security note worth a
   doc line on D26: the `--embedding` endpoint exposes hidden states —
   loopback only (our backend already binds loopback).
10. **The five Qwen3.5 GGUF conversion traps** (mindchain
    `build_imajev_gguf.sh`): arch lookup lives in the converter table, not a
    filename (no `qwen35.py`); `--mmproj` silently drops weights on the text
    model; the config announces an MTP layer the checkpoint lacks
    (`--no-mtp` fixes text and is undefined for vision); hand-patching
    `block_count` creates a second bug; the mmproj step fails silently
    without `preprocessor_config.json`. Recorded for conversion-adjacent
    work; the no-self-conversions rule stands — mindchain's builds ARE the
    ready-made artifacts for this family.
11. **Validation-discipline additions from their card**: the blacken-the-image
    probe (same question, blacked image; the distribution must move — 20 s
    that separates a vision decision model from a text model handed a
    picture) and the "task in state" effect (without the task, the same model
    answers near-flat and "guessing looks plausible"). Adopt for any future
    multimodal arm.

**Deltas:** #125 created (imajev-2B arm: probe → text leg vs 2B-class peers,
prior = below fork 2B per item 3); #124 rewritten as the measured A/B the
user directed (2026-10-06) + calibration fold + arm D; watch imajev-4B Q4_K_M
as the next arm if the 2B leg lands; #107 board watch: JevBench is now
v1.5.4 (106 systems, Jev 1.13 #1 at 80.0) and Image JevBench v0.1.5 (50
systems, Wity-1 API #1 at 80.2) — open-model bars 70.8 text / 76.4 image.

### §14.7 JevBench open-weights board snapshot + the 2B/4B arm landscape (2026-10-06, user-directed)

Live-board fetch (benchmarkheaven.com/jev-models, 2026-10-06): **JevBench
v1.6.1** (board v1.7.12), 1,500 decisions/system, **100 ranked open-weights
systems** (APIs live on a separate board; Jev 1.13.0 sits unranked as the
reference at Cap 77.1, $0.032/1k, 0.24 s). Capability Score =
mean(Intelligence, Calibration); cost and median latency are separate axes;
"Jev-class" = within 2× Jev's cost and latency. Live boards drift — §14.6's
v1.5.4/106-systems cite was the mindchain card's frozen snapshot; one number
per surface extends to one *version* per citation.

1. **The top is frontier-base-dominated**: #1 Quyet-1.0-Large 81.7 (Gemma-4-
   31B-it), #2 deck-31B 77.6 (frozen 31B), #3 torchcast-decision-12b 71.7,
   #4 Jev-Omni 71.3, #5 Winnow-12B Q8 71.2, #6 Cygnet 70.9 (all Gemma-4-12B),
   #7 decider-chat-on-31B 70.6, #8 GEV-26B-Decide 70.1 (26B-A4B MoE + LoRA).
   The decision-quality axis is being bought with backbone scale — which is
   the ladder thesis' foil: the board's own cheap end ranks encoders and
   rerankers on the same page (verdict-small, multilingual-e5-small, Cap
   37.6 at $0.00087/1k), and our zero-ML engine arm is the local proxy for
   that end.
2. **The Qwen3.5-4B trio the user flagged** (all Q4_K_M ≈ 2.71 GB): #10
   Plumb-4B 65.4, #11 decider-4b v2 64.9 (Qwen3.5-4B-**Base**), #12 JevK5
   v0.3 64.2 — with #9 Bespoke Nimble 9B v3 66.8 (Qwen3.5-9B LoRA) just
   above and #13 Clef-Flash 63.8 (Qwen3.5-9B post-train). Note Imajev-4B
   sits at 61.9 Cap on the live board while its own card cites 67.37 on
   v1.4.2.2 — cross-version board comparisons are exactly the trap §14.1's
   divergence table exists for.
3. **Rank 31 Decision 2B (FlyMy, "best 2B")**: Cap 54.8 with a split profile
   — Intelligence 22.9, Calibration 86.8; the composite rewards calibration
   over raw skill at the small end. Its card: openbmb/MiniCPM5-2B + LoRA
   (26.2 M trained params), **pointer-head** readout (a third readout family:
   imajev = letter-code tokens via lm_head; Jev-Omni = separate linear head
   applied client-side; FlyMy = pointer head), frozen temperature from own
   replay. Public-231 self-run 75.32% (easy 100 / standard 83.33 / hard
   59.46). NOT clean open weights — "evaluation use per
   EVALUATION-PERMISSION.md; a general commercial weight licence is not
   asserted", plus disclosed provenance gaps (unestablished entitlement on
   some upstream records). Evaluate-able, not adoptable.
4. **decider-2b (Mapika, v11)**: Qwen3.5-2B-Base, Apache-2.0 clean. Readout =
   **option-letter token logits at each answer slot ÷ fitted temperature** —
   the same mechanics as imajev-2B, so ONE runner readout adapter evaluates
   both arms. Per-type temperatures shipped (regression at T=1.145); ECE
   in-task/held-out 0.0383/0.0847 (bf16) and 0.0385/0.0815 (Q4_K_M) — far
   better calibrated than imajev-2B's self-recorded 0.123. Their 95-task /
   144,226-question quantization regression: Q4_K_M −0.3 in-task / −0.5
   held-out pts vs bf16 (worst: paws −4.5); **Q8_0 = bf16 quality (99.16%
   row match, "the recommended file")**; one long-context outlier where
   Q4_K_M chose another option (0.53 vs 0.62). Speed: 0.12–0.31 s/request
   on 8 CPU threads — comfortably host-runnable.
5. **Harness-discipline confirmations from decider's card**: batching
   multiple prompts per decode shifts probabilities "by up to 0.02 in BF16
   and 0.16 in Q4_K_M" — quantization comparisons are only valid under
   identical batch shape. Our byte-locked suite's serial per-item decode is
   already the correct discipline; record it as a stated requirement for any
   third-party arm.
6. **The 2B-class table for #125** (each on its OWN surface — not mutually
   comparable; #125 exists to put them on ours): our fork 2B 0.725 JevBench
   hard (verdict slots); decider-2b Q4_K_M 0.7984 in-task / 0.7472 held-out
   (their 95-task suite); decision-2b hard 0.5946 (public-231 self-run);
   imajev-2b 0.604 JevBench hard (its calibration note). Prior: our fork 2B
   leads the hard leg on record; decider-2b leads on calibration.

**Deltas:** #125's arm set is now: imajev-2B Q4_K_M (downloaded, readout
confirmed) + decider-2b Q4_K_M (same readout adapter, Apache-2.0, download
pending) vs our fork 2B, all on the byte-locked suite + 231 leg; one
prompt per decode, quantization file recorded per arm. decision-2b-preview
stays out of the default arm set (evaluation-only licence + provenance
gaps) — record-only unless the user wants it run. Future-arm watch: the
Qwen3.5-4B Q4_K_M trio (Plumb/decider-4b-v2/JevK5) defines the next size
class; imajev-4B Q4_K_M (2.42 GB, phase-3 trained) is the single most
informative 4B add.

## §15 — Zero-shot classification, the Jev evaluation literature, board scoring rules, and the ecosystem press (2026-10-07 sweep)

Four parallel research clusters, 23 sources, fetched and read the same
day (deep-read by research agents; numbers below are as published on
the fetched pages, slants noted inline). This section merges them;
where findings touch existing records (D15, D25, §14, BENCHMARKS.md's
cost methodology) it says so instead of restating.

### 15.1 The Jev evaluation literature — two papers that read like a spec review

- **arXiv:2609.37647** ("Evaluating and Benchmarking the System One
  Model JEV", Fraunhofer IAIS): 37 datasets, 346,009 requests against
  jev-1.13.0. Jev beats Qwen3.8-27B on 27/37 and Gemma-4-E4B on 37/37;
  pooled Choice **ECE 0.028**; US$9.15 for the whole campaign at 0.36 s
  mean latency. Reference-scoring recipe: one forward pass, single-token
  letter codes (A, B, … AA), restrict + renormalize the next-token
  distribution; median 0.997–1.000 of mass on valid codes. Documented
  pitfall: numeric candidate keys were abandoned because the key "36"
  collided with an atomic-number answer — a one-line adapter guard for
  our Jev schema work. Calibration is **overconfidence on 20/22
  datasets** and concentrates where accuracy is low (Spearman
  ρ = −0.83 between accuracy and ECE).
- **arXiv:2609.34024** ("Jev in Medicine"): the sharpest single finding
  of the sweep — **Jev is better calibrated (ECE 0.063 vs 0.146) and
  discriminates better (AUROC 0.845 vs 0.801) than GPT-6 on MetaMedQA,
  yet LOSES selective prediction** (AURC 0.087 vs 0.070; 93.9 % accuracy
  at 49.7 % coverage vs 94.2 % at 52.8 %). Calibration quality is not
  decision quality; the accept/reject ranking is what the gate consumes.
  Also: calibration is **task-local** (same model: ECE 0.063 on one
  benchmark, 0.141 on another — "calibration is not a model property");
  abstention is **not emergent** (on 162 "don't know"-answerable items
  the model abstained 10.5 % of the time, but when it did abstain it was
  right 17/18 — the behavior exists and is usable when elicited);
  Jev's separate `confidence` field ranks identically to its probability
  (Spearman 0.964–1.000) but sits systematically 0.04–0.10 lower —
  a coverage dial, not new information.
- **Both papers converge on what our confidence gate should measure**:
  risk–coverage curves (AURC) and accuracy-at-coverage, not ECE alone.
  Our `Calibration` seam already exists; this defines the *acceptance
  metric* that should gate a calibration artifact, per task/node.

### 15.2 Zero-shot mechanisms — what transfers to the ladder's cheap rungs

Sources: jaketae.github.io, statworx.com, datacamp.com,
sparknlp.org, towardsdatascience.com (+ arXiv:2312.17543 and
arXiv:2502.03793 from the arXiv cluster).

- **NLI verbalization** (premise = text, each candidate label wrapped in
  a hypothesis template, entailment scored per pair): the standard
  zero-shot mechanism; **+9.4 % zero-shot balanced accuracy** from a
  multi-dataset NLI mix (arXiv:2312.17543, DeBERTaV3, CPU-feasible).
  Cost: N forward passes for N candidates. The hypothesis template is a
  **tunable, cache-relevant parameter** (statworx: a German template
  requirement; datacamp: descriptive phrases beat bare label words) —
  if adopted, the template string must join the `CacheKeyBuilder` input.
- **Single-pass MLM-head classification** (arXiv:2502.03793,
  ModernBERT-Large-Instruct, 395 M params): one forward pass scores ALL
  candidates — `[input] [anchor] [MASK]`, read the MLM distribution
  restricted to per-candidate verbalizer tokens. Zero-shot MMLU 43.06
  (best in class; 93 % of a model 4× larger). **Backbone-sensitive**:
  identical recipe collapses on RoBERTa (26.44 avg vs 47.8) — any
  adoption is fixture-gated per the model-import parity discipline
  (quality.md). Structurally the cheapest realization of our
  candidate-conditioned decision model: N candidates, one pass, f64
  softmax in Rust behind the existing `InferenceBackend` seam.
- **Failure modes worth engineering against**: raw entailment softmax is
  shipped as "confidence" with no calibration by every library source
  (contradiction with PLANNING §73 — our invariant is right); vague/
  overlapping labels split confidence; fine-tuning gains are small when
  the pretrained representation is strong (~1.7 pts on IMDB) but large
  on fine-grained ordered tasks (SST-5 61.13 vs 59.28 — our Score kind).
- **Corpus-prep discipline** (2312.17543): ~17 % of an established
  multi-dataset collection was probable label noise; capping at
  ≤500 texts/class and ≤5,000/dataset (51,731 texts) BEAT >1 M uncapped
  on zero-shot transfer — "diversity and quality over quantity".
  Directly applicable to merged-v3's 173k rows and the 69 %-mixed-gold
  quarantine pool (#124): cap + noise-score + quarantine-with-manifest,
  never silent deletion (noise removal disproportionately deletes
  minority classes).

### 15.3 Board scoring rules, decoded (benchmarkheaven.com/jev-models v1.6.1/board v1.7.12)

The submission-relevant mechanics, exact from the board's own pages:

- **Capability = mean(Intelligence, Calibration)** within caps
  (≤2× Jev cost = $0.0646/1k, ≤2× Jev median latency = 1.23 s).
  Calibration is HALF the headline and the cheaper axis to buy —
  leaders sit I 73.4 / C 82–90; every Calibration point is worth an
  Intelligence point.
- **Composite** (secondary) = 4/(1/I+1/C+1/S+1/K) with ×(axis/50)²
  soft gates below 50 — a CPU engine that maxes Speed and Cost (our
  $0.0125/hr modeled basis lands Cost 85–100) lifts a mid Intelligence
  into the top five: e.g. I 60 / C 90 / S 95 / K 97 ≈ 82 Composite,
  above the current best ranked 71.4.
- **Public-sealed gap penalty**: −1 Intelligence point per excess point
  once (public − sealed) exceeds G_med + 8 (≈10.6) — tuning to the
  public split is structurally self-defeating; matches our
  no-teaching-to-the-test posture.
- **Noul decisiveness**: P ∈ [0.20, 0.80] either vanishes from the
  competence cell (Heaven) or counts WRONG (Open-Jev's 231 protocol) —
  mid-band hedging is a direct points loss on both boards. Our Boolean
  threshold policy must be per-surface.
- **Refused/failed answers count wrong for Intelligence but are
  EXCLUDED from Calibration** — a hard refusal path does not poison
  the calibration axis.
- The 23 very-long items (77–82k tokens) cause most rivals' only
  failures; a lexical path with no context window converts them.
- Category radar: **rules/policy is the largest pool (1,011 of 1,887
  items) and math & numbers is where the field is weakest**
  (leaders score 31.5–43.4; TypeSafe's own documented failure clusters
  are math, dates, indirection, noisy states — "often move into
  code"). Our exact-rule/relational rungs aim exactly there.
- Open-Jev's published BANKING77 negative baseline is third-party
  evidence for the deterministic-first thesis: their released 2B model
  routed 64.8 % vs **BM25 top-1's 82.4 %** — mirror its reporting
  format (coverage + accepted error + candidate recall, never accuracy
  alone).
- Release-gate re-read: our JevBench ≥ 0.6494 gate is a *shipping*
  gate; board parity on the 231 split is 0.8528 (Open-Jev-27B; Jev ≈
  0.866) — ~0.20 above the gate. Naming caution (three unrelated
  things are called "JevBench": Heaven's scored benchmark, the
  jevbench.dev game board, the community 534-task suite) — every
  public claim names its surface.

### 15.4 The ecosystem press — cost methodology, cascades, and the credibility bar

Sources: layer3labs.io, docs.litellm.ai, mindstudio.ai,
ayautomate.com (×2), typesafe.ai.

- **Three cost methodologies in circulation**: list-price × assumed
  tokens (largest multiples); **billed $/1k from a real meter** (AY:
  Jev $0.0136–0.0400/1k vs Terra 40–49× dearer — the market's most
  defensible form); registry-priced observed tokens (LiteLLM). None
  uses a hardware-amortized basis — ours (benchmarkheaven's
  $0.0125/hr convention) is the board's own currency. Obligations:
  publish modeled-CPU and billed-equivalent rows side by side, always
  naming task, n, and tokens/decision; **tokenizer drift breaks
  price-list ratios** (Jev billed 360 tokens where GPT billed 153 on
  the identical prompt).
- **The vendor headline did not reproduce**: TypeSafe's 193.6×/444.6×
  collapses to **2.0–3.6× faster / 40–49× cheaper** under fair LLM
  baselines (minimal reasoning, strict schema, short outputs) — always
  state the baseline configuration next to any multiple.
- **Agreement-with-a-bigger-model inflates 6–8 points** (AY: 90.0 %
  agreement vs 83.8 % true accuracy) — which is precisely the design
  of TypeSafe's workflow evals (reference = average of two frontier
  LLMs, no ground truth). Any of our rows scored against a reference
  model instead of a labeled key must say so.
- **Confidence-gated escalation is now a documented vendor pattern**
  (AY/TypeSafe cascade: Terra-level accuracy at 26–28 % of cost; gate
  0.80 → 80.6 % answered at 93.8 % accuracy). It is table stakes, not
  a differentiator. What NO system in the ecosystem publishes:
  abstention as an outcome, risk–coverage curves, execution traces,
  adversarial input isolation, offline operation. Those five are our
  unopposed claims.
- **Confidently-wrong clusters on overlapping labels** (AY: all five
  confident 8-way errors were one overlapping pair at 0.93–0.99 —
  "the confidence score cannot tell you that your label set
  overlaps"): a label-set overlap preflight (pairwise BM25/embedding
  similarity over candidate names — both already in the tree) that
  downgrades to abstain converts this entire documented failure class.
- **Credibility bar**: LiteLLM's frozen-evidence bundle (corpus/
  protocol/registry SHA256s, seeded shuffle, clustered bootstrap,
  repro script) and Layer3's §6 independence checklist (ECE + Brier,
  head-to-head vs schema-constrained LLMs, degradation near context
  ceilings, p95/p99 under concurrency, open data) — **no Jev vendor
  passes it**; meeting it is mostly packaging on top of
  BENCHMARKS.md's existing methodology section.
- **Tail latency compresses speedup ratios** (LiteLLM: 5.43× at p50 →
  3.88× at p95): report p95/p99 beside p50; our cost_per_1k is
  p50-derived today.
- **Dynamic-instruction following is Jev's one documented decisive
  win** (MindStudio: policy changed post-deployment, Jev adapted,
  fine-tuned classifiers kept matching the stale pattern). Our
  declarative DAG + version-stamped cache keys are the natural
  counter — a benchmark leg proving runtime policy edits with zero
  retrain closes the only axis where hosted zero-shot clearly beats
  local.
- Compatibility/discovery seams: `mandu5/jevcompat` (System One API
  conformance suite) and jevland.mnimiy.com (404-repo index) are the
  cheap credibility/visibility plays; jevbench.dev (game wins) needs a
  harness we don't have — enter through an existing one or record the
  surface out-of-scope in the board map.

### 15.5 Contradictions and traps worth keeping

1. Raw softmax presented as confidence by every library source vs
   PLANNING §73 — our invariant holds; the empirical caveat (AY's
   AUROC 0.990 with post-hoc thresholds) is ranking quality, not
   calibration proof.
2. ECE as the headline calibration metric vs the medicine paper's
   ECE-vs-AURC divorce — adopt both, gate on risk–coverage.
3. Abstention posture vs board scoring — mid-band Noul hedging LOSES
   points on both boards; abstention wins only where coverage is
   reported separately. Per-surface threshold policy, decided, not
   ambient.
4. Fine-tune vs zero-shot ordering (every source: fine-tune wins when
   labels exist) vs our ladder — already the architecture; the sweep
   adds the missing middle rungs (NLI head, MLM-head single-pass).
5. "Zero-shot will not close an accuracy gap by label tweaking"
   (datacamp) vs "template phrasing is the cheapest accuracy lever"
   (statworx) — both true: phrasing moves single-digit points,
   training moves tens.

### 15.6 Ranked enhancement program (merged across the four clusters)

1. **Risk–coverage acceptance for calibration artifacts** (engine +
   core): AURC + accuracy@50 %/80 % coverage per task/node, n ≥ 200
   floor, gate artifacts on it — not ECE alone. [2609.34024]
2. **Per-node/per-kind calibration + fitted Boolean thresholds** via
   the D25 `LadderPolicy` seam (empty = byte-identical): the largest
   measured lever in the entire sweep — tuned thresholds moved F1
   0.499 → 0.748 (UNFAIR-ToS), 0.243 → 0.353 (GoEmotions); median
   tuned threshold 0.86. Never hard-0.5 a Boolean. [2609.37647,
   2609.34024]
3. **Explicit abstain candidate** injected at narrowing time, scored
   by the model's own OOD/entropy or as an ordinary candidate;
   abstain-when-elicted was right 17/18. Pointer contracts must assert
   the synthetic candidate explicitly. [2609.34024, 2609.37647]
4. **Label-set overlap preflight** at candidate preparation: pairwise
   BM25/embedding similarity over candidate names → `label_overlap`
   trace event + downgrade-to-abstain under a config flag, measured
   on the 231 split before defaulting on. [ayautomate]
5. **MLM-head single-pass decision model** behind `InferenceBackend`:
   395 M-class encoder, all N candidates in one pass; fixture-gated
   backbone choice; Score-kind tasks are its sweet spot. [2502.03793]
6. **NLI verbalization head** as a structurally independent verifier
   arm (entailment per verbalized candidate, renormalized): a second
   opinion uncorrelated with the pointer/logits family. O(N) passes —
   budget-capped per node. [2312.17543]
7. **Board calibration buy**: per-type/per-tier temperature fitted on
   held-out data, ranked-probability error for Score + distribution
   distance for Choice as the verification metrics; Calibration is
   half of Capability and leaders sit at 82–90. Plus: no-refusal on
   the 23 long items via the lexical path, and the Capability-vs-
   Composite strategic decision (CPU engine maxes S and K — Composite
   arithmetic favors us). [benchmarkheaven]
8. **Reporting parity bundle**: ECE + Brier + AUROC + reliability
   curve per rung in Board B and the board CSV; p95/p99 beside p50;
   dual cost rows (modeled CPU + billed-equivalent) with task/n/
   tokens named; frozen-evidence packaging (SHA256s + repro script)
   per board run. [layer3labs, litellm, ayautomate]
9. **Corpus-prep gates for #124**: per-class/per-dataset caps +
   label-noise scoring → quarantine-with-manifest (never silent
   delete). [2312.17543]
10. **Dynamic-instruction leg** on a routing task: runtime policy
    edit, zero retrain, Jev vs our DAG — the only axis the press
    says hosted wins. [mindstudio]

Deltas queued into the plan: items 1–4 are engine work shaped for the
Phase 20b–20f tranche neighborhood (new behavior, new tests — they
shrink the coverage residual the same commit that lands them); item 7
is #107's submission work; item 8 is board/reporting; item 9 feeds
#124; items 5–6 are model-rung options behind existing seams, to be
sized before commitment. Watch-list adds: mandu5/jevcompat
conformance, jevland listing, LiteLLM-style frozen bundles as the
standard run-output format.

### 15.6.1 Items landed from the program (status ledger)

- **Item 2 (fitted thresholds as ladder profiles)** — shipped
  2026-10-08: `ladder_fit.rs` fits the Boolean verdict boundary by
  deterministic grid sweep (F1 on the true class, ties toward the
  smallest threshold) and emits a `LadderProfile` document that
  round-trips the engine's real D25 loader; the cli's
  `ladder fit-boolean` writes it (commit `c5d2527`).
- **Item 3 (explicit abstain candidate)** — shipped 2026-10-08:
  `DecisionPolicy::abstain_candidate: Option<AbstainCandidate>` — a
  marked synthetic candidate carries the refusal; elicited abstention
  rides the same typed answer shape as acceptance, and the trace
  discloses `abstain_candidate` / `abstain_elicited`; absent-on-default
  byte-identical (commit `c5d2527`).
- **Item 4 (label-overlap preflight)** — shipped 2026-10-08:
  `DecisionPolicy::max_label_overlap`, Jaccard over candidate-id
  tokens, terminal abstention with `label_overlap*` trace facts,
  default byte-identical (commit `d5f4733`).
- **Item 8 (reporting parity)** — shipped 2026-10-08: parity.py
  (Brier/AUROC/reliability/p95–p99/tokens per rung over frozen run
  JSONs), board.csv columns, BENCHMARKS.md parity table + dual cost
  rows, frozen-evidence packaging with repro script.
- **Item 9 (corpus-prep gates)** — shipped 2026-10-08:
  `runner/corpus_gates.py` (schema, teacher-noise, per-class/
  per-family caps, byte-exact label-fight majority; quarantine with
  manifest, never silent delete) + the first corpus-v3 measurement
  (TRAINING.md §11).
- **Item 10 (dynamic-instruction leg)** — shipped 2026-10-08:
  `tests/dynamic_instructions.rs` pins the counter to the MindStudio
  axis end-to-end — per-request policy edits flip decisions with zero
  retrain, the cache keeps edits from bleeding either direction, a
  ladder swap names its source on the trace, and injected input text
  cannot edit the policy the operator set. CI-enforced on every
  commit.

### 15.7 Addendum (same day): dzhng/jevgrep — Jev as a code-search relevance judge

`github.com/dzhng/jevgrep` (dzhng = the browser-use author; MIT,
Node 22+): a coding-agent CLI — ask a natural-language repository
question, it walks the hierarchy, uses **hosted Jev** (TypeSafe /
Vercel AI Gateway / OpenRouter key required) to judge relevance across
folders → files → declarations, and returns files, reading leads, and
verbatim excerpts on stdout. Why it matters here:

- **It is a production instance of our thesis category**: a decision
  model used as a cheap relevance ROUTER/FILTER (with the LiteLLM
  complexity-tier classifier, the "decision models as filters" family
  §15.4 named). Its two-stage shape — cheap content previews narrow,
  Jev judges the shortlist — is our metadata-filter → BM25 → model
  ladder with exactly one hosted rung and no deterministic rungs below.
- **A local-first equivalent is unclaimed**: jevgrep sends eligible
  source to a cloud provider by design (their own warning: choose the
  search root you intend to send). "Repository question answering that
  never leaves the machine" — our narrowing stack + extraction + trace
  — is a legitimate, differentiated demo surface for the engine.
  Recorded as an idea, not a commitment; sized before any work.
- **Reporting discipline worth copying**: their own benchmark section
  states 7/10 vs 8/10 solves at ~40 % lower cost, one 10-task repeat,
  baselines run once, "a cost reduction with a quality tradeoff, not
  evidence of equal or better solve quality", and both runs failed the
  original quality gate — third-party practice of exactly the
  §15.5/§15.6(8) rule (never report cost without quality alongside).
- **Not adoptable as a tool for this harness**: requires a cloud
  provider key and sends source externally — contrary to harness
  rules (no code exfiltration; retrieval here is codebase-memory MCP).
  Ecosystem-index candidate (jevland "integration" category) at most.

### 15.8 Addendum (same day): AlphaDev — what an RL program-search result does and does not teach this engine

Source: `deepmind.google/blog/alphadev-discovers-faster-sorting-algorithms/`
(DeepMind, 2023). AlphaDev searches assembly one instruction at a time
with a reward that requires **correct output and latency jointly** —
every candidate runs against test inputs (for sort3, ALL 3-element
inputs) before it can score — and landed the first RL-derived routines
in LLVM libc++ (sort3/4/5 up to ~70 % faster; ~1.7 % on ≥250 k
sequences; a 9–16-byte hash path 30 % faster in Abseil).

Honest applicability to this runtime, ranked:

1. **Exhaustive verification of small-N paths is free here** — and we
   should use it. AlphaDev could test every sort3 input; our analogs
   are the small candidate sets the engine actually sees: softmax over
   K ≤ 8 candidates, the f64 cosine loop, BM25 accumulate over a
   shortlist. The B2/B3 equivalence gate (bit-identical decisions on
   15 committed graphs) should gain **exhaustive small-K property
   tests** (all orderings/permutations up to K=6, deterministic RNG
   beyond) — aggressive micro-optimization becomes provable, exactly
   the AlphaDev reward shape: behavior AND latency, verified together.
2. **Redundant-guard discovery** — their `min(A,B,C)` → `min(A,B)`
   find: comparisons that look load-bearing but are implied. Direct
   tie-in: COVERAGE group 6 is *precisely* "escalation-walk guards
   implied by earlier checks, precluded by construction" — unreachable
   arms are often droppable guards. A pass over executor walk guards
   asking "is this check provably implied?" both simplifies the code
   and closes the uncovered arm by deletion rather than by test.
3. **Instruction-level inspection stays a diagnostic, not a program.**
   We consume LLVM like everyone; our standing rule (objdump the `.so`
   actually carrying kernels — the llama.cpp ISA lesson) is the right
   weight: `cargo asm`/`llvm-mca` on the softmax/BM25 kernels only
   AFTER B3 lands, only if the criterion number still misses budget.
4. **Latency-in-the-reward is already our discipline** — D9 budgets
   are CI-gated criterion asserts, not after-the-fact benchmarks.
   AlphaDev validates the philosophy; nothing to add except keeping
   the budgets honest as rungs land.
5. **What we will not do**: run program search. The wins live at
   invocation counts (trillions of sorts) we do not have, on kernels
   (sorting) we do not own — `sort_by`/`BTreeSet` are std's. The
   differentiator here is decisions-per-microsecond at the pipeline
   level, which B2/B3/B4 attack by algorithmic shape, not instruction
   golf.

Executed (2026-10-08, PLAN Phase 24g): the exhaustive small-K
property tests landed as `crates/opencodifier-engine/tests/
small_k_exhaustive.rs` (every assignment K = 2…6, every presentation
order, the boolean grid, every score-level permutation); the executor
redundant-guard pass found exactly one provably-implied guard
(`distributional_ood`'s zero-denominator arm) and deleted it — the
other 35 missed lines are `#[non_exhaustive]` totality arms or
boundary-honesty guards behind cross-module validation, each now
documented in place rather than deleted.
