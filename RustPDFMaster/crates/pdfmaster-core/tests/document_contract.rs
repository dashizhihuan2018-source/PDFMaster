use std::io::Cursor;

use lopdf::content::Content;
use lopdf::{
    Dictionary, Document, EncryptionState, EncryptionVersion, Object, Permissions, Stream,
    StringFormat, dictionary,
};
use pdfmaster_core::{Color, Edit, Rect, Session};

fn fixture_pdf(page_sizes: &[(f32, f32)], text: Option<&str>) -> Vec<u8> {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let mut page_ids = Vec::new();
    for (index, &(width, height)) in page_sizes.iter().enumerate() {
        let label = text.map(|value| format!("{value}-{index}"));
        let content = label.map_or_else(Vec::new, |value| {
            format!("BT /F1 18 Tf 20 40 Td ({value}) Tj ET\n").into_bytes()
        });
        let content_id = doc.add_object(Stream::new(Dictionary::new(), content));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), width.into(), height.into()],
            "Resources" => dictionary! {
                "Font" => dictionary! { "F1" => font_id },
            },
            "Contents" => content_id,
        });
        page_ids.push(page_id);
    }
    doc.objects.insert(
        pages_id,
        dictionary! {
            "Type" => "Pages",
            "Kids" => page_ids.iter().copied().map(Object::Reference).collect::<Vec<_>>(),
            "Count" => page_ids.len() as i64,
        }
        .into(),
    );
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}

fn encrypted_fixture() -> Vec<u8> {
    let bytes = fixture_pdf(&[(120.0, 80.0)], None);
    let mut doc = Document::load_mem(&bytes).unwrap();
    doc.trailer.set(
        "ID",
        vec![
            Object::string_literal("fixture-id"),
            Object::string_literal("fixture-id"),
        ],
    );
    let state = EncryptionState::try_from(EncryptionVersion::V2 {
        document: &doc,
        owner_password: "owner-secret",
        user_password: "",
        key_length: 128,
        permissions: Permissions::empty(),
    })
    .unwrap();
    doc.encrypt(&state).unwrap();
    let mut output = Vec::new();
    doc.save_to(&mut output).unwrap();
    output
}

fn save(mut doc: Document) -> Vec<u8> {
    let mut output = Vec::new();
    doc.save_to(&mut output).unwrap();
    output
}

fn nested_inherited_fixture() -> Vec<u8> {
    let mut doc = Document::with_version("1.7");
    let root_pages = doc.new_object_id();
    let nested_pages = doc.new_object_id();
    let font = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let first_content = doc.add_object(Stream::new(
        Dictionary::new(),
        b"BT /F0 12 Tf 5 5 Td (first) Tj ET".to_vec(),
    ));
    let second_content = doc.add_object(Stream::new(
        Dictionary::new(),
        b"BT /F0 12 Tf 5 5 Td (second) Tj ET".to_vec(),
    ));
    let first = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => nested_pages,
        "Contents" => first_content,
    });
    let second = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => root_pages,
        "MediaBox" => vec![0.into(), 0.into(), 200.into(), 100.into()],
        "Resources" => dictionary! { "Font" => dictionary! { "F0" => font } },
        "Contents" => second_content,
    });
    doc.objects.insert(
        nested_pages,
        dictionary! {
            "Type" => "Pages",
            "Parent" => root_pages,
            "Kids" => vec![Object::Reference(first)],
            "Count" => 1,
            "MediaBox" => vec![0.into(), 0.into(), 120.into(), 80.into()],
            "Resources" => dictionary! { "Font" => dictionary! { "F0" => font } },
            "Rotate" => 90,
        }
        .into(),
    );
    doc.objects.insert(
        root_pages,
        dictionary! {
            "Type" => "Pages",
            "Kids" => vec![Object::Reference(nested_pages), Object::Reference(second)],
            "Count" => 2,
        }
        .into(),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => root_pages });
    doc.trailer.set("Root", catalog);
    save(doc)
}

fn fixture_with_indirect_contents_array() -> Vec<u8> {
    let bytes = fixture_pdf(&[(200.0, 120.0)], Some("seed"));
    let mut doc = Document::load_mem(&bytes).unwrap();
    let page_id = doc.get_pages()[&1];
    let original = doc
        .get_object(page_id)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Contents")
        .unwrap()
        .clone();
    let array_id = doc.add_object(vec![original]);
    doc.get_object_mut(page_id)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Contents", array_id);
    save(doc)
}

fn fixture_with_existing_fpm() -> Vec<u8> {
    let bytes = fixture_pdf(&[(200.0, 120.0)], None);
    let mut doc = Document::load_mem(&bytes).unwrap();
    let page_id = doc.get_pages()[&1];
    let courier = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Courier",
    });
    doc.get_object_mut(page_id)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .get_mut(b"Resources")
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .get_mut(b"Font")
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("FPM", courier);
    save(doc)
}

fn fixture_with_indirect_annots() -> Vec<u8> {
    let bytes = fixture_pdf(&[(200.0, 120.0)], None);
    let mut doc = Document::load_mem(&bytes).unwrap();
    let page_id = doc.get_pages()[&1];
    let annots = doc.add_object(Vec::<Object>::new());
    doc.get_object_mut(page_id)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", annots);
    save(doc)
}

fn install_page_xobject(doc: &mut Document, name: &[u8], object: Object) {
    let page_id = doc.get_pages()[&1];
    let page = doc.get_object_mut(page_id).unwrap().as_dict_mut().unwrap();
    let resources = page.get_mut(b"Resources").unwrap().as_dict_mut().unwrap();
    if !resources.has(b"XObject") {
        resources.set("XObject", Dictionary::new());
    }
    resources
        .get_mut(b"XObject")
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set(name, object);
}

fn set_page_content(doc: &mut Document, content: &[u8]) {
    let page = doc.get_pages()[&1];
    let content_id = doc
        .get_object(page)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Contents")
        .unwrap()
        .as_reference()
        .unwrap();
    doc.get_object_mut(content_id)
        .unwrap()
        .as_stream_mut()
        .unwrap()
        .set_plain_content(content.to_vec());
}

fn image_stream(width: i64, height: i64, filter: Option<&[u8]>, content: Vec<u8>) -> Stream {
    let mut dict = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Image",
        "Width" => width,
        "Height" => height,
        "ColorSpace" => "DeviceRGB",
        "BitsPerComponent" => 8,
    };
    if let Some(filter) = filter {
        dict.set("Filter", Object::Name(filter.to_vec()));
    }
    Stream::new(dict, content)
}

fn fixture_with_page_content(content: &[u8]) -> Vec<u8> {
    let mut doc = Document::load_mem(&fixture_pdf(&[(100.0, 100.0)], None)).unwrap();
    set_page_content(&mut doc, content);
    save(doc)
}

