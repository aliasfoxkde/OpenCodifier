#!/usr/bin/env python3
"""Class-B burst runner (docs/VALIDATION.md §4).

Drives the five-level traffic matrix against an already-running
``opencodifier serve`` and records, per level: client-wall percentiles,
throughput, op-success and error classes, cache-hit rate, healthz
latency sampled during the burst, server resource accounting, and
load-average endpoints.

Levels (§4.2):

| level | clients | requests | bodies | purpose |
|---|---|---|---|---|
| 1 | 8   | 1,024 | unique | baseline concurrency |
| 2 | 32  | 1,024 | unique | scaling |
| 3 | 128 | 1,024 | unique | saturation |
| 4 | 8   | 256   | 128 × 2 | cache-hit pass |
| 5 | 128 | 1,024 unique + 51 malformed | mixed | error discipline under load |

Uniqueness (§4.1): every non-cache body carries a per-request counter
folded into a trailing marker sentence so the exact-decision cache can
never absorb the burst. The marker is asserted grammar-inert (matches no
fact pattern of ``crates/opencodifier-engine/src/facts.rs`` — the
assertion mirrors the auditor's independent check, not its code) and
vocabulary-disjoint from every suite item's question, candidate ids, and
candidate descriptions (the ``make_long_suite.py`` rule, applied globally
because the burst cycles all items). A colliding marker fails the run
loudly instead of quietly contaminating the measurement.

Level 4 re-sends a fixed 128-body set twice — the only pass where cache
hits are expected. Level 5 mixes 51 malformed bodies cycling the ten F2
shapes; their statuses are recorded per shape and judged in the report,
not here.

Client: stdlib threads, one persistent connection per client, per-request
timing, 30 s timeout. Outstanding in-flight requests equal the client
count by construction (no admission control exists server-side — every
request becomes one ``spawn_blocking`` task — so client-side concurrency,
server RSS, and load average are the queue evidence).

Usage (server already up on 8092, discovered pid for resource sampling)::

    python3 run_burst.py --port 8092 --server-pid $(pidof opencodifier) \\
        --level 3 --out-dir runs/
    python3 run_burst.py --port 8092            # all five levels
    python3 run_burst.py --smoke                # 64-request level-1 e2e
"""
from __future__ import annotations

import argparse
import http.client
import json
import queue
import sys
import threading
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
SUITE = REPO / "benchmarks" / "decision-model" / "suite" / "suite.json"
sys.path.insert(0, str(REPO / "benchmarks" / "decision-model" / "runner"))
from resources import ResourceMonitor  # noqa: E402  (campaign monitor, reused)

POLICY = {"min_confidence": 0.8, "verify_below": 0.65, "abstain_below": 0.5,
          "risk": "low"}
LIMITS = {
    "max_input_bytes": 1_048_576,
    "max_questions": 32,
    "max_candidates": 256,
    "max_graph_nodes": 128,
    "max_execution_time": {"secs": 120, "nanos": 0},
    "max_retrieval_results": 64,
}
CLIENT_TIMEOUT_S = 30.0
HEALTHZ_INTERVAL_S = 0.1

LEVELS = {
    1: {"clients": 8, "unique": 1024, "malformed": 0, "cache_bodies": 0},
    2: {"clients": 32, "unique": 1024, "malformed": 0, "cache_bodies": 0},
    3: {"clients": 128, "unique": 1024, "malformed": 0, "cache_bodies": 0},
    4: {"clients": 8, "unique": 0, "malformed": 0, "cache_bodies": 128},
    5: {"clients": 128, "unique": 1024, "malformed": 51, "cache_bodies": 0},
}

# Marker sentences: administrative-vocabulary prose (the make_long_suite
# register), counter zero-padded per its bare-small-integer lesson. The
# templates are fixed; disjointness is asserted globally below.
MARKER = "Ledger note {n:06d}: the quarterly archive lists correspondence, {a} and {b}."
MARKER_FILLERS = ["inventories", "reimbursements", "archived memoranda",
                  "filing queues", "stationery totals", "committee rosters"]
