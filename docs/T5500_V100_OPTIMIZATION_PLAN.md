# T5500 V100 Optimization Plan — Strands Decider + GSQ Serving Stack

**Owner:** Autonomous loop (pi)
**Status:** Research complete; awaiting PSU (T5500 power supply replacement)
**Date:** 2026-10-19
**Research sources:** All sourced from §21 of RESEARCH.md (2026-10-19)

---

## Context

The T5500 has 4× NVIDIA Tesla V100 16GB (SM_70) — the 2017 datacenter GPU.
The replacement PSU is pending. While waiting, this plan documents the full
quantization and serving stack so the work can begin immediately when hardware
is available.

**Reference system:** Strata runs Qwen3.8-Flash-Next (~125B MoE) on consumer
GPUs via IQ quantization + expert caching. 1Cat-vLLM achieves 260 tok/s on
4× V100 16GB for Qwen3.8-27B-NVFP4 + DFlash2. These establish the
performance envelope for the T5500 V100 cluster.

---

## Hardware Profile — T5500 V100 Cluster

| Spec | Value |
|---|---|
| GPUs | 4× NVIDIA Tesla V100 SXM2 16GB |
| Architecture | Pascal / SM_70 |
| Compute | 5120 CUDA cores per GPU |
| Memory | 16GB HBM2 per GPU, 900 GB/s bandwidth |
| NVLink | NVLink 2.0 (shared memory space) |
| PSU | **REPLACEMENT PENDING** |

**V100 limitations for modern quantization:**
- No native FP8 tensor cores (H100+ feature)
- No Transformer Engine
- Requires specialized kernels (Flash-V100, 1Cat-vLLM custom ops)
- Memory per GPU is the binding constraint

---

## Decision Model Serving Stack

### Primary: Strands Decider Gemma 4 E4B

**Model:** `StrandsAgents/strands-decider-E4B-gemma4-v1-2610`

| Metric | Value |
|---|---|
| JevBench | 187/231 |
| Brier | 0.243 |
| ECE | 0.037 |
| Decision Index | 39.42 |
| Latency (H100) | 139ms median |
| Memory (bf16) | ~15GB |
| **Memory (Q4_K_M)** | **~4.6GB** |
| Memory (IQ2_XS) | ~2–3GB |

**Why E4B over 12B/26B:**
- 12B at Q4_K_M ≈ 9.2GB → requires TP2 (two V100s) or aggressive KV cache
  quantization. E4B at Q4_K_M ≈ 4.6GB → fits in one V100 with KV cache headroom.
- Decision Index 39.42 vs 47.14 (19% gain for 3× compute is poor ROI on V100)
- 12B Gemma 4 requires `num_attention_heads % TP == 0`; with TP2 the Marlin
  kernel constraint `(moe_intermediate_size/TP) % max(64, group_size) == 0`
  may fail silently and fall back to broken Triton path

**E4B is the T5500 V100 sweet spot.**

### Secondary: Strands Decider 2B Qwen3.5 (v1)

**Model:** `StrandsAgents/strands-decider-2B-qwen3.5-v1-2610`

| Metric | Value |
|---|---|
| JevBench | 180/231 |
| Brier | 0.280 |
| Memory (bf16) | ~4GB |
| **Memory (Q4_K_M)** | **~1.2GB** |

Fits in V100 CPU fallback mode or as a secondary warm-cache rung.
ONNX export: `onnx-community/strands-decider-2B-hobson-v19-ONNX`
GGUF export: `fabricant451/strands-decider-2B-hobson-v19-GGUF` (526 downloads)

---

## Quantization Options — V100 Feasibility

### Option A: GGUF via llama.cpp (Simplest, No New Builds)

```
Strands-Decider-E4B safetensors
  → llama.cpp quantize (Q4_K_M)
  → llama.cpp server --n-gpu-layers 99 --tensor-split 4,0,0,0
  → POST /v1/chat (OpenAI-compatible)
```

**Pros:** Zero new build. llama.cpp is already on the host.
**Cons:** llama.cpp does not natively serve the Strands Decider LoRA adapter
+ pointer head. Needs community GGUF export of Gemma 4 models.

