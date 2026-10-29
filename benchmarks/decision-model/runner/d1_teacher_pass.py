#!/usr/bin/env python3
"""d1-2B teacher pass over teacher-items-v1 (output-only, lfm1.0).

Votes on the gold-free item stream via llama-server /v1/systemone
(one question per request, native probabilities). Resume: append-only
JSONL keyed by item id; a completed id is skipped on restart.

Order: pool-priority — quarantine-pool records first, then the rest in
file order. --limit N stops after N new votes (slice mode).

Output rows: {"id", "record_id", "question", "qtype", "probs", "label",
"latency_s", "ok", "error"}.
"""
import argparse
import json
import logging
import time
import urllib.request

ITEMS = "/nas/Temp/work/oc-model-eval/corpora/teacher-items-v1/items.jsonl"

logger = logging.getLogger("d1_teacher_pass")


def parse_args():
    parser = argparse.ArgumentParser()
    parser.add_argument("--endpoint", default="http://127.0.0.1:8756")
    parser.add_argument("--items", default=ITEMS)
    parser.add_argument("--pool-ids",
                        default="/nas/Temp/work/oc-model-eval/runs/d1-teacher-v1/pool_record_ids.json")
    parser.add_argument("--out",
                        default="/nas/Temp/work/oc-model-eval/runs/d1-teacher-v1/votes.jsonl")
    parser.add_argument("--limit", type=int, default=0,
                        help="stop after N new votes (0 = unbounded)")
    return parser.parse_args()


def load_pool(path):
    try:
        with open(path) as f:
            return set(json.load(f))
    except FileNotFoundError:
        logger.warning("no pool id file at %s; running in file order", path)
        return set()


def vote(item, endpoint):
    question = {"type": item["qtype"], "instructions": item["instructions"]}
    if item["qtype"] == "choice":
        question["criteria"] = {
            oid: opt.split(": ", 1)[1] if ": " in opt else opt
            for oid, opt in zip(item["ids"], item["options"])}
    elif item["qtype"] == "score":
        question["criteria"] = item["options"]
    body = {"state": item["state"], "questions": {"q0": question}}
    data = json.dumps(body).encode()
    request = urllib.request.Request(
        f"{endpoint}/v1/systemone", data=data, method="POST",
        headers={"Content-Type": "application/json"})
    started = time.perf_counter()
    try:
        with urllib.request.urlopen(request, timeout=300) as response:
            parsed = json.loads(response.read())
    except Exception as exc:
        return {"id": item["id"], "record_id": item["record_id"],
                "question": item["question"], "qtype": item["qtype"],
                "probs": None, "label": None, "ok": False,
                "error": f"{type(exc).__name__}: {exc}", "latency_s": 0.0}
    latency = time.perf_counter() - started
    answer = (parsed.get("answers") or {}).get("q0")
    row = {"id": item["id"], "record_id": item["record_id"],
           "question": item["question"], "qtype": item["qtype"],
           "probs": None, "label": None, "ok": False, "error": None,
           "latency_s": round(latency, 3)}
    if not isinstance(answer, dict):
        row["error"] = "missing answer q0"
        return row
    if item["qtype"] == "noul":
        p_true = answer.get("noul")
        if not isinstance(p_true, (int, float)):
            row["error"] = "missing noul probability"
            return row
        row["probs"] = {"true": float(p_true), "false": 1.0 - float(p_true)}
    else:
        probs = answer.get("probabilities")
        if not isinstance(probs, dict):
            row["error"] = "missing probabilities"
            return row
        row["probs"] = {str(k): float(v) for k, v in probs.items()}
    if row["probs"]:
        row["label"] = max(row["probs"], key=row["probs"].get)
        row["ok"] = True
    return row


def main():
    logging.basicConfig(level=logging.INFO,
                        format="%(asctime)s %(levelname)s %(message)s")
    args = parse_args()
    pool = load_pool(args.pool_ids)

    completed = set()
    try:
        with open(args.out) as f:
            for line in f:
                if line.strip():
                    completed.add(json.loads(line)["id"])
    except FileNotFoundError:
        pass

    pending = []
    with open(args.items) as f:
        for line in f:
            if line.strip():
                item = json.loads(line)
                if item["id"] not in completed:
                    pending.append(item)

    pool_items = [i for i in pending if i["record_id"] in pool]
    other_items = [i for i in pending if i["record_id"] not in pool]
    queue = pool_items + other_items
    logger.info("pending %d (pool-first %d pool, %d other); completed %d",
                len(queue), len(pool_items), len(other_items), len(completed))

    votes = 0
    started = time.time()
    with open(args.out, "a", buffering=1) as out:
        for item in queue:
            row = vote(item, args.endpoint)
            out.write(json.dumps(row) + "\n")
            votes += 1
            if not row["ok"]:
                logger.warning("vote failed %s: %s", row["id"], row["error"])
            if votes % 50 == 0:
                rate = votes / (time.time() - started)
                eta_h = (len(queue) - votes) / rate / 3600 if rate else 0.0
                logger.info("%d/%d (%.2f/s, ETA %.1f h)",
                            votes, len(queue), rate, eta_h)
            if args.limit and votes >= args.limit:
                break
    logger.info("finished %d new votes -> %s", votes, args.out)


if __name__ == "__main__":
    main()