STOP_WORDS = frozenset(
    "a an the and or but if then else when of to in on for with is are was "
    "were be been this that these those it its as at by from into do does".split())

# F2 malformed shapes (VALIDATION.md §6). Each builder returns
# (raw_body_bytes, extra_headers); status codes are recorded, judged later.
#
# Two shapes from the original list of ten were reclassified after the
# burst-r1 run: deleting `state.facts` (200 — `State.facts` is
# `#[serde(default)]`, "omitted means no typed facts") and duplicate
# JSON keys (200 — serde last-key-wins) are accepted by design, not
# malformed, so they are no longer judged under the malformed-gets-4xx
# gate.
def _m_bad_discriminator(body: bytes) -> tuple[bytes, dict]:
    obj = json.loads(body)
    obj["questions"][0]["type"] = "choices"
    return json.dumps(obj).encode(), {}


def _m_oversize(body: bytes) -> tuple[bytes, dict]:
    obj = json.loads(body)
    obj["state"]["text"] = obj["state"]["text"] + " pad." * 250_000
    return json.dumps(obj).encode(), {}


def _m_unknown_format(body: bytes) -> tuple[bytes, dict]:
    return body, {"x-opencodifier-format": "prehistoric-v9"}


def _m_not_json(body: bytes) -> tuple[bytes, dict]:
    return b"this is not json\x00\x01", {}


def _m_empty(body: bytes) -> tuple[bytes, dict]:
    return b"", {}


def _m_deep(body: bytes) -> tuple[bytes, dict]:
    return (b"[" * 5000) + (b"]" * 5000), {}


def _m_too_many_candidates(body: bytes) -> tuple[bytes, dict]:
    obj = json.loads(body)
    base = obj["questions"][0]["candidates"][0]
    obj["questions"][0]["candidates"] = [
        {"id": "cand-%04d" % i, "description": base["description"]}
        for i in range(300)]
    return json.dumps(obj).encode(), {}


def _m_bad_limits_type(body: bytes) -> tuple[bytes, dict]:
    obj = json.loads(body)
    obj["metadata"]["limits"]["max_input_bytes"] = "big"
    return json.dumps(obj).encode(), {}


MALFORMED_SHAPES = [
    ("wrong_discriminator", _m_bad_discriminator),
    ("body_over_1mib", _m_oversize),
    ("unknown_format_header", _m_unknown_format),
    ("non_json_bytes", _m_not_json),
    ("empty_body", _m_empty),
    ("nested_5000", _m_deep),
    ("max_candidates_300", _m_too_many_candidates),
    ("wrong_limits_types", _m_bad_limits_type),
]


def content_words(text: str) -> set[str]:
    return {w for w in
            "".join(c if c.isalnum() else " " for c in text.lower()).split()
            if w not in STOP_WORDS and not w.isdigit()}


def marker_matches_no_fact_pattern(marker: str) -> bool:
    """Independent mirror of facts.rs sentence matching (assertion only)."""
    sentence = marker.strip().rstrip(".").strip()
    tokens = sentence.split()
    lower = [t.strip(",").lower() for t in tokens]
    if len(lower) == 4 and lower[1] == "depends" and lower[2] == "on":
        return False
    if len(lower) == 3 and lower[1] == "is" and lower[2] in (
            "healthy", "degraded", "down", "failing"):
        return False
    if len(lower) == 7 and lower[1:6] == ["comes", "back", "online",
                                          "only", "after"]:
        return False
    if len(tokens) >= 2 and tokens[0].endswith(":"):
        return False  # colon form: candidate HealthList head
    return True


