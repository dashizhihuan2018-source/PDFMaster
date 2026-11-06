use std::io::Cursor;
use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};
use pdfmaster_core::PdfMasterError;
use skrifa::raw::ps::cff::CffFontRef;

const FIXTURES: [(&str, &[u8], &str); 3] = [
    (
        "chinese",
        include_bytes!("fixtures/unicode-ap/chinese.pdf"),
        "中文测试",
    ),
    (
        "latin",
        include_bytes!("fixtures/unicode-ap/latin.pdf"),
        "Café naïve",
    ),
    (
        "astral",
        include_bytes!("fixtures/unicode-ap/astral.pdf"),
        "𠮷",
    ),
];

#[test]
fn production_session_renders_embedded_unicode_appearances() {
    for (label, bytes, expected_contents) in FIXTURES {
        let session = pdfmaster_core::Session::open(bytes).unwrap();
        let before = session.bytes().unwrap();
        let before_page_contents = page_contents(&before);

        let png = session.render_png(0, 0.5).unwrap();
        assert!(!png.is_empty(), "{label}");
        let ink_pixels = decoded_ink_pixels(&png);
        assert!(ink_pixels > 10, "{label}: only {ink_pixels} ink pixels");
        assert_eq!(session.bytes().unwrap(), before, "{label}");

        let reopened = pdfmaster_core::Session::open(&before).unwrap();
        assert_eq!(
            page_contents(&reopened.bytes().unwrap()),
            before_page_contents
        );
        assert_eq!(reopened.render_png(0, 0.5).unwrap(), png, "{label}");
        assert_eq!(annotation_contents(&before), expected_contents, "{label}");
    }
}

#[test]
fn page_contents_snapshot_detects_referenced_stream_data_changes() {
    let before = page_contents(FIXTURES[0].1);
    let mut document = fixture_document();
    let page_id = *document.get_pages().values().next().unwrap();
    let contents = document
        .get_object(page_id)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Contents")
        .unwrap()
        .clone();
    let stream_id = match contents {
        Object::Reference(id) => id,
        Object::Array(values) => values
            .into_iter()
            .find_map(|value| value.as_reference().ok())
            .unwrap(),
        value => panic!("expected indirect page Contents, got {value:?}"),
    };
    let stream = document
        .get_object_mut(stream_id)
        .unwrap()
        .as_stream_mut()
        .unwrap();
    stream.content.extend_from_slice(b"\n% snapshot mutation\n");
    stream.dict.set("Length", stream.content.len() as i64);

    let after = page_contents(&save_document(document));
    assert_ne!(before, after);
}

fn decoded_ink_pixels(png: &[u8]) -> usize {
    let mut decoder = png::Decoder::new(Cursor::new(png));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().unwrap();
    let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buffer).unwrap();
    let bytes = &buffer[..info.buffer_size()];
    let channels = info.color_type.samples();
    bytes
        .chunks_exact(channels)
        .filter(|pixel| {
            pixel
                .iter()
                .take(3.min(channels))
                .any(|channel| *channel < 245)
        })
        .count()
}

fn page_contents(bytes: &[u8]) -> Vec<Stream> {
    let document = Document::load_mem(bytes).unwrap();
    let page_id = *document.get_pages().values().next().unwrap();
    let page = document.get_object(page_id).unwrap().as_dict().unwrap();
    let mut streams = Vec::new();
    collect_page_content_streams(&document, page.get(b"Contents").unwrap(), &mut streams);
    streams
}

fn collect_page_content_streams(document: &Document, object: &Object, streams: &mut Vec<Stream>) {
    match object {
        Object::Reference(id) => {
            collect_page_content_streams(document, document.get_object(*id).unwrap(), streams);
        }
        Object::Array(values) => {
            for value in values {
                collect_page_content_streams(document, value, streams);
            }
        }
        Object::Stream(stream) => streams.push(stream.clone()),
        value => panic!("page Contents must resolve to streams, got {value:?}"),
    }
}

fn annotation_contents(bytes: &[u8]) -> String {
    let document = Document::load_mem(bytes).unwrap();
    let page_id = *document.get_pages().values().next().unwrap();
    let page = document.get_object(page_id).unwrap().as_dict().unwrap();
    let annots = dereference(&document, page.get(b"Annots").unwrap())
        .as_array()
        .unwrap();
    let annotation = dereference(&document, annots.first().unwrap())
        .as_dict()
        .unwrap();
    let bytes = annotation.get(b"Contents").unwrap().as_str().unwrap();
    decode_pdf_text(bytes)
}

