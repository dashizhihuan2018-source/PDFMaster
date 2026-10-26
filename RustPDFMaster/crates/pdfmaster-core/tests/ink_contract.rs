use lopdf::Document;
use pdfmaster_core::{Edit, Session};
use serde_json::{Value, json};

fn ink(strokes: Value, smoothed: bool) -> Value {
    json!({"kind":"add_ink","page":0,"strokes":strokes,"color":{"r":1.,"g":0.,"b":0.},"line_width":4.,"smoothed":smoothed})
}
fn apply(s: &mut Session, v: Value) {
    s.apply(serde_json::from_value::<Edit>(v).expect("Ink API must exist"))
        .unwrap();
}
fn annotations(d: &Document) -> Vec<lopdf::Dictionary> {
    d.get_object(d.get_pages()[&1])
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Annots")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|x| d.dereference(x).unwrap().1.as_dict().unwrap().clone())
        .collect()
}

#[test]
fn ink_is_real_independent_annotation_with_absolute_polyline_appearance_and_one_history_step() {
    let mut s = Session::blank(200., 120., 1).unwrap();
    let original = s.bytes().unwrap();
    apply(
        &mut s,
        ink(
            json!([[{"x":20.,"y":20.},{"x":100.,"y":90.},{"x":180.,"y":20.}],[{"x":20.,"y":100.},{"x":180.,"y":100.}]]),
            false,
        ),
    );
    let changed = s.bytes().unwrap();
    let d = Document::load_mem(&changed).unwrap();
    let a = annotations(&d);
    assert_eq!(a.len(), 2);
    for a in a {
        assert_eq!(a.get(b"Subtype").unwrap().as_name().unwrap(), b"Ink");
        assert!(a.has(b"AP"));
        assert_eq!(
            a.get(b"P").unwrap().as_reference().unwrap(),
            d.get_pages()[&1]
        );
        assert_eq!(a.get(b"InkList").unwrap().as_array().unwrap().len(), 1);
    }
    let reopened = Session::open(&changed).unwrap();
    assert_eq!(
        s.render_png(0, 1.).unwrap(),
        reopened.render_png(0, 1.).unwrap()
    );
    let extracted = Session::open(&s.extract_pages(&[0]).unwrap()).unwrap();
    assert_eq!(
        s.render_png(0, 1.).unwrap(),
        extracted.render_png(0, 1.).unwrap()
    );
    let mut target = Session::blank(200., 120., 1).unwrap();
    target.insert_pdf(&changed, 0).unwrap();
    assert_eq!(
        s.render_png(0, 1.).unwrap(),
        target.render_png(0, 1.).unwrap()
    );
    s.undo().unwrap();
    assert_eq!(s.bytes().unwrap(), original);
    s.redo().unwrap();
    assert_eq!(s.bytes().unwrap(), changed);
}

#[test]
fn smoothing_is_serialized_as_the_same_polyline_with_exact_endpoints() {
    let mut s = Session::blank(200., 120., 1).unwrap();
    apply(
        &mut s,
        ink(
            json!([[{"x":20.,"y":20.},{"x":100.,"y":100.},{"x":180.,"y":20.}]]),
            true,
        ),
    );
    let d = Document::load_mem(&s.bytes().unwrap()).unwrap();
    let a = annotations(&d);
    let points = a[0].get(b"InkList").unwrap().as_array().unwrap()[0]
        .as_array()
        .unwrap();
    assert!(points.len() > 6);
    assert_eq!(
        points[..2]
            .iter()
            .map(|v| v.as_float().unwrap())
            .collect::<Vec<_>>(),
        vec![20., 20.]
    );
    assert_eq!(
        points[points.len() - 2..]
            .iter()
            .map(|v| v.as_float().unwrap())
            .collect::<Vec<_>>(),
        vec![180., 20.]
    );
    assert!(
        points
            .chunks_exact(2)
            .all(|p| p[1].as_float().unwrap() < 100.)
    );
}

#[test]
fn invalid_ink_batch_is_atomic_and_does_not_consume_redo() {
    let mut s = Session::blank(200., 120., 1).unwrap();
    apply(
        &mut s,
        ink(json!([[{"x":20.,"y":20.},{"x":100.,"y":90.}]]), false),
    );
    s.undo().unwrap();
    let before = s.bytes().unwrap();
    let info = s.info();
    for paths in [
        json!([]),
        json!([[]]),
        json!([[{"x":20.,"y":20.}]]),
        json!([[{"x":20.,"y":20.},{"x":201.,"y":30.}]]),
        json!([[{"x":20.,"y":20.},{"x":100.,"y":30.}],[]]),
    ] {
        assert!(
            s.apply(serde_json::from_value(ink(paths, true)).unwrap())
                .is_err()
        );
        assert_eq!(s.bytes().unwrap(), before);
        assert_eq!(s.info(), info);
    }
}

#[test]
fn ink_round_caps_and_smoothed_peak_have_real_pixels_after_reopen() {
    let mut s = Session::blank(200., 120., 1).unwrap();
    let mut request = ink(json!([[{"x":20.,"y":20.},{"x":180.,"y":20.}]]), false);
    request["line_width"] = json!(12.);
    apply(&mut s, request);
    let clean = Session::open(&s.bytes().unwrap()).unwrap();
    let bytes = clean.render_png(0, 1.).unwrap();
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    let stride = info.color_type.samples();
    let at = |x: usize, y: usize| {
        let offset = (y * info.width as usize + x) * stride;
        pixels[offset..offset + 3].to_vec()
    };
    assert_eq!(at(16, 100), vec![255, 0, 0]);
    assert_eq!(at(183, 100), vec![255, 0, 0]);
    assert_eq!(at(14, 94), vec![255, 255, 255]);
    assert_eq!(at(100, 100), vec![255, 0, 0]);
    let mut curved = Session::blank(200., 120., 1).unwrap();
    apply(
        &mut curved,
        ink(
            json!([[{"x":20.,"y":20.},{"x":100.,"y":100.},{"x":180.,"y":20.}]]),
            true,
        ),
    );
    let png = curved.render_png(0, 1.).unwrap();
    let mut reader = png::Decoder::new(std::io::Cursor::new(png))
        .read_info()
        .unwrap();
    let mut data = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut data).unwrap();
    let stride = info.color_type.samples();
    for (x, y, expected) in [(100usize, 40usize, [255, 0, 0]), (100, 20, [255, 255, 255])] {
        let off = (y * info.width as usize + x) * stride;
        assert_eq!(&data[off..off + 3], &expected);
    }
}
