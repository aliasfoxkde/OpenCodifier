# Board submission prep — what a row on each surface would take

Status: research artifact for task #107 (benchmark-dominance program). No
submission has been made, no accounts created, no issue filed. Every external
claim below carries the URL it was verified against; anything not verified
first-hand is marked **unverified**. Arithmetic we derived ourselves is labeled
*own arithmetic* and is never an official number. Companion reading:
`docs/BENCHMARKS.md` (the four eval surfaces; one number per surface, never
universal) and `benchmarks/decision-model/JEVBENCH.md` (harness contract and
the #105 numbers this checklist is grounded in).

## 0. Verdicts at a glance

| surface | submission path | local-first eligible? | what a row would be |
|---|---|---|---|
| Benchmark Heaven "Jev models" board | web form + GitHub issue; **maintainers run your system themselves** | **Yes** — CPU-container rows exist; open code is run on their pods | official, ranked row of a self-hosted CPU system |
| jevbench.xyz | **none** — first-party archive; "register interest" private evaluation only | n/a — they price only what they can verify | nothing, unless they run it |
| jabr | **no leaderboard/portal** — public CC0 harness; results are maintained in-repo by its owner | yes (all current rows except Jev are local) | self-run numbers on a public harness — no third-party tier unless the owner adds a backend |
| Our 231 public-split runs | none — self-run | n/a | "Not a board entry." Context, never certification |

---

## 1. Benchmark Heaven "Jev models" board

### 1.1 What it is (and which version)

- Live board at <https://benchmarkheaven.com/jev-models>, fetched at version
  **v1.5.7**: 117 roster systems / 111 ranked / 63 Jev-class; leader Cygnet
  73.7; Jev 1.13.0 third at 72.1. Full protocol: **1,624 decisions per system
  = 904 open + 720 sealed**, serial.
- The repo README (<https://github.com/fstandhartinger/jevbench>) still fronts
  **v1.4.2.2** (95 systems / 91 ranked). The board moves faster than the repo
  page.
- `docs/RESEARCH.md` §14 line 1585 records "JevBench **v1.6.0** | 1,500
  decisions self-hosted / 600 hosted; 92 systems | … best ≤4B Cap 65.4" —
  **unverified**: v1.6.0 appears on no page we could fetch, and the live
  decision count (1,624) and roster (117) disagree with it. Treat the §14
  row as stale until re-checked at submission time.

### 1.2 How a system gets on it — two verified routes

**Route A — the web form.** <https://benchmarkheaven.com/submit>:
"Anyone can submit; no account needed." Required: a name, "how to reach the
model (a GitHub link, a Hugging Face link or a public API URL, at least
one)", your email, and the benchmarks you want. An API key is optional and
"encrypted the moment we receive it". Free regular queue is evaluated in
order received; a paid fast lane returns results within 48 hours (JevBench
and ImageJevBench only). "Top-10 models are re-evaluated with every release;
the rest of the leaderboard is re-evaluated less often."

**Route B — a GitHub issue in the JevBench repo.** Board FAQ
(<https://benchmarkheaven.com/jev-models>): "Open an issue in the JevBench
repository with a reproducible endpoint or runnable code, the exact model and
licence, and whether public JevBench items were used during development. New
entrants use the same frozen harness and appear in a new version or a
disclosed roster addendum." The issues list
(<https://github.com/fstandhartinger/jevbench/issues?q=submission>) shows 47
intake issues with title conventions like `[bench request]: X (offline
Hugging Face artifact, …)`, `Model evaluation request: X (runnable offline
bundle)`, `API submission: …`. Two working templates:

- **Offline artifact** — issue #159 (OpenJev-4B,
  <https://github.com/fstandhartinger/jevbench/issues/159>): structured
  candidate table (public model name; weights with pinned HF revision;
  inference + training code with commit; base and adaptation; licence;
  interface; probability source; temperature with `fitted: false`; requested
  execution; modality), repro commands, SHA-256 manifest, the sentence "This
  is an offline artifact submission. Please download the pinned weights and
  code and run the evaluation in your own environment. No author-hosted
  endpoint or API credential is required.", and an explicit request to "run
  the maintainer-controlled v1.5 evaluation, including its sealed portion".
- **Runnable offline bundle** — issue #190 (decision-stack-0.8b-v0.1,
  <https://github.com/fstandhartinger/jevbench/issues/190>): adds artifact
  SHA-256s, a serving description, pricing honesty ("no public bookable API
  price or measured official serial latency. Please apply the method's
  disclosed base-model reference pricing and offline/self-hosted latency
  rules; we claim no free cost or speed advantage"), and a development-exposure
  disclosure ("The older public 231-case JevBench snapshot (`bb05a335`) was
  previously evaluated for the parent and prior candidates; this is disclosed
  development exposure, not an untouched official test"), closing with "I do
  not yet claim an official composite or rank."

**Who actually runs it: they do.** This is the part that decides feasibility
for a local system. The board's per-row provenance column shows their own
execution environments ("GPU pod (H100/…)", "CPU container", "CPU (shared
host)", "author demo endpoint", "hosted API"), and their sealed-item text is
sent to evaluated services *without answers*. Open-code systems are
downloaded and run by the maintainers on their own pods — no API flag, no
latency adjustment. Submitter-operated demo/hosted endpoints get the "API"
flag ("the operator's endpoint received sealed item text during evaluation,
without answers") plus a latency penalty. Precedent that they mean it: v1.2.7
jqv — they stopped sending held-out items to that submitter-operated endpoint
(partial row, not ranked, "because its endpoint runs on the submitter's own
machine") and later re-ran it in full on their own GPU once the serving code
went public (repo revision log,
<https://github.com/fstandhartinger/jevbench>). A submitter-side confirmation
of the same mechanics: "Sealed items are never seen by any submitter"
(<https://huggingface.co/wfzyx/von>).

Practically: the sealed-half requirement costs us nothing to satisfy — it
means we hand them runnable code and they run the whole 1,624-decision
protocol, sealed half included, in their own environment.

### 1.3 Scoring, verified (board method notes, v1.5.7)

- **Score = 4 / (1/I + 1/C + 1/S + 1/K)** — the equal-weight harmonic mean of
  Intelligence, Calibration, Speed, Cost, each axis 0–100, **with any axis
  below 50 multiplied by (axis/50)²**. The board offers weighting options;
  option A (default) is equal 25:25:25:25.
- **Capability Score = mean(Intelligence, Calibration)** — the separate view
  above the Jev-class divider.
- **Jev-class eligibility**: cost ≤ 2× Jev ($0.065 per 1,000 decisions) and
  median latency ≤ 2× Jev (≤ 1.23 s).
- **Speed = 100 − 20·log10(seconds ÷ 0.1)** averaged over p50 and p95, with
  "Self-hosted and demo endpoints get the published ×2 plus 0.15-second
  adjustment".
- **Cost = 100 − 30·log10(cost ÷ $0.001)** in USD per 1,000 decisions.
- **Intelligence** is chance-corrected, 50% open + 50% sealed, with an
  open-minus-sealed gap penalty once the gap exceeds field median G_med (5.2)
  + 8.
- **Calibration** is typed: Choice ECE/TVD, Noul ECE + Brier, Score normalized
  RPS + top-level ECE.

### 1.4 Cost for a local system is never zero

Board pricing notes (<https://benchmarkheaven.com/jev-models>): systems
without a public tariff — open weights, author demos, models they ran
themselves — "are priced as if a large inference provider hosted them: the
list price of the same weights, or the nearest larger sibling or size class."
They explicitly do **not** use per-minute GPU rental or their own CPU time,
the scoring price is never below the market reference price of the base
model, prices must be public/bookable/stable for 30 days, and "a system
without any eligible public, bookable price is listed as unpriced — no Cost
axis and no score." A v1.5 "measured-input proxy" basis exists for local CPU
rows (Needle 3 $0.019, open-jev-deberta-v3-large $0.0056, GLiNER2 $0.0028,
JevAct $0.011 per 1,000 decisions) — all of those are models with token
inputs. Contrast: jabr's own results table marks local models "free (local)"
(<https://github.com/jabr/classifier-benchmark>) — a convention Benchmark
Heaven does *not* share. Any "our decisions are free" claim is therefore
wrong on this board by construction.

### 1.5 Local/offline eligibility: yes

Verified by row provenance on the live board: "CPU container" and "CPU
(shared host)" and "local CPU" rows exist (GLiNER2 large, SimpleJev
Qwen3.5-0.8B, open-jev-deberta-v3-large, Needle 3 "(2-bit, local CPU)", Bosun
v3.1 0.6B, Laya). Systems outside Jev-class limits still get scored, ranked
rows — GLiNER2 large holds an official rank while appearing below the
Capability divider. Being local and slow costs Speed and Jev-class
membership; it does not cost the row.

### 1.6 Coverage and rank rules (why full-protocol completion is the gate)

From the board's method notes and limits: "A type a system does not support
is excluded, never scored zero; only full-coverage systems are ranked."
Recent rank-kill precedents: mica-v01-4b stopped at 1,088/1,624 by its
refusal policy → "not eligible for an official rank"; Decision-4B reached
1,550/1,624 with 74 context overflows at 2,048 tokens → no rank; SimpleJev
677/1,624 → "the missing ones count wrong and the row is not ranked." Our
known engine hazard — deterministic abstain patterns tripping the 3-consecutive-
infra-error stop before the 422 classification existed (stalled at item 186
twice; JEVBENCH.md "Wire facts") — is exactly this failure mode and must be
retired before any submission.

### 1.7 Precedent rows: official runs, not self-runs

- **decider-2b** — added v1.2.8: "the rest ran on our RunPod GPUs" (repo
  revision log). Live row: official, #33, "GPU pod (H100)", 0.18 s, $0.015,
  I 42.3 / C 71.5 / S 94.4 / K 64.9, Score 45.1
  (<https://benchmarkheaven.com/jev-models>).
- **Open-Jev 2B/9B (Zefan Cai)** — added v1.2.15: "Both pinned Apache-2.0
  adapter packages ran all 534 frozen decisions through the author's MIT
  server on our RunPod H100", including a normalized-text audit that found
  "no public JevBench state or instruction in the 79,116-row public training
  projection" (<https://github.com/fstandhartinger/jevbench>). Live 2B row:
  official, #69, outside Jev-class (cost 5.3× Jev), 1.0 s, $0.170, Score 9.1.

Both anchors in JEVBENCH.md's table marked "board" (decider-2b 71.0%,
open-jev-zefan-2b 64.5% on the public split) are Benchmark Heaven's own
public-half measurements of author-published artifacts. Neither author
self-ran their way onto the board.

### 1.8 Eligibility verdict for OpenCodifier

**Eligible — with four conditions.**

1. **Full coverage is mandatory for a rank.** Choice/Noul/Score all exist in
   our IR and all three are exercised by the 231-split run, so the surface
   condition is met; the risk is the stop rule (§1.6).
2. **Cost is a real unknown for a zero-ML runtime.** The engine arm has no
   weights, hence no "list price of the same weights"; the fusion arm's 4B
   rung does have a price basis. Either we ask the maintainers for a
   measured-input-proxy ruling (their v1.5 basis, §1.4) or the engine row is
   unpriced — which means no Cost axis and no score at all.
3. **Jev-class membership is probably out of reach on adjusted latency**
   (§7, own arithmetic), so the realistic outcome is a scored, ranked row
   below the Capability divider — which is still an official row.
4. **The public 231 items were used in developing our arms** (documented in
   JEVBENCH.md). This must be disclosed in the issue; it is an accepted
   disclosure shape — Von's model card states "The calibration map was fitted
   on the 231 public JevBench items" and holds board rows
   (<https://huggingface.co/wfzyx/von>) — but it means our row cannot be
   presented as development-naive.

---

## 2. jevbench.xyz — no submission surface

<https://jevbench.xyz/> and <https://jevbench.xyz/methodology>: a
first-party, single-vendor archive. Archive 001 is Banking77 (3,080 cases,
Jev 80.3%, p50 310 ms, est. $0.222), labeled "FIRST-PARTY = RUN BY JEVBENCH,
SO IT CARRIES NO EVIDENCE TIER AND IS NOT INDEPENDENT EVIDENCE." Collected
runs carry evidence tiers A/B/C/D; their own runs are tierless. The only
outsider path is "Private evaluation … Register interest without uploading
customer data. No payment, no public result." Their cost model cannot price
self-hosted systems at all ("TF-IDF + logistic regression: No verified
pricing … Needs a separate cost model before it can appear in the
calculator").

**Verdict:** there is nothing to submit to. A row there would be their run
of us, obtained only by registering interest, and it would not be independent
evidence even then. Not a target for the dominance program beyond monitoring.

---

## 3. jabr — public harness, no leaderboard

Primary source verified: <https://github.com/jabr/classifier-benchmark>
(CC0 1.0 "public domain. The test cases may be reused freely").

- Three primitives — **choice / noul / score** — matching JevBench's, so our
  IR covers the types.
- Two hash-locked suites: **v1 = 8 tasks / 78 cases**, **v2 = 49 tasks / 866
  cases** (TOML, `cases/v1.toml` / `cases/v2.toml`, hashes in
  `cases/hashes.json`, `just validate`). Note the count: the repo README says
  866; the Von model card and EveryDev both say 869
  (<https://huggingface.co/wfzyx/von>,
  <https://www.everydev.ai/tools/von-decision-model>) — count drift across
  revisions, unresolved from public sources.
- The harness is public and pluggable: `uv run python -m bench.run --backend
  <name> --suite v2 --device cpu --out results/run.json`, backends are
  adapters under `bench/`, `--device mps|cpu|cuda`.
- Headline v2 (Apple MPS): Jev 0.964 micro / **0.966 macro**; Von 1.1
  0.724 / **0.720**; GLiNER2 0.688 / 0.684; Laya 0.585 / 0.583; Jev ~$0.000014
  per call, local models "free (local)". `docs/RESEARCH.md` line 1586 records
  "Jev **0.966** vs best open **0.704** (Von); GLiNER2 0.698; Laya 0.583" —
  Jev and Laya match the current README exactly; Von 0.704 and GLiNER2 0.698
  match an earlier revision of the repo's results (current README: Von 0.720,
  GLiNER2 0.684). RESEARCH's numbers are one revision behind, not wrong about
  the shape: **Jev leads the open field on jabr by ~25 points.**
- **No submission path.** No leaderboard, no portal, no documented intake for
  new backends — the models under test (Von, GLiNER2, GLiNER2.5-Decide, Laya,
  jeff, Jev) were all added by the repo owner, and results live in
  `results/benchmark.md` in-repo. Whether the owner accepts outside backend
  PRs is **unverified** (the README documents no contribution policy).
- Because all cases are public and "models are free to incorporate the test
  cases into training data," there is no development-exposure problem here.
- Any numbers we produce ourselves are self-run on a public harness — same
  tier as our JevBench public-split runs. `docs/RESEARCH.md` line 1746's
  condition ("jabr run if the harness is public") is now met on the
  harness-existence half; what remains is a decision to write the backend
  adapter, not a missing harness.

**Verdict:** technically runnable today (CPU, TOML, CC0); it yields
*our* numbers, not a third-party measurement, unless the owner adds our
backend. Worth doing as an OOD evidence row, never as a dominance claim.

---

## 4. Our own 231 public-split runs — "Not a board entry"

All JEVBENCH.md results (including proofs-posture **0.6883** accuracy, macro
0.6622, ECE 0.171, Brier 0.431, p50 0.58 s / p95 14.0 s, 231/231 replay
determinism, native probs) are self-runs of the public half. JEVBENCH.md's
own caveat governs: no sealed half → no score, no rank; the public half is
training-exposed by construction. The only legitimate public-split comparison
is plain accuracy against the other published public-half rows: 0.6883 sits
between decider-2b 0.710 and open-jev-zefan-2b 0.645, above the 0.8B bridge
0.6494 and Laya 0.584, below hosted Jev 0.866
(<https://benchmarkheaven.com/jev-models> published-half rows; anchors table
in JEVBENCH.md).

---

## 5. Submission artifact checklist (Benchmark Heaven, offline route)

Grounded in the #105 numbers and the #159/#190 issue templates. The realistic
vehicle is a **runnable offline bundle** issue (#190 shape), since our
artifact is code plus a recipe, not weights.

| # | artifact | status |
|---|---|---|
| 1 | Public, cloneable repo link (form and FAQ both require a GitHub/HF link) | **operator decision** — repo/account/publishing call |
| 2 | Runnable recipe: build + serve + the exact adapter command for their runner | adapter exists locally (`benchmarks/decision-model/runner/`); needs extraction into a public, standalone form |
| 3 | Pinned commit SHA of the runtime + SHA-256 of the recipe bundle | trivial once #1–2 exist |
| 4 | Licence statement (Apache-2.0) | have it |
| 5 | Interface description: strict native wire (`state {text, facts}`, score levels as structs, full-probs entries, Boolean probability = confidence of the decided value) | documented in JEVBENCH.md "Wire facts"; must be restated in the issue |
| 6 | Probability source: `probs_source="native"` over the exact label set, ~unit sum, zero synthesized distributions | verified in the #105 bridge/arms; must hold on *their* run |
| 7 | Abstention → `status_code=422` (refusal, not infra) so the 3-consecutive-error stop cannot fire | **must be re-verified end-to-end on the full 1,624-decision protocol before submitting** — the item-186 stall is the known hazard (JEVBENCH.md) |
| 8 | Full-protocol completion: all 1,624 decisions, all three types | untestable locally (no sealed half); the 422 fix plus a public-split dry run is the best available proxy |
| 9 | Execution environment statement: CPU-only, no GPU, no weight download for the engine arm (novel — their intake assumes weights or an endpoint) | write it; state RAM/cores assumed |
| 10 | Truthful development-exposure disclosure: the public 231 items were used in developing/evaluating the engine and both gates (#190's language is the model) | facts on hand; wording is an operator call |
| 11 | Self-run diagnostics explicitly labeled *not official* (0.6883 / ECE 0.171 / p50 0.58 s on the 231 split), with repro commands | have it |
| 12 | Pricing statement: which cost basis we are asking them to apply — measured-input proxy vs base-model reference (rung weights) vs unpriced | **operator decision + maintainer question** |
| 13 | "I do not yet claim an official composite or rank." | closing sentence of the issue |
| 14 | Contact email + submitter identity for the form/issue | **operator decision** |

### Expected axis exposure (own arithmetic — illustrative only, not official)

Applying the board's published self-hosted rule (×2 + 0.15 s) to our raw
public-split latencies:

- **Proofs-posture**: p50 0.58 s → 1.31 s; p95 14.0 s → 28.15 s. The
  Jev-class median-latency cap is ≤ 1.23 s, so this posture is marginally
  outside Jev-class on our hardware's numbers (whether it is outside on
  *their* hardware is their measurement, not ours). Illustrative Speed:
  100−20·log10(1.31/0.1) = 77.7 (p50), 100−20·log10(28.15/0.1) = 51.0 (p95),
  mean ≈ **64.3**.
- **Fusion-v2 gate**: p50 1.2 ms → 0.152 s; p95 0.73 s → 1.61 s.
  Illustrative Speed ≈ **86.1** — the same accuracy/ECE trade in the other
  direction (0.5584 vs 0.6883 on the public split). Speed is the axis where
  posture choice moves the board row most.
- **Engine-only**: p50 2.0 ms → 0.154 s; p95 not recorded for that arm on the
  public split → Speed not computable without a new run.
- **Intelligence and Calibration cannot be estimated at all** from a pooled
  public-split accuracy and ECE: the board's axes are chance-corrected,
  per-type (Choice/Noul/Score), half-split (open/sealed), and gap-penalized.
  Anyone quoting "0.6883 → Intelligence ≈ X" is inventing a number.
- **Cost**: no defensible number exists for a zero-ML engine without a
  maintainer ruling (§1.4); "zero" is false on this board by construction.

---

## 6. Open questions needing the operator

1. **Publishing the artifact.** Submission requires a public GitHub (or HF)
   link. Which repo, under which account/org, and is a standalone
   submission bundle (adapter + recipe + LICENSE) acceptable to publish
   alongside the workspace? Nothing in the workspace is currently published.
2. **Submitter identity.** Which email address and GitHub identity files the
   issue/form (no credentials in-repo, per house rules).
3. **Which system identity.** One row per system: proofs-posture (accuracy
   0.6883 public, Speed-exposed tail), fusion-v2 (0.5584, Speed ≈ 86.1), or
   engine-only (no weights → the cleanest cost story and the hardest one).
   Two rows are precedented (variant candidates from one author in #159's
   family) but each needs its own full run.
4. **Cost basis.** Ask the maintainers how a zero-ML deterministic runtime is
   priced (measured-input proxy? base-model reference via the 4B rung?),
   before submitting — otherwise the row risks "unpriced: no Cost axis and
   no score."
5. **Free queue vs paid fast lane** (48 h, JevBench only).
6. **Sealed-text handling.** Their run puts sealed item text inside our
   running code on their pod. Confirm no licensing posture in the workspace
   objects to that.
7. **p95 exposure.** Accept 14.0 s (→ 28.15 s adjusted) on proofs-posture, or
   attack the rung tail first? This is the single largest controllable axis
   loss.
8. **jabr.** Write the `bench/` backend adapter for our own CPU numbers now,
   or first ask the owner whether outside backends are accepted upstream
   (unverified)?

---

## 7. Claims we would not make (carried over, and why)

- **No "we beat Jev" from anything we can run.** Jev's 86.6% public-split
  anchor and its board row (#3, 72.1) are both out of reach of a
  public-half-only run; the sealed half is the point.
- **No composite, score, or rank derived from our own runs.** JEVBENCH.md:
  "Not a board entry." The only official number is the one Benchmark Heaven
  produces after running us.
- **No cross-surface number.** One number per surface (BENCHMARKS.md): the
  231-split accuracy, a future board Score, a jabr macro, and jevbench.xyz's
  archive entries are four different quantities and never average.
- **No Jev-class claim from our own adjusted arithmetic.** §7's numbers are
  our hardware run through their published adjustment formula; their
  measurement on their pods is the only one that counts.
- **No zero-cost claim.** Benchmark Heaven prices local systems at a
  reference tariff; "free (local)" is jabr's convention, not theirs.
- **No cross-runtime latency comparison** without their adjustment (JEVBENCH
  caveat), and no calibration claim from the pooled ECE 0.171 (no per-type,
  no half split).
- **No development-naive framing.** The public 231 items were used in
  developing our arms; any submission says so in the issue.

---

## 8. Sources

Verified first-hand (fetched 2026-10-05 unless noted):

- <https://benchmarkheaven.com/jev-models> — live board v1.5.7: scoring
  formulas, Jev-class caps, latency/cost adjustments, pricing rules,
  coverage/rank rules, endpoint provenance, submission FAQ, per-row data for
  decider-2b and Open-Jev 2B.
- <https://benchmarkheaven.com/submit> — submission form fields, queue and
  fast-lane terms, re-evaluation policy.
- <https://github.com/fstandhartinger/jevbench> — README: issue submission
  route, house rules, adapter table, v1.2.x revision log (jqv v1.2.7,
  decider-2b v1.2.8, Open-Jev v1.2.15).
- <https://github.com/fstandhartinger/jevbench/issues/159> — offline-artifact
  issue template (OpenJev-4B).
- <https://github.com/fstandhartinger/jevbench/issues/190> — runnable-bundle
  issue template (decision-stack-0.8b-v0.1), disclosure and pricing language.
- <https://jevbench.xyz/> and <https://jevbench.xyz/methodology> — first-party
  archive, evidence tiers, private-evaluation path, unpriceable self-hosted
  cost entries.
- <https://jev.page/> — independent Jev 101; TypeSafe company claims
  (70–500 ms, $0.042/MTok input, console.typesafe.ai).
- <https://github.com/jabr/classifier-benchmark> — CC0 harness, TOML suites,
  v1 78 / v2 866 cases, headline results, backend/adapter layout, no intake
  channel.
- <https://huggingface.co/wfzyx/von> — submitter-side confirmation of sealed
  handling, board-axis examples (I 34.5 / C 75.7 / S 70.5 / K 77.8, composite
  27.5, sealed ECE 0.107), the "calibration map was fitted on the 231 public
  JevBench items" disclosure, jabr v2 72.0% macro / 869 cases.
- <https://www.everydev.ai/tools/von-decision-model> — Von tool page: jabr v2
  71.5% macro / 869 cases, repo dates, weights location.
- In-repo anchors: `benchmarks/decision-model/JEVBENCH.md` (harness contract,
  #105 results, wire facts); `docs/BENCHMARKS.md` (four surfaces, board-A/B
  separation); `docs/RESEARCH.md` lines 1585–1586, 1671, 1680, 1698, 1746,
  1752 (board map, jabr rows — read-only; line 1746's "jabr run if the
  harness is public" condition is now half-met).

Marked **unverified**: RESEARCH §14's v1.6.0 board figures (§1.1); whether
jabr accepts outside backend contributions (§3); the 866-vs-869 jabr case
count discrepancy (§3); Von 0.704 (RESEARCH) vs 0.715 (EveryDev) vs 0.720
(repo README current) on jabr v2 — revision drift, resolved in favor of the
current repo README until re-checked.
