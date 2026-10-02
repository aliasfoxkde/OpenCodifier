# Fusion study — confidence-gated escalation ladder (post-hoc)

Simulated offline over measured per-item arm rows; every probability,
prediction and latency is a harness measurement, nothing re-inferred.
This bounds a live ladder before one is wired into the policy.

Joined 231 JevBench items across engine-v4 + vtx-v1.

## Single arms (reference, joined set)

- **engine**: acc 0.377, ECE 0.402
- **fallback**: acc 0.411, ECE 0.126

Oracle (any rung correct): **0.584** — no routing policy can exceed this on the joined set.

## Cascade results (thresholds swept)

Accept a rung when its gate clears the threshold (`prob` = winner
probability, `margin` = top minus runner-up); final rung always
answers. Cost = rungs actually incurred.

| engine t | fallback t | gate | acc | ECE | mean ms | routing |
|---|---|---|---|---|---|---|
| 0.30 | — | prob | 0.455 | 0.357 | 6.3 | engine 81%, fallback 19% |

Best accuracy: 0.455 at engine t=0.30, gate prob (engine 81%, fallback 19%; mean 6.3 ms).
Best under 50 ms mean: 0.455 at engine t=0.30, gate prob (engine 81%, fallback 19%; mean 6.3 ms).

Best operating point by class: adequacy 0.50, adversarial 0.50, ambiguous 0.29, extraction 0.67, fact 0.50, intent 0.62, judge_hard 0.47, long_policy 0.21, multi_hop 0.17, ordinal 0.33, policy 0.42, probability 0.30, routing 0.42, routing_hard 1.00, temporal_numeric 0.33, tool_selection 0.75, tradeoff 0.33, trap 0.50.

