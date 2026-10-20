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


class PublicationAuditTests(unittest.TestCase):
    def scan(self, line):
        with tempfile.TemporaryDirectory(prefix="pdfmaster-audit-") as directory:
            root = pathlib.Path(directory)
            target = root / UI_PATH
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

    def test_provider_pattern_is_still_rejected_on_ui_line(self):
        synthetic = "sk-" + "syntheticfixture0000000000000000"
        failed, result = self.scan(UI_LINE + " // " + synthetic)
        self.assertTrue(failed)
        self.assertIn("provider_credential", {item["kind"] for item in result["unexpected_matches"]})
        self.assertNotIn(synthetic, json.dumps(result))


if __name__ == "__main__":
    unittest.main()
