use lopdf::{Document, Object, dictionary};
use pdfmaster_core::{Color, Edit, Rect, Session};

fn labeled_source() -> Session {
    let mut session = Session::blank(200., 120., 3).unwrap();
    for (page, text) in ["FIRST-PAGE", "EXCLUDED-PAGE", "LAST-PAGE"]
        .iter()
        .enumerate()
    {
        session
            .apply(Edit::AddText {
                page,
                bounds: Rect {
                    x: 10.,
                    y: 10.,
                    width: 170.,
                    height: 30.,
                },
                text: (*text).into(),
                font_size: 12.,
                color: Color {
                    r: 0.,
                    g: 0.,
                    b: 0.,
                },
            })
            .unwrap();
    }
    session
        .apply(Edit::Rotate {
            page: 2,
            degrees: 90,
        })
        .unwrap();
    session
}

fn save(mut document: Document) -> Vec<u8> {
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

#[test]
fn extraction_preserves_indexset_order_geometry_and_text_without_mutating_source_or_history() {
    let session = labeled_source();
    let before = session.bytes().unwrap();
    let info = session.info();
    let output = session.extract_pages(&[2, 0]).unwrap();
    let extracted = Session::open(&output).unwrap();
    assert_eq!(extracted.info().page_count, 2);
    assert!(extracted.text(0).unwrap().contains("FIRST-PAGE"));
    assert!(extracted.text(1).unwrap().contains("LAST-PAGE"));
    assert_eq!(
        extracted.info().pages,
        vec![info.pages[0].clone(), info.pages[2].clone()]
    );
    assert!(!extracted.info().dirty);
    assert!(!extracted.info().can_undo);
    assert_eq!(session.bytes().unwrap(), before);
    assert_eq!(session.info(), info);
    assert!(session.info().dirty);
}

#[test]
fn extraction_rejects_empty_invalid_duplicate_and_over_budget_selection_read_only() {
    let mut session = labeled_source();
    session.undo().unwrap();
    let before = session.bytes().unwrap();
    let info = session.info();
    for selection in [vec![], vec![3], vec![usize::MAX], vec![0, 0], vec![0; 4097]] {
        assert!(session.extract_pages(&selection).is_err());
        assert_eq!(session.bytes().unwrap(), before);
        assert_eq!(session.info(), info);
    }
    assert!(session.info().can_redo);
}

#[test]
fn extraction_builds_a_fresh_graph_without_unselected_page_streams_or_global_private_metadata() {
    let mut document = Document::load_mem(&labeled_source().bytes().unwrap()).unwrap();
    document.add_object(Object::string_literal("ORPHAN-PRIVATE-DATA"));
    let info =
        document.add_object(dictionary! {"Subject"=>Object::string_literal("GLOBAL-PRIVATE-DATA")});
    document.trailer.set("Info", info);
    let source = save(document);
    for marker in [
        "EXCLUDED-PAGE",
        "ORPHAN-PRIVATE-DATA",
        "GLOBAL-PRIVATE-DATA",
    ] {
        assert!(
            source
                .windows(marker.len())
                .any(|bytes| bytes == marker.as_bytes())
        );
    }
    let session = Session::open(&source).unwrap();
    let output = session.extract_pages(&[0, 2]).unwrap();
    for marker in [
        "EXCLUDED-PAGE",
        "ORPHAN-PRIVATE-DATA",
        "GLOBAL-PRIVATE-DATA",
    ] {
        assert!(
            !output
                .windows(marker.len())
                .any(|bytes| bytes == marker.as_bytes()),
            "retained {marker}"
        );
    }
    let parsed = Document::load_mem(&output).unwrap();
    assert_eq!(parsed.get_pages().len(), 2);
    assert_eq!(
        parsed
            .objects
            .values()
            .filter(|object| object
                .as_dict()
                .ok()
                .and_then(|dict| dict.get(b"Type").ok())
                .and_then(|value| value.as_name().ok())
                == Some(b"Page"))
            .count(),
        2
    );
    assert!(!parsed.trailer.has(b"Info"));
    assert_eq!(session.bytes().unwrap(), source);
}

#[test]
fn extraction_remaps_note_popup_cycles_page_backlinks_and_links_between_selected_pages() {
    let mut document = Document::load_mem(&labeled_source().bytes().unwrap()).unwrap();
    let first = document.get_pages()[&1];
    let last = document.get_pages()[&3];
    let note = document.new_object_id();
    let popup = document.add_object(dictionary! {"Type"=>"Annot", "Subtype"=>"Popup", "Rect"=>vec![10.into(), 10.into(), 40.into(), 40.into()], "Parent"=>note, "P"=>first});
    document.objects.insert(note, dictionary! {"Type"=>"Annot", "Subtype"=>"Text", "Rect"=>vec![10.into(), 10.into(), 40.into(), 40.into()], "Contents"=>lopdf::text_string("中文 🦀"), "Popup"=>popup, "P"=>first}.into());
    let link = document.add_object(dictionary! {"Type"=>"Annot", "Subtype"=>"Link", "Rect"=>vec![50.into(), 10.into(), 80.into(), 40.into()], "Dest"=>vec![Object::Reference(last), Object::Name(b"Fit".to_vec())], "P"=>first});
    document
        .get_object_mut(first)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set(
            "Annots",
            vec![
                Object::Reference(note),
                Object::Reference(popup),
                Object::Reference(link),
            ],
        );
    let source = save(document);
    let session = Session::open(&source).unwrap();
    let output = session.extract_pages(&[0, 2]).unwrap();
    let parsed = Document::load_mem(&output).unwrap();
    let new_first = parsed.get_pages()[&1];
    let new_last = parsed.get_pages()[&2];
    let annots = parsed
        .get_object(new_first)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Annots")
        .unwrap()
        .as_array()
        .unwrap();
    let root = parsed
        .get_object(annots[0].as_reference().unwrap())
        .unwrap()
        .as_dict()
        .unwrap();
    let popup = parsed
        .get_object(annots[1].as_reference().unwrap())
        .unwrap()
        .as_dict()
        .unwrap();
    let link = parsed
        .get_object(annots[2].as_reference().unwrap())
        .unwrap()
        .as_dict()
        .unwrap();
    assert_eq!(root.get(b"Popup").unwrap(), &annots[1]);
    assert_eq!(popup.get(b"Parent").unwrap(), &annots[0]);
    assert_eq!(popup.get(b"P").unwrap(), &Object::Reference(new_first));
    assert_eq!(
        link.get(b"Dest").unwrap().as_array().unwrap()[0],
        Object::Reference(new_last)
    );
    assert_eq!(
        Session::open(&output).unwrap().notes(0).unwrap().notes[0].contents,
        "中文 🦀"
    );
    assert!(
        session.extract_pages(&[0]).is_err(),
        "must not copy a link destination's excluded page"
    );
    assert_eq!(session.bytes().unwrap(), source);
}

#[test]
fn extraction_materializes_inherited_resources_crop_and_rotation() {
    let mut document = Document::load_mem(&labeled_source().bytes().unwrap()).unwrap();
    let first = document.get_pages()[&1];
    let parent = document
        .get_object(first)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Parent")
        .unwrap()
        .as_reference()
        .unwrap();
    let resources = document
        .get_object_mut(first)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .remove(b"Resources")
        .unwrap();
    document
        .get_object_mut(first)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .remove(b"Rotate");
    let pages = document
        .get_object_mut(parent)
        .unwrap()
        .as_dict_mut()
        .unwrap();
    pages.set("Resources", resources);
    pages.set("CropBox", vec![0.into(), 0.into(), 200.into(), 120.into()]);
    pages.set("Rotate", 180);
    let session = Session::open(&save(document)).unwrap();
    let output = session.extract_pages(&[0]).unwrap();
    let extracted = Session::open(&output).unwrap();
    assert_eq!(extracted.info().pages[0], session.info().pages[0]);
    assert!(extracted.text(0).unwrap().contains("FIRST-PAGE"));
    assert_eq!(
        extracted.render_png(0, 1.).unwrap(),
        session.render_png(0, 1.).unwrap()
    );
}

#[test]
fn extraction_resolves_legacy_and_name_tree_destinations_into_selected_page_links() {
    for legacy in [false, true] {
        let mut document = Document::load_mem(&labeled_source().bytes().unwrap()).unwrap();
        let first = document.get_pages()[&1];
        let last = document.get_pages()[&3];
        let destination = document.add_object(
            dictionary! {"D"=>vec![Object::Reference(last), Object::Name(b"Fit".to_vec())]},
        );
        if legacy {
            document
                .catalog_mut()
                .unwrap()
                .set("Dests", dictionary! {"chapter"=>destination});
        } else {
            let leaf = document.add_object(dictionary! {"Names"=>vec![Object::string_literal("chapter"), Object::Reference(destination)]});
            document.catalog_mut().unwrap().set(
                "Names",
                dictionary! {"Dests"=>dictionary! {"Kids"=>vec![Object::Reference(leaf)]}},
            );
        }
        let name = if legacy {
            Object::Name(b"chapter".to_vec())
        } else {
            Object::string_literal("chapter")
        };
        let direct = document.add_object(dictionary! {"Subtype"=>"Link", "Dest"=>name.clone()});
        let action = document.add_object(dictionary! {"S"=>"GoTo", "D"=>name});
        let indirect = document.add_object(dictionary! {"Subtype"=>"Link", "A"=>action});
        document
            .get_object_mut(first)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set(
                "Annots",
                vec![Object::Reference(direct), Object::Reference(indirect)],
            );
        let source = save(document);
        let session = Session::open(&source).unwrap();
        let output = session.extract_pages(&[0, 2]).unwrap();
        let parsed = Document::load_mem(&output).unwrap();
        let pages = parsed.get_pages();
        let annots = parsed
            .get_object(pages[&1])
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"Annots")
            .unwrap()
            .as_array()
            .unwrap();
        for (index, key) in [(0, b"Dest".as_slice()), (1, b"D".as_slice())] {
            let mut dict = parsed
                .get_object(annots[index].as_reference().unwrap())
                .unwrap()
                .as_dict()
                .unwrap();
            if index == 1 {
                dict = parsed
                    .get_object(dict.get(b"A").unwrap().as_reference().unwrap())
                    .unwrap()
                    .as_dict()
                    .unwrap();
            }
            assert_eq!(
                dict.get(key).unwrap().as_array().unwrap()[0],
                Object::Reference(pages[&2])
            );
        }
        assert!(session.extract_pages(&[0]).is_err());
        assert_eq!(session.bytes().unwrap(), source);
    }
}

