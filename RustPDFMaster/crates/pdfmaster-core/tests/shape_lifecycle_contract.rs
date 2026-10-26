use lopdf::{Document, Object, dictionary};
use pdfmaster_core::{Edit, Session};
use serde_json::{Value, json};

fn create(s: &mut Session) {
    let edit = json!({"kind":"add_shape","page":0,"shape":"rectangle","bounds":{"x":20.,"y":20.,"width":100.,"height":60.},"stroke":{"r":1.,"g":0.,"b":0.},"fill":null,"line_width":2.});
    s.apply(serde_json::from_value(edit).unwrap()).unwrap();
}

fn expected() -> Value {
    json!({"annotation_index":0,"shape":"rectangle","bounds":{"x":20.,"y":20.,"width":100.,"height":60.},"stroke":{"r":1.,"g":0.,"b":0.},"fill":null,"line_width":2.})
}

fn update() -> Value {
    json!({"kind":"update_shape","page":0,"expected":expected(),"bounds":{"x":40.,"y":30.,"width":110.,"height":70.},"stroke":{"r":0.,"g":1.,"b":0.},"fill":{"r":0.,"g":0.,"b":1.},"line_width":4.})
}

fn apply(s: &mut Session, request: Value) {
    let edit: Edit = serde_json::from_value(request).expect("shape lifecycle wire variant");
    s.apply(edit).unwrap();
}

fn annots(d: &Document, page: u32) -> Vec<Object> {
    let p = d
        .get_object(d.get_pages()[&page])
        .unwrap()
        .as_dict()
        .unwrap();
    d.dereference(p.get(b"Annots").unwrap())
        .unwrap()
        .1
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn update_shape_persists_bounds_style_and_one_undo_without_flattening() {
    let mut s = Session::blank(200., 120., 1).unwrap();
    create(&mut s);
    let before = s.bytes().unwrap();
    apply(&mut s, update());
    let changed = s.bytes().unwrap();
    let d = Document::load_mem(&changed).unwrap();
    let members = annots(&d, 1);
    let a = d.dereference(&members[0]).unwrap().1.as_dict().unwrap();
    assert_eq!(a.get(b"Subtype").unwrap().as_name().unwrap(), b"Square");
    assert_eq!(
        a.get(b"Rect").unwrap().as_array().unwrap(),
        &vec![40.into(), 30.into(), 150.into(), 100.into()]
    );
    assert_eq!(
        a.get(b"IC").unwrap().as_array().unwrap(),
        &vec![0.into(), 0.into(), 1.into()]
    );
    assert!(a.get(b"AP").unwrap().as_dict().unwrap().has(b"N"));
    assert!(
        !Session::open(&changed)
            .unwrap()
            .render_png(0, 1.)
            .unwrap()
            .is_empty()
    );
    s.undo().unwrap();
    assert_eq!(s.bytes().unwrap(), before);
    s.redo().unwrap();
    assert_eq!(s.bytes().unwrap(), changed);
}

#[test]
fn delete_shape_is_undoable_and_stale_selection_never_changes_bytes_or_redo() {
    let mut s = Session::blank(200., 120., 1).unwrap();
    create(&mut s);
    let before = s.bytes().unwrap();
    apply(
        &mut s,
        json!({"kind":"delete_shape","page":0,"expected":expected()}),
    );
    let d = Document::load_mem(&s.bytes().unwrap()).unwrap();
    assert!(annots(&d, 1).is_empty());
    s.undo().unwrap();
    assert_eq!(s.bytes().unwrap(), before);
    let info = s.info();
    let mut stale = expected();
    stale["line_width"] = json!(3.);
    let edit: Edit =
        serde_json::from_value(json!({"kind":"delete_shape","page":0,"expected":stale})).unwrap();
    assert!(s.apply(edit).is_err());
    assert_eq!(s.bytes().unwrap(), before);
    assert_eq!(s.info(), info);
}

#[test]
fn shared_shape_popup_cow_preserves_other_page_and_metadata_and_rebinds_links() {
    let mut s = Session::blank(200., 120., 2).unwrap();
    create(&mut s);
    let mut d = Document::load_mem(&s.bytes().unwrap()).unwrap();
    let pages = d.get_pages();
    let old = annots(&d, 1)[0].as_reference().unwrap();
    let popup=d.add_object(dictionary! {"Type"=>"Annot","Subtype"=>"Popup","Rect"=>vec![5.into(),5.into(),15.into(),15.into()],"Parent"=>old,"P"=>pages[&1]});
    d.get_object_mut(old)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Popup", popup);
    d.get_object_mut(old)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Contents", lopdf::text_string("shape metadata 🦀"));
    for id in pages.values() {
        d.get_object_mut(*id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Annots", vec![old.into(), popup.into()]);
    }
    let mut bytes = Vec::new();
    d.save_to(&mut bytes).unwrap();
    let mut s = Session::open(&bytes).unwrap();
    apply(&mut s, update());
    let changed = Document::load_mem(&s.bytes().unwrap()).unwrap();
    assert_eq!(annots(&changed, 2), annots(&d, 2));
    assert_eq!(changed.get_object(old).unwrap(), d.get_object(old).unwrap());
    assert_eq!(
        changed.get_object(popup).unwrap(),
        d.get_object(popup).unwrap()
    );
    let root = annots(&changed, 1)[0].as_reference().unwrap();
    assert_ne!(root, old);
    let a = changed.get_object(root).unwrap().as_dict().unwrap();
    assert_eq!(
        a.get(b"Contents").unwrap(),
        d.get_object(old)
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"Contents")
            .unwrap()
    );
    let new_popup = a.get(b"Popup").unwrap().as_reference().unwrap();
    assert_ne!(new_popup, popup);
    assert_eq!(
        changed
            .get_object(new_popup)
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"Parent")
            .unwrap()
            .as_reference()
            .unwrap(),
        root
    );
    // Removing the updated annotation removes only its owned popup membership.
    let mut current = expected();
    current["bounds"] = update()["bounds"].clone();
    current["stroke"] = update()["stroke"].clone();
    current["fill"] = update()["fill"].clone();
    current["line_width"] = json!(4.);
    apply(
        &mut s,
        json!({"kind":"delete_shape","page":0,"expected":current}),
    );
    let removed = Document::load_mem(&s.bytes().unwrap()).unwrap();
    assert!(annots(&removed, 1).is_empty());
    assert_eq!(annots(&removed, 2), annots(&d, 2));
}

