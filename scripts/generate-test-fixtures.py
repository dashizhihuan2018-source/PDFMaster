#!/usr/bin/env python3
"""Generate synthetic content-edit fixtures; never reads a user's document."""
from pathlib import Path
import pymupdf as fitz

root = Path(__file__).resolve().parent.parent
destination = root / "work/parallel/content-edit/evidence"
destination.mkdir(parents=True, exist_ok=True)
pdf = destination / "content-edit-qa.pdf"
image = destination / "replacement.png"
if pdf.exists() or image.exists():
    raise SystemExit("Fixtures already exist; use a fresh checkout to avoid overwriting data.")
doc = fitz.open()
page = doc.new_page(width=400, height=500)
page.insert_text((40, 60), "Original target", fontsize=16)
page.insert_text((40, 100), "Neighbor preserved", fontsize=12)
page.insert_text((40, 130), "中文原文", fontsize=14, fontname="china-s")
page.draw_rect(fitz.Rect(30, 180, 160, 220), color=(0, 0, 1), fill=(0.8, 0.8, 1))
pixels = fitz.Pixmap(fitz.csRGB, fitz.IRect(0, 0, 16, 16), 0)
pixels.clear_with(100)
page.insert_image(fitz.Rect(220, 180, 300, 260), stream=pixels.tobytes("png"))
doc.new_page(width=400, height=500).insert_text((40, 80), "Other page intact", fontsize=12)
doc.save(pdf)
doc.close()
pixels.clear_with(230)
pixels.save(image)
print("Generated synthetic fixtures under work/parallel/content-edit/evidence/.")
