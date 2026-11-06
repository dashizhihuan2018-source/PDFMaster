use pdfmaster_drawing::Point;
use pdfmaster_ui::canvas_shapes::*;
fn page() -> Page {
    Page {
        index: 0,
        width: 200.,
        height: 120.,
        rotation: 0,
    }
}
#[test]
fn horizontal_creation_and_affine_frame_are_not_endpoint_tilt() {
    let page = page();
    let d = new_draft(
        Kind::Line,
        Point { x: 40., y: 60. },
        Point { x: 140., y: 60. },
        Style::default(),
        page,
    )
    .unwrap();
    assert_eq!(
        d.bounds(page).unwrap(),
        Rect {
            x: 38.,
            y: 58.,
            width: 104.,
            height: 4.
        }
    );
    let next = Rect {
        x: 38.,
        y: 58.,
        width: 124.,
        height: 24.,
    };
    let moved = transform(&d, next, page).unwrap();
    let Geometry::Line {
        start, end, bounds, ..
    } = moved.geometry
    else {
        panic!("line")
    };
    assert_eq!(bounds, Some(next));
    assert_eq!(start.y, end.y);
    assert_eq!(start.y, 70.);
    assert!(start.x < end.x);
}
#[test]
fn rotation_corners_keep_source_handles() {
    let p = Page {
        rotation: 90,
        ..page()
    };
    let corners = display_corners(
        Rect {
            x: 38.,
            y: 58.,
            width: 104.,
            height: 4.,
        },
        Frame {
            left: 10.,
            top: 20.,
            width: 240.,
            height: 100.,
        },
        p,
    )
    .unwrap();
    assert_eq!(
        corners,
        [
            (Handle::Sw, Point { x: 126., y: 39. }),
            (Handle::Se, Point { x: 126., y: 91. }),
            (Handle::Ne, Point { x: 134., y: 91. }),
            (Handle::Nw, Point { x: 134., y: 39. })
        ]
    );
}
fn rect() -> Rect {
    Rect {
        x: 20.,
        y: 20.,
        width: 80.,
        height: 60.,
    }
}
fn area() -> Draft {
    Draft {
        geometry: Geometry::Rectangle { bounds: rect() },
        stroke: Style::default().stroke,
        fill: Some(FILL_SWATCH),
        line_width: 2.,
        opacity: 1.,
    }
}
fn selection(d: &Draft, index: usize) -> Selection {
    let (shape, line) = match &d.geometry {
        Geometry::Rectangle { .. } => (ShapeKind::Rectangle, None),
        Geometry::Ellipse { .. } => (ShapeKind::Ellipse, None),
        Geometry::Line {
            start,
            end,
            endings,
            dash_pattern,
            ..
        } => (
            ShapeKind::Line,
            Some(LineGeometry {
                start: *start,
                end: *end,
                endings: *endings,
                dash_pattern: dash_pattern.clone(),
            }),
        ),
    };
    Selection {
        page_index: 0,
        page_id: (4, 0),
        object_id: Some((20 + index as u32, 0)),
        revision: [3; 32],
        flags: 0,
        opacity: d.opacity,
        original: Annotation {
            annotation_index: index,
            shape,
            bounds: d.bounds(page()).unwrap_or(rect()),
            stroke: d.stroke,
            fill: d.fill,
            line_width: d.line_width,
            line,
        },
    }
}
fn list(s: Selection) -> List {
    List {
        annotations: vec![s],
        unsupported_count: 0,
        annotation_count: 1,
        revision: [3; 32],
    }
}
fn row(s: &Selection) -> pdfmaster_ui::annotations::Row {
    pdfmaster_ui::annotations::Row {
        page_index: s.page_index,
        annotation_index: s.original.annotation_index,
        page_id: s.page_id,
        object_id: s.object_id,
        subtype: match s.original.shape {
            ShapeKind::Rectangle => "Square",
            ShapeKind::Ellipse => "Circle",
            ShapeKind::Line => "Line",
        }
        .into(),
        contents: String::new(),
        color: None,
        bounds: Some(pdfmaster_ui::annotations::Bounds {
            x: s.original.bounds.x,
            y: s.original.bounds.y,
            width: s.original.bounds.width,
            height: s.original.bounds.height,
        }),
        modified: None,
        modified_millis: None,
    }
}
fn frame() -> Frame {
    Frame {
        left: 0.,
        top: 0.,
        width: 200.,
        height: 120.,
    }
}

