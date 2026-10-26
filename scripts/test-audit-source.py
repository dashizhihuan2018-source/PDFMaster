#!/usr/bin/env python3
"""Synthetic regression checks; no credentials or user data are used."""
import contextlib
import importlib.util
import io
import json
import pathlib
import tempfile
import unittest

SOURCE = pathlib.Path(__file__).resolve().parent / "audit-source.py"
spec = importlib.util.spec_from_file_location("publication_audit", SOURCE)
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)
PUBLIC_ROOT = audit.ROOT
UI_PATH = "CanvasWeb/src/components/agent/agent-chat-inline-tokens.ts"
UI_LINE = (PUBLIC_ROOT / UI_PATH).read_text().splitlines()[2]
RUST_PATH = "RustPDFMaster/crates/pdfmaster-core/tests/document_contract.rs"
RUST_LINE = next(line for line in (PUBLIC_ROOT / RUST_PATH).read_text().splitlines() if "owner_password:" in line)


class PublicationAuditTests(unittest.TestCase):
    def test_nested_product_tools_are_scanned_but_root_build_tools_are_excluded(self):
        with tempfile.TemporaryDirectory(prefix="pdfmaster-manifest-") as directory:
            root = pathlib.Path(directory)
            for relative in ("tools/local-build/cache.txt", "RustPDFMaster/tools/baseline.rs"):
                path = root / relative
                path.parent.mkdir(parents=True)
                path.write_text("synthetic fixture\n")
            previous = audit.ROOT
            audit.ROOT = root
            try:
                self.assertEqual([p.relative_to(root).as_posix() for p in audit.files()],
                                 ["RustPDFMaster/tools/baseline.rs"])
            finally:
                audit.ROOT = previous

    def scan(self, line, path=UI_PATH):
        with tempfile.TemporaryDirectory(prefix="pdfmaster-audit-") as directory:
            root = pathlib.Path(directory)
            target = root / path
            target.parent.mkdir(parents=True)
            target.write_text(line + "\n")
            previous = audit.ROOT
            audit.ROOT = root
            try:
                output = io.StringIO()
                with contextlib.redirect_stdout(output):
                    failed = audit.audit()
                return failed, json.loads(output.getvalue())
            finally:
                audit.ROOT = previous

    def test_exact_reviewed_css_token_line_is_not_a_credential(self):
        failed, result = self.scan(UI_LINE)
        self.assertFalse(failed)
        self.assertEqual(result["reviewed_ui_literal_lines"], 1)

    def test_changed_reviewed_line_requires_review_again(self):
        failed, result = self.scan(UI_LINE + " // changed")
        self.assertTrue(failed)
        self.assertIn("credential_literal_review", {item["kind"] for item in result["unexpected_matches"]})

    def test_exact_synthetic_rust_fixture_is_reviewed_without_exempting_rust_tests(self):
        failed, _ = self.scan(RUST_LINE, RUST_PATH)
        self.assertFalse(failed)
        failed, result = self.scan(RUST_LINE + " // changed", RUST_PATH)
        self.assertTrue(failed)
        self.assertIn("credential_literal_review", {item["kind"] for item in result["unexpected_matches"]})

    def test_provider_pattern_is_still_rejected_on_ui_line(self):
        synthetic = "sk-" + "syntheticfixture0000000000000000"
        failed, result = self.scan(UI_LINE + " // " + synthetic)
        self.assertTrue(failed)
        self.assertIn("provider_credential", {item["kind"] for item in result["unexpected_matches"]})
        self.assertNotIn(synthetic, json.dumps(result))


if __name__ == "__main__":
    unittest.main()
