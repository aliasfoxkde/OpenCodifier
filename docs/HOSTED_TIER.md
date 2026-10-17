# Hosted API Tier — VPS Phases, Cost Story, Local/Product Separation

Status: design (task #41). Nothing here is implemented; the binary this
design deploys is the one that exists. Local-first posture is a founding
property (PLANNING.md §37–§38, Rule 13/14) — this document is about
**where that same binary may run for remote callers**, never about
changing what it is.

## 1. The invariant: a deployment mode, not a product mode

The hosted tier ships the identical `opencodifier serve` binary with the
identical `/v1` surface (`POST /v1/decide`, `/v1/graph/validate`,
`/v1/healthz` today; §36 names the fuller set). There is no hosted-only
feature, no hosted-only API, no hosted-only model lane. Everything that
makes a deployment "hosted" lives **outside** the binary:

| Concern | Owner | Never in the runtime |
|---|---|---|
| Auth, tokens, tenancy | edge proxy / control-center backend | no accounts, no API keys in the binary |
| TLS, CORS, rate limits | edge | no request admissions logic |
| Quotas, billing, audit | ledger services | no metering |
| Model downloads, manifests | operator, pinned by SHA-256 (D14, §38) | no auto-download, no phone-home |
| Telemetry | nobody (Rule 13) | anywhere, in any mode |

The binary keeps its founding defaults in every phase: loopback bind
unless `--host` is passed explicitly (§37 — the existing loopback gate
test enforces it), no telemetry, no cloud calls, hostile-input rule,
deterministic-first ladder, calibrated-or-abstaining confidence. A
hosted deployment that needed a forked binary would be a different
product and is out of scope by definition.

## 2. Why host at all (the honest list)

- **Convenience tier**: callers without the CPU/GPU budget for the D16
  model tiers get the same decisions from a shared box. The local binary
  remains fully useful with zero models (the engine rung decides at
  1.3 ms p50 on the suite); hosted adds the model tiers, not capability.
- **Fleet integration**: Amortyx (or any router) on a VPS wants a
  decision service on its own loopback; a hosted endpoint extends the
  same contract to callers that cannot run one locally
  (`docs/INTEGRATION_AMORTYX.md` is the in-process-adjacent case).
- **One measured economy**: decisions are cheap where they are cheap and
  expensive where they are expensive; a hosted lane lets cheap rungs
  serve many callers while expensive tiers stay pooled (§4).

Anti-goal restated: hosted is never required. Anything a caller can only
do hosted is a defect in this design.

## 3. VPS phases

### Phase H0 — single-principal loopback behind an authenticated edge

The Amortyx deployment shape, already operated: systemd service bound to
`127.0.0.1`, nginx (or the control-center backend) terminating TLS and
authenticating, CORS restricted, anonymous probes must `401`. Phase H0
is exactly this for `opencodifier serve`:

```text
caller ──TLS──► edge (auth: owner's token) ──loopback──► opencodifier serve
                                                        (127.0.0.1, no models or pinned models)
```

- One principal: the owner's own agents and tools.
- Model lanes: none required (engine rung only) or the D16 fast tier
  (Qwen3.5-0.8B, 613 ms) pinned by manifest.
- The edge authenticates **before** proxying — Amortyx's HANDOFF open
  gate #1 (replace nginx trust with an authenticated backend route)
  applies here verbatim: trust the authenticated backend, not the
  proxy hop.
- Upgrade path is a config change, not a code change: `--host` stays
  loopback; the edge does the network-facing work.

### Phase H1 — multi-principal on one VPS

Entry criteria: a second real caller exists (not a hypothetical), and
the H0 ledger shows decision traffic worth sharing a lane for.

- Per-principal tokens at the edge; per-principal rate limits and
  quotas as edge policy. The binary still sees one unauthenticated
  loopback caller (the edge) — tenancy never enters the engine.
- Per-principal graph/catalog selection: each principal's decisions
  name their graph; cache keys already fold graph/model/calibration/
  policy versions (§64, D6), so principals sharing a model lane cannot
  read each other's cached decisions unless the graph identity matches.
- Audit: decision ids + confidences per principal in the ledger
  (routing-trail analog), no request content retention beyond what the
  caller opted into.
- Model lanes become workers: the cheap rungs stay in-process with
  `serve`; the reference tier (4B, 4.9 s p50 measured on the 8-core
  host) runs as a separate lane process so a long decision cannot head-
  of-line block the hot path.

### Phase H2 — scale-out (explicitly deferred)

Entry criteria: measured saturation of H1 (hot-path p50 breaches the D9
interactive budget under real traffic), not forecast load. Sketch, not
commitment: read-replica decision caches (keys are content-addressed,
so replicas are consistent by construction), region placement at the
edge, GPU lane pooling for the model tiers. Nothing in Phases H0–H1
may build scaffolding for H2 — D9 budgets and the ladder decide when
this phase exists.

## 4. Cost story (measured anchors only)

No invented prices. The anchors:

- **Decision cost, measured** (this 8-core host, CPU-only, D16 record):
  engine rung **1.3 ms** p50; fast tier (0.8B) **613 ms**; reference
  tier (4B UD-Q4_K_XL) **4.9 s**; frontier tier (9B) **14.3 s**. On a
  VPS of this class, an engine-rung decision costs CPU-milliseconds —
  effectively free; the model tiers are the cost curve, and the lane
  table in `docs/INTEGRATION_AMORTYX.md` §5 maps them to lanes that can
  afford them (hot path: deterministic rungs only).
- **Decision value, measured** (Amortyx VALIDATION-REPORT): realizable
  savings on agentic/retry-heavy workloads are **10–30% of tokens**;
  at market API rates ($0.10–0.60/Mtok) that is **$0.028–0.17 saved
  per Mtok processed**, on measured burst COGS of $0.0215–0.029/Mtok.
  A hosted decision tier earns its keep exactly to the degree its
  decisions avoid or shrink upstream tokens — §59's "tokens avoided" is
  the billing-relevant metric, not decisions/sec.
- **Where a hosted decision is net-negative**: any lane where the
  decision's own latency or compute exceeds the token value it
  protects. A 4.9 s reference-tier decision inline on a hot path is
  negative value even at zero marginal cost — which is why the lane
  table, not enthusiasm, assigns tiers.
- **Hosted pricing posture**: the local binary is free and complete;
  hosted charges for the operator's box and the model lanes, priced
  off the measured per-decision costs above. If a tier cannot be priced
  profitably against the $0.028–0.17/Mtok value anchor, the tier does
  not ship — a hosted lane that loses money per decision is a subsidy,
  not a product.

## 5. Security posture (hosted additions, all at the edge)

- **Auth**: per-principal tokens, validated by the backend route before
  any byte reaches `serve`. The binary's unauthenticated loopback is a
  feature here: there is nothing to authenticate against at the socket,
  by design.
- **Hostile input**: unchanged and load-bearing — request text can
  never modify policy, thresholds, graphs, or paths (§73); a public-ish
  endpoint raises the stakes, it does not change the rule. The §45
  focus extraction bounds what a giant hostile body can cost
  (~2.5 ms extraction, measured).
- **Abuse**: rate limits and payload-size limits at the edge (the
  engine's own `limits` metadata bounds questions/candidates/graph
  size; the edge bounds bytes and rate).
- **Secrets**: never in this repo, never in the binary, never in
  graphs; the credential path is the operator's (Amortyx convention:
  mode-600 files, digest-based auth).
