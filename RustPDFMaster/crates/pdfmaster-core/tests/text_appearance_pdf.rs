use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use hayro::hayro_interpret::{InterpreterSettings, InterpreterWarning};
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings, render};
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object};
use pdfmaster_core::text_appearance::{
    BundledFont, TextAppearanceRequest, prepare_text_appearance,
};
use pdfmaster_core::{Color, TextAlignment};
use sha2::{Digest, Sha256};
use skrifa::raw::ps::cff::CffFontRef;

fn request(contents: &str) -> TextAppearanceRequest<'_> {
    TextAppearanceRequest {
        contents,
        width: 360.0,
        height: 240.0,
        font: BundledFont::Sans,
        font_size: 18.0,
        color: Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
        },
        alignment: TextAlignment::Left,
        line_height: 1.3,
    }
}

#[test]
fn carrier_has_linked_embedded_cff_fonts_unicode_maps_and_real_text() {
    let text = "Latin 中文 مرحبا بالعالم हैलो वर्ल्ड\noffice Café e\u{301} 𠮷";
    let prepared = prepare_text_appearance(&request(text)).unwrap();
    assert!(prepared.pdf_bytes().len() <= 8 * 1024 * 1024);
    assert!(prepared.metrics().glyph_count < text.chars().count());
    let document = Document::load_mem(prepared.pdf_bytes()).unwrap();
    let pages = document.get_pages();
    assert_eq!(pages.len(), 1);
    let page_id = pages[&1];
    let page = document.get_object(page_id).unwrap().as_dict().unwrap();
    let media_box = resolve(&document, page.get(b"MediaBox").unwrap())
        .as_array()
        .unwrap();
    assert_eq!(floats(media_box), vec![0.0, 0.0, 360.0, 240.0]);

    let resources = resolve(&document, page.get(b"Resources").unwrap())
        .as_dict()
        .unwrap();
    assert_no_image_resources(&document, resources);
    let fonts = resolve(&document, resources.get(b"Font").unwrap())
        .as_dict()
        .unwrap();
    assert_eq!(fonts.len(), 4);
    for (_, font) in fonts.iter() {
        assert_linked_embedded_cff_font(&document, font);
    }

    let content = decoded_page_content(&document, page_id);
    assert!(content.operations.iter().any(is_text_show));
    let actual_text = actual_text_values(&content.operations);
    assert!(!actual_text.is_empty());
    assert!(actual_text.iter().any(|value| value == "e\u{301}"));
}

#[test]
fn indic_actual_pdf_semantics_preserve_complete_logical_clusters() {
    for text in ["हैलो वर्ल्ड", "नमस्ते दुनिया"] {
        let prepared = prepare_text_appearance(&request(text)).unwrap();
        let (logical, _) = associated_pdf_text(prepared.pdf_bytes());
        assert_eq!(logical, text, "actual ToUnicode/ActualText association");
    }
}

#[test]
fn arabic_lam_alef_has_actual_multiscalar_mapping_without_reversing_source() {
    for text in ["لا", "بلا"] {
        let prepared = prepare_text_appearance(&request(text)).unwrap();
        let (logical, pieces) = associated_pdf_text(prepared.pdf_bytes());
        assert_eq!(logical, text);
        assert!(pieces.iter().any(|piece| piece == "لا"), "{pieces:?}");
    }
}

#[test]
fn formatting_combining_and_variation_source_is_not_silently_removed() {
    for text in [
        "Café e\u{301} office",
        "A\u{200d}B",
        "A\u{fe0f}B",
        "A\u{2067}بلا\u{2069}B",
    ] {
        let prepared = prepare_text_appearance(&request(text)).unwrap();
        let (logical, _) = associated_pdf_text(prepared.pdf_bytes());
        assert_eq!(logical, text, "source semantics must retain format scalars");
    }
}

