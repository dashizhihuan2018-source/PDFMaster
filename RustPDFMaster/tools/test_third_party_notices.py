#!/usr/bin/env python3
"""Contract tests for bounded third-party notice generation."""

from __future__ import annotations

import importlib.util
import os
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
TEMPLATE = ROOT / "templates" / "third-party-notices.hbs"
GENERATOR = TOOLS / "generate-third-party-notices.sh"
VALIDATOR = TOOLS / "validate-third-party-notices.py"

EXPECTED_OUTPUTS = {
    "macos-aarch64.html",
    "windows-x86_64.html",
    "ios-simulator-aarch64.html",
    "ios-device-aarch64.html",
    "ui-wasm32.html",
}


def load_validator():
    spec = importlib.util.spec_from_file_location("third_party_notice_validator", VALIDATOR)
    if spec is None or spec.loader is None:
        raise RuntimeError("cannot load third-party notice validator")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def safe_notice(package_name: str = "safe-crate", version: str = "1.2.3") -> str:
    return f"""<!doctype html>
<html lang="en"><head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'">
<title>Third-party notices</title><style>body {{ margin: 0; }}</style></head>
<body><main>
<section class="license" data-license-id="MIT">
<h2>MIT License <span class="license-id">MIT</span></h2><h3>Packages</h3>
<ul class="packages"><li class="package" data-package-name="{package_name}" data-package-version="{version}">
<a class="crate-source" data-source-kind="repository" href="https://example.invalid/source" rel="noreferrer noopener">{package_name} {version}</a>
</li></ul>
<pre class="license-text">Copyright &lt;holder&gt; &amp; contributors</pre>
</section>
</main></body></html>"""


