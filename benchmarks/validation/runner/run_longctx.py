#!/usr/bin/env python3
"""Class-C long-context runner (docs/VALIDATION.md §5).

Drives `longctx_tiers.json` — the committed 120-item suite padded to
the 4k / 32k / 128k estimated-token tiers plus the 256k socket probe —
against an ``opencodifier serve`` it spawns itself, one configuration
per arm:

* ``full``       — default stack, no focus.
* ``focus512``   — ``--focus-budget 512``.
* ``focus4096``  — ``--focus-budget 4096``.
* ``escalate``   — the llamacpp-feature build with
  ``--ladder ladders/fusion-v2.json --llama URL``; the llama-server is
  expected to be up and healthy at ``--llama`` before traffic starts.

Requests are sequential by design: the subject is per-request cost
growth with state size, not concurrency. Every arm sends byte-identical
bodies (built once from the tier items), so ``--compare PREV.json``
replays the determinism gate across arms and reruns.

The 256k probe is recorded separately: the socket ceiling gate wants a
clean 413 with the documented envelope error, never a partial parse.
"""
from __future__ import annotations

import argparse
import http.client
import json
import statistics
import subprocess
import sys
import time
from pathlib import Path

CLIENT_TIMEOUT_S = 30.0          # deadline-honesty gate bound for
                                 # non-escalate arms is 5 s (§5.4)
HEALTHZ_WAIT_S = 120.0

POLICY = {
    "min_confidence": 0.8,
    "verify_below": 0.65,
    "abstain_below": 0.5,
    "risk": "low",
}
LIMITS = {
    "max_input_bytes": 1_048_576,
    "max_questions": 32,
    "max_candidates": 256,
    "max_graph_nodes": 128,
    "max_execution_time": {"secs": 120, "nanos": 0},
    "max_retrieval_results": 64,
}
# The committed short-suite engine row the stability gate anchors to
# (PLAN Phase 15): per-class correctness at the unpadded shape.
SHORT_SUITE_ANCHOR = {
    "metadata_match": 0.88,
    "lexical_semantic": 0.23,
    "relational_compositional": 0.950,
}


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


def build_body(item: dict) -> bytes:
    question = {
        "type": "choice",
        "id": item["id"],
        "text": item["question"],
        "candidates": item["candidates"],
    }
    body = {
        "state": {"text": item["context"], "facts": {}},
        "questions": [question],
        "policy": POLICY,
        "metadata": {
            "request_id": f"longctx-{item['id']}",
            "limits": LIMITS,
        },
    }
    return json.dumps(body, ensure_ascii=False).encode("utf-8")


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
    ev: dict = {"focus": 0, "rungs_fired": 0}
    for e in entries:
        node = e.get("node", "")
        detail = e.get("detail", {})
        if node.startswith("focus_"):
            ev["focus"] += 1
        for k in ("policy_source", "rungs_fired", "rung_chain"):
            v = detail.get(k)
            if v is not None:
                if isinstance(v, dict) and "value" in v:
                    v = v["value"]
                if k == "rungs_fired" and isinstance(v, int):
                    ev["rungs_fired"] = max(ev["rungs_fired"], v)
                elif k != "rungs_fired":
                    ev[k] = v
    return ev


def one_request(port: int, body: bytes) -> dict:
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
        elif resp.status == 413:
            # keep the envelope error text for the socket-ceiling record
            row["body_head"] = payload[:200].decode("utf-8", "replace")
    except (OSError, TimeoutError, http.client.HTTPException) as exc:
        row["err_kind"] = type(exc).__name__
    return row


