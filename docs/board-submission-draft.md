# Board submission draft — runnable offline bundle (Benchmark Heaven)

Status: draft for task #107/#112. Companion to
`docs/board-submission-prep.md` (research + verdicts; this file is the
artifact). Nothing submitted. Fields marked **OPERATOR** are the eight
§6 questions — everything else is self-decidable and drafted.

## Pre-submission gates

| gate | status |
|---|---|
| Abstention → 422 so the 3-error stop cannot fire | **green locally** — bridge emits `status=422` on abstain (`run_jevbench.py:292`); both 2026-10-05 postures completed 231/231 with replay determinism. The item-186 hazard is retired on the public split; the sealed half is untestable locally by construction. |
| Full-coverage 1,624-decision completion | their run; best local proxy passed (above) |
| Cost basis ruling | **open — ask maintainers before submitting** (prep §1.4, §6 Q4); without it the row risks "unpriced: no Cost axis and no score" |
| Development-exposure disclosure wording | facts on hand (prep §1.8.4); **OPERATOR** signs off final wording |

## Bundle contents (standalone, publishable)

1. `README.md` — what the system is, build + serve + adapter commands,
   licence, execution-environment statement (CPU-only, no GPU, no
   weight download for the engine arm; RAM/cores assumed stated).
2. Adapter — extracted from `benchmarks/decision-model/runner/` into a
   standalone package (the strict native wire: `state {text, facts}`,
   score levels as structs, full-probs entries, Boolean probability =
   confidence of the decided value; `probs_source="native"`).
3. Recipe bundle SHA-256 manifest + pinned commit SHA.
4. `LICENSE` (Apache-2.0).
5. Self-run diagnostics file, labeled *not official*: proofs-posture
   0.6883 / macro 0.6622 / ECE 0.171 / p50 0.58 s / p95 14.0 s,
   fusion-v2 0.5584 / p95 0.73 s — repro commands included.

Publishing target: **OPERATOR** (which repo/account; standalone bundle
alongside the workspace acceptable? — prep §6 Q1).

## Draft issue body (#190 shape)

> **Title:** `[bench request]: opencodifier-engine (<posture>) — runnable
> offline CPU bundle, native probabilities`
>
> This is an offline artifact submission. Please download the pinned
> code and recipe and run the evaluation in your own environment. No
> author-hosted endpoint or API credential is required. We request the
> maintainer-controlled v1.5 evaluation, including its sealed portion.
>
> - **System**: OpenCodifier decision runtime — deterministic
>   rule/relational engine (<posture> posture: **OPERATOR** —
>   proofs-posture vs fusion-v2 vs engine-only, prep §6 Q3).
> - **Weights**: none for the engine posture (CPU-only, no model
>   download); fusion-v2 adds a pinned 4B rung artifact (HF link +
>   revision — **OPERATOR** if that posture is chosen).
> - **Code**: <repo URL> @ <pinned SHA> — **OPERATOR** (prep §6 Q1).
> - **Licence**: Apache-2.0.
> - **Interface**: strict native wire (prep §5 item 5; restated in the
>   bundle README). Probability source: `probs_source="native"` over
>   the exact label set, ~unit sum, no synthesized distributions.
> - **Abstention**: `status_code=422` (refusal, not infra). Verified
>   on the public 231 split: 231/231 completion, replay determinism,
>   after the 3-consecutive-error stop hazard was fixed.
> - **Temperature**: none (deterministic engine; `fitted: false`
>   n/a — no calibration fit step in the engine posture).
> - **Requested execution**: maintainer-controlled CPU run of the full
>   1,624-decision protocol, all three types (full coverage claimed).
> - **Pricing**: **OPERATOR + maintainer question** (prep §6 Q4) — we
>   ask which cost basis applies to a zero-ML deterministic runtime:
>   measured-input proxy (their v1.5 basis) vs base-model reference via
>   the fusion rung vs unpriced. We claim no free cost or speed
>   advantage.
> - **Development exposure**: the public 231-case JevBench snapshot
>   (`bb05a335`) was previously evaluated for the engine and both
>   gates; this is disclosed development exposure, not an untouched
>   official test (#190's language is the model).
> - **Self-run diagnostics** (not official, public split only):
>   proofs-posture 0.6883 accuracy, ECE 0.171, p50 0.58 s / p95 14.0 s
>   (raw; board-adjusted ≈ ×2 + 0.15 s per their rule). No composite,
>   score, or rank is claimed from these.
> - **Contact**: **OPERATOR** (prep §6 Q2).
>
> I do not yet claim an official composite or rank.

## Operator question sheet (final form — prep §6)

1. Publishing: which repo/account; standalone bundle OK alongside the
   workspace?
2. Submitter identity: email + GitHub identity for the issue/form.
3. System identity: proofs-posture (0.6883 public, Speed-exposed tail)
   / fusion-v2 (0.5584, Speed ≈ 86.1) / engine-only (cleanest cost
   story, hardest).
4. Cost basis: which ruling to request from maintainers for a zero-ML
   runtime.
5. Free queue vs paid fast lane (48 h, JevBench only).
6. Sealed-text handling on their pods: confirm no licensing objection.
7. p95 exposure: accept 14.0 s (→ 28.15 s adjusted) on proofs-posture
   or attack the rung tail first (single largest controllable axis
   loss).
8. jabr: write the `bench/` backend adapter now or ask the owner first
   (outside-backend acceptance unverified).

## Claims carried into the issue (and refused claims)

Carried: deterministic execution trace as explainability; native
probabilities; full-coverage intent; disclosed development exposure.
Refused (prep §7 governs): no "beats Jev", no self-derived composite or
rank, no cross-surface number, no Jev-class claim from our arithmetic,
no zero-cost claim, no development-naive framing.