#[test]
fn extraction_rejects_invalid_local_destinations_and_cyclic_name_trees_read_only() {
    for destination in [
        Object::Array(vec![]),
        Object::Array(vec![0.into(), Object::Name(b"Fit".to_vec())]),
        Object::string_literal("missing"),
    ] {
        let mut document = Document::load_mem(&labeled_source().bytes().unwrap()).unwrap();
        let page = document.get_pages()[&1];
        let tree = document.new_object_id();
        document.objects.insert(
            tree,
            dictionary! {"Kids"=>vec![Object::Reference(tree)]}.into(),
        );
        document
            .catalog_mut()
            .unwrap()
            .set("Names", dictionary! {"Dests"=>tree});
        let link = document.add_object(dictionary! {"Subtype"=>"Link", "Dest"=>destination});
        document
            .get_object_mut(page)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Annots", vec![Object::Reference(link)]);
        let source = save(document);
        let session = Session::open(&source).unwrap();
        assert!(session.extract_pages(&[0]).is_err());
        assert_eq!(session.bytes().unwrap(), source);
    }
}

#[test]
fn extraction_rejects_missing_references_tagging_and_widget_ownership_without_source_changes() {
    for entry in [
        dictionary! {"Subtype"=>"Widget"},
        dictionary! {"StructParent"=>0},
        dictionary! {"AppearanceResource"=>Object::Reference((900_000,0))},
    ] {
        let mut document = Document::load_mem(&labeled_source().bytes().unwrap()).unwrap();
        let page = document.get_pages()[&1];
        let annotation = document.add_object(entry);
        document
            .get_object_mut(page)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Annots", vec![Object::Reference(annotation)]);
        let source = save(document);
        let session = Session::open(&source).unwrap();
        assert!(session.extract_pages(&[0]).is_err());
        assert_eq!(session.bytes().unwrap(), source);
    }
}

