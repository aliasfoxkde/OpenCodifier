#!/usr/bin/env python3
"""Failure-injection dimension driver (docs/VALIDATION.md §6).

F1 — model-rung unavailable: class-A traffic through the llamacpp
     build with the fusion-v2 ladder; the llama rung is `kill -9`ed at
     a scheduled turn, restarted 30 s later, and the traffic continues.
     Must hold: in-flight requests resolve (typed error or answer)
     within client timeout + 5 s — never a hang; serve healthz 200
     within 1 s of the kill; the first request after restart succeeds;
     a request that failed mid-walk serves NO cached decision on retry.

F3 — deadline pressure (D32): (a) a request declaring
     max_execution_time 121 s is refused at the wire; (b) the rung
     behind a +60 s delay proxy produces a typed timeout inside
     deadline + 5 s with the server healthy after.

F4 — verifier disagreement: the proofs-only ladder on class A forces
     non-proof questions down the verify path with no verifier
     configured: outcome `verify` with `verifier: "none"` in the trace,
     never an error, counted not failed.

F2 (the malformed burst) lives in run_burst.py level 5 and is recorded
from that run, not here.
"""
from __future__ import annotations

import argparse
import http.client
import json
import socket
import subprocess
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from run_sessions import one_turn  # noqa: E402  (same request shape)

CLIENT_TIMEOUT_S = 30.0
RESOLVE_GRACE_S = 5.0            # F1: typed resolution ≤ timeout + 5 s
RUNG_RESTART_DELAY_S = 30.0


def wait_healthz(port: int, deadline_s: float, path: str = "/v1/healthz") -> bool:
    deadline = time.monotonic() + deadline_s
    while time.monotonic() < deadline:
        try:
            conn = http.client.HTTPConnection("127.0.0.1", port, timeout=2.0)
            conn.request("GET", path)
            ok = conn.getresponse().status == 200
            conn.close()
            if ok:
                return True
        except OSError:
            pass
        time.sleep(0.25)
    return False


def healthz_wall(port: int, path: str = "/v1/healthz") -> float | None:
    start = time.monotonic()
    try:
        conn = http.client.HTTPConnection("127.0.0.1", port, timeout=5.0)
        conn.request("GET", path)
        conn.getresponse().read()
        conn.close()
        return (time.monotonic() - start) * 1000.0
    except OSError:
        return None


class DelayProxy:
    """TCP proxy that sleeps before relaying (F3's +60 s per call)."""

    def __init__(self, listen: int, target: int, delay_s: float) -> None:
        self.listen = listen
        self.target = target
        self.delay_s = delay_s
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None

    def start(self) -> None:
        self._thread = threading.Thread(target=self._serve, daemon=True)
        self._thread.start()

    def stop(self) -> None:
        self._stop.set()

    def _serve(self) -> None:
        srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        srv.bind(("127.0.0.1", self.listen))
        srv.listen(16)
        srv.settimeout(0.5)
        while not self._stop.is_set():
            try:
                client, _ = srv.accept()
            except socket.timeout:
                continue
            threading.Thread(target=self._pipe,
                             args=(client,), daemon=True).start()

    def _pipe(self, client: socket.socket) -> None:
        try:
            if self._stop.wait(self.delay_s):
                client.close()
                return
            upstream = socket.create_connection(
                ("127.0.0.1", self.target), timeout=130.0)
            client.settimeout(130.0)
            upstream.settimeout(130.0)
            streams = [(client, upstream), (upstream, client)]

            def relay(src: socket.socket, dst: socket.socket) -> None:
                try:
                    while True:
                        data = src.recv(65536)
                        if not data:
                            break
                        dst.sendall(data)
                except OSError:
                    pass
                finally:
                    try:
                        dst.shutdown(socket.SHUT_WR)
                    except OSError:
                        pass

            workers = [threading.Thread(target=relay, args=s, daemon=True)
                       for s in streams]
            for w in workers:
                w.start()
            for w in workers:
                w.join()
        except OSError:
            pass
        finally:
            client.close()


