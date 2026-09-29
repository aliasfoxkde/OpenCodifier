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
