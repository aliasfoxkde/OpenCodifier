#!/usr/bin/env python3
"""Fail closed unless a staged release is complete, clean, and single-source."""
from __future__ import annotations

import json
import hashlib
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]


def run(*args: str) -> str:
    proc = subprocess.run(args, cwd=ROOT, capture_output=True, text=True, check=False)
    if proc.returncode:
        raise RuntimeError(proc.stderr.strip() or proc.stdout.strip())
    return proc.stdout.strip()


def main() -> int:
    try:
        metadata = json.loads(run("cargo", "metadata", "--no-deps", "--format-version", "1"))
        version = next(p["version"] for p in metadata["packages"] if p["name"] == "opencodifier-cli")
        tag = f"v{version}"
        head = run("git", "rev-parse", "HEAD")
        tag_ref = subprocess.run(
            ["git", "show-ref", "--verify", "--quiet", f"refs/tags/{tag}"],
            cwd=ROOT, check=False,
        )
        if tag_ref.returncode:
            raise RuntimeError(f"release tag {tag} does not exist")
        tagged = run("git", "rev-parse", f"{tag}^{{commit}}")
        if head != tagged:
            raise RuntimeError(f"tag {tag} points to {tagged}, not HEAD {head}")
        if run("git", "status", "--porcelain", "--untracked-files=all"):
            raise RuntimeError("working tree is dirty; release artifacts must be built from a clean checkout")

        justfile = (ROOT / "justfile").read_text(encoding="utf-8")
        target_match = re.search(r"^release_targets\s*:=\s*'([^']+)'", justfile, re.MULTILINE)
        if not target_match:
            raise RuntimeError("cannot read release_targets from justfile")
        expected_targets = set(target_match.group(1).split())
        stage_dirs = sorted((ROOT / "dist").glob(f"v{version}-*"))
        found: dict[str, tuple[pathlib.Path, pathlib.Path, dict[str, object]]] = {}
        for stage in stage_dirs:
            for attestation_path in stage.glob("*.attestation.json"):
                binary_path = pathlib.Path(str(attestation_path)[:-len(".attestation.json")])
                check = subprocess.run(
                    [sys.executable, str(ROOT / "scripts" / "generate_attestation.py"),
                     "--check", str(attestation_path), str(binary_path)],
                    cwd=ROOT, capture_output=True, text=True, check=False,
                )
                if check.returncode:
                    raise RuntimeError(check.stderr.strip() or check.stdout.strip())
                document = json.loads(attestation_path.read_text(encoding="utf-8"))
                artifact = document["artifact"]
                source = document["source"]
                target = artifact["target"]
                if target in found:
                    raise RuntimeError(f"duplicate attestation for target {target}")
                if source.get("commit") != head:
                    raise RuntimeError(f"{target}: source commit does not match release HEAD")
                if source.get("working_tree_clean") is not True or source.get("dirty_path_count") != 0:
                    raise RuntimeError(f"{target}: source attestation reports a dirty tree")
                if stage.name != f"v{version}-{target}":
                    raise RuntimeError(f"{target}: artifact is staged in unexpected directory {stage.name}")
                found[target] = (binary_path, attestation_path, document)

        missing = expected_targets - set(found)
        extra = set(found) - expected_targets
        if missing or extra:
            raise RuntimeError(f"incomplete release target matrix; missing={sorted(missing)} extra={sorted(extra)}")

        baseline: tuple[object, ...] | None = None
        for target, (_, _, document) in sorted(found.items()):
            build = document["build"]
            current = (build.get("cargo_lock_sha256"), build.get("cargo_tree_sha256"),
                       build.get("rustc"), build.get("cargo"))
            if baseline is None:
                baseline = current
            elif current != baseline:
                raise RuntimeError(f"{target}: build toolchain/dependency provenance differs from other targets")
        notes = ROOT / "dist" / "RELEASE_NOTES.md"
        checksums = ROOT / "dist" / "sha256sums.txt"
        if not notes.is_file() or not checksums.is_file():
            raise RuntimeError("generate release notes and checksum manifest before publishing")
        checksum_rows: dict[str, str] = {}
        for line in checksums.read_text(encoding="utf-8").splitlines():
            parts = line.split(maxsplit=1)
            if len(parts) != 2 or not re.fullmatch(r"[0-9a-f]{64}", parts[0]):
                raise RuntimeError(f"malformed checksum row: {line!r}")
            name = parts[1].lstrip("* ")
            if name in checksum_rows:
                raise RuntimeError(f"duplicate checksum entry: {name}")
            checksum_rows[name] = parts[0]
        expected_checksums: dict[str, str] = {}
        for binary_path, _, document in found.values():
            digest = hashlib.sha256(binary_path.read_bytes()).hexdigest()
            if digest != document["artifact"]["sha256"]:
                raise RuntimeError(f"{binary_path.name}: checksum differs from its attestation")
            expected_checksums[binary_path.name] = digest
        if checksum_rows != expected_checksums:
            raise RuntimeError("sha256sums.txt entries do not exactly match the staged binaries")
        notes_text = notes.read_text(encoding="utf-8")
        if f"# OpenCodifier {tag}" not in notes_text or head not in notes_text:
            raise RuntimeError("release notes must name the exact tag and full commit")
        if "<filled-by-human>" in notes_text or "<receipt>" in notes_text or "<PASS|FAIL>" in notes_text:
            raise RuntimeError("release notes contain unfilled evidence fields")
        print(f"PASS  {tag}: {len(found)} targets; clean source {head}; unified lock/toolchain; notes and checksums present")
    except (OSError, StopIteration, KeyError, TypeError, json.JSONDecodeError, RuntimeError) as error:
        print(f"FAIL  {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