#[test]
fn complete_off_page_generic_bounds_only_refuse_actual_overlapping_hits() {
    for rotation in [0, 90, 180, 270] {
        let p = Page { rotation, ..page() };
        let s = selection(&area(), 0);
        let mut l = list(s.clone());
        l.annotation_count = 3;
        l.unsupported_count = 2;
        let mut widget = row(&s);
        widget.annotation_index = 1;
        widget.object_id = Some((50, 0));
        widget.subtype = "Widget".into();
        widget.bounds = Some(pdfmaster_ui::annotations::Bounds {
            x: -30.,
            y: 30.,
            width: 40.,
            height: 40.,
        });
        let mut popup = widget.clone();
        popup.annotation_index = 2;
        popup.object_id = None;
        popup.subtype = "Popup".into();
        popup.bounds = Some(pdfmaster_ui::annotations::Bounds {
            x: 250.,
            y: 0.,
            width: 40.,
            height: 40.,
        });
        let rows = vec![row(&s), widget.clone(), popup.clone()];
        let at = |x, y| {
            pdfmaster_ui::canvas_notes::page_anchor(
                Point { x, y },
                frame(),
                p.width,
                p.height,
                p.rotation,
            )
            .unwrap()
        };
        assert_eq!(
            hit_test(&l, &rows, None, at(170., 100.), frame(), p).unwrap(),
            None
        );
        assert!(
            hit_test(&l, &rows, None, at(5., 50.), frame(), p)
                .unwrap_err()
                .contains("unsupported")
        );
        for bad in [
            None,
            Some(pdfmaster_ui::annotations::Bounds {
                x: 0.,
                y: 0.,
                width: 0.,
                height: 4.,
            }),
            Some(pdfmaster_ui::annotations::Bounds {
                x: f32::NAN,
                y: 0.,
                width: 4.,
                height: 4.,
            }),
        ] {
            let mut invalid = rows.clone();
            invalid[1].bounds = bad;
            assert!(hit_test(&l, &invalid, None, at(170., 100.), frame(), p).is_err());
        }
    }
}
#[test]
fn complete_wire_roundtrips_and_required_identity_defaults() {
    let d = area();
    let s = selection(&d, 0);
    for target in [
        Target::New {
            draft: d.clone(),
            revision: [3; 32],
        },
        Target::Existing {
            original: s.clone(),
        },
    ] {
        let owner = Owner {
            id: 7,
            request: "req".into(),
            page: 0,
            target,
        };
        let v = serde_json::to_value(&owner).unwrap();
        assert_eq!(serde_json::from_value::<Owner>(v.clone()).unwrap(), owner);
        for key in ["id", "request", "page", "target"] {
            let mut bad = v.clone();
            bad.as_object_mut().unwrap().remove(key);
            assert!(serde_json::from_value::<Owner>(bad).is_err());
        }
    }
    let p = Payload {
        id: 7,
        request: "req".into(),
        draft: d.clone(),
    };
    assert_eq!(
        serde_json::from_value::<Payload>(serde_json::to_value(&p).unwrap()).unwrap(),
        p
    );
    let v = serde_json::to_value(&s).unwrap();
    for key in [
        "page_index",
        "page_id",
        "object_id",
        "revision",
        "flags",
        "original",
    ] {
        let mut bad = v.clone();
        bad.as_object_mut().unwrap().remove(key);
        assert!(
            serde_json::from_value::<Selection>(bad).is_err(),
            "missing {key}"
        );
    }
    let mut v = serde_json::to_value(&d).unwrap();
    v.as_object_mut().unwrap().remove("opacity");
    assert_eq!(serde_json::from_value::<Draft>(v).unwrap().opacity, 1.);
    let mut v = serde_json::to_value(&s).unwrap();
    v.as_object_mut().unwrap().remove("opacity");
    assert_eq!(serde_json::from_value::<Selection>(v).unwrap().opacity, 1.);
    let line = serde_json::json!({"kind":"line","start":{"x":20.,"y":20.},"end":{"x":80.,"y":60.},"endings":["none","none"],"dash_pattern":[]});
    for mut v in [line.clone(), line] {
        if let Some(o) = v.as_object_mut() {
            o.remove("bounds");
        }
        let Geometry::Line { bounds, .. } = serde_json::from_value(v).unwrap() else {
            panic!()
        };
        assert_eq!(bounds, None);
    }
    let mut line = serde_json::json!({"kind":"line","start":{"x":20.,"y":20.},"end":{"x":80.,"y":60.},"endings":["none","none"],"dash_pattern":[],"bounds":null});
    assert!(matches!(
        serde_json::from_value::<Geometry>(line.take()).unwrap(),
        Geometry::Line { bounds: None, .. }
    ));
    let mut bad = s;
    bad.original.shape = ShapeKind::Line;
    assert!(bad.draft().is_err());
}
#[test]
fn all_creation_kinds_rotations_widths_reverse_axis_edges() {
    for rotation in [0, 90, 180, 270] {
        let p = Page { rotation, ..page() };
        for kind in [
            Kind::Rectangle,
            Kind::Ellipse,
            Kind::Line,
            Kind::Arrow,
            Kind::DoubleArrow,
        ] {
            for width in [0.25, 12., 32.] {
                for (start, end) in [
                    (Point { x: 5., y: 5. }, Point { x: 195., y: 115. }),
                    (Point { x: 195., y: 115. }, Point { x: 5., y: 5. }),
                    (Point { x: 0., y: 0. }, Point { x: 200., y: 120. }),
                ] {
                    let d = new_draft(
                        kind,
                        start,
                        end,
                        Style {
                            line_width: width,
                            ..Style::default()
                        },
                        p,
                    )
                    .unwrap();
                    d.validate(p).unwrap();
                    assert_eq!(d.line_width, width);
                    assert_eq!(d.bounds(p).unwrap(), d.bounds(page()).unwrap());
                }
            }
        }
    }
    for kind in [Kind::Line, Kind::Arrow, Kind::DoubleArrow] {
        for (a, b) in [
            (Point { x: 40., y: 60. }, Point { x: 140., y: 60. }),
            (Point { x: 60., y: 20. }, Point { x: 60., y: 100. }),
        ] {
            assert!(new_draft(kind, a, b, Style::default(), page()).is_ok());
        }
    }
    for kind in [
        Kind::Rectangle,
        Kind::Ellipse,
        Kind::Line,
        Kind::Arrow,
        Kind::DoubleArrow,
    ] {
        let a = Point { x: 50., y: 50. };
        assert!(new_draft(kind, a, a, Style::default(), page()).is_err());
        assert!(
            new_draft(
                kind,
                Point { x: -1., y: 50. },
                Point { x: 100., y: 90. },
                Style::default(),
                page()
            )
            .is_err()
        );
    }
    assert!(
        new_draft(
            Kind::Rectangle,
            Point { x: 50., y: 50. },
            Point { x: 51., y: 51. },
            Style::default(),
            page()
        )
        .is_err()
    );
    assert_eq!(transparency(0.).unwrap(), 1.);
    assert_eq!(transparency(100.).unwrap(), 0.);
    assert_eq!(transparency(50.).unwrap(), 0.5);
    for v in [-1., 101., f32::NAN] {
        assert!(transparency(v).is_err());
    }
    assert!(Kind::Rectangle.supports_fill());
    assert!(Kind::Ellipse.supports_fill());
    assert!(!Kind::Arrow.supports_fill());
    assert_eq!(Kind::DoubleArrow.endings(), [LineEnding::OpenArrow; 2]);
}
#[test]
fn finite_admission_never_clamps_input() {
    let good = area();
    good.validate(page()).unwrap();
    for v in [f32::NAN, f32::INFINITY, -1., 0., 60.] {
        let mut d = good.clone();
        d.line_width = v;
        assert!(d.validate(page()).is_err(), "width {v}");
    }
    for v in [f32::NAN, -0.01, 1.01] {
        let mut d = good.clone();
        d.opacity = v;
        assert!(d.validate(page()).is_err());
        d = good.clone();
        d.stroke.r = v;
        assert!(d.validate(page()).is_err());
        d = good.clone();
        d.fill.as_mut().unwrap().b = v;
        assert!(d.validate(page()).is_err());
    }
    for b in [
        Rect { x: -1., ..rect() },
        Rect {
            width: 0.,
            ..rect()
        },
        Rect {
            x: f32::NAN,
            ..rect()
        },
        Rect {
            width: 300.,
            ..rect()
        },
    ] {
        let d = Draft {
            geometry: Geometry::Rectangle { bounds: b },
            ..good.clone()
        };
        assert!(d.validate(page()).is_err());
    }
    for rotation in [1, 45] {
        assert!(good.validate(Page { rotation, ..page() }).is_err());
    }
    assert!(
        good.validate(Page {
            width: f32::INFINITY,
            ..page()
        })
        .is_err()
    );
    let line = new_draft(
        Kind::Line,
        Point { x: 20., y: 20. },
        Point { x: 100., y: 60. },
        Style::default(),
        page(),
    )
    .unwrap();
    for width in [0.24, 32.01, f32::NAN] {
        assert!(
            Draft {
                line_width: width,
                ..line.clone()
            }
            .validate(page())
            .is_err()
        );
    }
    for dash in [
        vec![0., 0.],
        vec![1.; 33],
        vec![-1.],
        vec![1000.01],
        vec![f32::NAN],
    ] {
        let mut d = line.clone();
        if let Geometry::Line { dash_pattern, .. } = &mut d.geometry {
            *dash_pattern = dash;
        }
        assert!(d.validate(page()).is_err());
    }
    for point in [
        Point { x: 300., y: 20. },
        Point { x: 20., y: 20. },
        Point {
            x: f32::NAN,
            y: 60.,
        },
    ] {
        let mut d = line.clone();
        if let Geometry::Line { end, .. } = &mut d.geometry {
            *end = point;
        }
        assert!(d.validate(page()).is_err());
    }
    let mut historic = line.clone();
    if let Geometry::Line { bounds, .. } = &mut historic.geometry {
        *bounds = None;
    }
    assert_eq!(
        historic.bounds(page()).unwrap(),
        Rect {
            x: 8.,
            y: 8.,
            width: 104.,
            height: 64.
        }
    );
}
#[test]
fn moves_clamp_one_vector_and_four_corners_cross() {
    let b = rect();
    let moved = drag_bounds(
        b,
        Point { x: 20., y: 20. },
        Point { x: 400., y: -400. },
        Handle::Move,
        page(),
    )
    .unwrap();
    assert_eq!(
        moved,
        Rect {
            x: 120.,
            y: 0.,
            ..b
        }
    );
    for (h, opposite) in [
        (Handle::Sw, Point { x: 100., y: 80. }),
        (Handle::Se, Point { x: 20., y: 80. }),
        (Handle::Ne, Point { x: 20., y: 20. }),
        (Handle::Nw, Point { x: 100., y: 20. }),
    ] {
        let p = Point {
            x: opposite.x + 10.,
            y: opposite.y + 10.,
        };
        assert_eq!(
            drag_bounds(b, Point { x: 0., y: 0. }, p, h, page()).unwrap(),
            Rect {
                x: opposite.x,
                y: opposite.y,
                width: 10.,
                height: 10.
            }
        );
        assert!(drag_bounds(b, p, opposite, h, page()).is_err());
    }
    let mut line = new_draft(
        Kind::DoubleArrow,
        Point { x: 140., y: 80. },
        Point { x: 40., y: 30. },
        Style::default(),
        page(),
    )
    .unwrap();
    if let Geometry::Line {
        dash_pattern,
        endings,
        ..
    } = &mut line.geometry
    {
        *dash_pattern = vec![2., 3.];
        *endings = [LineEnding::Diamond, LineEnding::Slash];
    }
    let d = transform(
        &line,
        Rect {
            x: 30.,
            y: 30.,
            width: 100.,
            height: 70.,
        },
        page(),
    )
    .unwrap();
    let Geometry::Line {
        start,
        end,
        endings,
        dash_pattern,
        ..
    } = d.geometry
    else {
        panic!()
    };
    assert!(start.x > end.x);
    assert!(start.y > end.y);
    assert_eq!(endings, [LineEnding::Diamond, LineEnding::Slash]);
    assert_eq!(dash_pattern, vec![2., 3.]);
    for rotation in [0, 90, 180, 270] {
        let p = Page { rotation, ..page() };
        let c = display_corners(b, frame(), p).unwrap();
        for (h, q) in c {
            let source = match h {
                Handle::Sw => Point { x: 20., y: 20. },
                Handle::Se => Point { x: 100., y: 20. },
                Handle::Ne => Point { x: 100., y: 80. },
                Handle::Nw => Point { x: 20., y: 80. },
                _ => panic!(),
            };
            assert_eq!(
                q,
                pdfmaster_ui::canvas_notes::page_anchor(source, frame(), 200., 120., rotation)
                    .unwrap()
            );
        }
    }
    assert!(
        display_corners(
            b,
            Frame {
                width: 0.,
                ..frame()
            },
            page()
        )
        .is_err()
    );
}
#[test]
fn topmost_full_identity_hits_links_locks_and_alpha_zero() {
    let mut s = selection(&area(), 0);
    s.opacity = 0.;
    let l = list(s.clone());
    let r = row(&s);
    assert_eq!(
        hit_test(
            &l,
            std::slice::from_ref(&r),
            None,
            Point { x: 50., y: 70. },
            frame(),
            page()
        )
        .unwrap(),
        Some(Hit::Existing(s.clone()))
    );
    for flag in [64, 128, 512] {
        let mut locked = s.clone();
        locked.flags = flag;
        assert!(
            hit_test(
                &list(locked),
                std::slice::from_ref(&r),
                None,
                Point { x: 50., y: 70. },
                frame(),
                page()
            )
            .is_err()
        );
    }
    let mut unsupported = r.clone();
    unsupported.annotation_index = 1;
    unsupported.object_id = Some((99, 0));
    unsupported.subtype = "FreeText".into();
    let mut l = l;
    l.annotation_count = 2;
    l.unsupported_count = 1;
    assert!(
        hit_test(
            &l,
            &[r.clone(), unsupported.clone()],
            None,
            Point { x: 50., y: 70. },
            frame(),
            page()
        )
        .is_err()
    );
    unsupported.subtype = "Link".into();
    assert!(matches!(
        hit_test(
            &l,
            &[r.clone(), unsupported.clone()],
            None,
            Point { x: 50., y: 70. },
            frame(),
            page()
        )
        .unwrap(),
        Some(Hit::Existing(_))
    ));
    unsupported.subtype = "FreeText".into();
    unsupported.page_index = 1;
    // This page has one occurrence; another page's row is not count evidence.
    let current_page_list = list(s.clone());
    assert!(
        hit_test(
            &current_page_list,
            &[r.clone(), unsupported.clone()],
            None,
            Point { x: 50., y: 70. },
            frame(),
            page()
        )
        .unwrap()
        .is_some()
    );
    unsupported.page_index = 0;
    unsupported.bounds = None;
    assert!(
        hit_test(
            &l,
            &[r.clone(), unsupported.clone()],
            None,
            Point { x: 190., y: 110. },
            frame(),
            page()
        )
        .is_err()
    );
    assert_eq!(
        hit_test(
            &l,
            &[r.clone(), unsupported],
            Some(&s),
            Point { x: 27., y: 107. },
            frame(),
            page()
        )
        .unwrap(),
        Some(Hit::Handle(Handle::Sw))
    );
    let mut wrong = r.clone();
    wrong.object_id = Some((111, 0));
    assert!(
        hit_test(
            &list(s.clone()),
            &[wrong],
            None,
            Point { x: 50., y: 70. },
            frame(),
            page()
        )
        .is_err()
    );
    for (p, yes) in [
        (Point { x: 16., y: 70. }, true),
        (Point { x: 15.99, y: 70. }, false),
    ] {
        assert_eq!(
            hit_test(
                &list(s.clone()),
                std::slice::from_ref(&r),
                None,
                p,
                frame(),
                page()
            )
            .unwrap()
            .is_some(),
            yes
        );
    }
    assert_eq!(
        hit_test(
            &list(s.clone()),
            std::slice::from_ref(&r),
            Some(&s),
            Point { x: 27., y: 107. },
            frame(),
            page()
        )
        .unwrap(),
        Some(Hit::Handle(Handle::Sw))
    );
    assert!(!matches!(
        hit_test(
            &list(s.clone()),
            &[r],
            Some(&s),
            Point {
                x: 27.01,
                y: 107.01
            },
            frame(),
            page()
        )
        .unwrap(),
        Some(Hit::Handle(_))
    ));
}

