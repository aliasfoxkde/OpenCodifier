# Decision-model benchmark — results

Suite: `suite/suite.json` (120 items; metadata_match / lexical_semantic /
relational_compositional, 40 each). Accuracy is top-1 candidate id.
Columns: accuracy (meta/lex/rel), overall accuracy, ECE of the winner
probability, single-decision latency, determinism.

Rows marked `(chat screen)` are token-by-token chat baselines from forks
without a decision arm: sampled decode, JSON-writing, no calibrated
distribution — context for the decision rows, never comparable to them.

| run | acc (meta/lex/rel) | acc | ECE | p50 | determinism |
|---|---|---|---|---|---|
| embed__gte-modernbert-base.json | 0.45 / 0.78 / 0.50 | 0.575 | 0.330 | 3374.8ms | yes |
| embed__minilm-l6-v2.json | 0.30 / 0.62 / 0.35 | 0.425 | 0.172 | 107.0ms | yes |
| engine__builtin-lexical.json | 0.88 / 0.23 / 0.35 | 0.483 | 0.115 | 5.3ms | yes |
| k2chat__K2-Horizon-1B-Q4_K_M.json (chat screen) | 0.88 / 0.80 / 0.50 | 0.725 | — | 1499.1ms | n/a (sampled) |
| k2chat__K2-Horizon-4B-Q4_K_M.json (chat screen) | 1.00 / 0.93 / 0.17 | 0.700 | — | 4878.9ms | n/a (sampled) |
| k2chat__K2-Horizon-7B-Q4_K_M.json (chat screen) | 1.00 / 0.95 / 0.45 | 0.800 | — | 9052.1ms | n/a (sampled) |
| laya__en.json | 0.30 / 0.80 / 0.33 | 0.475 | 0.089 | 547.6ms | yes |
| llama__Falcon-H1-Tiny-90M-Instruct.json | 0.23 / 0.28 / 0.33 | 0.275 | 0.462 | 459.4ms | yes |
| llama__Falcon-H1-Tiny-Tool-Calling.json | 0.28 / 0.20 / 0.33 | 0.267 | 0.278 | 147.0ms | yes |
| llama__Jev-Style-0.8B-Decision-v3-Q4_K_M.json (caveat) | 0.20 / 0.30 / 0.15 | 0.217 | 0.408 | 866.7ms | yes |
| llama__Llama-3.2-1B-Instruct.json | 0.47 / 0.35 / 0.35 | 0.392 | 0.265 | 996.5ms | yes |
| llama__MiMo-V2.6-Distill-Qwen-9B-IQ3_XXS.json | 1.00 / 0.93 / 0.42 | 0.783 | 0.078 | 10570.2ms | yes |
| llama__MiMo-V2.6-Distill-Qwen-9B-Q3_K_M.json | 1.00 / 0.93 / 0.53 | 0.817 | 0.081 | 18395.2ms | yes |
| llama__MiMo-V2.6-Distill-Qwen-9B-Q3_K_S.json | 1.00 / 0.93 / 0.53 | 0.817 | 0.048 | 14307.4ms | yes |
| llama__MiniCPM5-1B.json | 0.95 / 0.35 / 0.15 | 0.483 | 0.232 | 693.9ms | yes |
| llama__Qwen3.5-0.8B-q4_0.json | 0.93 / 0.68 / 0.35 | 0.650 | 0.074 | 612.6ms | yes |
| llama__Qwen3.5-2B-UD-IQ2_XXS.json | 0.55 / 0.45 / 0.15 | 0.383 | 0.212 | 1619.9ms | yes |
| llama__Qwen3.5-2B-UD-Q2_K_XL.json | 0.62 / 0.40 / 0.33 | 0.450 | 0.134 | 1954.7ms | yes |
| llama__Qwen3.5-2B-UD-Q3_K_XL.json | 0.95 / 0.82 / 0.50 | 0.758 | 0.135 | 1826.4ms | yes |
| llama__Qwen3.5-2B-UD-Q4_K_XL.json | 0.97 / 0.80 / 0.42 | 0.733 | 0.095 | 2184.4ms | yes |
| llama__Qwen3.5-2B-UD-Q5_K_XL.json | 1.00 / 0.80 / 0.47 | 0.758 | 0.089 | 2189.1ms | yes |
| llama__Qwen3.5-2B-UD-Q6_K_XL.json | 0.97 / 0.80 / 0.50 | 0.758 | 0.079 | 2170.8ms | yes |
| llama__Qwen3.5-2B-UD-Q8_K_XL.json | 1.00 / 0.80 / 0.50 | 0.767 | 0.111 | 2175.4ms | yes |
| llama__Qwen3.5-2B.json | 0.95 / 0.72 / 0.50 | 0.725 | 0.062 | 1734.8ms | yes |
| llama__Qwen3.5-4B-Q3_K_M.json | 1.00 / 0.93 / 0.40 | 0.775 | 0.093 | 5660.3ms | yes |
| llama__Qwen3.5-4B-Q3_K_S.json | 1.00 / 0.95 / 0.45 | 0.800 | 0.069 | 6776.0ms | yes |
| llama__Qwen3.5-4B-UD-IQ2_XXS.json | 0.97 / 0.50 / 0.38 | 0.617 | 0.079 | 3697.6ms | yes |
| llama__Qwen3.5-4B-UD-IQ3_XXS.json | 1.00 / 0.95 / 0.42 | 0.792 | 0.065 | 15747.8ms | yes |
| llama__Qwen3.5-4B-UD-Q3_K_XL.json | 1.00 / 0.97 / 0.42 | 0.800 | 0.099 | 7222.6ms | yes |
| llama__Qwen3.5-4B-UD-Q4_K_XL.json | 1.00 / 0.95 / 0.45 | 0.800 | 0.074 | 4871.7ms | yes |
| llama__Qwen3.5-4B-q4_k_m.json | 1.00 / 0.93 / 0.38 | 0.767 | 0.098 | 4228.3ms | yes |
| llama__Qwen3.8-0.8B-Distilled.json | 0.85 / 0.55 / 0.30 | 0.567 | 0.098 | 807.7ms | yes |
| llama__Qwen3.8-2B-Distill.json | 0.90 / 0.62 / 0.33 | 0.617 | 0.095 | 1343.9ms | yes |
| llama__Qwen3.8-4B-Distill.json | 1.00 / 0.95 / 0.35 | 0.767 | 0.057 | 3756.6ms | yes |
| llama__gemma-3-270m-it-q4_k_m.json | 0.23 / 0.12 / 0.25 | 0.200 | 0.626 | 298.5ms | yes |
| llama__gemma-3-4b-it.json | 0.95 / 0.85 / 0.45 | 0.750 | 0.236 | 3039.7ms | yes |
| llama__glm5.1-distill.json | 0.40 / 0.15 / 0.20 | 0.250 | 0.285 | 2194.4ms | yes |
| llama__granite-4.0-350m-q4_k_m.json | 0.30 / 0.50 / 0.23 | 0.342 | 0.239 | 422.8ms | yes |
| llama__qwen0.5b.json | 0.60 / 0.33 / 0.25 | 0.392 | 0.218 | 694.8ms | yes |
| llama__qwen1.5b.json | 0.90 / 0.45 / 0.40 | 0.583 | 0.191 | 1247.8ms | yes |
| llama__qwen3b.json | 0.90 / 0.75 / 0.38 | 0.675 | 0.263 | 2575.2ms | yes |

