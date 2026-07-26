#!/usr/bin/env python3
"""Build a deterministic, self-contained Nivren release archive."""

from __future__ import annotations

import os
import re
import stat
import sys
import tempfile
import zipfile
from pathlib import Path
from typing import NoReturn


REPOSITORY = Path(__file__).resolve().parent.parent
DOCUMENTS = (
    "LICENSE",
    "README.md",
    "CHANGELOG.md",
    "SECURITY.md",
    "THIRD_PARTY.md",
    "Cargo.lock",
    "docs/GETTING_STARTED.md",
    "spec/LANGUAGE-2.md",
    "spec/BYTECODE-1.md",
    "spec/PACKAGE-1.md",
    "spec/STANDARD-LIBRARY-2.md",
)
SAFE_COMPONENT = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")
ZIP_EPOCH = (1980, 1, 1, 0, 0, 0)


def fail(message: str) -> NoReturn:
    raise SystemExit(message)


def checked_file(path: Path, description: str) -> Path:
    if path.is_symlink() or not path.is_file():
        fail(f"{description} must be a regular, non-symlink file: {path}")
    return path


def entry(name: str, mode: int) -> zipfile.ZipInfo:
    info = zipfile.ZipInfo(name, ZIP_EPOCH)
    info.create_system = 3
    info.compress_type = zipfile.ZIP_STORED
    info.external_attr = (stat.S_IFREG | mode) << 16
    return info


def dependency_licenses(prefix: str, binary: Path) -> list[tuple[str, bytes, int]]:
    members: list[tuple[str, bytes, int]] = []
    index = [
        "Nivren locked third-party dependency license inventory",
        "Generated from Cargo.lock and the exact package sources used for this build.",
        "",
    ]
    target = os.environ.get("CARGO_TARGET_DIR")
    if target:
        target_root = Path(target)
        if not target_root.is_absolute():
            target_root = REPOSITORY / target_root
    elif binary.parent.name == "release":
        target_root = binary.parent.parent
    else:
        target_root = REPOSITORY / "target"
    dependency_dir = target_root / "release" / "deps"
    records = sorted(dependency_dir.glob("*.d"), key=lambda path: path.name.encode())
    if not records:
        fail(f"release dependency records are missing: {dependency_dir}")
    dependency_paths = "\n".join(
        record.read_text(encoding="utf-8", errors="replace").replace("\\", "/")
        for record in records
    )

    cargo_home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
    registry_roots = sorted((cargo_home / "registry" / "src").glob("*"))
    sources = {
        package.resolve()
        for registry in registry_roots
        for package in registry.iterdir()
        if package.is_dir() and f"/{package.name}/" in dependency_paths
    }
    if not sources:
        fail("no third-party package sources matched the release dependency records")

    packages: list[tuple[str, str, str, Path, str | None]] = []
    for directory in sources:
        manifest = checked_file(directory / "Cargo.toml", "dependency manifest").read_text(
            encoding="utf-8"
        )
        fields = {}
        for field in ("name", "version", "license", "license-file"):
            match = re.search(rf'^\s*{re.escape(field)}\s*=\s*"([^"]+)"', manifest, re.MULTILINE)
            if match:
                fields[field] = match.group(1)
        if "name" not in fields or "version" not in fields:
            fail(f"dependency manifest lacks package identity: {directory}")
        packages.append(
            (
                fields["name"],
                fields["version"],
                fields.get("license", "license-file"),
                directory,
                fields.get("license-file"),
            )
        )

    for name, version, license_expression, directory, license_file in sorted(
        packages, key=lambda package: (package[0].encode(), package[1].encode())
    ):
        if not SAFE_COMPONENT.fullmatch(name) or not SAFE_COMPONENT.fullmatch(version):
            fail(f"unsafe dependency identity: {name!r} {version!r}")
        candidates = {
            child.resolve()
            for child in directory.iterdir()
            if child.name.upper().startswith(("LICENSE", "COPYING", "NOTICE"))
        }
        if license_file:
            candidates.add((directory / license_file).resolve())
        if not candidates:
            index.append(
                f"{name} {version} | {license_expression} | package supplied no license file"
            )
            continue
        index.append(f"{name} {version} | {license_expression}")
        for source in sorted(candidates, key=lambda path: path.name.encode("utf-8")):
            checked_file(source, "dependency license")
            members.append(
                (f"{prefix}/licenses/{name}-{version}/{source.name}", source.read_bytes(), 0o644)
            )
    index.append("")
    members.append(
        (f"{prefix}/licenses/INDEX.txt", "\n".join(index).encode("utf-8"), 0o644)
    )
    return members


def package(binary: Path, label: str, platform: str, output: Path) -> None:
    for kind, value in (("release label", label), ("platform", platform)):
        if not SAFE_COMPONENT.fullmatch(value):
            fail(f"unsafe {kind}: {value!r}")

    checked_file(binary, "release binary")
    prefix = f"nivren-{label}-{platform}"
    executable = "niv.exe" if platform.startswith("windows-") else "niv"
    members: list[tuple[str, bytes, int]] = [
        (f"{prefix}/bin/{executable}", binary.read_bytes(), 0o755)
    ]
    for relative in DOCUMENTS:
        source = checked_file(REPOSITORY / relative, "release document")
        members.append((f"{prefix}/{relative}", source.read_bytes(), 0o644))
    members.extend(dependency_licenses(prefix, binary))
    members.sort(key=lambda item: item[0].encode("utf-8"))

    output.parent.mkdir(parents=True, exist_ok=True)
    temporary: Path | None = None
    try:
        with tempfile.NamedTemporaryFile(
            prefix=f".{output.name}.", suffix=".tmp", dir=output.parent, delete=False
        ) as handle:
            temporary = Path(handle.name)
        with zipfile.ZipFile(temporary, "w", allowZip64=True) as archive:
            for name, contents, mode in members:
                archive.writestr(entry(name, mode), contents)
        with zipfile.ZipFile(temporary, "r") as archive:
            expected = [name for name, _, _ in members]
            if archive.namelist() != expected or archive.testzip() is not None:
                fail("release archive failed its integrity check")
        os.replace(temporary, output)
        temporary = None
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def main(arguments: list[str]) -> None:
    if len(arguments) != 5:
        fail("usage: package_release.py BINARY LABEL PLATFORM OUTPUT.zip")
    _, binary, label, platform, output = arguments
    package(Path(binary).resolve(), label, platform, Path(output).resolve())


if __name__ == "__main__":
    main(sys.argv)
