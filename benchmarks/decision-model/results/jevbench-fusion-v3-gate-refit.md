# Fusion-gate OOD refit — engine + E2B-QAT rung (post-hoc, task #114)

Offline over measured per-item rows: engine trace `engine-only-fedora-v1` (gate features, shipped build), rung trace `proofs-only-fedora-v1` (E2B-QAT counterfactual). Nothing re-inferred; abstains gate as never-accept.

Joined 231 items. The proofs posture's own engine-kept items (8) carry engine answers in the rung trace; they sit at prob = 1.0 and stay engine-answered under every swept threshold, so they never enter the sweep as rung outcomes (checked: True).

## Anchors (simulated vs live-measured)

| posture | t | sim acc | live acc | routing | mean ms | p95 |
|---|---|---|---|---|---|---|
| as-shipped fusion-v2 | 0.56 | 0.5498 | 0.5584 | engine 69%, llm 31% | 254 | 613 |
| proofs-only | 1.00 | 0.6883 | 0.6883 | engine 3%, llm 97% | 2694 | 13954 |

## Split-half refit

Fitted on 116 items (even indices), held out 115 (odd). Plateau-midpoint threshold on the fit half: **t* = 1.00**.

| view | t | acc | ECE | routing | mean ms | p50 | p95 |
|---|---|---|---|---|---|---|---|
| fit-half @ t* | 1.00 | 0.6897 | 0.162 | — | — | — | — |
| **hold-half @ t*** | 1.00 | **0.6870** | 0.180 | — | — | — | — |
| full set @ t* | 1.00 | 0.6883 | 0.171 | engine 3%, llm 97% | 2694 | 577 | 13954 |
| full set in-sample max | 1.00 | 0.6883 | 0.171 | engine 3%, llm 97% | 2694 | 577 | 13954 |

**Boundary finding:** the fit half's accuracy plateau reaches the grid edge — the OOD-optimal gate on this distribution IS the proofs posture (accept exact proofs, escalate everything else). No intermediate winner-prob threshold recovers the fusion latency advantage without paying accuracy: the engine's confidence is not OOD-informative (its accepted items convert below the rung's rate at every threshold, every kind, every band). The lever this closes: threshold refit. The levers it opens: a better gate *feature* (rule/extractor provenance, not softmax confidence) or an OOD-robust engine.

## Threshold curve (full set)

| t | acc |
|---|---|
| 0.30 | 0.5238 |
| 0.31 | 0.5238 |
| 0.32 | 0.5238 |
| 0.33 | 0.5238 |
| 0.34 | 0.5238 |
| 0.35 | 0.5238 |
| 0.36 | 0.5238 |
| 0.37 | 0.5238 |
| 0.38 | 0.5238 |
| 0.39 | 0.5238 |
| 0.40 | 0.5238 |
| 0.41 | 0.5238 |
| 0.42 | 0.5238 |
| 0.43 | 0.5238 |
| 0.44 | 0.5238 |
| 0.45 | 0.5238 |
| 0.46 | 0.5238 |
| 0.47 | 0.5238 |
| 0.48 | 0.5238 |
| 0.49 | 0.5238 |
| 0.50 | 0.5238 |
| 0.51 | 0.5455 |
| 0.52 | 0.5455 |
| 0.53 | 0.5455 |
| 0.54 | 0.5455 |
| 0.55 | 0.5498 |
| 0.56 | 0.5498 |
| 0.57 | 0.5498 |
| 0.58 | 0.5541 |
| 0.59 | 0.5541 |
| 0.60 | 0.5541 |
| 0.61 | 0.5541 |
| 0.62 | 0.5584 |
| 0.63 | 0.5584 |
| 0.64 | 0.5584 |
| 0.65 | 0.5887 |
| 0.66 | 0.5931 |
| 0.67 | 0.5931 |
| 0.68 | 0.5931 |
| 0.69 | 0.5931 |
| 0.70 | 0.5974 |
| 0.71 | 0.6104 |
| 0.72 | 0.6104 |
| 0.73 | 0.6104 |
| 0.74 | 0.6104 |
| 0.75 | 0.6104 |
| 0.76 | 0.6190 |
| 0.77 | 0.6190 |
| 0.78 | 0.6190 |
| 0.79 | 0.6147 |
| 0.80 | 0.6147 |
| 0.81 | 0.6277 |
| 0.82 | 0.6234 |
| 0.83 | 0.6234 |
| 0.84 | 0.6234 |
| 0.85 | 0.6234 |
| 0.86 | 0.6234 |
| 0.87 | 0.6234 |
| 0.88 | 0.6234 |
| 0.89 | 0.6234 |
| 0.90 | 0.6234 |
| 0.91 | 0.6320 |
| 0.92 | 0.6320 |
| 0.93 | 0.6364 |
| 0.94 | 0.6407 |
| 0.95 | 0.6364 |
| 0.96 | 0.6364 |
| 0.97 | 0.6450 |
| 0.98 | 0.6580 |
| 0.99 | 0.6494 |
| 1.00 | 0.6883 |

## Per-kind pockets (engine confidence bands)

Rung accuracy = the rung counterfactual on the same items (ok rows only). No pocket means the per-kind gate registry has nothing to register for this distribution.

| kind | n | engine acc | rung acc | bands (engine acc) |
|---|---|---|---|---|
| choice | 139 | 0.353 | 0.695 | [0.50,0.60) 0.77 (n=26); [0.60,0.70) 0.33 (n=9); [0.70,0.80) 0.57 (n=14); [0.80,0.90) 0.57 (n=7); [0.90,0.99) 0.33 (n=18); [0.99,1.01) 0.24 (n=34) |
| noul | 74 | 0.500 | 0.784 | [0.50,0.60) 0.59 (n=17); [0.60,0.70) 0.12 (n=8); [0.70,0.80) 0.53 (n=15); [0.80,0.90) 0.33 (n=3); [0.90,0.99) 0.62 (n=13); [0.99,1.01) 0.50 (n=18) |
| score | 18 | 0.056 | 0.706 | [0.50,0.60) 0.00 (n=1); [0.70,0.80) 0.00 (n=1); [0.90,0.99) 0.00 (n=2); [0.99,1.01) 1.00 (n=1) |

Oracle (either arm correct): **0.7446** — the routing ceiling on the joined set; the proofs posture extracts most of it because the rung, not the gate, is the asset OOD.


Disclosure: every number on this page is fitted and/or evaluated on the 231 public items (Von precedent) — in-sample for the full-set rows, half-held-out for the split-half rows. The sealed half is untestable locally by construction.

