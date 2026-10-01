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
