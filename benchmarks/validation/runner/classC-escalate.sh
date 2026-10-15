#!/bin/bash
# Class-C escalate arm: fusion-v2 ladder + Jev-Style-0.8B-Decision-v3 rung
# (build-pd, /v1/decision, --decision-seqs 8). Model substitution chain
# recorded in results/VALIDATION.md: runbook qate2b purged from NAS,
# Julia-1-Q8_0 unloadable in build-pd (mmbert pre-tokenizer) and its
# /v1/decision route absent from build-upstream; Jev-Style v3 is the
# decision-trained qwen-arch model both the fork loads and the rung's
# purpose wants. sha256 pinned at classC/jevstyle.sha256.
set -u
RUNS=$HOME/oc-model-eval/runs/realworld
BIN=$RUNS/binaries/opencodifier-llamacpp
SUITE=$HOME/repos/OpenCodifier/benchmarks/validation/suite/longctx_tiers.json
RUNNER=$HOME/repos/OpenCodifier/benchmarks/validation/runner/run_longctx.py
echo "=== escalate start $(date -Is) load=$(cut -d' ' -f1 /proc/loadavg) ==="
python3 $RUNNER --binary $BIN --suite $SUITE --arm escalate --port 8093 \
  --ladder $HOME/repos/OpenCodifier/ladders/fusion-v2.json \
  --llama http://127.0.0.1:8094 --llama-model-id jev-style-0.8b-decision-v3 \
  --out $RUNS/classC/C-escalate.json || echo "C-escalate rc=$?"
echo "=== escalate end $(date -Is) ==="
