#!/usr/bin/env python3
"""Kai-0.6B ONNX parity vs the export-baked ref.json (opencodifier #92).

Feeds each ref case's pre-tokenized ids / option_pos / answer_pos
through the quantized export and compares the f64 softmax of the
logits against the reference distributions the author's export baked
in. noul cases: `answer["noul"]` is P(keys[-1]) (positive/true last).
"""
import json
import math

import numpy as np
import onnxruntime as ort

BASE = "/home/mkinney/oc-model-eval/models/onnx/kai-0.6b-onnx"


def softmax(x: np.ndarray) -> np.ndarray:
    e = np.exp(x - x.max())
    return e / e.sum()


def main() -> None:
    ref = json.load(open(f"{BASE}/ref.json"))
    sess = ort.InferenceSession(f"{BASE}/model_quantized.onnx",
                                providers=["CPUExecutionProvider"])
    rows = match = 0
    worst = ("", 0.0)
    probs_out = []
    for entry in ref:
        for case in entry["cases"]:
            feed = {
                "input_ids": np.array([case["ids"]], np.int64),
                "attention_mask": np.ones((1, len(case["ids"])), np.int64),
                "answer_pos": np.array([case["answer_pos"]], np.int64),
                "option_pos": np.array([case["option_pos"]], np.int64),
            }
            logits = sess.run(None, feed)[0][0]
            got = softmax(logits.astype(np.float64))
            keys = case["keys"]
            want = case["answer"]
            rows += 1
            if want["type"] == "noul":
                ref_p = {keys[0]: 1.0 - want["noul"], keys[1]: want["noul"]}
            elif want["type"] == "choice":
                ref_p = want["probabilities"]
            else:
                ref_p = want["probabilities"]
            deltas = {k: abs(got[i] - ref_p[k])
                      for i, k in enumerate(keys)}
            m = max(deltas.values())
            if m > worst[1]:
                worst = (f"{entry['id']}/{case['question']}", m)
            got_map = {k: float(got[i]) for i, k in enumerate(keys)}
            if want["type"] == "choice":
                ok = max(got_map, key=got_map.get) == want["choice"]
            elif want["type"] == "score":
                ok = max(got_map, key=got_map.get) \
                    == max(ref_p, key=ref_p.get)
            else:
                ok = (got_map[keys[1]] > got_map[keys[0]]) \
                    == (want["noul"] > 0.5)
            match += int(ok)
            probs_out.append({"case": f"{entry['id']}/{case['question']}",
                              "got": got_map, "ref": ref_p})
    print(json.dumps({"rows": rows, "argmax_match": match,
                      "match_rate": round(match / rows, 4),
                      "max_abs_prob_delta": round(worst[1], 6),
                      "worst_case": worst[0]}, indent=1))
    with open("/home/mkinney/oc-model-eval/runs/kai-onnx-parity.json", "w") as f:
        json.dump({"rows": rows, "argmax_match": match,
                   "max_abs_prob_delta": worst[1], "cases": probs_out}, f, indent=1)


if __name__ == "__main__":
    main()
