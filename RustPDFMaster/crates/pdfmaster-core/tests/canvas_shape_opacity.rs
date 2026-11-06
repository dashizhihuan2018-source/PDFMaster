use lopdf::{Document, Object, Stream, dictionary};
use pdfmaster_core::{
    CanvasShapeCommit, CanvasShapeDraft, CanvasShapeGeometry, Color, DocumentInfo, Edit,
    LineEnding, Point, Rect, Session,
};
use sha2::Digest;

fn reopen(d: &mut Document) -> Session {
    let mut bytes = Vec::new();
    d.save_to(&mut bytes).unwrap();
    Session::open(&bytes).unwrap()
}

fn blue_session() -> Session {
    let s = Session::blank(200., 120., 2).unwrap();
    let mut d = Document::load_mem(&s.bytes().unwrap()).unwrap();
    let content = d.add_object(Stream::new(
        dictionary! {},
        b"0 0 1 rg 0 0 200 120 re f".to_vec(),
    ));
    for page in d.get_pages().values() {
        d.get_object_mut(*page)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Contents", content);
    }
    reopen(&mut d)
}

fn rectangle(opacity: f32) -> CanvasShapeDraft {
    let red = Color {
        r: 1.,
        g: 0.,
        b: 0.,
    };
    CanvasShapeDraft {
        geometry: CanvasShapeGeometry::Rectangle {
            bounds: Rect {
                x: 20.,
                y: 20.,
                width: 100.,
                height: 60.,
            },
        },
        stroke: red,
        fill: Some(red),
        line_width: 2.,
        opacity,
    }
}

#[test]
fn half_alpha_is_real_typed_pdf_after_reopen() {
    let mut s = blue_session();
    let draft = rectangle(0.5);
    s.create_canvas_shape(0, &draft).unwrap();
    let png = s.render_png(0, 1.).unwrap();
    let p = image::load_from_memory(&png)
        .unwrap()
        .to_rgba8()
        .get_pixel(60, 50)
        .0;
    assert!(
        p[0].abs_diff(128) <= 2 && p[1] <= 2 && p[2].abs_diff(128) <= 2 && p[3] == 255,
        "{p:?}"
    );
    let reopened = Session::open(&s.bytes().unwrap()).unwrap();
    assert_eq!(
        reopened.canvas_shapes(0).unwrap().annotations[0].opacity,
        0.5
    );
}

#[derive(Debug, PartialEq)]
struct History {
    bytes: Vec<u8>,
    info: DocumentInfo,
    past: Vec<Vec<u8>>,
    future: Vec<Vec<u8>>,
}
fn history(s: &mut Session) -> History {
    let bytes = s.bytes().unwrap();
    let info = s.info();
    let mut past = Vec::new();
    let mut future = Vec::new();
    while s.undo().is_ok() {
        past.push(s.bytes().unwrap());
    }
    for _ in &past {
        s.redo().unwrap();
    }
    while s.redo().is_ok() {
        future.push(s.bytes().unwrap());
    }
    for _ in &future {
        s.undo().unwrap();
    }
    assert_eq!(s.bytes().unwrap(), bytes);
    assert_eq!(s.info(), info);
    History {
        bytes,
        info,
        past,
        future,
    }
}
fn seed_history(s: &mut Session) {
    for _ in 0..2 {
        s.apply(Edit::Rotate {
            page: 1,
            degrees: 90,
        })
        .unwrap();
    }
    s.undo().unwrap();
}
fn pixel(s: &Session, page: usize, x: u32, y: u32) -> [u8; 4] {
    image::load_from_memory(&s.render_png(page, 1.).unwrap())
        .unwrap()
        .to_rgba8()
        .get_pixel(x, y)
        .0
}
fn assert_blend(p: [u8; 4], alpha: f32) {
    let red = (255. * alpha).round() as u8;
    let blue = (255. * (1. - alpha)).round() as u8;
    assert!(
        p[0].abs_diff(red) <= 2 && p[1] <= 2 && p[2].abs_diff(blue) <= 2 && p[3] == 255,
        "{p:?} alpha={alpha}"
    );
}

#[test]
fn alpha_only_updates_are_one_transaction_and_exact_noops_keep_both_histories() {
    let mut s = blue_session();
    s.create_canvas_shape(0, &rectangle(1.)).unwrap();
    for alpha in [0.5, 0., 1.] {
        let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
        let before = s.bytes().unwrap();
        assert_eq!(
            s.update_canvas_shape(&selected, &rectangle(alpha)).unwrap(),
            CanvasShapeCommit::Updated
        );
        assert_blend(pixel(&s, 0, 60, 50), alpha);
        assert_eq!(s.canvas_shapes(0).unwrap().annotations[0].opacity, alpha);
        let after = s.bytes().unwrap();
        s.undo().unwrap();
        assert_eq!(s.bytes().unwrap(), before);
        s.redo().unwrap();
        assert_eq!(s.bytes().unwrap(), after);
        seed_history(&mut s);
        let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
        let before = history(&mut s);
        assert_eq!(
            s.update_canvas_shape(&selected, &rectangle(alpha)).unwrap(),
            CanvasShapeCommit::Unchanged
        );
        assert_eq!(history(&mut s), before);
        let mut forged = selected.clone();
        forged.opacity = if alpha == 0. { 0.5 } else { 0. };
        assert!(s.update_canvas_shape(&forged, &rectangle(0.5)).is_err());
        assert!(s.delete_canvas_shape(&forged).is_err());
        assert_eq!(history(&mut s), before);
    }
    let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
    s.update_canvas_shape(&selected, &rectangle(0.5)).unwrap();
    let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
    let before = s.bytes().unwrap();
    s.delete_canvas_shape(&selected).unwrap();
    assert!(s.canvas_shapes(0).unwrap().annotations.is_empty());
    assert_blend(pixel(&s, 0, 60, 50), 0.);
    s.undo().unwrap();
    assert_eq!(s.bytes().unwrap(), before);
}

#[test]
fn invalid_opacities_and_legacy_numeric_operations_preserve_exact_history() {
    let mut s = blue_session();
    s.create_canvas_shape(0, &rectangle(0.5)).unwrap();
    seed_history(&mut s);
    let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
    let before = history(&mut s);
    assert!(s.shapes(0).unwrap().shapes.is_empty());
    for opacity in [-0.01, 1.01, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(s.create_canvas_shape(0, &rectangle(opacity)).is_err());
        assert!(
            s.update_canvas_shape(&selected, &rectangle(opacity))
                .is_err()
        );
        assert_eq!(history(&mut s), before);
    }
    assert!(
        s.apply(Edit::UpdateShape {
            page: 0,
            expected: selected.original.clone(),
            bounds: selected.original.bounds,
            stroke: selected.original.stroke,
            fill: selected.original.fill,
            line_width: 2.
        })
        .is_err()
    );
    assert!(
        s.apply(Edit::DeleteShape {
            page: 0,
            expected: selected.original
        })
        .is_err()
    );
    assert_eq!(history(&mut s), before);
}

