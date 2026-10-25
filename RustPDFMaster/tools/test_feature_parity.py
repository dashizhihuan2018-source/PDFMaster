#!/usr/bin/env python3
"""Contract tests for the source-backed feature parity inventory."""

from __future__ import annotations

import importlib.util
import json
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
INVENTORY = ROOT / "cross-platform" / "features.json"
VALIDATOR = ROOT / "tools" / "validate-feature-parity.py"

EXPECTED_TOOLS = {
    "annotate": "partial",
    "edit": "partial",
    "sign": "partial",
    "forms": "unavailable",
    "comments": "partial",
    "resize": "unavailable",
    "ocr": "partial",
    "pages": "partial",
    "compress": "partial",
    "merge": "partial",
    "compare": "unavailable",
    "page-numbers": "partial",
    "copilot": "partial",
    "blank": "available",
    "measure": "unavailable",
    "password": "partial",
    "translate": "unavailable",
    "redact": "partial",
    "redline": "unavailable",
    "draw": "partial",
    "image-to-pdf": "partial",
    "word-to-pdf": "unavailable",
    "sheet-to-pdf": "unavailable",
    "file-to-pdf": "unavailable",
    "pdf-to-word": "unavailable",
    "pdf-to-image": "partial",
    "pdf-to-excel": "unavailable",
    "pdf-to-powerpoint": "unavailable",
}


def load_validator():
    spec = importlib.util.spec_from_file_location("feature_parity_validator", VALIDATOR)
    if spec is None or spec.loader is None:
        raise RuntimeError("cannot load feature parity validator")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class FeatureParityContract(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.validator = load_validator()
        cls.document = json.loads(INVENTORY.read_text(encoding="utf-8"))

    def test_checked_in_inventory_passes_schema_and_coverage(self) -> None:
        self.assertEqual(self.validator.validate_document(self.document), [])
        self.assertGreaterEqual(len(self.document["features"]), 50)
        self.assertEqual(self.document["legacy_contracts"]["count"], 801)

    def test_original_tool_catalog_is_exact_and_unavailable_is_not_required(self) -> None:
        observed = {
            item["id"]: item["original_availability"]
            for item in self.document["original_tool_catalog"]
        }
        self.assertEqual(observed, EXPECTED_TOOLS)
        for item in self.document["original_tool_catalog"]:
            self.assertEqual(
                item["required_for_replica"],
                item["original_availability"] != "unavailable",
            )

    def test_inventory_covers_non_catalog_native_and_canvas_workflows(self) -> None:
        domains = {feature["domain"] for feature in self.document["features"]}
        self.assertTrue(
            {
                "tool_catalog",
                "shell",
                "reader",
                "document",
                "import_export",
                "local_canvas",
                "canvas_web",
                "ai_account",
                "safety",
            }.issubset(domains)
        )
        self.assertFalse(
            any(feature["rust"]["status"] == "complete" for feature in self.document["features"]),
            "the foundation candidate has no feature with cross-platform human acceptance yet",
        )

    def test_validator_rejects_duplicate_ids_and_missing_evidence(self) -> None:
        duplicate = json.loads(json.dumps(self.document))
        duplicate["features"].append(duplicate["features"][0])
        errors = self.validator.validate_document(duplicate)
        self.assertTrue(any("duplicate feature id" in error for error in errors))

        missing = json.loads(json.dumps(self.document))
        missing["features"][0]["original"]["evidence"] = []
        errors = self.validator.validate_document(missing)
        self.assertTrue(any("original evidence" in error for error in errors))

        bad_profile = json.loads(json.dumps(self.document))
        bad_profile["features"][0]["rust"]["platform_profile"] = "invented-profile"
        errors = self.validator.validate_document(bad_profile)
        self.assertTrue(any("platform profile" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
