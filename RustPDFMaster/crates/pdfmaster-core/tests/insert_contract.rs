use lopdf::{Document, Object, dictionary};
use pdfmaster_core::{Color, Edit, Rect, Session};

fn source(prefix: &str, count: usize, width: f32) -> Session {
    let mut session = Session::blank(width, 100., count).unwrap();
    for page in 0..count {
        session
            .apply(Edit::AddText {
                page,
                bounds: Rect {
                    x: 5.,
                    y: 15.,
                    width: 90.,
                    height: 20.,
                },
                text: format!("{prefix}-{page}"),
                font_size: 12.,
                color: Color {
                    r: 0.,
                    g: 0.,
                    b: 0.,
                },
            })
            .unwrap();
    }
    session.mark_saved();
    session
}

fn with_catalog(session: &Session, key: &str, value: Object) -> Vec<u8> {
    let mut doc = Document::load_mem(&session.bytes().unwrap()).unwrap();
    doc.catalog_mut().unwrap().set(key, value);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}

#[test]
fn insertion_preserves_document_language_without_invalid_page_language_or_amplification() {
    let base = source("language", 1, 120.);
    let same = with_catalog(&base, "Lang", Object::string_literal("en-US"));
    let different = with_catalog(&base, "Lang", Object::string_literal("zh-CN"));
    let invalid = with_catalog(
        &base,
        "Lang",
        dictionary! {"Huge"=>vec![1.into(),2.into()]}.into(),
    );
    let mut target = Session::open(&same).unwrap();
    let before = target.bytes().unwrap();
    let info = target.info();
    for incoming in [&different, &invalid, &base.bytes().unwrap()] {
        assert!(target.insert_pdf(incoming, 1).is_err());
        assert_eq!(target.bytes().unwrap(), before);
        assert_eq!(target.info(), info);
    }
    target.insert_pdf(&same, 1).unwrap();
    let doc = Document::load_mem(&target.bytes().unwrap()).unwrap();
    assert_eq!(
        doc.catalog()
            .unwrap()
            .get(b"Lang")
            .unwrap()
            .as_str()
            .unwrap(),
        b"en-US"
    );
    for id in doc.get_pages().values() {
        assert!(!doc.get_object(*id).unwrap().as_dict().unwrap().has(b"Lang"));
    }
}

#[test]
fn insertion_rejects_global_forms_and_tagged_structure_before_changing_target() {
    let incoming = source("import", 1, 120.);
    let mut target = source("target", 1, 180.);
    let before = target.bytes().unwrap();
    let info = target.info();
    for key in ["AcroForm", "StructTreeRoot", "MarkInfo"] {
        let bytes = with_catalog(&incoming, key, dictionary! {"Marker"=>true}.into());
        assert!(target.insert_pdf(&bytes, 1).is_err(), "{key}");
        assert_eq!(target.bytes().unwrap(), before);
        assert_eq!(target.info(), info);
    }
}

#[test]
fn insertion_resolves_named_links_and_keeps_target_metadata() {
    let incoming = source("import", 2, 120.);
    let mut doc = Document::load_mem(&incoming.bytes().unwrap()).unwrap();
    let first = doc.get_pages()[&1];
    let last = doc.get_pages()[&2];
    doc.catalog_mut().unwrap().set("Names", dictionary! {"Dests"=>dictionary! {
        "Names"=>vec![Object::string_literal("last-page"), Object::Array(vec![last.into(), "Fit".into()])]
    }});
    let link = doc.add_object(dictionary! {"Subtype"=>"Link", "Rect"=>vec![0.into(),0.into(),10.into(),10.into()], "Dest"=>Object::string_literal("last-page")});
    doc.get_object_mut(first)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", vec![link.into()]);
    let mut incoming_bytes = Vec::new();
    doc.save_to(&mut incoming_bytes).unwrap();
    let target = source("target", 1, 180.);
    let target = with_catalog(
        &target,
        "CustomMetadata",
        Object::string_literal("keep-target"),
    );
    let mut target = Session::open(&target).unwrap();
    target.insert_pdf(&incoming_bytes, 1).unwrap();
    let doc = Document::load_mem(&target.bytes().unwrap()).unwrap();
    assert_eq!(
        doc.catalog()
            .unwrap()
            .get(b"CustomMetadata")
            .unwrap()
            .as_str()
            .unwrap(),
        b"keep-target"
    );
    let pages = doc.get_pages();
    let annotation = doc
        .get_object(pages[&2])
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Annots")
        .unwrap()
        .as_array()
        .unwrap()[0]
        .as_reference()
        .unwrap();
    let destination = doc
        .get_object(annotation)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Dest")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(destination[0], Object::Reference(pages[&3]));
}

