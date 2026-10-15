#!/usr/bin/env python3
"""Class-A session-loop runner (docs/VALIDATION.md §3).

Drives the 24-session x 50-turn incident-triage plan against an
``opencodifier serve`` it spawns itself, so every run is hermetic:
fresh process, fresh cache, the exact flags the mode names. Modes:

* ``serial``     — 24 sessions back to back (run 1 / replay run).
* ``interleave`` — 8 sessions concurrently (run 2, the cross-session-
                   bleed check; each session's turns stay in order).
* ``focus``      — same traffic as serial, but serve runs with
                   ``--focus-budget N`` (run 3, the D18 A/B).

Every turn's request body is sent byte-exact from ``session_plan.json``
(``request_bodies`` already carry the repeat probes' bodies verbatim).
Per-request records keep the wire's own words: status, ``outcome``,
the chosen candidate, the four ``DecisionMetrics`` fields, and the
trace evidence the deciding-node inference uses (threshold
outcome/verifier, ``policy_source``, ``rungs_fired``/``rung_chain``).

``--compare PREV.json`` re-plays the determinism gate: answers,
outcomes, and confidences must match the prior run byte-for-byte.
"""
from __future__ import annotations

import argparse
import http.client
import json
import queue
import statistics
import subprocess
import sys
import threading
import time
from pathlib import Path

CLIENT_TIMEOUT_S = 30.0  # spec §3.2
HEALTHZ_WAIT_S = 120.0


def load_avg() -> float:
    try:
        return float(Path("/proc/loadavg").read_text().split()[0])
    except OSError:
        return -1.0


def pct(sorted_vals: list[float], q: float) -> float:
    if not sorted_vals:
        return 0.0
    idx = min(len(sorted_vals) - 1, int(q * len(sorted_vals)))
    return sorted_vals[idx]


def wait_healthz(port: int, deadline_s: float) -> bool:
    deadline = time.monotonic() + deadline_s
    while time.monotonic() < deadline:
        try:
            conn = http.client.HTTPConnection("127.0.0.1", port, timeout=2.0)
            conn.request("GET", "/v1/healthz")
            ok = conn.getresponse().status == 200
            conn.close()
            if ok:
                return True
        except OSError:
            pass
        time.sleep(0.5)
    return False


def trace_evidence(entries: list[dict]) -> dict:
    """Compact deciding-node evidence per spec §3.3 (inferable, not typed)."""
    ev: dict = {}
    for e in entries:
        node = e.get("node", "")
        detail = e.get("detail", {})
        if node == "threshold":
            for k in ("outcome", "verifier"):
                v = detail.get(k)
                if isinstance(v, dict) and "value" in v:
                    ev[f"threshold_{k}"] = v["value"]
        for k in ("policy_source", "rungs_fired", "rung_chain"):
            v = detail.get(k)
            if v is not None:
                if isinstance(v, dict) and "value" in v:
                    v = v["value"]
                ev[k] = v
    return ev


def one_turn(port: int, body: bytes) -> dict:
    start = time.monotonic()
    row: dict = {"wall_ms": None, "status": None, "err_kind": None}
    try:
        conn = http.client.HTTPConnection("127.0.0.1", port,
                                          timeout=CLIENT_TIMEOUT_S)
        conn.request("POST", "/v1/decide", body=body,
                     headers={"Content-Type": "application/json"})
        resp = conn.getresponse()
        payload = resp.read()
        conn.close()
        row["wall_ms"] = round((time.monotonic() - start) * 1000.0, 3)
        row["status"] = resp.status
        if resp.status == 200:
            d = json.loads(payload)
            answer = d["answers"][0] if d.get("answers") else None
            row["outcome"] = d.get("outcome")
            row["pred"] = answer.get("choice") if answer else None
            row["confidence"] = (d.get("confidence") or {}).get(
                "calibrated_confidence")
            row["metrics"] = d.get("metrics")
            row["evidence"] = trace_evidence(
                (d.get("trace") or {}).get("entries", []))
            row["decision_core"] = json.dumps(
                {"answers": d.get("answers"), "outcome": d.get("outcome"),
                 "confidence": d.get("confidence")}, sort_keys=True)
    except (OSError, TimeoutError, http.client.HTTPException) as exc:
        row["err_kind"] = type(exc).__name__
    return row


