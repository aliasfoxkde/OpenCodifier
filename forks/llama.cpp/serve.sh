#!/usr/bin/env bash
# Start llama-server from the decision fork.
#
# Usage: ./serve.sh --base <base.gguf> [--lora <adapter.gguf>]
#                   [--port 8080] [--host 127.0.0.1] [--decision-seqs 8]
#
# The server must run with --decision-seqs N (N >= 3): the decision
# engine needs a cached prefix sequence, a trunk sequence, and at least
# one branch sequence. Without it the server starts but refuses
# /v1/decision with "decisions are disabled".
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUILD_DIR="${SCRIPT_DIR}/build"
SERVERBIN=""

for path in "${BUILD_DIR}/bin/llama-server" "${BUILD_DIR}/llama-server"; do
    if [ -x "${path}" ]; then
        SERVERBIN="${path}"
        break
    fi
done

if [ -z "${SERVERBIN}" ]; then
    echo "ERROR: llama-server not found. Run ./build.sh first." >&2
    exit 1
fi

HOST="127.0.0.1"
PORT="8080"
DECISION_SEQS="8"
BASE_MODEL=""
LORA_ADAPTER=""

while [[ $# -gt 0 ]]; do
    case "${1}" in
        --base)
            BASE_MODEL="${2}"; shift 2 ;;
        --lora)
            LORA_ADAPTER="${2}"; shift 2 ;;
        --port)
            PORT="${2}"; shift 2 ;;
        --host)
            HOST="${2}"; shift 2 ;;
        --decision-seqs)
            DECISION_SEQS="${2}"; shift 2 ;;
        --help|-h)
            echo "Usage: $0 --base <base.gguf> [--lora <adapter.gguf>] [--port 8080] [--host 127.0.0.1] [--decision-seqs 8]"
            exit 0 ;;
        *)
            echo "Unknown option: ${1}" >&2; exit 1 ;;
    esac
done

if [ -z "${BASE_MODEL}" ]; then
    echo "ERROR: --base <model.gguf> is required" >&2
    exit 1
fi

if [ ! -f "${BASE_MODEL}" ]; then
    echo "ERROR: Base model not found: ${BASE_MODEL}" >&2
    exit 1
fi

if [ "${DECISION_SEQS}" -lt 3 ]; then
    echo "ERROR: --decision-seqs must be at least 3 (cached prefix, trunk, one branch)" >&2
    exit 1
fi

LORA_ARGS=()
if [ -n "${LORA_ADAPTER}" ]; then
    if [ ! -f "${LORA_ADAPTER}" ]; then
        echo "ERROR: LoRA adapter not found: ${LORA_ADAPTER}" >&2
        exit 1
    fi
    LORA_ARGS=(--lora "${LORA_ADAPTER}")
fi

echo "Starting llama-server..."
echo "  Binary:  ${SERVERBIN}"
echo "  Base:    ${BASE_MODEL}"
echo "  LoRA:    ${LORA_ADAPTER:-none}"
echo "  Host:    ${HOST}:${PORT}"
echo "  Decision: /v1/decision with ${DECISION_SEQS} sequences"
echo ""

exec "${SERVERBIN}" \
    --host "${HOST}" \
    --port "${PORT}" \
    --model "${BASE_MODEL}" \
    --ctx-size 2048 \
    --parallel 1 \
    --decision-seqs "${DECISION_SEQS}" \
    "${LORA_ARGS[@]+"${LORA_ARGS[@]}"}"
