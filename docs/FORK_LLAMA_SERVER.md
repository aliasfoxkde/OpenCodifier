# Decision Inference Fork — llama.cpp

**Status:** Working. The fork builds, serves the decision endpoint, and
the engine's model rung consumes it end to end.

## Problem

OpenCodifier's model rung is served by `llama-server` from a
parallel-decision fork of llama.cpp. The fork has the tree-mode
`/v1/decision` endpoint with full per-choice distribution readout — the
only serving path that scored the decision weights honestly (F26:
0.8083 vs 0.217 for the tree-mode mismatch).

However, applying a small LoRA adapter (r=16, ~3–4 MB) to the serving
process causes significant throughput degradation. On AMD Strix Halo, a
35 MB LoRA dropped token throughput from 1500 → 300 t/s. Fused LoRA
kernels recovered 1400 t/s (issue #27093).

## Lineage

```
thecodacus/llama.cpp @ parallel-decision        8f02796   (upstream base)
  └─ aliasfoxkde/llama.cpp @ parallel-decision-oc 7ae36f4  (OpenCodifier patch)
       └─ woct0rdho/llama.cpp @ strix                        (optional, AMD RDNA 3.5
                                                              fused LoRA kernels)
```

`7ae36f4` is the commit the engine requires. It adds the full per-choice
`distribution` map next to the winner-only `probability` in the
`/v1/decision` readout. `LlamaDecisionClassifier` **refuses** a
winner-only response rather than synthesizing probability mass, so a
server without this commit is rejected by the client with
`response carries no 'distribution'` — by design, not by accident.

`build.sh` clones `aliasfoxkde/llama.cpp` @ `parallel-decision-oc` by
default. The upstream base is the fallback if the fork is unreachable.
The built binary's provenance (repo, branch, base commit, final commit,
backend, host) is written to `build/BUILD-INFO.txt`.

## What the build does — and does not — do

The build is a **stock llama.cpp build of the decision fork**. Nothing
is stripped.

| Not stripped | Why it is not |
|---|---|
| Generation code (KV cache, samplers, beam search) | The decision path reads the same kernels; removing them is a size edit across the whole tree, not a latency one. |
| Grammar-guided generation | Same. |
| Paged KV / 128K context | The server is started with `--ctx-size 2048`; the code is present but unused. |
| Quantization types other than Q4_K_M | Same. |
| Attention variants other than SDPA | Same. |
| MoE routing, speculative decoding | Same. |

Stripping is a permanent merge burden against every upstream rebase,
and the binary is already **18 MB** with the full feature set — not the
constraint. If stripping is ever adopted, the rule is: strip only what
a measured constraint requires, and record the measurement here first.

The build does set `LLAMA_BUILD_EXAMPLES=OFF` and `LLAMA_BUILD_TESTS=OFF`
— those are *build-target* switches, not feature removal.
`LLAMA_BUILD_TOOLS` must stay `ON`: `tools/server/` lives under
`tools/`, and gating tools off silently removes the `llama-server`
target entirely.

## Architecture

```
OpenCodifier engine (Rust)
  └─ HTTP POST /v1/decision
       └─ llama-server (this fork)
            ├─ GGUF base model (Qwen3.5-2B Q4_K_M)
            └─ GGUF LoRA adapter (E1-E/E1-F/E1-G r=16)
```

The Rust HTTP client (`LlamaDecisionClassifier`) does not change when
the server binary changes. The fork replaces only the server process.

## Build

```bash
cd forks/llama.cpp
./build.sh                 # CPU backend (default)
BACKEND=vulkan ./build.sh  # AMD/NVIDIA Vulkan
BACKEND=cuda ./build.sh    # NVIDIA CUDA
```

Requires: CMake ≥ 3.20, C++17. CUDA and Vulkan SDKs only for their
respective backends.

Three build traps, each found by an actual build:

1. The cmake source directory must be the clone, not `..` — the build
   directory is a *sibling* of the clone.
2. `LLAMA_BUILD_TOOLS` must stay `ON` (see above).
3. `GGML_STATIC=ON` and `BUILD_SHARED_LIBS` must agree. The default is
   `BUILD_SHARED_LIBS=ON`, which links a shared `ggml-base` against
   `crtbeginT.o` and fails with `relocation R_X86_64_32 against hidden
   symbol '__TMC_END__'`.

## Serve

```bash
./serve.sh --base /path/to/Qwen3.5-2B-Q4_K_M.gguf --lora /path/to/adapter.gguf
```

The server binds `127.0.0.1:8080` by default and is started with
`--decision-seqs 8` (minimum 3: cached prefix, trunk, one branch).
Without it the server starts but refuses `/v1/decision` with
`decisions are disabled`.

## Measured

CPU backend, AMD Ryzen 7 5700U, 16 cores, no GPU:

| Fact | Value |
|---|---|
| Binary | 18 MB, statically linked |
| Model | `Qwen3.5-2B-Q4_K_M.gguf` |
| `/v1/decision` round trip | 1.37 s cold (prefill 1.26 s, scoring 0.10 s) |
| Readout | full `distribution` map over all declared choices |

The round trip is dominated by prefill of the shared prefix; the
decision path's own scoring is ~0.10 s. The engine's model rung fires
only when a cheaper rung declines to decide, so this latency is paid
on the escalation tail only.

## Integration with OpenCodifier

```bash
opencodifier serve \
    --llama http://127.0.0.1:8080 \
    --llama-model-id 'pd-fork-7ae36f4|qwen3.5-2b-q4_k_m|tree-dist' \
    --ladder ladders/fusion-v1.json
```

The `--llama-model-id` decorates the cache key so decisions served by
this build re-key separately from any other build of the same weights.
