use std::sync::{Arc, Mutex};

use hayro::hayro_interpret::{InterpreterSettings, InterpreterWarning};
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings, render};
use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, StringFormat, dictionary};
use pdfmaster_core::text_appearance::{
    BundledFont, TextAppearanceRequest, prepare_text_appearance,
};
use pdfmaster_core::{Color, PdfMasterError, Session, TextAlignment};

fn carrier(contents: &str, alignment: TextAlignment) -> Vec<u8> {
    prepare_text_appearance(&TextAppearanceRequest {
        contents,
        width: 360.,
        height: 240.,
        font: BundledFont::Sans,
        font_size: 18.,
        color: Color {
            r: 1. / 7.,
            g: 2. / 7.,
            b: 3. / 7.,
        },
        alignment,
        line_height: 1.3,
    })
    .unwrap()
    .pdf_bytes()
    .to_vec()
}

fn raw_raster(bytes: &[u8]) -> (Vec<u8>, usize, Vec<InterpreterWarning>) {
    let pdf = Pdf::new(bytes.to_vec()).unwrap();
    let warnings = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&warnings);
    let settings = InterpreterSettings {
        warning_sink: Arc::new(move |warning| captured.lock().unwrap().push(warning)),
        ..Default::default()
    };
    let pixmap = render(
        &pdf.pages()[0],
        &RenderCache::new(),
        &settings,
        &RenderSettings::default(),
        &PixmapSettings {
            x_scale: 1.,
            y_scale: 1.,
            bg_color: WHITE,
        },
    );
    let ink = pixmap
        .data_as_u8_slice()
        .chunks_exact(4)
        .filter(|pixel| pixel[..3].iter().any(|channel| *channel < 245))
        .count();
    let png = pixmap.into_png().unwrap();
    drop(settings);
    let warnings = Arc::try_unwrap(warnings).unwrap().into_inner().unwrap();
    (png, ink, warnings)
}

fn assert_read_only_render(bytes: &[u8], nonblank: bool) -> Vec<u8> {
    let (raw, ink, warnings) = raw_raster(bytes);
    assert!(warnings.is_empty(), "{warnings:?}");
    if nonblank {
        assert!(ink > 100, "expected actual glyph pixels, got {ink}");
    }
    let session = Session::open(bytes).unwrap();
    let before = session.bytes().unwrap();
    assert_eq!(before, bytes, "opening must not rewrite the carrier");
    let dirty = session.is_dirty();
    let rendered = session.render_png(0, 1.).unwrap();
    assert_eq!(
        rendered, raw,
        "Session must render the actual, unchanged carrier"
    );
    assert_eq!(session.bytes().unwrap(), before);
    assert_eq!(session.is_dirty(), dirty);
    assert_eq!(
        Document::load_mem(bytes).unwrap().objects,
        Document::load_mem(&session.bytes().unwrap())
            .unwrap()
            .objects
    );
    assert_eq!(rendered, session.render_png(0, 1.).unwrap());
    let reopened = Session::open(&before).unwrap();
    assert_eq!(rendered, reopened.render_png(0, 1.).unwrap());
    assert_eq!(reopened.bytes().unwrap(), before);
    rendered
}

#[derive(Clone, Copy, Debug)]
enum Placement {
    Page,
    Form,
    Appearance,
}

fn save(mut doc: Document) -> Vec<u8> {
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}