#[test]
fn insertion_preserves_order_geometry_raster_and_one_transaction_history() {
    let incoming = source("import", 2, 120.);
    let incoming_bytes = incoming.bytes().unwrap();
    for index in 0..=2 {
        let mut target = source("target", 2, 180.);
        let before = target.bytes().unwrap();
        let previous_info = target.info();
        target.insert_pdf(&incoming_bytes, index).unwrap();
        assert_eq!(target.info().page_count, 4);
        assert!(target.is_dirty());
        for page in 0..4 {
            if (index..index + 2).contains(&page) {
                assert!(
                    target
                        .text(page)
                        .unwrap()
                        .contains(&format!("import-{}", page - index))
                );
                assert_eq!(
                    target.render_png(page, 1.).unwrap(),
                    incoming.render_png(page - index, 1.).unwrap()
                );
            } else {
                let original = if page < index { page } else { page - 2 };
                assert!(
                    target
                        .text(page)
                        .unwrap()
                        .contains(&format!("target-{original}"))
                );
                assert_eq!(target.info().pages[page], previous_info.pages[original]);
            }
        }
        let merged = target.bytes().unwrap();
        let reopened = Session::open(&merged).unwrap();
        assert_eq!(reopened.info().pages, target.info().pages);
        target.undo().unwrap();
        assert_eq!(target.bytes().unwrap(), before);
        assert!(!target.is_dirty());
        target.redo().unwrap();
        assert_eq!(target.bytes().unwrap(), merged);
        assert_eq!(incoming.bytes().unwrap(), incoming_bytes);
    }
}

#[test]
fn insertion_rejects_invalid_input_index_and_page_budget_without_clearing_redo() {
    let incoming = source("import", 1, 120.).bytes().unwrap();
    let mut target = source("target", 1, 180.);
    target
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    target.undo().unwrap();
    let before = target.bytes().unwrap();
    let info = target.info();
    for (bytes, index) in [
        (b"not PDF".as_slice(), 0),
        (incoming.as_slice(), 2),
        (incoming.as_slice(), usize::MAX),
    ] {
        assert!(target.insert_pdf(bytes, index).is_err());
        assert_eq!(target.bytes().unwrap(), before);
        assert_eq!(target.info(), info);
    }
    let mut full = Session::blank(100., 100., 4096).unwrap();
    let before = full.bytes().unwrap();
    assert!(full.insert_pdf(&incoming, 4096).is_err());
    assert_eq!(full.bytes().unwrap(), before);
}

#[test]
fn insertion_remaps_note_page_backlinks_and_imported_page_destinations() {
    let mut incoming = source("import", 2, 120.);
    incoming
        .apply(Edit::AddNote {
            page: 0,
            bounds: Rect {
                x: 10.,
                y: 10.,
                width: 20.,
                height: 20.,
            },
            contents: "中文 🦀".into(),
            color: Color {
                r: 1.,
                g: 1.,
                b: 0.,
            },
        })
        .unwrap();
    let mut doc = Document::load_mem(&incoming.bytes().unwrap()).unwrap();
    let first = doc.get_pages()[&1];
    let last = doc.get_pages()[&2];
    let link = doc.add_object(dictionary! {"Subtype"=>"Link", "Rect"=>vec![10.into(),10.into(),30.into(),30.into()], "P"=>first,"Dest"=>vec![Object::Reference(last),Object::Name(b"Fit".to_vec())]});
    doc.get_object_mut(first)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .get_mut(b"Annots")
        .unwrap()
        .as_array_mut()
        .unwrap()
        .push(Object::Reference(link));
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    let mut target = source("target", 1, 180.);
    target.insert_pdf(&bytes, 1).unwrap();
    let merged = target.bytes().unwrap();
    let reopened = Session::open(&merged).unwrap();
    assert_eq!(reopened.notes(1).unwrap().notes[0].contents, "中文 🦀");
    let parsed = Document::load_mem(&merged).unwrap();
    let pages = parsed.get_pages();
    let annots = parsed
        .get_object(pages[&2])
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Annots")
        .unwrap()
        .as_array()
        .unwrap();
    let link = parsed
        .get_object(annots.last().unwrap().as_reference().unwrap())
        .unwrap()
        .as_dict()
        .unwrap();
    assert_eq!(link.get(b"P").unwrap(), &Object::Reference(pages[&2]));
    assert_eq!(
        link.get(b"Dest").unwrap().as_array().unwrap()[0],
        Object::Reference(pages[&3])
    );
}