class Rung:
    """llama-server lifecycle owned here so F1 can kill and revive it."""

    def __init__(self, binary: Path, model: Path, port: int, log: Path,
                 ctx: int = 40960, decision_seqs: int = 8) -> None:
        self.binary = binary
        self.model = model
        self.port = port
        self.log = log
        # Sized to the workload's largest prompt, not a round number: the
        # fork's decision seqs share the prompt KV, so the binding quantity
        # is the decision prompt itself — 8192 killed every L2 escalation
        # (rerun-1), and 24576 still did (rerun-2: an L2 state is ~32k
        # tokens before the question is appended). 40960 covers L1+L2;
        # L3 (~131k) is out of the CPU rung's envelope entirely (RV-009/
        # RV-011, results/VALIDATION.md) and fails honestly there.
        self.ctx = ctx
        self.decision_seqs = decision_seqs
        self.proc: subprocess.Popen | None = None

    def start(self, wait_s: float = 120.0) -> bool:
        fh = self.log.open("a")
        cmd = [str(self.binary), "-m", str(self.model),
               "--port", str(self.port), "-c", str(self.ctx),
               "-t", "8", "-ngl", "0"]
        if self.decision_seqs:
            cmd += ["--decision-seqs", str(self.decision_seqs)]
        self.proc = subprocess.Popen(cmd, stdout=fh, stderr=subprocess.STDOUT)
        ok = wait_healthz(self.port, wait_s, "/health")
        fh.close()
        return ok

    def kill9(self) -> None:
        if self.proc is not None:
            self.proc.kill()
            self.proc.wait()
            self.proc = None

    def alive(self) -> bool:
        return self.proc is not None and self.proc.poll() is None


def f1(injector: argparse.Namespace, out: dict) -> None:
    plan = json.loads(injector.plan.read_text())
    sessions = plan["sessions"][:injector.sessions]
    kill_at = (injector.kill_session, injector.kill_turn)
    rung = Rung(injector.rung_binary, injector.rung_model,
                injector.rung_port, injector.rung_log)
    records: list = []
    serve_cmd = [str(injector.binary), "serve",
                 "--bind", f"127.0.0.1:{injector.port}",
                 "--ladder", str(injector.ladder),
                 "--llama", f"http://127.0.0.1:{injector.rung_port}"]
    if injector.llama_model_id:
        serve_cmd += ["--llama-model-id", injector.llama_model_id]
    serve_log = injector.out_dir / "f1-serve.log"
    with serve_log.open("w") as sfh:
        serve = subprocess.Popen(serve_cmd, stdout=sfh,
                                 stderr=subprocess.STDOUT)
        try:
            assert wait_healthz(injector.port, 120.0), "serve not healthy"
            assert rung.start(), "rung not healthy"
            first_after_restart_done = False
            for session in sessions:
                sid = session["session_id"]
                for turn in session["turns"]:
                    n = turn["turn"]
                    body = session["request_bodies"][str(n)].encode("utf-8")
                    if (sid, n) == kill_at:
                        t_kill = time.monotonic()
                        rung.kill9()
                        hz = healthz_wall(injector.port)
                        records.append({
                            "event": "kill9", "session": sid, "turn": n,
                            "healthz_ms_after_kill": hz,
                            "healthz_within_1s": hz is not None
                            and hz <= 1000.0,
                        })
                    row = one_turn(injector.port, body)
                    row.update({"session": sid, "turn": n,
                                "gold": turn.get("gold"),
                                "kind": turn["kind"]})
                    if (sid, n) == kill_at:
                        row["during_kill"] = True
                        row["resolved_within_grace"] = (
                            row["err_kind"] is not None
                            or row["status"] is not None)
                        records.append({"event": "request_during_kill",
                                        "row": row})
                        # F1(d): retry later must not be a cache hit;
                        # schedule it after restart
                    records.append(row) if row.get("status") or \
                        row.get("err_kind") else None
                    if (sid, n) == kill_at:
                        print("rung killed; restarting in %ds"
                              % RUNG_RESTART_DELAY_S, flush=True)
                        time.sleep(RUNG_RESTART_DELAY_S)
                        restarted = rung.start()
                        records.append({"event": "restart",
                                        "ok": restarted})
                        # first request after restart goes to the NEXT turn
                    if restarted_flag(records) and not first_after_restart_done \
                            and (sid, n) == kill_at:
                        first_after_restart_done = True
            # F1(d): re-send the killed turn's request; must be a fresh
            # decide, not a cached completed response
            sid, n = kill_at
            body = next(s for s in sessions
                        if s["session_id"] == sid)["request_bodies"][str(n)]
            retry = one_turn(injector.port, body.encode("utf-8"))
            records.append({"event": "retry_of_failed",
                            "cache_hit": (retry.get("metrics") or {})
                            .get("cache_hit"),
                            "status": retry.get("status"),
                            "err_kind": retry.get("err_kind")})
        finally:
            serve.terminate()
            try:
                serve.wait(timeout=10)
            except subprocess.TimeoutExpired:
                serve.kill()
            rung.kill9()
    out["F1"] = {"records": records}


