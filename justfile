# OpenCodifier development tasks.
# Quality gates mirror CI exactly: `just ci` is the pre-push contract.

default := "ci"

# Full CI gate: format, lint, test, doc, deny, audit, pattern scan.
ci: fmt-check lint test doc deny scan

# Criterion benches for the D9 per-stage budgets (DECISIONS.md D9).
bench:
    cargo bench -p opencodifier-engine --bench engine
    cargo bench -p opencodifier-http --bench http

# Format all code.
fmt:
    cargo fmt --all

# Check formatting without writing.
fmt-check:
    cargo fmt --all -- --check

# Clippy with the workspace lint contract (deny warnings).
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Run the full test suite (unit + integration).
test:
    cargo test --workspace

# Build and test documentation examples.
doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
    cargo test --doc

# Dependency license/advisory/ban checks.
deny:
    cargo deny check

# Known-vulnerability audit of the dependency graph.
audit:
    cargo audit

# Security pattern scan: fail on findings newer than the baseline.
scan:
    aegis --format json scan --file . --baseline .aegis/baseline.json --quiet

# Unused-dependency check.
machete:
    cargo machete

# End-to-end validation of the release binary (PLAN 18d): serve + MCP over
# the real wire. Requires a prior `just build-release`.
e2e port='8188':
    python3 scripts/e2e_validate.py --binary target/release/opencodifier --port {{port}}

# The WASM artifact proof (PLAN 18j, D23): host tests, the wasm32 compile,
# a wasm-pack --target nodejs build, and the Node smoke test over the real
# artifact. NOT part of `just ci` — Node and wasm-pack are not CI
# dependencies; run this when the wasm boundary itself changes.
check-wasm:
    #!/usr/bin/env bash
    set -euo pipefail
    export TMPDIR="${TMPDIR:-/nas/Temp/tmp}"
    cargo test -p opencodifier-wasm
    cargo check -p opencodifier-wasm --target wasm32-unknown-unknown
    (cd crates/opencodifier-wasm && wasm-pack build --target nodejs --out-dir pkg)
    node crates/opencodifier-wasm/tests/node/smoke.cjs

# Pins for `just ci-image`; the Dockerfile ARG is the pin of record — keep
# the two in sync. Override via env when re-pinning.
aegis_src := env_var_or_default('AEGIS_SRC', '/nas/Temp/repos/aegis')
aegis_rev := env_var_or_default('AEGIS_REV', '445cf4c807c2e76b62382da60cb5def3a02be855')
ci_image := 'opencodifier-ci-rust:2'

# Build the runner-local CI image used by .gitforge.yml.
#
# aegis enters as build-context source (pinned rev) because job and build
# containers cannot reach the host git-server: the host firewall INPUT
# chain is default-DROP for docker-sourced traffic. cargo-deny and
# cargo-audit install from crates.io at build time. The contract and tag
# rules live in infrastructure/docker/ci-rust.Dockerfile.
ci-image:
    #!/usr/bin/env bash
    set -euo pipefail
    test -d "{{aegis_src}}/.git" \
        || { echo "aegis checkout not found: {{aegis_src}}" >&2; exit 1; }
    git -C "{{aegis_src}}" cat-file -e "{{aegis_rev}}^{commit}" \
        || { echo "rev not present in {{aegis_src}}: {{aegis_rev}}" >&2; exit 1; }
    ctx="$(mktemp -d /nas/Temp/tmp/opencodifier-ci-image.XXXXXX)"
    trap 'find "$ctx" -mindepth 1 -depth -delete; rmdir "$ctx"' EXIT
    git archive HEAD | tar -x -C "$ctx"
    mkdir "$ctx/aegis"
    git -C "{{aegis_src}}" archive "{{aegis_rev}}" | tar -x -C "$ctx/aegis"
    docker build -f infrastructure/docker/ci-rust.Dockerfile \
        --build-arg AEGIS_REV="{{aegis_rev}}" \
        -t "{{ci_image}}" "$ctx"
    docker run --rm "{{ci_image}}" sh -c \
        'cargo deny --version && cargo audit --version && aegis --version \
         && cargo llvm-cov --version'

# Line coverage report (HTML in coverage/).
coverage:
    cargo llvm-cov --workspace --html --output-dir coverage

# Coverage summary with line percentage.
coverage-summary:
    cargo llvm-cov --workspace --summary-only

# Release build of all binaries.
build-release:
    cargo build --release

# Targets the release matrix builds (D24: only what this host can name
# evidence for). Adding a target means adding its builder to the case in
# `release-build` — an absent toolchain fails the recipe with the missing
# tool's name; the matrix never narrows silently.
release_targets := 'x86_64-unknown-linux-gnu x86_64-unknown-linux-musl aarch64-unknown-linux-gnu x86_64-pc-windows-gnu x86_64-apple-darwin aarch64-apple-darwin aarch64-linux-android aarch64-apple-ios wasm32-unknown-unknown'