fn offset_startxref(mut bytes: Vec<u8>) -> Vec<u8> {
    let marker = b"startxref\n";
    let start = bytes
        .windows(marker.len())
        .rposition(|window| window == marker)
        .unwrap()
        + marker.len();
    let end = bytes[start..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map(|length| start + length)
        .unwrap();
    let original = std::str::from_utf8(&bytes[start..end])
        .unwrap()
        .parse::<usize>()
        .unwrap();
    let replacement = (original + 1).to_string();
    assert_eq!(replacement.len(), end - start);
    bytes[start..end].copy_from_slice(replacement.as_bytes());
    bytes
}

fn signed_fixture() -> Vec<u8> {
    let bytes = fixture_pdf(&[(120.0, 80.0)], None);
    let mut doc = Document::load_mem(&bytes).unwrap();
    let signature = doc.add_object(dictionary! {
        "Type" => "Sig",
        "Filter" => "Adobe.PPKLite",
        "ByteRange" => vec![0.into(), 10.into(), 20.into(), 30.into()],
        "Contents" => Object::string_literal("deadbeef"),
    });
    let field = doc.add_object(dictionary! { "FT" => "Sig", "V" => signature });
    let form = doc.add_object(dictionary! { "Fields" => vec![field.into()] });
    let root_id = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    doc.get_object_mut(root_id)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("AcroForm", form);
    let mut output = Vec::new();
    doc.save_to(&mut output).unwrap();
    output
}

fn fixture_without_resources() -> Vec<u8> {
    let bytes = fixture_pdf(&[(120.0, 80.0)], None);
    let mut doc = Document::load_mem(&bytes).unwrap();
    let page_id = doc.get_pages()[&1];
    doc.get_object_mut(page_id)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .remove(b"Resources");
    let mut output = Vec::new();
    doc.save_to(&mut output).unwrap();
    output
}

fn fixture_with_indirect_media_box() -> Vec<u8> {
    let bytes = fixture_pdf(&[(120.0, 80.0)], None);
    let mut doc = Document::load_mem(&bytes).unwrap();
    let page_id = doc.get_pages()[&1];
    let box_id = doc.add_object(vec![0.into(), 0.into(), 120.into(), 80.into()]);
    doc.get_object_mut(page_id)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("MediaBox", box_id);
    let mut output = Vec::new();
    doc.save_to(&mut output).unwrap();
    output
}

fn rect() -> Rect {
    Rect {
        x: 10.0,
        y: 10.0,
        width: 80.0,
        height: 24.0,
    }
}

fn black() -> Color {
    Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
    }
}

#[test]
fn blank_creates_a_parseable_pdf_with_requested_pages() {
    let session = Session::blank(200.0, 100.0, 3).unwrap();
    let bytes = session.bytes().unwrap();
    let parsed = Document::load_mem(&bytes).unwrap();
    assert_eq!(parsed.get_pages().len(), 3);
    assert_eq!(session.info().page_count, 3);
}

#[test]
fn open_reports_independent_page_geometry() {
    let bytes = fixture_pdf(&[(120.0, 80.0), (300.0, 500.0)], None);
    let session = Session::open(&bytes).unwrap();
    let info = session.info();
    assert_eq!(info.page_count, 2);
    assert_eq!((info.pages[0].width, info.pages[0].height), (120.0, 80.0));
    assert_eq!((info.pages[1].width, info.pages[1].height), (300.0, 500.0));
}

#[test]
fn open_resolves_an_indirect_media_box() {
    let session = Session::open(&fixture_with_indirect_media_box()).unwrap();
    assert_eq!(session.info().pages[0].width, 120.0);
    assert_eq!(session.info().pages[0].height, 80.0);
}

#[test]
fn malformed_pdf_is_rejected() {
    let error = Session::open(b"not a pdf").unwrap_err();
    assert!(error.to_string().contains("malformed"));
}

#[test]
fn strict_parser_rejects_a_repaired_startxref_offset() {
    let damaged = offset_startxref(fixture_pdf(&[(120.0, 80.0)], None));
    assert!(
        Document::load_mem(&damaged).is_ok(),
        "lopdf lenient mode repairs this fixture"
    );
    assert!(
        Session::open(&damaged).is_err(),
        "core must not accept repaired xref data"
    );
}

#[test]
fn encrypted_pdf_is_rejected_closed() {
    let error = Session::open(&encrypted_fixture()).unwrap_err();
    assert!(error.to_string().contains("encrypted"));
}

#[test]
fn signed_pdf_is_rejected_closed() {
    let error = Session::open(&signed_fixture()).unwrap_err();
    assert!(error.to_string().contains("signed"));
}

#[test]
fn blank_rejects_invalid_geometry_and_page_budget() {
    assert!(Session::blank(f32::NAN, 100.0, 1).is_err());
    assert!(Session::blank(100.0, 0.0, 1).is_err());
    assert!(Session::blank(100.0, 100.0, 0).is_err());
    assert!(Session::blank(100.0, 100.0, 4_097).is_err());
}

#[test]
fn rotate_updates_page_rotation_and_persists() {
    let mut session = Session::open(&fixture_pdf(&[(120.0, 80.0)], None)).unwrap();
    session
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    assert_eq!(session.info().pages[0].rotation, 90);
    assert_eq!(
        Session::open(&session.bytes().unwrap())
            .unwrap()
            .info()
            .pages[0]
            .rotation,
        90
    );
}

#[test]
fn invalid_rotation_is_atomic() {
    let mut session = Session::open(&fixture_pdf(&[(120.0, 80.0)], None)).unwrap();
    let before = session.bytes().unwrap();
    assert!(
        session
            .apply(Edit::Rotate {
                page: 0,
                degrees: 45
            })
            .is_err()
    );
    assert_eq!(session.bytes().unwrap(), before);
    assert!(!session.is_dirty());
}

#[test]
fn move_page_reorders_real_page_objects() {
    let mut session = Session::open(&fixture_pdf(
        &[(100.0, 100.0), (200.0, 100.0), (300.0, 100.0)],
        None,
    ))
    .unwrap();
    session.apply(Edit::MovePage { from: 2, to: 0 }).unwrap();
    assert_eq!(
        session
            .info()
            .pages
            .iter()
            .map(|p| p.width)
            .collect::<Vec<_>>(),
        vec![300.0, 100.0, 200.0]
    );
}

#[test]
fn insert_blank_adds_page_at_zero_based_index() {
    let mut session = Session::open(&fixture_pdf(&[(100.0, 100.0), (200.0, 100.0)], None)).unwrap();
    session
        .apply(Edit::InsertBlank {
            index: 1,
            width: 140.0,
            height: 90.0,
        })
        .unwrap();
    assert_eq!(
        session
            .info()
            .pages
            .iter()
            .map(|p| p.width)
            .collect::<Vec<_>>(),
        vec![100.0, 140.0, 200.0]
    );
}

#[test]
fn duplicate_page_copies_content_and_geometry() {
    let mut session = Session::open(&fixture_pdf(&[(120.0, 80.0)], Some("seed"))).unwrap();
    session
        .apply(Edit::DuplicatePage { page: 0, index: 1 })
        .unwrap();
    assert_eq!(session.info().page_count, 2);
    assert!(session.text(1).unwrap().contains("seed-0"));
}

#[test]
fn delete_page_removes_requested_page() {
    let mut session = Session::open(&fixture_pdf(&[(100.0, 100.0), (200.0, 100.0)], None)).unwrap();
    session.apply(Edit::DeletePage { page: 0 }).unwrap();
    assert_eq!(session.info().page_count, 1);
    assert_eq!(session.info().pages[0].width, 200.0);
}