fn dereference<'a>(document: &'a Document, object: &'a Object) -> &'a Object {
    match object {
        Object::Reference(id) => document.get_object(*id).unwrap(),
        value => value,
    }
}

fn decode_pdf_text(bytes: &[u8]) -> String {
    assert!(bytes.starts_with(&[0xfe, 0xff]));
    let units = bytes[2..]
        .chunks_exact(2)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]));
    char::decode_utf16(units)
        .collect::<std::result::Result<String, _>>()
        .unwrap()
}

fn fixture_document() -> Document {
    Document::load_mem(FIXTURES[0].1).unwrap()
}

fn save_document(mut document: Document) -> Vec<u8> {
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

fn assert_render_error_preserves_session(bytes: &[u8], expected: &str) {
    let mut session = pdfmaster_core::Session::open(bytes).unwrap();
    let before = session.bytes().unwrap();
    let error = session.render_png(0, 0.5).unwrap_err();
    let rendered = error.to_string();
    assert!(
        rendered.contains(expected),
        "expected error containing {expected:?}, got {rendered:?}"
    );
    assert_eq!(session.bytes().unwrap(), before);
    assert!(matches!(session.undo(), Err(PdfMasterError::NoUndo)));
    assert!(matches!(session.redo(), Err(PdfMasterError::NoRedo)));
    assert_eq!(session.bytes().unwrap(), before);
}

fn find_dictionary_id(document: &Document, subtype: &[u8]) -> ObjectId {
    document
        .objects
        .iter()
        .find_map(|(id, object)| {
            let dictionary = object.as_dict().ok()?;
            (dictionary.get(b"Subtype").and_then(Object::as_name).ok() == Some(subtype))
                .then_some(*id)
        })
        .unwrap()
}

fn find_stream_id(document: &Document, predicate: impl Fn(&Stream) -> bool) -> ObjectId {
    document
        .objects
        .iter()
        .find_map(|(id, object)| {
            object
                .as_stream()
                .ok()
                .filter(|stream| predicate(stream))
                .map(|_| *id)
        })
        .unwrap()
}

fn type0_font_id(document: &Document) -> ObjectId {
    find_dictionary_id(document, b"Type0")
}

fn descendant_font_id(document: &Document) -> ObjectId {
    find_dictionary_id(document, b"CIDFontType0")
}

fn descriptor_id(document: &Document) -> ObjectId {
    document
        .objects
        .iter()
        .find_map(|(id, object)| {
            let dictionary = object.as_dict().ok()?;
            (dictionary.get(b"Type").and_then(Object::as_name).ok() == Some(b"FontDescriptor"))
                .then_some(*id)
        })
        .unwrap()
}

fn cff_stream_id(document: &Document) -> ObjectId {
    find_stream_id(document, |stream| {
        stream.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"CIDFontType0C")
    })
}

fn cmap_stream_id(document: &Document) -> ObjectId {
    find_stream_id(document, |stream| {
        stream
            .decompressed_content()
            .is_ok_and(|bytes| bytes.windows(9).any(|window| window == b"begincmap"))
    })
}

fn appearance_stream_id(document: &Document) -> ObjectId {
    find_stream_id(document, |stream| {
        stream.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Form")
            && stream
                .decompressed_content()
                .is_ok_and(|bytes| bytes.windows(2).any(|window| window == b"Tf"))
    })
}

fn set_plain_stream(document: &mut Document, id: ObjectId, bytes: Vec<u8>) {
    let stream = document
        .objects
        .get_mut(&id)
        .unwrap()
        .as_stream_mut()
        .unwrap();
    stream.set_content(bytes);
    stream.dict.remove(b"Filter");
    stream.dict.remove(b"DecodeParms");
}

fn mutate_cmap(mutator: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut document = fixture_document();
    let id = cmap_stream_id(&document);
    let mut bytes = document
        .objects
        .get(&id)
        .unwrap()
        .as_stream()
        .unwrap()
        .decompressed_content()
        .unwrap();
    mutator(&mut bytes);
    set_plain_stream(&mut document, id, bytes);
    save_document(document)
}