- **Probes**: the H0 promotion contract mirrors Amortyx's — anonymous
  request through the edge must `401`; authenticated must `200`;
  readiness truthful; no-secret-logging check; rollback rehearsed.

## 6. What must not be built

- Accounts, tenants, quotas, or meters inside the binary.
- A hosted feature flag that changes decision behavior (the hosted tier
  answers the same bytes a local run would for the same request and
  artifact versions — determinism is the contract).
- Cloud calls from the local runtime (Rule 14) — including "phone home
  to the hosted tier for hard decisions". Escalation to an external
  model is a caller-side policy in a caller-side graph, never a default
  the binary grows.
- Telemetry (Rule 13), in any phase, hosted included.

## 7. Risks

| Risk | Mitigation |
|---|---|
| Hosted drift: a fork accretes hosted-only behavior | the invariant table (§1) is the review gate; no hosted-only surface is mergeable |
| Expensive tier on the hot path | lane table assigns tiers (§4); H1 separates model lanes into worker processes |
| Multi-tenant cache leakage | version-folded cache keys (§64/D6) make cross-principal reads an identity mismatch; per-principal graphs differ by construction |
| Edge trust regression (auth bypass via proxy hop) | Amortyx HANDOFF open gate #1 applies; promotion contract requires the 401/200 probe pair |
| Unpriced cost curve | tiers ship only when priced against the measured value anchor (§4); the lane table is the envelope |