def summarize(rows: list[dict], wall_s: float,
              compare_path: Path | None) -> dict:
    by_tier: dict[str, list[dict]] = {}
    for r in rows:
        by_tier.setdefault(r["tier"], []).append(r)
    tiers: dict = {}
    for tier, trs in sorted(by_tier.items()):
        correct = sum(1 for r in trs
                      if r["pred"] is not None and r["pred"] == r["gold"])
        per_class: dict = {}
        for cls in SHORT_SUITE_ANCHOR:
            sub = [r for r in trs if r.get("class") == cls]
            if sub:
                c = sum(1 for r in sub
                        if r["pred"] is not None and r["pred"] == r["gold"])
                anchor = SHORT_SUITE_ANCHOR[cls]
                per_class[cls] = {
                    "correct": round(c / len(sub), 4),
                    "delta_vs_short": round(c / len(sub) - anchor, 4),
                }
        walls = sorted(r["wall_ms"] for r in trs
                       if r["wall_ms"] is not None)
        escalations = sum(1 for r in trs
                          if r.get("evidence", {})
                          .get("rungs_fired", 0) > 0)
        focus_hits = sum(1 for r in trs
                         if r.get("evidence", {}).get("focus", 0) > 0)
        outcomes: dict[str, int] = {}
        for r in trs:
            k = r["outcome"] or f"err:{r['err_kind']}" \
                if r["outcome"] or r["err_kind"] else f"http_{r['status']}"
            outcomes[k] = outcomes.get(k, 0) + 1
        tiers[tier] = {
            "items": len(trs),
            "correct": round(correct / len(trs), 6) if trs else 0.0,
            "per_class_vs_short": per_class,
            "latency_ms": {
                "p50": round(pct(walls, 0.50), 3),
                "p95": round(pct(walls, 0.95), 3),
                "p99": round(pct(walls, 0.99), 3),
            },
            "escalation_rate": round(escalations / len(trs), 6)
            if trs else 0.0,
            "focus_engaged": focus_hits,
            "outcomes": dict(sorted(outcomes.items())),
        }
    summary: dict = {"tiers": tiers, "wall_s": round(wall_s, 2),
                     "load_avg": load_avg()}
    if compare_path is not None:
        prev = json.loads(compare_path.read_text())
        prev_core = {(r["tier"], r["id"]): r.get("decision_core")
                     for r in prev["rows"] if "decision_core" in r}
        mismatch = [ (r["tier"], r["id"]) for r in rows
                     if "decision_core" in r
                     and prev_core.get((r["tier"], r["id"]))
                     != r["decision_core"]]
        summary["replay_mismatches"] = len(mismatch)
        summary["replay_mismatch_keys"] = [list(k) for k in mismatch[:10]]
    return summary


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--binary", type=Path, required=True)
    ap.add_argument("--suite", type=Path, required=True)
    ap.add_argument("--arm", choices=("full", "focus512", "focus4096",
                                      "escalate"), required=True)
    ap.add_argument("--port", type=int, required=True)
    ap.add_argument("--ladder", type=Path, default=None)
    ap.add_argument("--llama", type=str, default=None)
    ap.add_argument("--llama-model-id", type=str, default=None)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--compare", type=Path, default=None)
    args = ap.parse_args()

    suite = json.loads(args.suite.read_text())
    args.out.parent.mkdir(parents=True, exist_ok=True)

    cmd = [str(args.binary), "serve", "--bind", f"127.0.0.1:{args.port}"]
    if args.arm == "focus512":
        cmd += ["--focus-budget", "512"]
    elif args.arm == "focus4096":
        cmd += ["--focus-budget", "4096"]
    elif args.arm == "escalate":
        if not (args.ladder and args.llama):
            ap.error("--arm escalate needs --ladder and --llama")
        cmd += ["--ladder", str(args.ladder), "--llama", args.llama]
        if args.llama_model_id:
            cmd += ["--llama-model-id", args.llama_model_id]
        # the rung must be up before traffic: a dead model endpoint is
        # F1's subject, not class C's
        try:
            conn = http.client.HTTPConnection(
                args.llama.split("//", 1)[1], timeout=5.0)
            conn.request("GET", "/health")
            if conn.getresponse().status != 200:
                raise OSError("not 200")
            conn.close()
        except OSError as exc:
            print(f"llama endpoint {args.llama} not healthy: {exc}")
            return 1
    log_path = args.out.with_suffix(".serve.log")
    with log_path.open("w") as log_fh:
        proc = subprocess.Popen(cmd, stdout=log_fh,
                                stderr=subprocess.STDOUT)
        print("serve pid %d: %s" % (proc.pid, " ".join(cmd)), flush=True)
        try:
            if not wait_healthz(args.port, HEALTHZ_WAIT_S):
                print("serve never became healthy; see %s" % log_path)
                return 1
            rows: list = []
            probe_rows: list = []
            t0 = time.monotonic()
            for tier_name in ("L1", "L2", "L3"):
                for item in suite["tiers"][tier_name]["items"]:
                    row = one_request(args.port, build_body(item))
                    row.update({"tier": tier_name, "id": item["id"],
                                "class": item.get("class"),
                                "gold": item["answer"]})
                    rows.append(row)
                    if len(rows) % 60 == 0:
                        print("  %s: %d items" % (tier_name, len(rows)),
                              flush=True)
            wall_s = time.monotonic() - t0
            for item in suite["tiers"]["probe"]["items"]:
                prow = one_request(args.port, build_body(item))
                prow.update({"tier": "probe", "id": item["id"],
                             "class": item.get("class"),
                             "gold": item["answer"]})
                probe_rows.append(prow)
            rows.sort(key=lambda r: (r["tier"], r["id"]))
            summary = summarize(rows, wall_s, args.compare)
            summary["socket_probe"] = probe_rows
            args.out.write_text(json.dumps(
                {"summary": summary, "rows": rows}, indent=1))
            print(json.dumps(summary, indent=1))
            gates = []
            for tier, ts in summary["tiers"].items():
                gates.append((f"{tier}:parity_vs_full<=3pp",
                              all(abs(v["delta_vs_short"]) <= 0.03
                                  for v in ts["per_class_vs_short"]
                                  .values())))
                gates.append((f"{tier}:p99_deadline_honest",
                              ts["latency_ms"]["p99"] <= 5000.0))
            probe_ok = all(p["status"] == 413 for p in probe_rows) \
                if probe_rows else False
            gates.append(("socket_probe_413", probe_ok))
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


if __name__ == "__main__":
    sys.exit(main())
