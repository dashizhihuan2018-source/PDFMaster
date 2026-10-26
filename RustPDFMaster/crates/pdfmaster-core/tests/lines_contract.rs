use lopdf::{Document, Object, dictionary};
use pdfmaster_core::{Edit, Session};
use serde_json::{Value, json};

fn request(start: [f32; 2], end: [f32; 2], endings: [&str; 2]) -> Value {
    json!({"kind":"add_line","page":0,"start":{"x":start[0],"y":start[1]},"end":{"x":end[0],"y":end[1]},"endings":endings,"stroke":{"r":1.,"g":0.,"b":0.},"fill":null,"line_width":4.})
}
fn apply(s: &mut Session, v: Value) {
    s.apply(serde_json::from_value::<Edit>(v).expect("line edit must exist"))
        .unwrap();
}
fn members(d: &Document, page: u32) -> Vec<Object> {
    d.dereference(
        d.get_object(d.get_pages()[&page])
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"Annots")
            .unwrap(),
    )
    .unwrap()
    .1
    .as_array()
    .unwrap()
    .clone()
}
fn annotation(d: &Document, page: u32) -> lopdf::Dictionary {
    d.dereference(&members(d, page)[0])
        .unwrap()
        .1
        .as_dict()
        .unwrap()
        .clone()
}
fn floats(a: &lopdf::Dictionary, key: &[u8]) -> Vec<f32> {
    a.get(key)
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_float().unwrap())
        .collect()
}
fn pixel(s: &Session, x: usize, y: usize) -> [u8; 3] {
    let mut reader = png::Decoder::new(std::io::Cursor::new(s.render_png(0, 1.).unwrap()))
        .read_info()
        .unwrap();
    let mut data = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut data).unwrap();
    let offset = (y * info.width as usize + x) * info.color_type.samples();
    data[offset..offset + 3].try_into().unwrap()
}

#[test]
fn line_creation_has_real_endpoints_endings_appearance_pixels_and_atomic_history() {
    for endings in [
        ["none", "none"],
        ["none", "open_arrow"],
        ["open_arrow", "open_arrow"],
    ] {
        let mut s = Session::blank(200., 120., 1).unwrap();
        let original = s.bytes().unwrap();
        apply(&mut s, request([30., 60.], [170., 60.], endings));
        let bytes = s.bytes().unwrap();
        let d = Document::load_mem(&bytes).unwrap();
        let a = annotation(&d, 1);
        assert_eq!(a.get(b"Subtype").unwrap().as_name().unwrap(), b"Line");
        assert_eq!(floats(&a, b"L"), vec![30., 60., 170., 60.]);
        assert!(a.has(b"AP"));
        assert_eq!(pixel(&s, 100, 60), [255, 0, 0]);
        assert_eq!(pixel(&s, 100, 40), [255, 255, 255]);
        let reopened = Session::open(&bytes).unwrap();
        assert_eq!(pixel(&reopened, 100, 60), [255, 0, 0]);
        let record = serde_json::to_value(reopened.shapes(0).unwrap()).unwrap();
        assert_eq!(record["shapes"][0]["shape"], "line");
        assert_eq!(record["shapes"][0]["line"]["endings"], json!(endings));
        s.undo().unwrap();
        assert_eq!(s.bytes().unwrap(), original);
        s.redo().unwrap();
        assert_eq!(s.bytes().unwrap(), bytes);
    }
}

