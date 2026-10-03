# Fusion study — confidence-gated escalation ladder (post-hoc)

Simulated offline over measured per-item arm rows; every probability,
prediction and latency is a harness measurement, nothing re-inferred.
This bounds a live ladder before one is wired into the policy.

Joined 231 JevBench items across engine-v4 + vtx-v1 + fork_4b-fedora-v1.

## Single arms (reference, joined set)

- **engine**: acc 0.377, ECE 0.402
- **fallback**: acc 0.411, ECE 0.126
- **llm**: acc 0.766, ECE 0.070

Oracle (any rung correct): **0.853** — no routing policy can exceed this on the joined set.

## Cascade results (thresholds swept)

Accept a rung when its gate clears the threshold (`prob` = winner
probability, `margin` = top minus runner-up); final rung always
answers. Cost = rungs actually incurred.

| engine t | fallback t | gate | acc | ECE | mean ms | routing |
|---|---|---|---|---|---|---|
| 0.50 | 0.50 | prob | 0.532 | 0.330 | 691.0 | engine 81%, fallback 2%, llm 17% |
| 0.60 | 0.50 | prob | 0.554 | 0.311 | 883.0 | engine 62%, fallback 12%, llm 26% |
| 0.95 | 0.80 | prob | 0.688 | 0.238 | 3207.0 | engine 32%, llm 68% |

Best accuracy: 0.688 at engine t=0.95, fallback t=0.80, gate prob (engine 32%, llm 68%; mean 3207.0 ms).

Best operating point by class: adequacy 1.00, adversarial 0.83, ambiguous 0.43, extraction 0.67, fact 1.00, intent 0.96, judge_hard 0.71, long_policy 0.26, multi_hop 0.22, ordinal 1.00, policy 1.00, probability 0.30, routing 1.00, routing_hard 1.00, temporal_numeric 0.13, tool_selection 1.00, tradeoff 0.33, trap 0.88.

