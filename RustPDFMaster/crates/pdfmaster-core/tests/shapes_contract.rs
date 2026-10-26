use std::io::Cursor;

use lopdf::{Document, Object, dictionary};
use pdfmaster_core::{Edit, Session};
use serde_json::{Value, json};

fn shape(kind: &str, fill: bool) -> Value {
    json!({"kind":"add_shape","page":0,"shape":kind,
        "bounds":{"x":20.,"y":20.,"width":100.,"height":60.},
        "stroke":{"r":1.,"g":0.,"b":0.},
        "fill":if fill {json!({"r":0.,"g":0.,"b":1.})} else {Value::Null},
        "line_width":4.})
}

fn apply(session: &mut Session, value: Value) {
    let edit: Edit = serde_json::from_value(value).expect("shape edit must be admitted");
    session.apply(edit).unwrap();
}

fn annotation(document: &Document, page: u32) -> lopdf::Dictionary {
    let page = document
        .get_object(document.get_pages()[&page])
        .unwrap()
        .as_dict()
        .unwrap();
    let (_, values) = document.dereference(page.get(b"Annots").unwrap()).unwrap();
    let (_, value) = document
        .dereference(&values.as_array().unwrap()[0])
        .unwrap();
    value.as_dict().unwrap().clone()
}

fn pixel(session: &Session, x: usize, y: usize) -> [u8; 3] {
    let bytes = session.render_png(0, 1.).unwrap();
    let mut reader = png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    let stride = info.color_type.samples();
    let offset = (y * info.width as usize + x) * stride;
    pixels[offset..offset + 3].try_into().unwrap()
}

#[test]
fn rectangle_is_a_real_annotation_with_independent_appearance_and_roundtrip_history() {
    let mut s = Session::blank(200., 120., 2).unwrap();
    let original = s.bytes().unwrap();
    apply(&mut s, shape("rectangle", true));
    let changed = s.bytes().unwrap();
    let d = Document::load_mem(&changed).unwrap();
    let a = annotation(&d, 1);
    assert_eq!(a.get(b"Subtype").unwrap().as_name().unwrap(), b"Square");
    assert_eq!(
        a.get(b"Rect").unwrap().as_array().unwrap(),
        &vec![20.into(), 20.into(), 120.into(), 80.into()]
    );
    assert_eq!(
        a.get(b"P").unwrap().as_reference().unwrap(),
        d.get_pages()[&1]
    );
    assert_eq!(a.get(b"F").unwrap().as_i64().unwrap(), 4);
    assert_eq!(
        a.get(b"BS")
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"W")
            .unwrap()
            .as_float()
            .unwrap(),
        4.
    );
    assert!(a.has(b"IC"));
    assert!(a.get(b"AP").unwrap().as_dict().unwrap().has(b"N"));
    assert_eq!(s.notes(0).unwrap().unsupported_count, 1);
    assert_eq!(pixel(&s, 70, 70), [0, 0, 255]);
    assert_eq!(pixel(&s, 10, 10), [255, 255, 255]);
    let clean = Session::open(&changed).unwrap();
    assert_eq!(pixel(&clean, 70, 70), [0, 0, 255]);
    assert!(!clean.info().can_undo);
    s.undo().unwrap();
    assert_eq!(s.bytes().unwrap(), original);
    assert!(!s.info().dirty);
    s.redo().unwrap();
    assert_eq!(s.bytes().unwrap(), changed);
}

