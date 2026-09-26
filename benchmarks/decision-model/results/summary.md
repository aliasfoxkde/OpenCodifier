# Decision-model benchmark — results

Suite: `suite/suite.json` (120 items; metadata_match / lexical_semantic /
relational_compositional, 40 each). Accuracy is top-1 candidate id.
Columns: accuracy (meta/lex/rel), overall accuracy, ECE of the winner
probability, single-decision latency, determinism.

| run | acc (meta/lex/rel) | acc | ECE | p50 | determinism |
|---|---|---|---|---|---|
| embed__minilm-l6-v2.json | 0.30 / 0.62 / 0.35 | 0.425 | 0.172 | 107.0ms | yes |
| engine__builtin-lexical.json | 0.88 / 0.23 / 0.35 | 0.483 | 0.115 | 5.3ms | yes |
| llama__Llama-3.2-1B-Instruct.json | 0.47 / 0.35 / 0.35 | 0.392 | 0.265 | 996.5ms | yes |
| llama__gemma-3-4b-it.json | 0.95 / 0.85 / 0.45 | 0.750 | 0.236 | 3039.7ms | yes |
| llama__qwen0.5b.json | 0.60 / 0.33 / 0.25 | 0.392 | 0.218 | 694.8ms | yes |
| llama__qwen1.5b.json | 0.90 / 0.45 / 0.40 | 0.583 | 0.191 | 1247.8ms | yes |
| llama__qwen3b.json | 0.90 / 0.75 / 0.38 | 0.675 | 0.263 | 2575.2ms | yes |

## Notes

- `engine__builtin-lexical.json` outcomes: {'abstain': 76, 'accept': 16, 'verify': 28} — abstention/verify routing is part of the engine contract, not a failure.
- `llama__Llama-3.2-1B-Instruct.json` chat (JSON-writing) baseline: acc 0.342, p50 780ms — the token-by-token alternative the decision arm replaces.
- `llama__Llama-3.2-1B-Instruct.json` bulk per-decision (batched contexts): lexical_semantic: 167ms; metadata_match: 1344ms; relational_compositional: 285ms.
- `llama__gemma-3-4b-it.json` chat (JSON-writing) baseline: acc 0.775, p50 3399ms — the token-by-token alternative the decision arm replaces.
- `llama__gemma-3-4b-it.json` bulk per-decision (batched contexts): lexical_semantic: 565ms; metadata_match: 4573ms; relational_compositional: 954ms.
- `llama__qwen0.5b.json` chat (JSON-writing) baseline: acc 0.433, p50 479ms — the token-by-token alternative the decision arm replaces.
- `llama__qwen0.5b.json` bulk per-decision (batched contexts): lexical_semantic: 103ms; metadata_match: 878ms; relational_compositional: 178ms.
- `llama__qwen1.5b.json` chat (JSON-writing) baseline: acc 0.633, p50 1076ms — the token-by-token alternative the decision arm replaces.
- `llama__qwen1.5b.json` bulk per-decision (batched contexts): lexical_semantic: 223ms; metadata_match: 1920ms; relational_compositional: 385ms.
- `llama__qwen3b.json` chat (JSON-writing) baseline: acc 0.550, p50 2160ms — the token-by-token alternative the decision arm replaces.
- `llama__qwen3b.json` bulk per-decision (batched contexts): lexical_semantic: 458ms; metadata_match: 3924ms; relational_compositional: 801ms.
