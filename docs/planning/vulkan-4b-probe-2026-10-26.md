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

**Verdict: NO-GO for the iGPU; OPEN for discrete Vulkan hardware.** The iGPU is 2.5×
slower than the same host's CPU at 4B: shared LPDDR5 bandwidth means
the GPU buys no memory-bandwidth advantage, and RADV kernel/queue
overhead dominates at this model size. This is an **iGPU-structural**
result — shared memory kills the bandwidth edge that is the whole
point of GPU decode at this scale. It is **not** a verdict on Vulkan
as a backend class: a discrete Vulkan device (AMD dGPU, Intel Arc)
has dedicated VRAM bandwidth and has never been probed, because none
exists in the fleet today (the only dGPUs are the T5500 V100s, which
are CUDA). Re-probe is queued as a standing task with the trigger:
*any discrete Vulkan-capable device enters the fleet* — concretely the
planned **AMD V620 32 GB** (Vega 20, dedicated HBM2) for the T5500 box,
or the same card on the Fedora machine via a riser. The CPU rung
remains the edge lane until then. The Vulkan build stays proven and
ready (`build-vulkan/`, deterministic scores, 1m39s load).

Caveat for that re-probe: the V620 is a mining card (Vega 20). Confirm
RADV enumerates it as a *compute* device (it may default to a display
or be blocked by the driver) before trusting a boot, and expect its
compute path to be narrower than a consumer dGPU's.

**Traps recorded for any re-probe:**
1. Fork default context (376k) must be overridden (`-c`) on small-VRAM
   devices; boot looks hung while it allocates.
2. `pkill -f` from a session shell can take the launching shell with
   it (exit 144); launch probes with `setsid nohup … < /dev/null`.
3. Sub-ms curl times = dead server (connection refused), never trust
   a latency table without a health check first.