#[test]
fn delete_protects_last_page_without_mutation() {
    let mut session = Session::open(&fixture_pdf(&[(100.0, 100.0)], None)).unwrap();
    let before = session.bytes().unwrap();
    assert!(session.apply(Edit::DeletePage { page: 0 }).is_err());
    assert_eq!(session.bytes().unwrap(), before);
    assert!(!session.is_dirty());
}

#[test]
fn invalid_page_index_is_atomic() {
    let mut session = Session::open(&fixture_pdf(&[(100.0, 100.0)], None)).unwrap();
    let before = session.bytes().unwrap();
    assert!(
        session
            .apply(Edit::Rotate {
                page: 7,
                degrees: 90
            })
            .is_err()
    );
    assert_eq!(session.bytes().unwrap(), before);
    assert_eq!(session.info().page_count, 1);
}

#[test]
fn editing_never_modifies_the_callers_input_buffer() {
    let input = fixture_pdf(&[(100.0, 100.0)], None);
    let original = input.clone();
    let mut session = Session::open(&input).unwrap();
    session
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    assert_eq!(input, original);
    assert_ne!(session.bytes().unwrap(), input);
}

#[test]
fn undo_redo_restore_snapshots_and_dirty_state() {
    let mut session = Session::open(&fixture_pdf(&[(100.0, 100.0)], None)).unwrap();
    session
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    let edited = session.bytes().unwrap();
    assert!(session.is_dirty());
    assert!(session.info().can_undo);
    session.undo().unwrap();
    assert!(!session.is_dirty());
    assert_eq!(session.info().pages[0].rotation, 0);
    assert!(session.info().can_redo);
    session.redo().unwrap();
    assert!(session.is_dirty());
    assert_eq!(session.bytes().unwrap(), edited);
}

#[test]
fn mark_saved_tracks_exact_revision_across_undo() {
    let mut session = Session::open(&fixture_pdf(&[(100.0, 100.0)], None)).unwrap();
    session
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    session.mark_saved();
    assert!(!session.is_dirty());
    session
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    assert!(session.is_dirty());
    session.undo().unwrap();
    assert!(!session.is_dirty());
}

#[test]
fn new_edit_clears_redo_history() {
    let mut session = Session::open(&fixture_pdf(&[(100.0, 100.0), (200.0, 100.0)], None)).unwrap();
    session
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    session.undo().unwrap();
    session.apply(Edit::MovePage { from: 0, to: 1 }).unwrap();
    assert!(!session.info().can_redo);
    assert!(session.redo().is_err());
}

#[test]
fn text_edit_is_extractable_after_independent_reopen() {
    let mut session = Session::open(&fixture_pdf(&[(200.0, 120.0)], None)).unwrap();
    session
        .apply(Edit::AddText {
            page: 0,
            bounds: rect(),
            text: "Rust PDF Master".into(),
            font_size: 14.0,
            color: black(),
        })
        .unwrap();
    assert!(session.text(0).unwrap().contains("Rust PDF Master"));
    let independently_reopened = Document::load_mem(&session.bytes().unwrap()).unwrap();
    assert!(
        independently_reopened
            .extract_text(&[1])
            .unwrap()
            .contains("Rust PDF Master")
    );
}

#[test]
fn text_edit_installs_resources_on_a_valid_resource_free_page() {
    let mut session = Session::open(&fixture_without_resources()).unwrap();
    session
        .apply(Edit::AddText {
            page: 0,
            bounds: rect(),
            text: "added".into(),
            font_size: 12.0,
            color: black(),
        })
        .unwrap();
    assert!(session.text(0).unwrap().contains("added"));
}

#[test]
fn note_annotation_has_pdf_semantics_and_renderable_appearance() {
    let mut session = Session::open(&fixture_pdf(&[(200.0, 120.0)], None)).unwrap();
    session
        .apply(Edit::AddNote {
            page: 0,
            bounds: rect(),
            contents: "review me".into(),
            color: Color {
                r: 1.0,
                g: 0.8,
                b: 0.0,
            },
        })
        .unwrap();
    let parsed = Document::load_mem(&session.bytes().unwrap()).unwrap();
    let page_id = parsed.get_pages()[&1];
    let page = parsed.get_object(page_id).unwrap().as_dict().unwrap();
    let annots = page.get(b"Annots").unwrap().as_array().unwrap();
    let annot_id = annots[0].as_reference().unwrap();
    let annot = parsed.get_object(annot_id).unwrap().as_dict().unwrap();
    assert_eq!(annot.get(b"Subtype").unwrap().as_name().unwrap(), b"Text");
    assert_eq!(
        annot.get(b"Contents").unwrap().as_str().unwrap(),
        b"review me"
    );
    let ap = annot.get(b"AP").unwrap().as_dict().unwrap();
    let normal_id = ap.get(b"N").unwrap().as_reference().unwrap();
    let appearance = parsed.get_object(normal_id).unwrap().as_stream().unwrap();
    assert!(!appearance.content.is_empty());
    assert_eq!(
        appearance.dict.get(b"Subtype").unwrap().as_name().unwrap(),
        b"Form"
    );
}

#[test]
fn invalid_text_geometry_is_atomic() {
    let mut session = Session::open(&fixture_pdf(&[(100.0, 100.0)], None)).unwrap();
    let before = session.bytes().unwrap();
    let result = session.apply(Edit::AddText {
        page: 0,
        bounds: Rect {
            x: 90.0,
            y: 90.0,
            width: 20.0,
            height: 20.0,
        },
        text: "outside".into(),
        font_size: 12.0,
        color: black(),
    });
    assert!(result.is_err());
    assert_eq!(session.bytes().unwrap(), before);
}

#[test]
fn render_png_has_expected_dimensions_and_nonwhite_pixels() {
    let mut session = Session::open(&fixture_pdf(&[(100.0, 60.0)], None)).unwrap();
    session
        .apply(Edit::AddText {
            page: 0,
            bounds: Rect {
                x: 10.0,
                y: 10.0,
                width: 80.0,
                height: 25.0,
            },
            text: "visible".into(),
            font_size: 18.0,
            color: black(),
        })
        .unwrap();
    let png = session.render_png(0, 2.0).unwrap();
    let decoder = png::Decoder::new(Cursor::new(png));
    let mut reader = decoder.read_info().unwrap();
    assert_eq!((reader.info().width, reader.info().height), (200, 120));
    let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
    let frame = reader.next_frame(&mut buffer).unwrap();
    let pixels = &buffer[..frame.buffer_size()];
    assert!(
        pixels
            .chunks_exact(4)
            .any(|rgba| rgba[0..3] != [255, 255, 255])
    );
}

#[test]
fn render_rejects_invalid_page_and_scale() {
    let session = Session::open(&fixture_pdf(&[(100.0, 60.0)], None)).unwrap();
    assert!(session.render_png(1, 1.0).is_err());
    assert!(session.render_png(0, 0.0).is_err());
    assert!(session.render_png(0, f32::NAN).is_err());
}

#[test]
fn render_rejects_excessive_pixel_budget() {
    let session = Session::open(&fixture_pdf(&[(20_000.0, 20_000.0)], None)).unwrap();
    assert!(session.render_png(0, 2.0).is_err());
}

#[test]
fn edit_serde_is_stable_and_tagged() {
    let edit = Edit::MovePage { from: 3, to: 1 };
    let json = serde_json::to_value(&edit).unwrap();
    assert_eq!(json["kind"], "move_page");
    assert_eq!(serde_json::from_value::<Edit>(json).unwrap(), edit);
}

