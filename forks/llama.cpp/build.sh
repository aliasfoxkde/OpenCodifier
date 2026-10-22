#!/usr/bin/env bash
# Build llama-server from the parallel-decision fork (D41).
#
# Usage: ./build.sh [clean]
#
# Environment:
#   BACKEND                cpu (default) | cuda | vulkan
#   PARALLEL_DECISION_REPO fork git URL
#   PARALLEL_DECISION_BRANCH fork branch (default: parallel-decision-oc)
#
# Output: forks/llama.cpp/build/bin/llama-server
#         forks/llama.cpp/build/BUILD-INFO.txt  (commit + flags, for release manifests)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUILD_DIR="${SCRIPT_DIR}/build"
SRC_DIR="${SCRIPT_DIR}/parallel-decision"
PARALLEL_DECISION_REPO="${PARALLEL_DECISION_REPO:-https://github.com/aliasfoxkde/llama.cpp}"
PARALLEL_DECISION_BRANCH="${PARALLEL_DECISION_BRANCH:-parallel-decision-oc}"
BACKEND="${BACKEND:-cpu}"

if [ "${1:-}" = "clean" ]; then
    rm -rf "${BUILD_DIR}" "${SRC_DIR}"
fi

# Clone the parallel-decision base — the tree-mode /v1/decision endpoint
# (tools/server/server.cpp routes.post_decision) is what the whole path
# exists for; a stock llama-server cannot score decision weights.
#
# The base is `thecodacus/llama.cpp` @ `parallel-decision` (upstream
# PR-style history, commit 8f02796), plus the one OpenCodifier commit
# that the Rust client requires: `7ae36f4` emits the full per-choice
# `distribution` map next to the winner-only `probability`, and
# `LlamaDecisionClassifier` refuses a winner-only response rather than
# inventing mass. `aliasfoxkde/llama.cpp` @ `parallel-decision-oc`
# carries that commit on top of the upstream base, and is the default
# here; the upstream branch is the fallback if the fork is unreachable.
if [ ! -d "${SRC_DIR}" ]; then
    echo "Cloning ${PARALLEL_DECISION_REPO} @ ${PARALLEL_DECISION_BRANCH}..."
    git clone --depth=1 --branch "${PARALLEL_DECISION_BRANCH}" \
        "${PARALLEL_DECISION_REPO}" \
        "${SRC_DIR}"
fi

cd "${SRC_DIR}"
BASE_COMMIT="$(git rev-parse HEAD)"

# Optional: woct0rdho fast LoRA kernels (AMD RDNA 3.5; the NVIDIA CUDA
# port does not exist yet — see docs/RESEARCH.md §28). Skip silently
# when the branch is unavailable; the base fork alone is a valid build.
git remote add woct0rdho https://github.com/woct0rdho/llama.cpp 2>/dev/null || true
if git fetch woct0rdho --depth=1 strix 2>/dev/null; then
    echo "Applying woct0rdho/strix fast LoRA kernels..."
    git merge --no-edit FETCH_HEAD || echo "Merge skipped: no overlap with base"
else
    echo "woct0rdho/strix not available — building base parallel-decision fork"
fi
FINAL_COMMIT="$(git rev-parse HEAD)"

# Backend selection: one flag set per target; everything else off is
# handled by the LLAMA_BUILD_* knobs below.
BACKEND_FLAGS=()
case "${BACKEND}" in
    cpu) ;;
    cuda) BACKEND_FLAGS+=(-DGGML_CUDA=ON) ;;
    vulkan) BACKEND_FLAGS+=(-DGGML_VULKAN=ON) ;;
    *) echo "ERROR: BACKEND must be cpu, cuda, or vulkan (got ${BACKEND})" >&2; exit 1 ;;
esac

mkdir -p "${BUILD_DIR}"
cd "${BUILD_DIR}"

# The option names are LLAMA_BUILD_* — there is no LLAMA_SERVER /
# LLAMA_EXAMPLES / GGML_STRIP (an earlier revision of this script used
# names that silently defaulted everything ON; do not "simplify" them).
#
# Three build traps, each found by an actual build:
#
# 1. The cmake source directory must be "${SRC_DIR}" (the clone), not
#    ".." — the build directory is a sibling of the clone, not a child
#    of it, so ".." points at forks/llama.cpp/ and configures nothing.
# 2. LLAMA_BUILD_TOOLS must stay ON: tools/server/ lives under tools/,
#    and gating tools off silently removes the llama-server target
#    entirely. The --target llama-server step below still compiles only
#    the server and its dependencies, not the other tools.
# 3. GGML_STATIC=ON and BUILD_SHARED_LIBS must agree. The default is
#    BUILD_SHARED_LIBS=ON, which links a shared ggml-base with
#    crtbeginT.o and fails with "relocation R_X86_64_32 against hidden
#    symbol `__TMC_END__'". BUILD_SHARED_LIBS=OFF produces the static
#    binary this lane wants anyway.
cmake "${SRC_DIR}" \
    -DCMAKE_BUILD_TYPE=Release \
    -DLLAMA_BUILD_SERVER=ON \
    -DLLAMA_BUILD_EXAMPLES=OFF \
    -DLLAMA_BUILD_TESTS=OFF \
    -DLLAMA_BUILD_TOOLS=ON \
    -DGGML_NATIVE=ON \
    -DBUILD_SHARED_LIBS=OFF \
    -DGGML_STATIC=ON \
    "${BACKEND_FLAGS[@]+"${BACKEND_FLAGS[@]}"}"

cmake --build . --config Release --target llama-server -j"$(nproc)"

SERVERBIN=""
for path in "${BUILD_DIR}/bin/llama-server" "${BUILD_DIR}/llama-server"; do
    if [ -x "${path}" ]; then
        SERVERBIN="${path}"
        break
    fi
done

if [ -z "${SERVERBIN}" ]; then
    echo "ERROR: llama-server binary not found after build" >&2
    exit 1
fi

# Record provenance next to the binary: which fork commit, which flags,
# which backend — the release manifest line and the --llama-model-id
# decoration both trace back to this.
cat > "${BUILD_DIR}/BUILD-INFO.txt" <<EOF
binary=${SERVERBIN}
repo=${PARALLEL_DECISION_REPO}
branch=${PARALLEL_DECISION_BRANCH}
base_commit=${BASE_COMMIT}
final_commit=${FINAL_COMMIT}
backend=${BACKEND}
built_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)
host=$(uname -srm)
EOF

echo ""
echo "Build complete: ${SERVERBIN}"
ls -lh "${SERVERBIN}"
cat "${BUILD_DIR}/BUILD-INFO.txt"
