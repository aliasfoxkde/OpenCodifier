# Benchmarks — OpenCodifier vs the decision-model field

The complete measured comparison in one place: every OpenCodifier system
and model arm we have run, and every external decision-model row with
published numbers, on the axes where they actually compare. Written to
stand alone — this page is the source for the repository table and any
future website.

**Record date: 2026-09-29.** Numbers are frozen to the artifacts pinned
by `benchmarks/decision-model/results/models.manifest.json` (SHA-256 per
model file); method documents are
[`benchmarks/decision-model/README.md`](../benchmarks/decision-model/README.md)
(internal suite) and
[`benchmarks/decision-model/JEVBENCH.md`](../benchmarks/decision-model/JEVBENCH.md)
(JevBench). Raw run evidence lives outside the repo and is regenerable
from the pinned inputs.

## The system being measured

OpenCodifier is a local-first, deterministic-first **decision runtime**:
unstructured state in, typed decisions out — `Choice` (one candidate
from a runtime-defined list), `Boolean` (with probability), `Score`
(ordered levels with the full distribution and expected value). The
escalation ladder spends the cheapest mechanism that can decide with
sufficient confidence: exact rule → cached decision → metadata filter →
lexical match → embedding similarity → small classifier →
candidate-conditioned decision model → verifier → external model.
Confidence is calibrated before it is exposed, abstention is a
successful outcome, and every answer is bit-reproducible.

## How to read the boards

Two boards, two axes, never mixed:

- **Board A — JevBench public split (231 items).** The one benchmark
  with published rows for the external Jev-class field *and* runnable by
  us. We run it with the authors' own harness package (their runner,
  their scoring, zero drift by construction).
- **Board B — our locked 120-item suite.** Three classes the field's
  suite does not isolate (constraint checking, paraphrase routing,
  relational composition), byte-locked, used to tier models for the
  runtime. Internal context, not comparable to Board A.

A machine-readable copy of every table below (plus the fork params A/B
from `results/REPORT.md`) lives at
`benchmarks/decision-model/results/board.csv` — regenerate it with
`python3 benchmarks/decision-model/runner/board_csv.py` after editing
any board table, so the CSV never drifts from this page.

## Board A — JevBench public split (shared axis)

Method of record: `JEVBENCH.md` — official harness
(`fstandhartinger/jevbench` @ `9ec6f15a`, MIT), public split 231 items
(choice 139 / noul 74 / score 18, mean chance 0.3176), serial no-retry,
zero synthesized probabilities, full replay determinism.

### Our runs (authors' harness, same split)

| system | n | accuracy | family-macro | ECE | Brier | p50 (client wall) | determinism |
|---|---:|---:|---:|---:|---:|---:|---|
| OpenCodifier engine (relational solver over lexical, `/v1/decide`) | 231 | 0.3766 | 0.4011 | 0.402 | 0.890 | **2.0 ms** | 231/231 |
| **vtx: VTX-JEV-3** (20.5 MB 2-bit static table, vendor client) | 231 | 0.4113 | 0.4318 | 0.126 | 0.684 | **7.9 ms** | 231/231 |
| Jev-Style-0.8B-Decision-v3 Q4_K_M (native verdict-slot readout — comparability bridge) | 231 | 0.6494 | 0.6378 | **0.080** | **0.425** | 6.72 s | 231/231 (labels + probabilities) |
| **fork: Qwen3.5-4B UD-Q4_K_XL tree mode** (D16 tier, fork-default T — base model, no decision tune; **fedora anchor**, 2026-10-02, default instructions; replicate bit-identical ²) | 231 | **0.7662** | **0.7571** | 0.070 ¹ | 0.305 ¹ | 2.28 s | 231/231 (labels + d15 distributions ¹) |

¹ ECE/Brier come from the decision-identical build-`d15` arm
(`fork_4b-d15-v1`, same host/build/split, predictions equal 231/231):
the anchor's own tree mode ships the winner's mass only (label-only
mapping, D15), but the `d15` rebuild emits full per-choice
distributions, and its raw-board ECE 0.070 / Brier 0.305 are the
fork's calibration of record. Exact d15 refit on those distributions:
T 1.313, ECE 0.070 → 0.048 in-sample
(`calibration/fork_4b-d15-v1.json` in the results tree). The anchor's
latency is a clean-host client wall (p95 45.9 s carries
sibling-session storm load, not arm time).
² The replicate (`fork_4b-anchor-rep1-v1`, fresh session, same build /
instructions / split) matched the anchor on all 231 `predicted` and
`raw_sha256` values (p50 1.74 s) — see the superseded-row paragraph.

**Superseded row, kept for the record**: the same arm's first run
(2026-09-30, this NAS box) scored 0.6667 / macro 0.6541 at p50 15.53 s
through a co-tenant storm. The 23-task delta to the anchor exceeds the
strands retrain-noise bar below (σ ≈ 3.2 tasks), so it is recorded as
an **unresolved run-vintage discrepancy**, not explained away: the fork
reads its decision sampling temperature from its own `/v1/decision`
default (no temperature in the runner payload, no seed in either
manifest), each row is a single draw, and the NAS row additionally ran
through the storm. The fedora anchor — same fork binary of record
(`ad129b0`), same harness commit, same split bytes
(`dc3995d8…`), in-run replay 231/231 — is the fork number of record.

An anchor **replicate** (`fork_4b-anchor-rep1-v1`, 2026-10-02, fresh
server session, same build / default instructions / split) reproduced
the anchor **bit-identically**: 231/231 rows match on both `predicted`
and `raw_sha256`, in-run replay 231/231 (`predictions_match: true`),
acc/macro identical at 0.7662 / 0.7571 (p50 1.74 s on the quieter
host). Single-draw sampling variance at this configuration is
therefore measured at **zero** on the anchor host. This does not
explain the NAS delta — it rules out one candidate explanation
(sampling) and narrows the discrepancy to run vintage/configuration
(what the NAS-era session actually ran). A seeded or fork-greedy
cross-check of that NAS-era configuration remains the only remaining
way to settle it.

### Fork instruction-template A/B (#76, 2026-10-02 — anchor host, build `ad129b0`)

Four instruction families over the identical tree readout, single draw
each, n = 231:

