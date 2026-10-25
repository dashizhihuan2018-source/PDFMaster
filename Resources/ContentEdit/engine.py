#!/usr/bin/env python3
"""Independent, fail-closed local content editing. No network or source writes."""
import argparse
import collections
import hashlib
import importlib.util
import json
import math
import os
import pathlib
import re
import stat
import sys
import tempfile
import urllib.parse

MAX_PDF = 256 * 1024 * 1024
MAX_OBJECTS = 100000
LIMITATIONS = ["Independent research subset, not complete PDF Expert parity.",
               "No rotated/cropped pages, forms, layers, signed/encrypted/active-content PDFs."]


def load_runtime():
    # -I -S suppresses ALL site, .pth and sitecustomize execution. Import ONLY
    # this installed package, not its enclosing user site (which contains .pth).
    root = pathlib.Path.home() / "Library/Python/3.12/lib/python/site-packages/pymupdf"
    if not sys.flags.isolated or not sys.flags.no_site:
        raise ValueError("Engine must run with Python -I -S")
    if sys.version_info[:2] != (3, 12) or not root.is_dir() or root.is_symlink():
        raise ValueError("Required external Python 3.12 PyMuPDF runtime not available")
    for item in [root, *root.rglob("*")]:
        info = item.lstat()
        if stat.S_ISLNK(info.st_mode) or info.st_uid not in (os.getuid(), 0) or info.st_mode & 0o022:
            raise ValueError("Untrusted external PyMuPDF runtime permissions")
    # PyMuPDF supports environment-controlled log destinations: disallow those.
    for key in list(os.environ):
        if key.startswith("PYMUPDF_") or key.startswith("MUPDF_"):
            os.environ.pop(key)
    spec = importlib.util.spec_from_file_location("pymupdf", root / "__init__.py", submodule_search_locations=[str(root)])
    module = importlib.util.module_from_spec(spec)
    sys.modules["pymupdf"] = module
    spec.loader.exec_module(module)
    if module.VersionBind != "1.27.2.3":
        raise ValueError("External PyMuPDF version must be 1.27.2.3 (tested version)")
    return module


def read_regular(path, limit):
    path = pathlib.Path(path)
    if not path.is_absolute():
        raise ValueError("All paths must be absolute")
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_size > limit:
            raise ValueError("Input is not a regular bounded file")
        with os.fdopen(fd, "rb", closefd=False) as source:
            data = source.read(limit + 1)
        if len(data) > limit:
            raise ValueError("Input grew beyond allowed size")
        return data
    finally:
        os.close(fd)


def validate_output_path(path, forbidden):
    path = pathlib.Path(path)
    if not path.is_absolute() or path.exists() or path.is_symlink():
        raise ValueError("Output paths must be absolute and not already exist")
    if path.resolve() in forbidden or not path.parent.is_dir():
        raise ValueError("Output paths must not alias input/request/response")
    return path


