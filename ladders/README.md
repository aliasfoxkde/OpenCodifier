# Ladder profiles

Named `LadderProfile` documents (see
`crates/opencodifier-engine/src/ladder.rs`): per-node-kind confidence
gates and per-rung calibration artifacts that the engine consults at the
existing gate instead of the request's own policy. Loading one is how an
escalation ladder ships — data, not code, validated before an engine is
built. The default ladder is empty; an engine without `--ladder` runs
byte-identically to a single-policy engine.

## Loading

Both `decide` and `serve` (and `mcp serve`) accept `--ladder <PATH>`:

```bash
opencodifier serve --ladder ladders/fusion-v1.json
opencodifier decide --ladder ladders/proofs-only-v1.json -i request.json
```

A non-empty profile decorates the cache identity with
`|ladder-v1@<id>`, so decisions cached under a different ladder (or no
ladder) never replay across profiles. The profile `id` is the
discriminator — bump it when a profile's gates or artifacts change.

## Shipped profiles

### `fusion-v1.json` — the measured escalation profile

From the fusion study
(`benchmarks/decision-model/results/fusion-suite.md`, task #59) and the
rank/margin gate study
(`benchmarks/decision-model/results/embed-margin-study.md`):

- `rule` rung — accept at p ≥ 1.0 only: exact proofs gate themselves
  (a proof is a single-entry distribution at p = 1.0 by construction);
  anything else verifies.
- `choice` / `boolean` / `score` rungs — the accept-on-margin shape:
  `min_confidence 0.0` with the §19 `min_margin` floor at the measured
  operating point **0.0183** (coverage 0.208, accepted accuracy 0.880,
  120-item embedding-arm measurement). The gate cascade is demote-only,
  so this — not a high `min_confidence` — is how a rung accepts on
  margin evidence alone: the probability gate never fires and only the
  margin floor demotes near-ties to verification.

Measured cautions carried from the margin study: the accepted-set ECE
(n = 25) is informational, not a gate input; coverage is modest — the
rung is a cheap pre-filter, not a replacement for the model rung. Per
F24, rung calibration precedes enabling a ladder by default; until the
per-domain artifacts land, ladders ship opt-in exactly like the
disabled §19 gates.

### `proofs-only-v1.json` — the maximal-caution posture

Only exact proofs are accepted outright; every classifier decision
verifies. The rung to serve behind a verifier when the deploying
operator wants zero unverified probabilistic answers.

## Kinds, not mechanisms

Profiles key on the **deciding node kinds** (`rule`, `choice`,
`boolean`, `score`, …) — the rung that decides is the node that decides.
In the default zero-ML stack the deciding kinds are `rule` (relational
proofs) and `choice`/`boolean`/`score` (the classifier's answers);
`lexical` and `filter` narrow but do not decide, so entries on them
would be dead configuration. Per-node-id overrides (`per_node`) win
over per-kind when one specific node needs a different gate.
