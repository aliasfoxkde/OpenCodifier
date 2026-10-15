#!/usr/bin/env python3
"""Generate a verified build attestation for one OpenCodifier release artifact.

The attestation is a deterministic JSON record of what was built and from
what source: artifact digest and size, target triple, git commit, toolchain
versions, lockfile digest, feature flags, and the build timestamp (ISO-8601
UTC). Key order is fixed by `CANONICAL_KEY_ORDER` and re-checked after
serialization, so two runs over the same inputs differ only in
`generated_at_utc`.

Nothing here is trusted from a single observation: every digest the record
carries is computed from two independent reads that must agree (artifact and
Cargo.lock), the declared target triple is cross-checked against the
artifact's `file` magic, and the written file is read back and re-verified
against the artifact before the tool reports success. `--check` re-runs that
verification against an existing attestation, which is the consumer-side
half of the same contract.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import pathlib
import re
import subprocess
import sys

SCHEMA_VERSION = "opencodifier.attestation/1"

# `file(1)` spells static linking two ways: "statically linked" (non-PIE) and
# "static-pie linked" (PIE, which is what this workspace's release profile
# produces). Both mean the artifact carries no dynamic libc dependency.
STATIC_MAGIC = re.compile(r"\bstatically linked\b|\bstatic-pie linked\b")

# Top-level key order of the emitted document. The writer asserts the
# serialized order matches this list, so a future edit that reorders the
# schema fails here instead of quietly changing the artifact consumers parse.
CANONICAL_KEY_ORDER = [
    "schema_version",
    "artifact",
    "source",
    "build",
    "generated_at_utc",
]

# Per-target artifact contract. Each entry names the `file(1)` container
# token the artifact must carry, the architecture token that must appear
# alongside it (None for bundles with no CPU architecture), and optional
# extra requirements: "static" demands a statically linked binary (the
# reason the musl legs exist); "member" demands that the first archive
# member is itself a Mach-O of the declared architecture (a static library
# is an `ar` archive, and the outer magic alone cannot see its contents).
TARGET_RULES: dict[str, dict[str, object]] = {
    "x86_64-unknown-linux-gnu": {"container": "ELF", "arch": "x86-64"},
    "x86_64-unknown-linux-musl": {
        "container": "ELF",
        "arch": "x86-64",
        "static": True,
    },
    "aarch64-unknown-linux-gnu": {"container": "ELF", "arch": "ARM aarch64"},
    "x86_64-pc-windows-gnu": {"container": "PE32+", "arch": "x86-64"},
    "x86_64-apple-darwin": {"container": "Mach-O", "arch": "x86_64"},
    "aarch64-apple-darwin": {"container": "Mach-O", "arch": "arm64"},
    "aarch64-linux-android": {"container": "ELF", "arch": "ARM aarch64"},
    "aarch64-apple-ios": {
        "container": "ar archive",
        "arch": "arm64",
        "member": True,
    },
    "wasm32-unknown-unknown": {"container": "gzip compressed", "arch": None},
}


class AttestationError(Exception):
    """A failed verification with the observation that failed."""


def run(command: list[str], cwd: pathlib.Path) -> str:
    """Run a command in `cwd`; return stripped stdout or fail with its stderr."""
    proc = subprocess.run(
        command, cwd=cwd, capture_output=True, text=True, check=False
    )
    if proc.returncode != 0:
        raise AttestationError(
            f"{' '.join(command)} failed (rc={proc.returncode}): "
            f"{proc.stderr.strip() or proc.stdout.strip()}"
        )
    return proc.stdout.strip()


def sha256_verified(path: pathlib.Path) -> tuple[str, int]:
    """Hash `path` from two independent reads that must agree exactly.

    Returns (hex digest, byte size). A mismatch between the reads means the
    file changed (or is being written) while it was read; that is a failure,
    never a digest we print anyway.
    """
    observations: list[tuple[str, int]] = []
    for _ in range(2):
        digest = hashlib.sha256()
        size = 0
        with path.open("rb") as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
                size += len(chunk)
        observations.append((digest.hexdigest(), size))
    if observations[0] != observations[1]:
        raise AttestationError(
            f"{path}: unstable while reading "
            f"(read 1 {observations[0][0]}/{observations[0][1]}B, "
            f"read 2 {observations[1][0]}/{observations[1][1]}B)"
        )
    return observations[0]


def git_commit(repo: pathlib.Path) -> dict[str, object]:
    """Identity of the commit the artifact was built from, plus tree state."""
    sha = run(["git", "rev-parse", "HEAD"], repo)
    # git's %cI keeps the author's offset; the field is named _utc, so
    # normalize instead of letting the name lie about the value.
    date = (
        dt.datetime.fromisoformat(run(["git", "show", "-s", "--format=%cI", "HEAD"], repo))
        .astimezone(dt.timezone.utc)
        .replace(microsecond=0)
        .isoformat()
        .replace("+00:00", "Z")
    )
    porcelain = run(["git", "status", "--porcelain"], repo)
    dirty = [line for line in porcelain.splitlines() if line.strip()]
    return {
        "commit": sha,
        "commit_date_utc": date,
        "working_tree_clean": not dirty,
        "dirty_path_count": len(dirty),
    }


def toolchain(repo: pathlib.Path) -> dict[str, object]:
    """Compiler versions and the digests that pin the dependency set."""
    rustc = run(["rustc", "--version"], repo)
    cargo = run(["cargo", "--version"], repo)
    lock = repo / "Cargo.lock"
    lock_sha256, lock_bytes = sha256_verified(lock)
    tree = run(
        ["cargo", "tree", "--locked", "--prefix", "none", "--workspace"], repo
    )
    return {
        "rustc": rustc,
        "cargo": cargo,
        "cargo_lock_sha256": lock_sha256,
        "cargo_lock_bytes": lock_bytes,
        "cargo_tree_sha256": hashlib.sha256(tree.encode()).hexdigest(),
        "cargo_tree_command": "cargo tree --locked --prefix none --workspace",
    }


def verify_magic(magic: str, target: str, path: pathlib.Path) -> None:
    """The artifact's `file` magic must agree with the triple it ships for.

    The declared target's contract comes from TARGET_RULES; an unmapped
    target is an authoring error (add the rule when you add the leg, never
    let an artifact ship unverified). Beyond container and architecture, the
    rule can demand static linkage (musl) or inspect the first archive
    member (iOS static library, whose outer `ar` magic cannot see the
    Mach-O inside).
    """
    rule = TARGET_RULES.get(target)
    if rule is None:
        raise AttestationError(f"{target}: no artifact contract mapped for target")
    if rule["container"] not in magic:
        raise AttestationError(
            f"{path}: expected {rule['container']} for {target} — {magic}"
        )
    arch = rule["arch"]
    if rule.get("member"):
        # An `ar` archive's outer magic names no architecture ("current ar
        # archive"), so the arch contract lives one level down: the member
        # probe below is the arch check.
        if not archive_member_is_macho(str(arch), path):
            raise AttestationError(
                f"{path}: archive holds no Mach-O {arch} object member"
            )
    elif arch is not None and arch not in magic:
        raise AttestationError(
            f"{path}: magic says the artifact is not for {target} — {magic}"
        )
    if rule.get("static") and STATIC_MAGIC.search(magic) is None:
        raise AttestationError(
            f"{path}: this target requires a statically linked artifact — {magic}"
        )


def archive_member_is_macho(arch: str, path: pathlib.Path) -> bool:
    """True when an `ar` archive holds a Mach-O of `arch`.

    Static libraries for the iOS leg are plain archives; the guarantee the
    attestation makes ("this really is an arm64 iOS library") lives one
    level down, in the members. The member bytes go to `file(1)` on stdin,
    binary-safe. Index members are skipped — rustc writes `__.SYMDEF`
    first, and an archive index is neither Mach-O nor evidence of an
    architecture; the probe reads the first real object member.
    """
    try:
        listing = run(["ar", "t", str(path)], path.parent)
    except AttestationError as error:
        if "file format not recognized" in str(error):
            # Not an archive at all (e.g. an ELF handed to this probe):
            # the answer is "no Mach-O member", not a crash.
            return False
        # Anything else is a tool failure on a real archive — reporting
        # it as "no Mach-O member" would make the gate lie.
        raise
    index_members = {"__.SYMDEF", "__.SYMDEF_64", "/", "//"}
    members = [
        line
        for line in listing.splitlines()
        if line.strip() and line.strip() not in index_members
    ]
    if not members:
        return False
    member = subprocess.run(
        ["ar", "p", str(path), members[0]],
        cwd=path.parent,
        capture_output=True,
        check=False,
    )
    if member.returncode != 0 or not member.stdout:
        # A listed member that cannot be extracted is a tool or IO
        # failure, never evidence about the architecture — fail loudly
        # so the operator re-runs instead of trusting a false negative.
        raise AttestationError(
            f"{path}: ar could not extract member {members[0]!r} "
            f"(rc={member.returncode})"
        )
    probe = subprocess.run(
        ["file", "-b", "--", "-"],
        input=member.stdout,
        capture_output=True,
        check=False,
    )
    magic = probe.stdout.decode("utf-8", errors="replace").strip()
    return bool(arch) and arch in magic and "Mach-O" in magic


def artifact_identity(
    path: pathlib.Path, repo: pathlib.Path, target: str
) -> dict[str, object]:
    """Digest, size, and `file` magic of the artifact — verified, not assumed."""
    if not path.is_file():
        raise AttestationError(f"{path}: no such artifact")
    sha256, size = sha256_verified(path)
    magic = run(["file", "-b", "--", str(path)], repo)
    verify_magic(magic, target, path)
    resolved = path.resolve()
    try:
        recorded_path = resolved.relative_to(repo.resolve()).as_posix()
    except ValueError:
        # An attestation that names a path outside the repository it describes
        # is not reproducible by its own recorded build command — refuse it.
        raise AttestationError(
            f"{resolved}: outside the repo root ({repo}); "
            "attest only artifacts inside the repository"
        ) from None
    return {
        "name": path.name,
        "path": recorded_path,
        "sha256": sha256,
        "bytes": size,
        "target": target,
        "file_magic": magic,
    }


def check_written(attestation_path: pathlib.Path, artifact_path: pathlib.Path) -> str:
    """Re-read the attestation we just wrote and re-verify it against the artifact.

    Content identity is the contract, so the digest and size must match, and
    the artifact is re-checked against the recorded target triple's magic; a
    renamed file is reported, not failed, because renaming for upload is a
    normal release step.
    """
    document = json.loads(attestation_path.read_text(encoding="utf-8"))
    keys = list(document)
    if keys != CANONICAL_KEY_ORDER:
        raise AttestationError(
            f"{attestation_path}: key order drifted from the schema — {keys}"
        )
    recorded = document["artifact"]
    sha256, size = sha256_verified(artifact_path)
    if recorded["sha256"] != sha256 or recorded["bytes"] != size:
        raise AttestationError(
            f"{attestation_path}: does not match {artifact_path} "
            f"(recorded {recorded['sha256']}/{recorded['bytes']}B, "
            f"observed {sha256}/{size}B)"
        )
    verify_magic(
        run(["file", "-b", "--", str(artifact_path.resolve())], artifact_path.parent),
        str(recorded["target"]),
        artifact_path,
    )
    if recorded["name"] != artifact_path.name:
        return f"renamed: recorded {recorded['name']}"
    return ""


def build_attestation(
    artifact_path: pathlib.Path,
    repo: pathlib.Path,
    target: str,
    features: str,
    no_default_features: bool,
    build_command: str,
    profile: str,
) -> dict[str, object]:
    """Assemble the document in the canonical key order."""
    artifact = artifact_identity(artifact_path, repo, target)
    timestamp = (
        dt.datetime.now(dt.timezone.utc)
        .replace(microsecond=0)
        .isoformat()
        .replace("+00:00", "Z")
    )
    feature_list = [item for item in features.replace(",", " ").split() if item]
    return {
        "schema_version": SCHEMA_VERSION,
        "artifact": artifact,
        "source": git_commit(repo),
        "build": {
            "profile": profile,
            "features": feature_list,
            "no_default_features": no_default_features,
            "build_command": build_command,
            **toolchain(repo),
        },
        "generated_at_utc": timestamp,
    }


def write_attestation(document: dict[str, object], output: pathlib.Path) -> None:
    """Serialize with fixed key order and a trailing newline."""
    text = json.dumps(document, indent=2, sort_keys=False, ensure_ascii=False) + "\n"
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(text, encoding="utf-8")


def load_attestation(path: pathlib.Path) -> dict[str, object]:
    """Read an attestation and require the schema version we know how to check."""
    document = json.loads(path.read_text(encoding="utf-8"))
    if list(document) != CANONICAL_KEY_ORDER:
        raise AttestationError(
            f"{path}: not a {SCHEMA_VERSION} document — key order is {list(document)}"
        )
    if document["schema_version"] != SCHEMA_VERSION:
        raise AttestationError(
            f"{path}: schema {document['schema_version']}, expected {SCHEMA_VERSION}"
        )
    return document


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Generate (or verify) a build attestation for one release artifact."
    )
    parser.add_argument(
        "--artifact", type=pathlib.Path, help="path to the built artifact"
    )
    parser.add_argument(
        "--target", default="x86_64-unknown-linux-gnu", help="target triple"
    )
    parser.add_argument(
        "--features",
        default="default",
        help="feature flags the artifact was built with, as given to cargo",
    )
    parser.add_argument(
        "--no-default-features",
        action="store_true",
        help="record that the build disabled default features",
    )
    parser.add_argument(
        "--build-command",
        default="",
        help="the exact command that produced the artifact (recorded verbatim)",
    )
    parser.add_argument(
        "--profile", default="release", help="cargo profile of the build"
    )
    parser.add_argument(
        "--repo-root",
        type=pathlib.Path,
        default=pathlib.Path(__file__).resolve().parent.parent,
        help="repository root the attestation describes (default: this repo)",
    )
    parser.add_argument(
        "--output",
        type=pathlib.Path,
        help="attestation path (default: alongside the artifact, .attestation.json)",
    )
    parser.add_argument(
        "--check",
        nargs=2,
        metavar=("ATTESTATION", "ARTIFACT"),
        help="verify an existing attestation against an artifact and exit",
    )
    args = parser.parse_args()

    try:
        if args.check is not None:
            # Resolve before use: the member probe runs `ar` from the
            # artifact's parent directory, so a repo-relative path here
            # would double and read as missing (build mode has resolved
            # its --artifact since it existed; check mode now matches).
            attestation_path, artifact_path = (
                pathlib.Path(args.check[0]).resolve(),
                pathlib.Path(args.check[1]).resolve(),
            )
            document = load_attestation(attestation_path)
            note = check_written(attestation_path, artifact_path)
            print(
                f"PASS  {attestation_path} matches {artifact_path} "
                f"({document['artifact']['sha256']})"
                + (f"  [{note}]" if note else "")
            )
            return 0

        if args.artifact is None:
            parser.error("--artifact is required unless --check is given")

        repo = args.repo_root.resolve()
        artifact_path = args.artifact.resolve()
        output = args.output if args.output is not None else (
            artifact_path.parent / f"{artifact_path.name}.attestation.json"
        )
        document = build_attestation(
            artifact_path=artifact_path,
            repo=repo,
            target=args.target,
            features=args.features,
            no_default_features=args.no_default_features,
            build_command=args.build_command,
            profile=args.profile,
        )
        write_attestation(document, output)
        note = check_written(output, artifact_path)
    except AttestationError as error:
        print(f"FAIL  {error}", file=sys.stderr)
        return 1

    artifact = document["artifact"]
    print(
        f"PASS  {output}\n"
        f"      {artifact['name']}  {artifact['bytes']} bytes  "
        f"{artifact['sha256']}\n"
        f"      {artifact['target']}  {document['source']['commit'][:12]}  "
        f"{document['build']['rustc']}"
        + (f"  [{note}]" if note else "")
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
