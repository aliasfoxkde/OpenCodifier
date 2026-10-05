#!/usr/bin/env python3
"""End-to-end validation of the OpenCodifier release binary (PLAN 18d).

Builds nothing and fakes nothing: it spawns the real binary twice —
`serve` on loopback and `mcp serve` over stdio — and exercises every
shipped interface the way a client would: native decisions of all three
types, the abstain path, cache identity, malformed and oversized input,
graph validation (clean and cyclic), and the MCP handshake plus tool
list. Exit 0 only when every check passes; each check prints its own
PASS/FAIL line so a failure is actionable without re-running.

Load-invariance: the engine is deterministic and CPU-cheap, so this
suite is valid on a busy host; only its wall clock is affected.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

CHECKS: list[tuple[str, bool, str]] = []


def check(name: str, ok: bool, detail: str = "") -> None:
    CHECKS.append((name, bool(ok), detail))
    print(f"{'PASS' if ok else 'FAIL'}  {name}{(' — ' + detail) if detail else ''}",
          flush=True)


def post(base: str, path: str, payload, timeout: float = 60.0):
    """POST JSON; returns (status, parsed-body-or-text). Never raises on HTTP 4xx/5xx."""
    data = payload if isinstance(payload, bytes) else json.dumps(payload).encode()
    req = urllib.request.Request(
        base + path, data=data, headers={"Content-Type": "application/json"}
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            body = r.read().decode()
            try:
                return r.status, json.loads(body)
            except json.JSONDecodeError:
                return r.status, body
    except urllib.error.HTTPError as e:
        # The server may close the socket early on oversized bodies.
        try:
            body = e.read().decode()
        except Exception:  # noqa: BLE001 — a truncated error body is still an answer
            body = ""
        try:
            return e.code, json.loads(body)
        except json.JSONDecodeError:
            return e.code, body


def get(base: str, path: str, timeout: float = 10.0):
    """GET JSON; returns (status, parsed-body-or-text). Never raises on HTTP 4xx/5xx."""
    try:
        with urllib.request.urlopen(base + path, timeout=timeout) as r:
            body = r.read().decode()
            try:
                return r.status, json.loads(body)
            except json.JSONDecodeError:
                return r.status, body
    except urllib.error.HTTPError as e:
        return e.code, {}  # metadata endpoints answer errors with a code


def wait_health(base: str, deadline_s: float = 30.0) -> bool:
    end = time.monotonic() + deadline_s
    while time.monotonic() < end:
        try:
            with urllib.request.urlopen(base + "/v1/healthz", timeout=2) as r:
                return r.status == 200
        except Exception:  # noqa: BLE001 — startup race is the expected case
            time.sleep(0.2)
    return False


def decide_payload(question: dict, policy: dict | None = None) -> dict:
    # `policy` and `metadata` are optional on the wire since Phase 19c
    # (omitted fields take the engine defaults); sent explicitly here so
    # the suite exercises non-default policies and request ids.
    return {
        "state": {"text": "The service is down and customers are affected.",
                  "facts": {}},
        "questions": [question],
        "policy": policy or {"min_confidence": 0.8, "verify_below": 0.65,
                             "abstain_below": 0.5, "risk": "low"},
        "metadata": {"request_id": "e2e",
                     "limits": {"max_input_bytes": 1048576,
                                "max_questions": 32,
                                "max_candidates": 256,
                                "max_graph_nodes": 128,
                                "max_execution_time": {"secs": 10, "nanos": 0},
                                "max_retrieval_results": 64}},
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--binary", default="target/release/opencodifier")
    ap.add_argument("--port", type=int, default=8188)
    ap.add_argument("--skip-mcp", action="store_true")
    args = ap.parse_args()

    base = f"http://127.0.0.1:{args.port}"
    # Fail fast if the port is already held: a stale server would answer
    # healthz for us and poison every cache/determinism assertion with its
    # warm state (the exact silent-bind-failure trap the engine arm hit).
    with socket.socket() as probe:
        if probe.connect_ex(("127.0.0.1", args.port)) == 0:
            print(f"FAIL  preflight: port {args.port} already in use — "
                  "pass --port with a free port", flush=True)
            return 1
    server = subprocess.Popen(
        [args.binary, "serve", "--bind", f"127.0.0.1:{args.port}"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    try:
        if not wait_health(base):
            check("serve: healthz reachable", False, "server did not become healthy")
            return _exit()
        check("serve: healthz reachable", True)

        run_http_checks(base)
        run_surface_checks(base)
        if not args.skip_mcp:
            run_mcp_checks(args.binary)
        run_recipe_checks(args.binary, args.port + 1)
    finally:
        server.terminate()
        try:
            server.wait(timeout=10)
        except subprocess.TimeoutExpired:
            server.kill()
    return _exit()


def run_http_checks(base: str) -> None:
    # --- healthz identity -------------------------------------------------
    st, health = None, None
    req = urllib.request.Request(base + "/v1/healthz")
    with urllib.request.urlopen(req, timeout=10) as r:
        st, health = r.status, json.loads(r.read())
    ident = health.get("identity", {}) if isinstance(health, dict) else {}
    check("healthz: 200 with real identity",
          st == 200 and health.get("status") == "ok"
          and all(k in ident for k in
                  ("graph_version", "model_id", "calibration_version", "engine_semver")),
          json.dumps(ident)[:120])

    # --- choice -----------------------------------------------------------
    q = {"type": "choice", "id": "decision",
         "text": "Which component is responsible for the outage?",
         "candidates": [{"id": "database", "description": "The database tier"},
                        {"id": "network", "description": "The network path"}]}
    st, resp = post(base, "/v1/decide", decide_payload(q))
    choice_answer = resp.get("answers", [{}])[0] if isinstance(resp, dict) else {}
    check("decide choice: 200, typed answer in candidate set",
          st == 200 and resp.get("outcome") in ("accept", "verify")
          and choice_answer.get("choice") in ("database", "network"),
          f"outcome={resp.get('outcome')} choice={choice_answer.get('choice')}")

    # --- determinism / cache identity -------------------------------------
    # The decision content must be identical; the only permitted difference
    # is the cache marker (miss on 1st, hit on 2nd) and the trace's cache
    # node, which records exactly that. Asserting the hit also proves the
    # exact-decision cache actually engaged with the same CacheKeyBuilder key.
    st2, resp2 = post(base, "/v1/decide", decide_payload(q))
    same = (st2 == 200
            and resp2.get("answers") == resp.get("answers")
            and resp2.get("outcome") == resp.get("outcome")
            and resp2.get("confidence") == resp.get("confidence")
            and resp2.get("metrics", {}).get("cache_hit") is True
            and resp.get("metrics", {}).get("cache_hit") is False)
    check("decide repeat: identical decision, cache engaged on 2nd", same,
          f"hit1={resp.get('metrics', {}).get('cache_hit')} "
          f"hit2={resp2.get('metrics', {}).get('cache_hit')}")

    # --- boolean ----------------------------------------------------------
    qb = {"type": "boolean", "id": "decision",
          "text": "Is the service currently degraded?"}
    st, resp = post(base, "/v1/decide", decide_payload(qb))
    ans = resp.get("answers", [{}])[0] if isinstance(resp, dict) else {}
    ok = (st == 200 and resp.get("outcome") in ("accept", "verify", "abstain")
          and isinstance(ans.get("value"), bool)
          and isinstance(ans.get("probability"), (int, float)))
    check("decide boolean: 200, typed value with probability", ok,
          f"outcome={resp.get('outcome')} value={ans.get('value')}")

    # --- score ------------------------------------------------------------
    qs = {"type": "score", "id": "decision",
          "text": "How severe is the incident?",
          "levels": [{"label": "low"}, {"label": "medium"}, {"label": "high"}]}
    st, resp = post(base, "/v1/decide", decide_payload(qs))
    ans = resp.get("answers", [{}])[0] if isinstance(resp, dict) else {}
    labels = {lv["label"] for lv in qs["levels"]}
    dist = ans.get("distribution") or {}
    keys = {e.get("key") for e in dist.get("entries", [])} if isinstance(dist, dict) else set()
    ok = (st == 200 and resp.get("outcome") in ("accept", "verify", "abstain")
          and (ans.get("value") in labels or resp.get("outcome") == "abstain"))
    covered = resp.get("outcome") == "abstain" or keys == labels
    check("decide score: 200, level answer, distribution covers levels",
          ok and covered, f"outcome={resp.get('outcome')} keys={sorted(keys)}")

    # --- abstain is a 200 --------------------------------------------------
    policy = {"min_confidence": 1.0, "verify_below": 1.0, "abstain_below": 1.0,
              "risk": "low"}
    st, resp = post(base, "/v1/decide", decide_payload(q, policy))
    check("abstain path: HTTP 200 with abstain outcome",
          st == 200 and resp.get("outcome") == "abstain",
          f"status={st} outcome={resp.get('outcome')}")

    # --- malformed native state --------------------------------------------
    # (`facts` is optional on the wire since 19c; a state missing its text
    # is still refused.)
    bad = decide_payload(q)
    del bad["state"]["text"]
    st, resp = post(base, "/v1/decide", bad)
    code = resp.get("error", {}).get("code", "") if isinstance(resp, dict) else ""
    check("malformed state (missing text): 400 schema.*",
          st == 400 and code.startswith("schema."), f"status={st} code={code}")

    # --- oversized body at the socket boundary ------------------------------
    big = b'{"state":{"text":"' + b"a" * (1_048_576 + 1024) + b'","facts":{}}}'
    st, _ = post(base, "/v1/decide", big)
    check("oversized body: rejected 4xx at transport", 400 <= st < 500, f"status={st}")

    # --- graph validation ---------------------------------------------------
    good = {"version": 1,
            "nodes": [{"id": "entry", "kind": "normalize"},
                      {"id": "lex", "kind": "lexical", "depends_on": ["entry"]},
                      {"id": "out", "kind": "output", "depends_on": ["lex"]}]}
    st, resp = post(base, "/v1/graph/validate", good)
    check("graph validate: clean DAG accepted",
          st == 200 and resp.get("valid") is True and resp.get("nodes") == 3,
          json.dumps(resp)[:100])

    cyc = {"version": 1,
           "nodes": [{"id": "a", "kind": "normalize", "depends_on": ["b"]},
                     {"id": "b", "kind": "lexical", "depends_on": ["a"]}]}
    st, resp = post(base, "/v1/graph/validate", cyc)
    detail = json.dumps(resp)
    check("graph validate: cycle rejected 4xx with graph.*",
          400 <= st < 500 and "graph." in detail, f"status={st} {detail[:100]}")

    # --- graph run: decide through a client-supplied graph (D19) ------------
    pipeline = {"version": 1, "nodes": [
        {"id": "normalize", "kind": "normalize"},
        {"id": "rule", "kind": "rule", "depends_on": ["normalize"]},
        {"id": "cache", "kind": "cache", "depends_on": ["normalize"]},
        {"id": "boolean", "kind": "boolean", "depends_on": ["rule"]},
        {"id": "score", "kind": "score", "depends_on": ["rule"]},
        {"id": "filter", "kind": "filter", "depends_on": ["rule", "cache"]},
        {"id": "lexical", "kind": "lexical", "depends_on": ["filter"]},
        {"id": "choice", "kind": "choice", "depends_on": ["lexical"]},
        {"id": "threshold", "kind": "threshold", "depends_on": ["choice", "boolean", "score"],
         "threshold": 0.8},
        {"id": "output", "kind": "output", "depends_on": ["threshold"]},
    ]}
    body = {"graph": pipeline, "request": decide_payload(q)}
    st, resp = post(base, "/v1/graph/run", body)
    ans = resp.get("response", {}).get("answers", [{}])[0] if isinstance(resp, dict) else {}
    ident = resp.get("identity", {}) if isinstance(resp, dict) else {}
    check("graph run: decides through the client graph, content-addressed identity",
          st == 200 and ans.get("type") == "choice"
          and isinstance(ident.get("graph_version"), int) and ident.get("graph_version") != 1
          and isinstance(ident.get("model_id"), str),
          f"status={st} answer={ans.get('type')} identity={json.dumps(ident)[:80]}")

    body["graph"] = cyc
    st, resp = post(base, "/v1/graph/run", body)
    code = resp.get("error", {}).get("code", "") if isinstance(resp, dict) else ""
    check("graph run: cyclic graph is 400 graph.cycle",
          st == 400 and code == "graph.cycle", f"status={st} code={code}")


def run_mcp_checks(binary: str) -> None:
    """MCP stdio handshake + tool list against the real binary."""
    try:
        proc = subprocess.Popen(
            [binary, "mcp", "serve"], stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True,
        )
    except OSError as e:
        check("mcp: spawn", False, str(e))
        return
    try:
        init = {"jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                           "clientInfo": {"name": "e2e", "version": "0"}}}
        proc.stdin.write(json.dumps(init) + "\n")
        proc.stdin.flush()
        line = proc.stdout.readline()
        ok, detail = False, "no response"
        if line:
            res = json.loads(line).get("result", {})
            name = (res.get("serverInfo") or {}).get("name")
            ok = name == "opencodifier"
            detail = f"serverInfo.name={name}"
        check("mcp: initialize handshake, serverInfo opencodifier", ok, detail)

        proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}) + "\n")
        proc.stdin.flush()
        # Skip the notification echo if the server emits one.
        names = []
        for _ in range(3):
            line = proc.stdout.readline()
            if not line:
                break
            msg = json.loads(line)
            if msg.get("id") == 2:
                names = {t.get("name") for t in msg.get("result", {}).get("tools", [])}
                break
        expected = {"codify_decide", "codify_batch", "codify_graph",
                    "codify_validate", "codify_verify", "codify_explain"}
        check("mcp: tools/list exposes the shipped tools",
              expected <= names, f"got={sorted(names)}")
    except Exception as e:  # noqa: BLE001 — any stdio break is a FAIL line
        check("mcp: handshake", False, repr(e))
    finally:
        proc.kill()
        proc.wait(timeout=10)


def _exit() -> int:
    failed = [c for c in CHECKS if not c[1]]
    print(f"\n{len(CHECKS) - len(failed)}/{len(CHECKS)} e2e checks passed",
          flush=True)
    return 1 if failed else 0


FLEET = ["model-routing", "task-classification", "tool-selection", "tool-gating",
         "context-pruning", "cache-eligibility", "skill-selection", "memory-selection",
         "escalation", "verification", "document-relevance", "code-review-risk"]


def run_recipe_checks(binary: str, port: int) -> None:
    """§34 recipe fleet: list, install, overwrite guard, decide-through-install."""
    repo = pathlib.Path(__file__).resolve().parent.parent
    proc = subprocess.run([binary, "recipe", "list"], capture_output=True, text=True,
                          timeout=60)
    stdout = proc.stdout
    check("recipe list: all twelve fleet areas named",
          proc.returncode == 0 and all(name in stdout for name in FLEET),
          f"rc={proc.returncode} lines={len(stdout.splitlines())}")

    with tempfile.TemporaryDirectory(prefix="oc-e2e-recipes-") as tmp:
        dest = pathlib.Path(tmp) / "fleet"
        proc = subprocess.run([binary, "recipe", "install", "model-routing",
                               "--dest", str(dest)], capture_output=True, text=True,
                              timeout=60)
        target = dest / "model-routing"
        files_ok = all((target / f).is_file()
                       for f in ("graph.json", "request.json", "expected-response.json"))
        check("recipe install: three fleet files written", proc.returncode == 0 and files_ok,
              f"rc={proc.returncode} {proc.stderr[:80]}")

        committed = (repo / "recipes" / "model-routing.json").read_bytes()
        installed = (target / "graph.json").read_bytes() if files_ok else b""
        check("recipe install: bytes identical to the committed fleet graph",
              installed == committed, f"{len(installed)} vs {len(committed)} bytes")

        proc = subprocess.run([binary, "graph", "validate", str(target / "graph.json")],
                              capture_output=True, text=True, timeout=60)
        check("recipe install: installed graph validates",
              proc.returncode == 0 and "ok: graph" in proc.stdout, proc.stdout[:80])

        proc = subprocess.run([binary, "recipe", "install", "model-routing",
                               "--dest", str(dest)], capture_output=True, text=True,
                              timeout=60)
        check("recipe install: overwrite refused without --force",
              proc.returncode != 0 and "cli.recipe_exists" in proc.stderr,
              f"rc={proc.returncode} {proc.stderr[:80]}")
        proc = subprocess.run([binary, "recipe", "install", "model-routing", "--force",
                               "--dest", str(dest)], capture_output=True, text=True,
                              timeout=60)
        check("recipe install: --force overwrites",
              proc.returncode == 0, proc.stderr[:80])

    proc = subprocess.run([binary, "recipe", "install", "does-not-exist",
                           "--dest", tempfile.mkdtemp(prefix="oc-e2e-recipe-bad-")],
                          capture_output=True, text=True, timeout=60)
    check("recipe install: unknown name is an input error",
          proc.returncode != 0 and "cli.unknown_recipe" in proc.stderr,
          f"rc={proc.returncode} {proc.stderr[:80]}")

    # Decide through an installed recipe on a fresh server: the install is
    # runnable, and the outcome matches the captured expected response.
    base = f"http://127.0.0.1:{port}"
    with socket.socket() as probe:
        if probe.connect_ex(("127.0.0.1", port)) == 0:
            check("recipe serve: port free for fresh server", False,
                  f"port {port} in use")
            return
    with tempfile.TemporaryDirectory(prefix="oc-e2e-recipe-serve-") as tmp:
        dest = pathlib.Path(tmp) / "recipes"
        subprocess.run([binary, "recipe", "install", "model-routing", "--dest", str(dest)],
                       capture_output=True, timeout=60)
        graph = dest / "model-routing" / "graph.json"
        request = dest / "model-routing" / "request.json"
        expected = json.loads((dest / "model-routing" / "expected-response.json").read_text())
        server = subprocess.Popen(
            [binary, "serve", "--bind", f"127.0.0.1:{port}", "--graph", str(graph)],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            if not wait_health(base):
                check("recipe serve: installed graph serves", False, "never healthy")
                return
            check("recipe serve: installed graph serves", True)
            st, response = post(base, "/v1/decide", json.loads(request.read_text()))
            check("recipe serve: decide matches the captured outcome",
                  st == 200 and response.get("outcome") == expected.get("outcome")
                  and response.get("answers") == expected.get("answers"),
                  f"st={st} outcome={response.get('outcome')} "
                  f"expected={expected.get('outcome')}")
        finally:
            server.terminate()
            try:
                server.wait(timeout=10)
            except subprocess.TimeoutExpired:
                server.kill()


def run_surface_checks(base: str) -> None:
    """PLAN 18e: the §36 metadata + batch + validate surface."""
    # --- /v1/validate ------------------------------------------------------
    q = {"type": "boolean", "id": "needs_tools",
         "text": "Does resolving this require touching external tools?"}
    st, resp = post(base, "/v1/validate", decide_payload(q))
    check("validate: 200 valid:true with question count",
          st == 200 and isinstance(resp, dict) and resp.get("valid") is True
          and resp.get("questions") == 1,
          json.dumps(resp)[:120])
    bad = decide_payload(q)
    del bad["state"]
    st, resp = post(base, "/v1/validate", bad)
    code = (resp.get("error") or {}).get("code", "?") if isinstance(resp, dict) else "?"
    check("validate: a payload missing its state is a 400 schema.*",
          st == 400 and str(code).startswith("schema."), f"status={st} code={code}")

    # --- /v1/batch ---------------------------------------------------------
    st, resp = post(base, "/v1/batch", {
        "requests": [decide_payload(q), {"state": {"text": 7}},
                     decide_payload(q)]})
    results = resp.get("results", []) if isinstance(resp, dict) else []
    item_error = results[1].get("error", {}) if len(results) > 1 else {}
    check("batch: independent items, per-item error envelope",
          st == 200 and resp.get("count") == 3 and len(results) == 3
          and results[0].get("response", {}).get("answers")
          and str(item_error.get("code", "")).startswith("schema."),
          f"count={resp.get('count') if isinstance(resp, dict) else resp!r} "
          f"err={item_error.get('code') if isinstance(item_error, dict) else item_error!r}")
    st, resp = post(base, "/v1/batch",
        {"requests": [decide_payload(q)] * 17})
    code = (resp.get("error") or {}).get("code", "?") if isinstance(resp, dict) else "?"
    check("batch: 17 items refused up front",
          st == 400 and code == "schema.limit_exceeded",
          f"status={st} code={code} body={str(resp)[:80]}")

    # --- /v1/models + /v1/capabilities -------------------------------------
    st, models = get(base, "/v1/models")
    lanes = models.get("models", []) if isinstance(models, dict) else []
    check("models: exactly the active decision lane",
          st == 200 and len(lanes) == 1 and lanes[0].get("role") == "decision"
          and lanes[0].get("active") is True and lanes[0].get("id"),
          json.dumps(lanes)[:120])
    st, caps = get(base, "/v1/capabilities")
    kinds = caps.get("decision_kinds", []) if isinstance(caps, dict) else []
    endpoints = caps.get("endpoints", []) if isinstance(caps, dict) else []
    check("capabilities: kinds, batch limit, cache probe, endpoints",
          st == 200 and sorted(kinds) == ["boolean", "choice", "score"]
          and caps.get("max_batch") == 16
          and isinstance(caps.get("cache", {}).get("enabled"), bool)
          and "/v1/decide" in endpoints and "/v1/batch" in endpoints,
          f"kinds={kinds} max_batch={caps.get('max_batch') if isinstance(caps, dict) else '?'}")


if __name__ == "__main__":
    sys.exit(main())