// Keep the original producer's fonts, ToUnicode and all object references in one graph.
fn place_content(source: &[u8], content: Vec<u8>, placement: Placement) -> Vec<u8> {
    let mut doc = Document::load_mem(source).unwrap();
    let page_id = doc.get_pages()[&1];
    let resources = doc
        .get_object(page_id)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Resources")
        .unwrap()
        .clone();
    let page_content = match placement {
        Placement::Page => content,
        Placement::Form | Placement::Appearance => {
            let form = doc.add_object(Stream::new(
                dictionary! {
                    "Type" => "XObject", "Subtype" => "Form", "FormType" => 1,
                    "BBox" => vec![0.into(), 0.into(), 360.into(), 240.into()],
                    "Resources" => resources,
                },
                content,
            ));
            match placement {
                Placement::Form => {
                    doc.get_object_mut(page_id)
                        .unwrap()
                        .as_dict_mut()
                        .unwrap()
                        .set(
                            "Resources",
                            dictionary! { "XObject" => dictionary! { "Carrier" => form } },
                        );
                    b"/Carrier Do".to_vec()
                }
                Placement::Appearance => {
                    let annotation = doc.add_object(dictionary! {
                        "Type" => "Annot", "Subtype" => "FreeText", "F" => 4,
                        "Rect" => vec![0.into(), 0.into(), 360.into(), 240.into()],
                        "AP" => dictionary! { "N" => form },
                    });
                    doc.get_object_mut(page_id)
                        .unwrap()
                        .as_dict_mut()
                        .unwrap()
                        .set("Annots", vec![Object::Reference(annotation)]);
                    Vec::new()
                }
                Placement::Page => unreachable!(),
            }
        }
    };
    let content = doc.add_object(Stream::new(dictionary! {}, page_content));
    doc.get_object_mut(page_id)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Contents", content);
    save(doc)
}

fn utf16(text: &str) -> Vec<u8> {
    [
        vec![0xfe, 0xff],
        text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
    ]
    .concat()
}

fn span(bytes: Vec<u8>, format: StringFormat) -> Operation {
    Operation::new(
        "BDC",
        vec![
            Object::Name(b"Span".to_vec()),
            dictionary! { "ActualText" => Object::String(bytes, format) }.into(),
        ],
    )
}

fn rejected(bytes: &[u8], owner: &str, context: &str) {
    let error = Session::open(bytes).unwrap().render_png(0, 1.).unwrap_err();
    assert!(
        matches!(&error, PdfMasterError::Render(message) if message.contains(owner) && message.contains(context)),
        "expected Render {owner}/{context}, got {error:?}"
    );
}