## Notes

- `engine__builtin-lexical.json` outcomes: {'abstain': 76, 'accept': 16, 'verify': 28} — abstention/verify routing is part of the engine contract, not a failure.
- `k2chat__K2-Horizon-1B-Q4_K_M.json` chat (JSON-writing) baseline: acc 0.725, p50 1499ms — the token-by-token alternative the decision arm replaces.
- `k2chat__K2-Horizon-4B-Q4_K_M.json` chat (JSON-writing) baseline: acc 0.700, p50 4879ms — the token-by-token alternative the decision arm replaces.
- `k2chat__K2-Horizon-7B-Q4_K_M.json` chat (JSON-writing) baseline: acc 0.800, p50 9052ms — the token-by-token alternative the decision arm replaces.
- `llama__Falcon-H1-Tiny-90M-Instruct.json` chat (JSON-writing) baseline: acc 0.217, p50 487ms — the token-by-token alternative the decision arm replaces.
- `llama__Falcon-H1-Tiny-90M-Instruct.json` bulk per-decision (batched contexts): lexical_semantic: 172ms; metadata_match: 535ms; relational_compositional: 348ms.
- `llama__Falcon-H1-Tiny-Tool-Calling.json` bulk per-decision (batched contexts): lexical_semantic: 36ms; metadata_match: 250ms; relational_compositional: 56ms.
- `llama__Jev-Style-0.8B-Decision-v3-Q4_K_M.json` caveat: interface mismatch, not model quality: this tune is read at per-option verdict slots (h.(w_yes - w_no) at each option's "->" position, shipped jev_score/temperatures) and was never trained on candidate-id token paths or JSON chat answers, so both of this harness's readouts are outside its trained interface; the row measures the mismatch (determinism of the mechanism still holds). Native-readout arm = new harness work.
- `llama__Jev-Style-0.8B-Decision-v3-Q4_K_M.json` chat (JSON-writing) baseline: acc 0.000, p50 4696ms — the token-by-token alternative the decision arm replaces.
- `llama__Jev-Style-0.8B-Decision-v3-Q4_K_M.json` bulk per-decision (batched contexts): lexical_semantic: 319ms; metadata_match: 1911ms; relational_compositional: 470ms.
- `llama__Llama-3.2-1B-Instruct.json` chat (JSON-writing) baseline: acc 0.342, p50 780ms — the token-by-token alternative the decision arm replaces.
- `llama__Llama-3.2-1B-Instruct.json` bulk per-decision (batched contexts): lexical_semantic: 167ms; metadata_match: 1344ms; relational_compositional: 285ms.
- `llama__MiMo-V2.6-Distill-Qwen-9B-IQ3_XXS.json` bulk per-decision (batched contexts): lexical_semantic: 1674ms; metadata_match: 11649ms; relational_compositional: 2465ms.
- `llama__MiMo-V2.6-Distill-Qwen-9B-Q3_K_M.json` bulk per-decision (batched contexts): lexical_semantic: 2932ms; metadata_match: 23832ms; relational_compositional: 4982ms.
- `llama__MiMo-V2.6-Distill-Qwen-9B-Q3_K_S.json` bulk per-decision (batched contexts): lexical_semantic: 2900ms; metadata_match: 31221ms; relational_compositional: 5807ms.
- `llama__MiniCPM5-1B.json` chat (JSON-writing) baseline: acc 0.092, p50 524ms — the token-by-token alternative the decision arm replaces.
- `llama__MiniCPM5-1B.json` bulk per-decision (batched contexts): lexical_semantic: 139ms; metadata_match: 952ms; relational_compositional: 227ms.
- `llama__Qwen3.5-0.8B-q4_0.json` chat (JSON-writing) baseline: acc 0.683, p50 649ms — the token-by-token alternative the decision arm replaces.
- `llama__Qwen3.5-0.8B-q4_0.json` bulk per-decision (batched contexts): lexical_semantic: 140ms; metadata_match: 960ms; relational_compositional: 205ms.
- `llama__Qwen3.5-2B-UD-IQ2_XXS.json` bulk per-decision (batched contexts): lexical_semantic: 340ms; metadata_match: 2311ms; relational_compositional: 514ms.
- `llama__Qwen3.5-2B-UD-Q2_K_XL.json` bulk per-decision (batched contexts): lexical_semantic: 381ms; metadata_match: 2637ms; relational_compositional: 573ms.
- `llama__Qwen3.5-2B-UD-Q3_K_XL.json` bulk per-decision (batched contexts): lexical_semantic: 390ms; metadata_match: 2775ms; relational_compositional: 617ms.
- `llama__Qwen3.5-2B-UD-Q4_K_XL.json` bulk per-decision (batched contexts): lexical_semantic: 436ms; metadata_match: 3084ms; relational_compositional: 670ms.
- `llama__Qwen3.5-2B-UD-Q5_K_XL.json` bulk per-decision (batched contexts): lexical_semantic: 481ms; metadata_match: 3626ms; relational_compositional: 784ms.
- `llama__Qwen3.5-2B-UD-Q6_K_XL.json` bulk per-decision (batched contexts): lexical_semantic: 452ms; metadata_match: 3194ms; relational_compositional: 685ms.
- `llama__Qwen3.5-2B-UD-Q8_K_XL.json` bulk per-decision (batched contexts): lexical_semantic: 474ms; metadata_match: 3114ms; relational_compositional: 663ms.
- `llama__Qwen3.5-2B.json` chat (JSON-writing) baseline: acc 0.717, p50 1565ms — the token-by-token alternative the decision arm replaces.
- `llama__Qwen3.5-2B.json` bulk per-decision (batched contexts): lexical_semantic: 378ms; metadata_match: 2667ms; relational_compositional: 577ms.
- `llama__Qwen3.5-4B-Q3_K_M.json` bulk per-decision (batched contexts): lexical_semantic: 1351ms; metadata_match: 9494ms; relational_compositional: 2193ms.
- `llama__Qwen3.5-4B-Q3_K_S.json` bulk per-decision (batched contexts): lexical_semantic: 1465ms; metadata_match: 10389ms; relational_compositional: 2240ms.
- `llama__Qwen3.5-4B-UD-IQ2_XXS.json` bulk per-decision (batched contexts): lexical_semantic: 874ms; metadata_match: 5960ms; relational_compositional: 1296ms.
- `llama__Qwen3.5-4B-UD-IQ3_XXS.json` bulk per-decision (batched contexts): lexical_semantic: 2177ms; metadata_match: 11989ms; relational_compositional: 2722ms.
- `llama__Qwen3.5-4B-UD-Q3_K_XL.json` bulk per-decision (batched contexts): lexical_semantic: 1332ms; metadata_match: 10351ms; relational_compositional: 2269ms.
- `llama__Qwen3.5-4B-UD-Q4_K_XL.json` bulk per-decision (batched contexts): lexical_semantic: 1649ms; metadata_match: 9644ms; relational_compositional: 2855ms.
- `llama__Qwen3.5-4B-q4_k_m.json` chat (JSON-writing) baseline: acc 0.483, p50 4891ms — the token-by-token alternative the decision arm replaces.
- `llama__Qwen3.5-4B-q4_k_m.json` bulk per-decision (batched contexts): lexical_semantic: 977ms; metadata_match: 6619ms; relational_compositional: 1423ms.
- `llama__Qwen3.8-0.8B-Distilled.json` chat (JSON-writing) baseline: acc 0.667, p50 966ms — the token-by-token alternative the decision arm replaces.
- `llama__Qwen3.8-0.8B-Distilled.json` bulk per-decision (batched contexts): lexical_semantic: 181ms; metadata_match: 1242ms; relational_compositional: 262ms.
- `llama__Qwen3.8-2B-Distill.json` chat (JSON-writing) baseline: acc 0.700, p50 1369ms — the token-by-token alternative the decision arm replaces.
- `llama__Qwen3.8-2B-Distill.json` bulk per-decision (batched contexts): lexical_semantic: 329ms; metadata_match: 2309ms; relational_compositional: 501ms.
- `llama__Qwen3.8-4B-Distill.json` chat (JSON-writing) baseline: acc 0.742, p50 3454ms — the token-by-token alternative the decision arm replaces.
- `llama__Qwen3.8-4B-Distill.json` bulk per-decision (batched contexts): lexical_semantic: 832ms; metadata_match: 5813ms; relational_compositional: 1276ms.
- `llama__gemma-3-270m-it-q4_k_m.json` bulk per-decision (batched contexts): lexical_semantic: 60ms; metadata_match: 559ms; relational_compositional: 108ms.
- `llama__gemma-3-4b-it.json` chat (JSON-writing) baseline: acc 0.775, p50 3399ms — the token-by-token alternative the decision arm replaces.
- `llama__gemma-3-4b-it.json` bulk per-decision (batched contexts): lexical_semantic: 565ms; metadata_match: 4573ms; relational_compositional: 954ms.
- `llama__glm5.1-distill.json` chat (JSON-writing) baseline: acc 0.392, p50 1792ms — the token-by-token alternative the decision arm replaces.
- `llama__glm5.1-distill.json` bulk per-decision (batched contexts): lexical_semantic: 1454ms; metadata_match: 1765ms; relational_compositional: 420ms.
- `llama__granite-4.0-350m-q4_k_m.json` bulk per-decision (batched contexts): lexical_semantic: 70ms; metadata_match: 658ms; relational_compositional: 174ms.
- `llama__qwen0.5b.json` chat (JSON-writing) baseline: acc 0.433, p50 479ms — the token-by-token alternative the decision arm replaces.
- `llama__qwen0.5b.json` bulk per-decision (batched contexts): lexical_semantic: 103ms; metadata_match: 878ms; relational_compositional: 178ms.
- `llama__qwen1.5b.json` chat (JSON-writing) baseline: acc 0.633, p50 1076ms — the token-by-token alternative the decision arm replaces.
- `llama__qwen1.5b.json` bulk per-decision (batched contexts): lexical_semantic: 223ms; metadata_match: 1920ms; relational_compositional: 385ms.
- `llama__qwen3b.json` chat (JSON-writing) baseline: acc 0.550, p50 2160ms — the token-by-token alternative the decision arm replaces.
- `llama__qwen3b.json` bulk per-decision (batched contexts): lexical_semantic: 458ms; metadata_match: 3924ms; relational_compositional: 801ms.