def restarted_flag(records: list) -> bool:
    return any(r.get("event") == "restart" and r.get("ok")
               for r in records if isinstance(r, dict))


def f1b(injector: argparse.Namespace, out: dict) -> None:
    """Kill the rung while a rung-bound (escalated) request is in flight.

    F1's kill over the class-A session turns is vacuous by measurement:
    all 50 turns resolve at the exact-rule layer (0 rung-engaged), so
    nothing was in flight against the rung when it died. This phase
    drives `lexical_semantic` L1 items — the class the escalate arm
    showed fires the rung — kills the rung mid-escalation, and checks
    the contract where it actually binds: the in-flight request resolves
    within client timeout + 5 s (typed error or answer, never a hang),
    the serve stays healthy, and after restart a FAILED request retries
    as a fresh decide (no cached decision served for a failed walk).
    """
    suite = json.loads(injector.suite.read_text())
    items = [i for i in suite["tiers"]["L1"]["items"]
             if i.get("class") == "lexical_semantic"]
    assert len(items) >= 2, "need two lexical_semantic L1 items"
    # warmup and kill attempt must be DIFFERENT items: the exact-decision
    # cache is content-keyed, so replaying the warmup's body on the kill
    # attempt hits the cache and never reaches the rung (measured: 0.6 ms
    # cache True on the first f1b run)
    warm_item, kill_item = items[0], items[1]

    def body_for(item: dict, tag: str) -> bytes:
        question = {"type": "choice", "id": item["id"],
                    "text": item["question"],
                    "candidates": item["candidates"]}
        return json.dumps({
            "state": {"text": item["context"], "facts": {}},
            "questions": [question],
            "policy": {"min_confidence": 0.8, "verify_below": 0.65,
                       "abstain_below": 0.5, "risk": "low"},
            "metadata": {"request_id": f"f1b-{tag}", "limits": {
                "max_input_bytes": 1_048_576, "max_questions": 32,
                "max_candidates": 256, "max_graph_nodes": 128,
                "max_execution_time": {"secs": 120, "nanos": 0},
                "max_retrieval_results": 64}}}).encode()

    records: list = []
    rung = Rung(injector.rung_binary, injector.rung_model,
                injector.rung_port, injector.rung_log)
    serve_cmd = [str(injector.binary), "serve",
                 "--bind", f"127.0.0.1:{injector.port}",
                 "--ladder", str(injector.ladder),
                 "--llama", f"http://127.0.0.1:{injector.rung_port}"]
    if injector.llama_model_id:
        serve_cmd += ["--llama-model-id", injector.llama_model_id]
    serve_log = injector.out_dir / "f1b-serve.log"
    with serve_log.open("w") as sfh:
        serve = subprocess.Popen(serve_cmd, stdout=sfh,
                                 stderr=subprocess.STDOUT)
        try:
            assert wait_healthz(injector.port, 120.0), "serve not healthy"
            assert rung.start(), "rung not healthy"

            # warm-up: prove this item actually escalates to the rung
            warm = one_turn(injector.port, body_for(warm_item, "warm"))
            engaged = "jev-style" in (json.dumps(warm.get("evidence") or {})
                                      + json.dumps(warm.get("decision_core")
                                                   or ""))
            records.append({"event": "warmup", "engaged_rung": engaged,
                            "wall_ms": warm["wall_ms"],
                            "outcome": warm.get("outcome")})

            # kill mid-flight: send, give the walk time to reach the
            # rung (the L1 prefill takes seconds), then kill9
            result: dict = {}

            kill_body = body_for(kill_item, "kill")

            def send() -> None:
                result["row"] = one_turn(injector.port, kill_body)

            worker = threading.Thread(target=send)
            worker.start()
            time.sleep(2.0)
            t_kill = time.monotonic()
            rung.kill9()
            hz = healthz_wall(injector.port)
            worker.join(CLIENT_TIMEOUT_S + RESOLVE_GRACE_S)
            resolve_s = time.monotonic() - t_kill
            row = result.get("row")
            records.append({
                "event": "kill9_mid_escalation",
                "healthz_ms_after_kill": hz,
                "healthz_within_1s": hz is not None and hz <= 1000.0,
                "resolved": row is not None,
                "resolve_s": round(resolve_s, 2),
                "within_timeout_plus_5": resolve_s <= CLIENT_TIMEOUT_S
                + RESOLVE_GRACE_S,
                "row": row,
            })

            time.sleep(RUNG_RESTART_DELAY_S)
            restarted = rung.start()
            records.append({"event": "restart", "ok": restarted})
            if restarted and row is not None and (
                    row.get("err_kind") or (row.get("status") or 0) != 200):
                # a 500 response is a normal HTTP reply — one_turn records
                # err_kind only for client-side exceptions — so the failed
                # request is any non-200, not just a raised error
                retry = one_turn(injector.port, body_for(kill_item, "retry"))
                records.append({
                    "event": "retry_of_failed",
                    "cache_hit": (retry.get("metrics") or {})
                    .get("cache_hit"),
                    "no_stale_cache": not (retry.get("metrics") or {})
                    .get("cache_hit"),
                    "outcome": retry.get("outcome"),
                    "status": retry.get("status"),
                    "err_kind": retry.get("err_kind"),
                })
        finally:
            serve.terminate()
            try:
                serve.wait(timeout=10)
            except subprocess.TimeoutExpired:
                serve.kill()
            rung.kill9()
    out["F1b"] = {"records": records}