// Interpret actual marked-content association, not an extractor's visual ordering.
// ReversedChars reverses semantic clusters, never Unicode scalars within a cluster.
fn associated_pdf_text(bytes: &[u8]) -> (String, Vec<String>) {
    let document = Document::load_mem(bytes).unwrap();
    let page_id = document.get_pages()[&1];
    let page = document.get_object(page_id).unwrap().as_dict().unwrap();
    let resources = resolve(&document, page.get(b"Resources").unwrap())
        .as_dict()
        .unwrap();
    let fonts = resolve(&document, resources.get(b"Font").unwrap())
        .as_dict()
        .unwrap();
    let mut font_name = Vec::new();
    let mut frames: Vec<(bool, bool, Vec<String>)> = vec![(false, false, Vec::new())];
    let mut pieces = Vec::new();
    for op in decoded_page_content(&document, page_id).operations {
        match op.operator.as_str() {
            "Tf" => font_name = op.operands[0].as_name().unwrap().to_vec(),
            "BMC" | "BDC" => {
                let reversed = op.operands[0].as_name().unwrap() == b"ReversedChars";
                let actual = actual_text_values(std::slice::from_ref(&op));
                let suppress = !actual.is_empty() || frames.last().unwrap().1;
                if !frames.last().unwrap().1 {
                    for value in &actual {
                        pieces.push(value.clone());
                    }
                }
                frames.push((reversed, suppress, actual));
            }
            "EMC" => {
                let (reversed, _, mut values) = frames.pop().unwrap();
                if reversed {
                    values.reverse();
                }
                if !frames.last().unwrap().1 {
                    frames.last_mut().unwrap().2.extend(values);
                }
            }
            "Tj" | "TJ" if !frames.last().unwrap().1 => {
                let font = resolve(&document, fonts.get(&font_name).unwrap())
                    .as_dict()
                    .unwrap();
                let encoding = font.get_font_encoding(&document).unwrap();
                let strings: Vec<&[u8]> = if op.operator == "TJ" {
                    op.operands[0]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(|v| v.as_str().ok())
                        .collect()
                } else {
                    vec![op.operands[0].as_str().unwrap()]
                };
                for string in strings {
                    for cid in string.chunks_exact(2) {
                        let value = encoding.bytes_to_string(cid).unwrap();
                        pieces.push(value.clone());
                        frames.last_mut().unwrap().2.push(value);
                    }
                }
            }
            _ => {}
        }
    }
    assert_eq!(frames.len(), 1);
    (frames.pop().unwrap().2.concat(), pieces)
}

#[test]
fn decoded_positions_match_metrics_for_all_physical_alignments_latin_and_rtl() {
    for text in ["physical alignment", "مرحبا بالعالم"] {
        for alignment in [
            TextAlignment::Left,
            TextAlignment::Center,
            TextAlignment::Right,
        ] {
            let mut input = request(text);
            input.alignment = alignment;
            let prepared = prepare_text_appearance(&input).unwrap();
            let line = &prepared.metrics().lines[0];
            let doc = Document::load_mem(prepared.pdf_bytes()).unwrap();
            let ops = decoded_page_content(&doc, doc.get_pages()[&1]).operations;
            let tm = ops.iter().find(|op| op.operator == "Tm").unwrap();
            let position = floats(&tm.operands);
            assert!(
                (position[4] - line.offset).abs() < 0.01,
                "{text} {alignment:?}: {position:?} {line:?}"
            );
            assert!(
                (position[5] - line.baseline).abs() < 0.01,
                "{position:?} {line:?}"
            );
            let expected = match alignment {
                TextAlignment::Left => 0.0,
                TextAlignment::Center => (input.width - line.advance) / 2.0,
                TextAlignment::Right => input.width - line.advance,
            };
            assert!((line.offset - expected).abs() < 0.01);
        }
    }
}

#[test]
fn mixed_paragraph_positions_and_pixels_include_blank_and_trailing_geometry() {
    let text = "first\r\n\r\nsecond\nthird\rfourth\r\n";
    let prepared = prepare_text_appearance(&request(text)).unwrap();
    let lines = &prepared.metrics().lines;
    assert_eq!(
        lines
            .iter()
            .map(|line| line.text_range.clone())
            .collect::<Vec<_>>(),
        vec![0..7, 7..9, 9..16, 16..22, 22..30, 30..30]
    );
    let doc = Document::load_mem(prepared.pdf_bytes()).unwrap();
    let baselines: Vec<_> = decoded_page_content(&doc, doc.get_pages()[&1])
        .operations
        .iter()
        .filter(|op| op.operator == "Tm")
        .map(|op| floats(&op.operands)[5])
        .collect();
    let visible = [0, 2, 3, 4];
    assert_eq!(baselines.len(), visible.len());
    let (pixels, bounds, warnings) = raw_raster(prepared.pdf_bytes());
    assert!(warnings.is_empty());
    assert!(bounds.4 > 100);
    for (baseline, index) in baselines.iter().zip(visible) {
        assert!((*baseline - lines[index].baseline).abs() < 0.01);
    }
    let ink_in_line = |index: usize| {
        let top = (index as f32 * 18.0 * 1.3).floor() as usize;
        let bottom = ((index + 1) as f32 * 18.0 * 1.3).floor() as usize;
        pixels
            .chunks_exact(4)
            .enumerate()
            .filter(|(i, p)| {
                let y = i / 360;
                y >= top && y < bottom && p[..3].iter().any(|c| *c < 245)
            })
            .count()
    };
    for index in visible {
        assert!(ink_in_line(index) > 30, "line {index}");
    }
    for index in [1, 5] {
        assert_eq!(ink_in_line(index), 0, "blank line {index}");
    }
    assert!((prepared.metrics().height - 6.0 * 18.0 * 1.3).abs() < 0.01);
}

