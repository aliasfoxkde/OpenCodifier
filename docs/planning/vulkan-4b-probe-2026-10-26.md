# Vulkan serving path for the 4B decision model — probe record (2026-10-26, task #4)

**Question.** Can the fork's `llama-server` (Vulkan build,
`/nas/Temp/work/oc-model-eval/build-vulkan`, `GGML_VULKAN=ON`,
`libggml-vulkan.so` linked) serve `Qwen3.8-4B-Distill-Q4_K_M` on the
host iGPU at decision latency better than the CPU path?

**Setup.** Host: 16-core, RADV RENOIR iGPU, Vulkan 1.4.305 / Mesa
25.0.7. iGPU heap ~5.3 GiB; model 2.8 GB → fits fully offloaded
(`-ngl 99`). Server booted `-c 8192` (default ctx tried to allocate a
376k unified KV — on a 5 GiB heap that is a non-starter; any Vulkan
deployment must bound context). Load time 1m39s incl. first-boot
shader compile. GPU busy sampled 25–76% during load → the GPU path
was genuinely exercised.

**Numbers.** Single decision (prefill+8-token readout, sequential ×8):

| path | p50 | min | max |
|---|---|---|---|
| CPU (-ngl 0, 8 threads) | **0.73 s** | 0.71 s | 1.80 s |
| Vulkan iGPU (-ngl 99) | 1.80 s | 1.52 s | 2.37 s |

20-way concurrent burst on Vulkan: p50 29.7 s / p90 45.8 s for
80 sequences — no batch amortization on this device class.

**Verdict: NO-GO for the Vulkan serving path.** The iGPU is 2.5×
slower than the same host's CPU at 4B: shared LPDDR5 bandwidth means
the GPU buys no memory-bandwidth advantage, and RADV kernel/queue
overhead dominates at this model size. The CPU rung stays the
edge-compute lane; the Vulkan build remains useful only as a
fallback if a discrete-Vulkan device ever appears in the fleet
(none currently does — the only dGPUs are the T5500 V100s, which are
CUDA).

**Traps recorded for any re-probe:**
1. Fork default context (376k) must be overridden (`-c`) on small-VRAM
   devices; boot looks hung while it allocates.
2. `pkill -f` from a session shell can take the launching shell with
   it (exit 144); launch probes with `setsid nohup … < /dev/null`.
3. Sub-ms curl times = dead server (connection refused), never trust
   a latency table without a health check first.
