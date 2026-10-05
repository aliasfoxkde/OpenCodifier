# JevBench full-system re-run — the engine + rung on the official public split

Task #105. The user directive: "better than JEV" must be earned on THEIR
benchmark. This is the full current system — fusion-v2 ladder + model rung
through the real engine — against the published numbers, on the authors'
own harness. Two arms, one contract, fedora (CPU), 2026-10-05.

## Contract

| | |
|---|---|
| Harness | authors' runner, `fstandhartinger/jevbench` @ `9ec6f15a` (MIT), serial, no-retry |
| Dataset | public 231 (`dc3995d8…`), easy 48 / hard 111 / original 72 |
| Engine | `opencodifier` @ `fa1f08a` — first run under the D32 contract (`max_execution_time` 120 s) |
| Ladders | `fusion-v2.json` (`36f3ab3e…`), `proofs-only-v1.json` (`01669dba…`) |
| Rung | gemma-4-E2B-it-QAT-Q4_0 via llama.cpp `build-pd` (`:8091`, `--parallel 1`, `--decision-seqs 24`), shared by both arms |
| Scoring | their summarize.py; not-ok → not correct; 422 = refusal not outage |

Both runs: 231/231 planned and attempted, replay determinism
predictions_match = true (231/231), zero infra stops.

## Results

| arm | acc (231) | macro | ECE | Brier | p50 | p95 | abstains | op. success |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| proofs-only-v1 (structural posture) | **0.6883** | 0.662 | 0.171 | 0.431 | 0.58 s | 14.0 s | 12 | 0.948 |
| fusion-v2 (as-shipped gate) | 0.5584 | 0.562 | 0.335 | 0.734 | 1.2 ms | 0.73 s | 0 | 1.000 |

Anchors on the same split (all ours unless noted): fork_4b rung
standalone 0.7662 (macro 0.7571) · bridge-only 0.6494 · engine-only
0.3766 · post-hoc cascade best (oracle-picked threshold) 0.688 · oracle
0.853 · hosted Jev 86.6 % (official).

fusion-v2 reproduces the banked 10 s-contract run **exactly** — same
129/231, same routing split (below) — so the D32 ceiling raise changed
nothing for the as-shipped gate: its failures were never timeouts.

## The decomposition — the gate is the liability, the rung is the asset

Routing split (latency > 50 ms = went through the rung):

| arm | engine-accepted | acc | escalated | acc |
|---|---:|---:|---:|---:|
| fusion-v2 | 159 (68.8 %) | **0.434** | 72 (31.2 %) | **0.833** |
| proofs-only | 8 (3.5 %) | 0.500 | 223 (96.5 %) | 0.695 |

F24's warning landed as measured fact: the gate tuned in-domain accepts
OOD garbage (0.434 on what it keeps) while the very same rung, asked
directly, scores 0.833 on what it is handed. Swapping the gate posture
— accept only exact proofs locally, escalate everything else — is worth
+13.0 points on the same weights, same items, same rung, and also
roughly halves ECE (0.335 → 0.171).

fusion's damage concentrates where cheap rungs guess: routing
adequacy 0.833→0.583, fact 1.00→0.50, extraction 1.00→0.667,
policy 1.00→0.667, judge_hard 0.824→0.471 versus the proofs arm.

## The honest full-system number

**0.688** (proofs posture) is the system-of-record number: a zero
benchmark-tuning structural configuration. It beats the bridge-only
anchor (0.6494) and ties the post-hoc cascade bound (0.688) — a bound
that was picked with oracle access to outcomes, which the posture
reaches a priori. It is still **below** the raw rung standalone
(0.7662) and **far below** hosted Jev (86.6). "Better than JEV" is not
earned on their benchmark; the gap to the rung standalone decomposes as:

- 12 engine abstains scored incorrect (5.2 pp): rung confidence < 0.5
  on long_policy (5), temporal_numeric (4), multi_hop, trap, routing.
  Honest refusals — their accuracy column counts them 0
  (their board's calibration column is the one that rewards
  abstention).
- 4 exact proofs wrong (proof rung declares confidence 1.0 but fact
  extraction misfired on hostile text): the "exact" rung is only as
  exact as its extractor.
- engine-path invocation of the rung retains a small residual delta
  versus the rung's own native harness mode.

## What this buys

- The official-board story is now two-sided and quantified: as-shipped
  (fusion-v2) 0.558, structural posture 0.688, rung asset 0.766,
  target 0.866. The binding constraint is calibration transfer, not
  model quality — the same conclusion F24 reached in-domain.
- Feeds #107: the proofs posture's 0.688 @ p50 0.58 s is the
  submission-shaped number (their board rewards its abstention
  profile); the fusion gate needs OOD-retuned thresholds before it
  should ever ship in front of the rung.

## Reproduction

```
# fedora, rung server first (pid-verified, port 8091):
python3 runner/run_jevbench.py --arm engine \
  --binary $HOME/oc-eval-repo/target/release/opencodifier \
  --ref $HOME/oc-model-eval/jevbench-ref \
  --tasks $HOME/oc-model-eval/jevbench-ref/datasets/public/{easy,hard,original}.jsonl \
  --out-dir $HOME/oc-model-eval/runs/jevbench/< proofs-only-fedora-v1 | fusion-v2-fedora-v2 > \
  --ladder $HOME/oc-eval-repo/ladders/< proofs-only-v1 | fusion-v2 >.json \
  --llama http://127.0.0.1:8091 --llama-model-id gemma4-e2b-it-qat-q4_0 --port 8179
```

Run dirs hold manifest.json (determinism block, serve provenance with
ladder sha256), summary.json, results.jsonl, results-replay.jsonl,
raw/, and the resources monitor report. proofs-only wall 1246 s
cpu 139 s; fusion wall ≈ 80 s. The earlier banked fusion run under the
10 s contract stays on disk as `fusion-v2-fedora-v1` (identical
result); the two aborted proofs-only attempts are
`proofs-only-timeout-abort-v1` (engine-side 10 s deadline, pre-D32) and
`proofs-only-fedora-abort-n3-v1` (schema rejected the 120 s request,
pre-D32) — both are the incident trail behind D32.