def build_body(item: dict, n: int, request_id: str) -> bytes:
    marker = MARKER.format(n=n, a=MARKER_FILLERS[n % len(MARKER_FILLERS)],
                           b=MARKER_FILLERS[(n // 3) % len(MARKER_FILLERS)])
    assert marker_matches_no_fact_pattern(marker), f"marker matches grammar: {marker}"
    body = {
        "state": {"text": item["context"] + "\n" + marker,
                  # empty facts map: valid IR, and gives the F2
                  # missing-state.facts shape something to delete
                  "facts": {}},
        "questions": [{
            "type": "choice",
            "id": "q-burst-%06d" % n,
            "text": item["question"],
            "candidates": item["candidates"],
        }],
        "policy": POLICY,
        "metadata": {"request_id": request_id, "limits": LIMITS},
    }
    return json.dumps(body, sort_keys=True).encode()


def assert_marker_disjoint(suite_items: list[dict]) -> None:
    """Global marker vocabulary check (burst cycles every item)."""
    marker_words = content_words(
        MARKER.format(n=999999, a=" ".join(MARKER_FILLERS),
                      b=" ".join(MARKER_FILLERS)))
    for item in suite_items:
        forbidden = content_words(item["question"])
        for cand in item["candidates"]:
            forbidden |= content_words(cand["description"])
            forbidden.add(cand["id"].lower())
        hit = marker_words & forbidden
        if hit:
            raise SystemExit(
                f"burst marker vocabulary collides with item "
                f"{item['id']}: {sorted(hit)} — fix MARKER/FILLERS")


def load_avg() -> float:
    try:
        return float(Path("/proc/loadavg").read_text().split()[0])
    except OSError:
        return -1.0


def one_request(conn: http.client.HTTPConnection, path: str, body: bytes,
                headers: dict) -> tuple[float, int, bytes]:
    start = time.monotonic()
    conn.request("POST", path, body=body,
                 headers={"Content-Type": "application/json", **headers})
    resp = conn.getresponse()
    payload = resp.read()
    return time.monotonic() - start, resp.status, payload


def _decision_core(resp: bytes) -> str:
    """The decision itself, independent of cache/trace bookkeeping."""
    try:
        d = json.loads(resp)
    except ValueError:
        return "<unparseable>"
    return json.dumps({k: d.get(k) for k in
                       ("answers", "outcome", "confidence")}, sort_keys=True)


def _cache_hit(resp: bytes) -> bool | None:
    try:
        return bool(json.loads(resp).get("metrics", {}).get("cache_hit"))
    except ValueError:
        return None


def client_worker(port: int, path: str, specs: "queue.Queue[tuple]",
                  results: list) -> None:
    conn = None
    while True:
        try:
            idx, body, headers, kind = specs.get_nowait()
        except queue.Empty:
            return
        if conn is None:
            conn = http.client.HTTPConnection("127.0.0.1", port,
                                              timeout=CLIENT_TIMEOUT_S)
        err_kind = None
        try:
            wall, status, payload = one_request(conn, path, body, headers)
        except TimeoutError:
            wall, status, payload = CLIENT_TIMEOUT_S, 0, b""
            err_kind = "timeout"
        except http.client.HTTPException as exc:
            wall, status, payload = 0.0, 0, b""
            err_kind = f"http:{type(exc).__name__}"
            conn.close()
            conn = None
        except OSError as exc:
            wall, status, payload = 0.0, 0, b""
            err_kind = f"oserror:{type(exc).__name__}"
            try:
                conn.close()
            except OSError:
                pass
            conn = None
        cache_hit = None
        outcome = None
        if status == 200 and kind in ("unique", "cache"):
            try:
                parsed = json.loads(payload)
                cache_hit = bool(parsed.get("metrics", {}).get("cache_hit"))
                outcome = parsed.get("outcome")
            except json.JSONDecodeError:
                pass
        results.append({
            "kind": kind, "wall_s": wall, "status": status,
            "err_kind": err_kind, "cache_hit": cache_hit,
            "outcome": outcome,
            "body_head": (payload[:160].decode("utf-8", "replace")
                          if status != 200 else ""),
        })


def healthz_sampler(port: int, stop_evt: threading.Event,
                    latencies: list) -> None:
    while not stop_evt.is_set():
        start = time.monotonic()
        try:
            conn = http.client.HTTPConnection("127.0.0.1", port, timeout=5.0)
            conn.request("GET", "/v1/healthz")
            resp = conn.getresponse()
            resp.read()
            if resp.status == 200:
                latencies.append(time.monotonic() - start)
            conn.close()
        except OSError:
            pass
        stop_evt.wait(HEALTHZ_INTERVAL_S)


def pct(sorted_vals: list[float], q: float) -> float:
    if not sorted_vals:
        return 0.0
    idx = min(len(sorted_vals) - 1, max(0, int(round(q * len(sorted_vals))) - 1))
    return sorted_vals[idx]


def run_level(level: int, spec: dict, items: list[dict], port: int,
              out_dir: Path, server_pid: int | None,
              control_body: bytes) -> dict:
    clients = spec["clients"]
    specs: "queue.Queue[tuple]" = queue.Queue()
    n = 0
    for _ in range(spec["unique"]):
        specs.put((n, build_body(items[n % len(items)], n,
                                 f"burst-l{level}-{n:06d}"), {},
                   "unique"))
        n += 1
    cache_pool = []
    for i in range(spec["cache_bodies"]):
        cache_pool.append(build_body(items[i % len(items)], n,
                                     f"burst-l{level}-cache-{i:06d}"))
    # (pool build must not advance n: total counts enqueued requests only)
    for body in cache_pool:
        specs.put((n, body, {}, "cache"))
        n += 1
        specs.put((n, body, {}, "cache"))
        n += 1
    malformed_counts: dict[str, int] = {}
    for i in range(spec["malformed"]):
        name, builder = MALFORMED_SHAPES[i % len(MALFORMED_SHAPES)]
        malformed_counts[name] = malformed_counts.get(name, 0) + 1
        try:
            body, headers = builder(cache_pool[0] if cache_pool
                                    else build_body(items[0], 999_999, "x"))
        except Exception as exc:  # builder itself failed — record, continue
            body, headers = b"{}", {}
            name = name + ":BUILDER_ERROR:" + type(exc).__name__
        specs.put((n, body, headers, f"malformed:{name}"))
        n += 1

    total = n
    # control request: pre-burst reference for the poisoning check
    conn = http.client.HTTPConnection("127.0.0.1", port,
                                      timeout=CLIENT_TIMEOUT_S)
    _, ctl_status, ctl_before = one_request(conn, "/v1/decide", control_body,
                                            {})
    conn.close()

    results: list = []
    health_latencies: list[float] = []
    stop_evt = threading.Event()
    sampler = threading.Thread(target=healthz_sampler,
                               args=(port, stop_evt, health_latencies),
                               daemon=True)
    sampler.start()
    monitor = None
    if server_pid:
        # A dead/mistyped pid would sample nothing and record zeros as if
        # they were measurements; refuse that silence up front.
        if not Path(f"/proc/{server_pid}/status").exists():
            print("WARNING: server pid %s has no /proc entry; RSS "
                  "unmeasured this level" % server_pid, flush=True)
        else:
            monitor = ResourceMonitor(root_pid=server_pid)
            monitor.start()
    load_start = load_avg()
    threads = [threading.Thread(target=client_worker,
                                args=(port, "/v1/decide", specs, results))
               for _ in range(clients)]
    t0 = time.monotonic()
    for t in threads:
        t.start()
    deadline = t0 + 1800.0
    done = 0
    while done < total:
        time.sleep(1.0)
        done = len(results)
        print("  level %d: %d/%d" % (level, done, total), flush=True)
        if time.monotonic() > deadline:
            print("  level %d: DEADLINE exceeded with %d/%d done"
                  % (level, done, total), flush=True)
            break
    for t in threads:
        t.join()
    wall = time.monotonic() - t0
    load_end = load_avg()
    stop_evt.set()
    sampler.join(timeout=2.0)
    if monitor:
        monitor.stop()

    # Post-burst control request. The poisoning check is the DECISION
    # core (answers/outcome/confidence), not raw bytes: a second send of
    # the identical body legitimately hits the exact-decision cache,
    # which flips metrics.cache_hit and rewrites the trace. Raw-byte
    # identity is kept as information — it holds exactly when the cache
    # replays the stored response verbatim.
    conn = http.client.HTTPConnection("127.0.0.1", port,
                                      timeout=CLIENT_TIMEOUT_S)
    _, ctl_status_after, ctl_after = one_request(conn, "/v1/decide",
                                                 control_body, {})
    conn.close()

    walls = sorted(r["wall_s"] for r in results if r["kind"] != "malformed")
    ok = [r for r in results if r["kind"] != "malformed"
          and r["status"] == 200]
    status_counts: dict[str, int] = {}
    for r in results:
        key = str(r["status"]) if r["status"] else r["err_kind"] or "unknown"
        status_counts[key] = status_counts.get(key, 0) + 1
    malformed_rows = {}
    for r in results:
        if r["kind"].startswith("malformed:"):
            shape = r["kind"].split(":", 1)[1]
            e = malformed_rows.setdefault(shape, {"n": 0, "statuses": {}})
            e["n"] += 1
            e["statuses"][str(r["status"])] = \
                e["statuses"].get(str(r["status"]), 0) + 1
    cache_rows = [r for r in results if r["kind"] == "cache"]
    hits = sorted(hlat for hlat in health_latencies)

    summary = {
        "level": level,
        "spec": spec,
        "clients": clients,
        "requests": total,
        "wall_s": round(wall, 3),
        "throughput_rps": round(total / wall, 2),
        "p50_ms": round(pct(walls, 0.50) * 1000, 2),
        "p95_ms": round(pct(walls, 0.95) * 1000, 2),
        "p99_ms": round(pct(walls, 0.99) * 1000, 2),
        "op_success_rate": round(len(ok) / max(1, len(walls)), 5),
        "status_counts": dict(sorted(status_counts.items())),
        "cache_hit_rate": round(
            sum(1 for r in cache_rows if r["cache_hit"]) / len(cache_rows), 4)
            if cache_rows else None,
        "healthz": {
            "n": len(hits),
            "p50_ms": round(pct(hits, 0.50) * 1000, 2),
            "p95_ms": round(pct(hits, 0.95) * 1000, 2),
            "p99_ms": round(pct(hits, 0.99) * 1000, 2),
        },
        "malformed": malformed_rows,
        "control_request": {
            "pre_status": ctl_status,
            "post_status": ctl_status_after,
            "decision_identical": _decision_core(ctl_before)
            == _decision_core(ctl_after),
            "cache_hit_after": _cache_hit(ctl_after),
            "raw_byte_identical": ctl_before == ctl_after,
        },
        "max_in_flight": clients,
        "load_avg_start": load_start,
        "load_avg_end": load_end,
        "resources": monitor.report() if monitor else "server-pid not given",
    }
    out = out_dir / f"burst_level{level}.json"
    out.write_text(json.dumps({
        "summary": summary,
        "rows": results,
    }, indent=1))
    print("level %d done: %s" % (level, json.dumps(
        {k: summary[k] for k in ("requests", "throughput_rps", "p50_ms",
                                 "p95_ms", "p99_ms", "op_success_rate",
                                 "cache_hit_rate", "status_counts")},
        indent=1)))
    return summary


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=8092)
    ap.add_argument("--level", type=int, choices=sorted(LEVELS),
                    default=None, help="run one level (default: all 5)")
    ap.add_argument("--suite", type=Path, default=SUITE)
    ap.add_argument("--out-dir", type=Path, default=Path("runs"))
    ap.add_argument("--server-pid", type=int, default=None,
                    help="pid of `opencodifier serve` for resource sampling")
    ap.add_argument("--smoke", action="store_true",
                    help="level-1 e2e with 64 requests (not a result)")
    args = ap.parse_args()

    suite = json.loads(args.suite.read_text())
    items = suite["items"]
    assert_marker_disjoint(items)

    control_body = build_body(items[0], 999_998, "burst-control")
    args.out_dir.mkdir(parents=True, exist_ok=True)

    levels = [args.level] if args.level else sorted(LEVELS)
    if args.smoke:
        LEVELS[1] = {"clients": 4, "unique": 64, "malformed": 0,
                     "cache_bodies": 0}
        levels = [1]
    summaries = {}
    for lv in levels:
        print(f"=== level {lv}: {LEVELS[lv]} ===", flush=True)
        summaries[lv] = run_level(lv, LEVELS[lv], items, args.port,
                                  args.out_dir, args.server_pid, control_body)
    (args.out_dir / "burst_summary.json").write_text(
        json.dumps(summaries, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