#[test]
fn shape_lifecycle_rejects_tagged_reply_and_wrong_subtype_without_mutation() {
    for case in 0..3 {
        let mut s = Session::blank(200., 120., 1).unwrap();
        create(&mut s);
        let mut d = Document::load_mem(&s.bytes().unwrap()).unwrap();
        let id = annots(&d, 1)[0].as_reference().unwrap();
        match case {
            0 => {
                d.get_object_mut(id)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .set("StructParent", 0);
            }
            1 => {
                d.get_object_mut(id)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .set("IRT", id);
            }
            _ => {
                d.get_object_mut(id)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .set("Subtype", "Text");
            }
        }
        let mut bytes = Vec::new();
        d.save_to(&mut bytes).unwrap();
        let mut s = Session::open(&bytes).unwrap();
        let before = s.bytes().unwrap();
        let info = s.info();
        for request in [
            update(),
            json!({"kind":"delete_shape","page":0,"expected":expected()}),
        ] {
            let edit: Edit = serde_json::from_value(request).expect("shape lifecycle variant");
            assert!(s.apply(edit).is_err());
            assert_eq!(s.bytes().unwrap(), before);
            assert_eq!(s.info(), info);
        }
    }
}

#[test]
fn shapes_list_is_read_only_and_preserves_indices_with_uneditable_styles() {
    let mut s = Session::blank(200., 120., 1).unwrap();
    create(&mut s);
    let mut d = Document::load_mem(&s.bytes().unwrap()).unwrap();
    let original = annots(&d, 1)[0].clone();
    let mut unsupported = d
        .dereference(&original)
        .unwrap()
        .1
        .as_dict()
        .unwrap()
        .clone();
    unsupported.set("CA", 0.5_f32);
    let extended = d.add_object(unsupported);
    let page = d.get_pages()[&1];
    d.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", vec![extended.into(), original]);
    let mut bytes = Vec::new();
    d.save_to(&mut bytes).unwrap();
    let s = Session::open(&bytes).unwrap();
    let before = s.bytes().unwrap();
    let info = s.info();
    let list = s.shapes(0).unwrap();
    assert_eq!(list.unsupported_count, 1);
    assert_eq!(list.shapes.len(), 1);
    let mut selected = expected();
    selected["annotation_index"] = json!(1);
    assert_eq!(serde_json::to_value(&list.shapes[0]).unwrap(), selected);
    assert_eq!(s.bytes().unwrap(), before);
    assert_eq!(s.info(), info);
    assert!(s.shapes(1).is_err());
}