#[test]
fn preserves_non_byte_rgb_without_changing_the_text_layout_or_other_operations() {
    let mut colored = request("中文 office مرحبا");
    let original = prepare_text_appearance(&colored).unwrap();
    colored.color = Color {
        r: 1. / 7.,
        g: 2. / 7.,
        b: 3. / 7.,
    };
    let result = prepare_text_appearance(&colored).unwrap();
    assert_eq!(result.metrics(), original.metrics());
    let document = Document::load_mem(result.pdf_bytes()).unwrap();
    let page = document.get_pages()[&1];
    let content = decoded_page_content(&document, page);
    let rgb = content
        .operations
        .iter()
        .find(|operation| operation.operator == "rg")
        .unwrap();
    assert_eq!(
        floats(&rgb.operands),
        vec![colored.color.r, colored.color.g, colored.color.b]
    );

    let original_document = Document::load_mem(original.pdf_bytes()).unwrap();
    let original_page = original_document.get_pages()[&1];
    let original_content = decoded_page_content(&original_document, original_page);
    assert_eq!(
        without_rgb(original_content.operations),
        without_rgb(content.operations)
    );
}

#[test]
fn repeated_and_concurrent_generation_has_equivalent_metrics_and_pdf_semantics() {
    let text = "repeat 中文 office مرحبا हैलो";
    let first = prepare_text_appearance(&request(text)).unwrap();
    let second = prepare_text_appearance(&request(text)).unwrap();
    assert_eq!(first.metrics(), second.metrics());
    assert_eq!(
        semantic_operations(first.pdf_bytes()),
        semantic_operations(second.pdf_bytes())
    );

    let workers: Vec<_> = (0..4)
        .map(|_| {
            std::thread::spawn(move || {
                let prepared = prepare_text_appearance(&request(text)).unwrap();
                (
                    prepared.metrics().clone(),
                    semantic_operations(prepared.pdf_bytes()),
                )
            })
        })
        .collect();
    for worker in workers {
        let (metrics, operations) = worker.join().unwrap();
        assert_eq!(&metrics, first.metrics());
        assert_eq!(operations, semantic_operations(first.pdf_bytes()));
    }
}

