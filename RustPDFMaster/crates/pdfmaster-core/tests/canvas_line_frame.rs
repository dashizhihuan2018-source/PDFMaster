use lopdf::{Document, Object, Stream, dictionary};
use pdfmaster_core::{
    CanvasShapeCommit, CanvasShapeDraft, CanvasShapeGeometry, CanvasShapeSelection, Color,
    DocumentInfo, Edit, LineEnding, Point, Rect, Session,
};

fn frame() -> Rect {
    Rect {
        x: 38.,
        y: 58.,
        width: 104.,
        height: 4.,
    }
}

fn line(bounds: Option<Rect>) -> CanvasShapeDraft {
    CanvasShapeDraft {
        geometry: CanvasShapeGeometry::Line {
            start: Point { x: 40., y: 60. },
            end: Point { x: 140., y: 60. },
            endings: [LineEnding::None; 2],
            dash_pattern: vec![],
            bounds,
        },
        stroke: Color {
            r: 1.,
            g: 0.,
            b: 0.,
        },
        fill: None,
        line_width: 2.,
        opacity: 1.,
    }
}

#[test]
fn exact_horizontal_frame_survives_create_and_reopen() {
    let mut s = Session::blank(200., 120., 2).unwrap();
    s.create_canvas_shape(0, &line(Some(frame()))).unwrap();
    assert_eq!(
        s.canvas_shapes(0).unwrap().annotations[0].original.bounds,
        frame()
    );
    let reopened = Session::open(&s.bytes().unwrap()).unwrap();
    assert_eq!(
        reopened.canvas_shapes(0).unwrap().annotations[0]
            .original
            .bounds,
        frame()
    );
}

#[test]
fn frame_only_update_is_updated_and_exact_after_reopen() {
    let mut s = Session::blank(200., 120., 2).unwrap();
    s.create_canvas_shape(0, &line(None)).unwrap();
    let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
    assert_ne!(selected.original.bounds, frame());
    assert_eq!(
        s.update_canvas_shape(&selected, &line(Some(frame())))
            .unwrap(),
        CanvasShapeCommit::Updated
    );
    let reopened = Session::open(&s.bytes().unwrap()).unwrap();
    assert_eq!(
        reopened.canvas_shapes(0).unwrap().annotations[0]
            .original
            .bounds,
        frame()
    );
}

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

fn selected(s: &Session) -> CanvasShapeSelection {
    s.canvas_shapes(0).unwrap().annotations.remove(0)
}

fn set_frame(d: &mut CanvasShapeDraft, frame: Option<Rect>) {
    if let CanvasShapeGeometry::Line { bounds, .. } = &mut d.geometry {
        *bounds = frame;
    } else {
        panic!("expected line");
    }
}

fn assert_stored(s: &Session, d: &CanvasShapeDraft, expected_bounds: Rect) {
    let selected = selected(s);
    assert_eq!(selected.original.bounds, expected_bounds);
    assert_eq!(selected.opacity, d.opacity);
    assert_eq!(selected.original.stroke, d.stroke);
    assert_eq!(selected.original.fill, d.fill);
    assert_eq!(selected.original.line_width, d.line_width);
    let stored = selected.original.line.unwrap();
    if let CanvasShapeGeometry::Line {
        start,
        end,
        endings,
        dash_pattern,
        ..
    } = &d.geometry
    {
        assert_eq!((stored.start, stored.end), (*start, *end));
        assert_eq!(stored.endings, *endings);
        assert_eq!(stored.dash_pattern, *dash_pattern);
    } else {
        panic!("expected line");
    }
    // Check the actual /Rect and AP /BBox, not just the typed projection.
    let pdf = Document::load_mem(&s.bytes().unwrap()).unwrap();
    let a = pdf
        .get_object(selected.object_id.unwrap())
        .unwrap()
        .as_dict()
        .unwrap();
    assert_eq!(
        a.get(b"Rect")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_float().unwrap())
            .collect::<Vec<_>>(),
        vec![
            expected_bounds.x,
            expected_bounds.y,
            expected_bounds.x + expected_bounds.width,
            expected_bounds.y + expected_bounds.height
        ]
    );
    let ap = a.get(b"AP").unwrap().as_dict().unwrap().get(b"N").unwrap();
    let ap = pdf.dereference(ap).unwrap().1.as_stream().unwrap();
    assert_eq!(
        ap.dict
            .get(b"BBox")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_float().unwrap())
            .collect::<Vec<_>>(),
        vec![0., 0., expected_bounds.width, expected_bounds.height]
    );
}

