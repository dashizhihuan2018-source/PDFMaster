#!/usr/bin/env python3
"""Validate generated third-party notices without exposing source metadata."""

from __future__ import annotations

import os
import re
import stat
import sys
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import urlsplit


EXPECTED_OUTPUTS = {
    "macos-aarch64.html",
    "windows-x86_64.html",
    "ios-simulator-aarch64.html",
    "ios-device-aarch64.html",
    "ui-wasm32.html",
}
MAX_NOTICE_BYTES = 64 * 1024 * 1024
ACTIVE_TAGS = {"script", "iframe", "object", "embed", "form"}
ALLOWED_ELEMENTS = {
    "html", "head", "meta", "title", "style", "body", "main", "h1", "p",
    "section", "h2", "span", "h3", "ul", "li", "a", "pre",
}
ALLOWED_ATTRIBUTES = {
    "html": {"lang"},
    "meta": {"charset", "name", "content", "http-equiv"},
    "p": {"class"},
    "section": {"class", "data-license-id"},
    "span": {"class", "data-source-kind"},
    "ul": {"class"},
    "li": {"class", "data-package-name", "data-package-version"},
    "a": {"class", "data-source-kind", "data-crate-source", "rel", "href"},
    "pre": {"class"},
}
EXPECTED_CLASSES = {
    "p": {frozenset({"scope"})},
    "section": {frozenset({"license"})},
    "span": {frozenset({"license-id"}), frozenset({"workspace-package"})},
    "ul": {frozenset({"packages"})},
    "li": {frozenset({"package"})},
    "a": {frozenset({"crate-source"})},
    "pre": {frozenset({"license-text"})},
}
EXPECTED_CSP = "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'"
CRATES_IO_SOURCE = "registry+https://github.com/rust-lang/crates.io-index"
WORKSPACE_PACKAGES = {"pdfmaster-core", "pdfmaster-app", "pdfmaster-ui"}
PACKAGE_NAME = re.compile(r"^[A-Za-z0-9_-]+$")
PACKAGE_VERSION = re.compile(r"^[A-Za-z0-9.+-]+$")
UNSAFE_STYLESHEET = re.compile(r"(?:url\s*\(|@import\b|expression\s*\()", re.IGNORECASE)
ABSOLUTE_PATH = re.compile(
    r"(?:/Users/|/home/|(?<![A-Za-z])[A-Za-z]:[\\/]|\.cargo[\\/](?:registry|git)[\\/]|file://)",
    re.IGNORECASE,
)
CREDENTIAL = re.compile(
    r"(?:ghp_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|AKIA[A-Z0-9]{16}|sk-[A-Za-z0-9]{20,})"
)