#[test]
fn bundled_fonts_licenses_notices_and_bundle_mappings_match_approved_provenance() {
    let assets: [(&[u8], usize, &str); 6] = [
        (
            include_bytes!("../assets/fonts/NotoSans-Regular.otf"),
            297_428,
            "7b8a545d63de82a3325dc3c545b597898c03219bd432b0a18086e7605859c6c4",
        ),
        (
            include_bytes!("../assets/fonts/NotoSansArabic-Regular.otf"),
            120_708,
            "ebd7f2bf3059b2927d945c77d9d023262a7e3553be67151882947db9dc8d594a",
        ),
        (
            include_bytes!("../assets/fonts/NotoSansDevanagari-Regular.otf"),
            109_504,
            "08cd6010bcf7ab779d43011ed6e6173f06eaf28814c58ce82e8e8cd6b07adda4",
        ),
        (
            include_bytes!("../assets/fonts/NotoSansCJKsc-Regular.otf"),
            16_437_364,
            "2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b",
        ),
        (
            include_bytes!("../assets/fonts/OFL-noto-fonts.txt"),
            4_377,
            "0dab92d0544f7b233403f14b84a663bdbfa746982eda629e7f4f9ffe1b036feb",
        ),
        (
            include_bytes!("../assets/fonts/OFL-noto-cjk.txt"),
            4_301,
            "6a73f9541c2de74158c0e7cf6b0a58ef774f5a780bf191f2d7ec9cc53efe2bf2",
        ),
    ];
    for (bytes, size, expected_sha) in assets {
        assert_eq!(bytes.len(), size);
        let actual_sha = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(actual_sha, expected_sha);
    }

    let notices = include_str!("../assets/fonts/FONT-NOTICES.md");
    for expected in [
        "ffebf8c1ee449e544955a7e813c54f9b73848eac",
        "f8d157532fbfaeda587e826d4cd5b21a49186f7c",
        "Copyright 2015-2021 Google LLC",
        "Copyright 2015-2022 Google Inc",
        "© 2014-2021 Adobe",
    ] {
        assert!(notices.contains(expected), "missing notice: {expected}");
    }
    let tauri = include_str!("../../../src-tauri/tauri.conf.json");
    for expected in [
        "third-party-fonts/OFL-noto-fonts.txt",
        "third-party-fonts/OFL-noto-cjk.txt",
        "third-party-fonts/FONT-NOTICES.md",
    ] {
        assert!(
            tauri.contains(expected),
            "missing bundle mapping: {expected}"
        );
    }
}

#[test]
fn raw_hayro_raster_is_nonblank_physically_aligned_stable_and_warning_free() {
    let mut left_request = request("中文 office مرحبا हैलो");
    left_request.width = 420.0;
    let left = prepare_text_appearance(&left_request).unwrap();
    let (left_pixels, left_bounds, left_warnings) = raw_raster(left.pdf_bytes());
    assert!(left_bounds.4 > 100, "ink pixels={}", left_bounds.4);
    assert!(left_warnings.is_empty(), "warnings={left_warnings:?}");

    let repeated = prepare_text_appearance(&left_request).unwrap();
    let (repeated_pixels, repeated_bounds, repeated_warnings) = raw_raster(repeated.pdf_bytes());
    assert_eq!(repeated_pixels, left_pixels);
    assert_eq!(repeated_bounds, left_bounds);
    assert!(repeated_warnings.is_empty());

    let mut right_request = left_request.clone();
    right_request.alignment = TextAlignment::Right;
    let right = prepare_text_appearance(&right_request).unwrap();
    let (_, right_bounds, right_warnings) = raw_raster(right.pdf_bytes());
    assert!(
        right_bounds.0 > left_bounds.0 + 100,
        "left={left_bounds:?} right={right_bounds:?}"
    );
    assert!(right_warnings.is_empty());

    let workers: Vec<_> = (0..4)
        .map(|_| {
            std::thread::spawn(|| {
                let mut request = request("中文 office مرحبا हैलो");
                request.width = 420.0;
                let prepared = prepare_text_appearance(&request).unwrap();
                raw_raster(prepared.pdf_bytes())
            })
        })
        .collect();
    for worker in workers {
        let (pixels, bounds, warnings) = worker.join().unwrap();
        assert_eq!(pixels, left_pixels);
        assert_eq!(bounds, left_bounds);
        assert!(warnings.is_empty());
    }
}

fn resolve<'a>(document: &'a Document, mut object: &'a Object) -> &'a Object {
    let mut seen = HashSet::new();
    for _ in 0..64 {
        let Object::Reference(id) = object else {
            return object;
        };
        assert!(seen.insert(*id));
        object = document.get_object(*id).unwrap();
    }
    panic!("reference depth exceeded");
}

fn decoded_page_content(document: &Document, page_id: (u32, u16)) -> Content<Vec<Operation>> {
    Content::decode_strict(&document.get_page_content(page_id)).unwrap()
}

fn floats(objects: &[Object]) -> Vec<f32> {
    objects
        .iter()
        .map(|object| object.as_float().unwrap())
        .collect()
}

fn is_text_show(operation: &Operation) -> bool {
    matches!(operation.operator.as_str(), "Tj" | "TJ" | "'" | "\"")
}

fn without_rgb(operations: Vec<Operation>) -> Vec<(String, Vec<Object>)> {
    operations
        .into_iter()
        .filter(|operation| operation.operator != "rg")
        .map(|operation| (operation.operator, operation.operands))
        .collect()
}