def f3a(injector: argparse.Namespace, out: dict) -> None:
    """Request declaring 121 s is refused at the wire."""
    # Own serve lifecycle: the refusal happens in validation middleware
    # before any ladder/rung work, so no --llama is needed — posting at a
    # port nobody listens on (the previous shape) just refused the conn.
    serve_cmd = [str(injector.binary), "serve",
                 "--bind", f"127.0.0.1:{injector.f3a_port}",
                 "--ladder", str(injector.ladder)]
    serve_log = injector.out_dir / "f3a-serve.log"
    with serve_log.open("w") as sfh:
        serve = subprocess.Popen(serve_cmd, stdout=sfh,
                                 stderr=subprocess.STDOUT)
        try:
            assert wait_healthz(injector.f3a_port, 120.0), \
                "f3a serve not healthy"
            plan = json.loads(injector.plan.read_text())
            s0 = plan["sessions"][0]
            body = json.loads(s0["request_bodies"]["1"])
            body["metadata"]["limits"]["max_execution_time"] = {
                "secs": 121, "nanos": 0}
            raw = json.dumps(body).encode()
            conn = http.client.HTTPConnection("127.0.0.1",
                                              injector.f3a_port,
                                              timeout=30.0)
            conn.request("POST", "/v1/decide", raw,
                         {"Content-Type": "application/json"})
            resp = conn.getresponse()
            payload = resp.read()
            conn.close()
            out["F3a"] = {"status": resp.status,
                          "body": payload[:300].decode("utf-8", "replace")}
        finally:
            serve.terminate()
            try:
                serve.wait(timeout=10)
            except subprocess.TimeoutExpired:
                serve.kill()


