#!/usr/bin/env python3
"""Validate an OpenCodifier release binary against its adjacent JSON attestation."""

from __future__ import annotations

import hashlib
import json
import pathlib
import re
import sys


def fail(message: str) -> int:
    print(f"FAIL  {message}", file=sys.stderr)
    return 1


def main() -> int:
    if len(sys.argv) != 4:
        return fail("usage: verify_prebuilt_attestation.py ATTESTATION BINARY TARGET")
    attestation_path = pathlib.Path(sys.argv[1])
    binary_path = pathlib.Path(sys.argv[2])
    expected_target = sys.argv[3]
    try:
        document = json.loads(attestation_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        return fail(f"cannot read valid attestation: {exc}")
    if not isinstance(document, dict) or document.get("schema_version") != "opencodifier.attestation/1":
        return fail("unsupported attestation schema")

    artifact = document.get("artifact")
    source = document.get("source")
    build = document.get("build")
    if not all(isinstance(value, dict) for value in (artifact, source, build)):
        return fail("attestation is missing artifact, source, or build objects")
    assert isinstance(artifact, dict) and isinstance(source, dict) and isinstance(build, dict)

    if artifact.get("name") != "opencodifier":
        return fail("attestation does not describe the opencodifier CLI")
    if artifact.get("target") != expected_target:
        return fail(f"target mismatch: expected {expected_target}, got {artifact.get('target')!r}")
    digest = artifact.get("sha256")
    if not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest):
        return fail("attestation contains an invalid SHA-256 digest")
    if source.get("working_tree_clean") is not True or source.get("dirty_path_count") != 0:
        return fail("release was not built from a clean source tree")
    commit = source.get("commit")
    if not isinstance(commit, str) or not re.fullmatch(r"[0-9a-f]{40}", commit):
        return fail("attestation contains an invalid source commit")
    lock_digest = build.get("cargo_lock_sha256")
    if not isinstance(lock_digest, str) or not re.fullmatch(r"[0-9a-f]{64}", lock_digest):
        return fail("attestation contains an invalid Cargo.lock digest")
    if not binary_path.is_file():
        return fail(f"binary not found: {binary_path}")

    actual_size = binary_path.stat().st_size
    if artifact.get("bytes") != actual_size:
        return fail(f"size mismatch: attestation says {artifact.get('bytes')!r}, file is {actual_size}")
    hasher = hashlib.sha256()
    with binary_path.open("rb") as binary:
        for chunk in iter(lambda: binary.read(1024 * 1024), b""):
            hasher.update(chunk)
    actual_digest = hasher.hexdigest()
    if digest != actual_digest:
        return fail(f"SHA-256 mismatch: expected {digest}, got {actual_digest}")
    print(f"PASS  {binary_path.name}: target={expected_target} sha256={actual_digest} source={commit[:12]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