fn semantic_operations(pdf: &[u8]) -> Vec<(String, Vec<Object>)> {
    let document = Document::load_mem(pdf).unwrap();
    let page = document.get_pages()[&1];
    decoded_page_content(&document, page)
        .operations
        .into_iter()
        .map(|operation| (operation.operator, operation.operands))
        .collect()
}

fn actual_text_values(operations: &[Operation]) -> Vec<String> {
    let mut values = Vec::new();
    for operation in operations {
        if operation.operator != "BDC" {
            continue;
        }
        for operand in &operation.operands {
            let Ok(dictionary) = operand.as_dict() else {
                continue;
            };
            let Ok(Object::String(bytes, _)) = dictionary.get(b"ActualText") else {
                continue;
            };
            values.push(decode_pdf_text(bytes));
        }
    }
    values
}

fn decode_pdf_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xfe, 0xff]) {
        let units = rest
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]));
        String::from_utf16(units.collect::<Vec<_>>().as_slice()).unwrap()
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

fn assert_no_image_resources(document: &Document, resources: &Dictionary) {
    let Ok(xobjects) = resources.get(b"XObject") else {
        return;
    };
    for (_, object) in resolve(document, xobjects).as_dict().unwrap().iter() {
        let dictionary = resolve(document, object).as_stream().unwrap();
        assert_ne!(
            dictionary.dict.get(b"Subtype").unwrap().as_name().unwrap(),
            b"Image"
        );
    }
}

fn assert_linked_embedded_cff_font(document: &Document, object: &Object) {
    let type0 = resolve(document, object).as_dict().unwrap();
    assert_eq!(type0.get(b"Subtype").unwrap().as_name().unwrap(), b"Type0");
    assert_eq!(
        type0.get(b"Encoding").unwrap().as_name().unwrap(),
        b"Identity-H"
    );
    let to_unicode = resolve(document, type0.get(b"ToUnicode").unwrap())
        .as_stream()
        .unwrap();
    assert!(!to_unicode.decompressed_content().unwrap().is_empty());

    let descendants = resolve(document, type0.get(b"DescendantFonts").unwrap())
        .as_array()
        .unwrap();
    assert_eq!(descendants.len(), 1);
    let cid = resolve(document, &descendants[0]).as_dict().unwrap();
    assert_eq!(
        cid.get(b"Subtype").unwrap().as_name().unwrap(),
        b"CIDFontType0"
    );
    let descriptor = resolve(document, cid.get(b"FontDescriptor").unwrap())
        .as_dict()
        .unwrap();
    let font_file = resolve(document, descriptor.get(b"FontFile3").unwrap())
        .as_stream()
        .unwrap();
    assert_eq!(
        font_file.dict.get(b"Subtype").unwrap().as_name().unwrap(),
        b"CIDFontType0C"
    );
    let bytes = font_file.decompressed_content().unwrap();
    assert!(bytes.len() > 128);
    CffFontRef::new_cff(&bytes, 0, None).unwrap();
}

type InkBounds = (usize, usize, usize, usize, usize);

fn raw_raster(pdf_bytes: &[u8]) -> (Vec<u8>, InkBounds, Vec<InterpreterWarning>) {
    let pdf = Pdf::new(pdf_bytes.to_vec()).unwrap();
    let page = pdf.pages().first().unwrap();
    let warnings = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&warnings);
    let settings = InterpreterSettings {
        warning_sink: Arc::new(move |warning| captured.lock().unwrap().push(warning)),
        ..Default::default()
    };
    let pixmap = render(
        page,
        &RenderCache::new(),
        &settings,
        &RenderSettings::default(),
        &PixmapSettings {
            x_scale: 1.0,
            y_scale: 1.0,
            bg_color: WHITE,
        },
    );
    let width = usize::from(pixmap.width());
    let mut min_x = width;
    let mut min_y = usize::from(pixmap.height());
    let mut max_x = 0usize;
    let mut max_y = 0usize;
    let mut ink = 0usize;
    let pixels = pixmap.data_as_u8_slice().to_vec();
    for (index, pixel) in pixels.chunks_exact(4).enumerate() {
        if pixel[..3].iter().any(|channel| *channel < 245) {
            let x = index % width;
            let y = index / width;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
            ink += 1;
        }
    }
    drop(settings);
    let warnings = Arc::try_unwrap(warnings).unwrap().into_inner().unwrap();
    (pixels, (min_x, min_y, max_x, max_y, ink), warnings)
}