#[test]
fn history_is_bounded_but_remains_usable() {
    let mut session = Session::open(&fixture_pdf(&[(100.0, 100.0)], None)).unwrap();
    for _ in 0..40 {
        session
            .apply(Edit::Rotate {
                page: 0,
                degrees: 90,
            })
            .unwrap();
    }
    let mut undo_count = 0;
    while session.undo().is_ok() {
        undo_count += 1;
    }
    assert_eq!(undo_count, 32);
}

#[test]
fn real_empty_password_restrictive_encryption_is_rejected_after_auto_decrypt() {
    let bytes = encrypted_fixture();
    let loaded = Document::load_mem(&bytes).unwrap();
    assert!(!loaded.is_encrypted());
    assert!(loaded.was_encrypted());
    assert!(
        Session::open(&bytes)
            .unwrap_err()
            .to_string()
            .contains("encrypted")
    );
}

#[test]
fn page_tree_count_mismatch_is_rejected() {
    let mut doc = Document::load_mem(&nested_inherited_fixture()).unwrap();
    let root = doc
        .catalog()
        .unwrap()
        .get(b"Pages")
        .unwrap()
        .as_reference()
        .unwrap();
    doc.get_object_mut(root)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Count", 1);
    assert!(
        Session::open(&save(doc))
            .unwrap_err()
            .to_string()
            .contains("Count")
    );
}

#[test]
fn page_tree_root_must_be_pages_not_page() {
    let mut doc = Document::load_mem(&fixture_pdf(&[(100.0, 100.0)], None)).unwrap();
    let root = doc
        .catalog()
        .unwrap()
        .get(b"Pages")
        .unwrap()
        .as_reference()
        .unwrap();
    let root_dict = doc.get_object_mut(root).unwrap().as_dict_mut().unwrap();
    root_dict.set("Type", "Page");
    root_dict.set("MediaBox", vec![0.into(), 0.into(), 100.into(), 100.into()]);
    root_dict.set("Resources", Dictionary::new());
    assert!(
        Session::open(&save(doc))
            .unwrap_err()
            .to_string()
            .contains("root")
    );
}

#[test]
fn page_tree_root_must_not_inherit_from_an_external_parent() {
    let mut doc = Document::load_mem(&fixture_pdf(&[(100.0, 100.0)], None)).unwrap();
    let root = doc
        .catalog()
        .unwrap()
        .get(b"Pages")
        .unwrap()
        .as_reference()
        .unwrap();
    let external = doc.add_object(dictionary! {
        "Type" => "Pages",
        "Kids" => Vec::<Object>::new(),
        "Count" => 0,
        "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
    });
    doc.get_object_mut(root)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Parent", external);
    assert!(
        Session::open(&save(doc))
            .unwrap_err()
            .to_string()
            .contains("root")
    );
}

#[test]
fn declared_page_budget_is_rejected_before_tree_collection() {
    let mut doc = Document::load_mem(&fixture_pdf(&[(100.0, 100.0)], None)).unwrap();
    let root = doc
        .catalog()
        .unwrap()
        .get(b"Pages")
        .unwrap()
        .as_reference()
        .unwrap();
    doc.get_object_mut(root)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Count", 4_097);
    assert!(
        Session::open(&save(doc))
            .unwrap_err()
            .to_string()
            .contains("page count")
    );
}

#[test]
fn cyclic_page_tree_is_rejected() {
    let mut doc = Document::load_mem(&nested_inherited_fixture()).unwrap();
    let root = doc
        .catalog()
        .unwrap()
        .get(b"Pages")
        .unwrap()
        .as_reference()
        .unwrap();
    let nested = doc
        .get_object(root)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Kids")
        .unwrap()
        .as_array()
        .unwrap()[0]
        .as_reference()
        .unwrap();
    doc.get_object_mut(nested)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Kids", vec![Object::Reference(root)]);
    assert!(
        Session::open(&save(doc))
            .unwrap_err()
            .to_string()
            .contains("cycle")
    );
}

#[test]
fn unknown_page_tree_kid_is_rejected_instead_of_skipped() {
    let mut doc = Document::load_mem(&fixture_pdf(&[(100.0, 100.0)], None)).unwrap();
    let root = doc
        .catalog()
        .unwrap()
        .get(b"Pages")
        .unwrap()
        .as_reference()
        .unwrap();
    let unknown = doc.add_object(dictionary! { "Type" => "NotAPage" });
    let root_dict = doc.get_object_mut(root).unwrap().as_dict_mut().unwrap();
    root_dict
        .get_mut(b"Kids")
        .unwrap()
        .as_array_mut()
        .unwrap()
        .push(unknown.into());
    root_dict.set("Count", 2);
    assert!(Session::open(&save(doc)).is_err());
}

#[test]
fn duplicate_page_tree_reference_is_rejected() {
    let mut doc = Document::load_mem(&fixture_pdf(&[(100.0, 100.0)], None)).unwrap();
    let root = doc
        .catalog()
        .unwrap()
        .get(b"Pages")
        .unwrap()
        .as_reference()
        .unwrap();
    let page = doc.get_pages()[&1];
    let root_dict = doc.get_object_mut(root).unwrap().as_dict_mut().unwrap();
    root_dict.set(
        "Kids",
        vec![Object::Reference(page), Object::Reference(page)],
    );
    root_dict.set("Count", 2);
    assert!(
        Session::open(&save(doc))
            .unwrap_err()
            .to_string()
            .contains("duplicate")
    );
}

#[test]
fn reordering_materializes_inherited_page_attributes_before_flattening() {
    let mut session = Session::open(&nested_inherited_fixture()).unwrap();
    assert_eq!(session.info().pages[0].rotation, 90);
    session.apply(Edit::MovePage { from: 0, to: 1 }).unwrap();
    let parsed = Document::load_mem(&session.bytes().unwrap()).unwrap();
    for page_id in parsed.get_pages().into_values() {
        let page = parsed.get_object(page_id).unwrap().as_dict().unwrap();
        assert!(page.has(b"MediaBox"));
        assert!(page.has(b"Resources"));
    }
    assert_eq!(session.info().pages[1].rotation, 90);
    assert!(session.text(1).unwrap().contains("first"));
}

#[test]
fn duplicating_an_inherited_page_materializes_its_attributes() {
    let mut session = Session::open(&nested_inherited_fixture()).unwrap();
    session
        .apply(Edit::DuplicatePage { page: 0, index: 2 })
        .unwrap();
    assert_eq!(session.info().page_count, 3);
    assert_eq!(session.info().pages[2], session.info().pages[0]);
    assert!(session.text(2).unwrap().contains("first"));
}