#[test]
fn half_alpha_delete_uses_canvas_admission() {
    let mut s = blue_session();
    s.create_canvas_shape(0, &rectangle(0.5)).unwrap();
    let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
    s.delete_canvas_shape(&selected).unwrap();
    assert_blend(pixel(&s, 0, 60, 50), 0.);
}

fn rotated_pixel(s: &Session, rotation: i32, x: u32, y: u32) -> [u8; 4] {
    let (x, y) = match rotation {
        0 => (x, y),
        90 => (119 - y, x),
        180 => (199 - x, 119 - y),
        270 => (y, 199 - x),
        _ => unreachable!(),
    };
    pixel(s, 0, x, y)
}
fn annotation(d: &Document, page: usize) -> (u32, u16) {
    let p = d.get_pages()[&(page as u32 + 1)];
    d.dereference(
        d.get_object(p)
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"Annots")
            .unwrap(),
    )
    .unwrap()
    .1
    .as_array()
    .unwrap()[0]
        .as_reference()
        .unwrap()
}
fn ap_id(d: &Document, root: (u32, u16)) -> (u32, u16) {
    let a = d.get_object(root).unwrap().as_dict().unwrap();
    d.dereference(a.get(b"AP").unwrap())
        .unwrap()
        .1
        .as_dict()
        .unwrap()
        .get(b"N")
        .unwrap()
        .as_reference()
        .unwrap()
}
fn state_mut(stream: &mut Stream) -> &mut lopdf::Dictionary {
    stream
        .dict
        .get_mut(b"Resources")
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .get_mut(b"ExtGState")
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .get_mut(b"PMShapeAlpha")
        .unwrap()
        .as_dict_mut()
        .unwrap()
}
fn assert_profile(s: &Session, opacity: f32) {
    let d = Document::load_mem(&s.bytes().unwrap()).unwrap();
    let root = annotation(&d, 0);
    let a = d.get_object(root).unwrap().as_dict().unwrap();
    if opacity == 1. {
        assert!(!a.has(b"CA") && !a.has(b"PDFMaster_ShapeOpacity"));
        return;
    }
    assert_eq!(
        a.get(b"PDFMaster_ShapeOpacity").unwrap().as_i64().unwrap(),
        1
    );
    assert_eq!(a.get(b"CA").unwrap().as_float().unwrap(), opacity);
    let ap = a.get(b"AP").unwrap().as_dict().unwrap();
    assert_eq!(ap.len(), 1);
    let stream = d.get_object(ap_id(&d, root)).unwrap().as_stream().unwrap();
    assert_eq!(stream.dict.len(), 6);
    assert_eq!(
        stream.dict.get(b"Type").unwrap().as_name().unwrap(),
        b"XObject"
    );
    assert_eq!(
        stream.dict.get(b"Subtype").unwrap().as_name().unwrap(),
        b"Form"
    );
    assert_eq!(stream.dict.get(b"FormType").unwrap().as_i64().unwrap(), 1);
    assert_eq!(
        stream.dict.get(b"Length").unwrap().as_i64().unwrap() as usize,
        stream.content.len()
    );
    let resources = stream.dict.get(b"Resources").unwrap().as_dict().unwrap();
    assert_eq!(resources.len(), 1);
    let gs = resources.get(b"ExtGState").unwrap().as_dict().unwrap();
    assert_eq!(gs.len(), 1);
    let state = gs.get(b"PMShapeAlpha").unwrap().as_dict().unwrap();
    assert_eq!(state.len(), 3);
    assert_eq!(state.get(b"Type").unwrap().as_name().unwrap(), b"ExtGState");
    for key in [b"CA".as_slice(), b"ca"] {
        assert_eq!(state.get(key).unwrap().as_float().unwrap(), opacity);
    }
    let ops = lopdf::content::Content::decode(&stream.content)
        .unwrap()
        .operations;
    assert_eq!(ops[0].operator, "q");
    assert_eq!(ops[1].operator, "gs");
    assert_eq!(
        ops[1].operands,
        vec![Object::Name(b"PMShapeAlpha".to_vec())]
    );
    assert_eq!(ops.last().unwrap().operator, "Q");
    assert_eq!(ops.iter().filter(|o| o.operator == "gs").count(), 1);
}