| instructions | accuracy | family-macro | p50 |
|---|---:|---:|---:|
| `Select the correct option.` (default = the anchor) | 0.7662 | 0.7571 | 2.28 s |
| `Decide.` | 0.7662 | 0.7587 | 3.23 s |
| `Apply the stated rules exactly, then select the single correct option.` | **0.7749** | **0.7674** | 2.23 s |
| `@task` | 0.7576 | 0.7520 | 2.16 s |
| build-`d15` + default instructions (#61) | 0.7662 | 0.7571 | 1.98 s |

Findings, stated at their honest strength: (1) the instruction family
is **not an established lever** — the whole spread is 4 of 231 tasks,
inside the single-draw noise the strands caveat warns about; the rules
phrasing is nominally best and is *not* adopted. (2) The `d15` build is
**decision-identical** to the anchor build — the D15 full-distribution
patch changes reporting only, so #61's calibration refit is purely
distribution-fidelity work with no accuracy claim attached. (3) The
template question is closed for this fork at this n; reopen only with a
seeded multi-draw design.

The bridge row is the validity anchor: its authors self-ran exactly
these weights on exactly these items with the official harness and
published 64.1 % (148/231) — our 64.94 % (150/231) is a +0.9 pp
reproduction on their own code, so every row above transfers to the
published context.

### Published rows — autotrust runs (2026-09-26, family-macro)

Source: the [autotrust/JEV-27B announcement](https://huggingface.co/blog/autotrust/autotrustjev-27b-fast-calibrated-decisions-and-ful)
(published 2026-09-27), which states JevBench here is the public
231-example set with its family-macro score — directly comparable with
the macro column above.

| system | JevBench (231, family-macro) |
|---|---:|
| autotrust/JEV-27B (open, Apache-2.0; Qwen3.8-27B + 109 M-param System 1 block) | **88.70** |
| TypeSafe Jev 1.13 (hosted API, closed — the distillation teacher) | 87.18 |
| Open-Jev-9B | 77.13 |
| NeoHorse-Jev-4B | 75.73 |
| Kev-4B | 73.71 |
| Laya English | 55.82 |
| *fork: Qwen3.5-4B tree mode (our run — fedora anchor)* | *75.71* |
| *Jev-Style-0.8B bridge (our run)* | *63.78* |
| *VTX-JEV-3 (our run)* | *43.18* |
| *OpenCodifier engine (our run)* | *40.11* |

### Published rows — JevBench authors' board (plain accuracy)

Source: jev-style repo README, JevBench RESULTS, jev.page (recorded in
`JEVBENCH.md`).

| system | public acc | provenance |
|---|---:|---|
| hosted Jev (TypeSafe) | 86.6 % | official |
| llm-qwen3.5-4b (jev.page decision server) | 80.5 % / 651 ms p50 | self-reported |
| Jev-Style-2B v3 | 73.6 % | self-run, GGUF F16 |
| strands-decider-2B-hobson-v19 | 72.3 % (167/231), Brier 0.342, ECE 0.050 | official, preregistered run record |
| open-jev-zefan-2b | 64.5 % | board |
| **Jev-Style-0.8B v3** | **64.1 % (148/231)** | self-run, official harness — reproduced by our bridge |
| *fork: Qwen3.5-4B tree mode (our run, base model — fedora anchor)* | *76.6 % (177/231)* | our run, authors' harness — no decision tune, so the gap to jev.page's 4B (80.5 %) is a training-recipe delta, not harness drift |
| decider-2b | 71.0 % | board |
| Laya | 58.4 % | official |

Caveats that keep this honest: the public half is training-exposed by
construction (rows are context, not certification); autotrust's table is
family-macro while the authors' board is plain accuracy — the two
published tables are kept separate for exactly that reason; and latency
is comparable only within our own runs (client wall on one CPU host).
The strands row (2026-10-01, source
`huggingface.co/StrandsAgents/strands-decider-2B-hobson-v19` +
`github.com/strands-labs/strands-decider`) publishes its own retrain
noise — σ ≈ 3.2 tasks of 231 across six retrains of one recipe — and
therefore treats single-run differences under ~10 tasks as unresolved;
our fork-arm comparisons inherit that caveat (RESEARCH.md §6.5).

## Board B — our locked suite (the full model board)

Suite: `suite/suite.json`, 120 items, byte-locked (seed 20260926),
three classes × 40 — `metadata_match` (constraint checking),
`lexical_semantic` (paraphrase routing), `relational_compositional`
(multi-fact composition). Accuracy is top-1 candidate id; ECE is the
winner probability's calibration; every decision arm replays the whole
suite twice and must match bit-for-bit. The engine's abstain/verify
routing is part of its contract, not error.

### Measured tiers (D16 — what the runtime actually ships)

| tier | pick | acc | ECE | rel | p50 | size |
|---|---|---:|---:|---:|---:|---:|
| frontier (verifier) | MiMo-V2.6-Distill-Qwen-9B **Q3_K_S** | 0.817 | **0.048** | **0.525** | 14.3 s | 4063 MiB |
| interactive reference | Qwen3.5-4B **Q3_K_S** | 0.800 | 0.069 | 0.45 | 6.8 s | 2009 MiB |
| interactive (fastest 0.800) | Qwen3.5-4B **UD-Q4_K_XL** | 0.800 | 0.074 | 0.45 | 4.9 s | 2778 MiB |
| balanced | Qwen3.5-2B (Q4_K_M) | 0.725 | **0.062** | 0.50 | 1.7 s | 1222 MiB |
| fast | Qwen3.5-0.8B (q4_0) | 0.650 | 0.074 | 0.35 | 613 ms | 537 MiB |
| zero-ML floor (engine default) | relational-v1 over lexical | 0.683 | 0.094 | **0.950** | **1.3 ms** | 0 MiB |

### Complete board (110 runs, 2026-09-25 → 2026-10-05)

Columns: accuracy (metadata / lexical / relational), overall accuracy,
ECE, single-decision p50, run-twice determinism. `(chat screen)` rows
are token-by-token JSON-writing baselines from forks without a decision
arm — sampled decode, never comparable to decision rows.

| run | acc (meta/lex/rel) | acc | ECE | p50 | determinism |
|---|---|---|---|---|---|
| embed__embeddinggemma-300M-Q8_0.json | 0.62 / 0.62 / 0.25 | 0.500 | 0.256 | 634.9ms | yes |
| embed__gte-modernbert-base.json | 0.45 / 0.78 / 0.50 | 0.575 | 0.330 | 3374.8ms | yes |
| embed__gte-modernbert-onnx-fp32.json | 0.45 / 0.78 / 0.50 | 0.575 | 0.330 | 811.4ms | yes |
| embed__gte-modernbert-onnx-int8.json | 0.28 / 0.78 / 0.28 | 0.442 | 0.233 | 674.1ms | yes |
| embed__gte-modernbert-onnx-q4-b32.json | 0.47 / 0.75 / 0.50 | 0.575 | 0.330 | 815.7ms | yes |
| embed__minilm-l6-v2.json | 0.30 / 0.62 / 0.35 | 0.425 | 0.172 | 107.0ms | yes |
| engine__builtin-lexical.json | 0.88 / 0.23 / 0.35 | 0.483 | 0.115 | 5.3ms | yes |
| engine__lexical__long__focused.json ¹ | 0.88 / 0.23 / 0.95 | 0.683 | 0.094 | 9.2ms | yes |
| engine__lexical__long__full.json ¹ | 0.88 / 0.23 / 0.95 | 0.683 | 0.094 | 6.7ms | yes |
| engine__relational-v1.json | 0.88 / 0.23 / 0.95 | 0.683 | 0.094 | 1.3ms | yes |
| k2chat__K2-Horizon-1B-Q4_K_M.json (chat screen) | 0.88 / 0.80 / 0.50 | 0.725 | — | 1499.1ms | n/a (sampled) |
| k2chat__K2-Horizon-4B-Q4_K_M.json (chat screen) | 1.00 / 0.93 / 0.17 | 0.700 | — | 4878.9ms | n/a (sampled) |
| k2chat__K2-Horizon-7B-Q4_K_M.json (chat screen) | 1.00 / 0.95 / 0.45 | 0.800 | — | 9052.1ms | n/a (sampled) |
| laya__en.json | 0.30 / 0.80 / 0.33 | 0.475 | 0.089 | 547.6ms | yes |
| llama__Bonsai-4B.json | 0.93 / 0.70 / 0.33 | 0.650 | 0.233 | 4861.2ms | yes |
| llama__Falcon-H1-Tiny-90M-Instruct.json | 0.23 / 0.28 / 0.33 | 0.275 | 0.462 | 459.4ms | yes |
| llama__Falcon-H1-Tiny-Tool-Calling.json | 0.28 / 0.20 / 0.33 | 0.267 | 0.278 | 147.0ms | yes |
| llama__Jev-Style-0.8B-Decision-v3-Q4_K_M.json (caveat) ² | 0.20 / 0.30 / 0.15 | 0.217 | 0.408 | 866.7ms | yes |
| llama__LFM2.5-2.6B-Q3.8-TBrilliance-NEO-MAX-Q6_K.json | 0.95 / 0.78 / 0.28 | 0.667 | 0.213 | 8347.8ms | yes |
| llama__LFM2.5-2.6B-Q4_K_M.json | 0.93 / 0.55 / 0.35 | 0.608 | 0.223 | 2546.8ms | yes |
| llama__Llama-3.2-1B-Instruct.json | 0.47 / 0.35 / 0.35 | 0.392 | 0.265 | 996.5ms | yes |
| llama__MiMo-V2.6-Distill-Qwen-9B-IQ3_XXS.json | 1.00 / 0.93 / 0.42 | 0.783 | 0.078 | 10570.2ms | yes |
| llama__MiMo-V2.6-Distill-Qwen-9B-Q3_K_M.json | 1.00 / 0.93 / 0.53 | 0.817 | 0.081 | 18395.2ms | yes |
| llama__MiMo-V2.6-Distill-Qwen-9B-Q3_K_S.json | 1.00 / 0.93 / 0.53 | 0.817 | 0.048 | 14307.4ms | yes |
| llama__MiniCPM5-1B.json | 0.95 / 0.35 / 0.15 | 0.483 | 0.232 | 693.9ms | yes |
| llama__Qwen3.5-0.8B-q4_0.json | 0.93 / 0.68 / 0.35 | 0.650 | 0.074 | 612.6ms | yes |
| llama__Qwen3.5-2B-UD-IQ2_XXS.json | 0.55 / 0.45 / 0.15 | 0.383 | 0.212 | 1619.9ms | yes |
| llama__Qwen3.5-2B-UD-Q2_K_XL.json | 0.62 / 0.40 / 0.33 | 0.450 | 0.134 | 1954.7ms | yes |
| llama__Qwen3.5-2B-UD-Q3_K_XL.json | 0.95 / 0.82 / 0.50 | 0.758 | 0.135 | 1826.4ms | yes |
| llama__Qwen3.5-2B-UD-Q4_K_XL.json | 0.97 / 0.80 / 0.42 | 0.733 | 0.095 | 2184.4ms | yes |
| llama__Qwen3.5-2B-UD-Q5_K_XL.json | 1.00 / 0.80 / 0.47 | 0.758 | 0.089 | 2189.1ms | yes |
| llama__Qwen3.5-2B-UD-Q6_K_XL.json | 0.97 / 0.80 / 0.50 | 0.758 | 0.079 | 2170.8ms | yes |
| llama__Qwen3.5-2B-UD-Q8_K_XL.json | 1.00 / 0.80 / 0.50 | 0.767 | 0.111 | 2175.4ms | yes |
| llama__Qwen3.5-2B.json | 0.95 / 0.72 / 0.50 | 0.725 | 0.062 | 1734.8ms | yes |
| llama__Qwen3.5-4B-Q3_K_M.json | 1.00 / 0.93 / 0.40 | 0.775 | 0.093 | 5660.3ms | yes |
| llama__Qwen3.5-4B-Q3_K_S.json | 1.00 / 0.95 / 0.45 | 0.800 | 0.069 | 6776.0ms | yes |
| llama__Qwen3.5-4B-UD-IQ2_XXS.json | 0.97 / 0.50 / 0.38 | 0.617 | 0.079 | 3697.6ms | yes |
| llama__Qwen3.5-4B-UD-IQ3_XXS.json | 1.00 / 0.95 / 0.42 | 0.792 | 0.065 | 15747.8ms | yes |
| llama__Qwen3.5-4B-UD-Q3_K_XL.json | 1.00 / 0.97 / 0.42 | 0.800 | 0.099 | 7222.6ms | yes |
| llama__Qwen3.5-4B-UD-Q4_K_XL.json | 1.00 / 0.95 / 0.45 | 0.800 | 0.074 | 4871.7ms | yes |
| llama__Qwen3.5-4B-q4_k_m.json | 1.00 / 0.93 / 0.38 | 0.767 | 0.098 | 4228.3ms | yes |
| llama__Qwen3.8-0.8B-Distilled.json | 0.85 / 0.55 / 0.30 | 0.567 | 0.098 | 807.7ms | yes |
| llama__Qwen3.8-2B-Distill.json | 0.90 / 0.62 / 0.33 | 0.617 | 0.095 | 1343.9ms | yes |
| llama__Qwen3.8-4B-Distill.json | 1.00 / 0.95 / 0.35 | 0.767 | 0.057 | 3756.6ms | yes |
| llama__gemma-3-270m-it-q4_k_m.json | 0.23 / 0.12 / 0.25 | 0.200 | 0.626 | 298.5ms | yes |
| llama__gemma-3-4b-it.json | 0.95 / 0.85 / 0.45 | 0.750 | 0.236 | 3039.7ms | yes |
| llama__glm5.1-distill.json | 0.40 / 0.15 / 0.20 | 0.250 | 0.285 | 2194.4ms | yes |
| llama__granite-4.0-350m-q4_k_m.json | 0.30 / 0.50 / 0.23 | 0.342 | 0.239 | 422.8ms | yes |
| llama__jebadiah-4b-v2-Q8_0.json | 1.00 / 0.90 / 0.50 | 0.800 | 0.051 | 2722.9ms | yes |
| llama__jebadiah-4b-v2-Q8_0__dist.json ³ | 1.00 / 0.90 / 0.50 | 0.800 | 0.051 | 2722.9ms | yes |
| llama__jebadiah-4b-v2-Q3_K_S.json | 1.00 / 0.88 / 0.50 | 0.792 | 0.087 | 2500.6ms | yes |
| llama__jebadiah-9b-v2-Q8_0.json | 1.00 / 0.95 / 0.55 | 0.833 | 0.075 | 5626.3ms | yes |
| llama__jebadiah-9b-v2-Q3_K_S.json | 1.00 / 0.90 / 0.55 | 0.817 | 0.099 | 5416.4ms | yes |
| llama__qwen0.5b.json | 0.60 / 0.33 / 0.25 | 0.392 | 0.218 | 694.8ms | yes |
| llama__qwen1.5b.json | 0.90 / 0.45 / 0.40 | 0.583 | 0.191 | 1247.8ms | yes |
| llama__qwen3b.json | 0.90 / 0.75 / 0.38 | 0.675 | 0.263 | 2575.2ms | yes |
| parity__jebadiah-4b-v2-letters.json ⁴ | 0.98 / 1.00 / 0.53 | 0.833 | 0.052 | 1973.8ms | yes; parity 0.908 |
| parity__jebadiah-9b-v2-letters.json ⁴ | 0.08 / 0.03 / 0.33 | 0.142 | 0.501 | 2869.0ms | yes; parity 0.150 |
| stock__clef-flash-q4_k_m-letters.json ¹⁰ | 0.00 / — / 0.33 | 0.008 | 0.529 | 1690.4ms | yes (Δp 0.000); 115 invalid dists |
| stock__e2b-abl-opusdist-q4km-letters.json ¹² | 0.63 / 0.16 / 0.30 | 0.350 | 0.423 | 632.5ms | **no** (Δp 1.000); 4 invalid dists |
| stock__e2b-gdist-g31pro-q4km-letters.json ¹² | 0.95 / 0.83 / 0.28 | 0.683 | 0.212 | 546.0ms | yes (Δp 0.242) |
| stock__e2b-noqat-bf16-letters.json ¹¹ | 1.00 / 0.24 / 0.45 | 0.500 | 0.355 | 889.3ms | yes (Δp 0.023); 10 invalid dists |
| stock__e2b-noqat-iq4nl-letters.json ¹¹ | 0.91 / 0.15 / 0.43 | 0.442 | 0.363 | 569.2ms | **no** (Δp 0.342); 7 invalid dists |
| stock__e2b-noqat-iq4xs-letters.json ¹¹ | 0.91 / 0.21 / 0.35 | 0.450 | 0.376 | 675.9ms | **no** (Δp 1.000); 7 invalid dists |
| stock__e2b-noqat-q3km-letters.json ¹¹ | 0.74 / 0.48 / 0.30 | 0.500 | 0.375 | 657.5ms | **no** (Δp 1.000); 1 invalid dist |
| stock__e2b-noqat-q3ks-letters.json ¹¹ | 0.97 / 0.41 / 0.40 | 0.550 | 0.226 | 610.9ms | **no** (Δp 0.484); 6 invalid dists |
| stock__e2b-noqat-q4_0-letters.json ⁹ | 0.21 / 0.71 / 0.30 | 0.350 | 0.435 | 472.4ms | **no** (Δp 1.000); 11 invalid dists |
| stock__e2b-noqat-q4_1-letters.json ¹¹ | 0.97 / 0.26 / 0.45 | 0.475 | 0.362 | 817.6ms | **no** (Δp 1.000); 11 invalid dists |
| stock__e2b-noqat-q4km-letters.json ¹¹ | 0.88 / 0.11 / 0.45 | 0.425 | 0.414 | 671.9ms | **no** (Δp 1.000); 10 invalid dists |
| stock__e2b-noqat-q4ks-letters.json ⁸ | 0.21 / 0.85 / 0.40 | 0.433 | 0.411 | 479.8ms | **no** (Δp 0.676); 9 invalid dists |
| stock__e2b-noqat-q5km-letters.json ¹¹ | 0.94 / 0.23 / 0.43 | 0.483 | 0.378 | 627.1ms | **no** (Δp 0.928); 6 invalid dists |
| stock__e2b-noqat-q5ks-letters.json ¹¹ | 0.97 / 0.25 / 0.43 | 0.500 | 0.376 | 589.1ms | **no** (Δp 1.000); 6 invalid dists |
| stock__e2b-noqat-q6k-letters.json ¹¹ | 1.00 / 0.28 / 0.48 | 0.525 | 0.355 | 656.9ms | **no** (Δp 1.000); 8 invalid dists |
| stock__e2b-noqat-q8_0-letters.json ¹¹ | 0.94 / 0.24 / 0.43 | 0.475 | 0.379 | 1100.2ms | yes (Δp 0.054); 10 invalid dists |
| stock__e2b-noqat-udiq2m-letters.json ¹¹ | 0.65 / 0.66 / 0.36 | 0.392 | 0.340 | 585.4ms | **no** (Δp 1.000); 37 invalid dists |
| stock__e2b-noqat-udiq3xxs-letters.json ¹¹ | 0.41 / 0.21 / 0.38 | 0.300 | 0.490 | 639.9ms | **no** (Δp 0.665); 9 invalid dists |
| stock__e2b-noqat-udq3kxl-letters.json ¹¹ | 0.78 / 0.27 / 0.28 | 0.433 | 0.418 | 589.4ms | **no** (Δp 0.484); 3 invalid dists |
| stock__e2b-noqat-udq4kxl-letters.json ⁸ | 0.23 / 0.72 / 0.33 | 0.375 | 0.461 | 523.9ms | **no** (Δp 0.645); 9 invalid dists |
| stock__e2b-noqat-udq5kxl-letters.json ¹¹ | 0.94 / 0.38 / 0.45 | 0.517 | 0.358 | 637.9ms | **no** (Δp 0.873); 11 invalid dists |
| stock__e2b-noqat-udq6kxl-letters.json ¹¹ | 1.00 / 0.21 / 0.45 | 0.475 | 0.379 | 677.6ms | **no** (Δp 1.000); 11 invalid dists |
| stock__e2b-noqat-udq8kxl-letters.json ¹¹ | 0.97 / 0.24 / 0.45 | 0.492 | 0.362 | 1045.6ms | yes (Δp 0.094); 10 invalid dists |
| stock__e2bqat-official-q4_0-letters.json ⁸ | 0.97 / 1.00 / 0.45 | 0.808 | 0.102 | 422.9ms | yes (Δp 0.020) |
| stock__e2bqat-r18b-crossbuild-letters.json ¹¹ | 1.00 / 0.98 / 0.45 | 0.808 | 0.102 | 460.2ms | yes (Δp 0.020) |
| stock__e2bqat-suiteimx-iq3xxs-letters.json ¹³ | 1.00 / 0.95 / 0.425 | 0.792 | 0.058 | 599.2ms | yes (Δp 0.140) |
| stock__e2bqat-suiteimx-q3km-letters.json ¹³ | 1.00 / 0.875 / 0.425 | 0.767 | 0.107 | 578.6ms | yes (Δp 0.022) |
| stock__e2bqat-suiteimx-q4km-letters.json ¹³ | 1.00 / 0.975 / 0.45 | 0.808 | 0.112 | 492.8ms | yes (Δp 0.063) |
| stock__e2bqat-udq2kxl-letters.json ⁸ | 0.57 / 0.53 / 0.35 | 0.483 | 0.170 | 426.5ms | **no** (Δp 0.502) |
| stock__e4bqat-official-q4_0-letters.json ⁹ | 0.90 / 1.00 / 0.50 | 0.558 | 0.169 | 883.8ms | yes (Δp 0.026); 29 invalid dists |
| stock__gemma-4-e2b-qat-letters.json ⁷ | 0.93 / 1.00 / 0.38 | 0.767 | 0.137 | 454.1ms | yes (Δp 0.020) |
| stock__gemma-4-e4b-it-letters.json ⁷ | 0.64 / 1.00 / 0.45 | 0.367 | 0.304 | 1052.4ms | **no** (Δp 0.247); 41 invalid dists |
| stock__gemma-4-e4b-qat-letters.json ⁷ | 0.54 / 0.88 / 0.50 | 0.400 | 0.281 | 977.0ms | **no** (Δp 1.000); 33 invalid dists |
| stock__kev-4b-letters.json ⁶ | 1.00 / 0.95 / 0.53 | 0.617 | 0.151 | 1378.2ms | yes (Δp 0.000); 25 invalid dists |
| stock__lfm25-230m-q4km-letters.json ¹² | 0.17 / 0.58 / 0.10 | 0.125 | 0.572 | 68.6ms | yes (Δp 0.000); 65 invalid dists |
| stock__lfm25-350m-q4_0-letters.json ¹² | 0.18 / 0.23 / 0.25 | 0.217 | 0.587 | 88.9ms | yes (Δp 0.000) |
| stock__lfm25-350m-q4km-letters.json ¹² | 0.25 / 0.05 / 0.20 | 0.167 | 0.631 | 101.9ms | yes (Δp 0.000) |
| stock__lfm25-350m-qad-q4_0-letters.json ¹² | 0.25 / 0.15 / 0.23 | 0.208 | 0.551 | 86.7ms | yes (Δp 0.000) |
| stock__lfm25-8ba1b-q4km-letters.json ¹² | — / — / — | 0.000 | — | 791.5ms | yes (Δp 0.000); 120 invalid dists ¹² |
| stock__lfm25-8ba1b-udq4km-letters.json ¹² | — / — / — | 0.000 | — | 851.8ms | yes (Δp 0.000); 120 invalid dists ¹² |
| stock__muse-1b-q4km-letters.json ¹² | 0.30 / 0.09 / 0.22 | 0.117 | 0.451 | 212.6ms | yes (Δp 0.140); 42 invalid dists |
| stock__neohorse-1-4b-alt-letters.json ⁷ | 0.80 / 0.65 / 0.30 | 0.583 | 0.195 | 1102.1ms | yes (Δp 0.000) |
| stock__qwen35-4b-noqat-q4_0-letters.json ¹⁰ | 0.12 / 0.05 / 0.10 | 0.092 | 0.567 | 965.6ms | yes (Δp 0.000) |
| stock__tiny1b-f16-letters.json ¹² | — / — / — | 0.000 | — | 240.1ms | yes (Δp 0.000); 120 invalid dists ¹² |
| stock__tiny1b-q4km-letters.json ¹² | — / — / — | 0.000 | — | 153.5ms | yes (Δp 0.000); 120 invalid dists ¹² |
| stock__winnow-12b-q4km-letters.json ⁷ | 0.42 / 0.81 / 0.65 | 0.542 | 0.284 | 2569.2ms | yes (Δp 0.000); 13 invalid dists |
| stock__winnow-12b-q8-letters.json ⁷ | 0.20 / 0.23 / 0.62 | 0.350 | 0.445 | 4741.7ms | yes (Δp 0.000); 1 invalid dist |
| stock__winnow-e4b-letters.json | 1.00 / 0.97 / 0.55 | 0.842 | 0.109 | 2171.3ms | yes (Δp 0.017) |
| stock__winnow-e4b-letters-Q4_K_S.json ⁵ | 1.00 / 1.00 / 0.55 | 0.825 | 0.101 | 951.4ms | yes (Δp 0.041); 3 invalid dists |
| stock__winnow-e4b-letters-Q4_K_M.json ⁵ | 0.97 / 1.00 / 0.57 | 0.817 | 0.083 | 973.3ms | **no** (Δp 0.998); 4 invalid dists |
| stock__winnow-e4b-letters-Q4_0.json ⁵ | 0.97 / 1.00 / 0.56 | 0.717 | 0.115 | 896.8ms | yes (Δp 0.025); 16 invalid dists |
| stock__winnow-e4b-letters-IQ4_XS.json ⁵ | 0.78 / 0.80 / 0.50 | 0.692 | 0.196 | 1202.6ms | **no** (Δp 0.473) |
| stock__winnow-e4b-letters-IQ3_M.json ⁵ | 0.90 / 0.97 / 0.23 | 0.658 | 0.082 | 1564.4ms | **no** (Δp 0.984); 5 invalid dists |
| stock__winnow-e4b-letters-Q3_K_S.json ⁵ | 0.81 / 0.92 / 0.40 | 0.408 | 0.124 | 1315.9ms | **no** (Δp 0.997); 41 invalid dists |
| stock__yoozlabs-qwen35-08b-qat-q4_0-letters.json ¹⁰ | 0.72 / 0.76 / 0.40 | 0.608 | 0.061 | 178.3ms | yes (Δp 0.000); 3 invalid dists |
| stock__yoozlabs-qwen35-4b-qat-q4_0-letters.json ¹⁰ | 0.65 / 0.62 / 0.35 | 0.542 | 0.237 | 910.4ms | yes (Δp 0.000) |
| torch__e2bqat-mobile-bf16-letters.json ¹¹ | 1.00 / 0.95 / 0.45 | 0.800 | 0.102 | 7490.2ms | yes (Δp 0.000) |
| torch__tiny1b-bfloat16-letters.json ¹¹ | 0.20 / 0.35 / 0.35 | 0.300 | 0.205 | 1585.5ms | yes (Δp 0.000) |
| torch__tiny1b-fp32-letters.json ¹¹ | 0.13 / 0.28 / 0.20 | 0.200 | 0.307 | 660.5ms | yes (Δp 0.000) |
| vtx__VTX-JEV-3-fp32.json | 0.20 / 0.33 / 0.42 | 0.317 | 0.099 | 0.8ms | yes |
| vtx__VTX-JEV-3-lf2.json | 0.17 / 0.25 / 0.30 | 0.242 | 0.136 | 1.1ms | yes |

¹ Long-context variant: the derived `suite_long.json` (~3,681-token
contexts), answer-identical to the standard engine run — listed for
completeness, not a board row of the 120-item suite.
² Interface-mismatch row: the 0.8B tune is read at per-option verdict
slots (its trained interface) and was never trained on this harness's
readouts — the row measures the mismatch; its JevBench-native row is the
bridge above (0.6494).
³ Distribution-emission validation (2026-10-03): the fork rebuilt at
`38de7eb` (native per-choice distributions) reproduces the verdict-slot
4B arm **exactly** — same decisions, same ECE, 100 % winner agreement,
near-one-hot native distributions (peaked decision-slot logits). This is
determinism evidence across builds and means the D15 refits are purely
distribution-fidelity work, as with the `d15` JevBench arm above.
⁴ Stock-build letters-parity rows (r10, 2026-10-03): the same artifacts
as the fork tree arms, read through the stock llama.cpp build's
constrained-letter readout (-t 12, quiet window, load 5.0–10.4).
`parity` is winner agreement against the fork decision arm (120/120
joined, winner-only — the fork rows carry winner probability only).
The 4B row **beats** its fork-tree arm (0.833 vs 0.800 at the same
artifact size); the 9B row collapses to chance (120/120 valid,
confidently wrong) while its fork-tree arm holds 0.833 — the 9B's
board advantage is a property of the fork's tree readout, not the
artifact alone. Confound note: build, readout, and threads changed
together (stock+letters+12t vs fork+tree+14t).
⁵ Requant ladder (r11b, 2026-10-03, fedora i5-13600K, stock build,
letters readout, -t 12): all six quants cut from the **Q8_0 release
artifact** with `--allow-requantize` — a requant, not a from-F16 quant.
Findings: Q4_K_S is the operating point (0.825, 2.3× faster than the
Q8_0 row above, −1.7 pp); K-quant requants hold accuracy down to 4-bit
and then cliff hard (Q3_K_S 0.408 with 41/120 malformed distributions);
legacy Q4_0 and the i-quants collapse under requant (Q4_0 0.717 with 16
invalid dists, IQ4_XS 0.692 with per-class breakdown damage, IQ3_M
0.658). This is a *provenance* rule, not a quant-size law: the 9B's
Q3_K_S — cut from F16 — holds 0.817 (row above). Determinism: 4 of 6
requants flip predictions between identical replays, **including the
default pick Q4_K_M** — a per-quant determinism replay must be a ship
gate, and from-Q8_0 requants should stay K-quant ≥ 4-bit. The ECE
column is computed on surviving distributions only, so it tracks
distribution validity, not calibration quality (IQ3_M's 0.082 sits
next to a det flip and 5 invalid dists).

⁶ r12 TypeSafeAI-ecosystem arms (2026-10-03, fedora, stock build,
letters readout, -t 12). **Kev-4B Q4_K_M** (row above): well under the
Winnow-E4B family on this suite (0.617 vs 0.842) with the worst
distribution health of any passing arm (25/120 invalid dists) — its
published strength is JevBench macro (73.71, published row above), a
different axis. Replay-clean (Δp 0.000). The other three candidates
did not yield letters arms: **NeoHorse-Jev-4B Q4_K_M is unloadable**
in the stock build (`wrong number of tensors; expected 728, got 426`
— foreign/newer arch export); **Julia-1 Q8_0 (168 MB, ~160M params)
loads as a `laya`-arch decision model but the context is pooling-only
— logits are refused, so the letters lane cannot drive it** (probe:
`--pooling none` still 500 "the current context does not support
logits computation"); **Qyvos ships safetensors + a Python `julia`
package, no GGUF**. Follow-up lanes, in preference order: Julia-1 via
its official ONNX export (the natural `InferenceBackend` arm for
`opencodifier-runtime`, parity cases included), a laya-GGUF endpoint
lane, NeoHorse under a newer llama.cpp, and a `julia`-package adapter
for Qyvos.

⁷ r13/r14 bake-off arms (2026-10-04, fedora i5-13600K, stock build,
letters readout, -t 12; load gate < 20 two-sample with per-run load
recorded — windows 4.2–18.7, so treat per-arm latencies as
quiet-window-relative, not absolute). All six artifacts are
third-party release downloads, no requants: Winnow-12B Q4_K_M
(Piotr1215) and Q8_0 (EldanRing — a different publisher's quant, so
part of the Q8 < Q4 gap may be publisher divergence rather than
quantization), gemma-4 E4B-it / E4B-qat / E2B-qat (unsloth
UD-Q4_K_XL), NeoHorse-1-4B Q4_K_M (TokenRhythm — the **base** model,
not the unloadable Jev-tuned 4B of footnote 6). Findings: **gemma-4
E2B-qat is the new small-model operating point** — 0.767 at 454 ms
p50, zero invalid distributions, replay-clean (Δp 0.020): below
Winnow-E4B Q8_0's 0.842 but 4.8× faster, and the size class of the
VIVERE transfer target. **The gemma-4 E4B arms collapse** (it 0.367,
qat 0.400) with 41/33-of-120 invalid distributions and determinism
flips (Δp 0.247 / 1.000) — same family, twice the parameters, worse;
the failure mode is corrupted distribution *emission* under the
constrained readout (jebadiah-9b's collapse was confident wrongness;
this is malformed output), and QAT did not rescue it. **Winnow-12B
inverts its own family** — Q4_K_M 0.542 / Q8_0 0.350 with the
board-worst ECE (0.445) vs Winnow-E4B's 0.842; the 12B arms do cross
the relational ceiling (0.65 / 0.62) yet lose overall: compositional
strength without calibrated decision quality. **NeoHorse-1-4B base,
untuned: 0.583 with perfectly clean emission** (0 invalid, Δp 0.000)
— beats both tuned Winnow-12B arms. Under this readout, decision
quality is not monotone in parameters.

⁸ r15 gemma-4-E2B 4-bit landscape (2026-10-04, fedora i5-13600K,
stock build, letters readout, -t 12, quiet windows, load 0.5–11.5;
all artifacts ready-made release downloads). Provenance first: **no
official Google GGUF export exists** (probed 2026-10-04 with a valid
token: the weights repo ships safetensors only, every `-GGUF` repo
name 404s), so the "official" arm is lmstudio-community's **plain
Q4_0 of the official QAT release** — official weights, community
quantizer. Findings: **the official-weights Q4_0 beats unsloth's
dynamic mix of the same QAT checkpoint** — 0.808 / ECE 0.102 /
p50 423 ms vs the r13 UD-Q4_K_XL arm's 0.767 / 0.137 / 454 ms
(+4.1 pp, better calibration, faster, both perfectly clean and
replay-clean). Caveat: the lmstudio file is larger (3.12 vs 2.44 GiB
— unsloth's dynamic mix compresses harder), so part of the gap is
bits, not just recipe. **QAT is carrying the number**: at the
matched UD-Q4_K_XL quant, QAT 0.767 vs non-QAT **0.375** — a 39 pp
collapse, and the non-QAT arms also break determinism (Δp 0.645) and
emit 9 malformed distributions. The unsloth non-QAT Q4_K_S arm fails
the same way (0.433, Δp 0.676, 9 invalid). The "QAT-lossless" naming
is load-bearing: on this lane, QAT is what makes a 4-bit quant work
at all. **The operator quant floor holds even for QAT weights**:
unsloth's sub-4-bit UD-Q2_K_XL manages only 0.483 with a determinism
flip — though note 2-bit-QAT still outscores non-QAT-4-bit. Net: the
new small-model operating point is the official-weights Q4_0
(0.808 at 423 ms p50), pending the per-class routing and calibration
levers (footnote 7 discussion) on top.

⁹ r16 QAT-format rescue test + matched-format control (2026-10-04,
fedora i5-13600K, stock build, letters readout, -t 12, quiet windows,
load 0.2–11.5; both artifacts ready-made release downloads —
lmstudio-community gemma-4-E4B-it-QAT-Q4_0 (native-format Q4_0,
4.80 GiB) and unsloth gemma-4-E2B-it Q4_0 (non-QAT weights, 2.83
GiB)). **The r13 E4B-qat collapse was part format, part arch.** At
its native Q4_0 the E4B-QAT recovers 0.400 → 0.558 (+15.8 pp; 29
invalid dists vs 33; determinism now clean, Δp 0.026) — the
format-mismatch hypothesis holds for the collapsed portion — but it
still trails E2B-QAT Q4_0's 0.808 by ~25 pp and keeps the class's
invalid-distribution problem: an arch-level deficit remains that no
primary source explains (unsloth's own KLD data predicts the
opposite ordering — REPORT.md r16 has the reconciliation). **The
matched-format QAT A/B lands at +45.8 pp**: non-QAT E2B Q4_0 scores
0.350 / ECE 0.435 with a full determinism flip (Δp ≈ 1.0) and 11
invalid distributions against official-QAT Q4_0's 0.808 — QAT at
matched format is worth more than the r15 K-quant A/B's +39.2 pp,
and non-QAT E2B at 4 bits is both inaccurate and unstable. Primary-
source context from the same day: Q4_0 is only the *secondary* QAT
target for the E-series (mobile int2/int4 + int8 is primary, tech
report Table 3), and llama.cpp promotes the tied per-layer-embedding
(PLE, ~46 % of E2B params) to Q6_K — which is why the official Q4_0
file outsizes unsloth's dynamic mix.

¹⁰ r17 second-family QAT A/B + clef-flash backbone (2026-10-04,
fedora i5-13600K, stock build, letters readout, -t 12, quiet
windows, load 0.2–9.0; all artifacts ready-made release downloads —
YoozLabs Qwen3.5 QAT Q4_0s are third-party QAT with unverified
provenance, the 4B control is unsloth's non-QAT Q4_0, clef-flash is
bartowski's Q4_K_M of Cloudflare's release). **QAT rescues legacy
Q4_0 in a second family — but does not beat the family's own
K-quants.** Matched-format on 4B: QAT 0.542 vs non-QAT **0.092** —
+45.0 pp, the same magnitude as gemma-E2B's +45.8 pp, yet both sit
far under the same family's non-QAT K-quants (Q3_K_S / UD-Q4_K_XL
0.800): for Qwen3.5, legacy Q4_0 is a dead end at 4 B with or
without QAT, and the operating point stays K-quant. At 0.8 B the
sign flips: QAT **regresses** accuracy (0.608 vs the board's
non-QAT q4_0 0.650) while improving calibration (ECE 0.061 vs
0.074) and p50 (178 ms vs 613 ms — cross-day caveat: the non-QAT
arm was measured Sep 26 on an earlier stock build). **The non-QAT
4B Q4_0 collapse is confident wrongness, not emission corruption**:
0 invalid distributions, perfectly replay-stable (Δp 0.000),
ECE 0.567 — well-formed distributions pointing at wrong answers
(the jebadiah-9b failure mode), unlike gemma's malformed-emission
collapses. All four r17 arms replay-clean (Δp 0.000). **clef-flash
Q4_K_M is emission-broken on the stock readout** — 115/120 invalid
distributions, 5 valid predictions total, 0.008 — and the row
carries a hard caveat: bartowski's GGUF is **backbone-only**
(Cloudflare's `joint_head.safetensors` is a separate custom-arch
file llama.cpp cannot execute), so this row measures the
post-trained Qwen3.5-9B backbone under constrained decoding and
does NOT reproduce Cloudflare's published Clef numbers. A template
A/B (`--template` runner flag lane) is the follow-up before any
conclusion about the backbone itself.

¹¹ r19 full E2B quantization ladder + mobile mixed-precision QAT +
safetensors-vs-GGUF format A/B start (2026-10-04/05, fedora i5-13600K,
stock build `1537a0a`, letters readout, -t 12, load gate < 20 — the
ladder ran against a co-tenant link storm whose waits stretched to
hours and several windows sat at load 130–290; accuracy is
load-insensitive by construction (temperature 0, single-token readout)
but per-arm latencies are quiet-window-relative). All artifacts
ready-made release downloads: 17 rungs of the gemma-4-E2B-it non-QAT
ladder (2.7→16 bpw, plain K-quants, i-quants, unsloth UD-* dynamic
mixes, BF16), the official Google QAT export's Q4_0 (r15/r16 rows),
and Google's **mixed-precision QAT mobile export** (lm_head 2-bit /
early-MLP 4-bit / late-MLP 2-bit / attn 4-bit / PLE 8-bit, ~2.5 average
bits) run through transformers 5.17 on the torch lane
(`--permutations 1`, headline metrics from pass 0 as always). Findings:

- **The non-QAT ladder is flat — the base model is the ceiling, not
  the bits.** All 17 rungs score 0.300–0.550 (mean ≈ 0.47) and BF16
  itself is 0.500: no format or bit-width buys decision-readout
  accuracy on this model, and the spread across the whole ladder fits
  inside the ±0.1 band 60–120 items expect. ECE is uniformly bad
  (0.226–0.490). The r15 "QAT is carrying the number" finding
  generalizes: below QAT, nothing is carrying anything.
- **The matched-format QAT delta (+45.8 pp, r16) stands as the single
  largest measured effect on the board**, and the ladder is its
  denominator: every one of the 17 rungs sits in quantization noise;
  the only move that escaped it is training-time.
- **QAT's own cutoff is placement-shaped, not bit-shaped.** Pure 2-bit
  QAT fails (UD-Q2_K_XL 0.483, determinism flip), while the mobile
  mixed-precision export — 2/4/2/4/8 by tensor class, ~2.5 average
  bits — holds **0.800, ECE 0.102, zero invalid distributions, Δp
  0.000**. Where the bits go matters more than how many on average.
  Its 7.49 s p50 is the per-forward Python dequant cost of the torch
  lane, not an arch property; a GGUF packing of this layout is the
  obvious follow-up.
- **UD (imatrix) dynamic mixes buy nothing here.** At matched nominal
  size the UD arms sit at-or-below their K-quant counterparts
  (UD-Q3_K_XL 0.433 vs Q3_K_M 0.500; UD-Q6_K_XL 0.475 vs Q6_K 0.525;
  UD-IQ2_M 0.392 with 37/120 invalid dists — the ladder's worst), with
  the two exceptions (UD-Q5_K_XL 0.517, UD-Q8_K_XL 0.492) inside
  noise. Whatever corpus the imatrix was fitted on, it is not this
  readout's distribution.
- **Determinism is a training property, not a quant property.** 14 of
  17 ladder rungs flip predictions between identical replays (Δp up to
  1.000); the three clean ones are BF16, Q8_0, UD-Q8_K_XL. Counting
  the r15/r16 4-bit arms, 17 of 20 non-QAT quants flip while every
  QAT arm above 2 bits replays clean.
- **Crossbuild reproducibility**: the official-QAT Q4_0 arm re-run on
  a second independent llama.cpp build (r18b crossbuild) reproduces
  accuracy exactly (0.808 / ECE 0.102; p50 423 → 460 ms) — the readout
  is build-stable even where per-quant determinism is not.
- **Format A/B is open, torch side in**: the tiny 0.8B weights score
  0.200 (fp32) / 0.300 (bf16) through transformers on the same suite —
  both far under any board arm of comparable size. The GGUF side is
  pending r20c: the first two attempts died on a build-`1537a0a`
  server bug — `--chat-template-file` appears in `--help` but is
  rejected at parse (`error: invalid argument`); the arms re-run
  without the override (embedded templates boot fine, verified
  health-200).

¹² r20 batch (2026-10-05, fedora, stock build `1537a0a`, letters
readout, -t 12; LFM2.5 family, third-party E2B distills, Muse-1B, and
the tiny-GGUF sweep). **Readout floor**: the four `— / — / —` rows
(tiny1b f16/Q4_K_M, LFM2.5-8B-A1B Q4_K_M/UD) are 120/120 invalid
distributions — not zero capability. Post-queue raw probes show
llama-server applies `logit_bias` to the *sample* but reports
`top_logprobs` from the *unbiased* distribution, so a model whose
natural top-20 lacks letter mass records invalid even though the
biased sample emitted a letter. The four arms sit below that floor
for two different reasons: the tiny 0.8B's natural distribution is
degenerate (the torch lane's full-letter-softmax readout puts the
same weights at 0.200/0.300 ≈ chance — the format A/B concludes
consistent), while LFM2.5-8B-A1B's logits are healthy but its chat
template opens with `<think>` at p≈1.0 (`enable_thinking: False` is
silently unsupported by LiquidAI's template), pushing letter mass to
≈e⁻¹²⁶. **QAD null**: LiquidAI's quantization-aware-distillation
350M at Q4_0 scores 0.208 vs the plain-Q4_0 control's 0.217 — the
r16 QAT delta does not replicate as quantization-aware *distillation*
at 350M (base too weak to elicit from, or teacher/task mismatch).
**Third-party distillation transfers**: the Gemini-3.1-Pro-reasoning
distill of the same E2B base at the same Q4_K_M scores **0.683**
(+18.3 pp over the base's own BF16 ceiling, metadata-match 0.95,
ECE 0.212, clean replay) — the first non-QAT arm above the r19 band —
while the abliterated-Opus distill lands at 0.350 with a replay flip
(Δp 1.000): teacher/recipe choice is decisive, and the winning distill
carries determinism with it, the third independent replication that
replay stability rides with training quality. Speed: pp400/tg t/s —
230M 2183/149, 350M 1514–1777/107–121, 8B-A1B 177–187/24–25,
Muse-1B 389/27, distills 169–210/15–18.

¹³ r21a suite-imatrix requant ladder (2026-10-05, fedora, stock build
`1537a0a`, letters readout, -t 12). The official QAT-Q4_0 checkpoint
requantized with an imatrix fitted on the decision suite's own rendered
prompts (120 items × 3 permutations, `Answer: X` closers) — the exact
distribution the readout runs on, the matched-corpus control §12.3
asked for (`--allow-requantize`, the supported from-Q4_0 direction).
**Null at the operating point**: suite-imx Q4_K_M ties the control
digit-for-digit on accuracy and every per-class accuracy
(0.808 / 0.975 / 1.00 / 0.45) while costing +17 % p50 (492.8 vs
422.9 ms) and +0.010 ECE — the QAT checkpoint's decision capability is
already pinned by the training-time recipe; no inference-side
importance reweighting adds to it, reaffirming §12's headline from the
matched-corpus side. Below the operating point the ladder degrades
normally, with one inversion: Q3_K_M (0.767) lands *under* IQ3_XXS
(0.792) — the coarser grid survives requant better than the middle K
rung, one measurement, unreplicated. All three arms replay clean
(predictions and valid rows match; Δp is numeric drift only, growing
as bits drop: 0.063 → 0.022 → 0.140). Speed p50: Q4_K_M 492.8, Q3_K_M
578.6, IQ3_XXS 599.2 ms. The control QAT-Q4_0 arm stays the operating
point.

What the board established (full findings catalog in REPORT.md):

- **The relational ceiling.** Every model arm ≤ 4B sits at or below 0.50
  on relational composition; only the 9B class crosses it — the MiMo MoE
  (0.525) and, above it, jebadiah-9b-v2 (0.55, the highest model arm) —
  and the engine's relational solver proves its way to **0.950 at 1.3 ms**,
  four orders of magnitude below the first model that crossed it.
- **Quantization is mostly free until it isn't.** Q3_K_S ↔ Q8_0 is
  within noise on 2B/4B; the collapses are sharp (int8 embedding rung,
  IQ2_XXS), not gradual.
- **Operator quant floor (2026-10-03): 4-bit K-quant or higher for
  anything deployment-picked.** Sub-3-bit is worthless (IQ2_XXS
  collapse), and the odd-bit i-quants carry a measured speed penalty
  that the composite prices correctly: Qwen3.5-4B UD-IQ3_XXS ran
  15.7 s vs 4.9 s for UD-Q4_K_XL — 3.2× slower for −0.8 pp. 3-bit
  arms in future sweeps (r11) are confirmatory measurements of this
  rule, not pick candidates — **r11b confirmed it** (Winnow-E4B Q3_K_S
  requant 0.408, footnote 5) and sharpened it with a provenance rule:
  requants from an already-quantized source must stay K-quant ≥ 4-bit
  (legacy Q4_0 and i-quants collapse under requant), and every
  deployment quant passes a determinism replay before ship (4 of 6
  requants flipped, including Q4_K_M).
- **Raw winner probabilities are overconfident everywhere** (ECE
  0.048–0.626) — calibration artifacts are fitted per tier (D15) before
  any confidence is exposed.
- **The ladder beats its best rung at home, and loses abroad.** Gated
  fusion over measured rows (engine → embedding → 2B decision arm)
  scores **0.867 on the suite at 753 ms** — +14.2 pp over the best
  single arm at 43 % of its latency (F23) — but the same gate on
  JevBench *lowers* the bridge's 0.6494 to 0.632, because the engine's
  out-of-domain calibration (ECE 0.402) poisons the routing decision
  (F24). Gates inherit their rung's calibration; validate them per
  domain before trusting them to route. With the 4B fork arm landed the
  full Board-A ladder is measurable end to end (engine → vtx → 4B
  fork). Re-joined on the fedora anchor
  (`results/fusion-jevbench-fork-anchor.md`): the oracle is **0.853**,
  but the best gated point is **0.688 — under the fork alone at
  0.766** — because the cheap rungs' accepted mass stays wrong even at
  strict gates (their raw probabilities are uncalibrated here). (The
  first join, `results/fusion-jevbench-fork.md`, used the superseded
  NAS-era arm: oracle 0.823, best gated 0.636 at 14.2 s mean.) On
  Board A the ladder is a cost policy, not an accuracy policy — but the
  anchor's sharper winner-probability channel (llm-rung ECE 0.070 vs
  the NAS arm's 0.142) buys the same best-point routing shape
  (engine 32 %, llm 68 %) at 3.2 s mean instead of 14.2 s, and the vtx
  rung earns no routing at the best point at all. Engine t=0.50 still
  answers 81 % of items for 0.532 at sub-millisecond rung cost. The
  0.867-at-home result stands — on Board B the gates separate
  because the rungs are calibrated there (F24's rule, twice confirmed).
  Study: `fusion_study.py`,
  outputs `results/fusion-*.md` in the decision-model README.
- **Adapter training is the measured road's next rung** — the target
  bar itself was built with a frozen base + small trained decision
  block. Feasibility on this (GPU-less) host, the recipe class, and a
  costed experiment ladder: `docs/TRAINING.md`.
- **Same weights, different runtime is not a free swap.** The ONNX
  export of Qwen3.5-2B driven by onnxruntime — with the decision-tree
  math ported exactly and greedy-parity-validated — scores 0.658 where
  llama.cpp scores 0.725 on the same weights, at ≥9× the latency
  (prefill-bound) and 2.6× the memory (F25). ONNX stays the product's
  model-rung format for portability; the decision arm stays on
  llama.cpp.

### r19 E2B quant-ladder speed reference (2026-10-05)

`llama-bench` pp400 (single-shot prompt processing over a 400-token
prompt, -t 12, -fa on, r 3, build `1537a0a`, quiet window) against the
footnote-11 letters arms. Prefill is compute-bound at this model size,
so t/s is nearly flat from 2.7 to 6 bpw — the accuracy column is what
separates rungs, and it (unlike every pre-r19 expectation) does not
separate them.

| quant | acc | p50 | pp400 t/s |
|---|---:|---:|---:|
| BF16 | 0.500 | 889ms | 63.7 |
| UD-IQ2_M (2.7 bpw) | 0.392 | 585ms | 194.4 |
| UD-IQ3_XXS | 0.300 | 640ms | 199.3 |
| Q3_K_S | 0.550 | 611ms | 190.1 |
| Q3_K_M | 0.500 | 658ms | 204.3 |
| UD-Q3_K_XL | 0.433 | 589ms | 206.7 |
| IQ4_XS | 0.450 | 676ms | 187.8 |
| IQ4_NL | 0.442 | 569ms | 231.3 |
| Q4_0 | 0.350 | 472ms | 225.5 |
| **QAT-Q4_0** | **0.808** | **423ms** | **230.8** |
| Q4_1 | 0.475 | 818ms | 118.2 |
| Q4_K_M | 0.425 | 672ms | 217.4 |
| Q5_K_S | 0.500 | 589ms | 197.4 |
| Q5_K_M | 0.483 | 627ms | 195.4 |
| UD-Q5_K_XL | 0.517 | 638ms | 193.8 |
| Q6_K | 0.525 | 657ms | 190.7 |
| UD-Q6_K_XL | 0.475 | 678ms | 164.3 |
| Q8_0 | 0.475 | 1100ms | 114.2 |
| UD-Q8_K_XL | 0.492 | 1046ms | 111.6 |
| QAT mobile mixed (torch bf16) | 0.800 | 7490ms | — (no bench) |

The official-weights QAT-Q4_0 is simultaneously the most accurate and
the fastest full-size row — sub-4-bit QAT costs nothing on either axis
the ladder measures, which is why it is the small-model operating
point. The tiny 0.8B arms benched at 147.8 (f16) and 701.5 t/s
(Q4_K_M).

### r20b Ternary-Bonsai-2-27B PTQ probe (2026-10-05)

Bounded probe, not a board row (one suite item, three server boots).
`Ternary-Bonsai-2-27B` is a post-training ternary quant ({−1,0,+1},
g128, blockwise Hadamard folded into weights, 1.72 bpw true) whose card
claims 98.2 % of FP16 quality; stock llama.cpp `1537a0a` rejects the
PTQ1_0 format, so this ran on the vendor fork (`PrismML-Eng/llama.cpp`,
0.2.0-dev `2459f68`, shallow clone, llama-server built -j8 in 79 s).
12 threads CPU, 27B ternary, 5.6 GB weights.

| probe | args | wall | first-token top-5 |
|---|---|---:|---|
| base | (default) | 73.8s | **A −0.005**, '' −7.0, The −7.37, Answer −8.09, D −8.43 |
| nothink | `--reasoning-budget 0` | 82.5s | identical to base (flag inert for this model) |
| lora | `--lora=…abliterate…gguf` | — | server rejects the flag at parse (`error: invalid argument`) |

Readings: (1) the PTQ1_0 artifact loads and produces an extremely
peaked, letter-first distribution — item A-0000's correct answer is
position 0, so the probed decision is **correct at p≈0.995**;
retention-consistent with the vendor claim, but one item is anecdote —
a full arm (~74 s × 480 readouts ≈ 10 h on this host) stays with the
Vulkan ladder (#35). (2) `--reasoning-budget 0` does not change the
computation at temperature 0 — the model is letter-first with or
without the thinking flag. (3) the fork inherits the
help-listed-but-rejected flag bug class (same signature as stock's
`--chat-template-file`): **`--lora` is untestable on this build**, so
the AtomicChat abliterate-LoRA remains unmeasured. F18's CPU
extrapolation (~20-25 min/request for 27B) was ~20× pessimistic for
this host: ternary 1.72-bpw prefill on 12 P-cores runs ~74 s/item.

## Composite deployment score

A single number for "which arm should a deployment pick", combining the
three axes that matter on this box. Every component is a measured
value; the weights are a stated choice, not a discovery.

```
A_trust = clamp(accuracy − ECE)                 # accuracy you can act on
Spd     = clamp(1 − log10(p50_ms) / 4)          # 1ms→1.00 10ms→0.75 100ms→0.50 1s→0.25
Res     = clamp((4.4 − log10(size_MiB)) / 2.2)  # 20MiB→1.00 2GiB→0.50 9GiB→0.20
Overall = 100 × (0.45·A_trust + 0.30·Spd + 0.25·Res)
```

Scored rows are arms whose artifacts are resident and measured on this
host (p50 values carry the standing co-tenant-load caveat; they
overstate latency, never understate it). **A composite below the
engine's accuracy cannot route decisions** regardless of its score —
sub-engine arms are screening rungs, and the ladder (F23/F24) is the
instrument that decides whether a rung earns traffic.

| System | Acc | ECE | A_trust | Spd | Res | **Overall** | Vision |
|---|---:|---:|---:|---:|---:|---:|---|
| OpenCodifier engine (relational over lexical) | 0.683 | 0.094 | 0.589 | 0.97 | 1.00 | **80.7** | no |
| VTX-JEV-3 lf2 (static embedding) | 0.242 | 0.136 | 0.106 | 0.99 | 1.00 | **59.5** | no |
| jebadiah-4b-v2 **Q3_K_S** | 0.792 | 0.087 | 0.705 | 0.15 | 0.50 | **48.7** | no |
| Suite ladder (simulated fusion, F23) | 0.867 | — | —¹ | 0.28 | —¹ | **47.4**¹ | no |
| jebadiah-4b-v2 Q8_0 | 0.800 | 0.051 | 0.749 | 0.14 | 0.34 | **46.5** | no |
| Qwen3.5-4B UD-Q4_K_XL (tree readout) | 0.800 | 0.074 | 0.726 | 0.08 | 0.43 | **45.9** | no |
| Winnow-E4B (letters, stock llama.cpp) | 0.842 | 0.109 | 0.733 | 0.17 | 0.23 | **43.8** | **yes** (mmproj BF16) |
| jebadiah-9b-v2 Q3_K_S | 0.817 | 0.099 | 0.718 | 0.07 | 0.36 | **43.2** | no |
| jebadiah-9b-v2 Q8_0 | 0.833 | 0.075 | 0.758 | 0.06 | 0.20 | **40.9** | no |

¹ A composition of measured rungs, not one artifact: its Spd/Res are the
slowest rung's, its ECE is per-rung — scored on accuracy and speed only,
flagged rather than hidden.

What the composite says (2026-10-03, Q3 sweep in flight):

- **The 4B class is the deployment sweet spot on CPU** — jebadiah-4b-v2
  Q3_K_S scores highest of all model arms at 1.98 GiB (half the 9B Q8_0
  footprint, ~4 pp under its accuracy). Among 9B-class arms, Q3_K_S
  strictly dominates Q8_0 (−2 pp accuracy, −55 % size).
- **The engine wins by construction of the weights** — and that is the
  intended reading: the architecture's first principle is that the
  zero-ML rung decides everything it can, and the measured tiers above
  price the model rungs it cannot.
- **VTX scores 59.5 and is still not deployable as a decider** (0.242
  accuracy, 0.0 at answer positions 4–5): speed and footprint cannot
  buy accuracy. Its role is screening/routing rung, pending position-
  bias mitigation (#76's instrument).
- **Winnow-E4B is the accuracy leader among deployable single models**
  (0.842) and the only vision-capable arm (mmproj ships in the repo);
  its cost is 7.46 GiB at Q8_0 and 2.2 s p50. Quantizing it is the open
  lever (below).
- Not scored (artifact not resident on the measuring host; board rows
  above carry their numbers): MiMo-V2.6-9B Q3_K_S (0.817 / ECE 0.048 /
  14.3 s — the D16 frontier pick), Qwen3.8-4B-Distill (0.767 / 0.057 /
  3.8 s, ours), gemma-3-4b-it (0.750 / 0.236 / 3.0 s, **vision yes**),
  Qwen3.5-2B Q8_K_XL (0.767 / 2.2 s).

## The target bar

The operator-set target: match the autotrust/JEV-27B numbers while being
**faster, smaller, and easier to adopt**. Their six-public-benchmark
table (their runs of all systems, 2026-09-26):

| system | JevBench | Kev | OpenJev text | Nimble | VitaminC | MASSIVE-en | Mean |
|---|---:|---:|---:|---:|---:|---:|---:|
| **autotrust/JEV-27B** (open) | **88.70** | 83.75 | **73.89** | **92.91** | 77.46 | **87.71** | **84.07** |
| TypeSafe Jev 1.13, hosted API (closed) | 87.18 | **85.52** | 72.96 | 91.84 | **78.46** | 87.14 | 83.85 |
| NeoHorse-Jev-4B | 75.73 | 81.92 | 58.74 | 87.23 | 77.13 | 85.43 | 77.70 |
| Open-Jev-9B | 77.13 | 77.87 | 65.39 | 80.50 | 68.28 | 84.86 | 75.67 |
| Kev-4B | 73.71 | 81.47 | 54.75 | 73.40 | 76.46 | 85.71 | 74.25 |
| Laya English | 55.82 | 61.30 | 40.07 | 45.04 | 78.63 | 68.57 | 58.24 |

Context that matters for the chase: JEV-27B is not a 27B retrain — it is
a frozen Qwen3.8-27B backbone plus a **108.9 M-parameter LoRA + 24-slot
decision head** (≈ 9.2 B200-hours), served as a one-token constrained
completion read back as log-probabilities, with per-kind temperature
calibration shipped in the artifact (measured ECE 0.0009, KL to its
teacher ≈ 0.017). That recipe is the confirmation of this project's
architecture, not a competitor to it: the decision rung is small,
detachable, calibrated, and the backbone is untouched — exactly the
`opencodifier-model` + `Calibration` seam design.

**The honest gap.** On the shared axis (JevBench public 231,
family-macro): JEV-27B 88.70 — our best row is now the 4B fork arm at
75.71 (a base Qwen3.5-4B through the constrained tree readout, no
decision tune, fedora anchor), then the 0.8B bridge at 63.78, the
20 MB VTX static table at 43.18, our engine at 40.11. The clean-host
anchor moved us from "20 points behind the bar" to 13 points behind
it, and the fork arm's own margin — 76.6 % plain against jev.page's
decision-tuned 4B at 80.5 % — is the measured price of skipping the
training recipe, not a harness difference (the bridge reproduced the
authors' own row at +0.9 pp; the fork delta is decision tuning plus
the anchor's unresolved-vintage caveat above). The road there, in
order of expected leverage: (1) a real candidate-conditioned
decision rung trained on typed-distribution distillation (the
`opencodifier-model` crate exists for exactly this; the corpus below is
Apache-2.0); (2) per-kind calibration fitted and versioned into the
artifact (the D15 pipeline is built and measured); (3) confidence-gated
escalation so the cheap rungs decide what they can and the trained rung
sees only the gray band (implemented; the field's own gray-band cascade
validates the shape). Speed, size, and adoptability are already on our
side of the ledger: 2.0 ms zero-ML decisions, 1.3 ms suite floor, no
GPU, no server, Apache-2.0, no API.

## Training-corpus landscape

- **`SargeDev/jev-distill-corpus-v3`** (Apache-2.0; Open-Jev stream CC0)
  — the Jev-1.13-distilled corpus both VTX-JEV-3 and autotrust/JEV-27B
  train on: paired typed judgments (Choice/Noul/Score with full teacher
  distributions) across chemistry, fact-checking, safety, security,
  code, dialogue routing. The obvious training source for our decision
  rung; license-compatible with this repo.
- **`SargeDev/jev-distill-corpus`** (the repo card is "Jev-Gate Student
  B") — a 1.1 M-parameter LoRA relevance judge on Qwen2.5-0.5B-Instruct,
  distilled from the same teacher: 86.4 % agreement with its teacher at
  43× lower latency, deployed in a gray-band cascade (vector recall →
  cross-encoder band gate → student judge → top-k, fail-open). The same
  design shape as our confidence-gated verification ladder, and a
  candidate future arm for the retrieval-rerank rungs.

## Methodology & integrity

- Deterministic arms replay every suite twice; `predictions_match` is a
  gate, not a statistic. All Board B decision arms are bit-deterministic;
  all Board A arms replay 231/231.
- No probability distribution is ever synthesized: a system that cannot
  produce one is scored label-only or incorrect, never renormalized into
  validity.
- Model weights never enter the repository; every result pins the
  SHA-256 of the artifact that produced it (`models.manifest.json`, D14).
- The benchmark suite is fixture data generated by a committed, seeded
  generator (`suite/generate_suite.py`); ground truth holds by
  construction.
- Raw run JSONs stay out of the repo tree and are regenerable from the
  pinned inputs.

## Sources

- autotrust/JEV-27B announcement and benchmark table —
  <https://huggingface.co/blog/autotrust/autotrustjev-27b-fast-calibrated-decisions-and-ful>
  (published 2026-09-27; runs dated 2026-09-26; JevBench column = public
  231 family-macro).
- TypeSafe Jev 1.13 — closed hosted API; numbers above are autotrust's
  runs of it (their published comparison), plus the JevBench authors'
  board row (86.6 %).
- VTX-JEV-3 — <https://huggingface.co/VTXAI/VTX-JEV-3> (Apache-2.0);
  JevBench row and suite rows are our runs (2026-09-29) through the
  vendor `inference.py` client, sha256s in the manifest.
- SargeDev/jev-distill-corpus(-v2/-v3) —
  <https://huggingface.co/SargeDev/jev-distill-corpus>.
- JevBench — <https://github.com/fstandhartinger/jevbench> (MIT),
  public split, harness commit `9ec6f15a`, dataset sha256
  `dc3995d8…`; methodology `benchmarks/decision-model/JEVBENCH.md`.
- Laya — Convai Innovations (`convaiinnovations/laya`); rows per the
  cited boards.

## Reproduction

```bash
cd benchmarks/decision-model
# Board B (any arm):
python3 runner/run_engine.py  --binary target/release/opencodifier --out "$RUNS/engine__relational-v1.json"
python3 runner/run_vtx.py     --model-dir /path/to/vtx-jev-3       --out "$RUNS/vtx__VTX-JEV-3-lf2.json"
python3 runner/summarize.py   --results-dir "$RUNS"
# Board A (official harness):
python3 runner/run_jevbench.py --arm engine   --tasks ... --out-dir "$RUNS/jevbench/engine"
python3 runner/run_jevbench.py --arm vtx      --tasks ... --out-dir "$RUNS/jevbench/vtx-v1"
python3 runner/run_jevbench.py --arm jev_native --tasks ... --out-dir "$RUNS/jevbench/bridge-v1"
```

Full commands, flags, and provenance requirements:
`benchmarks/decision-model/README.md` and
`benchmarks/decision-model/JEVBENCH.md`.
