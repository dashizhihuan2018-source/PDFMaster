#!/usr/bin/env python3
"""Fail-closed checks for exact candidate artifacts. Standard library only."""

import argparse
import hashlib
from pathlib import Path, PurePosixPath
import stat
import struct
import sys
import zipfile


def fail(message: str) -> "None":
    raise SystemExit(message)


def relative_path(value: str) -> PurePosixPath:
    if not value or "\\" in value or ":" in value:
        fail("artifact path must be a non-empty relative POSIX path")
    path = PurePosixPath(value)
    if path.is_absolute() or any(part in {"", ".", ".."} for part in path.parts):
        fail("artifact path must not escape its root")
    return path


def safe_file(root_value: str, relative_value: str) -> tuple[Path, PurePosixPath]:
    root = Path(root_value).resolve(strict=True)
    if not root.is_dir():
        fail("artifact root must be a directory")
    relative = relative_path(relative_value)
    cursor = root
    for part in relative.parts:
        cursor = cursor / part
        if cursor.is_symlink():
            fail("symlink artifact or path component is forbidden")
    resolved = cursor.resolve(strict=True)
    try:
        resolved.relative_to(root)
    except ValueError:
        fail("artifact escaped its root")
    if not resolved.is_file() or resolved.stat().st_size == 0:
        fail("artifact must be a non-empty regular file")
    return resolved, relative


def safe_output(root_value: str, relative_value: str) -> tuple[Path, PurePosixPath]:
    root = Path(root_value).resolve(strict=True)
    relative = relative_path(relative_value)
    cursor = root
    for part in relative.parts[:-1]:
        cursor = cursor / part
        if cursor.is_symlink():
            fail("symlink output path component is forbidden")
    cursor.mkdir(parents=True, exist_ok=True)
    output = cursor / relative.name
    if output.is_symlink():
        fail("symlink output is forbidden")
    try:
        output.resolve().relative_to(root)
    except ValueError:
        fail("output escaped its root")
    return output, relative


def validate_zip(root: str, artifact: str, expected_prefix: str) -> None:
    path, _ = safe_file(root, artifact)
    if not expected_prefix.endswith("/"):
        fail("ZIP prefix must end with a slash")
    prefix = relative_path(expected_prefix.removesuffix("/")).as_posix() + "/"
    regular_files = 0
    declared_bytes = 0
    seen = set()
    try:
        with zipfile.ZipFile(path) as archive:
            for info in archive.infolist():
                name = info.filename
                if "\\" in name or name.startswith("/"):
                    fail("ZIP contains a non-portable member path")
                member = PurePosixPath(name)
                normalized = member.as_posix().rstrip("/")
                if normalized != name.rstrip("/") or any(part in {"", ".", ".."} for part in member.parts):
                    fail("ZIP member escapes or aliases its root")
                if normalized in seen:
                    fail("ZIP contains duplicate member paths")
                seen.add(normalized)
                if not name.startswith(prefix):
                    fail("ZIP member is outside the exact expected package root")
                mode = (info.external_attr >> 16) & 0o170000
                if mode == stat.S_IFLNK:
                    fail("ZIP symlink member is forbidden")
                if not info.is_dir():
                    regular_files += 1
                    declared_bytes += info.file_size
                    if declared_bytes > 2 * 1024 * 1024 * 1024:
                        fail("ZIP declared content exceeds candidate budget")
            # Validate compressed data/CRC only after checking the total budget.
            corrupt_member = archive.testzip()
            if corrupt_member is not None:
                fail("ZIP member failed CRC validation")
    except (zipfile.BadZipFile, RuntimeError, NotImplementedError, EOFError) as error:
        fail(f"invalid ZIP: {error}")
    if regular_files == 0:
        fail("ZIP contains no regular files")