#[test]
fn omitted_null_and_explicit_wire_frames_preserve_legacy_expanded_geometry() {
    let explicit = line(Some(frame()));
    let wire = serde_json::to_value(&explicit).unwrap();
    assert_eq!(wire["geometry"]["bounds"]["height"], 4.);
    assert_eq!(
        serde_json::from_value::<CanvasShapeDraft>(wire).unwrap(),
        explicit
    );
    for omit in [false, true] {
        let mut wire = serde_json::to_value(line(None)).unwrap();
        if omit {
            wire["geometry"].as_object_mut().unwrap().remove("bounds");
        }
        let old: CanvasShapeDraft = serde_json::from_value(wire).unwrap();
        assert_eq!(old, line(None));
        let mut s = blue_session();
        s.create_canvas_shape(0, &old).unwrap();
        assert_eq!(
            selected(&s).original.bounds,
            Rect {
                x: 28.,
                y: 48.,
                width: 124.,
                height: 24.
            }
        );
        assert_eq!(
            selected(&Session::open(&s.bytes().unwrap()).unwrap()),
            selected(&s)
        );
    }
    let mut s = blue_session();
    s.apply(Edit::AddLine {
        page: 0,
        start: Point { x: 40., y: 60. },
        end: Point { x: 140., y: 60. },
        endings: [LineEnding::None; 2],
        stroke: explicit.stroke,
        fill: None,
        line_width: 2.,
    })
    .unwrap();
    let old = s.shapes(0).unwrap().shapes.remove(0);
    assert_eq!(
        old.bounds,
        Rect {
            x: 28.,
            y: 48.,
            width: 124.,
            height: 24.
        }
    );
    s.apply(Edit::UpdateShape {
        page: 0,
        expected: old.clone(),
        bounds: old.bounds,
        stroke: Color {
            r: 0.,
            g: 1.,
            b: 0.,
        },
        fill: None,
        line_width: 2.,
    })
    .unwrap();
    assert_eq!(s.shapes(0).unwrap().shapes[0].bounds, old.bounds);
    // Exact stored geometry still supports old callers' endpoint-only no-op equality.
    let mut s = blue_session();
    s.create_canvas_shape(0, &explicit).unwrap();
    seed_history(&mut s);
    let before = history(&mut s);
    assert_eq!(
        s.update_canvas_shape(&selected(&s), &line(None)).unwrap(),
        CanvasShapeCommit::Unchanged
    );
    assert_eq!(history(&mut s), before);
}

