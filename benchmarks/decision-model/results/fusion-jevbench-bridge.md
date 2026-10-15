# Fusion study — confidence-gated escalation ladder (post-hoc)

Simulated offline over measured per-item arm rows; every probability,
prediction and latency is a harness measurement, nothing re-inferred.
This bounds a live ladder before one is wired into the policy.

Joined 231 JevBench items across engine-v4 + bridge-v1.

## Single arms (reference, joined set)

- **engine**: acc 0.377, ECE 0.402
- **fallback**: acc 0.649, ECE 0.080

Oracle (any rung correct): **0.745** — no routing policy can exceed this on the joined set.

## Cascade results (thresholds swept)

Accept a rung when its gate clears the threshold (`prob` = winner
probability, `margin` = top minus runner-up); final rung always
answers. Cost = rungs actually incurred.

| engine t | fallback t | gate | acc | ECE | mean ms | routing |
|---|---|---|---|---|---|---|
| 0.75 | — | prob | 0.623 | 0.267 | 4979.8 | engine 48%, fallback 52% |
| 0.85 | — | prob | 0.632 | 0.244 | 7462.1 | engine 37%, fallback 63% |

Best accuracy: 0.632 at engine t=0.85, gate prob (engine 37%, fallback 63%; mean 7462.1 ms).
Best under 5709 ms mean: 0.623 at engine t=0.75, gate prob (engine 48%, fallback 52%; mean 4979.8 ms).

Best operating point by class: adequacy 0.83, adversarial 0.67, ambiguous 0.14, extraction 0.67, fact 0.92, intent 0.92, judge_hard 0.53, long_policy 0.26, multi_hop 0.17, ordinal 0.92, policy 0.75, probability 0.40, routing 0.92, routing_hard 0.80, temporal_numeric 0.27, tool_selection 1.00, tradeoff 0.50, trap 0.88.

