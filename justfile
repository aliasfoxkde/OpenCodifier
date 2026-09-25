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
