# OpenCodifier development tasks.
# Quality gates mirror CI exactly: `just ci` is the pre-push contract.

default := "ci"

# Full CI gate: format, lint, test, doc, deny, audit, pattern scan.
ci: fmt-check lint test doc deny scan

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

# Pins for `just ci-image`; the Dockerfile ARG is the pin of record — keep
# the two in sync. Override via env when re-pinning.
aegis_src := env_var_or_default('AEGIS_SRC', '/nas/Temp/repos/aegis')
aegis_rev := env_var_or_default('AEGIS_REV', '445cf4c807c2e76b62382da60cb5def3a02be855')
ci_image := 'opencodifier-ci-rust:1'

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
        'cargo deny --version && cargo audit --version && aegis --version'

# Line coverage report (HTML in coverage/).
coverage:
    cargo llvm-cov --workspace --html --output-dir coverage

# Coverage summary with line percentage.
coverage-summary:
    cargo llvm-cov --workspace --summary-only

# Release build of all binaries.
build-release:
    cargo build --release

# Remove build artifacts and coverage output.
clean:
    cargo clean
    rm -rf coverage
