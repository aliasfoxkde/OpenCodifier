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
| **fork: Qwen3.5-4B UD-Q4_K_XL tree mode** (D16 tier, fork-default T — base model, no decision tune; **fedora anchor**, 2026-10-02, default instructions; replicate bit-identical ²) | 231 | **0.7662** | **0.7571** | — ¹ | — ¹ | 2.28 s | 231/231 (labels only ¹) |

¹ The fork's tree mode ships the winner's mass only (label-only mapping,
D15), so no ECE/Brier is computable — the bridge row is the calibrated
reference. The anchor's latency is a clean-host client wall (p95
45.9 s carries sibling-session CUDA load, not arm time).
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

### Complete board (55 runs, 2026-09-25 → 2026-09-30)

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
| llama__jebadiah-9b-v2-Q8_0.json | 1.00 / 0.95 / 0.55 | 0.833 | 0.075 | 5626.3ms | yes |
| llama__qwen0.5b.json | 0.60 / 0.33 / 0.25 | 0.392 | 0.218 | 694.8ms | yes |
| llama__qwen1.5b.json | 0.90 / 0.45 / 0.40 | 0.583 | 0.191 | 1247.8ms | yes |
| llama__qwen3b.json | 0.90 / 0.75 / 0.38 | 0.675 | 0.263 | 2575.2ms | yes |
| vtx__VTX-JEV-3-fp32.json | 0.20 / 0.33 / 0.42 | 0.317 | 0.099 | 0.8ms | yes |
| vtx__VTX-JEV-3-lf2.json | 0.17 / 0.25 / 0.30 | 0.242 | 0.136 | 1.1ms | yes |

¹ Long-context variant: the derived `suite_long.json` (~3,681-token
contexts), answer-identical to the standard engine run — listed for
completeness, not a board row of the 120-item suite.
² Interface-mismatch row: the 0.8B tune is read at per-option verdict
slots (its trained interface) and was never trained on this harness's
readouts — the row measures the mismatch; its JevBench-native row is the
bridge above (0.6494).

What the board established (full findings catalog in REPORT.md):

- **The relational ceiling.** Every model arm ≤ 4B sits at or below 0.50
  on relational composition; only the 9B class crosses it — the MiMo MoE
  (0.525) and, above it, jebadiah-9b-v2 (0.55, the highest model arm) —
  and the engine's relational solver proves its way to **0.950 at 1.3 ms**,
  four orders of magnitude below the first model that crossed it.
- **Quantization is mostly free until it isn't.** Q3_K_S ↔ Q8_0 is
  within noise on 2B/4B; the collapses are sharp (int8 embedding rung,
  IQ2_XXS), not gradual.
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
