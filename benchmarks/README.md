# Benchmark results — v0.1.0 release baseline

Per DECISIONS.md D9: criterion measurements are kept per release as the
regression baseline. Regression = p99 over budget on the reference
machine class (4-core laptop). Everything below is **measured**
(`cargo bench -p opencodifier-engine --bench engine`), not projected.

Date: 2026-09-25 · Engine at commit `85cd2ec` (Phases 0–11 complete)

## Results vs the D9 per-stage budgets

| Benchmark | Measured (criterion point estimate) | Applicable D9 budget (p99) | Verdict |
|-----------|-------------------------------------|----------------------------|---------|
| `decide_cache_hit` | 17.2 µs (16.4–18.2) | 100 µs — rule match + cache-hit path | pass, ~6× headroom |
| `cache_key` | 4.9 µs (4.7–5.1) | component of the 100 µs cache-hit budget | pass |
| `decide_full_pipeline` | 693 µs (619–776) | 10 ms — deterministic decision path end-to-end | pass, ~14× headroom |
| `decide_mock_classifier` | 440 µs (410–476) | — (classifier-swap harness, no budget of its own) | informational |

Criterion reports the interval as [lower, point, upper]; the point
estimate is shown first above. The budgets are p99 figures; criterion
point estimates sit below p99, so passing here implies the p99 budget
with at least the stated headroom.

Not yet measurable (no implementation to drive): normalize-only and
BM25-narrowing-at-256-candidates stages are exercised inside the full
pipeline benchmark rather than in isolation; embedding scoring is
"backend-declared" per D9 and the `onnx` feature is deferred (D2).

## Reproducing

```bash
cargo bench -p opencodifier-engine --bench engine
```

Criterion's own history (trend deltas) lives under
`target/criterion/` and is not committed; this file is the release
record.
