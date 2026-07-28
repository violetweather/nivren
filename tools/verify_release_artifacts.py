#!/usr/bin/env python3
"""Fail-closed verification for a complete Nivren release asset directory."""

from __future__ import annotations

import hashlib
import json
import re
import stat
import sys
import zipfile
from pathlib import Path, PurePosixPath
from typing import NoReturn


PLATFORMS = (
    "linux-arm64",
    "linux-x64",
    "macos-arm64",
    "macos-x64",
    "windows-arm64",
    "windows-x64",
)
SAFE_LABEL = re.compile(r"^v\d+\.\d+\.\d+(?:-[A-Za-z0-9.]+)?$")
SAFE_ASSET = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
MAX_ASSET = 256 * 1024 * 1024
REQUIRED_ARCHIVE_PATHS = (
    "SBOM.spdx.json",
    "Cargo.lock",
    "SECURITY.md",
    "crates/nivren-ffi/include/nivren.h",
    "docs/LANGUAGE.md",
    "docs/STANDARD_LIBRARY.md",
    "docs/EDITION_4_PRODUCT_PROOF_AUDIT.md",
    "docs/SECURITY_AUDIT_SCOPE.md",
    "spec/LANGUAGE-4-DRAFT.md",
    "spec/BYTECODE-7.md",
)


def fail(message: str) -> NoReturn:
    raise SystemExit(message)


def expected_assets(label: str) -> set[str]:
    version = label.removeprefix("v")
    return {
        *(f"nivren-{label}-{platform}.zip" for platform in PLATFORMS),
        f"nivren-{label}-wasm32-wasip1.wasm",
        f"nivren-{label}-browser.wasm",
        f"nivren-{label}-browser.mjs",
        f"nivren-{version}.vsix",
        "SHA256SUMS",
    }


def digest(path: Path) -> str:
    result = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            result.update(chunk)
    return result.hexdigest()


def verify_checksums(directory: Path, assets: set[str]) -> None:
    lines = (directory / "SHA256SUMS").read_text(encoding="ascii").splitlines()
    records: dict[str, str] = {}
    order: list[str] = []
    for line in lines:
        match = re.fullmatch(r"([0-9a-f]{64})  ([A-Za-z0-9][A-Za-z0-9._-]*)", line)
        if not match or match[2] in records:
            fail("SHA256SUMS contains a malformed or duplicate record")
        records[match[2]] = match[1]
        order.append(match[2])
    expected = assets - {"SHA256SUMS"}
    if set(records) != expected or order != sorted(order, key=lambda value: value.encode()):
        fail("SHA256SUMS does not name the exact sorted release asset set")
    for name, expected_digest in records.items():
        if digest(directory / name) != expected_digest:
            fail(f"release checksum mismatch: {name}")


def verify_archive(path: Path, label: str, platform: str) -> None:
    prefix = f"nivren-{label}-{platform}/"
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)) or names != sorted(names, key=lambda value: value.encode()):
            fail(f"archive members are duplicate or unsorted: {path.name}")
        if archive.testzip() is not None:
            fail(f"archive CRC failed: {path.name}")
        for info in archive.infolist():
            pure = PurePosixPath(info.filename)
            mode = info.external_attr >> 16
            if (
                not info.filename.startswith(prefix)
                or info.is_dir()
                or pure.is_absolute()
                or ".." in pure.parts
                or "\\" in info.filename
                or stat.S_ISLNK(mode)
                or info.file_size > MAX_ASSET
            ):
                fail(f"unsafe archive member in {path.name}: {info.filename}")
        required = {prefix + relative for relative in REQUIRED_ARCHIVE_PATHS}
        executable = prefix + ("bin/niv.exe" if platform.startswith("windows-") else "bin/niv")
        if not required.issubset(names) or executable not in names:
            fail(f"archive is missing a required release member: {path.name}")
        sbom = json.loads(archive.read(prefix + "SBOM.spdx.json"))
        if sbom.get("spdxVersion") != "SPDX-2.3" or not sbom.get("packages"):
            fail(f"archive SBOM is invalid: {path.name}")


def verify(directory: Path, label: str) -> None:
    if not SAFE_LABEL.fullmatch(label):
        fail("release label is invalid")
    if directory.is_symlink() or not directory.is_dir():
        fail("release directory must be a regular directory")
    expected = expected_assets(label)
    actual = {path.name for path in directory.iterdir()}
    if actual != expected:
        fail(f"release directory has missing or unexpected assets: {sorted(actual ^ expected)}")
    for name in actual:
        path = directory / name
        if not SAFE_ASSET.fullmatch(name) or path.is_symlink() or not path.is_file():
            fail(f"unsafe release asset: {name}")
        if path.stat().st_size == 0 or path.stat().st_size > MAX_ASSET:
            fail(f"release asset size is invalid: {name}")
    verify_checksums(directory, expected)
    for platform in PLATFORMS:
        verify_archive(directory / f"nivren-{label}-{platform}.zip", label, platform)
    for suffix in ("wasm32-wasip1.wasm", "browser.wasm"):
        if not (directory / f"nivren-{label}-{suffix}").read_bytes().startswith(b"\0asm"):
            fail(f"invalid WebAssembly artifact: {suffix}")
    browser = (directory / f"nivren-{label}-browser.mjs").read_text(encoding="utf-8")
    if "NivrenBrowser" not in browser:
        fail("browser SDK does not expose NivrenBrowser")
    version = label.removeprefix("v")
    with zipfile.ZipFile(directory / f"nivren-{version}.vsix") as extension:
        if extension.testzip() is not None or not extension.namelist():
            fail("VS Code extension archive is invalid")


def main(arguments: list[str]) -> None:
    if len(arguments) != 3:
        fail("usage: verify_release_artifacts.py RELEASE_DIRECTORY vVERSION")
    verify(Path(arguments[1]).resolve(), arguments[2])
    print(f"verified complete Nivren release {arguments[2]}")


if __name__ == "__main__":
    main(sys.argv)
