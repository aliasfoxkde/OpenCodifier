#!/usr/bin/env python3
"""Re-capture the recipe expected-response files from a live engine run.

Every file under `recipes/expected/` is the response the committed engine
produced for one recipe's graph and request. They are shipped to users by
`opencodifier recipe install`, so they must match what the engine actually
does today — the composed model id moves when the solver or the lexical
classifier moves, and a stale capture would have a user comparing their run
against an expectation from a different engine.

This script drives the engine over HTTP (`POST /v1/graph/run`, which takes
the graph in the body, so one server covers every recipe) and writes each
response back in the capture format: minified, key-sorted, no trailing
newline.

Usage:
  python3 recipes/capture.py --binary target/debug/opencodifier   # write
  python3 recipes/capture.py --binary target/debug/opencodifier --check

`--check` writes nothing and exits non-zero when a capture differs from a
live run, which is what CI wants: the captures are verified, not regenerated
on the way past.
"""

from __future__ import annotations

import argparse
import json
import os
import signal
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
REQUESTS = HERE / "requests"
EXPECTED = HERE / "expected"

# The recipe name -> request document. A recipe without a request cannot be
# captured, so it is listed as skipped rather than silently passing.
#
# `minimal-choice` predates the fleet and its request is the generic choice
# request. `strict-verify` is the same graph family with a stricter 0.95
# gate, and it is exercised by the *same* request — that pairing is the
# point of the recipe: one request, two gates, two outcomes.
REQUEST_FOR: dict[str, str] = {
    "model-routing": "model-routing.json",
    "task-classification": "task-classification.json",
    "tool-selection": "tool-selection.json",
    "tool-gating": "tool-gating.json",
    "context-pruning": "context-pruning.json",
    "cache-eligibility": "cache-eligibility.json",
    "skill-selection": "skill-selection.json",
    "memory-selection": "memory-selection.json",
    "escalation": "escalation.json",
    "verification": "verification.json",
    "document-relevance": "document-relevance.json",
    "code-review-risk": "code-review-risk.json",
    "boolean-score": "boolean-score.json",
    "minimal-choice": "choice.json",
    "strict-verify": "choice.json",
}

# The port the capture server binds. Loopback only, and an unusual port so
# it cannot collide with a developer's own `opencodifier serve`.
PORT = 8178


def _read(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def _post(url: str, payload: dict) -> dict:
    body = json.dumps(payload).encode("utf-8")
    request = urllib.request.Request(  # noqa: S310 — loopback, fixed scheme
        url,
        data=body,
        headers={"content-type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=60) as response:  # noqa: S310
        return json.loads(response.read().decode("utf-8"))


def _wait_for_port(port: int, process: subprocess.Popen[bytes]) -> None:
    """Block until the server answers, or fail loudly."""
    url = f"http://127.0.0.1:{port}/v1/healthz"
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise SystemExit("the capture server exited before it was ready")
        try:
            with urllib.request.urlopen(url, timeout=2):  # noqa: S310
                return
        except (urllib.error.URLError, OSError):
            time.sleep(0.2)
    raise SystemExit("the capture server did not become ready in 30s")


def capture(binary: Path, port: int) -> list[tuple[str, bool, str]]:
    """Capture every recipe. Returns `(name, matched, detail)` per recipe."""
    server = subprocess.Popen(  # noqa: S603 — a fixed local binary
        [str(binary), "serve", "--bind", f"127.0.0.1:{port}"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )
    results: list[tuple[str, bool, str]] = []
    try:
        _wait_for_port(port, server)
        for name, request_name in sorted(REQUEST_FOR.items()):
            graph_path = HERE / f"{name}.json"
            request_path = REQUESTS / request_name
            if not graph_path.exists() or not request_path.exists():
                results.append((name, False, "missing graph or request document"))
                continue
            payload = {"graph": _read(graph_path), "request": _read(request_path)}
            response = _post(f"http://127.0.0.1:{port}/v1/graph/run", payload)
            encoded = response["response"]
            text = json.dumps(encoded, sort_keys=True, separators=(",", ":"))
            target = EXPECTED / f"{name}-response.json"
            previous = target.read_text(encoding="utf-8") if target.exists() else ""
            if previous == text:
                results.append((name, True, "unchanged"))
            else:
                results.append((name, False, "differs from the capture on disk"))
                if not args_check_only:
                    target.write_text(text, encoding="utf-8")
    finally:
        os.killpg(os.getpgid(server.pid), signal.SIGTERM)
        server.wait(timeout=30)
    return results


args_check_only = False


def main() -> int:
    global args_check_only
    ap = argparse.ArgumentParser()
    ap.add_argument("--binary", type=Path, default=Path("target/debug/opencodifier"))
    ap.add_argument("--port", type=int, default=PORT)
    ap.add_argument("--check", action="store_true", help="verify without writing")
    args = ap.parse_args()
    args_check_only = args.check

    binary = args.binary.resolve()
    if not binary.exists():
        sys.stderr.write(f"no binary at {binary}; build it first\n")
        return 2

    results = capture(binary, args.port)
    stale = [name for name, matched, _ in results if not matched]
    for name, matched, detail in results:
        sys.stderr.write(f"{'ok  ' if matched else 'STALE'} {name}: {detail}\n")
    if stale:
        sys.stderr.write(f"{len(stale)} capture(s) need re-recording\n")
        return 1
    sys.stderr.write("every capture matches a live run\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
