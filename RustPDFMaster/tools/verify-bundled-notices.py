#!/usr/bin/env python3
"""Verify packaged notices are the exact validated generated HTML files."""

from __future__ import annotations

import importlib.util
import stat
import sys
import zipfile
from pathlib import Path, PurePosixPath


TOOLS = Path(__file__).resolve().parent
VALIDATOR = TOOLS / "validate-third-party-notices.py"


def fail(message: str) -> "None":
    raise SystemExit(message)


def load_validator():
    spec = importlib.util.spec_from_file_location("third_party_notice_validator", VALIDATOR)
    if spec is None or spec.loader is None:
        fail("cannot load third-party notice validator")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def validated_source(root_value: str):
    validator = load_validator()
    root = Path(root_value)
    errors = validator.validate_directory(root)
    if errors:
        fail("generated notice source did not pass validation")
    return validator, root


def verify_directory(source_value: str, bundled_value: str) -> None:
    validator, source = validated_source(source_value)
    bundled = Path(bundled_value)
    errors = validator.validate_directory(bundled)
    if errors:
        fail("bundled notice directory did not pass validation")
    for name in validator.EXPECTED_OUTPUTS:
        if (source / name).read_bytes() != (bundled / name).read_bytes():
            fail(f"bundled notice differs from generated source: {name}")


def validated_prefix(value: str) -> str:
    if not value.endswith("/") or value.startswith("/") or "\\" in value:
        fail("notice ZIP prefix must be a relative POSIX directory")
    parts = value.removesuffix("/").split("/")
    if not parts or any(part in {"", ".", ".."} for part in parts):
        fail("notice ZIP prefix must not alias or escape")
    return PurePosixPath(*parts).as_posix() + "/"


def verify_zip(source_value: str, archive_value: str, prefix_value: str) -> None:
    validator, source = validated_source(source_value)
    archive_path = Path(archive_value)
    if archive_path.is_symlink() or not archive_path.is_file():
        fail("candidate ZIP must be a regular non-symlink file")
    prefix = validated_prefix(prefix_value)
    expected = {f"{prefix}{name}" for name in validator.EXPECTED_OUTPUTS}
    try:
        with zipfile.ZipFile(archive_path) as archive:
            matching = [info for info in archive.infolist() if info.filename.startswith(prefix)]
            observed = {info.filename for info in matching if not info.is_dir()}
            directory_entries = {info.filename for info in matching if info.is_dir()}
            if observed != expected or directory_entries - {prefix}:
                fail("candidate ZIP notice members are missing or extra")
            for info in matching:
                if info.is_dir():
                    continue
                mode = (info.external_attr >> 16) & 0o170000
                if mode == stat.S_IFLNK or info.file_size > validator.MAX_NOTICE_BYTES:
                    fail("candidate ZIP notice member is unsafe or exceeds its budget")
                name = info.filename.removeprefix(prefix)
                if archive.read(info) != (source / name).read_bytes():
                    fail(f"candidate ZIP notice differs from generated source: {name}")
    except (zipfile.BadZipFile, RuntimeError, NotImplementedError, EOFError):
        fail("candidate ZIP cannot be safely read")


def main(argv: list[str]) -> int:
    if len(argv) == 4 and argv[1] == "directory":
        verify_directory(argv[2], argv[3])
    elif len(argv) == 5 and argv[1] == "zip":
        verify_zip(argv[2], argv[3], argv[4])
    else:
        print(
            "usage: verify-bundled-notices.py directory SOURCE BUNDLED\n"
            "   or: verify-bundled-notices.py zip SOURCE ARCHIVE PREFIX",
            file=sys.stderr,
        )
        return 2
    print("Verified exact bundled third-party notices.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
