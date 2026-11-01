"""Contract tests for verifying downloaded OpenCodifier release binaries."""
from __future__ import annotations

import hashlib
import json
import pathlib
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
VERIFIER = ROOT / "scripts" / "verify_prebuilt_attestation.py"
TARGET = "x86_64-unknown-linux-musl"
BINARY = b"fixture executable bytes"


def fixture() -> dict[str, object]:
    return {
        "schema_version": "opencodifier.attestation/1",
        "artifact": {
            "name": "opencodifier",
            "target": TARGET,
            "sha256": hashlib.sha256(BINARY).hexdigest(),
            "bytes": len(BINARY),
        },
        "source": {
            "commit": "a" * 40,
            "working_tree_clean": True,
            "dirty_path_count": 0,
        },
        "build": {"cargo_lock_sha256": "b" * 64},
    }


class PrebuiltAttestationTests(unittest.TestCase):
    def verify(self, document: dict[str, object], binary: bytes = BINARY,
               target: str = TARGET) -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            attestation = root / "asset.attestation.json"
            artifact = root / "asset"
            attestation.write_text(json.dumps(document), encoding="utf-8")
            artifact.write_bytes(binary)
            return subprocess.run(
                ["python3", str(VERIFIER), str(attestation), str(artifact), target],
                check=False, capture_output=True, text=True,
            )

    def test_accepts_matching_clean_artifact(self) -> None:
        result = self.verify(fixture())
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("PASS", result.stdout)

    def test_rejects_platform_mismatch(self) -> None:
        result = self.verify(fixture(), target="aarch64-unknown-linux-gnu")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("target mismatch", result.stderr)

    def test_rejects_digest_mismatch(self) -> None:
        result = self.verify(fixture(), binary=b"X" * len(BINARY))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("SHA-256 mismatch", result.stderr)

    def test_rejects_size_mismatch(self) -> None:
        document = fixture()
        document["artifact"]["bytes"] += 1  # type: ignore[index]
        result = self.verify(document)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("size mismatch", result.stderr)

    def test_rejects_dirty_source(self) -> None:
        document = fixture()
        document["source"]["working_tree_clean"] = False  # type: ignore[index]
        document["source"]["dirty_path_count"] = 1  # type: ignore[index]
        result = self.verify(document)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("clean source tree", result.stderr)

    def test_rejects_unknown_schema(self) -> None:
        document = fixture()
        document["schema_version"] = "opencodifier.attestation/99"
        result = self.verify(document)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unsupported attestation schema", result.stderr)


if __name__ == "__main__":
    unittest.main()
