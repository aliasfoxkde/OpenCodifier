# Decision Inference Fork — llama.cpp

**Status:** Experimental. Branch-only tracking. Promotes to a real fork if it graduates.

## Problem

OpenCodifier's model rung is served by `llama-server` from a parallel-decision fork
of llama.cpp. The fork has the tree-mode `/v1/decision` endpoint with full per-choice
distribution readout — the only serving path that scored the decision weights honestly
(F26: 0.8083 vs 0.217 for the tree-mode mismatch).

However, applying a small LoRA adapter (r=16, ~3–4 MB) to the serving process causes
significant throughput degradation. On AMD Strix Halo, a 35 MB LoRA dropped token
throughput from 1500 → 300 t/s. Fused LoRA kernels recovered 1400 t/s (issue #27093).

## What this fork tracks

```
Base: OpenCodifier parallel-decision fork (hash ad129b0+)
  └─ woct0rdho/strix-llama.cpp (AMD Strix Halo fast LoRA kernels)
```

The parallel-decision fork is the base because it has the verdict-slot readout
and tree-mode decision endpoint. The woct0rdho work adds the fused `mat_mul_id`
LoRA kernels that recover throughput.

## What was stripped

For decision-only serving, the following are **not built**:

| Stripped | Reason |
|---|---|
| Generation code (KV cache, samplers, beam search) | We never generate text |
| Grammar-guided generation | Not applicable to decision classification |
| Paged KV / 128K context | We cap at 2048 tokens |
| All quantization types except Q4_K_M | Our adapters are Q4_K_M |
| All attention variants except SDPA | Qwen3.5-2B uses SDPA only |
| MoE expert routing | Not applicable to dense 2B model |
| Speculative decoding | Not applicable |

## What was kept

- Transformer forward pass
- Fused LoRA application (the fast kernel work)
- Tree-mode decision readout endpoint (`/v1/decision`)
- Static binary output

## Architecture

```
OpenCodifier engine (Rust)
  └─ HTTP POST /v1/decision
       └─ llama-server (this fork)
            ├─ GGUF base model (Qwen3.5-2B Q4_K_M)
            └─ GGUF LoRA adapter (E1-E/E1-F/E1-G r=16)
```

The Rust HTTP client (`LlamaDecisionClassifier`) does not change when the server
binary changes. The fork replaces only the server process.

## Build

```bash
cd forks/llama.cpp
./build.sh
```

Requires: CMake >= 3.20, C++17, Vulkan SDK (AMD) or CUDA SDK (NVIDIA).

## Serve

```bash
./serve.sh --base Qwen3.5-2B-Q4_K_M.gguf --lora e1e-adapter-Q4_K_M.gguf
```

Server listens on `127.0.0.1:8080` by default.

## Integration with OpenCodifier

```bash
opencodifier serve \
    --llama http://127.0.0.1:8080 \
    --llama-model-id pd-fork-fastlora \
    --graph ladders/fusion-v2.json
```

The `--llama-model-id` decorates the cache key so decisions served by the fork
re-key separately from the baseline fork.
