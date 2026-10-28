#!/usr/bin/env python3
"""B2 contamination-audit probe: autotrust/JEV-27B on JevBench public 231.

Runs entirely inside a Modal H100 function: vLLM serves JEV-27B through
autotrust's own serve_decide.py; the local runner (run_jevbench.py --arm
jev27) replays the 231 public items against it in-process over
localhost. Weights persist in a Modal Volume so only the first run pays
the ~54 GB download.

Usage:
  modal run run_b2_modal.py            # full: server + replay
"""
import logging
import os
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

import modal

LOG = logging.getLogger("b2")
logging.basicConfig(level=logging.INFO,
                    format="%(asctime)s %(levelname)s %(message)s")

APP_NAME = "b2-jev27-contamination-audit"
MODEL_DIR = "/models/JEV-27B"
REPO_ROOT = Path("/oc")
WORK_DIR = Path(__file__).resolve().parent

app = modal.App(APP_NAME)
volume = modal.Volume.from_name("jev27-weights", create_if_missing=True)

JEVBENCH_REF = Path("/nas/Temp/work/oc-model-eval/jevbench-ref")

image = (
    modal.Image.debian_slim(python_version="3.11")
    .pip_install("vllm==0.11.0", "huggingface_hub[hf_transfer]", "requests")
    .add_local_dir(str(WORK_DIR.parent.parent.parent),
                   str(REPO_ROOT / "OpenCodifier"))
    .add_local_dir(str(JEVBENCH_REF), str(REPO_ROOT / "jevbench-ref"))
    .pip_install(str(REPO_ROOT / "jevbench-ref"))
)


def wait_ready(url: str, deadline_s: float = 1800.0) -> None:
    t0 = time.time()
    while time.time() - t0 < deadline_s:
        try:
            with urllib.request.urlopen(f"{url}/v1/decide/info", timeout=5) as r:
                LOG.info("ready: %s", r.read()[:200])
                return
        except Exception:
            time.sleep(10)
    raise SystemExit("vLLM /v1/decide did not become ready")


@app.function(
    gpu="H100",
    timeout=60 * 60 * 3,
    volumes={"/models": volume},
    secrets=[modal.Secret.from_name("huggingface")],
)
def burst() -> str:
    env = dict(os.environ)
    env.setdefault("HF_HUB_ENABLE_HF_TRANSFER", "1")
    if not (Path(MODEL_DIR) / "config.json").exists():
        LOG.info("downloading autotrust/JEV-27B (~54 GB) ...")
        subprocess.run(
            ["hf", "download", "autotrust/JEV-27B", "--local-dir", MODEL_DIR],
            check=True, env=env,
        )
        volume.commit()

    serve_cmd = [
        sys.executable, f"{MODEL_DIR}/serve_decide.py",
        "--model", MODEL_DIR,
        "--served-model-name", "autotrust/JEV-27B",
        "--enable-lora", "--max-lora-rank", "32",
        "--lora-modules", f"jev-decision={MODEL_DIR}/adapter_vllm",
        "--logprobs-mode", "processed_logprobs",
        "--max-model-len", "32768",
        "--port", "8000",
    ]
    server = subprocess.Popen(serve_cmd, stdout=sys.stdout, stderr=sys.stderr)
    try:
        wait_ready("http://127.0.0.1:8000")
        tasks = ",".join(
            str(REPO_ROOT / "jevbench-ref" / "datasets" / "public" / f)
            for f in ("easy.jsonl", "hard.jsonl", "original.jsonl"))
        out = "/runs/jev27"
        replay_cmd = [
            sys.executable,
            str(REPO_ROOT / "OpenCodifier" / "benchmarks" / "decision-model"
                / "runner" / "run_jevbench.py"),
            "--arm", "jev27",
            "--jev27-endpoint", "http://127.0.0.1:8000",
            "--jev27-model", "autotrust/JEV-27B",
            "--tasks", tasks,
            "--out-dir", out,
            "--timeout", "600",
        ]
        subprocess.run(replay_cmd, check=True)
        summary = Path(out) / "summary.json"
        LOG.info("SUMMARY: %s", summary.read_text() if summary.exists()
                 else "(summary missing)")
        return str(summary) if summary.exists() else out
    finally:
        server.terminate()
        server.wait(timeout=30)


@app.local_entrypoint()
def main() -> None:
    result = burst.remote()
    LOG.info("result: %s", result)
