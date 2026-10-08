#!/usr/bin/env bash
# Reproduces the derived board artifacts from the frozen evidence of
# 20261008T142909Z (git 85ff920d3fa5). Run from the repository root.
# 1. verify the inputs are byte-identical to what this freeze hashed:
#      sha256sum --check results/evidence/EVIDENCE-20261008T142909Z.txt
set -euo pipefail
RUNS='/nas/Temp/work/oc-model-eval/runs' python3 benchmarks/decision-model/runner/parity.py \
  --runs-dir "$RUNS"
RUNS='/nas/Temp/work/oc-model-eval/runs' python3 benchmarks/decision-model/runner/board_csv.py \
  --runs-dir "$RUNS"
python3 benchmarks/decision-model/runner/summarize.py \
  --results-dir benchmarks/decision-model/results