fn mutate_cff(mutator: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut document = fixture_document();
    let id = cff_stream_id(&document);
    let mut bytes = document
        .objects
        .get(&id)
        .unwrap()
        .as_stream()
        .unwrap()
        .decompressed_content()
        .unwrap();
    mutator(&mut bytes);
    set_plain_stream(&mut document, id, bytes);
    save_document(document)
}

fn replace_ascii_once(bytes: &mut [u8], from: &[u8], to: &[u8]) {
    assert_eq!(from.len(), to.len());
    let offset = bytes
        .windows(from.len())
        .position(|window| window == from)
        .unwrap();
    bytes[offset..offset + from.len()].copy_from_slice(to);
}

fn appearance_font_resources(document: &Document) -> (Vec<u8>, Dictionary) {
    let form = document
        .objects
        .get(&appearance_stream_id(document))
        .unwrap()
        .as_stream()
        .unwrap();
    let resources = dereference(document, form.dict.get(b"Resources").unwrap())
        .as_dict()
        .unwrap();
    let fonts = dereference(document, resources.get(b"Font").unwrap())
        .as_dict()
        .unwrap()
        .clone();
    let name = fonts.iter().next().unwrap().0.clone();
    (name, fonts)
}

fn set_appearance_content(document: &mut Document, content: Vec<u8>, extra_standard: bool) {
    let form_id = appearance_stream_id(document);
    if extra_standard {
        let (type0_name, mut fonts) = appearance_font_resources(document);
        assert_ne!(type0_name, b"Std");
        fonts.set(
            "Std",
            dictionary! {
                "Type" => "Font",
                "Subtype" => "Type1",
                "BaseFont" => "Helvetica",
                "Encoding" => "WinAnsiEncoding",
            },
        );
        let form = document
            .objects
            .get_mut(&form_id)
            .unwrap()
            .as_stream_mut()
            .unwrap();
        let mut resources = Dictionary::new();
        resources.set("Font", fonts);
        resources.set(
            "ProcSet",
            vec![
                Object::Name(b"PDF".to_vec()),
                Object::Name(b"Text".to_vec()),
            ],
        );
        form.dict.set("Resources", resources);
    }
    set_plain_stream(document, form_id, content);
}

fn type0_font_name(document: &Document) -> Vec<u8> {
    appearance_font_resources(document).0
}

fn text_program(font: &[u8], show: &[u8]) -> Vec<u8> {
    let mut content = Vec::new();
    content.extend_from_slice(b"q BT /");
    content.extend_from_slice(font);
    content.extend_from_slice(b" 12 Tf 10 10 Td ");
    content.extend_from_slice(show);
    content.extend_from_slice(b" ET Q\n");
    content
}