#[test]
fn add_text_preserves_an_existing_fpm_font_resource() {
    let mut session = Session::open(&fixture_with_existing_fpm()).unwrap();
    session
        .apply(Edit::AddText {
            page: 0,
            bounds: rect(),
            text: "new".into(),
            font_size: 12.0,
            color: black(),
        })
        .unwrap();
    let parsed = Document::load_mem(&session.bytes().unwrap()).unwrap();
    let page_id = parsed.get_pages()[&1];
    let page = parsed.get_object(page_id).unwrap().as_dict().unwrap();
    let resources = page.get(b"Resources").unwrap().as_dict().unwrap();
    let fonts = resources.get(b"Font").unwrap().as_dict().unwrap();
    let fpm = fonts.get(b"FPM").unwrap().as_reference().unwrap();
    assert_eq!(
        parsed
            .get_object(fpm)
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"BaseFont")
            .unwrap()
            .as_name()
            .unwrap(),
        b"Courier"
    );
    assert!(fonts.has(b"FPM1"));
    let decoded = Content::decode(&parsed.get_page_content(page_id)).unwrap();
    assert!(decoded.operations.iter().any(|op| {
        op.operator == "Tf"
            && op.operands.first().and_then(|value| value.as_name().ok()) == Some(b"FPM1")
    }));
}

#[test]
fn add_text_normalizes_an_indirect_contents_array() {
    let mut session = Session::open(&fixture_with_indirect_contents_array()).unwrap();
    session
        .apply(Edit::AddText {
            page: 0,
            bounds: rect(),
            text: "appended".into(),
            font_size: 12.0,
            color: black(),
        })
        .unwrap();
    assert!(session.text(0).unwrap().contains("seed-0"));
    assert!(session.text(0).unwrap().contains("appended"));
    let parsed = Document::load_mem(&session.bytes().unwrap()).unwrap();
    let page = parsed
        .get_object(parsed.get_pages()[&1])
        .unwrap()
        .as_dict()
        .unwrap();
    let contents = page.get(b"Contents").unwrap().as_array().unwrap();
    assert!(contents.len() >= 2);
    assert!(contents.iter().all(|entry| {
        entry
            .as_reference()
            .ok()
            .and_then(|id| parsed.get_object(id).ok())
            .is_some_and(|object| object.as_stream().is_ok())
    }));
}

#[test]
fn unsupported_page_coordinate_systems_fail_closed() {
    let base = fixture_pdf(&[(120.0, 80.0)], None);
    let mut origin = Document::load_mem(&base).unwrap();
    let page_id = origin.get_pages()[&1];
    origin
        .get_object_mut(page_id)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set(
            "MediaBox",
            vec![(-10).into(), 0.into(), 110.into(), 80.into()],
        );
    assert!(
        Session::open(&save(origin))
            .unwrap_err()
            .to_string()
            .contains("origin")
    );

    let mut crop = Document::load_mem(&base).unwrap();
    let page_id = crop.get_pages()[&1];
    crop.get_object_mut(page_id)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("CropBox", vec![10.into(), 10.into(), 110.into(), 70.into()]);
    assert!(
        Session::open(&save(crop))
            .unwrap_err()
            .to_string()
            .contains("CropBox")
    );
}

#[test]
fn indirect_rotation_is_resolved_and_malformed_rotation_is_rejected() {
    let base = fixture_pdf(&[(120.0, 80.0)], None);
    let mut valid = Document::load_mem(&base).unwrap();
    let page_id = valid.get_pages()[&1];
    let rotate = valid.add_object(90_i64);
    valid
        .get_object_mut(page_id)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Rotate", rotate);
    assert_eq!(
        Session::open(&save(valid)).unwrap().info().pages[0].rotation,
        90
    );

    let mut invalid = Document::load_mem(&base).unwrap();
    let page_id = invalid.get_pages()[&1];
    let rotate = invalid.add_object(Object::string_literal("90"));
    invalid
        .get_object_mut(page_id)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Rotate", rotate);
    assert!(
        Session::open(&save(invalid))
            .unwrap_err()
            .to_string()
            .contains("rotation")
    );
}

#[test]
fn unicode_add_text_is_explicitly_rejected_atomically() {
    for text in ["café", "中文"] {
        let mut session = Session::open(&fixture_pdf(&[(200.0, 120.0)], None)).unwrap();
        let before = session.bytes().unwrap();
        let error = session
            .apply(Edit::AddText {
                page: 0,
                bounds: rect(),
                text: text.into(),
                font_size: 12.0,
                color: black(),
            })
            .unwrap_err();
        assert!(error.to_string().contains("ASCII"));
        assert_eq!(session.bytes().unwrap(), before);
    }
}

#[test]
fn unicode_note_contents_use_utf16be_pdf_text_encoding() {
    let mut session = Session::open(&fixture_pdf(&[(200.0, 120.0)], None)).unwrap();
    session
        .apply(Edit::AddNote {
            page: 0,
            bounds: rect(),
            contents: "批注 café".into(),
            color: black(),
        })
        .unwrap();
    let parsed = Document::load_mem(&session.bytes().unwrap()).unwrap();
    let page = parsed
        .get_object(parsed.get_pages()[&1])
        .unwrap()
        .as_dict()
        .unwrap();
    let annot_id = page.get(b"Annots").unwrap().as_array().unwrap()[0]
        .as_reference()
        .unwrap();
    let contents = parsed
        .get_object(annot_id)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Contents")
        .unwrap();
    match contents {
        Object::String(bytes, StringFormat::Hexadecimal) => {
            assert!(bytes.starts_with(&[0xFE, 0xFF]))
        }
        other => panic!("expected hexadecimal UTF-16BE string, got {other:?}"),
    }
}

#[test]
fn duplicate_page_rejects_owned_annotation_structures_atomically() {
    let mut with_note = Session::open(&fixture_pdf(&[(200.0, 120.0)], None)).unwrap();
    with_note
        .apply(Edit::AddNote {
            page: 0,
            bounds: rect(),
            contents: "owned".into(),
            color: black(),
        })
        .unwrap();
    let before = with_note.bytes().unwrap();
    assert!(
        with_note
            .apply(Edit::DuplicatePage { page: 0, index: 1 })
            .is_err()
    );
    assert_eq!(with_note.bytes().unwrap(), before);

    let mut doc = Document::load_mem(&fixture_pdf(&[(100.0, 100.0)], None)).unwrap();
    let page = doc.get_pages()[&1];
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("StructParents", 0);
    let mut structured = Session::open(&save(doc)).unwrap();
    assert!(
        structured
            .apply(Edit::DuplicatePage { page: 0, index: 1 })
            .is_err()
    );
}

#[test]
fn note_only_page_reopens_and_renders_visible_pixels_with_indirect_annots() {
    let mut session = Session::open(&fixture_with_indirect_annots()).unwrap();
    session
        .apply(Edit::AddNote {
            page: 0,
            bounds: rect(),
            contents: "visible note".into(),
            color: Color {
                r: 1.0,
                g: 0.8,
                b: 0.0,
            },
        })
        .unwrap();
    let reopened = Session::open(&session.bytes().unwrap()).unwrap();
    let png = reopened.render_png(0, 1.0).unwrap();
    let decoder = png::Decoder::new(Cursor::new(png));
    let mut reader = decoder.read_info().unwrap();
    let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
    let frame = reader.next_frame(&mut buffer).unwrap();
    assert!(
        buffer[..frame.buffer_size()]
            .chunks_exact(4)
            .any(|rgba| rgba[0..3] != [255, 255, 255])
    );
}

#[test]
fn render_uses_hayro_fractional_dimensions() {
    let session = Session::open(&fixture_pdf(&[(100.75, 60.75)], None)).unwrap();
    let png = session.render_png(0, 2.0).unwrap();
    let decoder = png::Decoder::new(Cursor::new(png));
    let reader = decoder.read_info().unwrap();
    assert_eq!((reader.info().width, reader.info().height), (201, 121));
}

