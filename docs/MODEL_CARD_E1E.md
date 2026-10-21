---
base_model: Qwen3.5-2B
library_name: peft
pipeline_tag: text-generation
tags:
- base_model:adapter:Qwen3.5-2B
- lora
- verdict-slot
- opencodifier
- decision-model
inference: false
---

# OpenCodifier E1-E 2B Decision Model

A verdict-slot LoRA adapter on Qwen3.5-2B for typed decision-making (choice, boolean, score).
Part of the OpenCodifier decision runtime: opencodifier.pages.dev.

## Model Details

- **Model type:** Decision classifier — verdict-slot LoRA on Qwen3.5-2B
- **Base model:** Qwen3.5-2B (Apache-2.0)
- **Adapter architecture:** LoRA r=16, alpha=32, dropout=0.05
- **License:** Apache-2.0
- **Developed by:** [OPERATOR: add org name]
- **Released by:** [OPERATOR: add org name]

## Gates Passed

| Benchmark | Result | Gate | Status |
|---|---|---|---|
| Suite holdout (231 items) | 0.8083 accuracy | ≥ 0.75 | ✅ PASS |
| JevBench public 231 | 0.6840 accuracy | ≥ 0.6494 | ✅ PASS |

Full evaluation evidence: `benchmarks/decision-model/results/CALIBRATION.md` and
`benchmarks/decision-model/results/e1e-slot-231/results.jsonl`.

## Training

- **Corpus:** 187,822 decision items (e1-softlabel-v2), teacher distributions from MiMo
- **Teacher-gold agreement:** 96.1–97.7% argmax match
- **Hyperparameters:** lr=1e-4, batch=2, accum=8, max_len=2048, fp16, sdpa, seed=42, 1 epoch
- **Platform:** T5500 (2× V100, 120W GPU cap)

## Evaluation Results

### JevBench Public 231 (231 items, authors' harness)

| Metric | Value |
|---|---|
| Accuracy | 0.6840 |
| Macro | 0.6840 |
| ECE | 0.127 (calibrated T=3.362) |
| Brier | 0.332 |
| p50 latency | ~1.7 s (CPU, single decision) |
| Replay determinism | 231/231 |

### Suite Holdout (231 items, internal)

| Metric | Value |
|---|---|
| Accuracy | 0.8083 |

## Uses

### Direct Use

Use via the OpenCodifier binary:

```bash
# Engine-only (no model needed)
opencodifier decide --input request.json

# With E1-E model (requires llama-server running the decision fork)
opencodifier decide --input request.json --ladder-url http://127.0.0.1:8080
```

### Out-of-Scope Use

- Not a general-purpose language model
- Not trained for prose generation
- Not calibrated for non-decision classification

## Calibration

Temperature fitted on the suite holdout: T = 3.362.
Apply the calibration artifact from `results/calibration/qwen3.5-2b-q4_k_m/fit.json`
before evaluating on any surface other than the holdout.

## How to Cite

OpenCodifier decision runtime: https://github.com/[OPERATOR]/OpenCodifier

## Remarks

- Corpus withheld (per OpenCodifier model release policy)
- Calibration artifacts published separately
- Explicit opt-in pull only — never auto-downloaded at build time