fn compress(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

fn replace_type0_resource(document: &mut Document, replacement: Object) {
    let target = type0_font_id(document);
    let form_id = appearance_stream_id(document);
    let form = document.objects.get(&form_id).unwrap().as_stream().unwrap();
    let mut resources = dereference(document, form.dict.get(b"Resources").unwrap())
        .as_dict()
        .unwrap()
        .clone();
    let mut fonts = dereference(document, resources.get(b"Font").unwrap())
        .as_dict()
        .unwrap()
        .clone();
    let value = fonts
        .iter_mut()
        .find_map(|(_, value)| {
            matches!(value, Object::Reference(id) if *id == target).then_some(value)
        })
        .expect("Type0 font resource link not found");
    *value = replacement;
    resources.set("Font", fonts);
    document
        .objects
        .get_mut(&form_id)
        .unwrap()
        .as_stream_mut()
        .unwrap()
        .dict
        .set("Resources", resources);
}

fn insert_object(document: &mut Document, object: Object) -> ObjectId {
    let id = document.new_object_id();
    document.objects.insert(id, object);
    id
}

fn rewrite_object_length(mut bytes: Vec<u8>, id: ObjectId) -> Vec<u8> {
    let marker = format!("{} {} obj", id.0, id.1);
    let object_start = bytes
        .windows(marker.len())
        .position(|window| window == marker.as_bytes())
        .unwrap();
    let stream_offset = bytes[object_start..]
        .windows(b"stream".len())
        .position(|window| window == b"stream")
        .map(|offset| object_start + offset)
        .unwrap();
    let length_marker = b"/Length ";
    let length_start = bytes[object_start..stream_offset]
        .windows(length_marker.len())
        .position(|window| window == length_marker)
        .map(|offset| object_start + offset + length_marker.len())
        .unwrap();
    let length_end = (length_start..stream_offset)
        .find(|index| !bytes[*index].is_ascii_digit())
        .unwrap();
    let last = length_end - 1;
    bytes[last] = if bytes[last] == b'9' {
        b'8'
    } else {
        bytes[last] + 1
    };
    bytes
}

#[test]
fn rejects_font_graph_cycle_missing_depth_and_object_budget() {
    let mut cycle = fixture_document();
    let first = insert_object(&mut cycle, Object::Null);
    let second = insert_object(&mut cycle, Object::Reference(first));
    cycle.objects.insert(first, Object::Reference(second));
    replace_type0_resource(&mut cycle, Object::Reference(first));
    assert_render_error_preserves_session(
        &save_document(cycle),
        "font graph contains an indirect reference cycle",
    );

    let mut missing = fixture_document();
    replace_type0_resource(&mut missing, Object::Reference((u32::MAX - 1, 0)));
    assert_render_error_preserves_session(&save_document(missing), "font graph reference");

    let mut deep = fixture_document();
    let mut next = type0_font_id(&deep);
    for _ in 0..65 {
        next = insert_object(&mut deep, Object::Reference(next));
    }
    replace_type0_resource(&mut deep, Object::Reference(next));
    assert_render_error_preserves_session(&save_document(deep), "font graph depth");

    let mut over_objects = fixture_document();
    let mut references = Vec::new();
    for value in 0..4_097i64 {
        let id = insert_object(&mut over_objects, Object::Integer(value));
        references.push(Object::Reference(id));
    }
    let type0 = type0_font_id(&over_objects);
    over_objects
        .objects
        .get_mut(&type0)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Name", references);
    assert_render_error_preserves_session(&save_document(over_objects), "font graph");
}

#[test]
fn rejects_unknown_font_fields_stream_filters_lengths_and_descriptor_values() {
    let mut unknown = fixture_document();
    let type0 = type0_font_id(&unknown);
    unknown
        .objects
        .get_mut(&type0)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("UnknownProfileField", true);
    assert_render_error_preserves_session(
        &save_document(unknown),
        "Type0 Font field UnknownProfileField is unsupported",
    );

    let mut decode_parms = fixture_document();
    let cff = cff_stream_id(&decode_parms);
    decode_parms
        .objects
        .get_mut(&cff)
        .unwrap()
        .as_stream_mut()
        .unwrap()
        .dict
        .set("DecodeParms", Dictionary::new());
    assert_render_error_preserves_session(
        &save_document(decode_parms),
        "FontFile3 stream field DecodeParms is unsupported",
    );

    let mut filters = fixture_document();
    let cff = cff_stream_id(&filters);
    filters
        .objects
        .get_mut(&cff)
        .unwrap()
        .as_stream_mut()
        .unwrap()
        .dict
        .set(
            "Filter",
            vec![
                Object::Name(b"FlateDecode".to_vec()),
                Object::Name(b"FlateDecode".to_vec()),
            ],
        );
    assert_render_error_preserves_session(
        &save_document(filters),
        "stream Filter must be a name or one-element array",
    );

    let length_document = fixture_document();
    let cff = cff_stream_id(&length_document);
    let wrong_length = rewrite_object_length(save_document(length_document), cff);
    assert!(
        matches!(pdfmaster_core::Session::open(&wrong_length), Err(PdfMasterError::Malformed(message)) if message.contains("invalid indirect object"))
    );

    let mut descriptor = fixture_document();
    let id = descriptor_id(&descriptor);
    descriptor
        .objects
        .get_mut(&id)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Flags", -1i64);
    assert_render_error_preserves_session(
        &save_document(descriptor),
        "FontDescriptor Flags must be a non-negative u32",
    );
}

#[test]
fn rejects_cmap_counts_unknown_tokens_duplicates_utf16_and_missing_cids() {
    let bad_count =
        mutate_cmap(|bytes| replace_ascii_once(bytes, b"4 beginbfchar", b"3 beginbfchar"));
    assert_render_error_preserves_session(&bad_count, "ToUnicode CMap audit failed");

    let unknown = mutate_cmap(|bytes| {
        let marker = b"1 begincodespacerange";
        let offset = bytes
            .windows(marker.len())
            .position(|window| window == marker)
            .unwrap();
        bytes.splice(offset..offset, b"/UnknownProfile 1 def\n".iter().copied());
    });
    assert_render_error_preserves_session(&unknown, "unknown or missing CMap header declaration");

    let duplicate =
        mutate_cmap(|bytes| replace_ascii_once(bytes, b"<0002> <6587>", b"<0001> <6587>"));
    assert_render_error_preserves_session(&duplicate, "source CID 1 is duplicated");

    let malformed_utf16 = mutate_cmap(|bytes| replace_ascii_once(bytes, b"<4E2D>", b"<D800>"));
    assert_render_error_preserves_session(&malformed_utf16, "target contains malformed UTF-16");

    let absent_cid =
        mutate_cmap(|bytes| replace_ascii_once(bytes, b"<0001> <4E2D>", b"<FFFF> <4E2D>"));
    assert_render_error_preserves_session(
        &absent_cid,
        "source CID 65535 is absent from CFF charset",
    );
}

#[test]
fn rejects_cff_truncation_noncid_bad_charstring_and_stream_budgets() {
    let truncated = mutate_cff(|bytes| bytes.truncate(16));
    assert_render_error_preserves_session(&truncated, "CFF parse failed");

    let noncid = mutate_cff(|bytes| *bytes = minimal_noncid_cff());
    assert_render_error_preserves_session(&noncid, "CFF font must be CID-keyed");

    let bad_charstring = mutate_cff(|bytes| {
        let font = CffFontRef::new_cff(bytes, 0, None).unwrap();
        let charstring = font.charstrings().get(1).unwrap();
        let offset = charstring.as_ptr() as usize - bytes.as_ptr() as usize;
        let length = charstring.len();
        assert!(length > 0);
        bytes[offset..offset + length].fill(0);
        bytes[offset + length - 1] = 255; // a truncated 32-bit charstring operand
        let font = CffFontRef::new_cff(bytes, 0, None).unwrap();
        let subfont = font
            .subfont(font.subfont_index(skrifa::GlyphId::new(1)).unwrap(), &[])
            .unwrap();
        struct NullPen;
        impl skrifa::outline::OutlinePen for NullPen {
            fn move_to(&mut self, _: f32, _: f32) {}
            fn line_to(&mut self, _: f32, _: f32) {}
            fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
            fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
            fn close(&mut self) {}
        }
        assert!(
            font.draw(
                &subfont,
                skrifa::GlyphId::new(1),
                &[],
                Some(1000.0),
                &mut NullPen
            )
            .is_err()
        );
    });
    assert_render_error_preserves_session(&bad_charstring, "CFF charstring draw failed");

    let mut raw_budget = fixture_document();
    let id = cff_stream_id(&raw_budget);
    set_plain_stream(&mut raw_budget, id, vec![0; 8 * 1024 * 1024 + 1]);
    assert_render_error_preserves_session(&save_document(raw_budget), "CFF raw bytes");

    let mut decoded_budget = fixture_document();
    let id = cff_stream_id(&decoded_budget);
    let oversized = vec![0; 16 * 1024 * 1024 + 1];
    let stream = decoded_budget
        .objects
        .get_mut(&id)
        .unwrap()
        .as_stream_mut()
        .unwrap();
    stream.set_content(compress(&oversized));
    stream.dict.set("Filter", "FlateDecode");
    assert_render_error_preserves_session(&save_document(decoded_budget), "CFF decoded bytes");
}

#[test]
fn rejects_cmap_cidset_and_cumulative_source_budgets() {
    let mut raw_cmap = fixture_document();
    let cmap = cmap_stream_id(&raw_cmap);
    let mut cmap_bytes = raw_cmap
        .objects
        .get(&cmap)
        .unwrap()
        .as_stream()
        .unwrap()
        .decompressed_content()
        .unwrap();
    cmap_bytes.resize(1024 * 1024 + 1, b' ');
    set_plain_stream(&mut raw_cmap, cmap, cmap_bytes);
    assert_render_error_preserves_session(&save_document(raw_cmap), "CMap raw bytes");

    let mut decoded_cmap = fixture_document();
    let cmap = cmap_stream_id(&decoded_cmap);
    let mut cmap_bytes = decoded_cmap
        .objects
        .get(&cmap)
        .unwrap()
        .as_stream()
        .unwrap()
        .decompressed_content()
        .unwrap();
    cmap_bytes.resize(1024 * 1024 + 1, b' ');
    let stream = decoded_cmap
        .objects
        .get_mut(&cmap)
        .unwrap()
        .as_stream_mut()
        .unwrap();
    stream.set_content(compress(&cmap_bytes));
    stream.dict.set("Filter", "FlateDecode");
    assert_render_error_preserves_session(&save_document(decoded_cmap), "CMap decoded bytes");

    let mut cidset = fixture_document();
    let cidset_id = insert_object(
        &mut cidset,
        Object::Stream(Stream::new(
            dictionary! { "Filter" => "FlateDecode" },
            compress(&vec![0xff; 8_193]),
        )),
    );
    let descriptor = descriptor_id(&cidset);
    cidset
        .objects
        .get_mut(&descriptor)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("CIDSet", Object::Reference(cidset_id));
    assert_render_error_preserves_session(&save_document(cidset), "CIDSet decoded bytes");

    let cumulative = document_with_distinct_large_font_streams();
    assert_render_error_preserves_session(&cumulative, "source resource bytes");
}

#[test]
fn validates_identity_h_bytes_for_every_text_show_operator() {
    for (show, operator) in [
        (b"<01> Tj".as_slice(), "operator Tj Identity-H string"),
        (b"[<01> 0] TJ".as_slice(), "operator TJ Identity-H string"),
        (b"<01> '".as_slice(), "operator ' Identity-H string"),
        (b"0 0 <01> \"".as_slice(), "operator \" Identity-H string"),
    ] {
        let mut document = fixture_document();
        let font = type0_font_name(&document);
        set_appearance_content(&mut document, text_program(&font, show), false);
        assert_render_error_preserves_session(&save_document(document), operator);
    }

    for (show, operator) in [
        (b"<FFFF> Tj".as_slice(), "operator Tj Identity-H CID 65535"),
        (
            b"[<FFFF> 0] TJ".as_slice(),
            "operator TJ Identity-H CID 65535",
        ),
        (b"<FFFF> '".as_slice(), "operator ' Identity-H CID 65535"),
        (
            b"0 0 <FFFF> \"".as_slice(),
            "operator \" Identity-H CID 65535",
        ),
    ] {
        let mut document = fixture_document();
        let font = type0_font_name(&document);
        set_appearance_content(&mut document, text_program(&font, show), false);
        assert_render_error_preserves_session(&save_document(document), operator);
    }
}

#[test]
fn graphics_restore_restores_the_selected_type0_font_identity() {
    let mut positive = fixture_document();
    let font = type0_font_name(&positive);
    let mut content = Vec::new();
    content.extend_from_slice(b"q BT /");
    content.extend_from_slice(&font);
    content.extend_from_slice(b" 12 Tf q /Std 12 Tf Q <0001> Tj ET Q\n");
    set_appearance_content(&mut positive, content, true);
    let positive = save_document(positive);
    let session = pdfmaster_core::Session::open(&positive).unwrap();
    assert!(!session.render_png(0, 0.5).unwrap().is_empty());
    assert_eq!(session.bytes().unwrap(), positive);

    let mut negative = fixture_document();
    let font = type0_font_name(&negative);
    let mut content = Vec::new();
    content.extend_from_slice(b"q BT /");
    content.extend_from_slice(&font);
    content.extend_from_slice(b" 12 Tf q /Std 12 Tf Q <01> Tj ET Q\n");
    set_appearance_content(&mut negative, content, true);
    assert_render_error_preserves_session(
        &save_document(negative),
        "operator Tj Identity-H string",
    );
}

#[test]
fn reuses_one_font_dag_across_page_and_annotation_appearance() {
    let mut document = fixture_document();
    let type0 = type0_font_id(&document);
    let page_id = *document.get_pages().values().next().unwrap();
    let content_id = insert_object(
        &mut document,
        Object::Stream(Stream::new(
            Dictionary::new(),
            text_program(b"Shared", b"<0001> Tj"),
        )),
    );
    let page = document
        .objects
        .get_mut(&page_id)
        .unwrap()
        .as_dict_mut()
        .unwrap();
    page.set("Contents", Object::Reference(content_id));
    page.set(
        "Resources",
        dictionary! {
            "Font" => dictionary! { "Shared" => Object::Reference(type0) },
            "ProcSet" => vec![Object::Name(b"PDF".to_vec()), Object::Name(b"Text".to_vec())],
        },
    );
    let bytes = save_document(document);
    let session = pdfmaster_core::Session::open(&bytes).unwrap();
    assert!(!session.render_png(0, 0.5).unwrap().is_empty());
    assert_eq!(session.bytes().unwrap(), bytes);
}

fn minimal_noncid_cff() -> Vec<u8> {
    vec![
        1, 0, 4, 4, // header
        0, 1, 1, 1, 2, b'A', // Name INDEX
        0, 1, 1, 1, 3, 160, 17, // Top DICT INDEX: CharStrings at byte 21
        0, 0, // String INDEX
        0, 0, // Global Subr INDEX
        0, 1, 1, 1, 2, 14, // one endchar CharString
    ]
}

#[test]
fn generated_noncid_cff_reaches_the_noncid_gate() {
    let bytes = minimal_noncid_cff();
    let font = CffFontRef::new_cff(&bytes, 0, None).unwrap();
    assert!(!font.is_cid());
    assert_eq!(font.num_glyphs(), 1);
}

fn document_with_distinct_large_font_streams() -> Vec<u8> {
    let mut document = fixture_document();
    let original_type0 = type0_font_id(&document);
    let original_descendant = descendant_font_id(&document);
    let original_descriptor = descriptor_id(&document);
    let original_cff = cff_stream_id(&document);
    let mut decoded = document
        .objects
        .get(&original_cff)
        .unwrap()
        .as_stream()
        .unwrap()
        .decompressed_content()
        .unwrap();
    decoded.resize(16 * 1024 * 1024, 0);

    let mut fonts = Dictionary::new();
    for index in 0..4 {
        let cff = insert_object(
            &mut document,
            Object::Stream(Stream::new(
                dictionary! {
                    "Filter" => "FlateDecode",
                    "Subtype" => "CIDFontType0C",
                },
                compress(&decoded),
            )),
        );
        let mut descriptor = document
            .objects
            .get(&original_descriptor)
            .unwrap()
            .as_dict()
            .unwrap()
            .clone();
        descriptor.set("FontFile3", Object::Reference(cff));
        let descriptor = insert_object(&mut document, Object::Dictionary(descriptor));

        let mut descendant = document
            .objects
            .get(&original_descendant)
            .unwrap()
            .as_dict()
            .unwrap()
            .clone();
        descendant.set("FontDescriptor", Object::Reference(descriptor));
        let descendant = insert_object(&mut document, Object::Dictionary(descendant));

        let mut type0 = document
            .objects
            .get(&original_type0)
            .unwrap()
            .as_dict()
            .unwrap()
            .clone();
        type0.set("DescendantFonts", vec![Object::Reference(descendant)]);
        let type0 = insert_object(&mut document, Object::Dictionary(type0));
        fonts.set(format!("Large{index}"), Object::Reference(type0));
    }
    let form_id = appearance_stream_id(&document);
    let form = document
        .objects
        .get_mut(&form_id)
        .unwrap()
        .as_stream_mut()
        .unwrap();
    let mut resources = Dictionary::new();
    resources.set("Font", fonts);
    resources.set(
        "ProcSet",
        vec![
            Object::Name(b"PDF".to_vec()),
            Object::Name(b"Text".to_vec()),
        ],
    );
    form.dict.set("Resources", resources);
    form.set_content(text_program(b"Large0", b"<0001> Tj"));
    form.dict.remove(b"Filter");
    save_document(document)
}

#[test]
fn render_preserves_nonempty_undo_and_redo_on_success_and_failure() {
    let bad = mutate_cmap(|bytes| replace_ascii_once(bytes, b"<4E2D>", b"<D800>"));
    for (bytes, succeeds) in [(FIXTURES[0].1, true), (bad.as_slice(), false)] {
        let mut session = pdfmaster_core::Session::open(bytes).unwrap();
        let original = session.bytes().unwrap();
        session
            .apply(pdfmaster_core::Edit::Rotate {
                page: 0,
                degrees: 90,
            })
            .unwrap();
        let first = session.bytes().unwrap();
        session
            .apply(pdfmaster_core::Edit::Rotate {
                page: 0,
                degrees: 90,
            })
            .unwrap();
        let second = session.bytes().unwrap();
        session.undo().unwrap();
        assert_eq!(session.bytes().unwrap(), first);
        assert_eq!(session.render_png(0, 0.5).is_ok(), succeeds);
        assert_eq!(session.bytes().unwrap(), first);
        session.redo().unwrap();
        assert_eq!(session.bytes().unwrap(), second);
        session.undo().unwrap();
        assert_eq!(session.bytes().unwrap(), first);
        session.undo().unwrap();
        assert_eq!(session.bytes().unwrap(), original);
        session.redo().unwrap();
        assert_eq!(session.bytes().unwrap(), first);
        session.redo().unwrap();
        assert_eq!(session.bytes().unwrap(), second);
    }
}

fn cmap_program(mappings: &str, header: &str) -> Vec<u8> {
    format!("/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n{header}\n/CMapName /Test def /CMapType 2 def\n1 begincodespacerange <0000> <FFFF> endcodespacerange\n{mappings}\nendcmap CMapName currentdict /CMap defineresource pop end end").into_bytes()
}

const CSI_HEADER: &str =
    "/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> def";

#[test]
fn rejects_actual_malformed_charset_and_fdselect() {
    use skrifa::raw::{
        FontRead,
        ps::cff::{dict, v1::Cff},
    };
    let charset = mutate_cff(|bytes| {
        let cff = Cff::read(bytes.as_slice().into()).unwrap();
        let offset = dict::entries(cff.top_dicts().get(0).unwrap(), None)
            .find_map(|e| match e.unwrap() {
                dict::Entry::Charset(o) => Some(o),
                _ => None,
            })
            .unwrap();
        bytes[offset] = 255;
        assert!(
            CffFontRef::new_cff(bytes, 0, None)
                .unwrap()
                .charset()
                .is_none()
        );
    });
    assert_render_error_preserves_session(&charset, "CFF charset");
    let fdselect = mutate_cff(|bytes| {
        let cff = Cff::read(bytes.as_slice().into()).unwrap();
        let offset = dict::entries(cff.top_dicts().get(0).unwrap(), None)
            .find_map(|e| match e.unwrap() {
                dict::Entry::FdSelectOffset(o) => Some(o),
                _ => None,
            })
            .unwrap();
        let field = match bytes[offset] {
            0 => offset + 1,
            3 => offset + 5,
            n => panic!("unexpected FDSelect {n}"),
        };
        bytes[field] = 255;
        let font = CffFontRef::new_cff(bytes, 0, None).unwrap();
        assert!(font.subfont_index(skrifa::GlyphId::new(0)).unwrap() >= font.num_subfonts());
    });
    assert_render_error_preserves_session(&fdselect, "CFF subfont index");
}

#[test]
fn accepts_declarative_cmap_variations_and_both_range_targets() {
    for mappings in [
        "1 beginbfrange <0001> <0004> <0041> endbfrange",
        "1 beginbfrange <0001> <0004> [<D840DC00> <D840DC01> <00660069> <4E2D>] endbfrange",
        "2 beginbfrange <0001> <0002> <D840DFFE> <0003> <0004> [<0066> <00660069>] endbfrange",
    ] {
        let bytes = mutate_cmap(|bytes| {
            *bytes = cmap_program(
                mappings,
                &format!("% flexible header\n{CSI_HEADER}\n/CMapVersion 1.2 def /WMode 0 def"),
            )
        });
        let session = pdfmaster_core::Session::open(&bytes).unwrap();
        assert!(!session.render_png(0, 0.5).unwrap().is_empty());
    }
}

#[test]
fn rejects_retained_cff_cid_missing_from_tounicode_at_text_show() {
    let bytes = mutate_cmap(|bytes| {
        *bytes = cmap_program("1 beginbfchar <0001> <4E2D> endbfchar", CSI_HEADER)
    });
    assert_render_error_preserves_session(&bytes, "operator TJ Identity-H CID 2");
}

#[test]
fn rejects_cmap_token_size_count_and_incremented_surrogate_boundary() {
    let oversized = mutate_cmap(|bytes| {
        *bytes = cmap_program(
            "1 beginbfchar <0001> <4E2D> endbfchar",
            &format!("{CSI_HEADER}\n/{} 0 def", "x".repeat(257)),
        )
    });
    assert_render_error_preserves_session(&oversized, "CMap token bytes");
    let tokens = mutate_cmap(|bytes| *bytes = b"x ".repeat(65_537));
    assert_render_error_preserves_session(&tokens, "CMap token count");
    let boundary = mutate_cmap(|bytes| {
        *bytes = cmap_program(
            "1 beginbfrange <0001> <0002> <D840DFFF> endbfrange",
            CSI_HEADER,
        )
    });
    assert_render_error_preserves_session(&boundary, "target contains malformed UTF-16");
}