#[test]
fn profile_rejects_every_non_direct_or_unmodeled_metadata_before_rendering() {
    let valid = span(utf16("metadata"), StringFormat::Hexadecimal).operands;
    let mut vectors: Vec<(&str, Vec<Object>)> = vec![
        ("zero operands", vec![]),
        ("one operand", vec![valid[0].clone()]),
        (
            "extra operand",
            vec![valid[0].clone(), valid[1].clone(), 1.into()],
        ),
        (
            "other tag",
            vec![Object::Name(b"Other".to_vec()), valid[1].clone()],
        ),
        ("OC", vec![Object::Name(b"OC".to_vec()), valid[1].clone()]),
        (
            "non-name",
            vec![Object::string_literal("Span"), valid[1].clone()],
        ),
        (
            "property name",
            vec![valid[0].clone(), Object::Name(b"Props".to_vec())],
        ),
        (
            "property array",
            vec![valid[0].clone(), Object::Array(vec![])],
        ),
        ("empty dict", vec![valid[0].clone(), dictionary! {}.into()]),
        (
            "missing ActualText",
            vec![
                valid[0].clone(),
                dictionary! {"Alt" => Object::string_literal("x")}.into(),
            ],
        ),
    ];
    for key in ["MCID", "Alt", "Lang", "OC", "OCG", "Unknown"] {
        let mut properties = valid[1].as_dict().unwrap().clone();
        properties.set(key, 1);
        vectors.push((key, vec![valid[0].clone(), properties.into()]));
    }
    for (name, value) in [
        ("integer value", 1.into()),
        ("name value", Object::Name(b"x".to_vec())),
        ("array value", Object::Array(vec![])),
        ("dictionary value", dictionary! {}.into()),
        ("null value", Object::Null),
        ("boolean value", Object::Boolean(true)),
    ] {
        vectors.push((
            name,
            vec![valid[0].clone(), dictionary! {"ActualText" => value}.into()],
        ));
    }
    let source = carrier("हैलो वर्ल्ड\nनमस्ते दुनिया", TextAlignment::Center);
    let mut failures = Vec::new();
    for (index, (name, operands)) in vectors.into_iter().enumerate() {
        let placement = [Placement::Page, Placement::Form, Placement::Appearance][index % 3];
        let content = Content {
            operations: vec![
                Operation::new("BDC", operands),
                Operation::new("EMC", vec![]),
            ],
        }
        .encode()
        .unwrap();
        // These are real parsed BDC operands, not a syntax error accidentally passing rejection.
        assert_eq!(
            Content::decode_strict(&content).unwrap().operations[0].operator,
            "BDC",
            "{name}"
        );
        let result = Session::open(&place_content(&source, content, placement))
            .unwrap()
            .render_png(0, 1.);
        let owner = if matches!(placement, Placement::Page) {
            "page"
        } else {
            "Form XObject"
        };
        if !matches!(&result, Err(PdfMasterError::Render(message)) if message.contains(owner) && message.contains("operator BDC") && message.contains("direct Span/ActualText"))
        {
            failures.push(format!(
                "{name}/{placement:?}: {:?}",
                result.as_ref().map(|_| "accepted")
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "missing metadata guards: {}",
        failures.join("\n")
    );
}

#[test]
fn encoding_rejects_each_malformed_utf16_payload_in_serialized_content() {
    let vectors = [
        ("empty", vec![]),
        ("no BOM", vec![0, 65]),
        ("LE", vec![0xff, 0xfe, 65, 0]),
        ("UTF8", "हैलो".as_bytes().to_vec()),
        ("odd", vec![0xfe, 0xff, 0]),
        ("high alone", vec![0xfe, 0xff, 0xd8, 0]),
        ("low alone", vec![0xfe, 0xff, 0xdc, 0]),
        ("inverted", vec![0xfe, 0xff, 0xdc, 0, 0xd8, 0]),
        ("high then BMP", vec![0xfe, 0xff, 0xd8, 0, 0, 65]),
        ("two high", vec![0xfe, 0xff, 0xd8, 0, 0xd8, 0]),
    ];
    let source = carrier("हैलो", TextAlignment::Left);
    let mut failures = Vec::new();
    for (name, bytes) in vectors {
        let content = Content {
            operations: vec![
                span(bytes, StringFormat::Hexadecimal),
                Operation::new("EMC", vec![]),
            ],
        }
        .encode()
        .unwrap();
        let result = Session::open(&place_content(&source, content, Placement::Appearance))
            .unwrap()
            .render_png(0, 1.);
        if !matches!(&result, Err(PdfMasterError::Render(message)) if message.contains("Form XObject") && message.contains("operator BDC") && message.contains("valid UTF16BE"))
        {
            failures.push(format!("{name}: {:?}", result.as_ref().map(|_| "accepted")));
        }
    }
    assert!(
        failures.is_empty(),
        "missing encoding guards: {}",
        failures.join("\n")
    );
}

fn operations(bytes: &[u8]) -> Vec<Operation> {
    let doc = Document::load_mem(bytes).unwrap();
    Content::decode_strict(&doc.get_page_content(doc.get_pages()[&1]))
        .unwrap()
        .operations
}

fn encode(operations: Vec<Operation>) -> Vec<u8> {
    Content { operations }.encode().unwrap()
}

fn unmarked(operations: &[Operation]) -> Vec<Operation> {
    operations
        .iter()
        .filter(|op| !matches!(op.operator.as_str(), "BMC" | "BDC" | "EMC"))
        .cloned()
        .collect()
}

#[test]
fn actual_carrier_form_and_freetext_appearance_preserve_pixels_and_object_graph() {
    let source = carrier(
        "Latin 中文 مرحبا بالعالم हैलो वर्ल्ड\nCafé e\u{301} office 𠮷",
        TextAlignment::Center,
    );
    let expected = assert_read_only_render(&source, true);
    let doc = Document::load_mem(&source).unwrap();
    let content = doc.get_page_content(doc.get_pages()[&1]);
    for placement in [Placement::Form, Placement::Appearance] {
        let bytes = place_content(&source, content.clone(), placement);
        assert_eq!(
            assert_read_only_render(&bytes, true),
            expected,
            "{placement:?}"
        );
    }
}

#[test]
fn direct_literal_hex_bom_astral_and_operator_words_are_only_metadata() {
    let source = carrier("हैलो वर्ल्ड\nनमस्ते दुनिया", TextAlignment::Right);
    let expected = assert_read_only_render(&source, true);
    for format in [StringFormat::Literal, StringFormat::Hexadecimal] {
        for text in [
            "",
            "BMP 中文",
            "𠮷",
            "EMC BDC q Q BT ET",
            "e\u{301}\u{200d}\u{fe0f}\u{2067}",
        ] {
            let mut ops = operations(&source);
            for op in &mut ops {
                if op.operator == "BDC" {
                    *op = span(utf16(text), format);
                }
            }
            let bytes = place_content(&source, encode(ops), Placement::Page);
            assert_eq!(assert_read_only_render(&bytes, true), expected);
        }
    }
}

#[test]
fn marked_frames_cover_nested_bmc_and_graphics_and_text_object_boundaries() {
    let source = carrier("हैलो वर्ल्ड\nनमस्ते दुनिया", TextAlignment::Left);
    let expected = assert_read_only_render(&source, true);
    // A true outer frame remains active across the producer's q/Q and BT/ET.
    let mut ops = vec![
        span(utf16("whole carrier"), StringFormat::Hexadecimal),
        Operation::new("BMC", vec![Object::Name(b"Span".to_vec())]),
    ];
    ops.extend(unmarked(&operations(&source)));
    ops.extend([Operation::new("EMC", vec![]), Operation::new("EMC", vec![])]);
    assert_eq!(
        assert_read_only_render(&place_content(&source, encode(ops), Placement::Page), true),
        expected
    );
}

#[test]
fn unmapped_cids_never_escape_actual_text_or_font_cache_into_page_form_or_ap() {
    let source = carrier("हैलो वर्ल्ड\nनमस्ते दुनिया", TextAlignment::Left);
    let original = operations(&source);
    let bare = unmarked(&original);
    for placement in [Placement::Page, Placement::Form, Placement::Appearance] {
        let owner = if matches!(placement, Placement::Page) {
            "page"
        } else {
            "Form XObject"
        };
        rejected(
            &place_content(&source, encode(bare.clone()), placement),
            owner,
            "Identity-H CID",
        );
        let mut cached = original.clone();
        cached.extend(bare.clone()); // Same font objects after a successful ActualText use.
        rejected(
            &place_content(&source, encode(cached), placement),
            owner,
            "Identity-H CID",
        );
    }
    for prefix in [
        vec![Operation::new("BMC", vec![Object::Name(b"Span".to_vec())])],
        vec![
            Operation::new("q", vec![]),
            span(utf16("closed"), StringFormat::Hexadecimal),
            Operation::new("EMC", vec![]),
            Operation::new("Q", vec![]),
        ],
        vec![
            span(utf16("closed"), StringFormat::Hexadecimal),
            Operation::new("EMC", vec![]),
        ],
    ] {
        let bmc = prefix[0].operator == "BMC";
        let mut ops = prefix;
        ops.extend(bare.clone());
        if bmc {
            ops.push(Operation::new("EMC", vec![]));
        }
        rejected(
            &place_content(&source, encode(ops), Placement::Page),
            "page",
            "Identity-H CID",
        );
    }
}

#[test]
fn parent_actual_text_never_covers_child_form_or_annotation_appearance() {
    let source = carrier("हैलो वर्ल्ड\nनमस्ते दुनिया", TextAlignment::Left);
    let bare = encode(unmarked(&operations(&source)));
    for placement in [Placement::Form, Placement::Appearance] {
        let bytes = place_content(&source, bare.clone(), placement);
        let mut doc = Document::load_mem(&bytes).unwrap();
        let page = doc.get_pages()[&1];
        let mut ops = vec![span(utf16("parent only"), StringFormat::Hexadecimal)];
        ops.extend(operations(&bytes));
        ops.push(Operation::new("EMC", vec![]));
        let stream = doc.add_object(Stream::new(dictionary! {}, encode(ops)));
        doc.get_object_mut(page)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Contents", stream);
        rejected(&save(doc), "Form XObject", "Identity-H CID");
    }
}

#[test]
fn missing_cff_glyph_and_odd_strings_remain_refused_inside_valid_actual_text() {
    let source = carrier("हैलो वर्ल्ड", TextAlignment::Left);
    for (bytes, context) in [
        (vec![0xff, 0xff], "Identity-H CID 65535"),
        (vec![0], "even byte length"),
    ] {
        let mut ops = operations(&source);
        let mut depth = 0;
        let mut changed = false;
        for op in &mut ops {
            if op.operator == "BDC" {
                depth += 1;
            }
            if op.operator == "EMC" {
                depth -= 1;
            }
            if depth > 0 && op.operator == "TJ" {
                op.operands = vec![Object::Array(vec![Object::String(
                    bytes.clone(),
                    StringFormat::Hexadecimal,
                )])];
                changed = true;
                break;
            }
        }
        assert!(changed, "must mutate real drawn glyphs inside ActualText");
        rejected(
            &place_content(&source, encode(ops), Placement::Page),
            "page",
            context,
        );
    }
}

#[test]
fn depth_balance_and_unsupported_operations_remain_fail_closed() {
    let source = carrier("हैलो", TextAlignment::Left);
    for depth in [64, 65] {
        let mut ops: Vec<_> = (0..depth)
            .map(|index| {
                if index % 2 == 0 {
                    span(utf16("depth"), StringFormat::Hexadecimal)
                } else {
                    Operation::new("BMC", vec![Object::Name(b"Span".to_vec())])
                }
            })
            .collect();
        ops.extend((0..depth).map(|_| Operation::new("EMC", vec![])));
        let bytes = place_content(&source, encode(ops), Placement::Page);
        if depth == 64 {
            assert_read_only_render(&bytes, false);
        } else {
            assert!(matches!(
                Session::open(&bytes).unwrap().render_png(0, 1.),
                Err(PdfMasterError::Budget("marked content depth"))
            ));
        }
    }
    for (ops, context) in [
        (vec![Operation::new("EMC", vec![])], "operator EMC"),
        (
            vec![span(utf16("open"), StringFormat::Hexadecimal)],
            "unbalanced BMC",
        ),
        (
            vec![
                span(utf16("open"), StringFormat::Hexadecimal),
                Operation::new("q", vec![]),
                Operation::new("Q", vec![]),
            ],
            "unbalanced BMC",
        ),
        (
            vec![
                span(utf16("open"), StringFormat::Hexadecimal),
                Operation::new("BT", vec![]),
                Operation::new("ET", vec![]),
            ],
            "unbalanced BMC",
        ),
        (vec![Operation::new("BMC", vec![])], "operator BMC"),
        (vec![Operation::new("BMC", vec![1.into()])], "operator BMC"),
    ] {
        rejected(
            &place_content(&source, encode(ops), Placement::Page),
            "page",
            context,
        );
    }
    for operator in [
        "gs",
        "DP",
        "UnknownActualTextOperator",
        "BX",
        "EX",
        "sh",
        "SC",
        "sc",
        "SCN",
        "scn",
    ] {
        let ops = vec![
            span(utf16("not an escape"), StringFormat::Hexadecimal),
            Operation::new(operator, vec![]),
            Operation::new("EMC", vec![]),
        ];
        rejected(
            &place_content(&source, encode(ops), Placement::Page),
            "page",
            &format!("operator {operator}"),
        );
    }
}

#[test]
fn resources_properties_and_serialized_indirect_properties_remain_refused() {
    let source = carrier("हैलो", TextAlignment::Left);
    let mut doc = Document::load_mem(&source).unwrap();
    let page = doc.get_pages()[&1];
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Resources", dictionary! { "Properties" => dictionary! {} });
    let error = Session::open(&save(doc))
        .unwrap()
        .render_png(0, 1.)
        .unwrap_err();
    assert!(matches!(error, PdfMasterError::Render(message) if message.contains("Properties")));
    for (properties, parsed_reference_operator) in [
        (Object::Reference((5000, 0)), true),
        (
            dictionary! { "ActualText" => Object::Reference((5000, 0)) }.into(),
            false,
        ),
    ] {
        let content = encode(vec![
            Operation::new("BDC", vec![Object::Name(b"Span".to_vec()), properties]),
            Operation::new("EMC", vec![]),
        ]);
        // Pinned lopdf parses a top-level reference as R, but retains a dictionary value
        // reference. Neither path may resolve a resource or widen the direct profile.
        let parsed = Content::decode_strict(&content).unwrap();
        let bytes = place_content(&source, content, Placement::Page);
        let error = Session::open(&bytes)
            .unwrap()
            .render_png(0, 1.)
            .unwrap_err();
        if parsed_reference_operator {
            assert_eq!(parsed.operations[0].operator, "R");
            assert!(
                matches!(error, PdfMasterError::Render(message) if message.contains("operator R") && message.contains("audited allowlist"))
            );
        } else {
            assert_eq!(parsed.operations[0].operator, "BDC");
            assert!(
                matches!(error, PdfMasterError::Render(message) if message.contains("operator BDC") && message.contains("direct Span/ActualText"))
            );
        }
    }
}

#[test]
fn every_serialized_text_show_obeys_the_current_validated_actual_text_context() {
    let source = carrier("हैलो वर्ल्ड\nनमस्ते दुनिया", TextAlignment::Left);
    for operator in ["Tj", "TJ", "'", "\""] {
        let mut ops = Vec::new();
        for op in operations(&source) {
            if op.operator == "TJ" && operator != "TJ" {
                for object in op.operands[0].as_array().unwrap() {
                    if matches!(object, Object::String(_, _)) {
                        let operands = if operator == "\"" {
                            vec![0.into(), 0.into(), object.clone()]
                        } else {
                            vec![object.clone()]
                        };
                        ops.push(Operation::new(operator, operands));
                    }
                }
            } else {
                ops.push(op);
            }
        }
        let bytes = place_content(&source, encode(ops.clone()), Placement::Page);
        assert_read_only_render(&bytes, true);
        rejected(
            &place_content(&source, encode(unmarked(&ops)), Placement::Page),
            "page",
            &format!("operator {operator} Identity-H CID"),
        );
    }
    let mut ops = operations(&source);
    ops.iter_mut()
        .find(|op| op.operator == "BDC")
        .unwrap()
        .operands[0] = Object::Name(b"Other".to_vec());
    rejected(
        &place_content(&source, encode(ops), Placement::Page),
        "page",
        "operator BDC requires direct Span/ActualText",
    );
}

#[test]
fn shared_font_cache_across_forms_never_carries_actual_text_state() {
    let source = carrier("हैलो वर्ल्ड\nनमस्ते दुनिया", TextAlignment::Left);
    let mut doc = Document::load_mem(&source).unwrap();
    let page = doc.get_pages()[&1];
    let resources = doc
        .get_object(page)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Resources")
        .unwrap()
        .clone();
    let content = operations(&source);
    let mut forms = Vec::new();
    for ops in [content.clone(), unmarked(&content)] {
        forms.push(doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 360.into(), 240.into()],
                "Resources" => resources.clone(),
            },
            encode(ops),
        )));
    }
    let stream = doc.add_object(Stream::new(
        dictionary! {},
        b"/First Do /Second Do".to_vec(),
    ));
    let page = doc.get_object_mut(page).unwrap().as_dict_mut().unwrap();
    page.set("Contents", stream);
    page.set(
        "Resources",
        dictionary! { "XObject" => dictionary! {"First" => forms[0], "Second" => forms[1]} },
    );
    rejected(&save(doc), "Form XObject", "Identity-H CID");
}