#[test]
fn six_directions_four_rotations_three_alphas_preserve_frames_transactions_and_reopen() {
    let geometries = [
        (Point { x: 40., y: 60. }, Point { x: 140., y: 60. }, frame()),
        (
            Point { x: 80., y: 30. },
            Point { x: 80., y: 90. },
            Rect {
                x: 78.,
                y: 28.,
                width: 4.,
                height: 64.,
            },
        ),
        (
            Point { x: 40., y: 30. },
            Point { x: 140., y: 90. },
            Rect {
                x: 38.,
                y: 28.,
                width: 104.,
                height: 64.,
            },
        ),
    ];
    let mut cases = 0;
    for (start, end, bounds) in geometries {
        for reverse in [false, true] {
            for rotation in [0, 90, 180, 270] {
                for alpha in [0., 0.5, 1.] {
                    let mut s = blue_session();
                    if rotation != 0 {
                        s.apply(Edit::Rotate {
                            page: 0,
                            degrees: rotation,
                        })
                        .unwrap();
                    }
                    let other = s.render_png(1, 1.).unwrap();
                    let mut d = line(Some(bounds));
                    if let CanvasShapeGeometry::Line {
                        start: a,
                        end: b,
                        dash_pattern,
                        ..
                    } = &mut d.geometry
                    {
                        (*a, *b) = if reverse { (end, start) } else { (start, end) };
                        *dash_pattern = vec![3., 2.];
                    }
                    d.opacity = alpha;
                    let original = s.bytes().unwrap();
                    s.create_canvas_shape(0, &d).unwrap();
                    let created = s.bytes().unwrap();
                    s.undo().unwrap();
                    assert_eq!(s.bytes().unwrap(), original);
                    s.redo().unwrap();
                    assert_eq!(s.bytes().unwrap(), created);
                    for current in [&s, &Session::open(&created).unwrap()] {
                        assert_stored(current, &d, bounds);
                        assert_eq!(current.info().pages[0].rotation, rotation);
                        assert_eq!(
                            current.canvas_shapes(0).unwrap(),
                            s.canvas_shapes(0).unwrap()
                        );
                    }
                    let wider = Rect {
                        x: bounds.x - 1.,
                        y: bounds.y - 1.,
                        width: bounds.width + 2.,
                        height: bounds.height + 2.,
                    };
                    set_frame(&mut d, Some(wider));
                    assert_eq!(
                        s.update_canvas_shape(&selected(&s), &d).unwrap(),
                        CanvasShapeCommit::Updated
                    );
                    let updated = s.bytes().unwrap();
                    s.undo().unwrap();
                    assert_eq!(s.bytes().unwrap(), created);
                    s.redo().unwrap();
                    assert_eq!(s.bytes().unwrap(), updated);
                    assert_stored(&Session::open(&updated).unwrap(), &d, wider);
                    assert_eq!(s.render_png(1, 1.).unwrap(), other);
                    seed_history(&mut s);
                    let other = s.render_png(1, 1.).unwrap();
                    let before = history(&mut s);
                    assert!(!before.past.is_empty() && !before.future.is_empty());
                    assert_eq!(
                        s.update_canvas_shape(&selected(&s), &d).unwrap(),
                        CanvasShapeCommit::Unchanged
                    );
                    assert_eq!(history(&mut s), before);
                    // Alpha-only update must not re-expand an explicit exact frame.
                    d.opacity = if alpha == 0.5 { 1. } else { 0.5 };
                    assert_eq!(
                        s.update_canvas_shape(&selected(&s), &d).unwrap(),
                        CanvasShapeCommit::Updated
                    );
                    assert_stored(&s, &d, wider);
                    assert_stored(&Session::open(&s.bytes().unwrap()).unwrap(), &d, wider);
                    assert_eq!(s.render_png(1, 1.).unwrap(), other);
                    let before_delete = s.bytes().unwrap();
                    s.delete_canvas_shape(&selected(&s)).unwrap();
                    let deleted = s.bytes().unwrap();
                    assert!(s.canvas_shapes(0).unwrap().annotations.is_empty());
                    assert!(
                        Session::open(&deleted)
                            .unwrap()
                            .canvas_shapes(0)
                            .unwrap()
                            .annotations
                            .is_empty()
                    );
                    assert_eq!(s.render_png(1, 1.).unwrap(), other);
                    s.undo().unwrap();
                    assert_eq!(s.bytes().unwrap(), before_delete);
                    s.redo().unwrap();
                    assert_eq!(s.bytes().unwrap(), deleted);
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 72);
}

#[test]
fn combined_edits_and_all_ten_endings_use_the_requested_frame() {
    let endings = [
        LineEnding::None,
        LineEnding::Square,
        LineEnding::Circle,
        LineEnding::Diamond,
        LineEnding::OpenArrow,
        LineEnding::ClosedArrow,
        LineEnding::Butt,
        LineEnding::ROpenArrow,
        LineEnding::RClosedArrow,
        LineEnding::Slash,
    ];
    for ending in endings {
        for alpha in [0., 0.5, 1.] {
            let mut s = blue_session();
            let other = s.render_png(1, 1.).unwrap();
            let mut d = line(Some(frame()));
            s.create_canvas_shape(0, &d).unwrap();
            let before = s.bytes().unwrap();
            let bounds = Rect {
                x: 20.,
                y: 10.,
                width: 150.,
                height: 100.,
            };
            d.geometry = CanvasShapeGeometry::Line {
                start: Point { x: 130., y: 80. },
                end: Point { x: 50., y: 40. },
                endings: [ending; 2],
                dash_pattern: vec![3., 2.],
                bounds: Some(bounds),
            };
            d.line_width = 3.;
            d.fill = Some(Color {
                r: 0.,
                g: 1.,
                b: 0.,
            });
            d.stroke = Color {
                r: 0.5,
                g: 0.,
                b: 1.,
            };
            d.opacity = alpha;
            assert_eq!(
                s.update_canvas_shape(&selected(&s), &d).unwrap(),
                CanvasShapeCommit::Updated
            );
            let after = s.bytes().unwrap();
            assert_stored(&s, &d, bounds);
            assert_stored(&Session::open(&after).unwrap(), &d, bounds);
            s.render_png(0, 1.).unwrap();
            assert_eq!(s.render_png(1, 1.).unwrap(), other);
            s.undo().unwrap();
            assert_eq!(s.bytes().unwrap(), before);
            s.redo().unwrap();
            assert_eq!(s.bytes().unwrap(), after);
        }
    }
}

#[test]
fn invalid_frames_points_widths_dashes_colors_opacities_fail_without_mutating_two_sided_history() {
    let mut s = blue_session();
    s.create_canvas_shape(0, &line(Some(frame()))).unwrap();
    seed_history(&mut s);
    let before = history(&mut s);
    assert!(!before.past.is_empty() && !before.future.is_empty());
    let mut invalid = Vec::new();
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        for field in 0..4 {
            let mut r = frame();
            match field {
                0 => r.x = value,
                1 => r.y = value,
                2 => r.width = value,
                _ => r.height = value,
            }
            invalid.push(line(Some(r)));
        }
    }
    for r in [
        Rect {
            width: 0.,
            ..frame()
        },
        Rect {
            height: 0.,
            ..frame()
        },
        Rect {
            width: -1.,
            ..frame()
        },
        Rect {
            height: -1.,
            ..frame()
        },
        Rect { x: -1., ..frame() },
        Rect { y: -1., ..frame() },
        Rect {
            width: 200.,
            ..frame()
        },
        Rect {
            height: 120.,
            ..frame()
        },
        Rect { x: 41., ..frame() },
        Rect { y: 61., ..frame() },
    ] {
        invalid.push(line(Some(r)));
    }
    for p in [
        Point { x: 37., y: 60. },
        Point { x: 143., y: 60. },
        Point { x: 40., y: 57. },
        Point { x: 40., y: 63. },
        Point {
            x: f32::NAN,
            y: 60.,
        },
        Point {
            x: f32::INFINITY,
            y: 60.,
        },
        Point {
            x: 40.,
            y: f32::NEG_INFINITY,
        },
        Point { x: 140., y: 60. },
    ] {
        let mut d = line(Some(frame()));
        if let CanvasShapeGeometry::Line { start, .. } = &mut d.geometry {
            *start = p;
        }
        invalid.push(d);
    }
    for width in [0., -1., 0.24, 32.1, f32::NAN, f32::INFINITY] {
        let mut d = line(Some(frame()));
        d.line_width = width;
        invalid.push(d);
    }
    for dash in [
        vec![-1.],
        vec![0., 0.],
        vec![1001.],
        vec![f32::NAN],
        vec![f32::INFINITY],
        vec![1.; 33],
    ] {
        let mut d = line(Some(frame()));
        if let CanvasShapeGeometry::Line { dash_pattern, .. } = &mut d.geometry {
            *dash_pattern = dash;
        }
        invalid.push(d);
    }
    for opacity in [-0.01, 1.01, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut d = line(Some(frame()));
        d.opacity = opacity;
        invalid.push(d);
    }
    for color in [
        Color {
            r: -0.1,
            g: 0.,
            b: 0.,
        },
        Color {
            r: 0.,
            g: 1.1,
            b: 0.,
        },
        Color {
            r: 0.,
            g: 0.,
            b: f32::NAN,
        },
    ] {
        for fill in [false, true] {
            let mut d = line(Some(frame()));
            if fill {
                d.fill = Some(color);
            } else {
                d.stroke = color;
            }
            invalid.push(d);
        }
    }
    assert_eq!(invalid.len(), 53);
    for (index, d) in invalid.iter().enumerate() {
        let current = selected(&s);
        assert!(s.create_canvas_shape(0, d).is_err(), "create case {index}");
        assert_eq!(history(&mut s), before, "create case {index}");
        assert!(
            s.update_canvas_shape(&current, d).is_err(),
            "update case {index}"
        );
        assert_eq!(history(&mut s), before, "update case {index}");
    }
    assert!(s.create_canvas_shape(2, &line(Some(frame()))).is_err());
    assert_eq!(history(&mut s), before);
}

#[test]
fn stale_missing_tampered_and_locked_selections_are_refused_before_mutation() {
    let mut s = blue_session();
    s.create_canvas_shape(0, &line(Some(frame()))).unwrap();
    let stale = selected(&s);
    seed_history(&mut s);
    let before = history(&mut s);
    let fresh = selected(&s);
    let mut forgeries = vec![stale];
    let mut bad = fresh.clone();
    bad.object_id = Some((u32::MAX, 0));
    forgeries.push(bad);
    let mut bad = fresh.clone();
    bad.original.annotation_index = 100;
    forgeries.push(bad);
    let mut bad = fresh.clone();
    bad.original.bounds.width += 1.;
    forgeries.push(bad);
    let mut bad = fresh.clone();
    bad.original.line.as_mut().unwrap().end.x -= 1.;
    forgeries.push(bad);
    let mut bad = fresh.clone();
    bad.page_id = (u32::MAX, 0);
    forgeries.push(bad);
    let mut bad = fresh.clone();
    bad.flags ^= 64;
    forgeries.push(bad);
    let mut bad = fresh;
    bad.opacity = 0.5;
    forgeries.push(bad);
    for bad in forgeries {
        assert!(s.validate_canvas_shape(&bad).is_err());
        assert!(s.update_canvas_shape(&bad, &line(Some(frame()))).is_err());
        assert!(s.delete_canvas_shape(&bad).is_err());
        assert!(s.render_canvas_shape_background_png(&bad, 1.).is_err());
        assert_eq!(history(&mut s), before);
    }
    for flags in [64, 128, 512] {
        let mut pdf = Document::load_mem(&s.bytes().unwrap()).unwrap();
        pdf.get_object_mut(selected(&s).object_id.unwrap())
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("F", flags);
        let mut locked = reopen(&mut pdf);
        seed_history(&mut locked);
        let target = selected(&locked);
        assert_eq!(target.flags, flags as u32);
        let before = history(&mut locked);
        assert!(locked.validate_canvas_shape(&target).is_err());
        assert!(
            locked
                .update_canvas_shape(&target, &line(Some(frame())))
                .is_err()
        );
        assert!(locked.delete_canvas_shape(&target).is_err());
        assert!(
            locked
                .render_canvas_shape_background_png(&target, 1.)
                .is_err()
        );
        assert_eq!(history(&mut locked), before);
    }
}

fn annots(pdf: &Document, page: (u32, u16)) -> Vec<Object> {
    pdf.dereference(
        pdf.get_object(page)
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

fn annotation(pdf: &Document, page: u32) -> (u32, u16) {
    annots(pdf, pdf.get_pages()[&page])[0]
        .as_reference()
        .unwrap()
}

#[test]
fn shared_exact_root_popup_author_metadata_and_other_page_are_copy_on_write() {
    for rotation in [0, 90, 180, 270] {
        for alpha in [0., 0.5, 1.] {
            let mut s = blue_session();
            let mut d = line(Some(frame()));
            d.opacity = alpha;
            s.create_canvas_shape(0, &d).unwrap();
            let mut pdf = Document::load_mem(&s.bytes().unwrap()).unwrap();
            let root = annotation(&pdf, 1);
            let pages = pdf.get_pages();
            let popup = pdf.add_object(dictionary! {
                "Subtype"=>"Popup", "Rect"=>vec![1.into(),1.into(),10.into(),10.into()],
                "Parent"=>root, "P"=>pages[&1], "UnknownPopup"=>"preserved",
            });
            let a = pdf.get_object_mut(root).unwrap().as_dict_mut().unwrap();
            a.set("Popup", popup);
            a.set("T", Object::string_literal("frame author"));
            a.set("Contents", Object::string_literal("frame source"));
            a.set("PrivateMetadata", dictionary! { "Keep"=>"yes" });
            let shared = pdf.add_object(vec![Object::Reference(root), Object::Reference(popup)]);
            for page in pages.values() {
                let p = pdf.get_object_mut(*page).unwrap().as_dict_mut().unwrap();
                p.set("Annots", shared);
                p.set("Rotate", rotation);
            }
            let mut s = reopen(&mut pdf);
            seed_history(&mut s);
            let target = selected(&s);
            let before = history(&mut s);
            let other_render = s.render_png(1, 1.).unwrap();
            let baseline = Document::load_mem(&before.bytes).unwrap();
            let old_root = baseline.get_object(root).unwrap().clone();
            let old_popup = baseline.get_object(popup).unwrap().clone();
            let old_page = baseline.get_object(pages[&2]).unwrap().clone();
            let wider = Rect {
                x: 35.,
                y: 55.,
                width: 110.,
                height: 10.,
            };
            set_frame(&mut d, Some(wider));
            assert_eq!(
                s.update_canvas_shape(&target, &d).unwrap(),
                CanvasShapeCommit::Updated
            );
            assert_stored(&s, &d, wider);
            let after = s.bytes().unwrap();
            let changed = Document::load_mem(&after).unwrap();
            let new_root = annotation(&changed, 1);
            assert_ne!(new_root, root);
            assert_eq!(annotation(&changed, 2), root);
            assert_eq!(changed.get_object(root).unwrap(), &old_root);
            assert_eq!(changed.get_object(popup).unwrap(), &old_popup);
            assert_eq!(changed.get_object(pages[&2]).unwrap(), &old_page);
            let a = changed.get_object(new_root).unwrap().as_dict().unwrap();
            for key in [b"T".as_slice(), b"Contents", b"PrivateMetadata"] {
                assert_eq!(
                    a.get(key).unwrap(),
                    old_root.as_dict().unwrap().get(key).unwrap()
                );
            }
            let new_popup = a.get(b"Popup").unwrap().as_reference().unwrap();
            assert_ne!(new_popup, popup);
            let p = changed.get_object(new_popup).unwrap().as_dict().unwrap();
            assert_eq!(p.get(b"Parent").unwrap().as_reference().unwrap(), new_root);
            assert_eq!(
                p.get(b"UnknownPopup").unwrap(),
                old_popup.as_dict().unwrap().get(b"UnknownPopup").unwrap()
            );
            assert_eq!(s.render_png(1, 1.).unwrap(), other_render);
            s.undo().unwrap();
            assert_eq!(s.bytes().unwrap(), before.bytes);
            s.redo().unwrap();
            assert_eq!(s.bytes().unwrap(), after);
            s.delete_canvas_shape(&selected(&s)).unwrap();
            assert!(s.canvas_shapes(0).unwrap().annotations.is_empty());
            assert!(
                annots(&Document::load_mem(&s.bytes().unwrap()).unwrap(), pages[&1]).is_empty()
            );
            assert_eq!(s.render_png(1, 1.).unwrap(), other_render);
            let deleted = Document::load_mem(&s.bytes().unwrap()).unwrap();
            assert_eq!(deleted.get_object(root).unwrap(), &old_root);
            assert_eq!(deleted.get_object(popup).unwrap(), &old_popup);
            assert_eq!(deleted.get_object(pages[&2]).unwrap(), &old_page);
            s.undo().unwrap();
            assert_eq!(s.bytes().unwrap(), after);
        }
    }
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
fn exact_horizontal_output_is_numerical_blue_purple_red_and_background_is_read_only() {
    for alpha in [0., 0.5, 1.] {
        let mut s = blue_session();
        let mut d = line(Some(frame()));
        d.opacity = alpha;
        s.create_canvas_shape(0, &d).unwrap();
        seed_history(&mut s);
        let before = history(&mut s);
        let reopened = Session::open(&before.bytes).unwrap();
        for current in [&s, &reopened] {
            assert_stored(current, &d, frame());
            let png = image::load_from_memory(&current.render_png(0, 1.).unwrap())
                .unwrap()
                .to_rgba8();
            // Pixel (80,59) samples source (80.5,60.5), strictly inside width2 shaft.
            assert_blend(png.get_pixel(80, 59).0, alpha);
            assert_blend(png.get_pixel(10, 10).0, 0.);
            let background = image::load_from_memory(
                &current
                    .render_canvas_shape_background_png(&selected(current), 1.)
                    .unwrap(),
            )
            .unwrap()
            .to_rgba8();
            assert_blend(background.get_pixel(80, 59).0, 0.);
            assert_blend(background.get_pixel(10, 10).0, 0.);
            assert!(background.pixels().all(|p| p.0 == [0, 0, 255, 255]));
        }
        assert_eq!(history(&mut s), before);
    }
}

#[test]
fn strict_original_appearance_refusal_cannot_be_bypassed_by_exact_target_omission() {
    for kind in 0..4 {
        let mut s = blue_session();
        s.create_canvas_shape(0, &line(Some(frame()))).unwrap();
        let mut pdf = Document::load_mem(&s.bytes().unwrap()).unwrap();
        let a = pdf
            .get_object_mut(selected(&s).object_id.unwrap())
            .unwrap()
            .as_dict_mut()
            .unwrap();
        match kind {
            0 => a.set("AP", 12),
            1 => a.set("AP", dictionary! { "N"=>12 }),
            _ => {
                let normal = a
                    .get(b"AP")
                    .unwrap()
                    .as_dict()
                    .unwrap()
                    .get(b"N")
                    .unwrap()
                    .clone();
                a.set("AP", dictionary! { "N"=>dictionary! { "On"=>normal } });
                if kind == 2 {
                    a.set("AS", Object::Name(b"Missing".to_vec()));
                }
            }
        }
        let mut s = reopen(&mut pdf);
        seed_history(&mut s);
        let target = selected(&s);
        let before = history(&mut s);
        let original_error = s.render_png(0, 1.).unwrap_err().to_string();
        assert!(original_error.contains("annotation"), "{original_error}");
        let omitted_error = s
            .render_canvas_shape_background_png(&target, 1.)
            .unwrap_err()
            .to_string();
        assert!(omitted_error.contains("annotation"), "{omitted_error}");
        assert_eq!(history(&mut s), before);
    }
}