def publish_exclusive(path, data):
    fd, temp = tempfile.mkstemp(prefix=".pdfmaster-", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as out:
            out.write(data)
            out.flush()
            os.fsync(out.fileno())
        os.link(temp, path)
    finally:
        os.unlink(temp)


def safe_document(doc):
    if not doc.is_pdf or doc.needs_pass or doc.is_encrypted:
        raise ValueError("Encrypted or non-PDF source refused")
    if doc.page_count < 1 or doc.page_count > 10000 or doc.xref_length() > MAX_OBJECTS:
        raise ValueError("Document object/page budget exceeded")
    if doc.get_sigflags() > 0 or doc.embfile_count():
        raise ValueError("Unsafe signed or attached-file document refused")
    # Semantic PDF names, including escaped spellings, outside streams.
    forbidden = {"JavaScript", "JS", "OpenAction", "AA", "EmbeddedFiles", "Filespec", "EF",
                 "Collection", "OCProperties", "OCG", "OCMD", "Sig", "XFA", "AcroForm",
                 "RichMedia", "Launch", "SubmitForm", "ImportData", "GoToR", "Sound", "Movie", "Encrypt"}
    for xref in range(1, doc.xref_length()):
        obj = doc.xref_object(xref, compressed=False)
        names = re.findall(r"/([^\s<>\[\](){}/%]+)", obj)
        names = {re.sub(r"#([0-9A-Fa-f]{2})", lambda m: chr(int(m[1], 16)), n) for n in names}
        if names & forbidden:
            raise ValueError("Unsafe active content, signature, attachment, form or layer refused")
    for page in doc:
        if page.rotation or page.cropbox != page.mediabox or page.mediabox.x0 or page.mediabox.y0:
            raise ValueError("Rotated/cropped/nonzero-origin pages unsupported")
        if page.rect.is_empty or not all(math.isfinite(x) for x in page.rect) or max(page.rect.width, page.rect.height) > 14400:
            raise ValueError("Invalid page geometry")
        if list(page.widgets() or []):
            raise ValueError("Unsafe form widgets unsupported")
        for link in page.get_links():
            if link.get("kind") not in (fitz.LINK_URI, fitz.LINK_GOTO):
                raise ValueError("Unsafe external action link unsupported")
            if link.get("kind") == fitz.LINK_URI:
                safe_uri(link.get("uri", ""))


def stable_id(kind, data):
    encoded = json.dumps(data, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()
    return kind + "-" + hashlib.sha256(encoded).hexdigest()[:24]


def inspect_page(page):
    spans = []
    for bi, block in enumerate(page.get_text("dict", flags=fitz.TEXTFLAGS_DICT & ~fitz.TEXT_PRESERVE_IMAGES)["blocks"]):
        for li, line in enumerate(block.get("lines", [])):
            for si, span in enumerate(line["spans"]):
                item = {key: span[key] for key in ("text", "bbox", "origin", "font", "size", "color")}
                item["bbox"] = list(item["bbox"])
                item["origin"] = list(item["origin"])
                item["id"] = stable_id("text", [page.number, bi, li, si, item])
                item["editable"] = (line.get("dir") == (1.0, 0.0) and not line.get("wmode", 0)
                                    and span.get("alpha",255) == 255 and span.get("char_flags",16) & 48 == 16)
                spans.append(item)
    images = []
    for i, image in enumerate(page.get_image_info(xrefs=True)):
        item = {"xref": image["xref"], "bbox": list(image["bbox"])}
        item["id"] = stable_id("image", [page.number, i, item])
        images.append(item)
    links = []
    for i, link in enumerate(page.get_links()):
        item = {"bbox":list(link["from"]), "uri":link.get("uri", ""), "xref":link.get("xref",0), "kind":link["kind"]}
        item["id"] = stable_id("link",[page.number, i, item])
        links.append(item)
    return {"page":page.number,"width":page.rect.width,"height":page.rect.height,
            "spans":spans,"images":images,"links":links,"limitations":LIMITATIONS}


def selected(items, identifier):
    matches = [x for x in items if x["id"] == identifier]
    if len(matches) != 1:
        raise ValueError("Stale or invalid content ID")
    return matches[0]


def checked_rect(values, page):
    if not isinstance(values, (list, tuple)) or len(values) != 4 or not all(type(v) in (int,float) and math.isfinite(v) for v in values):
        raise ValueError("Invalid rectangle")
    rect = fitz.Rect(values)
    if rect.is_empty or rect.is_infinite or not page.rect.contains(rect):
        raise ValueError("Rectangle must lie inside the page")
    return rect


def safe_uri(uri):
    if not isinstance(uri,str) or not uri or len(uri)>4096 or any(ord(c)<32 or c.isspace() for c in uri):
        raise ValueError("Invalid link URI")
    parsed = urllib.parse.urlsplit(uri)
    if parsed.scheme not in ("https","http","mailto") or parsed.username or parsed.password or (parsed.scheme != "mailto" and not parsed.hostname):
        raise ValueError("Only http/https/mailto links are supported")
    return uri


def span_signature(span):
    return (span["text"], tuple(round(v,3) for v in span["bbox"]), span["font"], round(span["size"],3),span["color"])


def image_signature(page):
    return sorted((tuple(round(v,3) for v in image["bbox"]),image["digest"].hex()) for image in page.get_image_info(hashes=True))


def graphics_signature(page):
    return [repr({k:v for k,v in drawing.items() if k != "seqno"}) for drawing in page.get_drawings()]


def page_fingerprint(page):
    content = inspect_page(page)
    links = sorted((tuple(link["bbox"]),link["uri"],link["kind"]) for link in content["links"])
    return (collections.Counter(span_signature(s) for s in content["spans"]),
            image_signature(page),graphics_signature(page),links,tuple(page.rect),page.rotation)


def check_image_header(data):
    """Bound decoded allocation before a MuPDF pixmap is ever constructed."""
    width = height = 0
    if data.startswith(b"\x89PNG\r\n\x1a\n"):
        if len(data)<33 or data[8:16] != b"\x00\x00\x00\rIHDR":
            raise ValueError("Invalid PNG image header")
        width,height = int.from_bytes(data[16:20],"big"),int.from_bytes(data[20:24],"big")
        if data[24]!=8 or data[25] not in (0,2,3):
            raise ValueError("Only opaque 8-bit PNG replacement images supported")
    elif data.startswith(b"\xff\xd8"):
        at = 2
        while at+4 <= len(data):
            if data[at]!=255:
                raise ValueError("Invalid JPEG image header")
            while at<len(data) and data[at]==255:
                at += 1
            marker = data[at]
            at += 1
            if marker in (0xd9,0xda):
                break
            if marker==0x01 or 0xd0<=marker<=0xd7:
                continue
            length = int.from_bytes(data[at:at+2],"big")
            if length<2 or at+length>len(data):
                raise ValueError("Invalid JPEG segment length")
            if marker in (0xc0,0xc1,0xc2):
                if length<8 or data[at+2]!=8 or data[at+7] not in (1,3):
                    raise ValueError("Unsupported JPEG precision/components")
                height = int.from_bytes(data[at+3:at+5],"big")
                width = int.from_bytes(data[at+5:at+7],"big")
                break
            at += length
    else:
        raise ValueError("Replacement image must be PNG or JPEG")
    if not width or not height or width>16384 or height>16384 or width*height>40_000_000:
        raise ValueError("Image header budget exceeded before decoding")
    return width,height


def replace_text(page, info, request):
    target = selected(info["spans"],request.get("id"))
    text = request.get("text")
    if not isinstance(text,str) or len(text)>4096 or any(ord(c)<32 for c in text):
        raise ValueError("Only a single non-control text line is supported")
    if not target["editable"]:
        raise ValueError("Only opaque horizontal spans are editable")
    rect = checked_rect(target["bbox"],page)
    for annotation in page.annots() or []:
        if annotation.type[0] == fitz.PDF_ANNOT_REDACT:
            raise ValueError("Existing pending redaction must not be applied by text editing")
        if rect.intersects(annotation.rect):
            raise ValueError("Text overlaps an existing annotation; edit refused")
    if any(rect.intersects(fitz.Rect(link["bbox"])) for link in info["links"]):
        raise ValueError("Text overlaps an existing link; edit refused")
    for span in info["spans"]:
        if span["id"] != target["id"] and rect.intersects(fitz.Rect(span["bbox"])):
            raise ValueError("Overlapping text spans cannot be safely replaced")
    # Text replacement is true redaction with graphics/images explicitly ignored.
    # Original text positioning/font may not be reconstructible from subset fonts.
    rendered_data = None
    native_path = request.get("rendered_text_pdf_path")
    if native_path is not None:
        # REA: RDPDFKitCore 0x24a00c builds a temporary native-text PDF;
        # 0x247ca4 imports generated streams/resources with name substitutions.
        # The Swift adapter supplies a system-rendered PDF. Importing it as an
        # isolated Form XObject prevents collisions with the source's /F1 etc.
        rendered_data = read_regular(native_path, MAX_PDF)
        with fitz.open(stream=rendered_data, filetype="pdf") as native:
            safe_document(native)
            if len(native) != 1 or native[0].rect != page.rect:
                raise ValueError("Native text PDF geometry mismatch")
            replacement_spans = inspect_page(native[0])["spans"]
            if ("".join(s["text"] for s in replacement_spans) != text or not replacement_spans
                    or native[0].get_images() or native[0].get_drawings()
                    or native[0].get_links() or list(native[0].annots() or [])):
                raise ValueError("Native text PDF must contain only the exact searchable replacement")
            newrect = fitz.Rect(replacement_spans[0]["bbox"])
            for span in replacement_spans[1:]:
                newrect |= fitz.Rect(span["bbox"])
        fontname = "native-PDF-font-resources"
    elif all(32 <= ord(c) <= 126 for c in text):
        fontname = {"Helvetica":"helv","Helvetica-Bold":"hebo","Helvetica-Oblique":"heit",
                    "Times-Roman":"tiro","Times-Bold":"tibo","Courier":"cour","Courier-Bold":"cobo"}.get(target["font"],"helv")
    elif all(32 <= ord(c) <=126 or 0x3400 <= ord(c) <=0x9fff or 0x3000 <= ord(c) <=0x303f or 0xff00<=ord(c)<=0xffef for c in text):
        fontname = "china-s"
    else:
        raise ValueError("Unsupported replacement glyphs; only basic Latin and Chinese supported")
    size = target["size"]
    origin = fitz.Point(target["origin"])
    # Font()'s CJK metrics differ from Page.insert_text's built-in font metrics.
    # Measure the actual inserted searchable glyph run in a disposable document.
    if text and rendered_data is None:
        with fitz.open() as trial:
            trial_page = trial.new_page(width=14400,height=14400)
            trial_page.insert_text((100,size*3),text,fontname=fontname,fontsize=size)
            measured = inspect_page(trial_page)["spans"]
            if len(measured)!=1 or measured[0]["text"]!=text:
                raise ValueError("Replacement glyphs cannot be encoded without substitution")
            box = fitz.Rect(measured[0]["bbox"])
            newrect = fitz.Rect(origin.x+box.x0-100,origin.y+box.y0-size*3,
                                origin.x+box.x1-100,origin.y+box.y1-size*3)
    elif not text:
        newrect = rect
    # The original glyph bbox is not a clipping rectangle. FangSong and other
    # compact fonts often have smaller ascender/descender metrics than china-s,
    # so requiring identical vertical metrics rejects even shorter CJK edits.
    # Keep baseline/size and the original horizontal footprint; validate the
    # ACTUALLY inserted font box against the page and all neighboring objects.
    if newrect.x0 < rect.x0-0.1 or newrect.x1 > rect.x1+0.1 or not page.rect.contains(newrect):
        raise ValueError("Replacement does not fit original span; use shorter text (no implicit shrinking)")
    if any(s["id"] != target["id"] and newrect.intersects(fitz.Rect(s["bbox"])) for s in info["spans"]):
        raise ValueError("Replacement would overlap neighboring text")
    if any(newrect.intersects(fitz.Rect(link["bbox"])) for link in info["links"]):
        raise ValueError("Replacement would overlap an existing link")
    if any(newrect.intersects(annotation.rect) for annotation in page.annots() or []):
        raise ValueError("Replacement would overlap an existing annotation")
    expected = collections.Counter(span_signature(s) for s in info["spans"] if s["id"] != target["id"])
    original_images, original_graphics = image_signature(page),graphics_signature(page)
    page.add_redact_annot(rect,fill=False,cross_out=False)
    page.apply_redactions(images=0,graphics=0,text=0)
    color = tuple(((target["color"]>>shift)&255)/255 for shift in (16,8,0))
    if rendered_data is not None:
        with fitz.open(stream=rendered_data, filetype="pdf") as native:
            page.show_pdf_page(page.rect, native, 0, keep_proportion=False)
    elif text:
        page.insert_text(origin,text,fontname=fontname,fontsize=size,color=color)
    def verify(out):
        actual = inspect_page(out)["spans"]
        actual_counts = collections.Counter(span_signature(s) for s in actual)
        if expected - actual_counts:
            raise ValueError("Verification failed: neighboring text changed")
        region = "".join(s["text"] for s in actual if span_signature(s) not in expected)
        if region != text or image_signature(out) != original_images or graphics_signature(out) != original_graphics:
            raise ValueError("Verification failed: replaced text or preserved image/vector mismatch")
    return verify,[f"Replacement font: {fontname}; embedded/subset original font not reused; no paragraph reflow."]


def edit_document(doc, request):
    page_index = request.get("page")
    if type(page_index) != int or not 0<=page_index<doc.page_count:
        raise ValueError("Invalid page index")
    page = doc[page_index]
    info = inspect_page(page)
    op = request.get("operation")
    if op == "inspect":
        return info,None
    unchanged_pages = {p.number:page_fingerprint(p) for p in doc if p.number != page_index}
    verify = lambda out: None
    limitations = list(LIMITATIONS)
    if op == "replace-text":
        verify, extra = replace_text(page,info,request)
        limitations += extra
    elif op == "replace-image":
        image = selected(info["images"],request.get("id"))
        xref = image["xref"]
        occurrences = sum(i["xref"] == xref for p in doc for i in p.get_image_info(xrefs=True))
        if not xref or occurrences != 1:
            raise ValueError("Shared or inline image cannot be uniquely replaced")
        for obj in range(1,doc.xref_length()):
            if obj == xref:
                continue
            for key in ("SMask","Mask"):
                kind,value = doc.xref_get_key(obj,key)
                if kind in ("xref","array","dict") and re.search(r"(?<!\d)"+str(xref)+r"\s+0\s+R\b",value):
                    raise ValueError("Image is shared as another object's mask; replacement refused")
        image_data = read_regular(request.get("image_path",""),64*1024*1024)
        dimensions = check_image_header(image_data)
        pix = fitz.Pixmap(image_data)
        if (pix.width,pix.height)!=dimensions or pix.width*pix.height>40_000_000 or pix.n not in (1,3) or pix.alpha:
            raise ValueError("Image size/alpha/colorspace unsupported")
        old_text = collections.Counter(span_signature(s) for s in info["spans"])
        old_graphics = graphics_signature(page)
        old_images = collections.Counter(image_signature(page))
        normalized_bbox = tuple(round(v,3) for v in image["bbox"])
        old_target = next(i for i in old_images if i[0] == normalized_bbox)
        old_images[old_target] -= 1
        expected_digest = hashlib.md5(pix.samples).hexdigest()
        page.replace_image(xref,stream=image_data)
        def verify(out):
            if collections.Counter(span_signature(s) for s in inspect_page(out)["spans"]) != old_text or graphics_signature(out) != old_graphics:
                raise ValueError("Verification failed: unrelated text/vector changed")
            found = [i for i in out.get_image_info(hashes=True) if fitz.Rect(i["bbox"]) == fitz.Rect(image["bbox"])]
            if len(found)!=1 or found[0]["digest"].hex()!=expected_digest:
                raise ValueError("Verification failed: replacement image mismatch")
            actual_images = collections.Counter(image_signature(out))
            actual_images[(normalized_bbox,expected_digest)] -= 1
            if +actual_images != +old_images:
                raise ValueError("Verification failed: non-target image changed")
        limitations += ["Replacement retains original image transform/bbox; shared, inline and alpha images refused."]
    elif op in ("insert-link","update-link","delete-link"):
        old_text = page.get_text()
        if op == "insert-link":
            rect = checked_rect(request.get("bbox"),page)
            if any(rect.intersects(fitz.Rect(l["bbox"])) for l in info["links"]):
                raise ValueError("Link overlaps existing link")
            uri = safe_uri(request.get("uri"))
            page.insert_link({"kind":fitz.LINK_URI,"from":rect,"uri":uri})
        else:
            link_info = selected(info["links"],request.get("id"))
            link = next(x for x in page.get_links() if x.get("xref") == link_info["xref"])
            rect = link["from"]
            if op == "delete-link":
                page.delete_link(link)
            else:
                uri = safe_uri(request.get("uri"))
                link.update(kind=fitz.LINK_URI,uri=uri)
                page.update_link(link)
        def verify(out):
            links = [l for l in out.get_links() if l["from"] == rect]
            if out.get_text()!=old_text or (op=="delete-link" and links) or (op!="delete-link" and (len(links)!=1 or links[0].get("uri")!=uri)):
                raise ValueError("Verification failed: link mismatch or text changed")
    elif op == "redact":
        rect = checked_rect(request.get("bbox"),page)
        if list(page.annots() or []):
            raise ValueError("Redaction requires page without existing annotations")
        page.add_redact_annot(rect,fill=(0,0,0),cross_out=False)
        page.apply_redactions(images=1,graphics=2,text=0)
        limitations += ["Destructive redaction removes ANY overlapping image/vector object completely; text characters intersecting box are removed. Not a black overlay."]
        def verify(out):
            if any(rect.intersects(fitz.Rect(i["bbox"])) for i in out.get_image_info()):
                raise ValueError("Verification failed: image remains in redaction")
            # MuPDF excludes black fill drawing but real text must not survive.
            if out.get_text("text",clip=rect).strip():
                raise ValueError("Verification failed: text remains in redaction")
    else:
        raise ValueError("Unsupported operation")
    data = doc.tobytes(garbage=4,deflate=True,clean=True)
    if len(data)>MAX_PDF:
        raise ValueError("Edited output exceeds limit")
    with fitz.open(stream=data,filetype="pdf") as check:
        safe_document(check)
        verify(check[page_index])
        if check.page_count != doc.page_count:
            raise ValueError("Verification failed: page count changed")
        for index,fingerprint in unchanged_pages.items():
            if page_fingerprint(check[index]) != fingerprint:
                raise ValueError("Verification failed: unrelated page content changed")
    return {"message":"Real PDF content updated; source unchanged.","limitations":limitations},data


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--input",required=True)
    parser.add_argument("--request",required=True)
    parser.add_argument("--response",required=True)
    parser.add_argument("--output")
    args = parser.parse_args()
    response = None
    output = None
    published = False
    try:
        forbidden = {pathlib.Path(args.input).resolve(),pathlib.Path(args.request).resolve()}
        response = validate_output_path(args.response,forbidden)
        if args.output:
            output = validate_output_path(args.output,forbidden|{response.resolve()})
        request = json.loads(read_regular(args.request,1024*1024))
        if not isinstance(request,dict):
            raise ValueError("Request must be a JSON object")
        if request.get("operation") != "inspect" and output is None:
            raise ValueError("Editing requires a new output path")
        global fitz
        fitz = load_runtime()
        source_data = read_regular(args.input,MAX_PDF)
        with fitz.open(stream=source_data,filetype="pdf") as doc:
            safe_document(doc)
            result,data = edit_document(doc,request)
        if data is not None:
            publish_exclusive(output,data)
            published = True
        publish_exclusive(response,json.dumps(result,ensure_ascii=False,allow_nan=False).encode())
        return 0
    except Exception as error:
        if published:
            output.unlink()
        if response is not None and not response.exists():
            publish_exclusive(response,json.dumps({"error":str(error),"limitations":LIMITATIONS},ensure_ascii=False).encode())
        print("Content edit refused: " + str(error),file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
