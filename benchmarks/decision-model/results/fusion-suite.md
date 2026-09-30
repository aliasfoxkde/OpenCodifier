# Fusion study — confidence-gated escalation ladder (post-hoc)

Simulated offline over measured per-item arm rows; every probability,
prediction and latency is a harness measurement, nothing re-inferred.
This bounds a live ladder before one is wired into the policy.

Joined 120 suite items across `engine`, `embed`, `llm`.

## Single arms (reference, joined set)

- **engine**: acc 0.683, ECE 0.094
- **embed**: acc 0.575, ECE 0.330
- **llm**: acc 0.725, ECE 0.062

Oracle (any rung correct): **0.975** — no routing policy can exceed this on the joined set.

## Cascade results (thresholds swept)

Accept a rung when its winner probability clears the threshold;
final rung always answers. Cost = rungs actually incurred.

| engine t | fallback t | acc | ECE | mean ms | routing |
|---|---|---|---|---|---|
| 0.35 | 0.30 | 0.800 | 0.105 | 409.9 | engine 71%, llm 29% |
| 0.50 | 0.50 | 0.842 | 0.114 | 713.8 | engine 57%, llm 42% |
| 0.55 | 0.30 | 0.867 | 0.127 | 752.7 | embed 1%, engine 54%, llm 45% |
| 0.60 | 0.50 | 0.833 | 0.108 | 928.0 | engine 46%, llm 54% |

Best accuracy: 0.867 at engine t=0.55, fallback t=0.30 (embed 1%, engine 54%, llm 45%; mean 752.7 ms).
Best under 497 ms mean: 0.800 at engine t=0.35, fallback t=0.30 (engine 71%, llm 29%; mean 409.9 ms).

Best operating point by class: lexical_semantic 0.72, metadata_match 1.00, relational_compositional 0.88.

