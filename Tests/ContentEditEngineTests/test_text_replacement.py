"""Production-engine regressions; synthetic PDFs only, no private documents.

Run with Python 3.12 -I -S. The engine loads its pinned external runtime.
"""
import importlib.util
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("content_engine", ROOT / "Resources/ContentEdit/engine.py")
engine = importlib.util.module_from_spec(spec)
spec.loader.exec_module(engine)
engine.fitz = engine.load_runtime()
fitz = engine.fitz
ORIGINAL = "合同管理办法，原文文字测试"
REPLACEMENT = "合同管理办法，修改文字测试"


def compact_font_pdf():
    """A standard CJK CID font with the compact metrics common to FangSong PDFs."""
    with fitz.open() as doc:
        page = doc.new_page(width=595, height=842)
        page.insert_text((80, 250), ORIGINAL, fontname="china-s", fontsize=12)
        font_xref = page.get_fonts()[0][0]
        descendant = int(doc.xref_get_key(font_xref, "DescendantFonts")[1].strip("[] ").split()[0])
        descriptor = int(doc.xref_get_key(descendant, "FontDescriptor")[1].split()[0])
        for xref in (font_xref, descendant):
            doc.xref_set_key(xref, "BaseFont", "/FangSong_GB2312")
        for key, value in {"FontName": "/FangSong_GB2312", "Ascent": "880",
                           "Descent": "-120", "FontBBox": "[0 -120 1000 880]"}.items():
            doc.xref_set_key(descriptor, key, value)
        # Do not alias the deliberately modified original to the fallback's
        # reserved resource name (which would test font-resource corruption).
        resources = int(doc.xref_get_key(page.xref, "Resources")[1].split()[0])
        doc.xref_set_key(resources, "Font", f"<< /original {font_xref} 0 R >>")
        for stream in page.get_contents():
            doc.update_stream(stream, doc.xref_stream(stream).replace(b"/china-s", b"/original"))
        page.insert_text((80, 275), "Neighbor preserved", fontsize=12)
        page.draw_rect(fitz.Rect(70, 230, 300, 285), color=(0, 0, 1))
        doc.new_page().insert_text((80, 80), "Other page intact")
        return doc.tobytes()


class TextReplacementTests(unittest.TestCase):
    def replace(self, doc, text):
        target = next(s for s in engine.inspect_page(doc[0])["spans"] if s["text"] == ORIGINAL)
        return engine.edit_document(doc, {"operation": "replace-text", "page": 0,
                                          "id": target["id"], "text": text})

    def test_equal_length_chinese_with_compact_original_font_applies_and_reopens(self):
        source = compact_font_pdf()
        with fitz.open(stream=source, filetype="pdf") as doc:
            before = engine.page_fingerprint(doc[1])
            graphics = engine.graphics_signature(doc[0])
            _, data = self.replace(doc, REPLACEMENT)
        with fitz.open(stream=data, filetype="pdf") as result:
            self.assertIn(REPLACEMENT, result[0].get_text())
            self.assertNotIn(ORIGINAL, result[0].get_text())
            self.assertIn("Neighbor preserved", result[0].get_text())
            self.assertEqual(engine.page_fingerprint(result[1]), before)
            self.assertEqual(engine.graphics_signature(result[0]), graphics)
            target = next(s for s in engine.inspect_page(result[0])["spans"] if s["text"] == REPLACEMENT)
            self.assertEqual(target["size"], 12)
            self.assertEqual(target["origin"], [80, 250])
        with fitz.open(stream=source, filetype="pdf") as original:
            self.assertIn(ORIGINAL, original[0].get_text())

    def test_shorter_chinese_is_not_rejected_for_font_height(self):
        with fitz.open(stream=compact_font_pdf(), filetype="pdf") as doc:
            self.replace(doc, "修改文字")

    def test_actual_horizontal_overflow_still_rejected(self):
        with fitz.open(stream=compact_font_pdf(), filetype="pdf") as doc:
            with self.assertRaisesRegex(ValueError, "fit"):
                self.replace(doc, REPLACEMENT * 3)

    def test_expanded_font_box_must_not_overlap_neighbor_text(self):
        with fitz.open(stream=compact_font_pdf(), filetype="pdf") as doc:
            # Tiny but real text below the old font box, inside fallback descenders.
            doc[0].insert_text((80, 252.8), "X", fontsize=1)
            with self.assertRaisesRegex(ValueError, "overlap"):
                self.replace(doc, REPLACEMENT)

    def test_expanded_font_box_must_not_overlap_link(self):
        with fitz.open(stream=compact_font_pdf(), filetype="pdf") as doc:
            doc[0].insert_link({"kind": fitz.LINK_URI, "from": fitz.Rect(80, 252, 90, 254),
                                "uri": "https://example.com"})
            doc.reload_page(doc[0])
            with self.assertRaisesRegex(ValueError, "overlap"):
                self.replace(doc, REPLACEMENT)

    def test_expanded_font_box_must_not_overlap_annotation(self):
        with fitz.open(stream=compact_font_pdf(), filetype="pdf") as doc:
            doc[0].add_rect_annot(fitz.Rect(80, 252, 90, 254))
            with self.assertRaisesRegex(ValueError, "overlap"):
                self.replace(doc, REPLACEMENT)


if __name__ == "__main__":
    unittest.main()