# Build, stage, and attest one matrix target:
#   just release-build aarch64-unknown-linux-gnu [preflight|build]
#
# `preflight` stops after the toolchain check, so the whole matrix can be
# probed without paying for builds. Artifacts land in
# dist/v<version>-<target>/ with an attestation beside them.
release-build target mode='build':
    #!/usr/bin/env bash
    set -euo pipefail
    target="{{target}}"
    mode="{{mode}}"
    release_targets="{{release_targets}}"
    repo_root="$(pwd)"
    missing=0
    builder=""

    need() {
        if command -v "$1" >/dev/null 2>&1; then
            printf '  ok        %s\n' "$1"
        else
            printf '  MISSING   %s\n' "$1" >&2
            missing=$((missing + 1))
        fi
    }

    # The Android leg links against the NDK's sysroot; without it the
    # build dies at link time ("unable to find dynamic system library
    # 'dl'"), so preflight refuses up front and names the fix.
    android_linker=""
    need_android_ndk() {
        local ndk="${ANDROID_NDK_HOME:-}"
        local wrapper
        if [ -n "$ndk" ]; then
            wrapper="$(ls "$ndk"/toolchains/llvm/prebuilt/*/bin/aarch64-linux-android24-clang 2>/dev/null | head -1)"
        fi
        if [ -n "${wrapper:-}" ] && [ -x "$wrapper" ]; then
            android_linker="$wrapper"
            printf '  ok        android NDK linker (%s)\n' "${wrapper%/toolchains*}"
        else
            printf '  MISSING   android NDK (fix: set ANDROID_NDK_HOME to an NDK with\n' >&2
            printf '            toolchains/llvm/prebuilt/*/bin/aarch64-linux-android24-clang)\n' >&2
            missing=$((missing + 1))
        fi
    }

    preflight() {
        # The tools every leg of the recipe depends on, before anything that
        # would die on their absence instead of naming them.
        need rustc
        need cargo
        need python3
        # std for the target, read from rustc's own sysroot view so the check
        # does not depend on rustup being installed.
        if [ "$missing" -eq 0 ]; then
            local std_dir
            std_dir="$(rustc --print target-libdir --target "$target")"
            if [ -d "$std_dir" ]; then
                printf '  ok        rust std for %s\n' "$target"
            else
                printf '  MISSING   rust std for %s (fix: rustup target add %s)\n' \
                    "$target" "$target" >&2
                missing=$((missing + 1))
            fi
        fi
        case "$target" in
            x86_64-unknown-linux-gnu)
                need cc
                builder="cargo build"
                package="opencodifier-cli"
                ;;
            x86_64-unknown-linux-musl)
                need cc
                need musl-gcc
                builder="cargo build"
                package="opencodifier-cli"
                ;;
            aarch64-unknown-linux-gnu | x86_64-pc-windows-gnu | \
            x86_64-apple-darwin | aarch64-apple-darwin)
                need cargo-zigbuild
                need zig
                builder="cargo zigbuild"
                package="opencodifier-cli"
                ;;
            aarch64-linux-android)
                # Android's system libraries (libdl, liblog, libunwind) ship
                # only in the NDK — zig's fallback search cannot produce a
                # linked binary without it, which the v0.5.0 matrix run
                # proved by failing at link time after a toolchain-only
                # preflight. The NDK's own clang wrapper is the linker.
                need_android_ndk
                export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$android_linker"
                builder="cargo build"
                package="opencodifier-cli"
                ;;
            aarch64-apple-ios)
                # Plain cargo, not zigbuild: zigbuild refuses this target,
                # and a static library is an archive of objects, not a
                # linked image, so cargo never invokes a linker anyway —
                # no Apple SDK needed. The artifact is the
                # opencodifier-ffi staticlib (C ABI, include/ocffi.h).
                builder="cargo build"
                package="opencodifier-ffi"
                ;;
            wasm32-unknown-unknown)
                need wasm-pack
                builder="wasm-pack build"
                package="opencodifier-wasm"
                ;;
            *)
                printf 'unknown matrix target: %s (matrix: %s)\n' \
                    "$target" "$release_targets" >&2
                exit 1
                ;;
        esac
    }

    if [ -z "$target" ]; then
        printf 'usage: just release-build <target> (matrix: %s)\n' \
            "$release_targets" >&2
        exit 1
    fi

    printf 'release-preflight %s\n' "$target"
    preflight
    if [ "$missing" -gt 0 ]; then
        printf 'FAIL  %s: %d tool(s) missing — install them to build this target\n' \
            "$target" "$missing" >&2
        exit 1
    fi
    printf '  ready     builder: %s\n' "$builder"
    if [ "$mode" = "preflight" ]; then
        exit 0
    fi
    if [ "$mode" != "build" ]; then
        printf 'unknown mode: %s (preflight|build)\n' "$mode" >&2
        exit 1
    fi

    version="$(cargo metadata --no-deps --format-version 1 | python3 -c \
        'import json, sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "opencodifier-cli"))')"

    # Per-leg build + staging. Every branch ends with `built` pointing at
    # the artifact this leg attests.
    case "$target" in
        wasm32-unknown-unknown)
            # The site's `just site-wasm` builds only the web target into
            # site/wasm; this release bundle ships BOTH glue targets so a
            # bundler and a Node host each get the shape they import.
            wasm_root="target/wasm-release"
            rm -rf "$wasm_root"
            mkdir -p "$wasm_root"
            # wasm-pack resolves --out-dir against the crate dir; absolute
            # keeps both outputs side by side regardless.
            wasm-pack build crates/opencodifier-wasm --release \
                --target web --out-dir "$PWD/$wasm_root/pkg-web" --out-name opencodifier
            wasm-pack build crates/opencodifier-wasm --release \
                --target nodejs --out-dir "$PWD/$wasm_root/pkg-nodejs" --out-name opencodifier
            stage="dist/v${version}-${target}"
            mkdir -p "$stage"
            asset="opencodifier-${version}-${target}.tar.gz"
            tar -czf "${stage}/${asset}" -C "$wasm_root" pkg-web pkg-nodejs
            built="${stage}/${asset}"
            build_command="wasm-pack build crates/opencodifier-wasm --release --target web --out-dir $wasm_root/pkg-web --out-name opencodifier && wasm-pack build crates/opencodifier-wasm --release --target nodejs --out-dir $wasm_root/pkg-nodejs --out-name opencodifier && tar -czf ${stage}/${asset} -C $wasm_root pkg-web pkg-nodejs"
            ;;
        aarch64-apple-ios)
            # shellcheck disable=SC2086
            $builder --release --locked --target "$target" -p "$package"
            built="target/${target}/release/libopencodifier_ffi.a"
            if [ ! -s "$built" ]; then
                printf 'FAIL  %s: the build reported success but left no artifact\n' "$built" >&2
                exit 1
            fi
            stage="dist/v${version}-${target}"
            mkdir -p "$stage"
            asset="opencodifier-${version}-${target}.a"
            cp -f "$built" "${stage}/${asset}"
            build_command="cargo build --release --locked --target ${target} -p ${package}"
            ;;
        x86_64-pc-windows-gnu)
            # shellcheck disable=SC2086
            $builder --release --locked --target "$target" -p "$package"
            built="target/${target}/release/opencodifier.exe"
            if [ ! -s "$built" ]; then
                printf 'FAIL  %s: the build reported success but left no artifact\n' "$built" >&2
                exit 1
            fi
            stage="dist/v${version}-${target}"
            mkdir -p "$stage"
            asset="opencodifier-${version}-${target}.exe"
            cp -f "$built" "${stage}/${asset}"
            build_command="cargo zigbuild --release --locked --target ${target} -p ${package}"
            ;;
        *)
            # Warnings are CI's gate (`.gitforge.yml` lint job, host target);
            # the cross lanes are not rebuilt there, so a release build stays
            # at cargo's default warning level rather than failing on a
            # linker note from zig cc.
            # shellcheck disable=SC2086
            $builder --release --locked --target "$target" -p "$package"
            built="target/${target}/release/opencodifier"
            if [ ! -s "$built" ]; then
                printf 'FAIL  %s: the build reported success but left no artifact\n' "$built" >&2
                exit 1
            fi
            stage="dist/v${version}-${target}"
            mkdir -p "$stage"
            asset="opencodifier-${version}-${target}"
            cp -f "$built" "${stage}/${asset}"
            build_command="${builder} --release --locked --target ${target} -p ${package}"
            ;;
    esac

    printf 'release-attest %s\n' "$target"
    python3 scripts/generate_attestation.py \
        --artifact "${built}" \
        --target "$target" \
        --build-command "$build_command" \
        --repo-root "$repo_root"
    # The attestation travels with the artifact: a release page that ships
    # binaries without the JSON beside each one attests nothing. (The wasm
    # leg stages first and attests the staged bundle, so its attestation
    # already sits beside the artifact — the copy is a same-file no-op
    # there.)
    attestation="${built}.attestation.json"
    if [ ! -s "$attestation" ]; then
        printf 'FAIL  %s: the attest step left no attestation\n' "$attestation" >&2
        exit 1
    fi
    staged_attestation="${stage}/$(basename "$asset").attestation.json"
    if [ "$attestation" != "$staged_attestation" ]; then
        cp -f "$attestation" "$staged_attestation"
    fi
    printf 'staged   %s/%s\n' "$stage" "$asset"

