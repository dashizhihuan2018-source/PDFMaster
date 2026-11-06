use lopdf::{Document, Object, dictionary};
use pdfmaster_core::{
    CanvasShapeCommit, CanvasShapeDraft, CanvasShapeGeometry, Color, DocumentInfo, Edit,
    LineEnding, PdfMasterError, Point, Rect, Session,
};
use sha2::{Digest, Sha256};

fn rewrite(s: &Session, edit: impl FnOnce(&mut Document, Vec<(u32, u16)>)) -> Session {
    let mut d = Document::load_mem(&s.bytes().unwrap()).unwrap();
    let pages = d.get_pages().into_values().collect();
    edit(&mut d, pages);
    let mut bytes = Vec::new();
    d.save_to(&mut bytes).unwrap();
    Session::open(&bytes).unwrap()
}

fn entries(d: &Document, page: (u32, u16)) -> Vec<Object> {
    let a = d
        .get_object(page)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Annots")
        .unwrap();
    match a {
        Object::Reference(id) => d.get_object(*id).unwrap(),
        a => a,
    }
    .as_array()
    .unwrap()
    .clone()
}

fn annotation_id(d: &Document, page: (u32, u16)) -> (u32, u16) {
    entries(d, page)[0].as_reference().unwrap()
}

fn rectangle() -> CanvasShapeDraft {
    CanvasShapeDraft {
        geometry: CanvasShapeGeometry::Rectangle {
            bounds: Rect {
                x: 20.,
                y: 20.,
                width: 100.,
                height: 60.,
            },
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
fn flags_counts_direct_entries_and_budget_are_strict() {
    let mut s = Session::blank(200., 120., 2).unwrap();
    s.create_canvas_shape(0, &rectangle()).unwrap();
    for flag in [0, 4, 64, 128, 512, 1, 2, 32] {
        let s = rewrite(&s, |d, p| {
            let id = annotation_id(d, p[0]);
            let mut a = d.get_object(id).unwrap().as_dict().unwrap().clone();
            let f = d.add_object(Object::Integer(flag));
            a.set("F", f);
            // Direct annotations must keep None object identity in JSON.
            d.get_object_mut(p[0]).unwrap().as_dict_mut().unwrap().set("Annots", vec![Object::Dictionary(a), Object::Dictionary(dictionary! {"Subtype"=>"Text", "Rect"=>vec![1.into(),1.into(),10.into(),10.into()]})]);
        });
        let l = s.canvas_shapes(0).unwrap();
        assert_eq!(l.annotation_count, 2);
        if flag & (1 | 2 | 32) != 0 {
            assert!(l.annotations.is_empty());
            assert_eq!(l.unsupported_count, 2);
        } else {
            assert_eq!(l.unsupported_count, 1);
            assert_eq!(l.annotations[0].object_id, None);
            assert!(serde_json::to_value(&l.annotations[0]).unwrap()["object_id"].is_null());
            assert_eq!(
                s.validate_canvas_shape(&l.annotations[0]).is_err(),
                flag & (64 | 128 | 512) != 0
            );
        }
    }
    for f in [
        Object::Integer(-1),
        Object::Integer(u32::MAX as i64 + 1),
        Object::Real(4.5),
        Object::Null,
    ] {
        let bad = rewrite(&s, |d, p| {
            let id = annotation_id(d, p[0]);
            d.get_object_mut(id)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("F", f);
        });
        assert!(bad.canvas_shapes(0).is_err());
    }
    let missing = rewrite(&s, |d, p| {
        let id = annotation_id(d, p[0]);
        d.get_object_mut(id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .remove(b"F");
    });
    assert_eq!(missing.canvas_shapes(0).unwrap().annotations[0].flags, 0);
    for n in [1024, 1025] {
        let b = rewrite(&s, |d, p| {
            let a = entries(d, p[0])[0].clone();
            d.get_object_mut(p[0])
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("Annots", vec![a; n]);
        });
        if n == 1024 {
            assert_eq!(b.canvas_shapes(0).unwrap().annotation_count, n);
        } else {
            assert!(b.canvas_shapes(0).is_err());
        }
    }
    let bad_style = rewrite(&s, |d, p| {
        let id = annotation_id(d, p[0]);
        d.get_object_mut(id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("BS", 12);
    });
    assert!(bad_style.canvas_shapes(0).is_err());
}

#[test]
fn every_identity_field_and_unrelated_revision_change_is_rejected() {
    let mut s = Session::blank(200., 120., 2).unwrap();
    s.create_canvas_shape(0, &rectangle()).unwrap();
    s.create_canvas_shape(0, &rectangle()).unwrap();
    s.create_canvas_shape(1, &rectangle()).unwrap();
    s.apply(Edit::Rotate {
        page: 1,
        degrees: 90,
    })
    .unwrap();
    s.undo().unwrap();
    let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
    let mut forged = Vec::new();
    let mut x = selected.clone();
    x.revision[0] ^= 1;
    forged.push(x);
    let mut x = selected.clone();
    x.page_index = 1;
    forged.push(x);
    let mut x = selected.clone();
    x.page_id.0 += 1;
    forged.push(x);
    let mut x = selected.clone();
    x.object_id = None;
    forged.push(x);
    let mut x = selected.clone();
    x.flags = 0;
    forged.push(x);
    let mut x = selected.clone();
    x.original.bounds.x += 1.;
    forged.push(x);
    let mut x = selected.clone();
    x.original.annotation_index = 1;
    forged.push(x);
    let mut x = selected.clone();
    x.original.shape = pdfmaster_core::ShapeKind::Ellipse;
    forged.push(x);
    let mut x = selected.clone();
    x.original.stroke.b = 1.;
    forged.push(x);
    let mut x = selected.clone();
    x.original.fill = Some(Color {
        r: 0.,
        g: 1.,
        b: 0.,
    });
    forged.push(x);
    let mut x = selected.clone();
    x.original.line_width = 4.;
    forged.push(x);
    let mut x = selected.clone();
    x.original.line = Some(pdfmaster_core::LineGeometry {
        start: Point { x: 20., y: 20. },
        end: Point { x: 100., y: 80. },
        endings: [LineEnding::None; 2],
        dash_pattern: vec![],
    });
    forged.push(x);
    let before = s.bytes().unwrap();
    let info = s.info();
    for x in forged {
        assert!(s.validate_canvas_shape(&x).is_err());
        assert!(s.update_canvas_shape(&x, &rectangle()).is_err());
        assert!(s.delete_canvas_shape(&x).is_err());
        assert!(s.render_canvas_shape_background_png(&x, 1.).is_err());
    }
    assert_eq!(s.bytes().unwrap(), before);
    assert_eq!(s.info(), info);
    s.apply(pdfmaster_core::Edit::Rotate {
        page: 1,
        degrees: 90,
    })
    .unwrap();
    assert!(s.validate_canvas_shape(&selected).is_err());
}

#[test]
fn list_binds_full_bytes_page_occurrence_and_legacy_projection() {
    let mut s = Session::blank(200., 120., 2).unwrap();
    s.create_canvas_shape(0, &rectangle()).unwrap();
    let list = s.canvas_shapes(0).unwrap();
    let revision: [u8; 32] = Sha256::digest(s.bytes().unwrap()).into();
    assert_eq!(list.revision, revision);
    assert_eq!(list.annotation_count, 1);
    assert_eq!(list.unsupported_count, 0);
    let selected = &list.annotations[0];
    assert_eq!(selected.revision, revision);
    assert_eq!(selected.page_index, 0);
    assert!(selected.object_id.is_some());
    assert_eq!(selected.flags, 4);
    assert_eq!(selected.original, s.shapes(0).unwrap().shapes[0]);
    assert_eq!(
        serde_json::to_value(&selected.original).unwrap(),
        serde_json::to_value(&s.shapes(0).unwrap().shapes[0]).unwrap()
    );
    s.validate_canvas_shape(selected).unwrap();
    let empty = s.canvas_shapes(1).unwrap();
    assert_eq!(empty.revision, revision);
    assert_eq!(empty.annotation_count, 0);
    assert!(empty.annotations.is_empty());
}

fn line(start: Point, end: Point) -> CanvasShapeDraft {
    CanvasShapeDraft {
        geometry: CanvasShapeGeometry::Line {
            start,
            end,
            endings: [LineEnding::Diamond, LineEnding::ClosedArrow],
            dash_pattern: vec![3., 2.],
            bounds: None,
        },
        ..rectangle()
    }
}

#[test]
fn unchanged_and_stale_updates_preserve_exact_bytes_and_redo() {
    let mut s = Session::blank(200., 120., 2).unwrap();
    let d = rectangle();
    s.create_canvas_shape(0, &d).unwrap();
    let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
    let before = s.bytes().unwrap();
    let info = s.info();
    assert_eq!(
        s.update_canvas_shape(&selected, &d).unwrap(),
        CanvasShapeCommit::Unchanged
    );
    assert_eq!(s.bytes().unwrap(), before);
    assert_eq!(s.info(), info);
    s.apply(Edit::Rotate {
        page: 1,
        degrees: 90,
    })
    .unwrap();
    let current = s.bytes().unwrap();
    let info = s.info();
    assert!(s.update_canvas_shape(&selected, &d).is_err());
    assert!(s.delete_canvas_shape(&selected).is_err());
    assert_eq!(s.bytes().unwrap(), current);
    assert_eq!(s.info(), info);
    s.undo().unwrap();
    let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
    let info = s.info();
    assert!(info.can_redo);
    assert_eq!(
        s.update_canvas_shape(&selected, &d).unwrap(),
        CanvasShapeCommit::Unchanged
    );
    assert_eq!(s.bytes().unwrap(), before);
    assert_eq!(s.info(), info);
    s.redo().unwrap();
    assert_eq!(s.bytes().unwrap(), current);
}

#[test]
fn rectangle_and_ellipse_transactions_reopen_and_undo_exactly_once() {
    for ellipse in [false, true] {
        let mut s = Session::blank(200., 120., 2).unwrap();
        let blank = s.bytes().unwrap();
        let mut d = rectangle();
        if ellipse {
            d.geometry = CanvasShapeGeometry::Ellipse {
                bounds: Rect {
                    x: 20.,
                    y: 20.,
                    width: 100.,
                    height: 60.,
                },
            };
        }
        s.create_canvas_shape(0, &d).unwrap();
        let created = s.bytes().unwrap();
        s.undo().unwrap();
        assert_eq!(s.bytes().unwrap(), blank);
        s.redo().unwrap();
        assert_eq!(s.bytes().unwrap(), created);
        let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
        let bounds = Rect {
            x: 30.,
            y: 10.,
            width: 140.,
            height: 90.,
        };
        d.geometry = if ellipse {
            CanvasShapeGeometry::Ellipse { bounds }
        } else {
            CanvasShapeGeometry::Rectangle { bounds }
        };
        d.stroke = Color {
            r: 0.,
            g: 1.,
            b: 0.,
        };
        d.fill = Some(Color {
            r: 0.,
            g: 0.,
            b: 1.,
        });
        d.line_width = 4.;
        assert_eq!(
            s.update_canvas_shape(&selected, &d).unwrap(),
            CanvasShapeCommit::Updated
        );
        let updated = s.bytes().unwrap();
        let reopened = Session::open(&updated).unwrap();
        let a = reopened
            .canvas_shapes(0)
            .unwrap()
            .annotations
            .remove(0)
            .original;
        assert_eq!(a.bounds, bounds);
        assert_eq!(a.stroke, d.stroke);
        assert_eq!(a.fill, d.fill);
        assert_eq!(a.line_width, 4.);
        assert!(!reopened.render_png(0, 1.).unwrap().is_empty());
        s.undo().unwrap();
        assert_eq!(s.bytes().unwrap(), created);
        s.redo().unwrap();
        assert_eq!(s.bytes().unwrap(), updated);
        let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
        s.delete_canvas_shape(&selected).unwrap();
        assert_eq!(s.canvas_shapes(0).unwrap().annotation_count, 0);
        s.undo().unwrap();
        assert_eq!(s.bytes().unwrap(), updated);
    }
}

#[test]
fn real_line_endpoints_dashes_and_endings_are_single_transactions() {
    let mut s = Session::blank(200., 120., 2).unwrap();
    let blank = s.bytes().unwrap();
    let mut d = line(Point { x: 20., y: 20. }, Point { x: 120., y: 80. });
    s.create_canvas_shape(0, &d).unwrap();
    let created = s.bytes().unwrap();
    s.undo().unwrap();
    assert_eq!(s.bytes().unwrap(), blank);
    s.redo().unwrap();
    assert_eq!(s.bytes().unwrap(), created);
    let initial = s.canvas_shapes(0).unwrap();
    assert_eq!(initial.annotation_count, 1);
    assert_eq!(initial.annotations[0].flags, 4);
    assert_eq!(
        initial.annotations[0]
            .original
            .line
            .as_ref()
            .unwrap()
            .dash_pattern,
        vec![3., 2.]
    );
    let initial_pdf = Document::load_mem(&created).unwrap();
    let initial_a = initial_pdf
        .get_object(initial.annotations[0].object_id.unwrap())
        .unwrap()
        .as_dict()
        .unwrap();
    assert_eq!(
        initial_a.get(b"P").unwrap().as_reference().unwrap(),
        initial.annotations[0].page_id
    );
    assert_eq!(
        initial_a
            .get(b"BS")
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"S")
            .unwrap()
            .as_name()
            .unwrap(),
        b"D"
    );
    for (start, end) in [
        (Point { x: 20., y: 20. }, Point { x: 120., y: 20. }),
        (Point { x: 20., y: 20. }, Point { x: 20., y: 100. }),
        (Point { x: 20., y: 100. }, Point { x: 20., y: 20. }),
        (Point { x: 0., y: 120. }, Point { x: 200., y: 0. }),
    ] {
        let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
        let before = s.bytes().unwrap();
        d.geometry = CanvasShapeGeometry::Line {
            start,
            end,
            endings: [LineEnding::OpenArrow, LineEnding::Slash],
            dash_pattern: vec![7., 1., 0.],
            bounds: None,
        };
        d.stroke = Color {
            r: 0.,
            g: 0.,
            b: 1.,
        };
        d.line_width = 4.;
        assert_eq!(
            s.update_canvas_shape(&selected, &d).unwrap(),
            CanvasShapeCommit::Updated
        );
        let bytes = s.bytes().unwrap();
        let reopened = Session::open(&bytes).unwrap();
        let selected = reopened.canvas_shapes(0).unwrap().annotations.remove(0);
        let geometry = selected.original.line.unwrap();
        assert_eq!(geometry.start, start);
        assert_eq!(geometry.end, end);
        assert_eq!(geometry.endings, [LineEnding::OpenArrow, LineEnding::Slash]);
        assert_eq!(geometry.dash_pattern, vec![7., 1., 0.]);
        let pdf = Document::load_mem(&bytes).unwrap();
        let a = pdf
            .get_object(selected.object_id.unwrap())
            .unwrap()
            .as_dict()
            .unwrap();
        assert_eq!(
            a.get(b"L")
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_float().unwrap())
                .collect::<Vec<_>>(),
            vec![start.x, start.y, end.x, end.y]
        );
        assert_eq!(
            a.get(b"LE").unwrap().as_array().unwrap(),
            &vec![
                Object::Name(b"OpenArrow".to_vec()),
                Object::Name(b"Slash".to_vec())
            ]
        );
        assert_eq!(
            a.get(b"BS")
                .unwrap()
                .as_dict()
                .unwrap()
                .get(b"D")
                .unwrap()
                .as_array()
                .unwrap(),
            &vec![7.into(), 1.into(), 0.into()]
        );
        assert!(a.has(b"Rect") && a.has(b"AP") && a.has(b"M"));
        let b = selected.original.bounds;
        let rect = a
            .get(b"Rect")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_float().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(rect, vec![b.x, b.y, b.x + b.width, b.y + b.height]);
        let ap = pdf
            .dereference(a.get(b"AP").unwrap().as_dict().unwrap().get(b"N").unwrap())
            .unwrap()
            .1
            .as_stream()
            .unwrap();
        assert_eq!(
            ap.dict
                .get(b"BBox")
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_float().unwrap())
                .collect::<Vec<_>>(),
            vec![0., 0., b.width, b.height]
        );
        assert!(!reopened.render_png(0, 1.).unwrap().is_empty());
        s.undo().unwrap();
        assert_eq!(s.bytes().unwrap(), before);
        let current = s.canvas_shapes(0).unwrap().annotations.remove(0);
        // The unchanged geometry at the undo frontier must not clear redo.
        let old_line = current.original.line.clone().unwrap();
        let old_draft = CanvasShapeDraft {
            geometry: CanvasShapeGeometry::Line {
                start: old_line.start,
                end: old_line.end,
                endings: old_line.endings,
                dash_pattern: old_line.dash_pattern,
                bounds: None,
            },
            stroke: current.original.stroke,
            fill: current.original.fill,
            line_width: current.original.line_width,
            opacity: 1.,
        };
        let info = s.info();
        assert_eq!(
            s.update_canvas_shape(&current, &old_draft).unwrap(),
            CanvasShapeCommit::Unchanged
        );
        assert_eq!(s.info(), info);
        s.redo().unwrap();
        assert_eq!(s.bytes().unwrap(), bytes);
    }
    let json = serde_json::to_value(&d).unwrap();
    assert_eq!(json["geometry"]["kind"], "line");
    assert_eq!(serde_json::from_value::<CanvasShapeDraft>(json).unwrap(), d);
    assert_eq!(
        serde_json::to_value(CanvasShapeCommit::Unchanged).unwrap(),
        "unchanged"
    );
    assert_eq!(
        serde_json::to_value(CanvasShapeCommit::Updated).unwrap(),
        "updated"
    );
}

#[test]
fn malformed_border_name_and_overbudget_line_dash_fail_canvas_admission() {
    let mut s = Session::blank(200., 120., 2).unwrap();
    s.create_canvas_shape(0, &rectangle()).unwrap();
    let bad = rewrite(&s, |d, p| {
        let id = annotation_id(d, p[0]);
        d.get_object_mut(id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("BS", dictionary! {"W"=>2,"S"=>12});
    });
    assert!(
        bad.canvas_shapes(0).is_err(),
        "non-name BS/S must not be silently counted unsupported"
    );
    let mut s = Session::blank(200., 120., 2).unwrap();
    s.create_canvas_shape(
        0,
        &line(Point { x: 20., y: 20. }, Point { x: 100., y: 80. }),
    )
    .unwrap();
    let bad = rewrite(&s, |d, p| {
        let id = annotation_id(d, p[0]);
        d.get_object_mut(id).unwrap().as_dict_mut().unwrap().set(
            "BS",
            dictionary! {"W"=>2,"S"=>"D","D"=>vec![Object::Integer(1);33]},
        );
    });
    assert!(
        bad.canvas_shapes(0).is_err(),
        "dash budget must fail admission"
    );
}

fn legacy_border(s: &Session, border: Object) -> Session {
    rewrite(s, |d, p| {
        let id = annotation_id(d, p[0]);
        let a = d.get_object_mut(id).unwrap().as_dict_mut().unwrap();
        a.remove(b"BS");
        a.set("Border", border);
    })
}

fn malformed_legacy_borders() -> Vec<(String, Object)> {
    let mut cases = vec![
        ("short".into(), Object::Array(vec![0.into(), 0.into()])),
        ("empty".into(), Object::Array(vec![])),
        (
            "long".into(),
            Object::Array(vec![
                0.into(),
                0.into(),
                2.into(),
                vec![3.into()].into(),
                0.into(),
            ]),
        ),
        ("nonarray".into(), Object::Integer(2)),
        ("dangling array".into(), Object::Reference((999_999, 0))),
    ];
    for (kind, value) in [
        ("name", Object::Name(b"NotANumber".to_vec())),
        ("null", Object::Null),
        ("boolean", Object::Boolean(false)),
        ("array", Object::Array(vec![])),
        ("dictionary", Object::Dictionary(dictionary! {})),
        ("dangling", Object::Reference((999_999, 0))),
    ] {
        for index in 0..3 {
            let mut prefix = vec![0.into(), 0.into(), 2.into()];
            prefix[index] = value.clone();
            cases.push((format!("prefix {index} {kind}"), Object::Array(prefix)));
        }
        // A valid unsupported radius must not hide a malformed width.
        cases.push((
            format!("nonzero radius with {kind} width"),
            Object::Array(vec![1.into(), 0.into(), value.clone()]),
        ));
        if !matches!(value, Object::Array(_)) {
            cases.push((
                format!("dash {kind}"),
                Object::Array(vec![0.into(), 0.into(), 2.into(), value.clone()]),
            ));
        }
        cases.push((
            format!("dash entry {kind}"),
            Object::Array(vec![0.into(), 0.into(), 2.into(), vec![value].into()]),
        ));
    }
    cases
}

#[derive(Debug, PartialEq)]
struct ShapeHistory {
    bytes: Vec<u8>,
    info: DocumentInfo,
    past: Vec<Vec<u8>>,
    future: Vec<Vec<u8>>,
}

fn shape_history(s: &mut Session) -> ShapeHistory {
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
    ShapeHistory {
        bytes,
        info,
        past,
        future,
    }
}

#[test]
fn malformed_legacy_border_admission_and_guards_preserve_complete_history() {
    let mut failures = Vec::new();
    for draft in [
        rectangle(),
        CanvasShapeDraft {
            geometry: CanvasShapeGeometry::Ellipse {
                bounds: Rect {
                    x: 20.,
                    y: 20.,
                    width: 100.,
                    height: 60.,
                },
            },
            ..rectangle()
        },
        line(Point { x: 20., y: 20. }, Point { x: 120., y: 80. }),
    ] {
        let mut source = Session::blank(200., 120., 2).unwrap();
        source.create_canvas_shape(0, &draft).unwrap();
        // Capture a real complete selection before serializing/reopening bad input.
        let selected = source.canvas_shapes(0).unwrap().annotations.remove(0);
        for (case, border) in malformed_legacy_borders() {
            let mut s = legacy_border(&source, border);
            let label = format!("{:?}: {case}", selected.original.shape);
            // Legacy nonstrict projection still skips these imported profiles.
            let legacy = s.shapes(0).unwrap();
            assert!(legacy.shapes.is_empty(), "{label}");
            assert_eq!(legacy.unsupported_count, 1, "{label}");
            let result = s.canvas_shapes(0);
            if !matches!(result, Err(PdfMasterError::Malformed(_))) {
                failures.push(format!("{label}: list was not Malformed"));
            }
            for _ in 0..2 {
                s.apply(Edit::Rotate {
                    page: 1,
                    degrees: 90,
                })
                .unwrap();
            }
            s.undo().unwrap();
            assert!(s.info().can_undo && s.info().can_redo);
            let before = shape_history(&mut s);
            for refreshed_revision in [false, true] {
                let mut expected = selected.clone();
                if refreshed_revision {
                    // Exercise malformed re-projection, not just the stale guard.
                    expected.revision = Sha256::digest(s.bytes().unwrap()).into();
                }
                for (api, result) in [
                    ("validate", s.validate_canvas_shape(&expected)),
                    (
                        "update",
                        s.update_canvas_shape(&expected, &draft).map(|_| ()),
                    ),
                    ("delete", s.delete_canvas_shape(&expected)),
                    (
                        "background",
                        s.render_canvas_shape_background_png(&expected, 1.)
                            .map(|_| ()),
                    ),
                ] {
                    if result.is_ok()
                        || (refreshed_revision
                            && !matches!(result, Err(PdfMasterError::Malformed(_))))
                    {
                        failures.push(format!(
                            "{label}: {api} refreshed={refreshed_revision}: {result:?}"
                        ));
                    }
                }
                assert_eq!(shape_history(&mut s), before, "{label}");
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} malformed Border admission failures; first 12: {:#?}",
        failures.len(),
        &failures[..failures.len().min(12)]
    );
}

#[test]
fn valid_legacy_border_radii_and_dashes_keep_existing_support_profiles() {
    for draft in [
        rectangle(),
        CanvasShapeDraft {
            geometry: CanvasShapeGeometry::Ellipse {
                bounds: Rect {
                    x: 20.,
                    y: 20.,
                    width: 100.,
                    height: 60.,
                },
            },
            ..rectangle()
        },
        line(Point { x: 20., y: 20. }, Point { x: 120., y: 80. }),
    ] {
        let mut source = Session::blank(200., 120., 2).unwrap();
        source.create_canvas_shape(0, &draft).unwrap();
        let is_line = matches!(draft.geometry, CanvasShapeGeometry::Line { .. });
        for (border, supported) in [
            (vec![0.into(), 0.into(), 2.into()], true),
            (vec![Object::Real(1.5), 0.into(), 2.into()], false),
            (vec![0.into(), Object::Real(2.5), 2.into()], false),
            (
                vec![
                    1.into(),
                    2.into(),
                    2.into(),
                    vec![3.into(), 2.into()].into(),
                ],
                false,
            ),
            (
                vec![
                    0.into(),
                    0.into(),
                    2.into(),
                    vec![3.into(), 2.into()].into(),
                ],
                is_line,
            ),
        ] {
            let dashed = border.len() == 4;
            let s = legacy_border(&source, Object::Array(border));
            let list = s.canvas_shapes(0).unwrap();
            let legacy = s.shapes(0).unwrap();
            assert_eq!(list.annotation_count, 1);
            assert_eq!(list.annotations.len(), usize::from(supported));
            assert_eq!(list.unsupported_count, usize::from(!supported));
            assert_eq!(legacy.unsupported_count, list.unsupported_count);
            assert_eq!(
                list.annotations
                    .iter()
                    .map(|a| &a.original)
                    .collect::<Vec<_>>(),
                legacy.shapes.iter().collect::<Vec<_>>()
            );
            if supported && is_line {
                assert_eq!(
                    list.annotations[0]
                        .original
                        .line
                        .as_ref()
                        .unwrap()
                        .dash_pattern,
                    if dashed { vec![3., 2.] } else { vec![] }
                );
            }
        }
        let indirect = rewrite(&source, |d, p| {
            let numbers = [0, 0, 2].map(|n| Object::Reference(d.add_object(n)));
            let border = d.add_object(Object::Array(numbers.to_vec()));
            let id = annotation_id(d, p[0]);
            let a = d.get_object_mut(id).unwrap().as_dict_mut().unwrap();
            a.remove(b"BS");
            a.set("Border", border);
        });
        assert_eq!(indirect.canvas_shapes(0).unwrap().annotations.len(), 1);
    }
}

fn bs_ordering_cases() -> Vec<(String, Object)> {
    let mut cases = vec![
        ("non-dictionary BS".into(), 2.into()),
        ("dangling BS".into(), Object::Reference((999_999, 0))),
    ];
    for (kind, value) in [
        ("name", Object::Name(b"NotANumber".to_vec())),
        ("null", Object::Null),
        ("boolean", Object::Boolean(false)),
        ("array", Object::Array(vec![])),
        ("dictionary", Object::Dictionary(dictionary! {})),
        ("dangling", Object::Reference((999_999, 0))),
    ] {
        for style in ["S", "D", "B"] {
            cases.push((
                format!("{style} width {kind}"),
                dictionary! {"S"=>style, "W"=>value.clone()}.into(),
            ));
            if !matches!(value, Object::Array(_)) {
                cases.push((
                    format!("{style} dash {kind}"),
                    dictionary! {"S"=>style, "W"=>2, "D"=>value.clone()}.into(),
                ));
            }
            cases.push((
                format!("{style} dash entry {kind}"),
                dictionary! {"S"=>style, "W"=>2, "D"=>vec![value.clone()]}.into(),
            ));
        }
        if !matches!(value, Object::Name(_)) {
            cases.push((
                format!("style {kind}"),
                dictionary! {"S"=>value, "W"=>2}.into(),
            ));
        }
    }
    cases.push(("style number".into(), dictionary! {"S"=>2, "W"=>2}.into()));
    cases
}

fn known_structure_extensions(is_line: bool) -> Vec<(&'static str, Object)> {
    let mut extensions = vec![
        ("BE", dictionary! {"S"=>"C", "I"=>1}.into()),
        ("RD", vec![0.into(), 0.into(), 0.into(), 0.into()].into()),
        ("CA", 1.into()),
        ("Rotate", 0.into()),
    ];
    if is_line {
        extensions.extend([
            ("LL", 1.into()),
            ("LLE", 1.into()),
            ("LLO", 1.into()),
            ("Cap", Object::Boolean(true)),
            ("CP", Object::Name(b"Inline".to_vec())),
            ("CO", vec![0.into(), 0.into()].into()),
            ("IT", Object::Name(b"LineDimension".to_vec())),
            ("Measure", dictionary! {"Type"=>"Measure", "Subtype"=>"RL", "R"=>Object::string_literal("1 pt = 1 pt")}.into()),
            ("ca", 1.into()),
        ]);
    }
    extensions
}

fn malformed_known_structures(is_line: bool) -> Vec<(String, lopdf::Dictionary)> {
    let mut cases = Vec::new();
    for (kind, value) in [
        ("number", 2.into()),
        ("null", Object::Null),
        ("name", Object::Name(b"WrongType".to_vec())),
        ("dictionary", dictionary! {}.into()),
        ("dangling", Object::Reference((999_999, 0))),
        ("short", vec![0.into()].into()),
        ("entry", vec![0.into(), Object::Null, 0.into()].into()),
        (
            "dangling entry",
            vec![0.into(), Object::Reference((999_999, 0)), 0.into()].into(),
        ),
    ] {
        for key in ["C", "IC"] {
            let mut fields = dictionary! {};
            fields.set(key, value.clone());
            cases.push((format!("{key} {kind}"), fields));
        }
    }
    // Valid numeric but unsupported C must not suppress malformed IC or borders.
    cases.push((
        "out-of-range C hides IC".into(),
        dictionary! {"C"=>vec![2.into(),0.into(),0.into()], "IC"=>Object::Null},
    ));
    for key in ["C", "IC"] {
        let mut fields = dictionary! {"BS"=>dictionary! {"W"=>Object::Null, "S"=>"Unknown"}};
        fields.set(key, vec![2.into(), 0.into(), 0.into()]);
        cases.push((format!("unsupported {key} hides BS"), fields));
        let mut fields = dictionary! {"Border"=>vec![0.into(),0.into(),Object::Null]};
        fields.set(key, vec![2.into(), 0.into(), 0.into()]);
        cases.push((format!("unsupported {key} hides Border"), fields));
    }
    if is_line {
        for (kind, value) in [
            ("nonarray", Object::Null),
            ("dangling", Object::Reference((999_999, 0))),
            ("short", vec![20.into(), 20.into(), 120.into()].into()),
            (
                "long",
                vec![20.into(), 20.into(), 120.into(), 80.into(), 1.into()].into(),
            ),
        ] {
            cases.push((format!("L {kind}"), dictionary! {"L"=>value}));
        }
        for index in 0..4 {
            for (kind, value) in [
                ("null", Object::Null),
                ("dangling", Object::Reference((999_999, 0))),
            ] {
                let mut values = vec![20.into(), 20.into(), 120.into(), 80.into()];
                values[index] = value;
                cases.push((format!("L {index} {kind}"), dictionary! {"L"=>values}));
            }
        }
        for (kind, value) in [
            ("nonarray", Object::Null),
            ("dangling", Object::Reference((999_999, 0))),
            ("short", vec![Object::Name(b"None".to_vec())].into()),
            ("long", vec![Object::Name(b"None".to_vec()); 3].into()),
        ] {
            cases.push((format!("LE {kind}"), dictionary! {"LE"=>value}));
        }
        for index in 0..2 {
            for (kind, value) in [
                ("null", Object::Null),
                ("dangling", Object::Reference((999_999, 0))),
            ] {
                let mut values = vec![Object::Name(b"Unknown".to_vec()); 2];
                values[index] = value;
                cases.push((
                    format!("LE unknown plus {index} {kind}"),
                    dictionary! {"LE"=>values},
                ));
            }
        }
        cases.push(("unknown LE hides L".into(), dictionary! {"LE"=>vec![Object::Name(b"Unknown".to_vec());2], "L"=>vec![Object::Null,20.into(),120.into(),80.into()]}));
        let unknown_border = cases
            .iter()
            .filter(|(label, _)| label.starts_with('L'))
            .map(|(label, fields)| {
                let mut fields = fields.clone();
                fields.set("BS", dictionary! {"S"=>"Unknown", "W"=>2});
                (format!("unknown BS hides {label}"), fields)
            })
            .collect::<Vec<_>>();
        cases.extend(unknown_border);
    }
    cases
}

#[test]
fn known_structure_ordering_malformed_admission_and_guards_preserve_complete_history() {
    let mut failures = Vec::new();
    let mut count = 0;
    for draft in [
        rectangle(),
        CanvasShapeDraft {
            geometry: CanvasShapeGeometry::Ellipse {
                bounds: Rect {
                    x: 20.,
                    y: 20.,
                    width: 100.,
                    height: 60.,
                },
            },
            ..rectangle()
        },
        line(Point { x: 20., y: 20. }, Point { x: 120., y: 80. }),
    ] {
        let is_line = matches!(draft.geometry, CanvasShapeGeometry::Line { .. });
        let mut source = Session::blank(200., 120., 2).unwrap();
        source.create_canvas_shape(0, &draft).unwrap();
        let selected = source.canvas_shapes(0).unwrap().annotations.remove(0);
        let mut extensions = vec![None];
        extensions.extend(known_structure_extensions(is_line).into_iter().map(Some));
        for extended in extensions {
            for (case, fields) in malformed_known_structures(is_line) {
                count += 1;
                let mut s = rewrite(&source, |d, p| {
                    let id = annotation_id(d, p[0]);
                    let a = d.get_object_mut(id).unwrap().as_dict_mut().unwrap();
                    if fields.has(b"Border") {
                        a.remove(b"BS");
                    }
                    for (key, value) in fields.iter() {
                        a.set(key.clone(), value.clone());
                    }
                    if let Some((key, value)) = &extended {
                        a.set(*key, value.clone());
                    }
                });
                let label = format!(
                    "{:?}/{:?}: {case}",
                    selected.original.shape,
                    extended.as_ref().map(|(k, _)| k)
                );
                let legacy = s.shapes(0).unwrap();
                assert!(legacy.shapes.is_empty(), "{label}");
                assert_eq!(legacy.unsupported_count, 1, "{label}");
                let result = s.canvas_shapes(0).map(|_| ());
                if !matches!(result, Err(PdfMasterError::Malformed(_))) {
                    failures.push(format!("{label}: list {result:?}"));
                }
                for _ in 0..2 {
                    s.apply(Edit::Rotate {
                        page: 1,
                        degrees: 90,
                    })
                    .unwrap();
                }
                s.undo().unwrap();
                assert!(s.info().can_undo && s.info().can_redo);
                let before = shape_history(&mut s);
                for refreshed in [false, true] {
                    let mut expected = selected.clone();
                    if refreshed {
                        expected.revision = Sha256::digest(s.bytes().unwrap()).into();
                    }
                    for (api, result) in [
                        ("validate", s.validate_canvas_shape(&expected)),
                        (
                            "update",
                            s.update_canvas_shape(&expected, &draft).map(|_| ()),
                        ),
                        ("delete", s.delete_canvas_shape(&expected)),
                        (
                            "background",
                            s.render_canvas_shape_background_png(&expected, 1.)
                                .map(|_| ()),
                        ),
                    ] {
                        if result.is_ok()
                            || (refreshed && !matches!(result, Err(PdfMasterError::Malformed(_))))
                        {
                            failures
                                .push(format!("{label}: {api} refreshed={refreshed}: {result:?}"));
                        }
                    }
                    assert_eq!(shape_history(&mut s), before, "{label}");
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} known structure failures across {count} cases; first 12: {:#?}",
        failures.len(),
        &failures[..failures.len().min(12)]
    );
}

#[test]
fn known_structure_ordering_valid_profiles_keep_legacy_wire_and_border_precedence() {
    for draft in [
        rectangle(),
        CanvasShapeDraft {
            geometry: CanvasShapeGeometry::Ellipse {
                bounds: Rect {
                    x: 20.,
                    y: 20.,
                    width: 100.,
                    height: 60.,
                },
            },
            ..rectangle()
        },
        line(Point { x: 20., y: 20. }, Point { x: 120., y: 80. }),
    ] {
        let is_line = matches!(draft.geometry, CanvasShapeGeometry::Line { .. });
        let mut source = Session::blank(200., 120., 2).unwrap();
        source.create_canvas_shape(0, &draft).unwrap();
        let mut profiles = known_structure_extensions(is_line)
            .into_iter()
            .map(|(key, value)| {
                let mut fields = dictionary! {};
                fields.set(key, value);
                (key, fields, false)
            })
            .collect::<Vec<_>>();
        profiles.extend([
            (
                "unsupported C",
                dictionary! {"C"=>vec![2.into(),0.into(),0.into()]},
                false,
            ),
            (
                "unsupported IC",
                dictionary! {"IC"=>vec![2.into(),0.into(),0.into()]},
                false,
            ),
            (
                "unknown BS",
                dictionary! {"BS"=>dictionary! {"S"=>"Unknown","W"=>2}},
                false,
            ),
            (
                "BS ignores malformed Border",
                dictionary! {"Border"=>Object::Null},
                true,
            ),
        ]);
        if is_line {
            profiles.push((
                "unknown LE",
                dictionary! {"LE"=>vec![Object::Name(b"Unknown".to_vec());2]},
                false,
            ));
        }
        for (label, fields, supported) in profiles {
            let s = rewrite(&source, |d, p| {
                let id = annotation_id(d, p[0]);
                let a = d.get_object_mut(id).unwrap().as_dict_mut().unwrap();
                for (key, value) in fields.iter() {
                    a.set(key.clone(), value.clone());
                }
            });
            let before = s.bytes().unwrap();
            let info = s.info();
            let legacy = s.shapes(0).unwrap();
            let wire = serde_json::to_vec(&legacy).unwrap();
            let list = s.canvas_shapes(0).unwrap();
            assert_eq!(list.annotations.len(), usize::from(supported), "{label}");
            assert_eq!(list.unsupported_count, usize::from(!supported), "{label}");
            assert_eq!(
                list.annotations
                    .iter()
                    .map(|a| &a.original)
                    .collect::<Vec<_>>(),
                legacy.shapes.iter().collect::<Vec<_>>(),
                "{label}"
            );
            assert_eq!(
                serde_json::to_vec(&s.shapes(0).unwrap()).unwrap(),
                wire,
                "{label}"
            );
            assert_eq!(s.bytes().unwrap(), before, "{label}");
            assert_eq!(s.info(), info, "{label}");
        }
        // Known fields may themselves use indirect arrays and components/names.
        let s = rewrite(&source, |d, p| {
            let c = [0, 1, 0].map(|v| Object::Reference(d.add_object(v)));
            let c = d.add_object(c.to_vec());
            let l = [20, 20, 120, 80].map(|v| Object::Reference(d.add_object(v)));
            let l = d.add_object(l.to_vec());
            let le = ["None", "OpenArrow"]
                .map(|v| Object::Reference(d.add_object(Object::Name(v.as_bytes().to_vec()))));
            let le = d.add_object(le.to_vec());
            let id = annotation_id(d, p[0]);
            let a = d.get_object_mut(id).unwrap().as_dict_mut().unwrap();
            a.set("C", c);
            a.set("IC", c);
            if is_line {
                a.set("L", l);
                a.set("LE", le);
            }
        });
        let list = s.canvas_shapes(0).unwrap();
        assert_eq!(list.annotations.len(), 1);
        assert_eq!(list.annotations[0].original, s.shapes(0).unwrap().shapes[0]);
    }
}

#[test]
fn color_component_ordering_rejects_later_malformed_values_without_losing_history() {
    let mut failures = Vec::new();
    let mut cases = 0;
    for draft in [
        rectangle(),
        CanvasShapeDraft {
            geometry: CanvasShapeGeometry::Ellipse {
                bounds: Rect {
                    x: 20.,
                    y: 20.,
                    width: 100.,
                    height: 60.,
                },
            },
            ..rectangle()
        },
        line(Point { x: 20., y: 20. }, Point { x: 120., y: 80. }),
    ] {
        let mut source = Session::blank(200., 120., 2).unwrap();
        source.create_canvas_shape(0, &draft).unwrap();
        let selected = source.canvas_shapes(0).unwrap().annotations.remove(0);
        for key in ["C", "IC"] {
            for (early, later) in [(0, 1), (0, 2), (1, 2)] {
                for (kind, bad) in [
                    ("name", Object::Name(b"WrongType".to_vec())),
                    ("null", Object::Null),
                    ("boolean", Object::Boolean(false)),
                    ("array", Object::Array(vec![])),
                    ("dictionary", Object::Dictionary(dictionary! {})),
                    ("dangling", Object::Reference((999_999, 0))),
                ] {
                    for indirect in [false, true] {
                        cases += 1;
                        let mut s = rewrite(&source, |d, p| {
                            let mut components = vec![Object::Integer(0); 3];
                            components[early] = Object::Integer(2);
                            components[later] = bad.clone();
                            if indirect {
                                components = components
                                    .into_iter()
                                    .map(|v| {
                                        if matches!(v, Object::Reference(_)) {
                                            v
                                        } else {
                                            Object::Reference(d.add_object(v))
                                        }
                                    })
                                    .collect();
                            }
                            let value = if indirect {
                                Object::Reference(d.add_object(Object::Array(components)))
                            } else {
                                Object::Array(components)
                            };
                            let id = annotation_id(d, p[0]);
                            let a = d.get_object_mut(id).unwrap().as_dict_mut().unwrap();
                            a.set(key, value);
                            // A valid unsupported extension must not bypass known RGB structure.
                            a.set("BE", dictionary! {"S"=>"C", "I"=>1});
                        });
                        let label = format!(
                            "{:?}/{key}/{early}->{later}/{kind}/indirect={indirect}",
                            selected.original.shape
                        );
                        let legacy = s.shapes(0).unwrap();
                        assert_eq!(legacy.unsupported_count, 1, "{label}");
                        assert!(legacy.shapes.is_empty(), "{label}");
                        let result = s.canvas_shapes(0).map(|_| ());
                        if !matches!(result, Err(PdfMasterError::Malformed(_))) {
                            failures.push(format!("{label}: list {result:?}"));
                        }
                        for _ in 0..2 {
                            s.apply(Edit::Rotate {
                                page: 1,
                                degrees: 90,
                            })
                            .unwrap();
                        }
                        s.undo().unwrap();
                        assert!(s.info().can_undo && s.info().can_redo);
                        let before = shape_history(&mut s);
                        for refreshed in [false, true] {
                            let mut expected = selected.clone();
                            if refreshed {
                                expected.revision = Sha256::digest(s.bytes().unwrap()).into();
                            }
                            for (api, result) in [
                                ("validate", s.validate_canvas_shape(&expected)),
                                (
                                    "update",
                                    s.update_canvas_shape(&expected, &draft).map(|_| ()),
                                ),
                                ("delete", s.delete_canvas_shape(&expected)),
                                (
                                    "background",
                                    s.render_canvas_shape_background_png(&expected, 1.)
                                        .map(|_| ()),
                                ),
                            ] {
                                if result.is_ok()
                                    || (refreshed
                                        && !matches!(result, Err(PdfMasterError::Malformed(_))))
                                {
                                    failures.push(format!(
                                        "{label}: {api} refreshed={refreshed}: {result:?}"
                                    ));
                                }
                            }
                            assert_eq!(shape_history(&mut s), before, "{label}");
                        }
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} color ordering failures across {cases} reopened cases; first 12: {:#?}",
        failures.len(),
        &failures[..failures.len().min(12)]
    );
}

#[test]
fn color_component_ordering_all_numeric_out_of_range_stays_unsupported() {
    for draft in [
        rectangle(),
        CanvasShapeDraft {
            geometry: CanvasShapeGeometry::Ellipse {
                bounds: Rect {
                    x: 20.,
                    y: 20.,
                    width: 100.,
                    height: 60.,
                },
            },
            ..rectangle()
        },
        line(Point { x: 20., y: 20. }, Point { x: 120., y: 80. }),
    ] {
        let mut source = Session::blank(200., 120., 2).unwrap();
        source.create_canvas_shape(0, &draft).unwrap();
        for key in ["C", "IC"] {
            for early in [0, 1] {
                for indirect in [false, true] {
                    let s = rewrite(&source, |d, p| {
                        let mut values = vec![Object::Integer(0); 3];
                        values[early] = Object::Integer(2);
                        if indirect {
                            values = values
                                .into_iter()
                                .map(|v| Object::Reference(d.add_object(v)))
                                .collect();
                        }
                        let value = if indirect {
                            Object::Reference(d.add_object(values))
                        } else {
                            Object::Array(values)
                        };
                        let id = annotation_id(d, p[0]);
                        let a = d.get_object_mut(id).unwrap().as_dict_mut().unwrap();
                        a.set(key, value);
                        a.set("BE", dictionary! {"S"=>"C", "I"=>1});
                    });
                    let label = format!("{:?}/{key}/{early}/indirect={indirect}", draft.geometry);
                    let bytes = s.bytes().unwrap();
                    let info = s.info();
                    let legacy = s.shapes(0).unwrap();
                    let wire = serde_json::to_vec(&legacy).unwrap();
                    let list = s.canvas_shapes(0).unwrap();
                    assert_eq!(list.annotation_count, 1, "{label}");
                    assert_eq!(list.unsupported_count, 1, "{label}");
                    assert!(list.annotations.is_empty(), "{label}");
                    assert_eq!(
                        serde_json::to_vec(&s.shapes(0).unwrap()).unwrap(),
                        wire,
                        "{label}"
                    );
                    assert_eq!(s.bytes().unwrap(), bytes, "{label}");
                    assert_eq!(s.info(), info, "{label}");
                }
            }
        }
    }
}

#[test]
fn bs_ordering_malformed_admission_and_guards_preserve_complete_history() {
    let mut failures = Vec::new();
    for draft in [
        rectangle(),
        CanvasShapeDraft {
            geometry: CanvasShapeGeometry::Ellipse {
                bounds: Rect {
                    x: 20.,
                    y: 20.,
                    width: 100.,
                    height: 60.,
                },
            },
            ..rectangle()
        },
        line(Point { x: 20., y: 20. }, Point { x: 120., y: 80. }),
    ] {
        let mut source = Session::blank(200., 120., 2).unwrap();
        source.create_canvas_shape(0, &draft).unwrap();
        let selected = source.canvas_shapes(0).unwrap().annotations.remove(0);
        // Shared early exits must not hide malformed border dictionaries either.
        for extended in [
            None,
            Some("BE"),
            Some("RD"),
            Some("CA"),
            Some("Rotate"),
            Some("LL"),
        ] {
            if extended == Some("LL") && selected.original.shape != pdfmaster_core::ShapeKind::Line
            {
                continue;
            }
            let mut cases = bs_ordering_cases()
                .into_iter()
                .map(|(name, value)| (name, "BS", value, false))
                .collect::<Vec<_>>();
            if extended.is_some() {
                cases.push((
                    "legacy Border behind extended style".into(),
                    "Border",
                    vec![0.into(), 0.into(), Object::Null].into(),
                    false,
                ));
            }
            for style in ["S", "D", "B"] {
                let mut dash = vec![Object::Integer(1); 33];
                // The existing Line budget wins without inspecting entries past it.
                dash[32] = Object::Null;
                cases.push((
                    format!("{style} overbudget bad tail"),
                    "BS",
                    dictionary! {"S"=>style, "W"=>2, "D"=>dash}.into(),
                    selected.original.shape == pdfmaster_core::ShapeKind::Line,
                ));
                if selected.original.shape == pdfmaster_core::ShapeKind::Line {
                    cases.push((
                        format!("{style} overbudget numeric"),
                        "BS",
                        dictionary! {"S"=>style, "W"=>2, "D"=>vec![Object::Integer(1);33]}.into(),
                        true,
                    ));
                }
            }
            if selected.original.shape == pdfmaster_core::ShapeKind::Line {
                let mut dash = vec![Object::Integer(1); 33];
                dash[32] = Object::Null;
                cases.push((
                    "legacy Border overbudget bad tail".into(),
                    "Border",
                    vec![0.into(), 0.into(), 2.into(), dash.into()].into(),
                    true,
                ));
            }
            for (case, key, border, budget) in cases {
                let mut s = rewrite(&source, |d, p| {
                    let id = annotation_id(d, p[0]);
                    let a = d.get_object_mut(id).unwrap().as_dict_mut().unwrap();
                    a.remove(b"BS");
                    a.set(key, border);
                    if let Some(key) = extended {
                        a.set(key, 1);
                    }
                });
                let label = format!("{:?}/{extended:?}: {case}", selected.original.shape);
                let legacy = s.shapes(0).unwrap();
                assert!(legacy.shapes.is_empty(), "{label}");
                assert_eq!(legacy.unsupported_count, 1, "{label}");
                let expected_error = |result: &pdfmaster_core::Result<()>| {
                    matches!(
                        (budget, result),
                        (true, Err(PdfMasterError::Budget(_)))
                            | (false, Err(PdfMasterError::Malformed(_)))
                    )
                };
                let result = s.canvas_shapes(0).map(|_| ());
                if !expected_error(&result) {
                    failures.push(format!("{label}: list {result:?}, budget={budget}"));
                }
                for _ in 0..2 {
                    s.apply(Edit::Rotate {
                        page: 1,
                        degrees: 90,
                    })
                    .unwrap();
                }
                s.undo().unwrap();
                assert!(s.info().can_undo && s.info().can_redo);
                let before = shape_history(&mut s);
                for refreshed in [false, true] {
                    let mut expected = selected.clone();
                    if refreshed {
                        expected.revision = Sha256::digest(s.bytes().unwrap()).into();
                    }
                    for (api, result) in [
                        ("validate", s.validate_canvas_shape(&expected)),
                        (
                            "update",
                            s.update_canvas_shape(&expected, &draft).map(|_| ()),
                        ),
                        ("delete", s.delete_canvas_shape(&expected)),
                        (
                            "background",
                            s.render_canvas_shape_background_png(&expected, 1.)
                                .map(|_| ()),
                        ),
                    ] {
                        if result.is_ok() || (refreshed && !expected_error(&result)) {
                            failures
                                .push(format!("{label}: {api} refreshed={refreshed}: {result:?}"));
                        }
                    }
                    assert_eq!(shape_history(&mut s), before, "{label}");
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} BS ordering failures; first 12: {:#?}",
        failures.len(),
        &failures[..failures.len().min(12)]
    );
}

#[test]
fn bs_ordering_valid_profiles_and_indirect_numbers_keep_existing_support() {
    for draft in [
        rectangle(),
        CanvasShapeDraft {
            geometry: CanvasShapeGeometry::Ellipse {
                bounds: Rect {
                    x: 20.,
                    y: 20.,
                    width: 100.,
                    height: 60.,
                },
            },
            ..rectangle()
        },
        line(Point { x: 20., y: 20. }, Point { x: 120., y: 80. }),
    ] {
        let mut source = Session::blank(200., 120., 2).unwrap();
        source.create_canvas_shape(0, &draft).unwrap();
        let is_line = matches!(draft.geometry, CanvasShapeGeometry::Line { .. });
        for (style, dash, supported) in [
            (None, None, true),
            (Some("S"), None, true),
            (Some("D"), None, is_line),
            (Some("D"), Some(vec![3, 2]), is_line),
            (Some("B"), None, false),
            (Some("B"), Some(vec![3, 2]), false),
            (Some("S"), Some(vec![3, 2]), false),
            (Some("Unknown"), None, false),
        ] {
            for indirect in [false, true] {
                let s = rewrite(&source, |d, p| {
                    let mut bs = dictionary! {};
                    let width = if indirect {
                        Object::Reference(d.add_object(2))
                    } else {
                        2.into()
                    };
                    bs.set("W", width);
                    if let Some(style) = style {
                        let style = Object::Name(style.as_bytes().to_vec());
                        bs.set(
                            "S",
                            if indirect {
                                Object::Reference(d.add_object(style))
                            } else {
                                style
                            },
                        );
                    }
                    if let Some(dash) = &dash {
                        let dash = Object::Array(
                            dash.iter()
                                .map(|n| {
                                    if indirect {
                                        Object::Reference(d.add_object(*n))
                                    } else {
                                        (*n).into()
                                    }
                                })
                                .collect(),
                        );
                        bs.set(
                            "D",
                            if indirect {
                                Object::Reference(d.add_object(dash))
                            } else {
                                dash
                            },
                        );
                    }
                    let bs = if indirect {
                        Object::Reference(d.add_object(bs))
                    } else {
                        Object::Dictionary(bs)
                    };
                    let id = annotation_id(d, p[0]);
                    d.get_object_mut(id)
                        .unwrap()
                        .as_dict_mut()
                        .unwrap()
                        .set("BS", bs);
                });
                let legacy = s.shapes(0).unwrap();
                let list = s.canvas_shapes(0).unwrap();
                assert_eq!(list.annotation_count, 1);
                assert_eq!(list.annotations.len(), usize::from(supported));
                assert_eq!(list.unsupported_count, usize::from(!supported));
                assert_eq!(list.unsupported_count, legacy.unsupported_count);
                assert_eq!(
                    list.annotations
                        .iter()
                        .map(|a| &a.original)
                        .collect::<Vec<_>>(),
                    legacy.shapes.iter().collect::<Vec<_>>()
                );
                if supported && is_line {
                    assert_eq!(
                        list.annotations[0]
                            .original
                            .line
                            .as_ref()
                            .unwrap()
                            .dash_pattern,
                        if style == Some("D") {
                            dash.as_ref()
                                .map(|d| d.iter().map(|n| *n as f32).collect())
                                .unwrap_or(vec![3.])
                        } else {
                            vec![]
                        }
                    );
                }
            }
        }
        // Numeric arrays beyond 32 remain valid unsupported Square/Circle profiles.
        if !is_line {
            let s = rewrite(&source, |d, p| {
                let id = annotation_id(d, p[0]);
                d.get_object_mut(id).unwrap().as_dict_mut().unwrap().set(
                    "BS",
                    dictionary! {"S"=>"D", "W"=>2, "D"=>vec![Object::Integer(1);33]},
                );
            });
            assert_eq!(s.canvas_shapes(0).unwrap().unsupported_count, 1);
        }
    }
}

#[test]
fn invalid_drafts_subtypes_permissions_and_relationships_preserve_redo() {
    for is_line in [false, true] {
        let mut s = Session::blank(200., 120., 2).unwrap();
        let valid = if is_line {
            line(Point { x: 20., y: 20. }, Point { x: 120., y: 80. })
        } else {
            rectangle()
        };
        s.create_canvas_shape(0, &valid).unwrap();
        s.apply(Edit::Rotate {
            page: 1,
            degrees: 90,
        })
        .unwrap();
        s.undo().unwrap();
        let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
        let before = s.bytes().unwrap();
        let info = s.info();
        let mut invalid = Vec::new();
        for width in [0., -1., f32::NAN, f32::INFINITY, 1000.] {
            let mut d = valid.clone();
            d.line_width = width;
            invalid.push(d);
        }
        for color in [-1., 1.1, f32::NAN, f32::INFINITY] {
            let mut d = valid.clone();
            d.stroke.r = color;
            invalid.push(d);
            let mut d = valid.clone();
            d.fill = Some(Color {
                r: color,
                g: 0.,
                b: 0.,
            });
            invalid.push(d);
        }
        for bounds in [
            Rect {
                x: 0.,
                y: 0.,
                width: 0.,
                height: 10.,
            },
            Rect {
                x: -1.,
                y: 0.,
                width: 10.,
                height: 10.,
            },
            Rect {
                x: 190.,
                y: 0.,
                width: 20.,
                height: 10.,
            },
            Rect {
                x: 0.,
                y: f32::NAN,
                width: 10.,
                height: 10.,
            },
            Rect {
                x: 0.,
                y: 0.,
                width: 10.,
                height: f32::INFINITY,
            },
        ] {
            invalid.push(CanvasShapeDraft {
                geometry: CanvasShapeGeometry::Rectangle { bounds },
                ..valid.clone()
            });
        }
        for (start, end, dash) in [
            (Point { x: 20., y: 20. }, Point { x: 20., y: 20. }, vec![]),
            (
                Point {
                    x: f32::NAN,
                    y: 20.,
                },
                Point { x: 40., y: 20. },
                vec![],
            ),
            (Point { x: 201., y: 20. }, Point { x: 40., y: 20. }, vec![]),
            (
                Point { x: 20., y: 20. },
                Point { x: 40., y: 20. },
                vec![0.; 2],
            ),
            (
                Point { x: 20., y: 20. },
                Point { x: 40., y: 20. },
                vec![-1.],
            ),
            (
                Point { x: 20., y: 20. },
                Point { x: 40., y: 20. },
                vec![1001.],
            ),
            (
                Point { x: 20., y: 20. },
                Point { x: 40., y: 20. },
                vec![f32::NAN],
            ),
            (
                Point { x: 20., y: 20. },
                Point { x: 40., y: 20. },
                vec![1.; 33],
            ),
        ] {
            invalid.push(CanvasShapeDraft {
                geometry: CanvasShapeGeometry::Line {
                    start,
                    end,
                    endings: [LineEnding::None; 2],
                    dash_pattern: dash,
                    bounds: None,
                },
                ..valid.clone()
            });
        }
        for d in invalid {
            assert!(s.create_canvas_shape(0, &d).is_err());
            assert!(s.update_canvas_shape(&selected, &d).is_err());
            assert_eq!(s.bytes().unwrap(), before);
            assert_eq!(s.info(), info);
        }
        let wrong_type = CanvasShapeDraft {
            geometry: CanvasShapeGeometry::Ellipse {
                bounds: Rect {
                    x: 20.,
                    y: 20.,
                    width: 100.,
                    height: 60.,
                },
            },
            ..valid.clone()
        };
        assert!(s.update_canvas_shape(&selected, &wrong_type).is_err());
        assert_eq!(s.bytes().unwrap(), before);
        assert_eq!(s.info(), info);
    }
    for case in 0..6 {
        let mut s = Session::blank(200., 120., 2).unwrap();
        s.create_canvas_shape(0, &rectangle()).unwrap();
        let mut s = rewrite(&s, |d, p| {
            let id = annotation_id(d, p[0]);
            match case {
                0..=2 => d
                    .get_object_mut(id)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .set("F", [64, 128, 512][case]),
                3 => d
                    .get_object_mut(id)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .set("StructParent", 0),
                4 => d
                    .get_object_mut(id)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .set("IRT", id),
                _ => {
                    let reply=d.add_object(dictionary!{"Subtype"=>"Text","Rect"=>vec![1.into(),1.into(),10.into(),10.into()],"IRT"=>id});
                    d.get_object_mut(p[1])
                        .unwrap()
                        .as_dict_mut()
                        .unwrap()
                        .set("Annots", vec![reply.into()]);
                }
            }
        });
        s.apply(Edit::Rotate {
            page: 1,
            degrees: 90,
        })
        .unwrap();
        s.undo().unwrap();
        let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
        let before = s.bytes().unwrap();
        let info = s.info();
        assert!(s.validate_canvas_shape(&selected).is_err());
        assert!(s.update_canvas_shape(&selected, &rectangle()).is_err());
        assert!(s.delete_canvas_shape(&selected).is_err());
        assert!(s.render_canvas_shape_background_png(&selected, 1.).is_err());
        assert_eq!(s.bytes().unwrap(), before);
        assert_eq!(s.info(), info);
        s.redo().unwrap();
        s.undo().unwrap();
        assert_eq!(s.bytes().unwrap(), before);
    }
}

#[test]
fn shared_annotation_popup_and_metadata_cow_are_retained() {
    let mut s = Session::blank(200., 120., 2).unwrap();
    s.create_canvas_shape(0, &rectangle()).unwrap();
    let mut s = rewrite(&s, |d, p| {
        let id = annotation_id(d, p[0]);
        let popup=d.add_object(dictionary!{"Subtype"=>"Popup","Rect"=>vec![1.into(),1.into(),10.into(),10.into()],"P"=>p[0],"Parent"=>id});
        let a = d.get_object_mut(id).unwrap().as_dict_mut().unwrap();
        a.set("Contents", lopdf::text_string("custom shape metadata"));
        a.set("CustomKey", 123);
        a.set("Popup", popup);
        let shared = d.add_object(vec![id.into(), popup.into()]);
        for page in p {
            d.get_object_mut(page)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("Annots", shared);
        }
    });
    let before = s.bytes().unwrap();
    let old = Document::load_mem(&before).unwrap();
    let p = old.get_pages();
    let id = annotation_id(&old, p[&1]);
    let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
    let mut draft = rectangle();
    draft.line_width = 4.;
    assert_eq!(
        s.update_canvas_shape(&selected, &draft).unwrap(),
        CanvasShapeCommit::Updated
    );
    let updated = s.bytes().unwrap();
    let d = Document::load_mem(&updated).unwrap();
    assert_eq!(entries(&d, p[&2]), entries(&old, p[&2]));
    assert_eq!(d.get_object(id).unwrap(), old.get_object(id).unwrap());
    let new_id = annotation_id(&d, p[&1]);
    assert_ne!(new_id, id);
    let a = d.get_object(new_id).unwrap().as_dict().unwrap();
    assert_eq!(a.get(b"CustomKey").unwrap().as_i64().unwrap(), 123);
    assert_eq!(
        a.get(b"Contents").unwrap(),
        old.get_object(id)
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"Contents")
            .unwrap()
    );
    assert_eq!(a.get(b"P").unwrap().as_reference().unwrap(), p[&1]);
    let popup = a.get(b"Popup").unwrap().as_reference().unwrap();
    assert_eq!(
        d.get_object(popup)
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"Parent")
            .unwrap()
            .as_reference()
            .unwrap(),
        new_id
    );
    assert_ne!(entries(&d, p[&1])[1], entries(&old, p[&1])[1]);
    let current = s.canvas_shapes(0).unwrap().annotations.remove(0);
    s.delete_canvas_shape(&current).unwrap();
    let deleted = Document::load_mem(&s.bytes().unwrap()).unwrap();
    assert!(entries(&deleted, p[&1]).is_empty());
    assert_eq!(entries(&deleted, p[&2]), entries(&old, p[&2]));
    s.undo().unwrap();
    assert_eq!(s.bytes().unwrap(), updated);
    s.undo().unwrap();
    assert_eq!(s.bytes().unwrap(), before);
}
