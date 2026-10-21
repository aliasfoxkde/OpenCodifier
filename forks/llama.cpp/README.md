# llama.cpp Decision Inference Fork

Branch-only tracking of llama.cpp variants optimized for OpenCodifier's
decision model serving. This is experimental — promote to a real fork if it
graduates.

## Tracking strategy

```
upstream/llama.cpp          — ggml-org/llama.cpp mainline
parallel-decision/          — OpenCodifier's parallel-decision fork (verdict-slot readout)
  woct0rdho/strix-llama   — AMD Strix Halo fast LoRA kernels (fused mat_mul_id)
```

Base: OpenCodifier's parallel-decision fork (hash ad129b0+), which has the
native tree-mode `/v1/decision` endpoint with full per-choice distribution readout.

Patches on top:
1. woct0rdho's fused LoRA kernels (recovers ~80% of throughput loss from small adapters)

## What was stripped

For decision-only serving, the following are **not built**:

- Generation code (KV cache management, samplers, beam search, grammar-guided generation)
- Long context support (paged KV, 128K contexts)
- All quantization types except Q4_K_M (our adapters are Q4_K_M GGUF)
- All attention variants except SDPA (Qwen3.5-2B uses SDPA only)
- MoE expert routing (not applicable to dense 2B model)

## What was kept

- Transformer forward pass
- Fused LoRA application (fast kernel work)
- Tree-mode decision readout endpoint (`/v1/decision`)
- Static binary output

## Build requirements

- CMake >= 3.20
- C++17 compiler
- Vulkan SDK (for AMD Vulkan builds)
- CUDA SDK (for NVIDIA GPU builds)

## Quick start

```bash
# Build the fork
./build.sh

# Start the server
./serve.sh --base Qwen3.5-2B-Q4_K_M.gguf --lora e1e-adapter-Q4_K_M.gguf

# Server runs on 127.0.0.1:8080, serves /v1/decision
```

## Remote setup

```bash
cd forks/llama.cpp
git remote add upstream https://github.com/ggml-org/llama.cpp
git remote add parallel-decision https://github.com/opencodifier/llama.cpp  # when promoted
git remote add woct0rdho https://github.com/woct0rdho/llama.cpp
git fetch --all
```

## OpenCodifier integration

The `LlamaDecisionClassifier` in `crates/opencodifier-model/src/llamacpp.rs`
is an HTTP client — it talks to this server over `POST /v1/decision`:

```bash
opencodifier serve --llama http://127.0.0.1:8080 --llama-model-id pd-fork-ad129b0
```

The Rust client does not change when the server binary changes.
