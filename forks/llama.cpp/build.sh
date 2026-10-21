#!/usr/bin/env bash
# Build llama-server from the decision fork.
# Usage: ./build.sh [clean]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUILD_DIR="${SCRIPT_DIR}/build"
PARALLEL_DECISION_REPO="${PARALLEL_DECISION_REPO:-https://github.com/opencodifier/llama.cpp}"
PARALLEL_DECISION_BRANCH="${PARALLEL_DECISION_BRANCH:-main}"

if [ "${1:-}" = "clean" ]; then
    rm -rf "${BUILD_DIR}" "${SCRIPT_DIR}/parallel-decision"
fi

# Clone parallel-decision fork base
if [ ! -d "${SCRIPT_DIR}/parallel-decision" ]; then
    echo "Cloning parallel-decision fork..."
    git clone --depth=1 --branch "${PARALLEL_DECISION_BRANCH}" \
        "${PARALLEL_DECISION_REPO}" \
        "${SCRIPT_DIR}/parallel-decision"
fi

cd "${SCRIPT_DIR}/parallel-decision"

# Fetch woct0rdho fast LoRA branch
git remote add woct0rdho https://github.com/woct0rdho/llama.cpp 2>/dev/null || true
git fetch woct0rdho --depth=1 2>/dev/null || echo "woct0rdho remote not available, building base only"

# Check if woct0rdho/strix branch exists and apply if available
STRIX_COMMIT=""
if git rev-parse woct0rdho/strix >/dev/null 2>&1; then
    echo "Applying woct0rdho/strix fast LoRA kernels..."
    STRIX_COMMIT="$(git rev-parse woct0rdho/strix)"
    # Merge the fast LoRA branch (non-interactive)
    git merge --no-edit "${STRIX_COMMIT}" || echo "Merge skipped: no overlap with base"
else
    echo "woct0rdho/strix branch not available — building base parallel-decision fork"
fi

mkdir -p "${BUILD_DIR}"
cd "${BUILD_DIR}"

# Configure with CMake — decision-only features
cmake .. \
    -DCMAKE_BUILD_TYPE=Release \
    -DLLAMA_SERVER=ON \
    -DLLAMA_EXAMPLES=OFF \
    -DLLAMA_TESTS=OFF \
    -DLLAMA_TOOLS=OFF \
    -DLLAMA_BUILD_SERVER=ON \
    -DGGML_NATIVE=ON \
    -DGGML_LLAMAFILE=OFF \
    -DGGML_STATIC=ON \
    -DGGML_STRIP=ON

# Build llama-server only
cmake --build . --config Release --target llama-server -j"$(nproc)"

# Find the binary
SERVERBIN=""
for path in "${BUILD_DIR}/bin/llama-server" "${BUILD_DIR}/llama-server"; do
    if [ -x "${path}" ]; then
        SERVERBIN="${path}"
        break
    fi
done

if [ -z "${SERVERBIN}" ]; then
    echo "ERROR: llama-server binary not found after build"
    exit 1
fi

echo ""
echo "Build complete: ${SERVERBIN}"
ls -lh "${SERVERBIN}"