# Preflight every matrix target: toolchain readiness only, no builds.
release-preflight:
    #!/usr/bin/env bash
    set -euo pipefail
    failed=0
    for target in {{release_targets}}; do
        just release-build "$target" preflight || failed=1
    done
    if [ "$failed" -ne 0 ]; then
        printf 'FAIL  at least one matrix target lacks its toolchain\n' >&2
        exit 1
    fi
    printf 'PASS  every matrix target has its toolchain\n'

# Build and attest the whole matrix (D24), staging under dist/.
release-matrix:
    #!/usr/bin/env bash
    set -euo pipefail
    for target in {{release_targets}}; do
        just release-build "$target" build
    done

# Re-verify every attestation under dist/ against its artifact.
release-verify:
    #!/usr/bin/env bash
    set -euo pipefail
    shopt -s nullglob
    attestations=(dist/v*/*.attestation.json)
    if [ "${#attestations[@]}" -eq 0 ]; then
        printf 'no attestations under dist/ — run `just release-build <target>` first\n' >&2
        exit 1
    fi
    for attestation in "${attestations[@]}"; do
        python3 scripts/generate_attestation.py --check "$attestation" "${attestation%.attestation.json}"
    done

# Hash every staged artifact into dist/sha256sums.txt (attestations and
# the notes file excluded — the sums cover the shippable bytes, and the
# attestations carry the same digests individually). Re-run after any
# rebuild; the file is rewritten, never appended to.
release-checksums:
    #!/usr/bin/env bash
    set -euo pipefail
    shopt -s nullglob
    # Scope to this release: prior releases keep their dist/ staging dirs,
    # but a checksum manifest mixing versions would attest nothing.
    version="$(cargo metadata --no-deps --format-version 1 | python3 -c \
        'import json, sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "opencodifier-cli"))')"
    artifacts=()
    for staged in dist/v${version}-*/; do
        for file in "$staged"*; do
            case "$file" in
                *.attestation.json) continue ;;
            esac
            artifacts+=("$file")
        done
    done
    if [ "${#artifacts[@]}" -eq 0 ]; then
        printf 'no staged artifacts under dist/ — run `just release-build <target>` first\n' >&2
        exit 1
    fi
    tmp="$(mktemp)"
    for file in "${artifacts[@]}"; do
        (cd "$(dirname "$file")" && sha256sum "$(basename "$file")") >> "$tmp"
    done
    mkdir -p dist
    mv "$tmp" dist/sha256sums.txt
    printf 'PASS  %d artifact(s) hashed into dist/sha256sums.txt\n' "${#artifacts[@]}"