#[test]
fn vertical_page_edge_line_and_reverse_direction_are_preserved_on_resize() {
    let mut s = Session::blank(200., 120., 1).unwrap();
    let mut edge = request([200., 0.], [200., 120.], ["none", "open_arrow"]);
    edge["line_width"] = json!(32.);
    apply(&mut s, edge);
    let d = Document::load_mem(&s.bytes().unwrap()).unwrap();
    let r = floats(&annotation(&d, 1), b"Rect");
    assert!(r[0] >= 0. && r[1] >= 0. && r[2] <= 200. && r[3] <= 120.);
    s.undo().unwrap();
    apply(
        &mut s,
        request([170., 30.], [30., 90.], ["open_arrow", "open_arrow"]),
    );
    let expected = serde_json::to_value(s.shapes(0).unwrap()).unwrap()["shapes"][0].clone();
    let b = &expected["bounds"];
    let x = b["x"].as_f64().unwrap();
    let y = b["y"].as_f64().unwrap();
    let w = b["width"].as_f64().unwrap();
    let h = b["height"].as_f64().unwrap();
    let before = s.bytes().unwrap();
    apply(
        &mut s,
        json!({"kind":"update_shape","page":0,"expected":expected,"bounds":{"x":10.,"y":10.,"width":w*0.5,"height":h*0.5},"stroke":{"r":0.,"g":0.,"b":1.},"fill":null,"line_width":2.}),
    );
    let changed = s.bytes().unwrap();
    let a = annotation(&Document::load_mem(&changed).unwrap(), 1);
    let points = floats(&a, b"L");
    for (actual, want) in points.iter().zip([
        10. + (170. - x) * 0.5,
        10. + (30. - y) * 0.5,
        10. + (30. - x) * 0.5,
        10. + (90. - y) * 0.5,
    ]) {
        assert!((f64::from(*actual) - want).abs() < 0.001);
    }
    assert!(points[0] > points[2]);
    s.undo().unwrap();
    assert_eq!(s.bytes().unwrap(), before);
    s.redo().unwrap();
    assert_eq!(s.bytes().unwrap(), changed);
}

#[test]
fn imported_diamond_closed_arrow_and_dash_survive_cow_update_and_delete_undo() {
    let mut s = Session::blank(200., 120., 2).unwrap();
    apply(&mut s, request([30., 30.], [150., 90.], ["none", "none"]));
    let mut d = Document::load_mem(&s.bytes().unwrap()).unwrap();
    let root = members(&d, 1)[0].as_reference().unwrap();
    let a = d.get_object_mut(root).unwrap().as_dict_mut().unwrap();
    a.set(
        "LE",
        vec![
            Object::Name(b"Diamond".to_vec()),
            Object::Name(b"ClosedArrow".to_vec()),
        ],
    );
    a.set(
        "BS",
        dictionary! {"W"=>4,"S"=>"D","D"=>vec![3.into(),2.into()]},
    );
    a.set("Contents", lopdf::text_string("preserve 🦀"));
    let other = d.get_pages()[&2];
    d.get_object_mut(other)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", vec![root.into()]);
    let old = d.get_object(root).unwrap().clone();
    let mut bytes = Vec::new();
    d.save_to(&mut bytes).unwrap();
    let mut s = Session::open(&bytes).unwrap();
    let expected = serde_json::to_value(s.shapes(0).unwrap()).unwrap()["shapes"][0].clone();
    assert_eq!(
        expected["line"]["endings"],
        json!(["diamond", "closed_arrow"])
    );
    apply(
        &mut s,
        json!({"kind":"update_shape","page":0,"expected":expected,"bounds":{"x":10.,"y":10.,"width":180.,"height":100.},"stroke":{"r":0.,"g":1.,"b":0.},"fill":{"r":0.,"g":0.,"b":1.},"line_width":7.5}),
    );
    let changed = s.bytes().unwrap();
    let out = Document::load_mem(&changed).unwrap();
    assert_eq!(out.get_object(root).unwrap(), &old);
    assert_eq!(members(&out, 2), vec![root.into()]);
    let a = annotation(&out, 1);
    assert_eq!(
        floats(a.get(b"BS").unwrap().as_dict().unwrap(), b"D"),
        vec![3., 2.]
    );
    assert_eq!(
        a.get(b"LE").unwrap(),
        old.as_dict().unwrap().get(b"LE").unwrap()
    );
    assert_eq!(
        a.get(b"Contents").unwrap(),
        old.as_dict().unwrap().get(b"Contents").unwrap()
    );
    let latest = serde_json::to_value(s.shapes(0).unwrap()).unwrap()["shapes"][0].clone();
    apply(
        &mut s,
        json!({"kind":"delete_shape","page":0,"expected":latest}),
    );
    let deleted = Document::load_mem(&s.bytes().unwrap()).unwrap();
    assert!(members(&deleted, 1).is_empty());
    assert_eq!(members(&deleted, 2), vec![root.into()]);
    s.undo().unwrap();
    assert_eq!(s.bytes().unwrap(), changed);
    let info = s.info();
    assert!(
        s.apply(
            serde_json::from_value(json!({"kind":"delete_shape","page":0,"expected":expected}))
                .unwrap()
        )
        .is_err()
    );
    assert_eq!(s.bytes().unwrap(), changed);
    assert_eq!(s.info(), info);
}

