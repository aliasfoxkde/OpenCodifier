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
import threading
import time
import urllib.request
from queue import Queue

ITEMS = "/nas/Temp/work/oc-model-eval/corpora/teacher-items-v1/items.jsonl"

logger = logging.getLogger("d1_teacher_pass")


def parse_args():
    parser = argparse.ArgumentParser()
    parser.add_argument("--endpoint", default="http://127.0.0.1:8756")
    parser.add_argument("--workers", type=int, default=4,
                        help="concurrent requests (match llama-server -np)")
    parser.add_argument("--items", default=ITEMS)
    parser.add_argument("--pool-ids",
                        default="/nas/Temp/work/oc-model-eval/runs/d1-teacher-v1/pool_record_ids.json")
    parser.add_argument("--out",
                        default="/nas/Temp/work/oc-model-eval/runs/d1-teacher-v1/votes.jsonl")
    parser.add_argument("--limit", type=int, default=0,
                        help="stop after N new votes (0 = unbounded)")
    parser.add_argument("--retry-failed", action="store_true",
                        help="strip failed rows from the output file so "
                             "resume re-votes them (run before the pass)")
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

    if args.retry_failed:
        kept = dropped = 0
        lines = []
        with open(args.out) as f:
            for line in f:
                if line.strip() and not json.loads(line).get("ok"):
                    dropped += 1
                    continue
                lines.append(line)
                kept += 1
        tmp = args.out + ".tmp"
        with open(tmp, "w") as f:
            f.writelines(lines)
        import os
        os.replace(tmp, args.out)
        logger.info("retry-failed: kept %d, dropped %d failed rows", kept, dropped)

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

    # Parallel voting: worker threads share the single append-only output
    # file behind a writer thread; resume semantics are unchanged because
    # the completed-id set was snapshotted above and rows are only
    # appended once. --limit bounds NEW votes; the overshoot past the
    # limit is at most workers-1 items.
    progress = {"done": 0, "ok": 0, "started": time.time()}
    progress_lock = threading.Lock()
    write_q = Queue()
    write_done = threading.Event()

    def writer():
        with open(args.out, "a", buffering=1) as out:
            while True:
                row = write_q.get()
                if row is None:
                    write_done.set()
                    return
                out.write(json.dumps(row) + "\n")

    def worker():
        while True:
            item = work_q.get()
            if item is None:
                return
            row = vote(item, args.endpoint)
            # Only completed votes go in the resume file; transient
            # failures (5xx, timeouts) stay pending for the next resume.
            # Permanent parse failures (malformed answer shape) are kept
            # so a poisoned item cannot loop forever.
            transient = (not row["ok"] and row["error"]
                         and row["error"].startswith(("HTTPError", "URLError",
                                                      "TimeoutError",
                                                      "ConnectionError")))
            if transient:
                logger.warning("transient failure %s: %s (not recorded)",
                               row["id"], row["error"])
            else:
                write_q.put(row)
            with progress_lock:
                progress["done"] += 1
                if row["ok"]:
                    progress["ok"] += 1
                n = progress["done"]
                stop = bool(args.limit and n >= args.limit)
                if n % 50 == 0:
                    rate = n / (time.time() - progress["started"])
                    eta_h = (len(queue) - n) / rate / 3600 if rate else 0.0
                    logger.info("%d/%d ok=%d (%.2f/s, ETA %.1f h)",
                                n, len(queue), progress["ok"], rate, eta_h)
            if not row["ok"] and not transient:
                logger.warning("vote failed %s: %s", row["id"], row["error"])
            work_q.task_done()
            if stop:
                return

    work_q = Queue()
    for item in queue:
        if args.limit and work_q.qsize() >= args.limit:
            break
        work_q.put(item)
    for _ in range(args.workers):
        work_q.put(None)
    threading.Thread(target=writer, daemon=True).start()
    threads = [threading.Thread(target=worker) for _ in range(args.workers)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    write_q.put(None)
    write_done.wait(timeout=30)
    logger.info("finished %d new votes (%d ok) -> %s",
                progress["done"], progress["ok"], args.out)


if __name__ == "__main__":
    main()
