"""Contract tests for the release-set command's fail-closed behavior."""
from __future__ import annotations

import contextlib
import hashlib
import importlib.util
import io
import json
import pathlib
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = pathlib.Path(__file__).resolve().parents[1] / "scripts" / "verify_release_set.py"
HEAD = "a" * 40
VERSION = "0.6.2"

SPEC = importlib.util.spec_from_file_location("verify_release_set", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
VERIFY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VERIFY)


class ReleaseSetTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temp.name)
        (self.root / "dist").mkdir()
        (self.root / "scripts").mkdir()
        (self.root / "scripts" / "generate_attestation.py").write_text("# mocked boundary\n")
        (self.root / "justfile").write_text(
            "release_targets := 'target-a target-b'\n", encoding="utf-8"
        )
        self.tag_exists = True
        self.status = ""
        self.tagged_commit = HEAD
        self._add_target("target-a")
        self._add_target("target-b")
        self._write_release_notes()
        self._write_checksums()

    def tearDown(self) -> None:
        self.temp.cleanup()

    def _add_target(self, target: str, *, commit: str = HEAD,
                    tree_digest: str = "c" * 64) -> None:
        stage = self.root / "dist" / f"v{VERSION}-{target}"
        stage.mkdir(parents=True, exist_ok=True)
        asset = stage / f"opencodifier-{VERSION}-{target}"
        content = f"fixture binary for {target}".encode()
        asset.write_bytes(content)
        attestation = {
            "artifact": {
                "target": target,
                "sha256": hashlib.sha256(content).hexdigest(),
            },
            "source": {
                "commit": commit,
                "working_tree_clean": True,
                "dirty_path_count": 0,
            },
            "build": {
                "cargo_lock_sha256": "b" * 64,
                "cargo_tree_sha256": tree_digest,
                "rustc": "rustc 1.99.1",
                "cargo": "cargo 1.99.1",
            },
        }
        (stage / f"{asset.name}.attestation.json").write_text(
            json.dumps(attestation), encoding="utf-8"
        )

    def _write_release_notes(self, text: str | None = None) -> None:
        notes = text or f"# OpenCodifier v{VERSION}\nSource commit: {HEAD}\n"
        (self.root / "dist" / "RELEASE_NOTES.md").write_text(notes, encoding="utf-8")

    def _write_checksums(self, *, omit: str | None = None) -> None:
        rows = []
        for asset in sorted((self.root / "dist").glob(f"v{VERSION}-*/*")):
            if asset.name.endswith(".attestation.json") or asset.name == omit:
                continue
            rows.append(f"{hashlib.sha256(asset.read_bytes()).hexdigest()}  {asset.name}")
        (self.root / "dist" / "sha256sums.txt").write_text(
            "\n".join(rows) + "\n", encoding="utf-8"
        )

    def _run_command(self, *args: str) -> str:
        if args[:2] == ("cargo", "metadata"):
            return json.dumps({"packages": [{"name": "opencodifier-cli", "version": VERSION}]})
        if args[:2] == ("git", "rev-parse"):
            if args[2] == "HEAD":
                return HEAD
            if args[2] == f"v{VERSION}^{{commit}}":
                return self.tagged_commit
        if args[:2] == ("git", "status"):
            return self.status
        raise AssertionError(f"unexpected command: {args!r}")

    def _run_process(self, args: list[str], **_: object) -> subprocess.CompletedProcess[str]:
        if args[:3] == ["git", "show-ref", "--verify"]:
            return subprocess.CompletedProcess(args, 0 if self.tag_exists else 1)
        if args[1:3] == [str(self.root / "scripts" / "generate_attestation.py"), "--check"]:
            return subprocess.CompletedProcess(args, 0, stdout="", stderr="")
        raise AssertionError(f"unexpected process: {args!r}")

    def _verify(self) -> tuple[int, str, str]:
        out, err = io.StringIO(), io.StringIO()
        with patch.object(VERIFY, "ROOT", self.root), \
             patch.object(VERIFY, "run", side_effect=self._run_command), \
             patch.object(VERIFY.subprocess, "run", side_effect=self._run_process), \
             contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            result = VERIFY.main()
        return result, out.getvalue(), err.getvalue()

    def test_complete_clean_release_set_passes(self) -> None:
        result, out, err = self._verify()
        self.assertEqual(result, 0, err)
        self.assertIn("2 targets", out)

    def test_missing_tag_fails_closed(self) -> None:
        self.tag_exists = False
        result, _, err = self._verify()
        self.assertEqual(result, 1)
        self.assertIn("release tag v0.6.2 does not exist", err)

    def test_tag_must_point_at_head(self) -> None:
        self.tagged_commit = "d" * 40
        result, _, err = self._verify()
        self.assertEqual(result, 1)
        self.assertIn("not HEAD", err)

    def test_dirty_checkout_fails_closed(self) -> None:
        self.status = " M Cargo.toml"
        result, _, err = self._verify()
        self.assertEqual(result, 1)
        self.assertIn("working tree is dirty", err)

    def test_missing_target_fails_closed(self) -> None:
        shutil.rmtree(self.root / "dist" / f"v{VERSION}-target-b")
        result, _, err = self._verify()
        self.assertEqual(result, 1)
        self.assertIn("incomplete release target matrix", err)

    def test_target_attestation_must_match_release_commit(self) -> None:
        stage = self.root / "dist" / f"v{VERSION}-target-a"
        path = next(stage.glob("*.attestation.json"))
        document = json.loads(path.read_text(encoding="utf-8"))
        document["source"]["commit"] = "d" * 40
        path.write_text(json.dumps(document), encoding="utf-8")
        result, _, err = self._verify()
        self.assertEqual(result, 1)
        self.assertIn("source commit does not match release HEAD", err)

    def test_target_toolchain_and_dependency_tree_must_match(self) -> None:
        self._add_target("target-b", tree_digest="e" * 64)
        result, _, err = self._verify()
        self.assertEqual(result, 1)
        self.assertIn("build toolchain/dependency provenance differs", err)

    def test_checksum_manifest_must_cover_exact_assets(self) -> None:
        self._write_checksums(omit="opencodifier-0.6.2-target-b")
        result, _, err = self._verify()
        self.assertEqual(result, 1)
        self.assertIn("sha256sums.txt entries do not exactly match", err)

    def test_release_notes_must_name_commit_and_have_no_placeholders(self) -> None:
        self._write_release_notes(f"# OpenCodifier v{VERSION}\n{HEAD}\n<receipt>\n")
        result, _, err = self._verify()
        self.assertEqual(result, 1)
        self.assertIn("release notes contain unfilled evidence fields", err)


if __name__ == "__main__":
    unittest.main()
