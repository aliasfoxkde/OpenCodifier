# llama.cpp Decision Inference Fork

The decision-serving lane: a llama.cpp tree that answers **finite-schema
decisions** in one batched forward pass instead of generating prose.

This is a *tracking* fork, not a rewrite. The upstream decision engine
(`tools/parallel-decision/`) does the work; OpenCodifier's job is to
build it correctly, pin it, and prove the contract.

## Lineage

| Layer | Where | Commit |
|---|---|---|
| Upstream base | `thecodacus/llama.cpp` @ `parallel-decision` | `8f02796` |
| OpenCodifier patch | `aliasfoxkde/llama.cpp` @ `parallel-decision-oc` | `7ae36f4` |

`7ae36f4` adds the full per-choice `distribution` map next to the
winner-only `probability` in the `/v1/decision` readout. The Rust client
(`LlamaDecisionClassifier`) **refuses** a winner-only response rather
than synthesizing mass, so this commit is the difference between a
server the engine can use and one it rejects.

`build.sh` clones the published fork by default and falls back to the
upstream base only if the fork is unreachable. The fork's branch tip is
recorded in `build/BUILD-INFO.txt` with the backend and the build host,
so a shipped binary is traceable to a commit.

Optional third layer: `woct0rdho/llama.cpp` @ `strix` (AMD RDNA 3.5
fused LoRA kernels, recovering ~80% of the throughput an unfused
`mat_mul_id` costs on small adapters — see `docs/RESEARCH.md` §28).
`build.sh` merges it when the branch is fetchable and skips it
otherwise. The NVIDIA CUDA port of those kernels does not exist yet.

## What the build actually does

The build is a **stock llama.cpp build of the decision fork**. It does
*not* strip features — no generation code, quantization type, or
attention variant is removed. Concretely, the build passes:

```
-DCMAKE_BUILD_TYPE=Release
-DLLAMA_BUILD_SERVER=ON      -DLLAMA_BUILD_EXAMPLES=OFF
-DLLAMA_BUILD_TESTS=OFF       -DLLAMA_BUILD_TOOLS=ON
-DGGML_NATIVE=ON              -DBUILD_SHARED_LIBS=OFF
-DGGML_STATIC=ON
```

plus `-DGGML_CUDA=ON` or `-DGGML_VULKAN=ON` when `BACKEND` says so.

Why not strip? Stripping is a per-ggml-quantization-type and
per-attention-variant edit across the whole tree, and it is a *size*
optimization, not a latency one — the decision path reads the same
kernels the generation path does. The cost of stripping is a permanent
merge burden against every upstream rebase; the benefit is a smaller
binary. The binary is already 18 MB with the full feature set, which is
not the constraint. Stripping stays unbuilt until someone measures a
real constraint it removes.

`LLAMA_BUILD_TOOLS` must stay `ON`: `tools/server/` lives under
`tools/`, and gating tools off silently removes the `llama-server`
target entirely.

## Measured

CPU backend, AMD Ryzen 7 5700U, 16 cores, no GPU:

| Fact | Value |
|---|---|
| Binary | `build/bin/llama-server`, 18 MB, statically linked |
| Model | `Qwen3.5-2B-Q4_K_M.gguf` |
| `/v1/decision` round trip | 1.37 s cold (prefill 1.26 s, scoring 0.10 s) |
| Readout | full `distribution` map, all declared choices |

The 1.37 s is dominated by prefill of the shared prefix. The decision
path's own scoring is ~0.10 s.

## Build

```bash
./build.sh                # clone + configure + build llama-server
BACKEND=vulkan ./build.sh # AMD/NVIDIA Vulkan backend
./build.sh clean          # remove the clone and the build tree
```

Output:

- `build/bin/llama-server`
- `build/BUILD-INFO.txt` — binary path, repo, branch, base commit, final
  commit, backend, build time, host

Requirements: CMake ≥ 3.20, a C++17 compiler. CUDA and Vulkan SDKs only
for their respective backends.

## Serve

```bash
./serve.sh --base /path/to/Qwen3.5-2B-Q4_K_M.gguf
```

The server binds `127.0.0.1:8080` by default. It must be started with
`--decision-seqs N` (N ≥ 3) or `/v1/decision` is refused with
`decisions are disabled`; `serve.sh` sets this.

## OpenCodifier integration

`LlamaDecisionClassifier` (`crates/opencodifier-model/src/llamacpp.rs`)
is an HTTP client. It posts to `/v1/decision` and reads
`results[0].fields.choice.distribution`.

```bash
opencodifier serve \
    --llama http://127.0.0.1:8080 \
    --llama-model-id 'pd-fork-7ae36f4|qwen3.5-2b-q4_k_m|tree-dist'
```

The Rust client does not change when the server binary changes. The
model id is the cache-key discriminator for the served weights: changing
it re-keys every cached decision, exactly like a model swap.
