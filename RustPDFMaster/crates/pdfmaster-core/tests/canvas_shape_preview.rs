use lopdf::{Document, Object, Stream, dictionary};
use pdfmaster_core::{
    CanvasShapeDraft, CanvasShapeGeometry, Color, DocumentInfo, Edit, Rect, Session,
};

fn reopen(d: &mut Document) -> Session {
    let mut bytes = Vec::new();
    d.save_to(&mut bytes).unwrap();
    Session::open(&bytes).unwrap()
}

fn annots(d: &Document, page: (u32, u16)) -> Vec<Object> {
    d.dereference(
        d.get_object(page)
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

fn colored_shapes(rotation: i64, direct: bool) -> Session {
    let mut s = Session::blank(200., 120., 2).unwrap();
    let d = CanvasShapeDraft {
        geometry: CanvasShapeGeometry::Rectangle {
            bounds: Rect {
                x: 30.,
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
        line_width: 4.,
        opacity: 1.,
    };
    s.create_canvas_shape(0, &d).unwrap();
    let mut pdf = Document::load_mem(&s.bytes().unwrap()).unwrap();
    let p = pdf.get_pages();
    let root = annots(&pdf, p[&1])[0].clone();
    let mut overlap = pdf.dereference(&root).unwrap().1.as_dict().unwrap().clone();
    // A different transparent overlapping shape remains in the background.
    overlap.set("CA", 0.4_f32);
    let overlap = pdf.add_object(overlap);
    let root = if direct {
        pdf.dereference(&root).unwrap().1.clone()
    } else {
        root
    };
    let shared = pdf.add_object(vec![root.clone(), root, overlap.into()]);
    let content = pdf.add_object(Stream::new(
        dictionary! {},
        b"0.18 0.42 0.76 rg 0 0 200 120 re f".to_vec(),
    ));
    for page in p.values() {
        let page = pdf.get_object_mut(*page).unwrap().as_dict_mut().unwrap();
        page.set("Annots", shared);
        page.set("Contents", content);
        page.set("Rotate", rotation);
    }
    reopen(&mut pdf)
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
    s.apply(Edit::Rotate {
        page: 1,
        degrees: 90,
    })
    .unwrap();
    s.apply(Edit::Rotate {
        page: 1,
        degrees: 90,
    })
    .unwrap();
    s.undo().unwrap();
    assert!(s.info().can_undo && s.info().can_redo);
}

fn pixels(png: &[u8]) -> image::RgbaImage {
    image::load_from_memory(png).unwrap().to_rgba8()
}

#[test]
fn omission_matches_independent_pdf_pixels_for_rotations_and_shared_entries() {
    for rotation in [0, 90, 180, 270] {
        for direct in [false, true] {
            let mut s = colored_shapes(rotation, direct);
            seed_history(&mut s);
            let selected = s.canvas_shapes(0).unwrap().annotations.remove(1);
            let before = history(&mut s);
            let mut expected_pdf = Document::load_mem(&before.bytes).unwrap();
            let pages = expected_pdf.get_pages();
            let keep = annots(&expected_pdf, pages[&1])
                .into_iter()
                .enumerate()
                .filter_map(|(i, a)| (i != 1).then_some(a))
                .collect::<Vec<_>>();
            expected_pdf
                .get_object_mut(pages[&1])
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("Annots", keep);
            let expected = reopen(&mut expected_pdf);
            for scale in [0.75, 1., 2.] {
                let rendered = s
                    .render_canvas_shape_background_png(&selected, scale)
                    .unwrap();
                assert_eq!(
                    pixels(&rendered),
                    pixels(&expected.render_png(0, scale).unwrap())
                );
                assert_eq!(
                    pixels(&s.render_png(1, scale).unwrap()),
                    pixels(
                        &Session::open(&before.bytes)
                            .unwrap()
                            .render_png(1, scale)
                            .unwrap()
                    )
                );
                assert_eq!(history(&mut s), before);
            }
        }
    }
}

#[test]
fn transparent_shape_removal_restores_colored_content_not_white_mask() {
    let mut s = Session::blank(200., 120., 1).unwrap();
    let d = CanvasShapeDraft {
        geometry: CanvasShapeGeometry::Ellipse {
            bounds: Rect {
                x: 30.,
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
        line_width: 4.,
        opacity: 1.,
    };
    s.create_canvas_shape(0, &d).unwrap();
    let mut pdf = Document::load_mem(&s.bytes().unwrap()).unwrap();
    let page = pdf.get_pages()[&1];
    let content = pdf.add_object(Stream::new(
        dictionary! {},
        b"0.18 0.42 0.76 rg 0 0 200 120 re f".to_vec(),
    ));
    pdf.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Contents", content);
    let s = reopen(&mut pdf);
    let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
    let full = pixels(&s.render_png(0, 1.).unwrap());
    pdf.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", Vec::<Object>::new());
    let expected = reopen(&mut pdf);
    let actual = pixels(&s.render_canvas_shape_background_png(&selected, 1.).unwrap());
    assert_eq!(actual, pixels(&expected.render_png(0, 1.).unwrap()));
    assert_ne!(actual, full);
    assert_ne!(actual.get_pixel(80, 60).0, [255, 255, 255, 255]);
}

fn one_shape() -> Session {
    let mut s = Session::blank(200., 120., 2).unwrap();
    s.create_canvas_shape(
        0,
        &CanvasShapeDraft {
            geometry: CanvasShapeGeometry::Rectangle {
                bounds: Rect {
                    x: 30.,
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
            line_width: 4.,
            opacity: 1.,
        },
    )
    .unwrap();
    s
}

#[test]
fn malformed_original_appearance_cannot_be_hidden_by_selected_omission() {
    let mut incorrectly_rendered = Vec::new();
    for kind in 0..4 {
        let s = one_shape();
        let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
        let mut d = Document::load_mem(&s.bytes().unwrap()).unwrap();
        let a = d
            .get_object_mut(selected.object_id.unwrap())
            .unwrap()
            .as_dict_mut()
            .unwrap();
        match kind {
            0 => a.set("AP", 12),
            1 => a.set("AP", dictionary! {"N"=>12}),
            2 | 3 => {
                let normal = a
                    .get(b"AP")
                    .unwrap()
                    .as_dict()
                    .unwrap()
                    .get(b"N")
                    .unwrap()
                    .clone();
                a.set("AP", dictionary! {"N"=>dictionary!{"On"=>normal}});
                if kind == 2 {
                    a.set("AS", Object::Name(b"Missing".to_vec()));
                }
                // kind 3 has On only, no Off and no AS.
            }
            _ => unreachable!(),
        }
        let mut s = reopen(&mut d);
        seed_history(&mut s);
        let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
        let normal_error = s.render_png(0, 1.25).unwrap_err().to_string();
        assert!(normal_error.contains("annotation"), "{normal_error}");
        let before = history(&mut s);
        match s.render_canvas_shape_background_png(&selected, 1.25) {
            Ok(_) => incorrectly_rendered.push(kind),
            Err(error) => assert!(error.to_string().contains("annotation"), "{error}"),
        }
        assert_eq!(history(&mut s), before);
    }
    assert!(
        incorrectly_rendered.is_empty(),
        "invalid original AP cases incorrectly rendered: {incorrectly_rendered:?}"
    );
}

#[test]
fn invalid_scale_stale_selection_and_forbidden_flags_preserve_full_history() {
    let mut s = one_shape();
    seed_history(&mut s);
    let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
    let before = history(&mut s);
    for scale in [0., -1., f32::NAN, f32::INFINITY, 1000.] {
        assert!(
            s.render_canvas_shape_background_png(&selected, scale)
                .is_err()
        );
        assert_eq!(history(&mut s), before);
    }
    let mut wrong = selected.clone();
    wrong.flags ^= 4;
    assert!(s.render_canvas_shape_background_png(&wrong, 1.).is_err());
    let mut wrong = selected.clone();
    wrong.revision[0] ^= 1;
    assert!(s.render_canvas_shape_background_png(&wrong, 1.).is_err());
    assert_eq!(history(&mut s), before);
    s.redo().unwrap();
    assert!(s.render_canvas_shape_background_png(&selected, 1.).is_err());
    for flags in [64, 128, 512, 1, 2, 32] {
        let mut d = Document::load_mem(&before.bytes).unwrap();
        d.get_object_mut(selected.object_id.unwrap())
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("F", flags);
        let mut forbidden = reopen(&mut d);
        seed_history(&mut forbidden);
        let mut selection = selected.clone();
        selection.revision = forbidden.canvas_shapes(0).unwrap().revision;
        selection.flags = flags as u32;
        let before = history(&mut forbidden);
        assert!(
            forbidden
                .render_canvas_shape_background_png(&selection, 1.)
                .is_err()
        );
        assert_eq!(history(&mut forbidden), before);
    }
}

#[test]
fn background_preserves_same_page_popup_and_shared_other_page_pixels() {
    let s = one_shape();
    let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
    let root = selected.object_id.unwrap();
    let mut d = Document::load_mem(&s.bytes().unwrap()).unwrap();
    let p = d.get_pages();
    let popup=d.add_object(dictionary!{"Subtype"=>"Popup","Rect"=>vec![1.into(),1.into(),10.into(),10.into()],"Parent"=>root,"P"=>p[&1]});
    d.get_object_mut(root)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Popup", popup);
    let shared = d.add_object(vec![root.into(), popup.into()]);
    for page in p.values() {
        d.get_object_mut(*page)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Annots", shared);
    }
    let mut s = reopen(&mut d);
    seed_history(&mut s);
    let selected = s.canvas_shapes(0).unwrap().annotations.remove(0);
    let before = history(&mut s);
    let full_other = pixels(&s.render_png(1, 1.).unwrap());
    // Independent expected PDF keeps the popup; only root occurrence is omitted.
    d = Document::load_mem(&before.bytes).unwrap();
    d.get_object_mut(p[&1])
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", vec![Object::Reference(popup)]);
    let expected = reopen(&mut d);
    let actual = s.render_canvas_shape_background_png(&selected, 1.).unwrap();
    assert_eq!(
        pixels(&actual),
        pixels(&expected.render_png(0, 1.).unwrap())
    );
    assert_eq!(pixels(&s.render_png(1, 1.).unwrap()), full_other);
    assert_eq!(history(&mut s), before);
}