#[test]
fn ellipse_uses_curves_and_keeps_unfilled_interior_and_corners_clear() {
    let mut s = Session::blank(200., 120., 1).unwrap();
    apply(&mut s, shape("ellipse", false));
    let d = Document::load_mem(&s.bytes().unwrap()).unwrap();
    let a = annotation(&d, 1);
    assert_eq!(a.get(b"Subtype").unwrap().as_name().unwrap(), b"Circle");
    assert!(!a.has(b"IC"));
    assert_eq!(pixel(&s, 70, 70), [255, 255, 255]);
    assert_eq!(pixel(&s, 22, 42), [255, 255, 255]);
    let ap = a.get(b"AP").unwrap().as_dict().unwrap().get(b"N").unwrap();
    let (_, ap) = d.dereference(ap).unwrap();
    let content = lopdf::content::Content::decode(&ap.as_stream().unwrap().content).unwrap();
    assert_eq!(
        content
            .operations
            .iter()
            .filter(|op| op.operator == "c")
            .count(),
        4
    );
    assert!(content.operations.iter().any(|op| op.operator == "S"));
    assert!(pixel(&s, 70, 42)[0] > 200);
    assert!(pixel(&s, 70, 42)[1] < 30);
}

#[test]
fn shape_addition_keeps_shared_annotation_arrays_and_page_contents_untouched() {
    let blank = Session::blank(200., 120., 2).unwrap().bytes().unwrap();
    let mut d = Document::load_mem(&blank).unwrap();
    let annots = d.add_object(Vec::<Object>::new());
    for id in d.get_pages().values() {
        d.get_object_mut(*id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Annots", annots);
    }
    let mut bytes = Vec::new();
    d.save_to(&mut bytes).unwrap();
    let mut s = Session::open(&bytes).unwrap();
    let before = d.clone();
    apply(&mut s, shape("rectangle", false));
    let after = Document::load_mem(&s.bytes().unwrap()).unwrap();
    assert_eq!(
        after.get_object(annots).unwrap(),
        before.get_object(annots).unwrap()
    );
    assert_eq!(s.notes(1).unwrap().unsupported_count, 0);
    for (number, id) in before.get_pages() {
        let old = before.get_object(id).unwrap().as_dict().unwrap();
        let new = after
            .get_object(after.get_pages()[&number])
            .unwrap()
            .as_dict()
            .unwrap();
        assert_eq!(old.get(b"Contents").unwrap(), new.get(b"Contents").unwrap());
        assert_eq!(
            old.get(b"Resources").unwrap(),
            new.get(b"Resources").unwrap()
        );
    }
}

#[test]
fn bad_geometry_style_and_page_preserve_bytes_history_and_redo() {
    for (field, value) in [
        ("bounds", json!({"x":-1.,"y":20.,"width":100.,"height":60.})),
        ("bounds", json!({"x":20.,"y":20.,"width":0.,"height":60.})),
        (
            "bounds",
            json!({"x":190.,"y":20.,"width":100.,"height":60.}),
        ),
        ("stroke", json!({"r":1.1,"g":0.,"b":0.})),
        ("fill", json!({"r":0.,"g":-0.1,"b":0.})),
        ("line_width", json!(0.)),
        ("line_width", json!(-1.)),
        ("line_width", json!(60.)),
        ("page", json!(99)),
    ] {
        let mut s = Session::blank(200., 120., 1).unwrap();
        s.apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
        s.undo().unwrap();
        let before = s.bytes().unwrap();
        let info = s.info();
        let mut request = shape("rectangle", true);
        request[field] = value;
        let edit: Edit = serde_json::from_value(request).expect("shape request schema");
        assert!(s.apply(edit).is_err(), "invalid {field}");
        assert_eq!(s.bytes().unwrap(), before);
        assert_eq!(s.info(), info);
    }
}

#[test]
fn rotated_pages_keep_source_coordinates_and_annotation_budget_is_atomic() {
    let mut s = Session::blank(200., 120., 1).unwrap();
    s.apply(Edit::Rotate {
        page: 0,
        degrees: 90,
    })
    .unwrap();
    apply(&mut s, shape("ellipse", true));
    assert_eq!(s.info().pages[0].rotation, 90);
    assert!(!s.render_png(0, 1.).unwrap().is_empty());
    let mut d = Document::load_mem(&s.bytes().unwrap()).unwrap();
    let a=d.add_object(dictionary! {"Type"=>"Annot","Subtype"=>"Square","Rect"=>vec![0.into(),0.into(),10.into(),10.into()]});
    let id = d.get_pages()[&1];
    d.get_object_mut(id)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", vec![Object::Reference(a); 1024]);
    let mut bytes = Vec::new();
    d.save_to(&mut bytes).unwrap();
    let mut s = Session::open(&bytes).unwrap();
    let before = s.bytes().unwrap();
    let edit: Edit = serde_json::from_value(shape("rectangle", false)).unwrap();
    assert!(s.apply(edit).is_err());
    assert_eq!(s.bytes().unwrap(), before);
}

#[test]
fn nonfinite_rust_api_values_are_rejected_without_mutation() {
    use pdfmaster_core::{Color, Rect, ShapeKind};
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        for field in 0..5 {
            let mut bounds = Rect {
                x: 20.,
                y: 20.,
                width: 100.,
                height: 60.,
            };
            let mut stroke = Color {
                r: 1.,
                g: 0.,
                b: 0.,
            };
            let mut line = 2.;
            match field {
                0 => bounds.x = value,
                1 => bounds.width = value,
                2 => stroke.r = value,
                3 => line = value,
                _ => bounds.height = value,
            }
            let mut s = Session::blank(200., 120., 1).unwrap();
            let before = s.bytes().unwrap();
            assert!(
                s.apply(Edit::AddShape {
                    page: 0,
                    bounds,
                    shape: ShapeKind::Rectangle,
                    stroke,
                    fill: None,
                    line_width: line
                })
                .is_err()
            );
            assert_eq!(s.bytes().unwrap(), before);
            assert!(!s.info().can_undo);
        }
    }
}

