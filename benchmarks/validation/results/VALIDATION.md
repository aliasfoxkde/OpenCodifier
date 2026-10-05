# Real-world validation campaign — results of record

Class-by-class results for the real-world validation campaign
(`docs/VALIDATION.md` is the spec). Findings are numbered `RV-##` per
the findings-artifact contract (spec §9). Raw run JSONs live on the
compute host (`fedora:~/oc-model-eval/runs/…`) and are never committed;
every number here is traceable to one.

Manifest: repo commit at run time `8caf34f` (release build,
`target/release/opencodifier`); host fedora, 16 cores; serve on
`127.0.0.1:8092` with default policy, no ladder, no model rung. Host
load was hostile for every window tried (sibling agent sessions hold
the 16-core floor at ~17–21 and spike to 30+; gate policy v2 =
proceed-dirty with load recorded per run). Latency gates carry 5–9×
headroom over budget, so load contamination does not flip any verdict
recorded here.

## Class B — burst (levels 1–5), run `burst-r2` (2026-10-05)

Five-level traffic matrix against `/v1/decide`: 8/32/128 concurrent
clients × 1,024 unique bodies (level-4: 8 clients, 128 bodies × 2 cache
passes; level-5: 128 clients, 1,024 unique + 51 malformed over 8 F2
shapes). Uniqueness by zero-padded counter in a marker sentence asserted
grammar-inert and vocabulary-disjoint per the spec. Load 29–31 for the
whole run (1-min avg), recorded per level.

| level | clients | requests | throughput | p50 | p99 | valid-leg success | peak RSS |
|---|---:|---:|---:|---:|---:|---:|---:|
| 1 | 8 | 1,024 | 1,022 rps | 1.38 ms | 2.69 ms | 1,024/1,024 | 31.2 MiB |
| 2 | 32 | 1,024 | 968 rps | 4.91 ms | 15.60 ms | 1,024/1,024 | 31.2 MiB |
| 3 | 128 | 1,024 | 826 rps | 8.13 ms | 27.00 ms | 1,024/1,024 | 31.4 MiB |
| 4 | 8 | 256 (128×2) | 255 rps | 1.21 ms | 2.66 ms | 256/256 | 31.5 MiB |
| 5 | 128 | 1,024 + 51 malformed | 913 rps | 5.70 ms | 22.73 ms | 1,024/1,024 | 34.7 MiB |

healthz sampled at 10 Hz through every level: worst p99 14.71 ms
(level 3, budget ≤ 50 ms).

### Gates (spec §4.4)

| gate | budget | observed | verdict |
|---|---|---|---|
| op-success (valid legs) | 100 % every level | 100 % at all five levels; zero 5xx, zero timeouts, zero resets | pass |
| latency p99 | ≤ 25 / 75 / 250 ms @ 8 / 32 / 128 | 2.69 / 15.60 / 27.00 ms | pass (9× / 5× / 9× headroom, under load 29–31) |
| liveness | healthz p99 ≤ 50 ms @ L3 | 14.71 ms | pass |
| memory | L3 peak RSS ≤ 2× L1 | ratio 1.01 (31.4 vs 31.2 MiB) | pass |
| error discipline | every malformed body → 4xx + stable code, 0 × 5xx | 51/51 rejected: 44 × 400, 7 × 413 (>1 MiB); zero 5xx, zero panics | pass |
| no collateral damage | post-burst control decision identical | identical at all five levels (`answers`/`outcome`/`confidence`) | pass |
| absence recorded | — | no 429/503/backpressure exists in the response vocabulary at any level → RV-005 | recorded |

### Findings opened by class B

- **RV-001 (runner defect, fixed+re-verified)** — the burst runner
  recorded all-zero server resources when handed a dead `--server-pid`
  (burst-r1 passed 3743795; the live serve was 3069208): the tree walker
  unconditionally includes the root, every `/proc` read failed, and the
  zeros landed in the JSON as if they were measurements. The memory gate
  was *unmeasured* in burst-r1, not passing. Fix: refuse to start the
  monitor without a live `/proc/<pid>` entry and say so in the log;
  re-verified by burst-r2 recording 31.2–34.7 MiB.
- **RV-002 (spec reclassification, waived:accepted-by-design)** — two of
  the ten planned F2 shapes are not malformed on this surface: omitting
  `state.facts` returns 200 (`State.facts` is `#[serde(default)]`;
  omitted means "no typed facts" and the engine extracts from text) and
  duplicate JSON keys return 200 (serde last-key-wins). Reclassified out
  of the malformed set; spec §6 amended with the rationale.
- **RV-003 (spec amendment, waived:unsatisfiable-as-stated)** — the
  post-burst poison check cannot be raw byte-identity on any server with
  the exact-decision cache: the second identical send legitimately
  returns `cache_hit: true` and a rewritten trace. Amended to
  decision-core identity (passes everywhere) with raw byte-identity kept
  as information; spec §4.4 amended.
- **RV-004 (cache behavior, note)** — the exact-decision cache is a
  1,024-entry LRU (`CacheConfig::default`, ttl 300 s). Level 1/2/3/5
  flood exactly ≥ 1,024 unique bodies, evicting the pre-burst control
  entry, so the post-burst control is a miss; level 4's 128-body pool
  keeps it and the control replays verbatim (raw bytes identical). Not a
  defect — bounded caches evict — but any consumer that assumes
  "repeat ⇒ cache_hit" across a large unique-body flood will be wrong,
  and this campaign's fixtures now record the boundary.
- **RV-005 (admission control, open)** — zero 429/503/backpressure
  signals at any level (predicted as §9.3 row 2, now measured): every
  request is a `spawn_blocking` task with no bound. Throughput plateaus
  rather than scales (1,022 → 968 → 826 rps across 8 → 32 → 128 clients,
  a mild decline, under load 29–31) while latency budgets hold with 5–9×
  headroom; at ~1 ms server-side per request the tier saturates its
  pipeline near 1k rps and extra clients only queue. This is the
  evidence base §10.3 (admission-control decision) wanted; the verdict
  stays open there.
- **RV-006 (flaky client reset under load, note)** — burst-r1 saw 1 of 5
  `nested_5000` legs end in a client-side `BrokenPipeError` (server
  closed mid-reject under load 24); burst-r2 saw 6/6 clean 400s for the
  same shape. Not reproducible as a server defect; recorded for
  completeness.

Class B verdict: **all §4.4 gates pass**; the four note-grade findings
above are recorded, one runner defect fixed and re-verified.
