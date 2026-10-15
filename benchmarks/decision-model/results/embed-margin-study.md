# Rank/margin gate study — embedding rung (CALIBRATION finding 3)

Source: `embed__gte-modernbert-onnx-fp32__margins.json` (sha256 31254e3f35db, 120 items, config {"backend": "onnx", "device": "cpu", "max_len": 256, "pooling": "mean", "similarity": "cosine", "softmax": "tau=1", "threads": 4}). Every margin, probability and label is a harness measurement.

Separation: mean margin 0.0156 on correct vs 0.0059 on wrong items; P(correct margin wider) = 0.718.

## Gate sweep (`min_margin`)

| margin ≥ | coverage | accepted acc | accepted ECE | accepted errors |
|---|---|---|---|---|
| 0.0000 | 1.000 | 0.575 | 0.330 | 51 |
| 0.0006 | 0.942 | 0.584 | 0.337 | 47 |
| 0.0011 | 0.900 | 0.611 | 0.361 | 42 |
| 0.0018 | 0.850 | 0.618 | 0.366 | 39 |
| 0.0022 | 0.800 | 0.635 | 0.382 | 35 |
| 0.0029 | 0.750 | 0.644 | 0.389 | 32 |
| 0.0034 | 0.708 | 0.635 | 0.377 | 31 |
| 0.0039 | 0.650 | 0.679 | 0.420 | 25 |
| 0.0046 | 0.592 | 0.690 | 0.438 | 22 |
| 0.0050 | 0.550 | 0.712 | 0.453 | 19 |
| 0.0062 | 0.500 | 0.767 | 0.499 | 14 |
| 0.0079 | 0.450 | 0.759 | 0.490 | 13 |
| 0.0088 | 0.400 | 0.771 | 0.501 | 11 |
| 0.0107 | 0.350 | 0.810 | 0.536 | 8 |
| 0.0129 | 0.300 | 0.806 | 0.530 | 7 |
| 0.0147 | 0.258 | 0.806 | 0.530 | 6 |
| 0.0183 | 0.208 | 0.880 | 0.599 | 3 | ← operating point
| 0.0228 | 0.150 | 0.944 | 0.660 | 1 |
| 0.0283 | 0.100 | 1.000 | 0.711 | 0 |
| 0.0400 | 0.050 | 1.000 | 0.703 | 0 |

**Operating point: margin ≥ 0.0183** — coverage 0.208, accepted accuracy 0.880, accepted ECE 0.599, 3 errors let through of 51 total.

Ladder reading: the embedding rung accepts the top 21% of items outright at ≥88% accuracy and escalates the rest to the decision-model rung; its gate key is `min_margin`, never the (uncalibratable) probability. Two cautions: accepted-set ECE is computed on a small subset (n = 25) and swings wildly with selection — it is informational, not a gate input; and coverage at the operating point is modest, which is the honest price of an ordering-only scorer — the rung is a cheap pre-filter, not a replacement for the model rung.