class ThirdPartyNoticeContract(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.validator = load_validator()

    def write_complete_fixture(self, root: Path) -> None:
        for name in EXPECTED_OUTPUTS:
            (root / name).write_text(safe_notice(), encoding="utf-8")

    def test_generator_pins_tool_and_declares_every_target(self) -> None:
        source = GENERATOR.read_text(encoding="utf-8")
        self.assertIn('cargo_about_version="0.9.2"', source)
        self.assertIn("--locked", source)
        self.assertIn("--fail", source)
        self.assertIn('[[ -L "$output_dir/$output" ]]', source)
        self.assertIn("reject_symlink_components", source)
        for manifest, target, output in [
            ("src-tauri/Cargo.toml", "aarch64-apple-darwin", "macos-aarch64.html"),
            ("src-tauri/Cargo.toml", "x86_64-pc-windows-msvc", "windows-x86_64.html"),
            ("src-tauri/Cargo.toml", "aarch64-apple-ios-sim", "ios-simulator-aarch64.html"),
            ("src-tauri/Cargo.toml", "aarch64-apple-ios", "ios-device-aarch64.html"),
            ("app/Cargo.toml", "wasm32-unknown-unknown", "ui-wasm32.html"),
        ]:
            self.assertIn(manifest, source)
            self.assertIn(target, source)
            self.assertIn(output, source)

    def test_template_uses_escaped_fields_and_excludes_private_metadata(self) -> None:
        source = TEMPLATE.read_text(encoding="utf-8")
        self.assertIn("{{text}}", source)
        self.assertIn("{{crate.name}}", source)
        self.assertIn("{{crate.version}}", source)
        self.assertIn("{{crate.repository}}", source)
        self.assertIn("{{#if crate.source}}", source)
        self.assertIn('data-crate-source="{{crate.source}}"', source)
        self.assertIn("https://crates.io/crates/{{crate.name}}/{{crate.version}}", source)
        self.assertNotIn("{{{", source)
        for forbidden in ["source_path", "manifest_path", "used_by.path"]:
            self.assertNotIn(forbidden, source)

    def test_validator_accepts_complete_escaped_notices(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            self.write_complete_fixture(root)
            self.assertEqual(self.validator.validate_directory(root), [])

    def test_validator_rejects_every_extra_file_and_subdirectory(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            self.write_complete_fixture(root)

            extra = root / "private-paths.txt"
            extra.write_text("/Users/private/source", encoding="utf-8")
            errors = self.validator.validate_directory(root)
            self.assertTrue(
                any("private-paths.txt" in error and "unexpected" in error for error in errors),
                errors,
            )

            extra.unlink()
            (root / "nested-private-data").mkdir()
            errors = self.validator.validate_directory(root)
            self.assertTrue(
                any(
                    "nested-private-data" in error and "regular file" in error
                    for error in errors
                ),
                errors,
            )

    def test_validator_accepts_only_explicit_pdfmaster_workspace_package_without_link(self) -> None:
        notice = safe_notice().replace(
            'data-package-name="safe-crate" data-package-version="1.2.3"',
            'data-package-name="pdfmaster-core" data-package-version="0.1.0"',
        ).replace(
            '<a class="crate-source" data-source-kind="repository" href="https://example.invalid/source" rel="noreferrer noopener">safe-crate 1.2.3</a>',
            '<span class="workspace-package" data-source-kind="workspace">pdfmaster-core 0.1.0</span>',
        )
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            for name in EXPECTED_OUTPUTS:
                (root / name).write_text(notice, encoding="utf-8")
            self.assertEqual(self.validator.validate_directory(root), [])

            unknown = notice.replace("pdfmaster-core", "unknown-local")
            (root / "macos-aarch64.html").write_text(unknown, encoding="utf-8")
            self.assertTrue(
                any("unknown workspace package" in error for error in self.validator.validate_directory(root))
            )

            missing_source = safe_notice().replace(
                '<a class="crate-source" data-source-kind="repository" href="https://example.invalid/source" rel="noreferrer noopener">safe-crate 1.2.3</a>',
                '<span>safe-crate 1.2.3</span>',
            )
            (root / "macos-aarch64.html").write_text(missing_source, encoding="utf-8")
            self.assertTrue(
                any("unknown missing source" in error for error in self.validator.validate_directory(root))
            )

    def test_validator_accepts_bounded_crates_io_registry_fallback(self) -> None:
        notice = safe_notice().replace(
            'data-source-kind="repository" href="https://example.invalid/source"',
            'data-source-kind="registry" data-crate-source="registry+https://github.com/rust-lang/crates.io-index" href="https://crates.io/crates/safe-crate/1.2.3"',
        )
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            for name in EXPECTED_OUTPUTS:
                (root / name).write_text(notice, encoding="utf-8")
            self.assertEqual(self.validator.validate_directory(root), [])

    def test_validator_requires_every_target_and_nonempty_identity(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            self.write_complete_fixture(root)
            (root / "ui-wasm32.html").unlink()
            self.assertTrue(self.validator.validate_directory(root))

            (root / "ui-wasm32.html").write_text(
                safe_notice(package_name="", version=""), encoding="utf-8"
            )
            errors = self.validator.validate_directory(root)
            self.assertTrue(any("package name" in error for error in errors))
            self.assertTrue(any("package version" in error for error in errors))

    def test_validator_rejects_symlinked_notice_output(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            self.write_complete_fixture(root)
            linked = root / "macos-aarch64.html"
            linked.unlink()
            linked.symlink_to(root / "ui-wasm32.html")
            self.assertTrue(
                any("symbolic link" in error for error in self.validator.validate_directory(root))
            )

    def test_validator_and_generator_reject_symlinked_output_ancestors(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            real = root / "real"
            output = real / "notices"
            output.mkdir(parents=True)
            self.write_complete_fixture(output)
            linked = root / "linked"
            linked.symlink_to(real, target_is_directory=True)
            linked_output = linked / "notices"
            self.assertTrue(
                any("symbolic link component" in error for error in self.validator.validate_directory(linked_output))
            )

            fake_about = root / "cargo-about"
            fake_about.write_text(
                "#!/bin/sh\n"
                "if [ \"$1\" = \"--version\" ]; then echo 'cargo-about 0.9.2'; exit 0; fi\n"
                "exit 99\n",
                encoding="utf-8",
            )
            fake_about.chmod(0o755)
            # Windows CreateProcess cannot execute a .sh file directly. Let Bash
            # also derive fixture paths from its own PWD (MSYS /d/... rather
            # than a Windows D:\... path interpreted as relative by the script).
            result = subprocess.run(
                [
                    "bash", "-c",
                    'CARGO_ABOUT_BIN="$PWD/cargo-about" exec bash "$1" "$PWD/linked/notices"',
                    "notice-fixture", GENERATOR.as_posix(),
                ],
                cwd=root,
                env=os.environ.copy(),
                capture_output=True,
                text=True,
                check=False,
                timeout=15,
            )
            self.assertEqual(result.returncode, 2)
            self.assertIn("symbolic link component", result.stderr)

    def test_validator_rejects_paths_secrets_active_html_and_unsafe_links(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            self.write_complete_fixture(root)
            unsafe = safe_notice().replace(
                "Copyright &lt;holder&gt; &amp; contributors",
                '<script>alert(1)</script> /Users/private/project ghp_abcdefghijklmnopqrstuvwxyz123456',
            ).replace("https://example.invalid/source", "javascript:alert(1)")
            (root / "macos-aarch64.html").write_text(unsafe, encoding="utf-8")
            errors = self.validator.validate_directory(root)
            self.assertTrue(any("absolute path" in error for error in errors))
            self.assertTrue(any("credential" in error for error in errors))
            self.assertTrue(any("active HTML" in error for error in errors))
            self.assertTrue(any("unsafe link" in error for error in errors))

    def test_validator_rejects_attribute_css_svg_meta_and_url_credential_injection(self) -> None:
        mutations = {
            "event handler": lambda text: text.replace("<main>", '<main onload="alert(1)">'),
            "unsafe stylesheet": lambda text: text.replace(
                "body { margin: 0; }", "body { background: url(https://evil.invalid/x); }"
            ),
            "unexpected element": lambda text: text.replace("<main>", "<main><svg>"),
            "unsafe meta": lambda text: text.replace(
                '<meta charset="utf-8">',
                '<meta charset="utf-8"><meta http-equiv="refresh" content="0;url=https://evil.invalid">',
            ),
            "embedded credentials": lambda text: text.replace(
                "https://example.invalid/source", "https://user:pass@example.invalid/source"
            ),
        }
        for expected, mutate in mutations.items():
            with self.subTest(expected=expected), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary).resolve()
                self.write_complete_fixture(root)
                (root / "macos-aarch64.html").write_text(
                    mutate(safe_notice()), encoding="utf-8"
                )
                errors = self.validator.validate_directory(root)
                self.assertTrue(any(expected in error for error in errors), errors)


if __name__ == "__main__":
    unittest.main()