def f3c(injector: argparse.Namespace, out: dict) -> None:
    """Rung behind a +60 s proxy: typed timeout inside deadline + 5 s."""
    proxy = DelayProxy(injector.proxy_port, injector.rung_port, 60.0)
    proxy.start()
    serve_cmd = [str(injector.binary), "serve",
                 "--bind", f"127.0.0.1:{injector.proxy_serve_port}",
                 "--ladder", str(injector.ladder),
                 "--llama", f"http://127.0.0.1:{injector.proxy_port}"]
    # the serve binary refuses to start with --llama and no model id
    # (clap: "required arguments were not provided") — f3c's first run
    # died exactly there, before a single request was sent
    if injector.llama_model_id:
        serve_cmd += ["--llama-model-id", injector.llama_model_id]
    serve_log = injector.out_dir / "f3c-serve.log"
    serve = None
    rung: Rung | None = None
    try:
        with serve_log.open("w") as sfh:
            serve = subprocess.Popen(serve_cmd, stdout=sfh,
                                     stderr=subprocess.STDOUT)
            assert wait_healthz(injector.proxy_serve_port, 120.0)
            rung = Rung(injector.rung_binary, injector.rung_model,
                        injector.rung_port, injector.rung_log)
            if not rung.alive():
                assert rung.start(), "rung not healthy"
            # one lexical_semantic item: its low-probability distribution
            # is the escalation trigger
            suite = json.loads(injector.suite.read_text())
            item = next(i for i in suite["tiers"]["L1"]["items"]
                        if i.get("class") == "lexical_semantic")
            question = {"type": "choice", "id": item["id"],
                        "text": item["question"],
                        "candidates": item["candidates"]}
            body = json.dumps({
                "state": {"text": item["context"], "facts": {}},
                "questions": [question],
                "policy": {"min_confidence": 0.8, "verify_below": 0.65,
                           "abstain_below": 0.5, "risk": "low"},
                "metadata": {"request_id": "f3c", "limits": {
                    "max_input_bytes": 1_048_576, "max_questions": 32,
                    "max_candidates": 256, "max_graph_nodes": 128,
                    "max_execution_time": {"secs": 120, "nanos": 0},
                    "max_retrieval_results": 64}}}).encode()
            start = time.monotonic()
            row = one_turn(injector.proxy_serve_port, body)
            wall = time.monotonic() - start
            hz_after = healthz_wall(injector.proxy_serve_port)
            out["F3c"] = {
                "wall_s": round(wall, 2),
                "row": row,
                "server_healthy_after": hz_after is not None,
                "within_deadline_plus_5": wall <= 120.0 + RESOLVE_GRACE_S,
            }
    finally:
        if serve is not None:
            serve.terminate()
            try:
                serve.wait(timeout=10)
            except subprocess.TimeoutExpired:
                serve.kill()
        if rung is not None:
            # f3c's first run leaked the rung here — the port stayed
            # bound after the phase ended
            rung.kill9()
        proxy.stop()


