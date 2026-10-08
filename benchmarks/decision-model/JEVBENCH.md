# JevBench public-split run — methodology of record

Status: methodology pinned 2026-09-28 (task #39); the run itself waits for a
quiet host behind the params A/B and the native verdict-slot arm. Dataset and
harness verified against the official repo at commit
`9ec6f15a8773aaffbbb41e7d7e65a20599ab0cc3` (2026-09-28, MIT), cloned to
`/nas/Temp/work/oc-model-eval/jevbench-ref/`.

## What JevBench is

Benchmark Heaven's benchmark for Jev-class typed decision models: state in,
a typed answer with a probability for every option out. It is the closest
public analogue to what OpenCodifier's engine does, and the only external
suite with published rows for the exact model families we benchmark
(Jev-Style, Laya, decider, Qwen3.5-4B decision servers) — which makes it the
outside anchor our 120-item internal suite lacks.

- Board: 95 systems (v1.4.2.2), scored as the equal-weight harmonic mean of
  Intelligence (chance-corrected), Calibration, Speed, Cost.
- Items: frozen v1.2 set — **534 public + 308 sealed per system**. The
  official v1.4 score requires the sealed half; sealed text and golds are
  not published. We run the **public half only** and report raw accuracy /
  Brier / ECE, never a "JevBench Score".

## The public split (verified counts)

`datasets/public/{easy,hard,original}.jsonl` — exactly **231 items**:

| file | n | tiers |
|---|---:|---|
| easy.jsonl | 48 | intent/extraction-style checks |
| hard.jsonl | 111 | Opus 5 / GPT-5.6-authored, cross-reviewed |
| original.jsonl | 72 | v1.0-authored base set |

Item shape (one JSON object per line):

```json
{"id": "...", "family": "intent", "group": null,
 "state": "<text to evaluate>",
 "question": {"type": "choice|noul|score", "instructions": "...",
              "criteria": {label: description} | [descriptions] | null},
 "labels": ["<exact label strings>"],
 "expected": "<label>" | "yes"/"no" | <int level index>,
 "split": "public", "provenance": {...}}
```

Verified composition: **choice 139, noul 74 (binary yes/no), score 18
(ordinal levels "0".."3", scored by expected value)**; 18 families
(intent, extraction, long_policy, multi_hop, judge_hard, temporal_numeric,
fact, tool_selection, policy, ordinal, adequacy, routing, probability, trap,
ambiguous, tradeoff, adversarial, routing_hard); 2–6 labels (74 items have
2, 70 have 4, 56 have 5); mean chance accuracy **0.3176**. `group` pairs
paraphrase items (the harness reports paraphrase consistency over pairs).

## Harness (use theirs, not a port)

`python -m jevbench.cli run --tasks datasets/public/easy.jsonl,datasets/public/hard.jsonl,datasets/public/original.jsonl --adapter <name> --results <out.jsonl> --raw-dir <outside-repo>`

- **Adapter contract** (`jevbench/adapters/base.py`): implement
  `run(task) -> DecisionResult(adapter, ok, probs, probs_source, label,
  latency_s, usage, raw, request_body)`. `probs` is a dict over the exact
  label set; `probs_source` is `"native"` or `"verbalized"`; label-only
  systems set `label` and get accuracy but zero Calibration (a label is not
  a calibrated forecast).
- **Runner semantics** (`jevbench/runner.py`): serial, no retry, durable
  raw evidence (sha256 per item, raw dir must live outside the repo),
  budget ledger with per-run reserve. Failures count as incorrect; 422
  (over context limit) counts as wrong but not toward the infra stop rule;
  3 consecutive infra errors stop the run. Invalid distributions count as
  incorrect — never synthesized ("no invented calibration").
- **Scoring** (`jevbench/scoring.py` + `metrics.py`): argmax for
  choice/noul, expected value for score; multi-class Brier **sum** over the
  exact label set (binary convention `2·(p_yes − y)²`); top-label ECE, 10
  equal-width bins; macro accuracy per family.

Running our systems through *their* runner + *their* metrics is the whole
point: zero scoring drift against every published row. Same decision pattern
as the native verdict-slot arm (use the authors' shipped code, not a
reimplementation). The harness is MIT; raw evidence lands outside both repos.

## Mapping: JevBench item → OpenCodifier IR

| JevBench | OpenCodifier |
|---|---|
| `state` | `context` |
| `question.instructions` | `question` |
| `labels` + `question.criteria` | `candidates [{id: label, description: criterion}]` |
| `question.type: choice` | Choice question |
| `question.type: noul` | Boolean question (`false:`/`true:` criteria) |
| `question.type: score` | Score question (ordinal levels; EV scoring) |
| `expected` | `answer` |
| `family` | (our suite has `class`; JevBench families are finer) |

92 items have `labels` with no per-label `criteria` (easy-fact style) — those
candidates carry no description; adapters must not invent one. The engine arm
runs criteria-less candidates through the same candidate list unchanged.
35 items (multi_hop and friends) carry a **structured** `state` object; the
adapters serialize it deterministically (`json.dumps`) into the `state` text
channel, and the raw evidence records the exact rendered request.

## Arms (in run order)

1. **Jev-Style-0.8B-Decision-v3 Q4_K_M — the comparability bridge.** Their
   authors self-ran this exact model with the official harness on these 231
   items and published **64.1 % (148/231)**. Re-running it first through the
   official harness validates our adapter and environment: if we land near
   64.1 %, every other number we produce on this suite transfers to the
   published context. Run through the native verdict-slot readout
   (`run_jev_native.py` mapped onto JevBench items), since that is the
   model's trained interface — the same one its published row used.
2. **Engine arm (relational-v1 default stack)** — OpenCodifier `EngineHandle`
   on the same items via a local adapter (Choice/noul/score all exist in the
   IR). This is the row that matters: the deterministic ladder vs the
   Jev-class field, engine rung 0.683 on our suite, on external items now.
3. **Qwen3.5-4B UD-Q4_K_XL decision arm** — the D16 reference tier through
   the fork's tree mode (same config as the D16 run: T 1.400 shipped
   calibration). Direct competitor row: jev.page claims **80.5 % / 651 ms**
   for an llm-qwen3.5-4b decision server on the same 231 items.
   **Done (2026-10-02, `runs/jevbench/fork_4b-v1/`)**: 0.6667 (154/231),
   family-macro 0.6541, replay-deterministic 231/231, paraphrase
   agreement 0.861 (36 pairs), label-only mapping (the tree ships the
   winner's mass, D15) so no ECE/Brier; p50 15.5 s under a co-tenant
   storm (latency is context, not a speed row). The 13.8-point gap to
   jev.page's row is the price of a base model with no decision tune on
   a readout the model was never trained for — the bridge arm already
   proved the harness transfers at +0.9 pp.
4. **VTX-JEV-3 (VTXAI) — the static-embedding survey arm.** A
   Model2Vec-class 255,753×256 table (20.5 MB 2-bit LF2) with a
   position-gated attention pooler, distilled on
   `SargeDev/jev-distill-corpus-v3`, through the vendor `JevClient`
   (`run_jevbench.py --arm vtx`): the jev_native rendering (noul as
   no/yes, score levels as options) with the model's own cosine softmax,
   so Brier/ECE stay comparable. Adds a data point the field lacked: what
   a 20 MB static-embedding decision engine actually scores on this suite.
5. **Zero-shot NLI cross-encoder survey arm (PLAN 24j).**
   `MoritzLaurer/deberta-v3-base-zeroshot-v2.0`, its own published
   `onnx/model.onnx` (705 MB fp32, no conversion), through in-process
   ONNX Runtime (`run_jevbench.py --arm nli`, `--nli-dir`): the vtx
   rendering (every kind as option descriptions), premise =
   state + instructions, hypothesis = `"<description>."`, entailment
   index read from `config.json` `id2label`, entailment masses
   renormalized over the option set — the same renormalization the
   Rust serving contract (`opencodifier-model::nli`) ships. Known
   honest divergence, recorded in each run manifest: the serving
   boolean path uses one-hypothesis complement (IR booleans carry only
   text), while this adapter verbalizes per-option from JevBench
   criteria — what the zeroshot training objective does. Adds the data
   point the field lacked: what a generic entailment cross-encoder —
   no Jev training at all — scores on the suite.

Anchor rows published for the public split (sources: jev-style repo README;
JevBench RESULTS; jev.page):

| system | public acc | provenance |
|---|---:|---|
| hosted Jev (TypeSafe) | 86.6 % | official |
| llm-qwen3.5-4b (jev.page) | 80.5 % / 651 ms p50 | self-reported |
| Jev-Style-2B v3 | 73.6 % | self-run, GGUF F16 |
| decider-2b | 71.0 % | board |
| **Jev-Style-0.8B v3** | **64.1 % (148/231)** | **self-run, official harness — our bridge** |
| open-jev-zefan-2b | 64.5 % | board |
| Laya | 58.4 % | official |

## Comparability caveats (stated up front)

- **Not a board entry.** No sealed half → no v1.4 score, no rank. We report
  public-split accuracy / Brier / ECE / latency only. The public half is
  training-exposed by construction (Benchmark Heaven says so themselves);
  rows are context, not certification.
- **Self-run rows vs official rows.** The 0.8B/2B/jev.page numbers are
  self-runs; only some rows are official. The bridge row (#1 above) is what
  makes the comparison meaningful despite that.
- **Latency is not comparable across runtimes** (same caveat as
  NATIVE_VERDICT_ARM.md): their Speed axis uses adjusted production-load
  assumptions; we report raw client wall p50/p95 and compare only within
  our own runs.
- **Option order sensitivity is a known failure mode of this suite** (issue
  #40: one model dropped 72 % → 21 % on judge items with options
  reversed). Our adapters must present `labels` in file order, never
  re-ordered or alphabetized.
- **Zero-shot only.** No few-shot examples, no item-specific prompt tuning;
  that is the harness convention and the published rows' convention.

## Cost and acceptance

231 items × 3 arms; the 4B arm dominates (~minutes-hours CPU, same scale as
one D16 arm run); the 0.8B native arm is minutes; the engine arm is
milliseconds. Acceptance criteria:

1. **Bridge reproduction**: our 0.8B run within a few points of 64.1 %
   (same weights, same harness, same split). A large gap means our adapter
   or environment diverges — resolve before reading any other row.
2. **Schema discipline**: zero synthesized probabilities; every failed or
   invalid answer scored incorrect by their scorer, counts preserved in the
   raw evidence.
3. **Determinism**: greedy arms replay with identical predictions (their
   runner is serial no-retry; we add a replay pass in our own driver).
4. **Provenance**: harness commit, dataset sha256, model sha256s, adapter
   source recorded in each result JSON; raw evidence outside both repos.

## Results

| arm | accuracy | macro | ECE | Brier | p50 / p95 | determinism | probs |
|---|---:|---:|---:|---:|---|---|---|
| engine (relational-v1\|builtin-lexical-v2, 2026-10-06 D35 rerun) | 0.3766 | 0.4011 | 0.393 | 0.875 | 0.9 ms / 2.0 ms | 231/231 | native |
| **jev_native bridge (0.8B-v3 Q4_K_M)** | **0.6494** | 0.6378 | **0.080** | 0.425 | 6.72 s / 103.8 s | 231/231 (labels + probs) | native |
| vtx (VTX-JEV-3 LF2, vendor client) | 0.4113 | 0.4318 | 0.126 | 0.684 | 7.9 ms | 231/231 | native |
| nli-zeroshot (deberta-v3-base-zeroshot-v2.0, entailment renorm, 2026-10-08) | 0.5325 | 0.4904 | 0.232 | 0.648 | 0.57 s / 6.9 s | 231/231 (labels + probs) | native |
| engine + 4B rung, fusion-v2 gate (2026-10-05) | 0.5584 | 0.5622 | 0.335 | 0.734 | 1.2 ms / 0.73 s | 231/231 | native |
| engine + 4B rung, proofs-only posture (2026-10-05) | **0.6883** | 0.6622 | 0.171 | 0.431 | 0.58 s / 14.0 s | 231/231 | native |
| engine + 4B rung, proofs-only posture (2026-10-06 D35 rerun) | 0.6840 | 0.6593 | 0.170 | 0.436 | 0.82 s / 18.7 s | 231/231 | native |

The engine row was re-measured on the shipped build
(`runs/jevbench/engine-only-fedora-v1/`, archived on the compute host):
0.3766 reproduced byte-identically (187 ok / 44 abstains), and the row's
missing Speed cell is filled — p50 0.7 ms, p95 1.6 ms, the fastest arm
on the suite by two orders. The D35 confidence bound re-measures it
again with predictions byte-identical (`engine-only-fedora-v2-d35/`:
same 0.3766, same 187/44, full replay) while the confidence columns
move as designed: the 8 rows that reported p = 1.0 — BM25 softmax
saturation, the maximum-confidence-wrong mechanism D35 removes — now
report p ≤ 0.99, and the calibration cells improve for free
(ECE 0.402 → 0.393, Brier 0.890 → 0.875) without any calibration being
fit. The §45 focus-extraction A/B also reproduces answer-identical on
the D35 build (acc 0.683 / ECE 0.095 with and without
`--focus-budget 512` on the padded suite).

The two full-system rows (task #105, contract and decomposition in
`results/jevbench-fullsystem-fedora.md`): same 4B rung, same weights —
the only difference is the gate posture, worth +13.0 pp. The
in-domain-tuned fusion gate keeps 68.8 % of items at 0.434 accuracy
while escalating 0.833 to the rung (F24's warning, measured OOD); the
structural posture ties the post-hoc cascade bound a priori and pays
for 12 honest abstentions in the accuracy column.

**Proofs-posture D35 delta (2026-10-06, task #115).** Re-running the
proofs posture on the D35 build
(`runs/jevbench/proofs-only-fedora-v2-d35/`) moves it −1 item,
0.6883 → 0.6840. The mechanism is exactly the saturation repair: D35
caps the BM25 softmax at p ≤ 0.99, so the 8 rows that previously
reported p = 1.0 and were engine-accepted under `min_confidence: 1.0`
now escalate to the rung. The per-item diff over the whole suite finds
exactly one flip: 4 of the 8 saturated items were correct at p = 1.0,
the rung confirms 3 of them (probability-03, opus-c long_policy-04/11)
and loses 1 (hard-opus-a-long_policy-19, engine-right → rung-wrong at
14.2 s); the 4 previously-wrong saturated items stay wrong — the rung
confirms them wrong too, consistent with the gate-refit finding that
long_policy is rung-hostile. The "escalation converts engine misfires"
hypothesis is refuted on this suite: the 2B rung is not better than the
lexical engine on the items the engine could not verify. What D35 buys
here is calibrated honesty — the posture no longer sells maximum
confidence on items it cannot prove — at the cost of one formerly-lucky
keep, plus latency from 11 added escalations (p50 0.58 → 0.82 s, p95
14.0 → 18.7 s).

**Gate-refit negative result (2026-10-06, task #114).** The obvious
repair — refit the fusion thresholds on the OOD distribution — is
measured and closed: the offline winner-prob sweep
(`results/jevbench-fusion-v3-gate-refit.md`) is monotone to the grid
edge, so the OOD-optimal gate IS the proofs posture. Split-half
discipline agrees (fit 0.6897 / held-out 0.6870 at the boundary). The
per-kind view finds no pocket either: at every decision kind and every
confidence band the rung beats the engine's accepted items — the
lexical engine's winner-probability is not an OOD-informative gate
feature (on choice it is anti-correlated: the p≈1.0 band converts at
0.235 while p∈[0.5,0.6) converts at 0.769). What remains open is a
better gate *feature* (rule/extractor provenance rather than softmax
confidence), not a better threshold.

The vtx row (added 2026-09-29): VTX-JEV-3 through its vendor `JevClient`
with the jev_native rendering — a 20.5 MB 2-bit static table scoring
3.5 pp above the engine and ~23 pp below the 0.8B bridge, fully
deterministic. Its sub-millisecond vendor latency claim reproduces
(0.79 ms vendor-side p50 per suite decision); its accuracy sits at the
engine's level, not the Jev-class field's. Family profile: strongest on
fact 0.750, policy 0.750, ambiguous 0.571, routing_hard 0.600,
temporal_numeric 0.467; weakest on long_policy 0.105, multi_hop 0.167,
routing 0.250, ordinal 0.333 — static similarity guesses where the
engine abstains and cannot compose. Archived `runs/jevbench/vtx-v1/`.

Bridge vs acceptance: (1) reproduction — 150/231 = 64.9 % vs the
published 64.1 % (148/231), +0.9 pp, PASS; (2) schema discipline —
`probability_sources: ["native"]`, zero synthesized rows, zero
renormalizations, PASS; (3) determinism — full replay matches on
predicted labels and probability vectors, PASS; (4) provenance —
manifest records harness commit `9ec6f15`, dataset sha256
`dc3995d8…`, model sha256 `0a19bc29…`, PASS. Raw evidence archived at
`/nas/Temp/work/oc-model-eval/runs/jevbench/bridge-v1/` (results,
replay, ledger, manifest; tmpfs originals volatile).

Bridge family profile (accuracy): tool_selection 1.000, fact 1.000,
intent 0.917, ordinal 0.917, routing 0.917, extraction 0.875,
adequacy 0.833, routing_hard 0.800, policy 0.750, trap 0.750,
judge_hard 0.529, probability 0.500, tradeoff 0.500, adversarial
0.333, temporal_numeric 0.333, multi_hop 0.278, ambiguous 0.143,
long_policy 0.105 — the mirror image of the engine's abstention-first
profile on the same items.

## What OpenCodifier buys

- **The external anchor.** Every number in REPORT.md so far is internal
  (our suite, our seeds). JevBench's 231 public items are the first
  third-party ground truth with published rows for comparable systems —
  the engine-vs-field question ("is the deterministic ladder actually
  competitive?") gets an answer measured on someone else's items.
- **Typed-surface evidence.** choice + noul + score with EV scoring is
  exactly the IR's three decision types; JevBench is the only public suite
  that exercises all three the way the spec (§2) defines them.
- **Calibration context.** Their Calibration axis (ECE + fidelity to gold
  distributions) and the published ECE of Jev-class models give D15's
  temperature artifacts an external reference point.

## Wire facts verified in smoke (2026-09-28, `runner/run_jevbench.py --arm engine`)

Facts the engine adapter depends on, each verified against the live engine
or the harness source, not assumed:

- **Native `state` is strict**: `{"text": ..., "facts": {}}` — an omitted
  `facts` field is `schema.invalid_value` (the native codec rejects unknown
  *and* missing fields by design).
- **Native score `levels` are structs**: `[{"label": "0"}, ...]`; level
  descriptions are not representable (the IR's `ScoreLevel` carries a label
  only), so the JevBench rubric rides in the question text (`Rubric — 0:
  ...; 1: ...` — the item's own content, never invented).
- **Answer distributions** serialize as `{"entries": [{"key": k,
  "probability": p}]}`; a full-probs emission requires exact label-set
  coverage and an ~unit sum, else the item degrades to label-only.
- **The Boolean answer's `probability` is the confidence of the DECIDED
  value** (raw evidence: `value=false, probability=0.64` = p(no) 0.64), so
  p(yes) = 1 − p for a false answer. This contradicts the Jev wire format's
  p(yes) convention — the Jev codec translates; the native codec does not.
- **Abstention maps to incorrect — and to their 422 refusal bucket.** The
  engine's policy (min_confidence 0.8, abstain_below 0.5) abstains readily
  on 2-option items; their runner scores a failed answer incorrect, which
  is the honest mapping — an abstaining engine is a system with no answer,
  not a half-right one. But the refusal must carry `status_code=422`:
  their runner counts a not-ok record as an infrastructure error unless it
  is a 422 (their comment: "a 422 is the system refusing this input, not
  an outage"), and 3 consecutive infra errors stop the run. The engine's
  abstain pattern is deterministic, so both engine runs stopped at exactly
  item 186 (3 consecutive abstains) before the classification — an
  item-triggered stop that masqueraded as a load wedge twice.
- **Their `Runner` opens results files exclusively** (`open("x")`) and
  keeps raw evidence keyed by sha256 of the task id — reruns need a fresh
  out-dir; the ledger enforces budget from reservations, so local zero-tariff
  arms pass `default_reserve_usd=0.0` rather than a pretend cap.

## Sources

- Repo: <https://github.com/fstandhartinger/jevbench> (MIT), commit
  `9ec6f15a8773aaffbbb41e7d7e65a20599ab0cc3`; datasets verified locally.
- Method: `docs/METHOD-v1.4.md` in that repo; board:
  <https://benchmarkheaven.com/jev-models>.
- Published rows: jev-style repo README (0.8B 64.1 %, 2B 73.6 %, Laya
  58.4 %, hosted Jev 86.6 %); jev.page (llm-qwen3.5-4b 80.5 % / 651 ms);
  JevBench RESULTS (decider-2b 71.0 %, open-jev-zefan-2b 64.5 %).