**Status:** No GGUF export of Strands Decider Gemma 4 models exists yet.
The community (`fabricant451`) exported v19 (Qwen3.5) but not Gemma 4 v1.

**Action required:** Build the GGUF export or use the Qwen3.5 v19 GGUF.

---

### Option B: llama.cpp + Community GGUF (v19 Qwen3.5, Available Now)

```
fabricant451/strands-decider-2B-hobson-v19-GGUF
  → Download GGUF files
  → llama.cpp server on V100
  → POST /v1/systemone (Strands protocol) via adapter
```

**Status:** Available, 526 downloads. File sizes not yet catalogued.
Estimate: Q4_K_M ≈ 1.2GB for 2B model.

**Action:** Catalog file sizes, benchmark on T5500 V100.

---

### Option C: 1Cat-vLLM + Compressed Tensors (Highest Throughput)

```
Strands-Decider-E4B safetensors
  → 1Cat-vLLM compressed-tensors format
  → 1Cat-vLLM server (V100 SM70 kernels)
  → POST /v1/chat
```

**1Cat-vLLM** achieves on V100:
- Qwen3.8-27B-NVFP4: 61 tok/s @ 128K decode, 260 tok/s with DFlash2
- Flash-Next GSQ-RCO IQ3_S: 92 tok/s decode (C1 workload)
- Attention: 71.1 TFLOP/s (useful causal QK/PV compute)

**Status:** 1Cat-vLLM is a vLLM fork; it needs to be built from source.
No pip wheel for SM70.

---

### Option D: GSQ Quantization + 1Cat-vLLM (Best Quality/Size)

```
Strands-Decider-E4B safetensors
  → IST-DASLab/GSQ quantization (H100 or borrow GPU time)
    - 2-bit GSQ: {-2,-1,0,+1} × scale, GPTQ init + Gumbel-Softmax refinement
  → GGUF K-Quant export (llama.cpp compatible)
  → 1Cat-vLLM compressed-tensors (if not GGUF-native)
  → Serve on V100
```

**GSQ quality:** Llama-3.1-70B 2-bit GSQ at 75.57 vs QTIP 77.25 —
within ~1.7 points of vector-quantized methods with scalar kernels.

**Key insight:** The pointer head (~1M params) should be kept at Q8_0 minimum;
the LLM torso can use aggressive quantization.

**RCO (Rotated Calibration Optimization):** Applied to Qwen3.8-27B to produce
GSQ-RCO-GGUF. RCO allocates precision bits per layer under a budget.
For Strands Decider, RCO would find which transformer layers are most
sensitive to quantization and allocate more bits there.

**Status:** GSQ requires H100/Hopper for the quantization compute.
1Cat-vLLM serves GSQ GGUF natively through llama.cpp compatibility layer.

---

### Option E: TurboQuant v3 + 1Cat-vLLM (Aggressive V100-Optimized)

```
Strands-Decider-E4B
  → TurboQuant v3
    - INT4 group-wise quantization
    - AWQ-style activation-aware scaling
    - Protected FP16 channels (top 2% outlier-sensitive)
    - Optional SVD correction (rank 8)
  → 1Cat-vLLM compressed-tensors
  → V100 SM70 optimized serving
```

**TurboQuant v3 kernels (CUDA):**
- INT4 dequantization + fused matmul
- AWQ scale computation
- Protected channel handling

**Status:** TurboQuant v3 PR #779 (SDPA kernel) is draft/pending integration
into 1Cat-vLLM. The standalone TurboQuant v3 works for quantization but
the serving integration is not yet merged.

**Action:** Monitor 1Cat-vLLM PR #779. When merged, TurboQuant v3 becomes
the preferred quantization path for V100.

---

## Recommended Serving Architecture

