# Release policy — weights vs. corpus vs. engine

Status: BINDING (operator policy, recorded 2026-10-14; reaffirms the
gate already enforced in TRAINING.md). Supersedes any contradicting
statement in planning docs.

## The split

| Asset | Disposition | License |
|---|---|---|
| Engine + all code (this repo) | **Open, always** — it is the product and the adoption surface | Apache-2.0 |
| Model weights (own trained arms) | **Open, gate-gated** — publishes only after both release-gate legs pass | Apache-2.0 |
| Training corpus (merged-v5 + predecessors, teacher-facts, disputes ledger) | **Withheld, private** | n/a (not distributed) |
| Extraction/consensus machinery (VIVERE lane, multi-family consensus tooling, per-kind calibration fit, slice-probe/parity discipline) | **Process knowledge** — described in docs, artifacts withheld | n/a |

## The gate (unchanged, TRAINING.md)

No weights publish until **suite ≥ 0.68 AND JevBench ≥ 0.6494** — the
arm must beat its base and the community checkpoint on both surfaces.
First arm through: e1-e (0.8083 / 0.6840). Quantized variants inherit
the gate only with their own replayed numbers, like everything else.

## Rationale

- **The corpus is the moat, not the weights.** Weights alone are
  inspectable and (with a comparable corpus) replicable; the corpus —
  183k typed decision labels with two-family consensus, the per-question
  disputes ledger, and teacher-facts-v1 — is not cheaply reproducible by
  a single-model distill. Publishing weights without corpus keeps the
  training recipe's input private while making the *result* useful.
- **Open weights are the advertisement for the engine.** The engine is
  only demonstrably good when a real model rides it; the corpus keeps
  the *next* generation (and any training-service offering) ours.
- **Accepted trade:** Apache-2.0 weights let anyone build on them,
  including competitors — including distilling *from* our released
  model. That is the price of adoption and consistent with the
  local-first, no-telemetry, honest-claims brand ("Claims we refuse to
  make"). A one-teacher distill of our model cannot reconstruct the
  multi-family consensus corpus.
- **Honest disclosure:** the published model card states plainly what
  is withheld (data + machinery) and why. No "fully open" claims.

## Distribution discipline

- Explicit opt-in pull only; the engine never auto-downloads weights at
  build or run time (PLANNING.md §73). Base binary stays useful at
  zero ML.
- Model cards carry: base-model lineage, gate numbers per surface
  (one number per surface, never a universal claim), calibration
  artifacts shipped (per-kind T), and the license.

## License lineage of training inputs

Every teacher/judge contributing to a corpus that touches a released
arm must be permissively licensed. Current judges: qwen-line teacher
(Apache-2.0), strands-decider-E2B (Apache-2.0) — clean. Before adding
future judge families (e.g. GLM, Kimi), verify their license permits
corpus-derived derivative *training*; corpus stays private regardless,
but keeping the lineage clean preserves the option to release it later.

## Commercialization paths preserved by this split

1. Training/fine-tuning services on the withheld machinery.
2. A future corpus release under a negotiated/non-commercial license.
3. Next-generation weights (gated on their own legs, not auto-published).
4. Enterprise support, certification, and integration work on the
   Apache-2.0 engine.
