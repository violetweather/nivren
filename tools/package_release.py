#!/usr/bin/env python3
"""Build a deterministic, self-contained Nivren release archive."""

from __future__ import annotations

import os
import re
import stat
import sys
import tempfile
import json
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
    "docs/LANGUAGE.md",
    "docs/STANDARD_LIBRARY.md",
    "docs/BYTECODE.md",
    "docs/CAPABILITY_AUDIT.md",
    "docs/EDITION_4_CHECKPOINTS.md",
    "docs/EDITION_4_COMPILER_PROOF_AUDIT.md",
    "docs/EDITION_4_PRODUCT_PROOF_AUDIT.md",
    "docs/PACKAGES.md",
    "docs/PERFORMANCE.md",
    "docs/REGISTRY_OPERATIONS.md",
    "docs/REGISTRY_SECURITY.md",
    "docs/RELEASES.md",
    "docs/SECURITY_AUDIT_SCOPE.md",
    "docs/TESTING.md",
    "docs/WASM.md",
    "crates/nivren-database-host/README.md",
    "install/install.sh",
    "install/install.ps1",
    "install/README.md",
    "sdk/mobile/README.md",
    "sdk/mobile/ios/CNivren/module.modulemap",
    "sdk/mobile/ios/NivrenMobile.swift",
    "sdk/mobile/android/NivrenMobile.kt",
    "sdk/mobile/android/nivren_mobile_jni.c",
    "spec/LANGUAGE-2.md",
    "spec/BYTECODE-1.md",
    "spec/PACKAGE-1.md",
    "spec/STANDARD-LIBRARY-2.md",
    "spec/LANGUAGE-3.md",
    "spec/BYTECODE-2.md",
    "spec/STANDARD-LIBRARY-3.md",
    "spec/LANGUAGE-4-DRAFT.md",
    "spec/BYTECODE-7.md",
    "spec/WASM-1.md",
    "crates/nivren-ffi/include/nivren.h",
)
SAFE_COMPONENT = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")
# Crate versions may carry semver build metadata after "+" (for example ash
# 0.38.0+1.3.281); "+" is safe in archive member names on every platform.
SAFE_VERSION = re.compile(r"^[A-Za-z0-9][A-Za-z0-9.+_-]*$")
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
        if not SAFE_COMPONENT.fullmatch(name) or not SAFE_VERSION.fullmatch(version):
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


def spdx_sbom() -> bytes:
    lock = (REPOSITORY / "Cargo.lock").read_text(encoding="utf-8")
    locked_packages = []
    for block in lock.split("[[package]]")[1:]:
        fields = {
            match.group(1): match.group(2)
            for match in re.finditer(r'^(name|version|checksum) = "([^"]+)"', block, re.MULTILINE)
        }
        if "name" in fields and "version" in fields:
            locked_packages.append(fields)
    packages = []
    for item in sorted(locked_packages, key=lambda value: (value["name"], value["version"])):
        package = {
            "SPDXID": f"SPDXRef-Package-{item['name']}-{item['version']}".replace("+", "-"),
            "name": item["name"],
            "versionInfo": item["version"],
            "downloadLocation": "NOASSERTION",
            "filesAnalyzed": False,
            "licenseConcluded": "NOASSERTION",
            "licenseDeclared": "NOASSERTION",
            "copyrightText": "NOASSERTION",
        }
        if checksum := item.get("checksum"):
            package["checksums"] = [{"algorithm": "SHA256", "checksumValue": checksum}]
        packages.append(package)
    document = {
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": "Nivren release dependency SBOM",
        "documentNamespace": "https://nivren.dev/sbom/release",
        "creationInfo": {"created": "1980-01-01T00:00:00Z", "creators": ["Tool: Nivren-release-packager"]},
        "packages": packages,
    }
    return (json.dumps(document, sort_keys=True, separators=(",", ":")) + "\n").encode()


def package(
    binary: Path,
    label: str,
    platform: str,
    output: Path,
    library_dir: Path | None = None,
) -> None:
    for kind, value in (("release label", label), ("platform", platform)):
        if not SAFE_COMPONENT.fullmatch(value):
            fail(f"unsafe {kind}: {value!r}")

    checked_file(binary, "release binary")
    prefix = f"nivren-{label}-{platform}"
    executable = "niv.exe" if platform.startswith("windows-") else "niv"
    members: list[tuple[str, bytes, int]] = [
        (f"{prefix}/bin/{executable}", binary.read_bytes(), 0o755),
        (f"{prefix}/SBOM.spdx.json", spdx_sbom(), 0o644),
    ]
    if library_dir is not None:
        if not library_dir.is_dir() or library_dir.is_symlink():
            fail(f"native library directory is invalid: {library_dir}")
        libraries = sorted(
            (
                path
                for path in library_dir.iterdir()
                if "nivren_ffi" in path.name
                and path.suffix.lower() in {".a", ".dll", ".dylib", ".lib", ".so"}
            ),
            key=lambda path: path.name.encode(),
        )
        if not libraries:
            fail(f"native Nivren libraries are missing: {library_dir}")
        for library in libraries:
            checked_file(library, "native Nivren library")
            members.append((f"{prefix}/lib/{library.name}", library.read_bytes(), 0o644))
    for relative in DOCUMENTS:
        source = checked_file(REPOSITORY / relative, "release document")
        mode = 0o755 if relative == "install/install.sh" else 0o644
        members.append((f"{prefix}/{relative}", source.read_bytes(), mode))
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
    if len(arguments) not in {5, 6}:
        fail("usage: package_release.py BINARY LABEL PLATFORM OUTPUT.zip [LIBRARY_DIR]")
    _, binary, label, platform, output, *library_dir = arguments
    package(
        Path(binary).resolve(),
        label,
        platform,
        Path(output).resolve(),
        Path(library_dir[0]).resolve() if library_dir else None,
    )


if __name__ == "__main__":
    main(sys.argv)
