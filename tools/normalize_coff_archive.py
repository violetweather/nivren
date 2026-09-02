#!/usr/bin/env python3
"""Canonicalize timestamps in a Windows COFF archive in place."""

from __future__ import annotations

import sys
from pathlib import Path


MAGIC = b"!<arch>\n"
HEADER_SIZE = 60
COFF_MACHINES = {0x014C, 0x01C0, 0x01C2, 0x01C4, 0x8664, 0xAA64}


def normalize(path: Path) -> None:
    data = bytearray(path.read_bytes())
    if not data.startswith(MAGIC):
        raise SystemExit(f"not a COFF archive: {path}")

    offset = len(MAGIC)
    while offset < len(data):
        end = offset + HEADER_SIZE
        if end > len(data) or data[offset + 58 : end] != b"`\n":
            raise SystemExit(f"invalid COFF archive member header: {path}")
        try:
            size = int(bytes(data[offset + 48 : offset + 58]).strip() or b"0")
        except ValueError as error:
            raise SystemExit(f"invalid COFF archive member size: {path}") from error
        if size < 0:
            raise SystemExit(f"negative COFF archive member size: {path}")

        data[offset + 16 : offset + 28] = b"0" + (b" " * 11)
        data[offset + 28 : offset + 34] = b"0" + (b" " * 5)
        data[offset + 34 : offset + 40] = b"0" + (b" " * 5)

        body = end
        body_end = body + size
        if body_end > len(data):
            raise SystemExit(f"truncated COFF archive member: {path}")
        member = data[body:body_end]
        if len(member) >= 12 and member[:4] == b"\x00\x00\xff\xff":
            data[body + 8 : body + 12] = b"\x00" * 4
        elif len(member) >= 20 and int.from_bytes(member[:2], "little") in COFF_MACHINES:
            data[body + 4 : body + 8] = b"\x00" * 4
        offset = body_end + (size & 1)

    if offset != len(data):
        raise SystemExit(f"invalid COFF archive padding: {path}")
    path.write_bytes(data)


def main(arguments: list[str]) -> int:
    if not arguments:
        raise SystemExit("usage: normalize_coff_archive.py ARCHIVE.lib [...]")
    for argument in arguments:
        normalize(Path(argument))
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