```
┌─────────────────────────────────────────────────────┐
│              T5500 V100 Cluster                     │
│                                                     │
│  1Cat-vLLM (V100 SM70 kernels)                    │
│  ├── Strands-Decider-E4B (Q4_K_M, ~4.6GB)        │
│  │   └── LoRA adapter + pointer head               │
│  ├── Strands-Decider-2B-Qwen3.5-v1 (Q4_K_M, ~1.2GB)  │
│  │   └── Warm cache / fast fallback                │
│  └── KV cache: FP8 E4M3 (1Cat-vLLM optimized)     │
│                                                     │
│  Tensor Parallelism: TP1 per V100 for E4B          │
│  (E4B Q4_K_M ~4.6GB fits single V100 with KV)   │
│  (12B would need TP2, Marlin kernel risk)          │
│                                                     │
│  Speculative Decoding: DFlash2 paths where stable  │
└──────────────────┬────────────────────────────────┘
                   │ HTTP
                   ▼
         ┌──────────────────────┐
         │ OpenCodifier Engine   │
         │                      │
         │ EngineHandle::lexical │
         │   (BM25, metadata)   │
         │         ↓            │
         │ InferenceBackend trait│
         │   (Strands / ONNX)   │
         └──────────────────────┘
```

---

## Quantization Pipeline — Step by Step

### Phase 0: PSU Arrives → Baseline Measurement

```bash
# 1. Verify V100 visibility
nvidia-smi

# 2. Build 1Cat-vLLM from source
git clone https://github.com/1CatAI/1Cat-vLLM.git
cd 1Cat-vLLM
pip install -e .

# 3. Baseline: Qwen3.8-27B NVFP4 on 4× V100
vllm serve Qwen/Qwen3.8-27B \
  --tensor-parallel-size 4 \
  --quantization NVFP4 \
  --gpu-memory-utilization 0.90 \
  --max-model-len 8192

# 4. Benchmark: capture tok/s baseline
```

### Phase 1: Strands Decider E4B via HuggingFace Transformers (Reference)

```bash
# Install Strands Decider
pip install strands-decider

# Benchmark on V100 via transformers (bf16, single GPU)
python -c "
from strands_decider import StrandsDecider
model = StrandsDecider('StrandsAgents/strands-decider-E4B-gemma4-v1-2610')
result = model.ask(state='...', choice='team?=billing,sales,support')
print(result)
"

# This will OOM at bf16 (~15GB) on a single V100 16GB
# Expected: needs TP2 or aggressive KV cache quantization
```

### Phase 2: GGUF Export + llama.cpp Serving (E2B/2B first)

```bash
# Option: Use community GGUF (available now)
# strands-decider-2B-hobson-v19-GGUF

# Download
huggingface-cli download fabricant451/strands-decider-2B-hobson-v19-GGUF

# Build llama.cpp with V100 support
git clone https://github.com/ggerganov/llama.cpp.git
cd llama.cpp
cmake -B build -DLLAMA_CUBLAS=ON -DLLAMA_SM=70
cmake --build build --config Release -j16

# Serve on V100
./build/bin/llama-server \
  -m strands-decider-2B-hobson-v19-Q4_K_M.gguf \
  --host 0.0.0.0 --port 8080 \
  --n-gpu-layers 99 \
  -ngl 99 \
  -c 4096

# Benchmark
curl localhost:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "...", "messages": [...]}'
```

### Phase 3: GGUF Export of Gemma 4 Models

**Critical missing piece:** No GGUF export exists for Strands Decider Gemma 4
models (E2B/E4B/12B/26B-A4B). Required steps:

```bash
# Option A: Use strands-decider's built-in export tool
pip install strands-decider
python -m strands_decider.hf_export \
  StrandsAgents/strands-decider-E4B-gemma4-v1-2610 \
  --output ./gemma4-gguf/

# Option B: Manual LoRA merge + llama.cpp quantize
# 1. Download base: google/gemma-4-E4B-it
# 2. Apply Strands Decider LoRA adapter
# 3. Export to GGUF
# 4. Quantize with llama-quantize

# Quantize
./build/bin/llama-quantize \
  gemma4-decider-f16.gguf \
  gemma4-decider-q4_k_m.gguf \
  Q4_K_M

# Serve
./build/bin/llama-server \
  -m gemma4-decider-q4_k_m.gguf \
  --n-gpu-layers 99 -c 4096
```

### Phase 4: GSQ Quantization (Best Quality)