class NoticeParser(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.licenses: list[dict[str, object]] = []
        self.links: list[str] = []
        self.active_tags: list[str] = []
        self.structure_errors: list[str] = []
        self.current_license: dict[str, object] | None = None
        self.current_package: dict[str, str] | None = None
        self.in_license_text = False
        self.in_style = False
        self.styles: list[str] = []

    @staticmethod
    def classes(attributes: dict[str, str]) -> set[str]:
        return set(attributes.get("class", "").split())

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        tag = tag.lower()
        attribute_names = [key.lower() for key, _ in attrs]
        attributes = {key: value or "" for key, value in attrs}
        if tag in ACTIVE_TAGS:
            self.active_tags.append(tag)
        if tag not in ALLOWED_ELEMENTS:
            self.structure_errors.append(f"unexpected element {tag}")
        if len(attribute_names) != len(set(attribute_names)):
            self.structure_errors.append(f"duplicate attribute on {tag}")
        if any(name.startswith("on") for name in attribute_names):
            self.structure_errors.append(f"event handler attribute on {tag}")
        unexpected_attributes = set(attribute_names) - ALLOWED_ATTRIBUTES.get(tag, set())
        if unexpected_attributes:
            self.structure_errors.append(f"unexpected attribute on {tag}")
        if tag in EXPECTED_CLASSES:
            classes = frozenset(self.classes(attributes))
            if classes not in EXPECTED_CLASSES[tag]:
                self.structure_errors.append(f"unexpected class shape on {tag}")
        if tag == "html" and attributes.get("lang") != "en":
            self.structure_errors.append("unexpected html language")
        if tag == "meta":
            allowed_meta = (
                attributes == {"charset": "utf-8"}
                or attributes
                == {"name": "viewport", "content": "width=device-width, initial-scale=1"}
                or attributes
                == {"http-equiv": "Content-Security-Policy", "content": EXPECTED_CSP}
            )
            if not allowed_meta:
                self.structure_errors.append("unsafe meta declaration")
        if tag == "style":
            self.in_style = True
        if tag == "section" and "license" in self.classes(attributes):
            license_record: dict[str, object] = {
                "id": attributes.get("data-license-id", "").strip(),
                "packages": [],
                "text": [],
            }
            self.licenses.append(license_record)
            self.current_license = license_record
        elif tag == "li" and "package" in self.classes(attributes):
            package = {
                "name": attributes.get("data-package-name", "").strip(),
                "version": attributes.get("data-package-version", "").strip(),
                "href": "",
                "source_kind": "",
                "crate_source": "",
            }
            self.current_package = package
            if self.current_license is not None:
                packages = self.current_license["packages"]
                assert isinstance(packages, list)
                packages.append(package)
        elif tag == "a" and "crate-source" in self.classes(attributes):
            href = attributes.get("href", "").strip()
            self.links.append(href)
            if self.current_package is not None:
                self.current_package["href"] = href
                self.current_package["source_kind"] = attributes.get("data-source-kind", "")
                self.current_package["crate_source"] = attributes.get("data-crate-source", "")
            if set(attributes.get("rel", "").split()) != {"noreferrer", "noopener"}:
                self.structure_errors.append("unexpected link relation")
        elif tag == "span" and "workspace-package" in self.classes(attributes):
            if self.current_package is not None:
                self.current_package["source_kind"] = attributes.get("data-source-kind", "")
        elif tag == "pre" and "license-text" in self.classes(attributes):
            self.in_license_text = True

    def handle_endtag(self, tag: str) -> None:
        tag = tag.lower()
        if tag == "pre":
            self.in_license_text = False
        elif tag == "style":
            self.in_style = False
        elif tag == "li":
            self.current_package = None
        elif tag == "section":
            self.current_license = None

    def handle_data(self, data: str) -> None:
        if self.in_style:
            self.styles.append(data)
        if self.in_license_text and self.current_license is not None:
            text = self.current_license["text"]
            assert isinstance(text, list)
            text.append(data)


def validate_notice(name: str, path: Path) -> list[str]:
    errors: list[str] = []
    if path.is_symlink():
        return [f"{name}: notice output must not be a symbolic link"]
    if not path.is_file():
        return [f"{name}: missing expected notice"]
    size = path.stat().st_size
    if size == 0:
        return [f"{name}: notice is empty"]
    if size > MAX_NOTICE_BYTES:
        return [f"{name}: notice exceeds {MAX_NOTICE_BYTES} bytes"]
    try:
        source = path.read_text(encoding="utf-8")
    except (OSError, UnicodeError):
        return [f"{name}: notice is not readable UTF-8"]

    if ABSOLUTE_PATH.search(source):
        errors.append(f"{name}: contains an absolute path or local file URL")
    if CREDENTIAL.search(source):
        errors.append(f"{name}: contains a credential-shaped value")
    for private_field in ("manifest_path", "source_path", "used_by.path"):
        if private_field in source:
            errors.append(f"{name}: exposes private metadata field {private_field}")

    parser = NoticeParser()
    try:
        parser.feed(source)
        parser.close()
    except Exception:
        return errors + [f"{name}: HTML parsing failed"]
    if parser.active_tags:
        errors.append(f"{name}: contains active HTML tag {parser.active_tags[0]}")
    errors.extend(f"{name}: {error}" for error in parser.structure_errors)
    if UNSAFE_STYLESHEET.search("".join(parser.styles)):
        errors.append(f"{name}: contains an unsafe stylesheet construct")
    if not parser.licenses:
        errors.append(f"{name}: has no license records")
    for index, license_record in enumerate(parser.licenses, start=1):
        license_id = license_record["id"]
        packages = license_record["packages"]
        text = license_record["text"]
        assert isinstance(license_id, str)
        assert isinstance(packages, list)
        assert isinstance(text, list)
        if not license_id:
            errors.append(f"{name}: license {index} has an empty identifier")
        if not "".join(text).strip():
            errors.append(f"{name}: license {index} has empty license text")
        if not packages:
            errors.append(f"{name}: license {index} has no packages")
        for package_index, package in enumerate(packages, start=1):
            assert isinstance(package, dict)
            if not package["name"]:
                errors.append(
                    f"{name}: license {index} package {package_index} has empty package name"
                )
            if not package["version"]:
                errors.append(
                    f"{name}: license {index} package {package_index} has empty package version"
                )
            package_name = package["name"]
            package_version = package["version"]
            if package_name and not PACKAGE_NAME.fullmatch(package_name):
                errors.append(f"{name}: license {index} package {package_index} has unsafe package name")
            if package_version and not PACKAGE_VERSION.fullmatch(package_version):
                errors.append(f"{name}: license {index} package {package_index} has unsafe package version")
            source_kind = package["source_kind"]
            if source_kind == "workspace":
                if package_name not in WORKSPACE_PACKAGES:
                    errors.append(f"{name}: unknown workspace package {package_name}")
                if package["href"] or package["crate_source"]:
                    errors.append(f"{name}: workspace package exposes an unexpected source")
            elif source_kind == "registry":
                if package["crate_source"] != CRATES_IO_SOURCE:
                    errors.append(f"{name}: unsupported registry source")
                expected = f"https://crates.io/crates/{package_name}/{package_version}"
                if package["href"] != expected:
                    errors.append(f"{name}: registry package has an unexpected crates.io link")
            elif source_kind == "repository":
                if not package["href"] or package["crate_source"]:
                    errors.append(f"{name}: repository package has invalid source metadata")
            else:
                errors.append(f"{name}: package has unknown missing source")
    for href in parser.links:
        parsed = urlsplit(href)
        if parsed.scheme != "https" or not parsed.netloc:
            errors.append(f"{name}: contains an unsafe link scheme")
        if parsed.username is not None or parsed.password is not None:
            errors.append(f"{name}: link contains embedded credentials")
    return errors


def validate_directory(root: Path) -> list[str]:
    errors: list[str] = []
    absolute_root = Path(os.path.abspath(root))
    if any(component.is_symlink() for component in (absolute_root, *absolute_root.parents)):
        return ["notice output path contains a symbolic link component"]
    if not root.is_dir():
        return ["notice output directory is missing"]
    try:
        entries = sorted(root.iterdir(), key=lambda path: path.name)
    except OSError:
        return ["notice output directory cannot be enumerated"]
    observed = {path.name for path in entries}
    for unexpected in sorted(observed - EXPECTED_OUTPUTS):
        errors.append(f"{unexpected}: unexpected notice output")
    for path in entries:
        try:
            mode = path.lstat().st_mode
        except OSError:
            errors.append(f"{path.name}: notice output cannot be inspected")
            continue
        if stat.S_ISLNK(mode):
            errors.append(f"{path.name}: notice output must not be a symbolic link")
        elif not stat.S_ISREG(mode):
            errors.append(f"{path.name}: notice output must be a regular file")
    for name in sorted(EXPECTED_OUTPUTS):
        errors.extend(validate_notice(name, root / name))
    return errors


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("usage: validate-third-party-notices.py OUTPUT_DIRECTORY", file=sys.stderr)
        return 2
    errors = validate_directory(Path(argv[1]))
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    print(f"Validated {len(EXPECTED_OUTPUTS)} third-party notice targets.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
