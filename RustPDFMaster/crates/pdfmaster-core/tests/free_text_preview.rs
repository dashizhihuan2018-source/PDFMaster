use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream};
use pdfmaster_core::text_appearance::BundledFont;
use pdfmaster_core::{
    Color, DocumentInfo, Edit, FreeTextAnnotation, FreeTextAppearanceStyle, FreeTextDraft, Rect,
    Session, TextAlignment,
};

fn style(color: Color) -> FreeTextAppearanceStyle {
    FreeTextAppearanceStyle {
        font: BundledFont::Sans,
        font_size: 18.0,
        color,
        alignment: TextAlignment::Left,
        line_height: 1.2,
    }
}

fn colored_page() -> Session {
    let blank = Session::blank(180.0, 140.0, 1).unwrap();
    let mut document = Document::load_mem(&blank.bytes().unwrap()).unwrap();
    let page_id = *document.get_pages().values().next().unwrap();
    let content = Content {
        operations: vec![
            Operation::new("rg", vec![0.18.into(), 0.42.into(), 0.76.into()]),
            Operation::new("re", vec![0.into(), 0.into(), 180.into(), 140.into()]),
            Operation::new("f", Vec::new()),
        ],
    }
    .encode()
    .unwrap();
    let content_id = document.add_object(Stream::new(lopdf::Dictionary::new(), content));
    document
        .get_object_mut(page_id)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Contents", Object::Reference(content_id));
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    Session::open(&bytes).unwrap()
}

fn draft<'a>(
    contents: &'a str,
    bounds: Rect,
    style: &'a FreeTextAppearanceStyle,
) -> FreeTextDraft<'a> {
    FreeTextDraft {
        contents,
        bounds,
        style: style.clone(),
    }
}

#[derive(Debug, PartialEq)]
struct HistoryTrace {
    current: Vec<u8>,
    info: DocumentInfo,
    past: Vec<Vec<u8>>,
    future: Vec<Vec<u8>>,
}

fn history_trace(session: &mut Session) -> HistoryTrace {
    let current = session.bytes().unwrap();
    let info = session.info();
    let mut past = Vec::new();
    let mut future = Vec::new();
    while session.undo().is_ok() {
        past.push(session.bytes().unwrap());
    }
    for _ in &past {
        session.redo().unwrap();
    }
    while session.redo().is_ok() {
        future.push(session.bytes().unwrap());
    }
    for _ in &future {
        session.undo().unwrap();
    }
    assert_eq!(session.bytes().unwrap(), current);
    assert_eq!(session.info(), info);
    HistoryTrace {
        current,
        info,
        past,
        future,
    }
}

fn seed_two_sided_history(session: &mut Session) {
    session
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    session
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    session.undo().unwrap();
    assert!(session.info().can_undo && session.info().can_redo);
}

fn assert_rejected_without_mutation(session: &mut Session, wrong: &FreeTextAnnotation) {
    let before = history_trace(session);
    assert!(session.render_free_text_background_png(wrong, 1.0).is_err());
    assert_eq!(history_trace(session), before);
}

fn session_with_two_free_texts() -> Session {
    let mut session = colored_page();
    let dark = style(Color {
        r: 0.05,
        g: 0.05,
        b: 0.05,
    });
    for (contents, bounds) in [
        (
            "kept",
            Rect {
                x: 12.0,
                y: 96.0,
                width: 64.0,
                height: 28.0,
            },
        ),
        (
            "selected",
            Rect {
                x: 80.0,
                y: 20.0,
                width: 84.0,
                height: 32.0,
            },
        ),
    ] {
        session
            .create_free_text(0, &draft(contents, bounds, &dark))
            .unwrap();
    }
    session
}

fn reopen_with_page_mutation(
    session: &Session,
    mutate: impl FnOnce(&mut lopdf::Dictionary),
) -> Session {
    let mut document = Document::load_mem(&session.bytes().unwrap()).unwrap();
    let page_id = *document.get_pages().values().next().unwrap();
    mutate(
        document
            .get_object_mut(page_id)
            .unwrap()
            .as_dict_mut()
            .unwrap(),
    );
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    Session::open(&bytes).unwrap()
}

fn reopen_with_direct_annots(session: &Session) -> Session {
    let mut document = Document::load_mem(&session.bytes().unwrap()).unwrap();
    let page_id = *document.get_pages().values().next().unwrap();
    let annots = document
        .get_object(page_id)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Annots")
        .unwrap()
        .clone();
    let annots = match annots {
        Object::Reference(id) => document.get_object(id).unwrap().as_array().unwrap().clone(),
        Object::Array(values) => values,
        _ => panic!("Annots must be an array"),
    };
    document
        .get_object_mut(page_id)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", Object::Array(annots));
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    Session::open(&bytes).unwrap()
}

