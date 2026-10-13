#!/usr/bin/env python3
"""T1.2 slice probe: strands-decider-E2B as second teacher on the distill corpus.

Campaign doc T1.2 (docs/planning/08b-gate-closure-campaign.md), no gate.
Posts each corpus row's state + questions VERBATIM to the authors'
`strands-decider serve /v1/systemone` — the same wire mapping as
run_jevbench.py --arm strands (state passthrough, native answer shapes,
never renormalized, no label override) — then compares argmax against
the corpus's teacher-gold `target.*` block.

Fixture-first discipline: run with --limit 10 and eyeball the output
shapes before any volume. Results are appended row-by-row (resume-safe:
already-probed record_ids are skipped on restart).

Output row: {record_id, source, source_config, state_chars, latency_s,
  questions: {name: {type, gold, gold_probs, answer, predicted,
                     correct|null, error|null}}}
"""

from __future__ import annotations

import argparse
import json
import time
import urllib.error
import urllib.request
from pathlib import Path


def question_record(q: dict) -> dict:
    """Wire question: verbatim type/instructions/criteria from the corpus."""
    rec = {"type": q["type"], "instructions": q["instructions"]}
    if q.get("criteria") is not None:
        rec["criteria"] = q["criteria"]
    return rec


def derive_prediction(ans: dict, qtype: str):
    """(predicted_label, probs_dict) from a native serve answer, or (None, None)."""
    if qtype == "noul":
        p = ans.get("noul")
        if not isinstance(p, (int, float)) or not 0.0 <= float(p) <= 1.0:
            return None, None
        return ("true" if float(p) > 0.5 else "false"), None
    probs = ans.get("probabilities")
    if not isinstance(probs, dict) or not probs:
        return None, None
    items = sorted(probs.items(), key=lambda kv: -float(kv[1]))
    return items[0][0], probs


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--endpoint", default="http://127.0.0.1:8462")
    ap.add_argument("--corpus", default="/home/mkinney/oc-model-eval/corpora/merged-v4.jsonl")
    ap.add_argument("--stride", type=int, default=183,
                    help="row stride; uniform coverage across corpus sources")
    ap.add_argument("--limit", type=int, default=1000)
    ap.add_argument("--out", default="/home/mkinney/oc-model-eval/runs/strands2b-slice-v1/results.jsonl")
    ap.add_argument("--timeout", type=float, default=600.0)
    args = ap.parse_args()

    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    done_ids: set[str] = set()
    if out.exists():
        with out.open() as f:
            for line in f:
                if line.strip():
                    done_ids.add(json.loads(line).get("record_id"))

    rows: list[str] = []
    with open(args.corpus) as f:
        for i, line in enumerate(f):
            if i % args.stride == 0:
                rows.append(line)
            if len(rows) >= args.limit + len(done_ids):
                break

    n_new = 0
    with out.open("a") as w:
        for line in rows:
            rec = json.loads(line)
            if rec["record_id"] in done_ids:
                continue
            req, tgt = rec["request"], rec["target"]
            body = json.dumps({
                "state": req["state"],
                "model": "strands-e2b",
                "questions": {name: question_record(q)
                              for name, q in req["questions"].items()},
            }).encode()
            http = urllib.request.Request(
                f"{args.endpoint}/v1/systemone", data=body, method="POST",
                headers={"Content-Type": "application/json"})
            t0 = time.perf_counter()
            try:
                with urllib.request.urlopen(http, timeout=args.timeout) as r:
                    parsed = json.loads(r.read())
            except urllib.error.HTTPError as e:
                w.write(json.dumps({"record_id": rec["record_id"],
                                    "status": "http_error", "code": e.code}) + "\n")
                w.flush()
                continue
            except Exception as e:  # noqa: BLE001 — probe records, never raises
                w.write(json.dumps({"record_id": rec["record_id"],
                                    "status": "error",
                                    "error": f"{type(e).__name__}: {e}"}) + "\n")
                w.flush()
                continue
            latency = time.perf_counter() - t0
            answers = parsed.get("answers") or {}
            questions_out = {}
            for name, q in req["questions"].items():
                qtype = q["type"]
                gold = tgt.get(name, {})
                gold_probs = gold.get("probabilities")
                entry = {"type": qtype, "gold": gold.get("label"),
                         "gold_probs": gold_probs, "answer": answers.get(name),
                         "predicted": None, "correct": None, "error": None}
                ans = answers.get(name)
                if not isinstance(ans, dict):
                    entry["error"] = "missing answer"
                elif ans.get("type") != qtype:
                    entry["error"] = "answer type mismatch"
                else:
                    pred, _ = derive_prediction(ans, qtype)
                    entry["predicted"] = pred
                    if pred is not None and gold.get("label") is not None:
                        entry["correct"] = bool(
                            str(pred) == str(gold["label"])
                            or (qtype == "score" and _score_hit(ans, str(gold["label"]))))
                questions_out[name] = entry
            w.write(json.dumps({
                "record_id": rec["record_id"], "source": rec.get("source"),
                "source_config": rec.get("source_config"),
                "state_chars": len(json.dumps(req["state"])),
                "latency_s": round(latency, 3),
                "status": "ok", "questions": questions_out,
            }) + "\n")
            w.flush()
            n_new += 1
            if n_new % 25 == 0:
                import sys
                sys.stdout.write(f"probed {n_new} rows\n")
                sys.stdout.flush()


def _score_hit(ans: dict, gold_label: str) -> bool:
    """Score fallback: argmax over legend order mapped to a gold bucket index."""
    legend = ans.get("legend")
    probs = ans.get("probabilities")
    if not isinstance(legend, list) or not isinstance(probs, dict) or not legend:
        return False
    keys = [str(k) for k in legend if str(k) in probs]
    if not keys:
        return False
    best = max(keys, key=lambda k: float(probs[k]))
    try:
        return str(legend.index(best)) == gold_label
    except ValueError:
        return False


if __name__ == "__main__":
    main()