#[test]
fn invalid_line_geometry_width_and_endings_leave_redo_unchanged() {
    let mut s = Session::blank(200., 120., 1).unwrap();
    apply(&mut s, request([30., 30.], [150., 90.], ["none", "none"]));
    s.undo().unwrap();
    let before = s.bytes().unwrap();
    let info = s.info();
    for (start, end) in [
        ([0., 0.], [0., 0.]),
        ([-1., 30.], [40., 30.]),
        ([20., 20.], [201., 30.]),
    ] {
        assert!(
            s.apply(serde_json::from_value(request(start, end, ["none", "none"])).unwrap())
                .is_err()
        );
        assert_eq!(s.bytes().unwrap(), before);
        assert_eq!(s.info(), info);
    }
    for width in [0., 0.1, 32.1, 10000.] {
        let mut v = request([20., 20.], [40., 20.], ["none", "none"]);
        v["line_width"] = json!(width);
        assert!(s.apply(serde_json::from_value(v).unwrap()).is_err());
        assert_eq!(s.bytes().unwrap(), before);
        assert_eq!(s.info(), info);
    }
    assert!(
        serde_json::from_value::<Edit>(request([20., 20.], [40., 20.], ["none", "bogus"])).is_err()
    );
}

#[test]
fn extended_line_opacity_and_unknown_endings_are_not_silently_repainted() {
    let mut s = Session::blank(200., 120., 1).unwrap();
    apply(&mut s, request([30., 60.], [170., 60.], ["none", "none"]));
    let original = s.bytes().unwrap();
    for (key, value) in [
        ("ca", Object::Real(0.5)),
        ("Cap", Object::Boolean(true)),
        (
            "LE",
            Object::Array(vec![
                Object::Name(b"None".to_vec()),
                Object::Name(b"Unknown".to_vec()),
            ]),
        ),
    ] {
        let mut d = Document::load_mem(&original).unwrap();
        let id = members(&d, 1)[0].as_reference().unwrap();
        d.get_object_mut(id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set(key, value);
        let mut bytes = Vec::new();
        d.save_to(&mut bytes).unwrap();
        let s = Session::open(&bytes).unwrap();
        let list = s.shapes(0).unwrap();
        assert!(
            list.shapes.is_empty(),
            "extended {key} must not be editable"
        );
        assert_eq!(list.unsupported_count, 1);
    }
}

#[test]
fn legacy_border_dash_is_preserved_and_malformed_patterns_are_rejected() {
    let mut s = Session::blank(200., 120., 1).unwrap();
    apply(&mut s, request([30., 60.], [170., 60.], ["none", "none"]));
    let mut d = Document::load_mem(&s.bytes().unwrap()).unwrap();
    let id = members(&d, 1)[0].as_reference().unwrap();
    let a = d.get_object_mut(id).unwrap().as_dict_mut().unwrap();
    a.remove(b"BS");
    a.set(
        "Border",
        vec![
            0.into(),
            0.into(),
            4.into(),
            Object::Array(vec![3.into(), 2.into()]),
        ],
    );
    let mut bytes = Vec::new();
    d.save_to(&mut bytes).unwrap();
    let mut s = Session::open(&bytes).unwrap();
    let expected = serde_json::to_value(s.shapes(0).unwrap()).unwrap()["shapes"][0].clone();
    assert_eq!(expected["line"]["dash_pattern"], json!([3., 2.]));
    apply(
        &mut s,
        json!({"kind":"update_shape","page":0,"expected":expected,"bounds":{"x":10.,"y":10.,"width":180.,"height":100.},"stroke":{"r":0.,"g":1.,"b":0.},"fill":null,"line_width":32.}),
    );
    assert_eq!(
        s.shapes(0).unwrap().shapes[0]
            .line
            .as_ref()
            .unwrap()
            .dash_pattern,
        vec![3., 2.]
    );
    for dash in [vec![0., 0.], vec![-1., 2.], vec![1.; 33]] {
        let mut d = Document::load_mem(&bytes).unwrap();
        let a = d.get_object_mut(id).unwrap().as_dict_mut().unwrap();
        a.set(
            "Border",
            vec![
                0.into(),
                0.into(),
                4.into(),
                Object::Array(dash.into_iter().map(Object::Real).collect()),
            ],
        );
        let mut bad = Vec::new();
        d.save_to(&mut bad).unwrap();
        assert!(
            Session::open(&bad)
                .unwrap()
                .shapes(0)
                .unwrap()
                .shapes
                .is_empty()
        );
    }
}

#[test]
fn every_standard_ending_survives_reopen_extract_insert_and_renders_consistently() {
    for ending in [
        "none",
        "square",
        "circle",
        "diamond",
        "open_arrow",
        "closed_arrow",
        "butt",
        "r_open_arrow",
        "r_closed_arrow",
        "slash",
    ] {
        let mut s = Session::blank(200., 120., 1).unwrap();
        let mut edit = request([30., 60.], [170., 60.], [ending, ending]);
        edit["fill"] = json!({"r":0.,"g":0.,"b":1.});
        apply(&mut s, edit);
        let before = s.render_png(0, 1.).unwrap();
        let list = s.shapes(0).unwrap();
        assert_eq!(list.shapes.len(), 1);
        let extracted = s.extract_pages(&[0]).unwrap();
        let extracted = Session::open(&extracted).unwrap();
        assert_eq!(before, extracted.render_png(0, 1.).unwrap());
        assert_eq!(list, extracted.shapes(0).unwrap());
        let mut target = Session::blank(200., 120., 1).unwrap();
        target.insert_pdf(&extracted.bytes().unwrap(), 0).unwrap();
        assert_eq!(before, target.render_png(0, 1.).unwrap());
        assert_eq!(list, target.shapes(0).unwrap());
        let out = Document::load_mem(&target.bytes().unwrap()).unwrap();
        assert_eq!(
            annotation(&out, 1)
                .get(b"P")
                .unwrap()
                .as_reference()
                .unwrap(),
            out.get_pages()[&1]
        );
    }
}

#[test]
fn increasing_line_width_expands_appearance_bounds_without_clipping_or_moving_endpoints() {
    let mut s = Session::blank(600., 500., 1).unwrap();
    let mut edit = request([200., 200.], [400., 200.], ["none", "open_arrow"]);
    edit["line_width"] = json!(1.);
    apply(&mut s, edit);
    let expected = serde_json::to_value(s.shapes(0).unwrap()).unwrap()["shapes"][0].clone();
    let before = s.bytes().unwrap();
    apply(
        &mut s,
        json!({"kind":"update_shape","page":0,"expected":expected,"bounds":expected["bounds"],"stroke":{"r":1.,"g":0.,"b":0.},"fill":null,"line_width":32.}),
    );
    assert_eq!(
        pixel(&s, 300, 500 - 214),
        [255, 0, 0],
        "32pt shaft must not be clipped to old 12pt Rect"
    );
    let out = Document::load_mem(&s.bytes().unwrap()).unwrap();
    assert_eq!(
        floats(&annotation(&out, 1), b"L"),
        vec![200., 200., 400., 200.]
    );
    let b = s.shapes(0).unwrap().shapes[0].bounds;
    assert!(
        b.y <= 136. && b.y + b.height >= 264.,
        "arrow wings must fit"
    );
    s.undo().unwrap();
    assert_eq!(s.bytes().unwrap(), before);
}

#[test]
fn arrow_wings_point_the_right_way_and_closed_markers_use_fill() {
    for (endings, back, front) in [
        (["open_arrow", "open_arrow"], [255, 0, 0], [255, 255, 255]),
        (["none", "r_open_arrow"], [255, 255, 255], [255, 0, 0]),
    ] {
        let mut s = Session::blank(200., 120., 1).unwrap();
        apply(&mut s, request([30., 60.], [170., 60.], endings));
        assert_eq!(pixel(&s, 155, 120 - 67), back);
        assert_eq!(pixel(&s, 185, 120 - 67), front);
        if endings[0] == "open_arrow" {
            assert_eq!(pixel(&s, 45, 120 - 67), [255, 0, 0]);
        }
    }
    for (ending, x) in [
        ("closed_arrow", 160),
        ("r_closed_arrow", 180),
        ("square", 170),
        ("circle", 170),
        ("diamond", 170),
    ] {
        let mut s = Session::blank(200., 120., 1).unwrap();
        let mut edit = request([30., 60.], [170., 60.], ["none", ending]);
        edit["fill"] = json!({"r":0.,"g":0.,"b":1.});
        apply(&mut s, edit);
        assert_eq!(
            pixel(&s, x, 60),
            [0, 0, 255],
            "{ending} interior must use fill"
        );
        assert_eq!(pixel(&s, 100, 60), [255, 0, 0]);
    }
}
