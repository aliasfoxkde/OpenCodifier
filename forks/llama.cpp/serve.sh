#!/usr/bin/env bash
# Start llama-server from the decision fork.
# Usage: ./serve.sh --base <base.gguf> --lora <adapter.gguf> [--port 8080] [--host 127.0.0.1]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUILD_DIR="${SCRIPT_DIR}/build"
SERVERBIN=""

# Find llama-server binary
for path in "${BUILD_DIR}/bin/llama-server" "${BUILD_DIR}/llama-server"; do
    if [ -x "${path}" ]; then
        SERVERBIN="${path}"
        break
    fi
done

if [ -z "${SERVERBIN}" ]; then
    echo "ERROR: llama-server not found. Run ./build.sh first."
    exit 1
fi

HOST="127.0.0.1"
PORT="8080"
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
        --help|-h)
            echo "Usage: $0 --base <base.gguf> --lora <adapter.gguf> [--port 8080] [--host 127.0.0.1]"
            exit 0 ;;
        *)
            echo "Unknown option: ${1}"; exit 1 ;;
    esac
done

if [ -z "${BASE_MODEL}" ]; then
    echo "ERROR: --base <model.gguf> is required"
    exit 1
fi

if [ ! -f "${BASE_MODEL}" ]; then
    echo "ERROR: Base model not found: ${BASE_MODEL}"
    exit 1
fi

LORA_ARGS=()
if [ -n "${LORA_ADAPTER}" ]; then
    if [ ! -f "${LORA_ADAPTER}" ]; then
        echo "ERROR: LoRA adapter not found: ${LORA_ADAPTER}"
        exit 1
    fi
    LORA_ARGS=(--lora "${LORA_ADAPTER}")
fi

echo "Starting llama-server..."
echo "  Binary: ${SERVERBIN}"
echo "  Base:   ${BASE_MODEL}"
echo "  LoRA:   ${LORA_ADAPTER:-none}"
echo "  Host:   ${HOST}:${PORT}"
echo ""

exec "${SERVERBIN}" \
    --host "${HOST}" \
    --port "${PORT}" \
    --model "${BASE_MODEL}" \
    --ctx-size 2048 \
    --parallel 1 \
    "${LORA_ARGS[@]+"${LORA_ARGS[@]}"}"