def validate_pe(root: str, artifact: str, machine: int, magic: int) -> tuple[Path, int]:
    path, _ = safe_file(root, artifact)
    size = path.stat().st_size
    with path.open("rb") as stream:
        header = stream.read(64)
        if len(header) != 64 or header[:2] != b"MZ":
            fail("candidate is not a PE executable")
        pe_offset = struct.unpack_from("<I", header, 0x3C)[0]
        if pe_offset < 64 or pe_offset > 16 * 1024 * 1024:
            fail("PE header offset exceeds validation budget")
        stream.seek(pe_offset)
        pe_header = stream.read(24)
        if len(pe_header) != 24 or pe_header[:4] != b"PE\0\0":
            fail("candidate has an invalid PE signature")
        actual_machine, sections = struct.unpack_from("<HH", pe_header, 4)
        optional_size, characteristics = struct.unpack_from("<HH", pe_header, 20)
        if actual_machine != machine or not characteristics & 2:
            fail("unexpected PE architecture or non-executable image")
        if not 1 <= sections <= 96 or optional_size < (112 if magic == 0x20B else 96):
            fail("invalid PE section count or optional header size")
        optional = stream.read(optional_size)
        if len(optional) != optional_size or struct.unpack_from("<H", optional)[0] != magic:
            fail("invalid PE optional header")
        end = pe_offset + 24 + optional_size + sections * 40
        table_end = end
        if end > size:
            fail("truncated PE section table")
        for _ in range(sections):
            section = stream.read(40)
            raw_size, raw_offset = struct.unpack_from("<II", section, 16)
            if raw_size:
                if raw_offset < table_end or raw_offset + raw_size > size:
                    fail("PE section data exceeds file bounds")
                end = max(end, raw_offset + raw_size)
    return path, end


def validate_pe_x64(root: str, artifact: str) -> None:
    validate_pe(root, artifact, 0x8664, 0x20B)


def validate_nsis_stub(root: str, artifact: str) -> None:
    # Tauri's pinned NSIS bundler uses a PE32 stub even for x64 payloads.
    # This checks the container, NOT installation or the embedded app's machine.
    path, image_end = validate_pe(root, artifact, 0x14C, 0x10B)
    size = path.stat().st_size
    offset = ((image_end + 511) // 512) * 512
    with path.open("rb") as stream:
        while offset + 28 <= min(size, 16 * 1024 * 1024):
            stream.seek(offset)
            flags, sig, a, b, c, header_length, following = struct.unpack("<7I", stream.read(28))
            if (sig, a, b, c) == (0xDEADBEEF, 0x6C6C754E, 0x74666F73, 0x74736E49):
                if flags & ~15 or flags & 1 or header_length == 0 or following <= 28 or following > size - offset:
                    fail("invalid NSIS first header bounds or installer flags")
                return
            offset += 512
    fail("NSIS installer first header not found within validation budget")


def write_hash(root: str, artifact: str, output: str) -> None:
    path, relative = safe_file(root, artifact)
    output_path, _ = safe_output(root, output)
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    output_path.write_text(
        f"{digest.hexdigest()}  {relative.name}\n", encoding="ascii", newline="\n"
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    subcommands = parser.add_subparsers(dest="command", required=True)

    zip_parser = subcommands.add_parser("zip")
    zip_parser.add_argument("root")
    zip_parser.add_argument("artifact")
    zip_parser.add_argument("expected_prefix")

    pe_parser = subcommands.add_parser("pe-x64")
    pe_parser.add_argument("root")
    pe_parser.add_argument("artifact")

    nsis_parser = subcommands.add_parser("nsis-stub")
    nsis_parser.add_argument("root")
    nsis_parser.add_argument("artifact")

    hash_parser = subcommands.add_parser("hash")
    hash_parser.add_argument("root")
    hash_parser.add_argument("artifact")
    hash_parser.add_argument("output")

    arguments = parser.parse_args()
    if arguments.command == "zip":
        validate_zip(arguments.root, arguments.artifact, arguments.expected_prefix)
    elif arguments.command == "pe-x64":
        validate_pe_x64(arguments.root, arguments.artifact)
    elif arguments.command == "nsis-stub":
        validate_nsis_stub(arguments.root, arguments.artifact)
    elif arguments.command == "hash":
        write_hash(arguments.root, arguments.artifact, arguments.output)
    else:
        fail("unknown command")


if __name__ == "__main__":
    main()
