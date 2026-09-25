# Pre-warmed Rust CI image for the OpenCodifier pipeline (opencodifier-ci).
#
# Tools are baked into the image instead of fetched at job runtime — the
# runner's preflight (F25) contract, and here also a hard requirement: job
# AND build containers cannot reach host services at all. The host firewall
# INPUT chain is default-DROP for docker-sourced traffic (only DNS/53 is
# accepted), so the GitForge git-server on 127.0.0.1:42782 is unreachable
# from inside any container — this is why the supply-chain job's
# `cargo install --git http://127.0.0.1:42782/...aegis.git` step of run
# 8185900a died with "connection refused" and no URL alternative could have
# fixed it. Tools enter this image in exactly two ways:
#
#   1. from crates.io (registry egress from containers works — run 8185900a
#      compiled cargo-deny and cargo-audit in-job), used for cargo-deny and
#      cargo-audit;
#   2. from build-context source for aegis, which is not published to
#      crates.io. `just ci-image` assembles the context from a clean
#      `git archive` of the pinned aegis rev.
#
# Unlike dsc-ci-rust (GitForge's own CI image) this image deliberately does
# NOT set CARGO_NET_OFFLINE: the supply-chain job's advisory databases
# (cargo-audit, cargo-deny) need egress at job runtime, and OpenCodifier
# jobs demonstrably complete crates.io transfers on this network. The baked
# registry cache below is a warm-start optimization, not an offline
# contract.
#
# Cache contract: bump the `image:` tag in .gitforge.yml whenever a pin in
# this file changes (toolchain, cargo-deny, cargo-audit, AEGIS_REV) or
# Cargo.lock changes. ensure_image() prefers the local image store; an
# absent tag fails fast with a pull error rather than silently running old
# tools. The image is runner-local and never published.
#
# Rebuild:  just ci-image
#
# The base tag is exact on purpose: RUSTUP_TOOLCHAIN below pins the same
# version, and the pair must move together (cache contract above).
FROM rust:1.90.0

# Exact tool pins. cargo-deny/cargo-audit track the versions the local
# quality gates run (host ~/.cargo/bin as of 2026-09-25).
ARG CARGO_DENY_VERSION=0.20.2
ARG CARGO_AUDIT_VERSION=0.22.2
# aegis: pinned rev of the GitForge-hosted repo. This is the
# fix/state-dir-self-scan change (0.6.3) — the .aegis-state-exclusion fix
# the committed .aegis/baseline.json was generated with, so CI must scan
# with the same tool or the gate is not the documented gate. Keep in sync
# with AEGIS_REV in the justfile ci-image recipe (the recipe passes this
# pin explicitly). Advance the pin when the branch merges upstream.
ARG AEGIS_REV=445cf4c807c2e76b62382da60cb5def3a02be855

RUN cargo install --locked \
        "cargo-deny@${CARGO_DENY_VERSION}" \
        "cargo-audit@${CARGO_AUDIT_VERSION}"

COPY aegis/ /opt/aegis-src/
# the `aegis` binary is the aegis-cli package of that workspace
RUN cargo install --locked --path /opt/aegis-src/crates/aegis-cli

# rust-toolchain.toml in job workspaces selects channel "stable" (local-dev
# convenience, see that file). Without a pin, rustup resolves that channel
# at job runtime by downloading the latest stable — measured mid-change on
# the stock image: 1.98.1 downloaded inside a job container, meaning every
# job ran a toolchain the image never baked (a silent F25 violation).
# RUSTUP_TOOLCHAIN beats the override file (rustup itself reports
# "overridden by environment variable RUSTUP_TOOLCHAIN"), so every job runs
# exactly the baked compiler. Keep in lockstep with the FROM above.
ENV RUSTUP_TOOLCHAIN=1.90.0

# Warm the crate registry from the committed lockfile. Must precede the
# workspace manifests check below: this is the one workspace step allowed to
# use the network (at image build time).
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
RUN cargo fetch --locked

# The official rust image ships the minimal component profile — rustfmt and
# clippy are absent (verified: `cargo fmt` fails with "'cargo-fmt' is not
# installed"). Placed after the expensive layers so a change here rebuilds
# only this step.
RUN rustup component add rustfmt clippy

# Fail the build, not a job, if any tool is broken or mispinned.
RUN cargo deny --version && cargo audit --version && aegis --version \
    && cargo fmt --version && cargo clippy --version
