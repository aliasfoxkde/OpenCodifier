# Fusion study — confidence-gated escalation ladder (post-hoc)

Simulated offline over measured per-item arm rows; every probability,
prediction and latency is a harness measurement, nothing re-inferred.
This bounds a live ladder before one is wired into the policy.

Joined 231 JevBench items across engine-v4 + vtx-v1 + fork_4b-v1.

## Single arms (reference, joined set)

- **engine**: acc 0.377, ECE 0.402
- **fallback**: acc 0.411, ECE 0.126
- **llm**: acc 0.667, ECE 0.142

Oracle (any rung correct): **0.823** — no routing policy can exceed this on the joined set.

## Cascade results (thresholds swept)

Accept a rung when its gate clears the threshold (`prob` = winner
probability, `margin` = top minus runner-up); final rung always
answers. Cost = rungs actually incurred.

| engine t | fallback t | gate | acc | ECE | mean ms | routing |
|---|---|---|---|---|---|---|
| 0.50 | 0.50 | prob | 0.524 | 0.326 | 2857.3 | engine 81%, fallback 2%, llm 17% |
| 0.60 | 0.50 | prob | 0.545 | 0.314 | 4464.5 | engine 62%, fallback 12%, llm 26% |
| 0.95 | 0.75 | prob | 0.636 | 0.252 | 14199.4 | engine 32%, fallback 2%, llm 66% |

Best accuracy: 0.636 at engine t=0.95, fallback t=0.75, gate prob (engine 32%, fallback 2%, llm 66%; mean 14199.4 ms).

Best operating point by class: adequacy 0.83, adversarial 0.83, ambiguous 0.29, extraction 0.67, fact 0.92, intent 1.00, judge_hard 0.59, long_policy 0.21, multi_hop 0.22, ordinal 0.83, policy 0.83, probability 0.30, routing 0.83, routing_hard 1.00, temporal_numeric 0.07, tool_selection 1.00, tradeoff 0.33, trap 1.00.

