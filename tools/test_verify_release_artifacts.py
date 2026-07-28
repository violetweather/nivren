#!/usr/bin/env python3
"""Tests for complete release artifact verification."""

from __future__ import annotations

import hashlib
import json
import stat
import tempfile
import unittest
import zipfile
from pathlib import Path

import verify_release_artifacts


LABEL = "v0.10.0-beta.7"


def write_archive(path: Path, platform: str) -> None:
    prefix = f"nivren-{LABEL}-{platform}/"
    executable = "bin/niv.exe" if platform.startswith("windows-") else "bin/niv"
    members = {
        prefix + executable: b"executable",
        **{prefix + name: b"document\n" for name in verify_release_artifacts.REQUIRED_ARCHIVE_PATHS},
    }
    members[prefix + "SBOM.spdx.json"] = json.dumps(
        {"spdxVersion": "SPDX-2.3", "packages": [{"name": "nivren"}]}
    ).encode()
    with zipfile.ZipFile(path, "w") as archive:
        for name, contents in sorted(members.items(), key=lambda item: item[0].encode()):
            info = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0))
            info.create_system = 3
            info.external_attr = (stat.S_IFREG | (0o755 if name.endswith(("/niv", "/niv.exe")) else 0o644)) << 16
            archive.writestr(info, contents)


def fixture(directory: Path) -> None:
    for platform in verify_release_artifacts.PLATFORMS:
        write_archive(directory / f"nivren-{LABEL}-{platform}.zip", platform)
    (directory / f"nivren-{LABEL}-wasm32-wasip1.wasm").write_bytes(b"\0asm\x01\0\0\0")
    (directory / f"nivren-{LABEL}-browser.wasm").write_bytes(b"\0asm\x01\0\0\0")
    (directory / f"nivren-{LABEL}-browser.mjs").write_text("export class NivrenBrowser {}\n")
    with zipfile.ZipFile(directory / "nivren-0.10.0-beta.7.vsix", "w") as extension:
        extension.writestr("extension/package.json", "{}")
    assets = sorted(
        verify_release_artifacts.expected_assets(LABEL) - {"SHA256SUMS"},
        key=lambda value: value.encode(),
    )
    sums = "".join(
        f"{hashlib.sha256((directory / name).read_bytes()).hexdigest()}  {name}\n"
        for name in assets
    )
    (directory / "SHA256SUMS").write_text(sums, encoding="ascii")


class ReleaseArtifactVerifierTests(unittest.TestCase):
    def test_complete_release_passes_and_tampering_fails(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            fixture(directory)
            verify_release_artifacts.verify(directory, LABEL)
            (directory / f"nivren-{LABEL}-browser.wasm").write_bytes(b"tampered")
            with self.assertRaisesRegex(SystemExit, "checksum mismatch"):
                verify_release_artifacts.verify(directory, LABEL)

    def test_extra_asset_fails(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            fixture(directory)
            (directory / "unexpected.txt").write_text("no")
            with self.assertRaisesRegex(SystemExit, "missing or unexpected"):
                verify_release_artifacts.verify(directory, LABEL)


if __name__ == "__main__":
    unittest.main()
