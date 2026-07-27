#!/usr/bin/env python3
"""Regression test for deterministic COFF archive normalization."""

from __future__ import annotations

import tempfile
from pathlib import Path

from normalize_coff_archive import MAGIC, normalize


def member(name: bytes, timestamp: int, body: bytes) -> bytes:
    header = (
        name.ljust(16)
        + str(timestamp).encode().ljust(12)
        + b"7".ljust(6)
        + b"9".ljust(6)
        + b"100666".ljust(8)
        + str(len(body)).encode().ljust(10)
        + b"`\n"
    )
    return header + body + (b"\n" if len(body) & 1 else b"")


def archive(timestamp: int) -> bytes:
    coff = b"\x64\xaa\x01\x00" + timestamp.to_bytes(4, "little") + (b"\x00" * 12)
    imported = b"\x00\x00\xff\xff\x00\x00\x64\xaa" + timestamp.to_bytes(4, "little")
    return MAGIC + member(b"first.obj/", timestamp, coff) + member(b"import.obj/", timestamp, imported)


with tempfile.TemporaryDirectory() as directory:
    first = Path(directory) / "first.lib"
    second = Path(directory) / "second.lib"
    first.write_bytes(archive(123))
    second.write_bytes(archive(987))
    normalize(first)
    normalize(second)
    if first.read_bytes() != second.read_bytes():
        raise SystemExit("COFF archive normalization is not deterministic")

print("COFF archive normalization test passed")