```bash
# Requires H100 or compute borrow (NOT on V100)
# Borrow GPU time or use cloud H100 spot

# Clone GSQ
git clone https://github.com/IST-DASLab/GSQ.git
cd GSQ
pip install -e .

# Configure for Gemma 4 E4B
# Edit configs/local/config.yaml:
#   model:
#     name: google/gemma-4-E4B-it
#   quantization:
#     gsq_bits: 2  # 2-bit GSQ

# Quantize (requires ~80GB GPU memory for E4B)
bash scripts/run.sh

# Export to GGUF for llama.cpp / 1Cat-vLLM
python save_model.py --checkpoint_dir ./training/checkpoint_dir/<run_id>/

# RCO: Allocate precision per layer under budget
# The RCO layer (arxiv:2605.00649) allocates more bits to sensitive layers,
# fewer bits to robust layers — maximizing quality per bit budget
```

### Phase 5: TurboQuant v3 + 1Cat-vLLM Integration

```bash
# Monitor 1Cat-vLLM PR #779 (TurboQuant SDPA kernel)
# Status: https://github.com/1CatAI/1Cat-vLLM/pull/779

# When PR merges:
git clone https://github.com/Kubenew/TurboQuant-v3.git
cd TurboQuant-v3
pip install -e ".[dev]"
python setup_cuda.py install

# Quantize
from turboquant import TurboQuantConfig, quantize_model
from transformers import AutoModelForCausalLM

model = AutoModelForCausalLM.from_pretrained(
    "StrandsAgents/strands-decider-E4B-gemma4-v1-2610"
)

config = TurboQuantConfig(
    bits=4,
    group_size=128,
    version="gemm",
    activation_aware=True,
    outlier_keep_ratio=0.02,  # 2% FP16 protected channels
    rank=8,                   # SVD correction
)

quantized = quantize_model(model, quantization_config=config)

# Serve through 1Cat-vLLM
# (When TurboQuant PR #779 is merged)
```

---

## OpenCodifier Integration

### `InferenceBackend` implementation for Strands Decider

```rust
// crates/opencodifier-runtime/src/backends/strands_decider.rs

use crate::{InferenceBackend, BackendConfig};

pub struct StrandsDeciderBackend {
    base_url: String,
    model: String,
    client: reqwest::Client,
}

impl InferenceBackend for StrandsDeciderBackend {
    fn decision(
        &self,
        state: &str,
        candidates: &[String],
        kind: DecisionKind,
    ) -> Result<CalibratedDecision, BackendError> {
        let questions = match kind {
            DecisionKind::Choice => serde_json::json!({
                "type": "choice",
                "instructions": "Select the best option.",
                "criteria": candidates
                    .iter()
                    .enumerate()
                    .map(|(i, c)| (format!("opt_{}", i), c.clone()))
                    .collect::<HashMap<_, _>>()
            }),
            // ...
        };

        let response = self.client
            .post(format!("{}/v1/systemone", self.base_url))
            .json(&serde_json::json!({
                "state": state,
                "questions": questions
            }))
            .send()?;

        let answer = response.json::<SystemOneResponse>()?;
        Ok(CalibratedDecision {
            winner: answer.answers[0].choice.clone(),
            confidence: answer.answers[0].confidence,
            probabilities: answer.answers[0].probabilities.clone(),
        })
    }
}
```

### Serving configuration

```yaml
# opencodifier.yaml
rung:
  name: strands-gemma4-e4b
  backend: strands_decider
  config:
    base_url: "http://localhost:8080"  # llama.cpp or 1Cat-vLLM server
    model: "strands-decider-E4B-gemma4-v1-2610"
    device: v100      # for 1Cat-vLLM
    quantization: q4_k_m

cache:
  # Strands uses model + calibration version in cache keys
  # Mirror this: include adapter SHA in cache key
  key_include:
    - model_sha256
    - calibration_version
    - prompt_template_version
```

---

## Benchmarking Plan

### Baseline (before optimization)

```bash
# Strands Decider 2B via ONNX on V100 CPU fallback
python -c "
import time, numpy as np
from strands_decider import StrandsDecider
model = StrandsDecider('StrandsAgents/strands-decider-2B-qwen3.5-v1-2610')

# Warm up
for _ in range(5):
    model.ask(state='test', choice='?=a,b,c')

# Benchmark
times = []
for _ in range(100):
    t0 = time.time()
    model.ask(state='Help! My payouts have been failing for 3 days!',
              choice='team?=billing,sales,retail')
    times.append(time.time() - t0)

print(f'Median: {np.median(times)*1000:.1f}ms')
print(f'P95: {np.percentile(times, 95)*1000:.1f}ms')
"
```