#[test]
fn render_rejects_a_single_side_over_u16_independently() {
    let session = Session::open(&fixture_pdf(&[(20_000.0, 1.0)], None)).unwrap();
    assert!(
        session
            .render_png(0, 4.0)
            .unwrap_err()
            .to_string()
            .contains("side")
    );
}

#[test]
fn render_rejects_total_pixels_independently() {
    let session = Session::open(&fixture_pdf(&[(10_000.0, 5_000.0)], None)).unwrap();
    assert!(
        session
            .render_png(0, 1.0)
            .unwrap_err()
            .to_string()
            .contains("total")
    );
}

#[test]
fn renderer_surfaces_material_annotation_warnings() {
    let mut doc = Document::load_mem(&fixture_pdf(&[(100.0, 100.0)], None)).unwrap();
    let page_id = doc.get_pages()[&1];
    let annot = doc.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Square",
        "Rect" => vec![10.into(), 10.into(), 40.into(), 40.into()],
        "F" => 4,
        "AP" => dictionary! { "N" => dictionary! { "Broken" => true } },
    });
    doc.get_object_mut(page_id)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", vec![Object::Reference(annot)]);
    let session = Session::open(&save(doc)).unwrap();
    assert!(
        session
            .render_png(0, 1.0)
            .unwrap_err()
            .to_string()
            .contains("warning")
    );
}

#[test]
fn renderer_rejects_corrupt_flate_page_content_before_hayro() {
    let mut doc = Document::load_mem(&fixture_pdf(&[(100.0, 100.0)], None)).unwrap();
    let page_id = doc.get_pages()[&1];
    let content_id = doc
        .get_object(page_id)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Contents")
        .unwrap()
        .as_reference()
        .unwrap();
    let stream = doc
        .get_object_mut(content_id)
        .unwrap()
        .as_stream_mut()
        .unwrap();
    stream.dict.set("Filter", "FlateDecode");
    stream.set_content(vec![0xff; 32]);
    let session = Session::open(&save(doc)).unwrap();
    assert!(
        session
            .render_png(0, 1.0)
            .unwrap_err()
            .to_string()
            .contains("content preflight")
    );
}

#[test]
fn renderer_accepts_valid_flate_page_content() {
    let mut doc = Document::load_mem(&fixture_pdf(&[(100.0, 100.0)], None)).unwrap();
    let page_id = doc.get_pages()[&1];
    let content_id = doc
        .get_object(page_id)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Contents")
        .unwrap()
        .as_reference()
        .unwrap();
    let stream = doc
        .get_object_mut(content_id)
        .unwrap()
        .as_stream_mut()
        .unwrap();
    stream.set_plain_content(b"q 0 0 10 10 re f Q".to_vec());
    stream.compress().unwrap();
    let session = Session::open(&save(doc)).unwrap();
    assert!(!session.render_png(0, 1.0).unwrap().is_empty());
}

#[test]
fn renderer_rejects_malformed_page_operations_before_hayro() {
    let mut doc = Document::load_mem(&fixture_pdf(&[(100.0, 100.0)], None)).unwrap();
    let page_id = doc.get_pages()[&1];
    let content_id = doc
        .get_object(page_id)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Contents")
        .unwrap()
        .as_reference()
        .unwrap();
    let malformed = b"q [1 2".to_vec();
    assert!(Content::decode_strict(&malformed).is_err());
    doc.get_object_mut(content_id)
        .unwrap()
        .as_stream_mut()
        .unwrap()
        .set_plain_content(malformed);
    let session = Session::open(&save(doc)).unwrap();
    assert!(
        session
            .render_png(0, 1.0)
            .unwrap_err()
            .to_string()
            .contains("content preflight")
    );
}

#[test]
fn renderer_rejects_huge_source_image_independent_of_small_output() {
    let mut doc = Document::load_mem(&fixture_pdf(&[(10.0, 10.0)], None)).unwrap();
    let image = doc.add_object(image_stream(5_000, 5_000, None, vec![0]));
    install_page_xobject(&mut doc, b"Huge", Object::Reference(image));
    let session = Session::open(&save(doc)).unwrap();
    assert!(
        session
            .render_png(0, 1.0)
            .unwrap_err()
            .to_string()
            .contains("source image")
    );
}

#[test]
fn renderer_accepts_a_small_bounded_raw_image() {
    let mut doc = Document::load_mem(&fixture_pdf(&[(10.0, 10.0)], None)).unwrap();
    let image = doc.add_object(image_stream(1, 1, None, vec![0, 0, 0]));
    install_page_xobject(&mut doc, b"Im0", Object::Reference(image));
    let page = doc.get_pages()[&1];
    let content = doc
        .get_object(page)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Contents")
        .unwrap()
        .as_reference()
        .unwrap();
    doc.get_object_mut(content)
        .unwrap()
        .as_stream_mut()
        .unwrap()
        .set_plain_content(b"q 10 0 0 10 0 0 cm /Im0 Do Q".to_vec());
    let session = Session::open(&save(doc)).unwrap();
    assert!(!session.render_png(0, 1.0).unwrap().is_empty());
}

#[test]
fn renderer_rejects_dct_until_encoded_dimensions_are_safely_validated() {
    let mut doc = Document::load_mem(&fixture_pdf(&[(10.0, 10.0)], None)).unwrap();
    let image = doc.add_object(image_stream(
        i64::from(u16::MAX),
        i64::from(u16::MAX),
        Some(b"DCTDecode"),
        vec![0xff, 0xd8, 0xff, 0xd9],
    ));
    install_page_xobject(&mut doc, b"Dct", Object::Reference(image));
    let session = Session::open(&save(doc)).unwrap();
    assert!(
        session
            .render_png(0, 1.0)
            .unwrap_err()
            .to_string()
            .contains("DCTDecode")
    );
}

#[test]
fn renderer_rejects_recursive_form_resource_cycles() {
    let mut doc = Document::load_mem(&fixture_pdf(&[(10.0, 10.0)], None)).unwrap();
    let form_id = doc.new_object_id();
    let form = Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 1.into(), 1.into()],
            "Resources" => dictionary! {
                "XObject" => dictionary! { "Self" => form_id },
            },
        },
        Vec::new(),
    );
    doc.objects.insert(form_id, form.into());
    install_page_xobject(&mut doc, b"Form", Object::Reference(form_id));
    let session = Session::open(&save(doc)).unwrap();
    assert!(
        session
            .render_png(0, 1.0)
            .unwrap_err()
            .to_string()
            .contains("cycle")
    );
}

#[test]
fn renderer_preflights_annotation_appearance_resources() {
    let mut doc = Document::load_mem(&fixture_pdf(&[(10.0, 10.0)], None)).unwrap();
    let image = doc.add_object(image_stream(5_000, 5_000, None, vec![0]));
    let form = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 1.into(), 1.into()],
            "Resources" => dictionary! {
                "XObject" => dictionary! { "Huge" => image },
            },
        },
        Vec::new(),
    ));
    let annot = doc.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Square",
        "Rect" => vec![0.into(), 0.into(), 1.into(), 1.into()],
        "F" => 4,
        "AP" => dictionary! { "N" => form },
    });
    let page = doc.get_pages()[&1];
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", vec![Object::Reference(annot)]);
    let session = Session::open(&save(doc)).unwrap();
    assert!(
        session
            .render_png(0, 1.0)
            .unwrap_err()
            .to_string()
            .contains("source image")
    );
}