#[test]
fn real_output_matrix_all_kinds_fill_rotations_and_alpha() {
    let mut cases = 0;
    for alpha in [0., 0.5, 1.] {
        for kind in 0..5 {
            for fill in [false, true] {
                for rotation in [0, 90, 180, 270] {
                    let mut s = blue_session();
                    if rotation != 0 {
                        s.apply(Edit::Rotate {
                            page: 0,
                            degrees: rotation,
                        })
                        .unwrap();
                    }
                    let mut draft = rectangle(alpha);
                    draft.line_width = 6.;
                    if !fill {
                        draft.fill = None;
                    }
                    if kind == 1 {
                        draft.geometry = CanvasShapeGeometry::Ellipse {
                            bounds: Rect {
                                x: 20.,
                                y: 20.,
                                width: 100.,
                                height: 60.,
                            },
                        };
                    }
                    if kind >= 2 {
                        let endings = match kind {
                            2 => [LineEnding::None; 2],
                            3 => [LineEnding::None, LineEnding::OpenArrow],
                            _ => [LineEnding::OpenArrow; 2],
                        };
                        draft.geometry = CanvasShapeGeometry::Line {
                            start: Point { x: 30., y: 50. },
                            end: Point { x: 130., y: 50. },
                            endings,
                            dash_pattern: vec![],
                            bounds: None,
                        };
                        draft.line_width = 4.;
                    }
                    s.create_canvas_shape(0, &draft).unwrap();
                    let reopened = Session::open(&s.bytes().unwrap()).unwrap();
                    for current in [&s, &reopened] {
                        assert_eq!(
                            current.canvas_shapes(0).unwrap().annotations[0].opacity,
                            alpha
                        );
                        assert_profile(current, alpha);
                        assert_blend(rotated_pixel(current, rotation, 5, 5), 0.);
                        // Width6: Rect fill starts x23, ellipse right fill ends x117.
                        // x21 (Rect) and x118 (ellipse) are fully covered stroke-only
                        // pixels, safely separated from both fill and outer AA edges.
                        let (x, y) = match kind {
                            0 => (21, 69),
                            1 => (118, 69),
                            _ => (70, 69),
                        };
                        assert_blend(rotated_pixel(current, rotation, x, y), alpha);
                        // Open arrow wings: width4, line from (114,58) to (130,50).
                        // These pixel squares are fully inside a single wing stroke.
                        if kind >= 3 {
                            assert_blend(rotated_pixel(current, rotation, 121, 65), alpha);
                        }
                        if kind == 4 {
                            assert_blend(rotated_pixel(current, rotation, 38, 65), alpha);
                        }
                        if kind < 2 && fill {
                            // Inner stroke pixels x24/115 also receive fill: two paints.
                            let x = if kind == 0 { 24 } else { 115 };
                            assert_blend(
                                rotated_pixel(current, rotation, x, 69),
                                1. - (1. - alpha).powi(2),
                            );
                        }
                        if kind < 2 {
                            assert_blend(
                                rotated_pixel(current, rotation, 60, 69),
                                if fill { alpha } else { 0. },
                            );
                        }
                    }
                    let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
                    assert_eq!(
                        s.update_canvas_shape(&selected, &draft).unwrap(),
                        CanvasShapeCommit::Unchanged
                    );
                    let mut changed = draft.clone();
                    changed.opacity = 0.25;
                    s.update_canvas_shape(&selected, &changed).unwrap();
                    let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
                    assert_eq!(selected.opacity, 0.25);
                    let (x, y) = match kind {
                        0 => (21, 69),
                        1 => (118, 69),
                        _ => (70, 69),
                    };
                    assert_blend(rotated_pixel(&s, rotation, x, y), 0.25);
                    if let CanvasShapeGeometry::Line {
                        start,
                        end,
                        endings,
                        dash_pattern,
                        ..
                    } = draft.geometry
                    {
                        let line = selected.original.line.as_ref().unwrap();
                        assert_eq!(
                            (line.start, line.end, line.endings, &line.dash_pattern),
                            (start, end, endings, &dash_pattern)
                        );
                    }
                    s.delete_canvas_shape(&selected).unwrap();
                    assert!(s.canvas_shapes(0).unwrap().annotations.is_empty());
                    assert_blend(rotated_pixel(&s, rotation, x, y), 0.);
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 120);
}

#[test]
fn own_fractional_large_offset_and_reversed_lines_roundtrip_exactly() {
    for alpha in [0., 0.5] {
        for kind in 0..5 {
            let mut s = Session::blank(20000., 20000., 1).unwrap();
            let mut d = rectangle(alpha);
            d.line_width = 0.5;
            d.geometry = match kind {
                0 => CanvasShapeGeometry::Rectangle {
                    bounds: Rect {
                        x: 17.123,
                        y: 22.456,
                        width: 71.134,
                        height: 47.123,
                    },
                },
                1 => CanvasShapeGeometry::Ellipse {
                    bounds: Rect {
                        x: 18000.123,
                        y: 17000.456,
                        width: 1.134,
                        height: 2.123,
                    },
                },
                _ => CanvasShapeGeometry::Line {
                    start: Point {
                        x: 18000.123,
                        y: 17000.456,
                    },
                    end: match kind {
                        2 => Point {
                            x: 17950.3,
                            y: 16960.9,
                        },
                        3 => Point {
                            x: 17950.3,
                            y: 17000.456,
                        },
                        _ => Point {
                            x: 18000.123,
                            y: 16960.9,
                        },
                    },
                    endings: [LineEnding::OpenArrow, LineEnding::ClosedArrow],
                    dash_pattern: vec![3.25, 1.125],
                    bounds: None,
                },
            };
            s.create_canvas_shape(0, &d).unwrap();
            let mut s = Session::open(&s.bytes().unwrap()).unwrap();
            let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
            // Reconstruct from the stored coordinates, not pre-serialization widths.
            d.geometry = if let Some(line) = &selected.original.line {
                CanvasShapeGeometry::Line {
                    start: line.start,
                    end: line.end,
                    endings: line.endings,
                    dash_pattern: line.dash_pattern.clone(),
                    bounds: None,
                }
            } else if kind == 0 {
                CanvasShapeGeometry::Rectangle {
                    bounds: selected.original.bounds,
                }
            } else {
                CanvasShapeGeometry::Ellipse {
                    bounds: selected.original.bounds,
                }
            };
            let before = history(&mut s);
            assert_eq!(
                s.update_canvas_shape(&selected, &d).unwrap(),
                CanvasShapeCommit::Unchanged
            );
            assert_eq!(history(&mut s), before);
            d.opacity = 0.25;
            s.update_canvas_shape(&selected, &d).unwrap();
            let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
            assert_eq!(selected.opacity, 0.25);
            if let CanvasShapeGeometry::Line {
                start,
                end,
                endings,
                dash_pattern,
                ..
            } = &d.geometry
            {
                let line = selected.original.line.as_ref().unwrap();
                assert_eq!(
                    (line.start, line.end, line.endings, &line.dash_pattern),
                    (*start, *end, *endings, dash_pattern)
                );
            }
            s.delete_canvas_shape(&selected).unwrap();
            assert!(s.canvas_shapes(0).unwrap().annotations.is_empty());
        }
    }
}

#[test]
fn malformed_known_alpha_cannot_hide_behind_unsupported_style() {
    let mut source = blue_session();
    source.create_canvas_shape(0, &rectangle(0.5)).unwrap();
    for field in ["CA", "gsCA", "BBox", "FormType", "Resources"] {
        let mut d = Document::load_mem(&source.bytes().unwrap()).unwrap();
        let root = annotation(&d, 0);
        let ap = ap_id(&d, root);
        d.get_object_mut(root)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("BE", dictionary! {"S"=>"C"});
        match field {
            "CA" => d
                .get_object_mut(root)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("CA", Object::Null),
            "gsCA" => state_mut(d.get_object_mut(ap).unwrap().as_stream_mut().unwrap())
                .set("CA", Object::Null),
            field => d
                .get_object_mut(ap)
                .unwrap()
                .as_stream_mut()
                .unwrap()
                .dict
                .set(field, Object::Null),
        }
        let mut s = reopen(&mut d);
        seed_history(&mut s);
        let before = history(&mut s);
        assert!(
            matches!(
                s.canvas_shapes(0),
                Err(pdfmaster_core::PdfMasterError::Malformed(_))
            ),
            "{field}"
        );
        assert_eq!(history(&mut s), before);
    }
}

fn assert_alpha_refused_without_mutation(
    mut s: Session,
    mut selected: pdfmaster_core::CanvasShapeSelection,
    malformed: bool,
    label: &str,
) {
    seed_history(&mut s);
    let before = history(&mut s);
    assert!(!before.past.is_empty() && !before.future.is_empty());
    let listed = s.canvas_shapes(0);
    if malformed {
        assert!(
            matches!(listed, Err(pdfmaster_core::PdfMasterError::Malformed(_))),
            "{label}: {listed:?}"
        );
    } else {
        let listed = listed.unwrap();
        assert!(listed.annotations.is_empty(), "{label}: {listed:?}");
        assert_eq!(listed.unsupported_count, 1, "{label}");
    }
    assert_eq!(history(&mut s), before, "{label}: projection");
    // Only the revision is refreshed: this is the original genuine selection,
    // not an artificial stale-revision refusal that bypasses source inspection.
    selected.revision = sha2::Sha256::digest(&before.bytes).into();
    assert!(s.validate_canvas_shape(&selected).is_err(), "{label}");
    assert_eq!(history(&mut s), before, "{label}: validate");
    assert!(
        s.update_canvas_shape(&selected, &rectangle(0.25)).is_err(),
        "{label}"
    );
    assert_eq!(history(&mut s), before, "{label}: update");
    assert!(s.delete_canvas_shape(&selected).is_err(), "{label}");
    assert_eq!(history(&mut s), before, "{label}: delete");
    assert!(
        s.render_canvas_shape_background_png(&selected, 1.).is_err(),
        "{label}"
    );
    assert_eq!(history(&mut s), before, "{label}: background");
}

#[test]
fn invalid_normal_and_named_appearances_cannot_hide_behind_extended_styles() {
    let mut source = blue_session();
    source.create_canvas_shape(0, &rectangle(0.5)).unwrap();
    let selected = source.canvas_shapes(0).unwrap().annotations.remove(0);
    for style in ["BE", "RD", "Rotate"] {
        for hops in 0..=1 {
            for named in [false, true] {
                for invalid in [
                    Object::Null,
                    Object::Integer(7),
                    Object::Array(vec![]),
                    Object::Boolean(false),
                    Object::Name(b"Other".to_vec()),
                ] {
                    let mut d = Document::load_mem(&source.bytes().unwrap()).unwrap();
                    let root = annotation(&d, 0);
                    let mut value = invalid.clone();
                    if hops == 1 {
                        value = Object::Reference(d.add_object(value));
                    }
                    if named {
                        // A valid state must not hide a malformed sibling state.
                        let ap = ap_id(&d, root);
                        value = Object::Dictionary(dictionary! {"Good"=>ap,"Bad"=>value});
                    }
                    let a = d.get_object_mut(root).unwrap().as_dict_mut().unwrap();
                    a.set("AP", dictionary! {"N"=>value});
                    match style {
                        "BE" => a.set(style, dictionary! {"S"=>"C"}),
                        "RD" => a.set(style, vec![0.into(), 0.into(), 0.into(), 0.into()]),
                        _ => a.set(style, 0),
                    }
                    assert_alpha_refused_without_mutation(
                        reopen(&mut d),
                        selected.clone(),
                        true,
                        &format!("{style}/hops{hops}/named{named}/{invalid:?}"),
                    );
                }
            }
            let mut d = Document::load_mem(&source.bytes().unwrap()).unwrap();
            let root = annotation(&d, 0);
            let ap = ap_id(&d, root);
            let mut states = Object::Dictionary(dictionary! {"On"=>ap});
            if hops == 1 {
                states = Object::Reference(d.add_object(states));
            }
            let a = d.get_object_mut(root).unwrap().as_dict_mut().unwrap();
            a.set("AP", dictionary! {"N"=>states});
            match style {
                "BE" => a.set(style, dictionary! {"S"=>"C"}),
                "RD" => a.set(style, vec![0.into(), 0.into(), 0.into(), 0.into()]),
                _ => a.set(style, 0),
            }
            assert_alpha_refused_without_mutation(
                reopen(&mut d),
                selected.clone(),
                false,
                &format!("valid named states/hops{hops}"),
            );
        }
    }
}

fn reference_hops(d: &mut Document, mut value: Object, hops: usize) -> Object {
    for _ in 0..hops {
        value = Object::Reference(d.add_object(value));
    }
    value
}

fn replace_rect_reference(
    d: &mut Document,
    root: lopdf::ObjectId,
    coordinate: Option<usize>,
    hops: usize,
) {
    let mut rect = d
        .get_object(root)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Rect")
        .unwrap()
        .clone();
    if let Some(coordinate) = coordinate {
        let values = rect.as_array_mut().unwrap();
        values[coordinate] = reference_hops(d, values[coordinate].clone(), hops);
    } else {
        rect = reference_hops(d, rect, hops);
    }
    d.get_object_mut(root)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Rect", rect);
}

#[test]
fn two_hop_rect_or_coordinate_is_not_an_editable_alpha_source() {
    let mut source = blue_session();
    source.create_canvas_shape(0, &rectangle(0.5)).unwrap();
    let selected = source.canvas_shapes(0).unwrap().annotations.remove(0);
    for coordinate in [None, Some(0), Some(1), Some(2), Some(3)] {
        let mut d = Document::load_mem(&source.bytes().unwrap()).unwrap();
        let root = annotation(&d, 0);
        replace_rect_reference(&mut d, root, coordinate, 2);
        let s = reopen(&mut d);
        if coordinate.is_none() {
            // Preserve the actual backend incompatibility as evidence. A
            // refused source is not a successfully rendered editable shape.
            let reopened = Session::open(&s.bytes().unwrap()).unwrap();
            for current in [&s, &reopened] {
                assert_blend(pixel(current, 0, 70, 69), 0.);
                assert_blend(pixel(current, 0, 21, 69), 0.);
            }
        }
        assert_alpha_refused_without_mutation(
            s,
            selected.clone(),
            false,
            &format!("two-hop Rect {coordinate:?}"),
        );
    }
}

#[test]
fn one_hop_rect_and_each_coordinate_reopen_render_and_edit() {
    let mut source = blue_session();
    // x21 is inside the stroke but outside the inset fill at x23. Thus both
    // sampled points have exactly one half-alpha source-over, not two.
    let mut draft = rectangle(0.5);
    draft.line_width = 6.;
    source.create_canvas_shape(0, &draft).unwrap();
    for coordinate in [None, Some(0), Some(1), Some(2), Some(3)] {
        let mut d = Document::load_mem(&source.bytes().unwrap()).unwrap();
        let root = annotation(&d, 0);
        replace_rect_reference(&mut d, root, coordinate, 1);
        let mut s = reopen(&mut d);
        let reopened = Session::open(&s.bytes().unwrap()).unwrap();
        for current in [&s, &reopened] {
            assert_eq!(current.canvas_shapes(0).unwrap().annotations.len(), 1);
            assert_blend(pixel(current, 0, 70, 69), 0.5);
            assert_blend(pixel(current, 0, 21, 69), 0.5);
        }
        seed_history(&mut s);
        let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
        let before = history(&mut s);
        s.validate_canvas_shape(&selected).unwrap();
        assert_eq!(
            s.update_canvas_shape(&selected, &draft).unwrap(),
            CanvasShapeCommit::Unchanged
        );
        assert_eq!(history(&mut s), before);
        let background =
            image::load_from_memory(&s.render_canvas_shape_background_png(&selected, 1.).unwrap())
                .unwrap()
                .to_rgba8();
        assert_blend(background.get_pixel(70, 69).0, 0.);
        assert_eq!(history(&mut s), before);
        let mut changed = draft.clone();
        changed.opacity = 0.25;
        s.update_canvas_shape(&selected, &changed).unwrap();
        assert_blend(pixel(&s, 0, 70, 69), 0.25);
        let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
        s.delete_canvas_shape(&selected).unwrap();
        assert_blend(pixel(&s, 0, 70, 69), 0.);
    }
}

#[test]
fn two_hop_color_fill_and_border_remain_renderable_alpha_controls() {
    let mut source = blue_session();
    let mut draft = rectangle(0.5);
    draft.line_width = 6.;
    source.create_canvas_shape(0, &draft).unwrap();
    for key in [b"C".as_slice(), b"IC", b"BS"] {
        let mut d = Document::load_mem(&source.bytes().unwrap()).unwrap();
        let root = annotation(&d, 0);
        let value = d
            .get_object(root)
            .unwrap()
            .as_dict()
            .unwrap()
            .get(key)
            .unwrap()
            .clone();
        let value = reference_hops(&mut d, value, 2);
        d.get_object_mut(root)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set(key, value);
        let s = reopen(&mut d);
        let reopened = Session::open(&s.bytes().unwrap()).unwrap();
        for current in [&s, &reopened] {
            assert_eq!(current.canvas_shapes(0).unwrap().annotations.len(), 1);
            assert_blend(pixel(current, 0, 70, 69), 0.5);
            assert_blend(pixel(current, 0, 21, 69), 0.5);
        }
    }
}

#[test]
fn malformed_rect_array_and_each_coordinate_preserve_both_histories() {
    let mut source = blue_session();
    source.create_canvas_shape(0, &rectangle(0.5)).unwrap();
    let selected = source.canvas_shapes(0).unwrap().annotations.remove(0);
    for coordinate in [None, Some(0), Some(1), Some(2), Some(3)] {
        for invalid in 0..4 {
            let mut d = Document::load_mem(&source.bytes().unwrap()).unwrap();
            let root = annotation(&d, 0);
            let value = match invalid {
                0 => Object::Null,
                1 => Object::Reference((999_999, 0)),
                2 => {
                    let id = d.new_object_id();
                    d.objects.insert(id, Object::Reference(id));
                    Object::Reference(id)
                }
                _ => {
                    if coordinate.is_none() {
                        Object::Array(vec![0.into(), 0.into()])
                    } else {
                        Object::Name(b"NotANumber".to_vec())
                    }
                }
            };
            let mut rect = d
                .get_object(root)
                .unwrap()
                .as_dict()
                .unwrap()
                .get(b"Rect")
                .unwrap()
                .clone();
            if let Some(coordinate) = coordinate {
                rect.as_array_mut().unwrap()[coordinate] = value;
            } else {
                rect = value;
            }
            let a = d.get_object_mut(root).unwrap().as_dict_mut().unwrap();
            a.set("Rect", rect);
            a.set("BE", dictionary! {"S"=>"C"});
            assert_alpha_refused_without_mutation(
                reopen(&mut d),
                selected.clone(),
                true,
                &format!("malformed Rect {coordinate:?}/{invalid}"),
            );
        }
    }
}

#[test]
fn profile_depth_refusal_cannot_mask_malformed_known_siblings() {
    let mut source = blue_session();
    source.create_canvas_shape(0, &rectangle(0.5)).unwrap();
    let selected = source.canvas_shapes(0).unwrap().annotations.remove(0);
    for field in ["CA", "Rect", "AP", "N", "Type", "BBox", "stateCA"] {
        let mut d = Document::load_mem(&source.bytes().unwrap()).unwrap();
        let root = annotation(&d, 0);
        let ap = ap_id(&d, root);
        match field {
            "CA" => {
                let value = reference_hops(&mut d, Object::Real(0.5), 2);
                let a = d.get_object_mut(root).unwrap().as_dict_mut().unwrap();
                a.set("CA", value);
                a.set("AP", dictionary! {"N"=>Object::Null});
            }
            "Rect" => {
                replace_rect_reference(&mut d, root, None, 2);
                d.get_object_mut(root)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .set("AP", dictionary! {"N"=>Object::Null});
            }
            "AP" | "N" => {
                let normal = if field == "N" {
                    reference_hops(&mut d, Object::Reference(ap), 1)
                } else {
                    Object::Reference(ap)
                };
                let mut appearance = Object::Dictionary(dictionary! {"N"=>normal});
                if field == "AP" {
                    appearance = reference_hops(&mut d, appearance, 2);
                }
                d.get_object_mut(root)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .set("AP", appearance);
                state_mut(d.get_object_mut(ap).unwrap().as_stream_mut().unwrap())
                    .set("ca", Object::Null);
            }
            _ => {
                let value = match field {
                    "Type" => reference_hops(&mut d, Object::Name(b"XObject".to_vec()), 2),
                    "BBox" => reference_hops(
                        &mut d,
                        Object::Array(vec![0.into(), 0.into(), 100.into(), 60.into()]),
                        2,
                    ),
                    _ => reference_hops(&mut d, Object::Real(0.5), 2),
                };
                let stream = d.get_object_mut(ap).unwrap().as_stream_mut().unwrap();
                if field == "stateCA" {
                    state_mut(stream).set("CA", value);
                } else {
                    stream.dict.set(field, value);
                }
                state_mut(stream).set("ca", Object::Null);
            }
        }
        assert_alpha_refused_without_mutation(reopen(&mut d), selected.clone(), true, field);
    }
}

#[test]
fn forged_incomplete_complex_and_backend_incompatible_alpha_is_never_normalized() {
    let mut source = blue_session();
    source.create_canvas_shape(0, &rectangle(0.5)).unwrap();
    for case in 0..36 {
        let compressed_source = if case == 14 {
            let mut s = blue_session();
            let mut draft = rectangle(0.5);
            draft.geometry = CanvasShapeGeometry::Ellipse {
                bounds: Rect {
                    x: 20.,
                    y: 20.,
                    width: 100.,
                    height: 60.,
                },
            };
            s.create_canvas_shape(0, &draft).unwrap();
            Some(s)
        } else {
            None
        };
        let source = compressed_source.as_ref().unwrap_or(&source);
        let selected = source.canvas_shapes(0).unwrap().annotations.remove(0);
        let mut d = Document::load_mem(&source.bytes().unwrap()).unwrap();
        let root = annotation(&d, 0);
        let ap = ap_id(&d, root);
        match case {
            0 => { d.get_object_mut(root).unwrap().as_dict_mut().unwrap().remove(b"PDFMaster_ShapeOpacity"); },
            1 => { d.get_object_mut(root).unwrap().as_dict_mut().unwrap().remove(b"CA"); },
            2 => d.get_object_mut(root).unwrap().as_dict_mut().unwrap().set("PDFMaster_ShapeOpacity", 2),
            3 => d.get_object_mut(root).unwrap().as_dict_mut().unwrap().set("CA", 0.25_f32),
            4 => d.get_object_mut(root).unwrap().as_dict_mut().unwrap().set("ca", 0.5_f32),
            5 => d.get_object_mut(root).unwrap().as_dict_mut().unwrap().set("AS", "On"),
            6 => { let a = d.get_object_mut(root).unwrap().as_dict_mut().unwrap(); a.set("AP", dictionary! {"N"=>ap,"R"=>ap}); },
            7 => d.get_object_mut(root).unwrap().as_dict_mut().unwrap().set("AP", dictionary! {"N"=>dictionary! {"On"=>ap}}),
            8 => d.get_object_mut(ap).unwrap().as_stream_mut().unwrap().dict.set("Type", "Other"),
            9 => d.get_object_mut(ap).unwrap().as_stream_mut().unwrap().dict.set("Subtype", "Other"),
            10 => d.get_object_mut(ap).unwrap().as_stream_mut().unwrap().dict.set("FormType", 2),
            11 => d.get_object_mut(ap).unwrap().as_stream_mut().unwrap().dict.set("BBox", vec![0.into(),0.into(),99.into(),60.into()]),
            12 => d.get_object_mut(ap).unwrap().as_stream_mut().unwrap().dict.set("Matrix", vec![1.into(),0.into(),0.into(),1.into(),0.into(),0.into()]),
            13 => d.get_object_mut(ap).unwrap().as_stream_mut().unwrap().dict.set("Group", dictionary! {"S"=>"Transparency"}),
            14 => { let stream=d.get_object_mut(ap).unwrap().as_stream_mut().unwrap(); stream.compress().unwrap(); assert!(stream.dict.has(b"Filter")); },
            15 => d.get_object_mut(ap).unwrap().as_stream_mut().unwrap().dict.set("DecodeParms", dictionary! {}),
            16 => d.get_object_mut(ap).unwrap().as_stream_mut().unwrap().dict.set("Unknown", 1),
            17 => { let stream=d.get_object_mut(ap).unwrap().as_stream_mut().unwrap(); stream.dict.get_mut(b"Resources").unwrap().as_dict_mut().unwrap().set("Font", dictionary! {}); },
            18 => state_mut(d.get_object_mut(ap).unwrap().as_stream_mut().unwrap()).set("Type", "Other"),
            19 => state_mut(d.get_object_mut(ap).unwrap().as_stream_mut().unwrap()).set("CA", 0.25_f32),
            20 => state_mut(d.get_object_mut(ap).unwrap().as_stream_mut().unwrap()).set("ca", 0.25_f32),
            21 => state_mut(d.get_object_mut(ap).unwrap().as_stream_mut().unwrap()).set("Unused", 1),
            22 => { let stream=d.get_object_mut(ap).unwrap().as_stream_mut().unwrap(); stream.content.extend_from_slice(b"\n0 0 1 rg"); },
            23 => { let stream=d.get_object_mut(ap).unwrap().as_stream_mut().unwrap(); stream.content = b"q\n/Other gs\nQ".to_vec(); },
            24 => { let stream=d.get_object_mut(ap).unwrap().as_stream_mut().unwrap(); stream.content = b"q\n/PMShapeAlpha gs\nQ".to_vec(); },
            25 => { let terminal = d.add_object(Object::Real(0.5)); let hop = d.add_object(Object::Reference(terminal)); d.get_object_mut(root).unwrap().as_dict_mut().unwrap().set("CA", hop); },
            26 => d.get_object_mut(root).unwrap().as_dict_mut().unwrap().set("CA", Object::Reference((999_999,0))),
            27 => { let cycle=d.new_object_id(); d.objects.insert(cycle,Object::Reference(cycle)); d.get_object_mut(root).unwrap().as_dict_mut().unwrap().set("CA", cycle); },
            28 => { let hop=d.add_object(Object::Reference(ap)); d.get_object_mut(root).unwrap().as_dict_mut().unwrap().set("AP", dictionary! {"N"=>hop}); },
            29 => { let hop=d.add_object(Object::Reference(root)); let page=d.get_pages()[&1]; d.get_object_mut(page).unwrap().as_dict_mut().unwrap().set("Annots", vec![Object::Reference(hop)]); },
            30 => { let stream=d.get_object_mut(ap).unwrap().as_stream_mut().unwrap(); let states=stream.dict.get_mut(b"Resources").unwrap().as_dict_mut().unwrap().get_mut(b"ExtGState").unwrap().as_dict_mut().unwrap(); let state=states.get(b"PMShapeAlpha").unwrap().clone(); states.set("Unused", state); },
            31 => { let stream=d.get_object_mut(ap).unwrap().as_stream_mut().unwrap(); state_mut(stream).remove(b"ca"); },
            32 => d.get_object_mut(root).unwrap().as_dict_mut().unwrap().set("CA", -0.1_f32),
            33 => d.get_object_mut(root).unwrap().as_dict_mut().unwrap().set("CA", 1.1_f32),
            34 => d.get_object_mut(ap).unwrap().as_stream_mut().unwrap().dict.set("Resources", dictionary! {"ExtGState"=>dictionary! {"WrongName"=>dictionary! {"Type"=>"ExtGState","CA"=>0.5_f32,"ca"=>0.5_f32}}}),
            35 => { d.get_object_mut(ap).unwrap().as_stream_mut().unwrap().content=vec![b' ';65537]; },
            _ => unreachable!(),
        }
        // Stream::content is public; refresh Length after intentional content
        // mutation so this tests the AP auditor, not stale PDF serialization.
        if matches!(case, 22..=24 | 35) {
            let stream = d.get_object_mut(ap).unwrap().as_stream_mut().unwrap();
            stream.dict.set("Length", stream.content.len() as i64);
        }
        let mut bytes = Vec::new();
        d.save_to(&mut bytes).unwrap();
        let mut s = Session::open(&bytes)
            .unwrap_or_else(|error| panic!("case {case} fixture must open: {error}"));
        seed_history(&mut s);
        let before = history(&mut s);
        let listed = s.canvas_shapes(0);
        assert!(
            listed
                .as_ref()
                .map(|l| l.annotations.is_empty())
                .unwrap_or(true),
            "case {case}: {listed:?}"
        );
        let mut selected = selected;
        selected.revision = sha2::Sha256::digest(&before.bytes).into();
        assert!(s.validate_canvas_shape(&selected).is_err(), "case {case}");
        assert!(
            s.update_canvas_shape(&selected, &rectangle(0.25)).is_err(),
            "case {case}"
        );
        assert!(s.delete_canvas_shape(&selected).is_err(), "case {case}");
        assert!(
            s.render_canvas_shape_background_png(&selected, 1.).is_err(),
            "case {case}"
        );
        assert_eq!(history(&mut s), before, "case {case}");
    }
}

#[test]
fn shared_alpha_root_popup_and_metadata_are_copy_on_write_and_omission_is_real() {
    for rotation in [0, 90, 180, 270] {
        let mut source = blue_session();
        source.create_canvas_shape(0, &rectangle(0.5)).unwrap();
        let mut d = Document::load_mem(&source.bytes().unwrap()).unwrap();
        let root = annotation(&d, 0);
        let pages = d.get_pages();
        let popup=d.add_object(dictionary! {"Subtype"=>"Popup","Rect"=>vec![1.into(),1.into(),10.into(),10.into()],"Parent"=>root,"P"=>pages[&1],"UnknownPopup"=>"preserved"});
        let a = d.get_object_mut(root).unwrap().as_dict_mut().unwrap();
        a.set("Popup", popup);
        a.set("PrivateMetadata", dictionary! {"Keep"=>"yes"});
        let shared = d.add_object(vec![Object::Reference(root), Object::Reference(popup)]);
        for page in pages.values() {
            let p = d.get_object_mut(*page).unwrap().as_dict_mut().unwrap();
            p.set("Annots", shared);
            p.set("Rotate", rotation);
        }
        let mut s = reopen(&mut d);
        seed_history(&mut s);
        let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
        let before = history(&mut s);
        let other = s.render_png(1, 1.).unwrap();
        let mut independent = Document::load_mem(&before.bytes).unwrap();
        independent
            .get_object_mut(pages[&1])
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Annots", vec![Object::Reference(popup)]);
        let expected = reopen(&mut independent).render_png(0, 1.).unwrap();
        assert_eq!(
            image::load_from_memory(&s.render_canvas_shape_background_png(&selected, 1.).unwrap())
                .unwrap()
                .to_rgba8(),
            image::load_from_memory(&expected).unwrap().to_rgba8()
        );
        assert_eq!(history(&mut s), before);
        let baseline = Document::load_mem(&before.bytes).unwrap();
        let old_root = baseline.get_object(root).unwrap().clone();
        let old_popup = baseline.get_object(popup).unwrap().clone();
        let old_page = baseline.get_object(pages[&2]).unwrap().clone();
        s.update_canvas_shape(&selected, &rectangle(0.25)).unwrap();
        let updated = Document::load_mem(&s.bytes().unwrap()).unwrap();
        let new_root = annotation(&updated, 0);
        assert_ne!(new_root, root);
        assert_eq!(annotation(&updated, 1), root);
        assert_eq!(updated.get_object(root).unwrap(), &old_root);
        assert_eq!(updated.get_object(popup).unwrap(), &old_popup);
        assert_eq!(updated.get_object(pages[&2]).unwrap(), &old_page);
        let a = updated.get_object(new_root).unwrap().as_dict().unwrap();
        assert_eq!(
            a.get(b"PrivateMetadata").unwrap(),
            old_root.as_dict().unwrap().get(b"PrivateMetadata").unwrap()
        );
        let new_popup = a.get(b"Popup").unwrap().as_reference().unwrap();
        assert_ne!(new_popup, popup);
        let p = updated.get_object(new_popup).unwrap().as_dict().unwrap();
        assert_eq!(p.get(b"Parent").unwrap().as_reference().unwrap(), new_root);
        assert_eq!(
            p.get(b"UnknownPopup").unwrap(),
            old_popup.as_dict().unwrap().get(b"UnknownPopup").unwrap()
        );
        assert_eq!(s.render_png(1, 1.).unwrap(), other);
        let after = s.bytes().unwrap();
        s.undo().unwrap();
        assert_eq!(s.bytes().unwrap(), before.bytes);
        s.redo().unwrap();
        assert_eq!(s.bytes().unwrap(), after);
        let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
        s.delete_canvas_shape(&selected).unwrap();
        assert_eq!(s.render_png(1, 1.).unwrap(), other);
        assert!(s.canvas_shapes(0).unwrap().annotations.is_empty());
    }
}

#[test]
fn opaque_appearance_states_cannot_be_discarded_to_convert_to_alpha() {
    for case in 0..3 {
        let mut s = blue_session();
        s.create_canvas_shape(0, &rectangle(1.)).unwrap();
        let mut d = Document::load_mem(&s.bytes().unwrap()).unwrap();
        let root = annotation(&d, 0);
        let ap = ap_id(&d, root);
        let a = d.get_object_mut(root).unwrap().as_dict_mut().unwrap();
        match case {
            0 => a.set("AP", dictionary! {"N"=>ap,"R"=>ap}),
            1 => a.set("AS", "On"),
            _ => a.set("AP", dictionary! {"N"=>dictionary! {"On"=>ap}}),
        }
        let mut s = reopen(&mut d);
        seed_history(&mut s);
        let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
        let before = history(&mut s);
        assert!(s.update_canvas_shape(&selected, &rectangle(0.5)).is_err());
        assert_eq!(history(&mut s), before);
    }
}

#[test]
fn canonical_direct_and_one_hop_values_are_editable_and_old_wire_defaults_opaque() {
    let draft = rectangle(0.5);
    let mut value = serde_json::to_value(&draft).unwrap();
    assert_eq!(value["opacity"], 0.5);
    value.as_object_mut().unwrap().remove("opacity");
    let old: CanvasShapeDraft = serde_json::from_value(value).unwrap();
    assert_eq!(old.opacity, 1.);
    let mut s = blue_session();
    s.create_canvas_shape(0, &draft).unwrap();
    let mut d = Document::load_mem(&s.bytes().unwrap()).unwrap();
    let root = annotation(&d, 0);
    let ap = ap_id(&d, root);
    let mut stream = d.get_object(ap).unwrap().as_stream().unwrap().clone();
    for key in [b"Type".as_slice(), b"FormType", b"Length"] {
        let value = stream.dict.get(key).unwrap().clone();
        let id = d.add_object(value);
        stream.dict.set(key, id);
    }
    let mut bbox = stream
        .dict
        .get(b"BBox")
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    for v in &mut bbox {
        *v = Object::Reference(d.add_object(v.clone()));
    }
    let bbox = d.add_object(bbox);
    stream.dict.set("BBox", bbox);
    let mut resources = stream
        .dict
        .get(b"Resources")
        .unwrap()
        .as_dict()
        .unwrap()
        .clone();
    let mut states = resources
        .get(b"ExtGState")
        .unwrap()
        .as_dict()
        .unwrap()
        .clone();
    let mut state = states
        .get(b"PMShapeAlpha")
        .unwrap()
        .as_dict()
        .unwrap()
        .clone();
    for key in [b"Type".as_slice(), b"CA", b"ca"] {
        let id = d.add_object(state.get(key).unwrap().clone());
        state.set(key, id);
    }
    let id = d.add_object(state);
    states.set("PMShapeAlpha", id);
    let id = d.add_object(states);
    resources.set("ExtGState", id);
    let id = d.add_object(resources);
    stream.dict.set("Resources", id);
    d.objects.insert(ap, Object::Stream(stream));
    let mut a = d.get_object(root).unwrap().as_dict().unwrap().clone();
    for key in [b"CA".as_slice(), b"PDFMaster_ShapeOpacity", b"AP"] {
        let id = d.add_object(a.get(key).unwrap().clone());
        a.set(key, id);
    }
    d.objects.insert(root, Object::Dictionary(a));
    let mut s = reopen(&mut d);
    assert_blend(pixel(&s, 0, 60, 50), 0.5);
    let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
    assert_eq!(selected.opacity, 0.5);
    let mut value = serde_json::to_value(&selected).unwrap();
    value.as_object_mut().unwrap().remove("opacity");
    let old: pdfmaster_core::CanvasShapeSelection = serde_json::from_value(value).unwrap();
    assert_eq!(old.opacity, 1.);
    assert!(s.validate_canvas_shape(&old).is_err());
    s.update_canvas_shape(&selected, &rectangle(0.)).unwrap();
    assert_blend(pixel(&s, 0, 60, 50), 0.);
    let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
    s.delete_canvas_shape(&selected).unwrap();
}

#[test]
fn one_hop_subtype_backend_limit_must_not_be_an_editable_alpha_selection() {
    let mut s = blue_session();
    s.create_canvas_shape(0, &rectangle(0.5)).unwrap();
    let mut d = Document::load_mem(&s.bytes().unwrap()).unwrap();
    let root = annotation(&d, 0);
    let ap = ap_id(&d, root);
    let subtype = d.add_object(Object::Name(b"Form".to_vec()));
    d.get_object_mut(ap)
        .unwrap()
        .as_stream_mut()
        .unwrap()
        .dict
        .set("Subtype", subtype);
    let mut s = reopen(&mut d);
    seed_history(&mut s);
    let before = history(&mut s);
    let error = s.render_png(0, 1.).unwrap_err().to_string();
    assert!(
        error.contains("source XObject is missing Subtype"),
        "{error}"
    );
    assert!(s.canvas_shapes(0).unwrap().annotations.is_empty());
    assert_eq!(history(&mut s), before);
}

#[test]
fn each_supported_one_hop_field_reopens_renders_and_edits() {
    let mut source = blue_session();
    source.create_canvas_shape(0, &rectangle(0.5)).unwrap();
    for field in 0..14 {
        let mut d = Document::load_mem(&source.bytes().unwrap()).unwrap();
        let root = annotation(&d, 0);
        let ap = ap_id(&d, root);
        let mut stream = d.get_object(ap).unwrap().as_stream().unwrap().clone();
        match field {
            0..=2 => {
                let key = match field {
                    0 => b"CA".as_slice(),
                    1 => b"PDFMaster_ShapeOpacity",
                    _ => b"AP",
                };
                let value = d
                    .get_object(root)
                    .unwrap()
                    .as_dict()
                    .unwrap()
                    .get(key)
                    .unwrap()
                    .clone();
                let id = d.add_object(value);
                d.get_object_mut(root)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .set(key, id);
            }
            3..=6 => {
                let key = match field {
                    3 => b"Type".as_slice(),
                    4 => b"FormType",
                    5 => b"Length",
                    _ => b"BBox",
                };
                let id = d.add_object(stream.dict.get(key).unwrap().clone());
                stream.dict.set(key, id);
            }
            7 => {
                let mut bbox = stream
                    .dict
                    .get(b"BBox")
                    .unwrap()
                    .as_array()
                    .unwrap()
                    .clone();
                for v in &mut bbox {
                    *v = Object::Reference(d.add_object(v.clone()));
                }
                stream.dict.set("BBox", bbox);
            }
            8 => {
                let id = d.add_object(stream.dict.get(b"Resources").unwrap().clone());
                stream.dict.set("Resources", id);
            }
            9 => {
                let resources = stream
                    .dict
                    .get_mut(b"Resources")
                    .unwrap()
                    .as_dict_mut()
                    .unwrap();
                let id = d.add_object(resources.get(b"ExtGState").unwrap().clone());
                resources.set("ExtGState", id);
            }
            10 => {
                let states = stream
                    .dict
                    .get_mut(b"Resources")
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .get_mut(b"ExtGState")
                    .unwrap()
                    .as_dict_mut()
                    .unwrap();
                let id = d.add_object(states.get(b"PMShapeAlpha").unwrap().clone());
                states.set("PMShapeAlpha", id);
            }
            _ => {
                let key = match field {
                    11 => b"Type".as_slice(),
                    12 => b"CA",
                    _ => b"ca",
                };
                let state = state_mut(&mut stream);
                let id = d.add_object(state.get(key).unwrap().clone());
                state.set(key, id);
            }
        }
        d.objects.insert(ap, Object::Stream(stream));
        let mut s = reopen(&mut d);
        let reopened = Session::open(&s.bytes().unwrap()).unwrap();
        for current in [&s, &reopened] {
            assert_blend(pixel(current, 0, 60, 50), 0.5);
            assert_eq!(
                current.canvas_shapes(0).unwrap().annotations[0].opacity,
                0.5
            );
        }
        let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
        let before = history(&mut s);
        assert_eq!(
            s.update_canvas_shape(&selected, &rectangle(0.5)).unwrap(),
            CanvasShapeCommit::Unchanged
        );
        assert_eq!(history(&mut s), before);
        s.update_canvas_shape(&selected, &rectangle(0.)).unwrap();
        assert_blend(pixel(&s, 0, 60, 50), 0.);
        let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
        s.delete_canvas_shape(&selected).unwrap();
    }
}

#[test]
fn reversed_dashed_arrow_has_real_alpha_on_shaft_and_real_background_in_gap() {
    for rotation in [0, 90, 180, 270] {
        let mut s = blue_session();
        if rotation != 0 {
            s.apply(Edit::Rotate {
                page: 0,
                degrees: rotation,
            })
            .unwrap();
        }
        let mut d = rectangle(0.5);
        d.line_width = 4.;
        d.geometry = CanvasShapeGeometry::Line {
            start: Point { x: 130., y: 50. },
            end: Point { x: 30., y: 50. },
            endings: [LineEnding::None, LineEnding::OpenArrow],
            dash_pattern: vec![10., 10.],
            bounds: None,
        };
        s.create_canvas_shape(0, &d).unwrap();
        let reopened = Session::open(&s.bytes().unwrap()).unwrap();
        for current in [&s, &reopened] {
            assert_blend(rotated_pixel(current, rotation, 105, 69), 0.5);
            assert_blend(rotated_pixel(current, rotation, 95, 69), 0.);
        }
        let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
        assert_eq!(
            s.update_canvas_shape(&selected, &d).unwrap(),
            CanvasShapeCommit::Unchanged
        );
    }
}

#[test]
fn opaque_nominal_shape_without_ap_can_gain_a_new_canonical_appearance() {
    let mut s = blue_session();
    s.create_canvas_shape(0, &rectangle(1.)).unwrap();
    let mut d = Document::load_mem(&s.bytes().unwrap()).unwrap();
    let root = annotation(&d, 0);
    let a = d.get_object_mut(root).unwrap().as_dict_mut().unwrap();
    a.remove(b"AP");
    a.set("PrivateMetadata", "preserved");
    let mut s = reopen(&mut d);
    let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
    s.update_canvas_shape(&selected, &rectangle(0.5)).unwrap();
    assert_blend(pixel(&s, 0, 60, 50), 0.5);
    assert_profile(&s, 0.5);
    let d = Document::load_mem(&s.bytes().unwrap()).unwrap();
    assert_eq!(
        d.get_object(annotation(&d, 0))
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"PrivateMetadata")
            .unwrap()
            .as_name()
            .unwrap(),
        b"preserved"
    );
}