# Render dist/RELEASE_NOTES.md from the template plus the staged attestations.
release-notes:
    #!/usr/bin/env bash
    set -euo pipefail
    shopt -s nullglob
    # Scope to this release, like release-checksums: prior releases keep
    # their staging dirs, and one release's notes cannot cite another's
    # build attestations (the generator itself refuses mixed commits).
    version="$(cargo metadata --no-deps --format-version 1 | python3 -c \
        'import json, sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "opencodifier-cli"))')"
    attestations=(dist/v${version}-*/*.attestation.json)
    if [ "${#attestations[@]}" -eq 0 ]; then
        printf 'no attestations under dist/ — run `just release-build <target>` first\n' >&2
        exit 1
    fi
    args=()
    for attestation in "${attestations[@]}"; do
        args+=(--attestation "$attestation")
    done
    python3 scripts/generate_release_notes.py \
        --template docs/RELEASE_NOTES_TEMPLATE.md \
        --output dist/RELEASE_NOTES.md \
        --force \
        "${args[@]}"

# Sync the benchmark board CSV into the site deploy directory. Run after any
# `runner/board_csv.py` regeneration and before `wrangler pages deploy site`
# so the live board page serves the same rows the docs of record do.
site-board:
    cp benchmarks/decision-model/results/board.csv site/board.csv

# Build the site's WASM engine bundle (the try.html playground). site/wasm is
# a build artifact, not source — gitignored and rebuilt by this recipe before
# every deploy. The web-target glue exposes its async initializer as the
# DEFAULT export (`__wbg_init as default`), which site/demo.js relies on.
site-wasm:
    cd crates/opencodifier-wasm && wasm-pack build \
        --target web --out-dir ../../site/wasm --out-name opencodifier_wasm

# Remove build artifacts and coverage output.
clean:
    cargo clean
    rm -rf coverage