#[test]
fn missing_generic_occurrence_is_uncertain_not_blank_space() {
    let list = List {
        annotations: vec![],
        unsupported_count: 1,
        annotation_count: 1,
        revision: [3; 32],
    };
    assert!(hit_test(&list, &[], None, Point { x: 50., y: 70. }, frame(), page()).is_err());
}

fn assert_gap_refuses_body_and_blank(list: List, rows: Vec<pdfmaster_ui::annotations::Row>) {
    let refused: Vec<_> = [Point { x: 50., y: 70. }, Point { x: 190., y: 110. }]
        .into_iter()
        .map(|point| hit_test(&list, &rows, None, point, frame(), page()).is_err())
        .collect();
    assert_eq!(refused, vec![true, true]);
}

#[test]
fn missing_unsupported_tail_refuses_supported_body_and_blank() {
    let s = selection(&area(), 0);
    let mut l = list(s.clone());
    l.annotation_count = 2;
    l.unsupported_count = 1;
    assert_gap_refuses_body_and_blank(l, vec![row(&s)]);
}

#[test]
fn missing_interior_occurrence_refuses_supported_body_and_blank() {
    let s = selection(&area(), 0);
    let last = selection(&area(), 2);
    let l = List {
        annotations: vec![s.clone(), last.clone()],
        annotation_count: 3,
        unsupported_count: 1,
        revision: s.revision,
    };
    assert_gap_refuses_body_and_blank(l, vec![row(&s), row(&last)]);
}