#[test]
fn selected_text_is_omitted_from_immutable_background() {
    let mut session = colored_page();
    let dark = style(Color {
        r: 0.05,
        g: 0.05,
        b: 0.05,
    });
    session
        .create_free_text(
            0,
            &draft(
                "kept",
                Rect {
                    x: 12.0,
                    y: 96.0,
                    width: 64.0,
                    height: 28.0,
                },
                &dark,
            ),
        )
        .unwrap();
    let baseline = session.render_png(0, 1.0).unwrap();
    session
        .create_free_text(
            0,
            &draft(
                "selected",
                Rect {
                    x: 80.0,
                    y: 20.0,
                    width: 84.0,
                    height: 32.0,
                },
                &dark,
            ),
        )
        .unwrap();
    let selected = session.free_texts(0).unwrap().annotations.pop().unwrap();
    assert_ne!(session.render_png(0, 1.0).unwrap(), baseline);
    let before = session.bytes().unwrap();
    let info = session.info();

    assert_eq!(
        session
            .render_free_text_background_png(&selected, 1.0)
            .unwrap(),
        baseline
    );
    assert_eq!(session.bytes().unwrap(), before);
    assert_eq!(session.info(), info);
}

#[test]
fn selected_projection_must_match_every_field_and_revision_without_mutation() {
    let mut session = session_with_two_free_texts();
    seed_two_sided_history(&mut session);
    let selected = session.free_texts(0).unwrap().annotations.pop().unwrap();

    let mut wrong = selected.clone();
    wrong.annotation_index = 0;
    assert_rejected_without_mutation(&mut session, &wrong);

    let mut wrong = selected.clone();
    wrong.bounds.x += 1.0;
    assert_rejected_without_mutation(&mut session, &wrong);

    let mut wrong = selected.clone();
    wrong.contents.push('!');
    assert_rejected_without_mutation(&mut session, &wrong);

    let mut wrong = selected.clone();
    wrong.revision[0] ^= 1;
    assert_rejected_without_mutation(&mut session, &wrong);

    let mut wrong = selected.clone();
    wrong.object_id = Some((u32::MAX, 0));
    assert_rejected_without_mutation(&mut session, &wrong);

    let mut wrong = selected.clone();
    wrong.page_id = (u32::MAX, 0);
    assert_rejected_without_mutation(&mut session, &wrong);
}

#[test]
fn stale_projection_is_rejected_after_legitimate_session_change() {
    let mut session = session_with_two_free_texts();
    let stale = session.free_texts(0).unwrap().annotations.pop().unwrap();
    let dark = style(Color {
        r: 0.05,
        g: 0.05,
        b: 0.05,
    });
    session
        .create_free_text(
            0,
            &draft(
                "later",
                Rect {
                    x: 12.0,
                    y: 54.0,
                    width: 64.0,
                    height: 28.0,
                },
                &dark,
            ),
        )
        .unwrap();
    let current = session.free_texts(0).unwrap().annotations[1].clone();
    let before = history_trace(&mut session);
    assert!(
        session
            .render_free_text_background_png(&current, 1.0)
            .is_ok()
    );
    assert_eq!(history_trace(&mut session), before);
    assert_rejected_without_mutation(&mut session, &stale);
}

#[test]
fn selected_unsupported_appearance_is_rejected_before_temporary_omission() {
    let session = session_with_two_free_texts();
    let selected = session.free_texts(0).unwrap().annotations.pop().unwrap();
    let mut document = Document::load_mem(&session.bytes().unwrap()).unwrap();
    document
        .get_object_mut(selected.object_id.unwrap())
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("AP", 1i64);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    let session = Session::open(&bytes).unwrap();
    let selected = session.free_texts(0).unwrap().annotations[1].clone();
    let normal_error = session.render_png(0, 1.0).unwrap_err().to_string();
    assert!(normal_error.contains("annotation appearance"));
    let before = session.bytes().unwrap();
    let info = session.info();

    let preview_error = session
        .render_free_text_background_png(&selected, 1.0)
        .unwrap_err()
        .to_string();
    assert!(preview_error.contains("annotation appearance"));
    assert_eq!(session.bytes().unwrap(), before);
    assert_eq!(session.info(), info);
}

#[test]
fn selected_unresolved_appearance_state_is_rejected_before_temporary_omission() {
    let session = session_with_two_free_texts();
    let selected = session.free_texts(0).unwrap().annotations.pop().unwrap();
    let mut document = Document::load_mem(&session.bytes().unwrap()).unwrap();
    let annotation = document
        .get_object_mut(selected.object_id.unwrap())
        .unwrap()
        .as_dict_mut()
        .unwrap();
    let normal = annotation
        .get(b"AP")
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"N")
        .unwrap()
        .clone();
    annotation.set(
        "AP",
        lopdf::dictionary! {"N" => lopdf::dictionary! {"On" => normal}},
    );
    annotation.set("AS", Object::Name(b"Missing".to_vec()));
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    let session = Session::open(&bytes).unwrap();
    let selected = session.free_texts(0).unwrap().annotations[1].clone();
    let normal_error = session.render_png(0, 1.0).unwrap_err().to_string();
    assert!(normal_error.contains("unresolved-annotation-appearance"));
    let before = session.bytes().unwrap();
    let info = session.info();

    let preview_error = session
        .render_free_text_background_png(&selected, 1.0)
        .unwrap_err()
        .to_string();
    assert!(preview_error.contains("unresolved-annotation-appearance"));
    assert_eq!(session.bytes().unwrap(), before);
    assert_eq!(session.info(), info);
}

