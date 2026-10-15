#!/usr/bin/env python3
"""Vision probe (task #42): LFM2.5-VL-450M through the fork's llama-server.

Two probes, both decision-shaped:
  A. text-only   — plain chat completion, no image; can the VL model's text
                   backbone answer a choice decision at all?
  B. rendered    — a PIL-rendered context table sent as a base64 image_url;
                   does the mmproj path load and can it read the table?

A mmproj load failure is a VALID probe result (known mmproj breakage in
llama.cpp derivatives, e.g. KoboldCpp issue #1921) — the probe records the
failure mode verbatim rather than treating it as harness error.
"""

from __future__ import annotations

import argparse
import base64
import io
import json
import subprocess
import time
import urllib.error
import urllib.request
from pathlib import Path

from PIL import Image, ImageDraw


def http_json(port: int, path: str, payload: dict, timeout: float) -> tuple[int, dict | str]:
    body = json.dumps(payload).encode()
    req = urllib.request.Request(
        f"http://127.0.0.1:{port}{path}",
        data=body,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    t0 = time.monotonic()
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return time.monotonic() - t0, json.loads(resp.read())
    except urllib.error.HTTPError as e:
        return time.monotonic() - t0, e.read().decode(errors="replace")
    except Exception as e:  # noqa: BLE001 — probe records everything
        return time.monotonic() - t0, f"{type(e).__name__}: {e}"


def render_context_png(path: Path) -> None:
    """A small ops-context table the model must read to decide."""
    img = Image.new("RGB", (640, 220), "white")
    d = ImageDraw.Draw(img)
    rows = [
        ("FIELD", "VALUE"),
        ("service", "cache"),
        ("status", "degraded"),
        ("dependency queue", "up"),
        ("restarts (1h)", "0"),
    ]
    y = 20
    for i, (k, v) in enumerate(rows):
        if i == 0:
            d.rectangle([20, y - 6, 620, y + 22], fill="#dddddd")
        d.text((30, y), k, fill="black")
        d.text((320, y), v, fill="black")
        y += 34
    img.save(path, format="PNG")


DECISION_ASK = (
    "Candidates: restart_cache, restart_queue, escalate. "
    "Based on {src}, which single candidate should the decision engine pick? "
    "Reply with exactly one candidate label and nothing else."
)

TEXT_CONTEXT = (
    "Context: service 'cache' reports status=degraded; "
    "its dependency 'queue' is up; restarts in the last hour: 0."
)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--llama-dir", type=Path, required=True)
    ap.add_argument("--build-dir", default="build-pd")
    ap.add_argument("--model-dir", type=Path, required=True)
    ap.add_argument("--model", default="LFM2.5-VL-450M-Q4_K_M.gguf")
    ap.add_argument("--mmproj", default="mmproj-LFM2.5-VL-450m-Q8_0.gguf")
    ap.add_argument("--port", type=int, default=8396)
    ap.add_argument("--threads", type=int, default=6)
    ap.add_argument("--ctx", type=int, default=4096)
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()

    server = args.llama_dir / args.build_dir / "bin" / "llama-server"
    model = args.model_dir / args.model
    mmproj = args.model_dir / args.mmproj
    png = args.out.parent / (args.out.stem + "_context.png")
    render_context_png(png)

    result: dict = {
        "probe": "vision_lfm25vl_450m",
        "model": {"file": model.name, "bytes": model.stat().st_size},
        "mmproj": {"file": mmproj.name, "bytes": mmproj.stat().st_size},
        "server": str(server),
        "build_dir": args.build_dir,
        "load_ok": False,
        "text_ok": None,
        "vision_ok": None,
    }

    cmd = [
        str(server),
        "-m", str(model),
        "--mmproj", str(mmproj),
        "--port", str(args.port),
        "-c", str(args.ctx),
        "-t", str(args.threads),
        "-ngl", "0",
        "--jinja",
    ]
    print("spawn:", " ".join(cmd), flush=True)
    with open(args.out.with_suffix(".server.log"), "wb") as log:
        proc = subprocess.Popen(cmd, stdout=log, stderr=subprocess.STDOUT)
    try:
        # health poll — mtmd/mmproj load happens during startup
        deadline = time.monotonic() + 180
        healthy = False
        while time.monotonic() < deadline:
            if proc.poll() is not None:
                result["error"] = f"server exited rc={proc.returncode} during startup"
                break
            try:
                with urllib.request.urlopen(
                    f"http://127.0.0.1:{args.port}/health", timeout=5
                ) as r:
                    if r.status == 200:
                        healthy = True
                        break
            except Exception:
                time.sleep(2)
        if not healthy:
            result.setdefault("error", "server never became healthy within 180s")
            return finish(result, proc, args)

        result["load_ok"] = True
        result["load_seconds"] = round(time.monotonic() - t_start, 2)

        # Probe A — text only
        lat_a, ans_a = http_json(
            args.port,
            "/v1/chat/completions",
            {
                "messages": [
                    {"role": "system", "content": "You are a decision engine. Answer with exactly one candidate label."},
                    {"role": "user", "content": f"{TEXT_CONTEXT}\n{DECISION_ASK.format(src='the context')}"},
                ],
                "temperature": 0,
                "max_tokens": 32,
            },
            300,
        )
        result["probe_a_text"] = {
            "latency_s": round(lat_a, 2),
            "ok": isinstance(ans_a, dict),
            "answer": _content(ans_a),
            "raw" if not isinstance(ans_a, dict) else "usage": ans_a if not isinstance(ans_a, dict) else ans_a.get("usage"),
        }
        result["text_ok"] = isinstance(ans_a, dict) and "cache" in (result["probe_a_text"]["answer"] or "").lower()

        # Probe B — rendered PNG as image_url
        b64 = base64.b64encode(png.read_bytes()).decode()
        lat_b, ans_b = http_json(
            args.port,
            "/v1/chat/completions",
            {
                "messages": [
                    {"role": "system", "content": "You are a decision engine. Answer with exactly one candidate label."},
                    {
                        "role": "user",
                        "content": [
                            {"type": "image_url", "image_url": {"url": f"data:image/png;base64,{b64}"}},
                            {"type": "text", "text": DECISION_ASK.format(src="the table in the image")},
                        ],
                    },
                ],
                "temperature": 0,
                "max_tokens": 32,
            },
            300,
        )
        result["probe_b_vision"] = {
            "latency_s": round(lat_b, 2),
            "ok": isinstance(ans_b, dict),
            "answer": _content(ans_b),
            "raw" if not isinstance(ans_b, dict) else "usage": ans_b if not isinstance(ans_b, dict) else ans_b.get("usage"),
        }
        result["vision_ok"] = isinstance(ans_b, dict) and "cache" in (result["probe_b_vision"]["answer"] or "").lower()
        return finish(result, proc, args)
    finally:
        pass


t_start = time.monotonic()


def _content(ans: dict | str) -> str | None:
    if not isinstance(ans, dict):
        return None
    try:
        return ans["choices"][0]["message"]["content"]
    except (KeyError, IndexError, TypeError):
        return None


def finish(result: dict, proc: subprocess.Popen, args) -> int:
    try:
        proc.wait(timeout=10)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()
    server_log = ""
    slog = args.out.with_suffix(".server.log")
    if slog.exists():
        server_log = slog.read_text(errors="replace")[-4000:]
    result["server_log_tail"] = server_log
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, indent=2) + "\n")
    verdict = "PASS" if (result["load_ok"] and result["text_ok"] and result["vision_ok"]) else (
        "PARTIAL" if result["load_ok"] else "LOAD-FAIL"
    )
    print(f"probe verdict: {verdict}  load_ok={result['load_ok']} "
          f"text_ok={result['text_ok']} vision_ok={result['vision_ok']}", flush=True)
    return 0 if result["load_ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