#[test]
fn other_page_occurrence_cannot_fill_current_page_gap() {
    let s = selection(&area(), 0);
    let mut l = list(s.clone());
    l.annotation_count = 2;
    l.unsupported_count = 1;
    let mut other = row(&s);
    other.page_index = 1;
    other.annotation_index = 1;
    assert_gap_refuses_body_and_blank(l, vec![row(&s), other]);
}

#[test]
fn selected_corner_precedes_occurrence_gap_but_complete_pages_still_admit() {
    let s = selection(&area(), 0);
    let mut l = list(s.clone());
    l.annotation_count = 2;
    l.unsupported_count = 1;
    assert_eq!(
        hit_test(
            &l,
            &[],
            Some(&s),
            Point { x: 27., y: 107. },
            frame(),
            page()
        )
        .unwrap(),
        Some(Hit::Handle(Handle::Sw))
    );
    let mut link = row(&s);
    link.annotation_index = 1;
    link.subtype = "Link".into();
    link.bounds = None;
    assert_eq!(
        hit_test(
            &l,
            &[row(&s), link],
            None,
            Point { x: 50., y: 70. },
            frame(),
            page()
        )
        .unwrap(),
        Some(Hit::Existing(s.clone()))
    );
    let last = selection(&area(), 1);
    l.annotations.push(last.clone());
    l.unsupported_count = 0;
    assert_eq!(
        hit_test(
            &l,
            &[row(&s), row(&last)],
            None,
            Point { x: 50., y: 70. },
            frame(),
            page()
        )
        .unwrap(),
        Some(Hit::Existing(last))
    );
    l.annotations.clear();
    l.annotation_count = 0;
    assert_eq!(
        hit_test(&l, &[], None, Point { x: 50., y: 70. }, frame(), page()).unwrap(),
        None
    );
}
#[test]
fn list_integrity_rejects_revision_count_duplicate_and_bad_geometry() {
    let s = selection(&area(), 0);
    let mut bad = list(s.clone());
    bad.revision = [5; 32];
    assert!(reconcile(&bad, 0, &area(), page()).is_err());
    let mut bad = list(s.clone());
    bad.annotation_count = 1025;
    assert!(reconcile(&bad, 0, &area(), page()).is_err());
    let mut bad = list(s.clone());
    bad.annotations.push(s.clone());
    assert!(reconcile(&bad, 0, &area(), page()).is_err());
    let mut bad = list(s.clone());
    bad.annotations[0].page_index = 1;
    assert!(reconcile(&bad, 0, &area(), page()).is_err());
    let mut bad = list(s.clone());
    bad.annotations[0].original.line_width = 0.;
    assert!(reconcile(&bad, 0, &area(), page()).is_err());
    let mut bad = list(s);
    bad.unsupported_count = 2;
    assert!(reconcile(&bad, 0, &area(), page()).is_err());
}
#[test]
fn reconcile_exact_occurrence_style_geometry_and_f32_storage() {
    let d = area();
    let s = selection(&d, 0);
    assert_eq!(reconcile(&list(s.clone()), 0, &d, page()).unwrap(), s);
    assert!(reconcile(&list(s.clone()), 1, &d, page()).is_err());
    for mut changed in [d.clone(), d.clone(), d.clone(), d.clone()] {
        changed.opacity = 0.5;
        assert!(reconcile(&list(s.clone()), 0, &changed, page()).is_err());
    }
    let mut changed = d.clone();
    changed.stroke.r = 0.;
    assert!(reconcile(&list(s.clone()), 0, &changed, page()).is_err());
    changed = d.clone();
    changed.fill = None;
    assert!(reconcile(&list(s.clone()), 0, &changed, page()).is_err());
    changed = d.clone();
    changed.line_width = 3.;
    assert!(reconcile(&list(s.clone()), 0, &changed, page()).is_err());
    changed = d.clone();
    changed.geometry = Geometry::Ellipse { bounds: rect() };
    assert!(reconcile(&list(s.clone()), 0, &changed, page()).is_err());
    let mut fractional = d.clone();
    let b = Rect {
        x: 0.1,
        y: 0.2,
        width: 13.3,
        height: 21.1,
    };
    fractional.geometry = Geometry::Rectangle { bounds: b };
    let mut stored = selection(&fractional, 0);
    stored.original.bounds.width = (b.x + b.width) - b.x;
    stored.original.bounds.height = (b.y + b.height) - b.y;
    assert!(reconcile(&list(stored.clone()), 0, &fractional, page()).is_ok());
    stored.original.bounds.width += 0.00001;
    assert!(reconcile(&list(stored), 0, &fractional, page()).is_err());
    let line = new_draft(
        Kind::Arrow,
        Point { x: 20., y: 20. },
        Point { x: 100., y: 60. },
        Style::default(),
        page(),
    )
    .unwrap();
    let s = selection(&line, 0);
    for field in 0..4 {
        let mut wrong = line.clone();
        if let Geometry::Line {
            start,
            endings,
            dash_pattern,
            bounds,
            ..
        } = &mut wrong.geometry
        {
            match field {
                0 => start.x += 1.,
                1 => endings[0] = LineEnding::Slash,
                2 => dash_pattern.push(2.),
                _ => bounds.as_mut().unwrap().height += 1.,
            }
        }
        assert!(reconcile(&list(s.clone()), 0, &wrong, page()).is_err());
    }
}
#[test]
fn analytic_area_paths_are_inset_and_ellipse_has_four_cubics() {
    use pdfmaster_ui::canvas_shape_paths::paths;
    let d = area();
    let p = paths(&d, page()).unwrap();
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].d, "M 21 21 L 99 21 L 99 79 L 21 79 Z");
    assert!(p[0].fill);
    assert!(!p[0].round_join);
    let e = Draft {
        geometry: Geometry::Ellipse { bounds: rect() },
        ..d
    };
    let p = paths(&e, page()).unwrap();
    assert!(p[0].d.starts_with("M 99 50 C 99 "));
    assert_eq!(p[0].d.matches(" C ").count(), 4);
    assert!(p[0].d.ends_with(" Z"));
}
#[test]
fn all_native_endings_analytic_paint_roles_and_independent_alpha() {
    use pdfmaster_ui::canvas_shape_paths::paths;
    for (ending, name, closed) in [
        (LineEnding::None, "none", false),
        (LineEnding::Square, "square", true),
        (LineEnding::Circle, "circle", true),
        (LineEnding::Diamond, "diamond", true),
        (LineEnding::OpenArrow, "open_arrow", false),
        (LineEnding::ClosedArrow, "closed_arrow", true),
        (LineEnding::Butt, "butt", false),
        (LineEnding::ROpenArrow, "r_open_arrow", false),
        (LineEnding::RClosedArrow, "r_closed_arrow", true),
        (LineEnding::Slash, "slash", false),
    ] {
        assert_eq!(serde_json::to_value(ending).unwrap(), name);
        let mut d = new_draft(
            Kind::Line,
            Point { x: 40., y: 60. },
            Point { x: 140., y: 60. },
            Style {
                fill: Some(FILL_SWATCH),
                ..Style::default()
            },
            page(),
        )
        .unwrap();
        if let Geometry::Line {
            endings,
            dash_pattern,
            ..
        } = &mut d.geometry
        {
            *endings = [ending; 2];
            *dash_pattern = vec![3., 2.];
        }
        let p = paths(&d, page()).unwrap();
        assert_eq!(p.len(), if ending == LineEnding::None { 1 } else { 3 });
        assert_eq!(p[0].d, "M 40 60 L 140 60");
        assert_eq!(p[0].dash_pattern, vec![3., 2.]);
        assert!(!p[0].fill);
        for ending in &p[1..] {
            assert_eq!(ending.fill, closed);
            assert!(ending.round_join);
            assert!(ending.dash_pattern.is_empty());
        }
        if ending == LineEnding::OpenArrow {
            assert_eq!(p[1].d, "M 48 56 L 40 60 L 48 64");
            assert_eq!(p[2].d, "M 132 64 L 140 60 L 132 56");
        }
        if ending == LineEnding::Square {
            assert_eq!(p[2].d, "M 136 56 L 144 56 L 144 64 L 136 64 Z");
        }
        if ending == LineEnding::Diamond {
            assert_eq!(p[2].d, "M 134 60 L 140 64 L 146 60 L 140 56 Z");
        }
        if ending == LineEnding::Butt {
            assert_eq!(p[2].d, "M 140 64 L 140 56");
        }
        if ending == LineEnding::ROpenArrow {
            assert_eq!(p[2].d, "M 148 64 L 140 60 L 148 56");
        }
        if ending == LineEnding::Circle {
            assert_eq!(p[2].d.matches(" C ").count(), 4);
            assert!(p[2].d.starts_with("M 144 60"));
        }
        for alpha in [0., 0.5, 1.] {
            d.opacity = alpha;
            assert_eq!(paths(&d, page()).unwrap(), p);
        }
        d.fill = None;
        assert!(paths(&d, page()).unwrap().iter().all(|p| !p.fill));
    }
}
#[test]
fn exact_preview_controls_reverse_shaft_and_all_arrow_closures() {
    use pdfmaster_ui::canvas_shape_paths::paths;
    let d = Draft {
        geometry: Geometry::Ellipse {
            bounds: Rect {
                x: 10.,
                y: 10.,
                width: 22.,
                height: 22.,
            },
        },
        ..area()
    };
    let k = 10. * 0.552_284_8;
    let e = paths(&d, page()).unwrap();
    assert_eq!(
        e[0].d,
        format!(
            "M 31 21 C 31 {} {} 31 21 31 C {} 31 11 {} 11 21 C 11 {} {} 11 21 11 C {} 11 31 {} 31 21 Z",
            21. + k,
            21. + k,
            21. - k,
            21. + k,
            21. - k,
            21. - k,
            21. + k,
            21. - k
        )
    );
    for ending in [
        LineEnding::ClosedArrow,
        LineEnding::RClosedArrow,
        LineEnding::Slash,
    ] {
        let d = Draft {
            geometry: Geometry::Line {
                start: Point { x: 140., y: 60. },
                end: Point { x: 40., y: 60. },
                endings: [LineEnding::None, ending],
                dash_pattern: vec![0., 1000.],
                bounds: Some(Rect {
                    x: 38.,
                    y: 58.,
                    width: 104.,
                    height: 4.,
                }),
            },
            ..area()
        };
        let p = paths(&d, page()).unwrap();
        assert_eq!(p[0].d, "M 140 60 L 40 60");
        let expected = match ending {
            LineEnding::ClosedArrow => "M 48 56 L 40 60 L 48 64 Z".to_string(),
            LineEnding::RClosedArrow => "M 32 56 L 40 60 L 32 64 Z".to_string(),
            _ => format!(
                "M 38 {} L 42 {}",
                60_f32 - 1.732_050_8 * 2.,
                60_f32 + 1.732_050_8 * 2.
            ),
        };
        assert_eq!(p[1].d, expected);
    }
}
#[test]
fn client_tolerances_are_css_pixels_for_every_rotation_and_nonsquare_frame() {
    let d = area();
    let s = selection(&d, 0);
    let r = row(&s);
    let l = list(s.clone());
    let f = Frame {
        left: 15.,
        top: 30.,
        width: 400.,
        height: 360.,
    };
    for rotation in [0, 90, 180, 270] {
        let p = Page { rotation, ..page() };
        let c = display_corners(rect(), f, p).unwrap();
        let q = c[0].1;
        assert_eq!(
            hit_test(
                &l,
                std::slice::from_ref(&r),
                Some(&s),
                Point {
                    x: q.x + 7.,
                    y: q.y + 7.
                },
                f,
                p
            )
            .unwrap(),
            Some(Hit::Handle(Handle::Sw))
        );
        assert!(!matches!(
            hit_test(
                &l,
                std::slice::from_ref(&r),
                Some(&s),
                Point {
                    x: q.x + 7.01,
                    y: q.y + 7.01
                },
                f,
                p
            )
            .unwrap(),
            Some(Hit::Handle(_))
        ));
        let min_x = c.iter().map(|(_, p)| p.x).fold(f32::INFINITY, f32::min);
        let min_y = c.iter().map(|(_, p)| p.y).fold(f32::INFINITY, f32::min);
        let max_y = c.iter().map(|(_, p)| p.y).fold(f32::NEG_INFINITY, f32::max);
        for (offset, yes) in [(4., true), (4.01, false)] {
            assert_eq!(
                hit_test(
                    &l,
                    std::slice::from_ref(&r),
                    None,
                    Point {
                        x: min_x - offset,
                        y: (min_y + max_y) / 2.
                    },
                    f,
                    p
                )
                .unwrap()
                .is_some(),
                yes
            );
        }
    }
}
#[test]
fn missing_target_identity_and_null_direct_object_identity() {
    let s = selection(&area(), 0);
    let mut v = serde_json::to_value(&s).unwrap();
    v["object_id"] = serde_json::Value::Null;
    assert_eq!(
        serde_json::from_value::<Selection>(v).unwrap().object_id,
        None
    );
    for target in [
        Target::Existing { original: s },
        Target::New {
            draft: area(),
            revision: [3; 32],
        },
    ] {
        let value = serde_json::to_value(target).unwrap();
        for key in ["original", "draft", "revision"] {
            if value.get(key).is_some() {
                let mut bad = value.clone();
                bad.as_object_mut().unwrap().remove(key);
                assert!(serde_json::from_value::<Target>(bad).is_err());
            }
        }
    }
}