#[test]
fn exact_background_matches_for_all_rotations_and_direct_or_indirect_annots() {
    for rotation in [0, 90, 180, 270] {
        for direct in [false, true] {
            let mut session = colored_page();
            if rotation != 0 {
                session = reopen_with_page_mutation(&session, |page| page.set("Rotate", rotation));
            }
            let dark = style(Color {
                r: 0.05,
                g: 0.05,
                b: 0.05,
            });
            session
                .create_free_text(
                    0,
                    &draft(
                        "kept",
                        Rect {
                            x: 12.0,
                            y: 96.0,
                            width: 64.0,
                            height: 28.0,
                        },
                        &dark,
                    ),
                )
                .unwrap();
            if direct {
                session = reopen_with_direct_annots(&session);
            }
            let baseline = session.render_png(0, 1.0).unwrap();
            session
                .create_free_text(
                    0,
                    &draft(
                        "selected",
                        Rect {
                            x: 80.0,
                            y: 20.0,
                            width: 84.0,
                            height: 32.0,
                        },
                        &dark,
                    ),
                )
                .unwrap();
            if direct {
                session = reopen_with_direct_annots(&session);
            }
            let selected = session.free_texts(0).unwrap().annotations.pop().unwrap();
            assert_ne!(session.render_png(0, 1.0).unwrap(), baseline);
            assert_eq!(
                session
                    .render_free_text_background_png(&selected, 1.0)
                    .unwrap(),
                baseline,
                "rotation={rotation}, direct={direct}"
            );
        }
    }
}

#[test]
fn successful_preview_preserves_complete_two_sided_history() {
    let mut session = session_with_two_free_texts();
    seed_two_sided_history(&mut session);
    let selected = session.free_texts(0).unwrap().annotations.pop().unwrap();
    let before = history_trace(&mut session);
    assert!(
        !session
            .render_free_text_background_png(&selected, 1.0)
            .unwrap()
            .is_empty()
    );
    assert_eq!(history_trace(&mut session), before);
}

#[test]
fn other_unsupported_appearance_and_page_content_keep_strict_render_refusals() {
    let session = session_with_two_free_texts();
    let rows = session.free_texts(0).unwrap().annotations;
    let mut document = Document::load_mem(&session.bytes().unwrap()).unwrap();
    document
        .get_object_mut(rows[0].object_id.unwrap())
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("AP", 1i64);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    let session = Session::open(&bytes).unwrap();
    let selected = session.free_texts(0).unwrap().annotations[1].clone();
    let error = session
        .render_free_text_background_png(&selected, 1.0)
        .unwrap_err()
        .to_string();
    assert!(error.contains("annotation appearance"));

    let session = session_with_two_free_texts();
    let selected_index = 1usize;
    let mut document = Document::load_mem(&session.bytes().unwrap()).unwrap();
    let page_id = *document.get_pages().values().next().unwrap();
    let bad_content = Content {
        operations: vec![Operation::new("gs", vec![Object::Name(b"bad".to_vec())])],
    }
    .encode()
    .unwrap();
    let content_id = document.add_object(Stream::new(lopdf::Dictionary::new(), bad_content));
    document
        .get_object_mut(page_id)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Contents", content_id);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    let session = Session::open(&bytes).unwrap();
    let selected = session.free_texts(0).unwrap().annotations[selected_index].clone();
    let error = session
        .render_free_text_background_png(&selected, 1.0)
        .unwrap_err()
        .to_string();
    assert!(error.contains("content preflight"));
}

#[test]
fn invalid_scales_and_pixel_budget_fail_without_mutating_source() {
    let session = session_with_two_free_texts();
    let selected = session.free_texts(0).unwrap().annotations[1].clone();
    let before = session.bytes().unwrap();
    let info = session.info();
    for scale in [0.0, f32::NAN, 17.0] {
        assert!(
            session
                .render_free_text_background_png(&selected, scale)
                .is_err()
        );
        assert_eq!(session.bytes().unwrap(), before);
        assert_eq!(session.info(), info);
    }

    let mut large = Session::blank(1000.0, 1000.0, 1).unwrap();
    let dark = style(Color {
        r: 0.05,
        g: 0.05,
        b: 0.05,
    });
    large
        .create_free_text(
            0,
            &draft(
                "selected",
                Rect {
                    x: 20.0,
                    y: 20.0,
                    width: 120.0,
                    height: 40.0,
                },
                &dark,
            ),
        )
        .unwrap();
    let selected = large.free_texts(0).unwrap().annotations[0].clone();
    let before = large.bytes().unwrap();
    let info = large.info();
    assert!(
        large
            .render_free_text_background_png(&selected, 8.0)
            .is_err()
    );
    assert_eq!(large.bytes().unwrap(), before);
    assert_eq!(large.info(), info);
}
