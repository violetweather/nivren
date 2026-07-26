#!/usr/bin/env python3
"""Regression tests for the deterministic release archive builder."""

from __future__ import annotations

import hashlib
import tempfile
import unittest
import zipfile
from pathlib import Path

import package_release


class ReleasePackagerTests(unittest.TestCase):
    def test_archives_are_reproducible_complete_and_executable(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "niv"
            binary.write_bytes(b"test executable\n")
            first = root / "first.zip"
            second = root / "second.zip"

            package_release.package(binary, "0.10.0-beta.5", "linux-x64", first)
            package_release.package(binary, "0.10.0-beta.5", "linux-x64", second)

            self.assertEqual(
                hashlib.sha256(first.read_bytes()).digest(),
                hashlib.sha256(second.read_bytes()).digest(),
            )
            with zipfile.ZipFile(first) as archive:
                self.assertIsNone(archive.testzip())
                names = archive.namelist()
                self.assertEqual(names, sorted(names, key=lambda name: name.encode("utf-8")))
                executable = archive.getinfo(
                    "nivren-0.10.0-beta.5-linux-x64/bin/niv"
                )
                self.assertEqual((executable.external_attr >> 16) & 0o777, 0o755)
                installer = archive.getinfo(
                    "nivren-0.10.0-beta.5-linux-x64/install/install.sh"
                )
                self.assertEqual((installer.external_attr >> 16) & 0o777, 0o755)
                for document in package_release.DOCUMENTS:
                    self.assertIn(
                        f"nivren-0.10.0-beta.5-linux-x64/{document}", names
                    )
                self.assertIn(
                    "nivren-0.10.0-beta.5-linux-x64/licenses/INDEX.txt", names
                )
                self.assertTrue(
                    any("/licenses/rustls-0.23.42/" in name for name in names)
                )

    def test_rejects_unsafe_labels_and_symlinked_inputs(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "niv"
            binary.write_bytes(b"binary")
            with self.assertRaises(SystemExit):
                package_release.package(binary, "../escape", "linux-x64", root / "x.zip")

            link = root / "linked-niv"
            try:
                link.symlink_to(binary)
            except (NotImplementedError, OSError):
                self.skipTest("symlinks are unavailable")
            with self.assertRaises(SystemExit):
                package_release.package(link, "0.9.0", "linux-x64", root / "x.zip")


if __name__ == "__main__":
    unittest.main()