#[test]
fn marked_content_cannot_be_closed_across_page_and_form_boundaries() {
    let source = carrier("हैलो", TextAlignment::Left);
    let bytes = place_content(&source, b"EMC".to_vec(), Placement::Form);
    let mut doc = Document::load_mem(&bytes).unwrap();
    let page = doc.get_pages()[&1];
    let content = doc.add_object(Stream::new(
        dictionary! {},
        b"/Span << /ActualText <FEFF> >> BDC /Carrier Do".to_vec(),
    ));
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Contents", content);
    rejected(&save(doc), "Form XObject", "operator EMC");
}

#[test]
fn producer_actual_text_renders_unchanged_for_all_physical_alignments() {
    for text in [
        "हैलो वर्ल्ड\nनमस्ते दुनिया",
        "Latin 中文 مرحبا بالعالم हैलो वर्ल्ड\nCafé e\u{301} office 𠮷",
    ] {
        for alignment in [
            TextAlignment::Left,
            TextAlignment::Center,
            TextAlignment::Right,
        ] {
            let bytes = carrier(text, alignment);
            let doc = Document::load_mem(&bytes).unwrap();
            let operations = Content::decode_strict(&doc.get_page_content(doc.get_pages()[&1]))
                .unwrap()
                .operations;
            assert!(operations.iter().any(|op| op.operator == "BDC"));
            assert_read_only_render(&bytes, true);
        }
    }
}