#[test]
fn ellipse_update_clears_fill_and_reopens_as_an_independent_circle() {
    let mut s = Session::blank(200., 120., 1).unwrap();
    let mut add = update();
    add["kind"] = json!("add_shape");
    add["shape"] = json!("ellipse");
    add.as_object_mut().unwrap().remove("expected");
    apply(&mut s, add);
    let expected = serde_json::to_value(&s.shapes(0).unwrap().shapes[0]).unwrap();
    let mut request = update();
    request["expected"] = expected;
    request["fill"] = Value::Null;
    apply(&mut s, request);
    let bytes = s.bytes().unwrap();
    let reopened = Session::open(&bytes).unwrap();
    assert!(reopened.shapes(0).unwrap().shapes[0].fill.is_none());
    assert_eq!(
        reopened.shapes(0).unwrap().shapes[0].shape,
        pdfmaster_core::ShapeKind::Ellipse
    );
    let d = Document::load_mem(&bytes).unwrap();
    let a = annots(&d, 1);
    let a = d.dereference(&a[0]).unwrap().1.as_dict().unwrap();
    assert!(!a.has(b"IC"));
    assert_eq!(a.get(b"Subtype").unwrap().as_name().unwrap(), b"Circle");
    assert!(!reopened.render_png(0, 1.).unwrap().is_empty());
}

#[test]
fn rejected_shape_updates_leave_bytes_history_and_redo_unchanged() {
    let mut s = Session::blank(200., 120., 1).unwrap();
    create(&mut s);
    apply(&mut s, update());
    s.undo().unwrap();
    let before = s.bytes().unwrap();
    let info = s.info();
    for (key, value) in [
        ("line_width", json!(0.)),
        ("line_width", json!(100.)),
        ("stroke", json!({"r":2.,"g":0.,"b":0.})),
        ("fill", json!({"r":0.,"g":-1.,"b":0.})),
        (
            "bounds",
            json!({"x":190.,"y":30.,"width":110.,"height":70.}),
        ),
    ] {
        let mut edit = update();
        edit[key] = value;
        assert!(s.apply(serde_json::from_value(edit).unwrap()).is_err());
        assert_eq!(s.bytes().unwrap(), before);
        assert_eq!(s.info(), info);
    }
}

#[test]
fn shape_lifecycle_rejects_cross_page_incoming_reply_without_mutation() {
    let mut s = Session::blank(200., 120., 2).unwrap();
    create(&mut s);
    let mut d = Document::load_mem(&s.bytes().unwrap()).unwrap();
    let root = annots(&d, 1)[0].clone();
    let reply=d.add_object(dictionary! {"Type"=>"Annot","Subtype"=>"Text","Rect"=>vec![0.into(),0.into(),20.into(),20.into()],"IRT"=>root});
    let page = d.get_pages()[&2];
    d.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", vec![reply.into()]);
    let mut bytes = Vec::new();
    d.save_to(&mut bytes).unwrap();
    let mut s = Session::open(&bytes).unwrap();
    let before = s.bytes().unwrap();
    let info = s.info();
    for edit in [
        update(),
        json!({"kind":"delete_shape","page":0,"expected":expected()}),
    ] {
        assert!(s.apply(serde_json::from_value(edit).unwrap()).is_err());
        assert_eq!(s.bytes().unwrap(), before);
        assert_eq!(s.info(), info);
    }
}

#[test]
fn lifecycle_rejects_multiple_reverse_popups_even_without_a_forward_link() {
    let mut s = Session::blank(200., 120., 1).unwrap();
    create(&mut s);
    let mut d = Document::load_mem(&s.bytes().unwrap()).unwrap();
    let root = annots(&d, 1)[0].clone();
    let popup = dictionary! {"Type"=>"Annot","Subtype"=>"Popup","Rect"=>vec![0.into(),0.into(),20.into(),20.into()],"Parent"=>root.clone()};
    let first = d.add_object(popup.clone());
    let second = d.add_object(popup);
    let page = d.get_pages()[&1];
    d.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", vec![root, first.into(), second.into()]);
    let mut bytes = Vec::new();
    d.save_to(&mut bytes).unwrap();
    let mut s = Session::open(&bytes).unwrap();
    let before = s.bytes().unwrap();
    let info = s.info();
    for edit in [
        json!({"kind":"delete_shape","page":0,"expected":expected()}),
        update(),
    ] {
        assert!(
            s.apply(serde_json::from_value(edit).unwrap()).is_err(),
            "ambiguous reverse popups must be refused"
        );
        assert_eq!(s.bytes().unwrap(), before);
        assert_eq!(s.info(), info);
    }
}
