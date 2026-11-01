"""Integration contract tests for the no-Rust prebuilt installer."""
from __future__ import annotations

import hashlib
import http.server
import json
import os
import pathlib
import socketserver
import subprocess
import tempfile
import threading
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
INSTALLER = ROOT / "scripts" / "install-prebuilt.sh"
VERSION = "9.8.7"
TARGET = "x86_64-unknown-linux-musl"
BINARY = (
    "#!/bin/sh\n"
    "[ \"${1:-}\" = --version ] && printf 'opencodifier %s\\n' '9.8.7'\n"
).encode()


def attestation(binary: bytes = BINARY) -> bytes:
    document = {
        "schema_version": "opencodifier.attestation/1",
        "artifact": {
            "name": "opencodifier",
            "target": TARGET,
            "sha256": hashlib.sha256(binary).hexdigest(),
            "bytes": len(binary),
        },
        "source": {
            "commit": "c" * 40,
            "working_tree_clean": True,
            "dirty_path_count": 0,
        },
        "build": {"cargo_lock_sha256": "d" * 64},
    }
    return json.dumps(document).encode()


class QuietHandler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, format: str, *args: object) -> None:
        pass


class InstallPrebuiltTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.temp = tempfile.TemporaryDirectory()
        cls.release_root = pathlib.Path(cls.temp.name)
        release_dir = cls.release_root / f"v{VERSION}"
        release_dir.mkdir()
        cls.asset_name = f"opencodifier-{VERSION}-{TARGET}"
        (release_dir / cls.asset_name).write_bytes(BINARY)
        (release_dir / f"{cls.asset_name}.attestation.json").write_bytes(attestation())

        handler = lambda *args, **kwargs: QuietHandler(  # noqa: E731
            *args, directory=str(cls.release_root), **kwargs
        )
        cls.server = socketserver.TCPServer(("127.0.0.1", 0), handler)
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()
        cls.base_url = f"http://127.0.0.1:{cls.server.server_address[1]}"

    @classmethod
    def tearDownClass(cls) -> None:
        cls.server.shutdown()
        cls.server.server_close()
        cls.thread.join(timeout=3)
        cls.temp.cleanup()

    def setUp(self) -> None:
        self.install_root = tempfile.TemporaryDirectory()
        self.install_dir = pathlib.Path(self.install_root.name) / "bin"

    def tearDown(self) -> None:
        self.install_root.cleanup()

    def run_installer(self, path: pathlib.Path | None = None,
                      version: str = f"v{VERSION}",
                      env_overrides: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        env["OPENCODIFIER_RELEASE_BASE_URL"] = self.base_url
        if env_overrides:
            env.update(env_overrides)
        args = ["sh", str(INSTALLER), "--version", version,
                "--install-dir", str(path or self.install_dir)]
        return subprocess.run(args, env=env, check=False, capture_output=True, text=True)

    def test_installs_verified_binary_without_cargo(self) -> None:
        env_path = "/usr/bin:/bin"
        result = self.run_installer(env_overrides={"PATH": env_path})
        self.assertEqual(result.returncode, 0, result.stderr)
        installed = self.install_dir / "opencodifier"
        self.assertEqual(installed.read_bytes(), BINARY)
        self.assertTrue(os.access(installed, os.X_OK))
        self.assertIn("No service was started", result.stdout)

    def test_bad_digest_preserves_existing_install(self) -> None:
        old_binary = b"existing usable binary"
        self.install_dir.mkdir()
        installed = self.install_dir / "opencodifier"
        installed.write_bytes(old_binary)
        asset = self.release_root / f"v{VERSION}" / self.asset_name
        asset.write_bytes(b"X" * len(BINARY))
        try:
            result = self.run_installer()
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("SHA-256 mismatch", result.stderr)
            self.assertEqual(installed.read_bytes(), old_binary)
        finally:
            asset.write_bytes(BINARY)

    def test_runtime_version_mismatch_does_not_install(self) -> None:
        asset = self.release_root / f"v{VERSION}" / self.asset_name
        wrong = BINARY.replace(b"9.8.7", b"9.8.6")
        asset.write_bytes(wrong)
        attestation_path = asset.with_name(f"{asset.name}.attestation.json")
        attestation_path.write_bytes(attestation(wrong))
        try:
            result = self.run_installer()
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Binary reports", result.stderr)
            self.assertFalse((self.install_dir / "opencodifier").exists())
        finally:
            asset.write_bytes(BINARY)
            attestation_path.write_bytes(attestation())

    def test_unsupported_host_fails_before_download_or_install(self) -> None:
        with tempfile.TemporaryDirectory() as fake_bin:
            fake_uname = pathlib.Path(fake_bin) / "uname"
            fake_uname.write_text("#!/bin/sh\n[ \"${1:-}\" = -s ] && echo Plan9 || echo strange\n")
            fake_uname.chmod(0o755)
            result = self.run_installer(env_overrides={
                "PATH": f"{fake_bin}:{os.environ['PATH']}"
            })
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("No first-party prebuilt CLI installer target", result.stderr)
        self.assertFalse((self.install_dir / "opencodifier").exists())

    def test_rejects_non_semver_release_before_download(self) -> None:
        result = self.run_installer(version="v9.8.7.1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("major.minor.patch", result.stderr)
        self.assertFalse((self.install_dir / "opencodifier").exists())


if __name__ == "__main__":
    unittest.main()