def f4(injector: argparse.Namespace, out: dict) -> None:
    """proofs-only ladder on class A: verify outcomes, never errors."""
    serve_cmd = [str(injector.binary), "serve",
                 "--bind", f"127.0.0.1:{injector.f4_port}",
                 "--ladder", str(injector.proofs_ladder)]
    serve_log = injector.out_dir / "f4-serve.log"
    with serve_log.open("w") as sfh:
        serve = subprocess.Popen(serve_cmd, stdout=sfh,
                                 stderr=subprocess.STDOUT)
        try:
            assert wait_healthz(injector.f4_port, 120.0)
            plan = json.loads(injector.plan.read_text())
            sessions = plan["sessions"][:2]
            rows = []
            for session in sessions:
                for turn in session["turns"][:10]:
                    row = one_turn(
                        injector.f4_port,
                        session["request_bodies"][str(turn["turn"])]
                        .encode("utf-8"))
                    rows.append(row)
            outcomes = {}
            verifier_nones = 0
            for r in rows:
                outcomes[r.get("outcome") or f"err:{r.get('err_kind')}"] = \
                    outcomes.get(
                        r.get("outcome") or f"err:{r.get('err_kind')}",
                        0) + 1
                if (r.get("evidence") or {}).get("threshold_verifier") \
                        == "none":
                    verifier_nones += 1
            out["F4"] = {"outcomes": outcomes,
                         "verifier_none_count": verifier_nones,
                         "requests": len(rows)}
        finally:
            serve.terminate()
            try:
                serve.wait(timeout=10)
            except subprocess.TimeoutExpired:
                serve.kill()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--binary", type=Path, required=True,
                    help="opencodifier-llamacpp (F1/F3c need the rung)")
    ap.add_argument("--rung-binary", type=Path, required=True)
    ap.add_argument("--rung-model", type=Path, required=True)
    ap.add_argument("--port", type=int, default=8092)
    ap.add_argument("--rung-port", type=int, default=8094)
    ap.add_argument("--proxy-port", type=int, default=8095)
    ap.add_argument("--proxy-serve-port", type=int, default=8096)
    ap.add_argument("--f4-port", type=int, default=8097)
    ap.add_argument("--f3a-port", type=int, default=8098)
    ap.add_argument("--ladder", type=Path, required=True,
                    help="fusion-v2.json (F1/F3c)")
    ap.add_argument("--proofs-ladder", type=Path, required=True,
                    help="proofs-only-v1.json (F4)")
    ap.add_argument("--llama-model-id", type=str, default=None)
    ap.add_argument("--plan", type=Path, required=True)
    ap.add_argument("--suite", type=Path, default=None,
                    help="longctx_tiers.json (F3c picks an escalating item)")
    ap.add_argument("--out-dir", type=Path, required=True)
    ap.add_argument("--rung-log", type=Path, default=None)
    ap.add_argument("--sessions", type=int, default=1)
    ap.add_argument("--kill-session", type=int, default=0)
    ap.add_argument("--kill-turn", type=int, default=25)
    ap.add_argument("--only", choices=("f1", "f1b", "f3a", "f3c", "f4"),
                    default=None)
    args = ap.parse_args()
    if not args.rung_log:
        args.rung_log = args.out_dir / "rung.log"
    args.out_dir.mkdir(parents=True, exist_ok=True)

    out: dict = {}
    phases = []
    if args.only in (None, "f3a", "f4"):
        if args.only == "f3a":
            phases = [("F3a", f3a)]
        elif args.only == "f4":
            phases = [("F4", f4)]
        else:
            phases = [("F3a", f3a), ("F4", f4), ("F1", f1)]
            if args.suite:
                phases.append(("F3c", f3c))
                phases.append(("F1b", f1b))
    else:
        phases = [(args.only, {"f1": f1, "f1b": f1b, "f3c": f3c}
                   .get(args.only, f1))]
    # each phase persists its own evidence the moment it finishes: one
    # phase crashing must not destroy the phases that already ran (the
    # first full-sequence run lost F3a/F4/F1 to a late f3c serve-flag
    # error because this wrote only at the end)
    for name, fn in phases:
        try:
            fn(args, out)
        except Exception as exc:  # noqa: BLE001 — record, keep going
            out[name] = {"phase_error": f"{type(exc).__name__}: {exc}"}
        (args.out_dir / "injections.json").write_text(
            json.dumps(out, indent=1, default=str))
    (args.out_dir / "injections.json").write_text(
        json.dumps(out, indent=1, default=str))
    print(json.dumps({k: (v if k in ("F3a", "F4")
                          else {kk: vv for kk, vv in v.items()
                                 if kk != "records"})
                      for k, v in out.items()}, indent=1, default=str))
    return 0


if __name__ == "__main__":
    sys.exit(main())