#[test]
fn renderer_rejects_a_bound_form_without_bbox() {
    let mut doc = Document::load_mem(&fixture_pdf(&[(20.0, 20.0)], None)).unwrap();
    let form = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "Resources" => Dictionary::new(),
        },
        b"0 0 1 1 re f".to_vec(),
    ));
    install_page_xobject(&mut doc, b"Form", Object::Reference(form));
    set_page_content(&mut doc, b"/Form Do");
    let session = Session::open(&save(doc)).unwrap();
    assert!(
        session
            .render_png(0, 1.0)
            .unwrap_err()
            .to_string()
            .contains("BBox")
    );
}

#[test]
fn renderer_rejects_malformed_or_unmodeled_form_fields() {
    let cases = vec![
        (
            "BBox",
            Object::Array(vec![0.into(), 0.into(), 1.into()]),
            "BBox",
        ),
        (
            "BBox",
            Object::Array(vec![
                0.into(),
                0.into(),
                1.into(),
                Object::string_literal("bad"),
            ]),
            "BBox",
        ),
        (
            "Matrix",
            Object::Array(vec![1.into(), 0.into(), 0.into(), 1.into(), 0.into()]),
            "Matrix",
        ),
        (
            "Matrix",
            Object::Array(vec![
                1.into(),
                0.into(),
                0.into(),
                1.into(),
                0.into(),
                Object::string_literal("bad"),
            ]),
            "Matrix",
        ),
        ("FormType", 2.into(), "FormType"),
        (
            "Group",
            dictionary! { "S" => "Transparency" }.into(),
            "Group",
        ),
    ];
    for (key, value, expected) in cases {
        let mut doc = Document::load_mem(&fixture_pdf(&[(20.0, 20.0)], None)).unwrap();
        let mut form_dictionary = dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 1.into(), 1.into()],
            "Resources" => Dictionary::new(),
        };
        form_dictionary.set(key, value);
        let form = doc.add_object(Stream::new(form_dictionary, Vec::new()));
        install_page_xobject(&mut doc, b"Form", Object::Reference(form));
        set_page_content(&mut doc, b"/Form Do");
        let session = Session::open(&save(doc)).unwrap();
        assert!(
            session
                .render_png(0, 1.0)
                .unwrap_err()
                .to_string()
                .contains(expected),
            "Form field {key} must fail closed"
        );
    }
}

#[test]
fn renderer_accepts_audited_form_geometry() {
    let mut doc = Document::load_mem(&fixture_pdf(&[(20.0, 20.0)], None)).unwrap();
    let form = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "FormType" => 1,
            "BBox" => vec![0.into(), 0.into(), 1.into(), 1.into()],
            "Matrix" => vec![1.into(), 0.into(), 0.into(), 1.into(), 1.into(), 1.into()],
            "Resources" => Dictionary::new(),
        },
        b"0 0 1 1 re f".to_vec(),
    ));
    install_page_xobject(&mut doc, b"Form", Object::Reference(form));
    set_page_content(&mut doc, b"/Form Do");
    let session = Session::open(&save(doc)).unwrap();
    assert!(!session.render_png(0, 1.0).unwrap().is_empty());
}

#[test]
fn renderer_enforces_total_source_decode_budget_across_images() {
    let mut doc = Document::load_mem(&fixture_pdf(&[(10.0, 10.0)], None)).unwrap();
    let mut compressed = image_stream(2_048, 2_048, None, vec![0; 2_048 * 2_048 * 3]);
    compressed.compress().unwrap();
    for index in 0..3 {
        let image = doc.add_object(compressed.clone());
        install_page_xobject(
            &mut doc,
            format!("Im{index}").as_bytes(),
            Object::Reference(image),
        );
    }
    let session = Session::open(&save(doc)).unwrap();
    assert!(
        session
            .render_png(0, 1.0)
            .unwrap_err()
            .to_string()
            .contains("source resource bytes")
    );
}

#[test]
fn renderer_rejects_unmodeled_fontfile_even_when_unused() {
    let mut doc = Document::load_mem(&fixture_pdf(&[(100.0, 100.0)], None)).unwrap();
    let mut font_file = Stream::new(Dictionary::new(), vec![0; 2 * 1024 * 1024]);
    font_file.compress().unwrap();
    let font_file = doc.add_object(font_file);
    let descriptor = doc.add_object(dictionary! {
        "Type" => "FontDescriptor",
        "FontName" => "Helvetica",
        "FontFile2" => font_file,
    });
    let font = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
        "FontDescriptor" => descriptor,
    });
    let page = doc.get_pages()[&1];
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .get_mut(b"Resources")
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Font", dictionary! { "Unsafe" => font });
    let session = Session::open(&save(doc)).unwrap();
    assert!(
        session
            .render_png(0, 1.0)
            .unwrap_err()
            .to_string()
            .contains("Font")
    );
}

#[test]
fn renderer_rejects_icc_shading_pattern_and_extgstate_categories() {
    for category in ["ColorSpace", "Shading", "Pattern", "ExtGState"] {
        let mut doc = Document::load_mem(&fixture_pdf(&[(100.0, 100.0)], None)).unwrap();
        let mut payload = Stream::new(Dictionary::new(), vec![0; 1024 * 1024]);
        payload.compress().unwrap();
        let payload = doc.add_object(payload);
        let value = match category {
            "ColorSpace" => dictionary! {
                "CS0" => vec![Object::Name(b"ICCBased".to_vec()), Object::Reference(payload)],
            },
            _ => dictionary! { "Unsafe" => payload },
        };
        let page = doc.get_pages()[&1];
        doc.get_object_mut(page)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .get_mut(b"Resources")
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set(category, value);
        let session = Session::open(&save(doc)).unwrap();
        assert!(
            session
                .render_png(0, 1.0)
                .unwrap_err()
                .to_string()
                .contains(category)
        );
    }
}

#[test]
fn renderer_validates_indexed_color_space_shape_and_lookup() {
    let invalid_spaces = vec![
        vec![
            Object::Name(b"Indexed".to_vec()),
            Object::Name(b"DeviceRGB".to_vec()),
            999.into(),
            Object::string_literal(vec![0; 6]),
        ],
        vec![
            Object::Name(b"Indexed".to_vec()),
            Object::Name(b"ICCBased".to_vec()),
            1.into(),
            Object::string_literal(vec![0; 6]),
        ],
        vec![
            Object::Name(b"Indexed".to_vec()),
            Object::Name(b"DeviceRGB".to_vec()),
            1.into(),
            Object::string_literal(vec![0; 3]),
        ],
    ];
    for color_space in invalid_spaces {
        let mut doc = Document::load_mem(&fixture_pdf(&[(10.0, 10.0)], None)).unwrap();
        let image = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => 1,
                "Height" => 1,
                "ColorSpace" => color_space,
                "BitsPerComponent" => 8,
            },
            vec![0],
        ));
        install_page_xobject(&mut doc, b"Indexed", Object::Reference(image));
        let session = Session::open(&save(doc)).unwrap();
        assert!(
            session
                .render_png(0, 1.0)
                .unwrap_err()
                .to_string()
                .contains("Indexed")
        );
    }
}