### After quantization

```bash
# llama.cpp benchmark script
./build/bin/llama-bench \
  -m gemma4-decider-q4_k_m.gguf \
  -ngl 99 \
  -c 4096 \
  -t 4

# Expected: sub-200ms per decision on V100 (vs 139ms on H100)
# V100 is ~2× slower than H100 for transformer attention
```

### Quality gate (adopted from czl CLM-GGUF methodology)

- Score quantized against bf16 reference on JevBench-231
- Pass line: top-1 ≥ bf16 noise floor **and** top-1 on decisive
  decisions (reference lead > 1 nat) ≥ 0.995
- End-task accuracy delta within 3.0 point budget
- Variants holding decisive decisions but losing end-task accuracy →
  "usable", not recommended

---

## Timeline (PSU-dependent)

| Phase | Action | Blockers |
|---|---|---|
| 0 | PSU arrives, baseline measurement | **PSU pending** |
| 1 | 1Cat-vLLM build from source on T5500 | PSU, CUDA 12.x |
| 2 | Benchmark Strands 2B ONNX on V100 | None |
| 3 | Community GGUF (v19) → llama.cpp on V100 | None (available now) |
| 4 | GGUF export of Gemma 4 v1 models | strands-decider export tool |
| 5 | llama.cpp quantize + serve Gemma 4 E4B Q4_K_M | GGUF export |
| 6 | 1Cat-vLLM compressed-tensors serving | 1Cat-vLLM build |
| 7 | TurboQuant v3 integration (monitor PR #779) | PR merge |
| 8 | GSQ quantization of E4B (borrow H100 time) | H100 compute |
| 9 | Quality gates on JevBench-231 | Benchmark harness |

---

## Decision Record

**Q: Should we use Strands Decider Gemma 4 E4B or Qwen3.5 2B as the primary
model rung on T5500?**

**A: E4B for quality, 2B as warm-cache fallback.**

- E4B Q4_K_M (~4.6GB) fits one V100 with KV cache headroom → TP1
- 2B Q4_K_M (~1.2GB) fits CPU or as a second-instance warm-cache
- E4B Decision Index 39.42 vs 2B 30.82 (28% better capability)
- Both are Apache-2.0 (base models) → can redistribute

**Q: Should we invest in GSQ quantization or use llama.cpp K-Quants?**

**A: Start with llama.cpp K-Quants; GSQ when H100 compute is available.**

- llama.cpp K-Quants are immediately available and well-tested
- GSQ requires H100/Hopper for quantization (not available on T5500)
- RCO layer (per-layer precision allocation) requires additional compute
- Monitor 1Cat-vLLM PR #779 (TurboQuant integration) as the preferred path

**Q: Should we wait for the community GGUF export of Gemma 4 models or
build it ourselves?**

**A: Build it ourselves. It's a blocker that needs resolving regardless.**

The GGUF export requires:
1. `hf_export` tool from strands-decider package
2. LoRA adapter merge with Gemma 4 base
3. llama.cpp quantize to Q4_K_M
4. Quality validation against JevBench-231

This is a prerequisite for any V100 serving path for the Gemma 4 models.

---

## References

- GSQ: `arxiv.org/abs/2604.18556` — Gumbel-Softmax post-training quantization
- RCO: `arxiv.org/abs/2605.00649` — Budgeted assignment for mixed-precision
- 1Cat-vLLM: `github.com/1CatAI/1Cat-vLLM` — V100 optimization fork
- TurboQuant v3: `github.com/Kubenew/TurboQuant-v3` — INT4 + AWQ + protected channels
- Strands Decider: `github.com/strands-labs/strands-decider` — Full research history
- Gemma 4 GGUF: `huggingface.co/unsloth/gemma-4-E4B-it-GGUF` — File sizes reference
- Strands GGUF: `huggingface.co/fabricant451/strands-decider-2B-hobson-v19-GGUF` — Available now