#[test]
fn strict_shared_admission_currently_refuses_off_page_or_gray_note_metadata_atomically() {
    // Both classes can occur in valid PDFs. Preserve this known limitation as
    // an explicit compatibility regression, not as a PDF validity assertion.
    for gray_note in [false, true] {
        let base = Session::blank(200., 120., 1).unwrap();
        let mut d = Document::load_mem(&base.bytes().unwrap()).unwrap();
        let mut annotation = if gray_note {
            dictionary! {"Type"=>"Annot", "Subtype"=>"Text", "Rect"=>vec![10.into(),10.into(),30.into(),30.into()], "Contents"=>"Gray note", "C"=>vec![0.5_f32.into()]}
        } else {
            dictionary! {"Type"=>"Annot", "Subtype"=>"Square", "Rect"=>vec![(-10).into(),10.into(),30.into(),30.into()]}
        };
        let page = d.get_pages()[&1];
        annotation.set("P", page);
        let id = d.add_object(annotation);
        d.get_object_mut(page)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Annots", vec![Object::Reference(id)]);
        let mut bytes = Vec::new();
        d.save_to(&mut bytes).unwrap();
        let mut s = Session::open(&bytes).unwrap();
        let before = s.bytes().unwrap();
        let info = s.info();
        let edit: Edit = serde_json::from_value(shape("rectangle", false)).unwrap();
        assert!(s.apply(edit).is_err());
        assert_eq!(s.bytes().unwrap(), before);
        assert_eq!(s.info(), info);
    }
}

#[test]
fn shapes_survive_extraction_and_insertion_with_appearance_and_page_backlinks() {
    let mut s = Session::blank(200., 120., 1).unwrap();
    apply(&mut s, shape("rectangle", true));
    let extracted = s.extract_pages(&[0]).unwrap();
    let extracted = Session::open(&extracted).unwrap();
    assert_eq!(pixel(&extracted, 70, 70), [0, 0, 255]);
    let mut target = Session::blank(200., 120., 1).unwrap();
    target.insert_pdf(&extracted.bytes().unwrap(), 0).unwrap();
    assert_eq!(pixel(&target, 70, 70), [0, 0, 255]);
    let d = Document::load_mem(&target.bytes().unwrap()).unwrap();
    assert_eq!(
        annotation(&d, 1).get(b"P").unwrap().as_reference().unwrap(),
        d.get_pages()[&1]
    );
}