#[test]
fn extraction_handles_indirect_widget_subtypes_and_local_action_names() {
    for widget in [true, false] {
        let mut document = Document::load_mem(&labeled_source().bytes().unwrap()).unwrap();
        let page = document.get_pages()[&1];
        let name = document.add_object(Object::Name(if widget {
            b"Widget".to_vec()
        } else {
            b"GoTo".to_vec()
        }));
        document.catalog_mut().unwrap().set(
            "Dests",
            dictionary! {"first"=>vec![Object::Reference(page), Object::Name(b"Fit".to_vec())]},
        );
        let entry = if widget {
            dictionary! {"Subtype"=>name}
        } else {
            dictionary! {"Subtype"=>"Link", "A"=>dictionary! {"S"=>name,"D"=>Object::Name(b"first".to_vec())}}
        };
        let annotation = document.add_object(entry);
        document
            .get_object_mut(page)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Annots", vec![Object::Reference(annotation)]);
        let source = save(document);
        let session = Session::open(&source).unwrap();
        let result = session.extract_pages(&[0]);
        if widget {
            assert!(result.is_err());
        } else {
            let parsed = Document::load_mem(&result.unwrap()).unwrap();
            let new_page = parsed.get_pages()[&1];
            let annots = parsed
                .get_object(new_page)
                .unwrap()
                .as_dict()
                .unwrap()
                .get(b"Annots")
                .unwrap()
                .as_array()
                .unwrap();
            let action = parsed
                .get_object(annots[0].as_reference().unwrap())
                .unwrap()
                .as_dict()
                .unwrap()
                .get(b"A")
                .unwrap()
                .as_dict()
                .unwrap();
            assert_eq!(
                action.get(b"D").unwrap().as_array().unwrap()[0],
                Object::Reference(new_page)
            );
        }
        assert_eq!(session.bytes().unwrap(), source);
    }
}