def run_session(port: int, session: dict, rows: list,
                lock: threading.Lock) -> None:
    bodies = session["request_bodies"]
    for turn in session["turns"]:
        n = turn["turn"]
        body = bodies[str(n)].encode("utf-8")
        row = one_turn(port, body)
        row.update({
            "session": session["session_id"],
            "turn": n,
            "kind": turn["kind"],
            "gold": turn.get("gold"),
            "question_id": (turn.get("question") or {}).get("id"),
        })
        if turn["kind"] == "repeat_probe":
            row["probe_of_turn"] = turn["probe_of_turn"]
        with lock:
            rows.append(row)


def run_all(port: int, plan: dict, mode: str, rows: list) -> None:
    lock = threading.Lock()
    if mode == "interleave":
        pending: "queue.Queue[dict]" = queue.Queue()
        for s in plan["sessions"]:
            pending.put(s)
        def worker() -> None:
            while True:
                try:
                    s = pending.get_nowait()
                except queue.Empty:
                    return
                run_session(port, s, rows, lock)
                pending.task_done()
        threads = [threading.Thread(target=worker) for _ in range(8)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
    else:
        for s in plan["sessions"]:
            run_session(port, s, rows, lock)


def slope_ms_per_turn(rows: list[dict]) -> float:
    """Least-squares slope of per-turn p50 wall against turn index."""
    by_turn: dict[int, list[float]] = {}
    for r in rows:
        if r["kind"] == "decision" and r["wall_ms"] is not None:
            by_turn.setdefault(r["turn"], []).append(r["wall_ms"])
    xs = sorted(by_turn)
    if len(xs) < 2:
        return 0.0
    ys = [statistics.median(by_turn[x]) for x in xs]
    n = len(xs)
    mx = sum(xs) / n
    my = sum(ys) / n
    denom = sum((x - mx) ** 2 for x in xs)
    if denom == 0:
        return 0.0
    return sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / denom


def summarize(rows: list[dict], plan: dict, wall_s: float,
              compare_path: Path | None) -> dict:
    decisions = [r for r in rows if r["kind"] == "decision"]
    probes = [r for r in rows if r["kind"] == "repeat_probe"]
    ok200 = sum(1 for r in rows if r["status"] == 200)
    correct = sum(1 for r in decisions
                  if r.get("pred") is not None and r["pred"] == r["gold"])
    probe_ok = sum(1 for r in probes
                   if r.get("pred") is not None and r["pred"] == r["gold"])
    probe_hit = sum(1 for r in probes
                    if (r.get("metrics") or {}).get("cache_hit") is True)
    outcomes: dict[str, int] = {}
    for r in decisions:
        outcomes[r["outcome"] or f"err:{r['err_kind']}"] = \
            outcomes.get(r["outcome"] or f"err:{r['err_kind']}", 0) + 1
    walls = sorted(r["wall_ms"] for r in rows if r["wall_ms"] is not None)
    narrowing = [
        (r["metrics"]["candidates_out"] / r["metrics"]["candidates_in"])
        for r in decisions
        if r.get("metrics") and r["metrics"].get("candidates_in")]
    gold_by_key = {}
    for s in plan["sessions"]:
        for t in s["turns"]:
            gold_by_key[(s["session_id"], t["turn"])] = t.get("gold")
    summary: dict = {
        "requests": len(rows),
        "http_200": ok200,
        "op_success_rate": round(ok200 / len(rows), 6) if rows else 0.0,
        "decision_correct": round(correct / len(decisions), 6)
        if decisions else 0.0,
        "probe_correct": round(probe_ok / len(probes), 6) if probes else 0.0,
        "probe_cache_hit_rate": round(probe_hit / len(probes), 6)
        if probes else 0.0,
        "outcomes": dict(sorted(outcomes.items())),
        "abstain_rate": round(
            outcomes.get("abstain", 0) / len(decisions), 6)
        if decisions else 0.0,
        "latency_ms": {
            "p50": round(pct(walls, 0.50), 3),
            "p95": round(pct(walls, 0.95), 3),
            "p99": round(pct(walls, 0.99), 3),
        },
        "growth_slope_ms_per_turn": round(slope_ms_per_turn(rows), 5),
        "mean_narrowing_ratio": round(statistics.mean(narrowing), 6)
        if narrowing else None,
        "wall_s": round(wall_s, 2),
        "load_avg": load_avg(),
    }
    if compare_path is not None:
        prev = json.loads(compare_path.read_text())
        prev_core = {
            (r["session"], r["turn"]): r.get("decision_core")
            for r in prev["rows"]}
        mismatch = []
        for r in rows:
            key = (r["session"], r["turn"])
            if prev_core.get(key) != r.get("decision_core"):
                mismatch.append(key)
        summary["replay_mismatches"] = len(mismatch)
        summary["replay_mismatch_keys"] = [list(k) for k in mismatch[:10]]
    return summary


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--plan", type=Path, required=True)
    ap.add_argument("--binary", type=Path, required=True)
    ap.add_argument("--mode", choices=("serial", "interleave", "focus"),
                    required=True)
    ap.add_argument("--focus-budget", type=int, default=None)
    ap.add_argument("--port", type=int, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--ladder", type=Path, default=None,
                    help="passed through to serve --ladder (F4 arm)")
    ap.add_argument("--compare", type=Path, default=None,
                    help="prior run JSON: replay determinism gate")
    ap.add_argument("--log", type=Path, default=None,
                    help="serve stdout/stderr capture (default: out.dir)")
    args = ap.parse_args()
    if args.mode == "focus" and not args.focus_budget:
        ap.error("--focus-budget is required with --mode focus")

    plan = json.loads(args.plan.read_text())
    args.out.parent.mkdir(parents=True, exist_ok=True)
    log_path = args.log or (args.out.with_suffix(".serve.log"))
    cmd = [str(args.binary), "serve",
           "--bind", f"127.0.0.1:{args.port}"]
    if args.mode == "focus":
        cmd += ["--focus-budget", str(args.focus_budget)]
    if args.ladder is not None:
        cmd += ["--ladder", str(args.ladder)]
    log_fh = log_path.open("w")
    proc = subprocess.Popen(cmd, stdout=log_fh, stderr=subprocess.STDOUT)
    print("serve pid %d: %s" % (proc.pid, " ".join(cmd)), flush=True)
    try:
        if not wait_healthz(args.port, HEALTHZ_WAIT_S):
            print("serve never became healthy; see %s" % log_path)
            return 1
        rows: list = []
        t0 = time.monotonic()
        run_all(args.port, plan, args.mode, rows)
        wall_s = time.monotonic() - t0
        rows.sort(key=lambda r: (r["session"], r["turn"]))
        summary = summarize(rows, plan, wall_s, args.compare)
        args.out.write_text(json.dumps(
            {"summary": summary, "rows": rows}, indent=1))
        print(json.dumps(summary, indent=1))
        gates = [
            ("op_success", summary["op_success_rate"] == 1.0),
            ("decision_correct>=0.95", summary["decision_correct"] >= 0.95),
            ("probes_correct_1.0", summary["probe_correct"] == 1.0),
            ("probe_cache_hit_1.0", summary["probe_cache_hit_rate"] == 1.0),
            ("p99<=12ms", summary["latency_ms"]["p99"] <= 12.0),
            ("slope<=0.05ms/turn",
             summary["growth_slope_ms_per_turn"] <= 0.05),
            ("abstain<=0.20", summary["abstain_rate"] <= 0.20),
        ]
        if args.compare is not None:
            gates.append(("replay_identical",
                          summary["replay_mismatches"] == 0))
        print("GATES: " + " ".join(
            f"{n}={'PASS' if ok else 'FAIL'}" for n, ok in gates))
        return 0 if all(ok for _, ok in gates) else 2
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
        log_fh.close()


if __name__ == "__main__":
    sys.exit(main())
