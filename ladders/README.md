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

## The model rung

The escalation tail is a running llama-server serving the
`parallel-decision` fork's `POST /v1/decision` readout (DECISIONS.md
D26), attached with `--llama`:

```bash
opencodifier serve --ladder ladders/fusion-v1.json \
    --llama http://127.0.0.1:8080 \
    --llama-model-id 'pd-fork-ad129b0|qwen3.5-2b-q4_k_m|tree-v2'
```

The primary rungs stay cheap: exact proofs accept outright, and a
classifier decision must clear its rung's gate before it is trusted.
Only a non-accepting outcome (`Verify` *or* `Abstain`) fires the llama
rung, whose own distribution is gated exactly like any other rung's —
one classifier per accepted question, never two (D27). The model id is
the cache-key discriminator for the served weights; changing it re-keys
every cached decision, exactly like a model swap. Per-request timeouts
default to 30 s and move with `--llama-timeout-ms` (the board's CPU p50
for a 2B arm is ~1.7 s; 9B tails run longer).

`--llama` requires a build with `--features llamacpp` (it adds the
`ureq` dependency); a default build refuses the flag with
`cli.model_rung_unavailable` instead of silently ignoring it. The
deterministic-first contract is unchanged: without `--llama` the base
binary is useful with no model files anywhere on the machine, and with
it the model is fired only by the gate.

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

### `fusion-v2.json` — the confidence-gated refit (F28, measured)

The tree-shaped refit of `fusion-v1`'s classifier gate, fitted with
`benchmarks/decision-model/runner/tree_margin_refit.py` on measured
per-rung rows (lexical arm with full distributions; the model rung
collected under the engine's exact payload contract):

- `choice` / `boolean` / `score` rungs — accept on the winner's
  **confidence** at **0.56**, `min_margin 0.0`. The sweep showed the
  margin axis is flat on lexical/BM25 confidences (accuracy identical
  across margin floors 0.00–0.11): v1's 0.0183 floor was measured on
  embedding-cosine margins in the letters shape and did no work in the
  served shape. The confidence axis is the discriminator; 0.56 is the
  full-suite optimum, the modal per-half fit, and mid-plateau (0.933
  holds over 0.55–0.58).
- `rule` rung — unchanged from v1.

Measured through the real engine on the locked suite (120 items,
`engine__rung-qate2b-q4_0-fusion-v2.json`): blended **0.933** at
**259.8 ms** mean (p50 0.94 ms), 56 model-rung escalations, replay
bit-exact — vs v1's 0.842 @ 117.2 ms (37 escalations). Zero lexical
accept is wrong at this gate; all residual error is model-rung error on
escalated items, 2 of which fall below the gate themselves and are
honestly flagged `verify` instead of `accept` (v1's `min_confidence 0.0`
masked this distinction). Caveat carried in the report: the 0.933 is
in-sample (the gate was fitted on the same 120 items); per-half
refitting never beat fixed-0.56 out-of-sample, and a conservative
held-out estimate is ~0.90. **Measured held-out (F29, `suite_holdout.json`,
120 fully disjoint items): 0.883 vs v1's 0.817 under the identical stack
and host, and the holdout's own optimum gate band is 0.55–0.58 — the
shipped 0.56 sits at the holdout optimum with zero drift.**

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