#[test]
fn renderer_rejects_stream_lookup_in_indexed_color_space() {
    let mut doc = Document::load_mem(&fixture_pdf(&[(10.0, 10.0)], None)).unwrap();
    let mut lookup = Stream::new(Dictionary::new(), vec![0; 1024 * 1024]);
    lookup.compress().unwrap();
    let lookup = doc.add_object(lookup);
    let image = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => 1,
            "Height" => 1,
            "ColorSpace" => vec![
                Object::Name(b"Indexed".to_vec()),
                Object::Name(b"DeviceRGB".to_vec()),
                1.into(),
                Object::Reference(lookup),
            ],
            "BitsPerComponent" => 8,
        },
        vec![0],
    ));
    install_page_xobject(&mut doc, b"Indexed", Object::Reference(image));
    let session = Session::open(&save(doc)).unwrap();
    assert!(
        session
            .render_png(0, 1.0)
            .unwrap_err()
            .to_string()
            .contains("direct lookup")
    );
}

#[test]
fn renderer_accepts_a_bounded_direct_indexed_lookup() {
    let mut doc = Document::load_mem(&fixture_pdf(&[(10.0, 10.0)], None)).unwrap();
    let image = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => 1,
            "Height" => 1,
            "ColorSpace" => vec![
                Object::Name(b"Indexed".to_vec()),
                Object::Name(b"DeviceRGB".to_vec()),
                1.into(),
                Object::string_literal(vec![0, 0, 0, 255, 255, 255]),
            ],
            "BitsPerComponent" => 8,
        },
        vec![0],
    ));
    install_page_xobject(&mut doc, b"Indexed", Object::Reference(image));
    let session = Session::open(&save(doc)).unwrap();
    assert!(!session.render_png(0, 1.0).unwrap().is_empty());
}

#[test]
fn renderer_rejects_unknown_and_wrongly_typed_operators() {
    for content in [
        b"q ZZ Q".as_slice(),
        b"BT 12 /F1 Tf ET".as_slice(),
        b"1 gs".as_slice(),
        b"1 sh".as_slice(),
        b"0.5 /P scn".as_slice(),
    ] {
        let session = Session::open(&fixture_with_page_content(content)).unwrap();
        assert!(
            session
                .render_png(0, 1.0)
                .unwrap_err()
                .to_string()
                .contains("operator")
        );
    }
}

#[test]
fn renderer_rejects_unlinked_font_and_xobject_names() {
    for content in [
        b"BT /Missing 12 Tf (x) Tj ET".as_slice(),
        b"q /Missing Do Q".as_slice(),
    ] {
        let session = Session::open(&fixture_with_page_content(content)).unwrap();
        assert!(
            session
                .render_png(0, 1.0)
                .unwrap_err()
                .to_string()
                .contains("resource")
        );
    }
}

#[test]
fn renderer_rejects_text_show_without_an_explicit_tf_in_each_text_object() {
    for content in [
        b"BT (x) Tj ET".as_slice(),
        b"BT [(x)] TJ ET".as_slice(),
        b"BT (x) ' ET".as_slice(),
        b"BT 0 0 (x) \" ET".as_slice(),
        b"BT /F1 12 Tf ET BT (x) Tj ET".as_slice(),
    ] {
        let session = Session::open(&fixture_with_page_content(content)).unwrap();
        assert!(
            session
                .render_png(0, 1.0)
                .unwrap_err()
                .to_string()
                .contains("font")
        );
    }
}

#[test]
fn renderer_restores_text_font_state_on_graphics_restore() {
    let rejected =
        Session::open(&fixture_with_page_content(b"BT q /F1 12 Tf Q (x) Tj ET")).unwrap();
    assert!(
        rejected
            .render_png(0, 1.0)
            .unwrap_err()
            .to_string()
            .contains("font")
    );

    let accepted = Session::open(&fixture_with_page_content(
        b"BT /F1 12 Tf q Q 5 5 Td (x) Tj ET",
    ))
    .unwrap();
    assert!(!accepted.render_png(0, 1.0).unwrap().is_empty());
}

#[test]
fn renderer_does_not_inherit_page_font_state_into_a_form() {
    let mut doc = Document::load_mem(&fixture_pdf(&[(20.0, 20.0)], None)).unwrap();
    let page = doc.get_pages()[&1];
    let font = doc
        .get_object(page)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Resources")
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Font")
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"F1")
        .unwrap()
        .clone();
    let form = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 10.into(), 10.into()],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        },
        b"BT 1 1 Td (x) Tj ET".to_vec(),
    ));
    install_page_xobject(&mut doc, b"Form", Object::Reference(form));
    set_page_content(&mut doc, b"BT /F1 12 Tf ET /Form Do");
    let session = Session::open(&save(doc)).unwrap();
    assert!(
        session
            .render_png(0, 1.0)
            .unwrap_err()
            .to_string()
            .contains("font")
    );
}

#[test]
fn delete_page_rejects_annotation_and_structure_ownership_atomically() {
    let mut with_note =
        Session::open(&fixture_pdf(&[(100.0, 100.0), (100.0, 100.0)], None)).unwrap();
    with_note
        .apply(Edit::AddNote {
            page: 0,
            bounds: rect(),
            contents: "owned".into(),
            color: black(),
        })
        .unwrap();
    let before = with_note.bytes().unwrap();
    assert!(with_note.apply(Edit::DeletePage { page: 0 }).is_err());
    assert_eq!(with_note.bytes().unwrap(), before);

    let mut doc =
        Document::load_mem(&fixture_pdf(&[(100.0, 100.0), (100.0, 100.0)], None)).unwrap();
    let page = doc.get_pages()[&1];
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("StructParents", 0);
    let mut structured = Session::open(&save(doc)).unwrap();
    let before = structured.bytes().unwrap();
    assert!(structured.apply(Edit::DeletePage { page: 0 }).is_err());
    assert_eq!(structured.bytes().unwrap(), before);
}

#[test]
fn delete_page_rejects_global_acroform_relationship_atomically() {
    let mut doc =
        Document::load_mem(&fixture_pdf(&[(100.0, 100.0), (100.0, 100.0)], None)).unwrap();
    let page = doc.get_pages()[&1];
    let field = doc.add_object(dictionary! { "FT" => "Btn", "P" => page });
    let form = doc.add_object(dictionary! { "Fields" => vec![Object::Reference(field)] });
    let catalog = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    doc.get_object_mut(catalog)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("AcroForm", form);
    let mut session = Session::open(&save(doc)).unwrap();
    let before = session.bytes().unwrap();
    assert!(session.apply(Edit::DeletePage { page: 0 }).is_err());
    assert_eq!(session.bytes().unwrap(), before);
}

#[test]
fn total_history_bytes_evict_old_snapshots_before_count_limit() {
    let mut session = Session::open(&fixture_pdf(&[(200.0, 120.0)], None)).unwrap();
    let text = "x".repeat(60_000);
    for _ in 0..60 {
        session
            .apply(Edit::AddText {
                page: 0,
                bounds: rect(),
                text: text.clone(),
                font_size: 8.0,
                color: black(),
            })
            .unwrap();
    }
    let mut undo_count = 0;
    while session.undo().is_ok() {
        undo_count += 1;
    }
    assert!(
        undo_count < 32,
        "byte budget must bind before snapshot count"
    );
}
