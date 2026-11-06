use lopdf::{Document, Object, dictionary};
use pdfmaster_core::text_appearance::BundledFont;
use pdfmaster_core::{
    Color, FreeTextAppearanceStyle, FreeTextCommit, FreeTextDraft, FreeTextUpdate, Rect, Session,
    TextAlignment,
};
fn style() -> FreeTextAppearanceStyle {
    FreeTextAppearanceStyle {
        font: BundledFont::Sans,
        font_size: 18.,
        color: Color {
            r: 1. / 7.,
            g: 2. / 7.,
            b: 3. / 7.,
        },
        alignment: TextAlignment::Left,
        line_height: 1.3,
    }
}
fn bounds() -> Rect {
    Rect {
        x: 31.,
        y: 61.,
        width: 360.,
        height: 240.,
    }
}
fn created() -> Session {
    let mut session = Session::blank(595., 842., 1).unwrap();
    session
        .create_free_text(
            0,
            &FreeTextDraft {
                contents: "Latin 中文𠮷",
                bounds: bounds(),
                style: style(),
            },
        )
        .unwrap();
    session
}
fn row(session: &Session) -> pdfmaster_core::FreeTextAnnotation {
    session.free_texts(0).unwrap().annotations.remove(0)
}
fn edited_fixture(edit: impl FnOnce(&mut Document, (u32, u16))) -> Session {
    let session = created();
    let selected = row(&session);
    let mut doc = Document::load_mem(&session.bytes().unwrap()).unwrap();
    edit(&mut doc, selected.object_id.unwrap());
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    Session::open(&bytes).unwrap()
}
fn same_update<'a>(selected: &'a pdfmaster_core::FreeTextAnnotation) -> FreeTextUpdate<'a> {
    FreeTextUpdate {
        contents: &selected.contents,
        bounds: selected.bounds,
        style: None,
    }
}
#[derive(Debug, PartialEq)]
struct HistoryTrace {
    current: Vec<u8>,
    info: pdfmaster_core::DocumentInfo,
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
        .apply(pdfmaster_core::Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    session
        .apply(pdfmaster_core::Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    session.undo().unwrap();
    assert!(session.info().can_undo && session.info().can_redo);
}
#[test]
fn actual_create_embedded_style_and_reopen_without_update() {
    let session = created();
    let bytes = session.bytes().unwrap();
    let reopened = Session::open(&bytes).unwrap();
    assert_eq!(row(&reopened).contents, "Latin 中文𠮷");
    assert_eq!(
        reopened
            .free_text_appearance_style(&row(&reopened))
            .unwrap(),
        Some(style())
    );
    assert!(!reopened.render_png(0, 1.).unwrap().is_empty());
}

#[test]
fn create_reopen_update_and_exact_history() {
    let mut session = Session::blank(595., 842., 1).unwrap();
    let original = session.bytes().unwrap();
    let source = "Latin 中文 مرحبا بالعالم हैलो वर्ल्ड\r\nCafé e\u{301} office 𠮷";
    assert_eq!(
        session
            .create_free_text(
                0,
                &FreeTextDraft {
                    contents: source,
                    bounds: bounds(),
                    style: style()
                }
            )
            .unwrap(),
        FreeTextCommit::Created
    );
    let created = session.bytes().unwrap();
    let reopened = Session::open(&created).unwrap();
    let row = reopened.free_texts(0).unwrap().annotations.remove(0);
    assert_eq!(row.contents, source);
    assert_eq!(row.bounds, bounds());
    assert_eq!(
        reopened.free_text_appearance_style(&row).unwrap(),
        Some(style())
    );
    assert!(!reopened.render_png(0, 1.).unwrap().is_empty());
    assert_eq!(
        session
            .update_free_text(
                &row,
                &FreeTextUpdate {
                    contents: "中文𠮷 changed",
                    bounds: bounds(),
                    style: None
                }
            )
            .unwrap(),
        FreeTextCommit::Updated
    );
    let changed = session.bytes().unwrap();
    session.undo().unwrap();
    assert_eq!(session.bytes().unwrap(), created);
    session.undo().unwrap();
    assert_eq!(session.bytes().unwrap(), original);
    session.redo().unwrap();
    assert_eq!(session.bytes().unwrap(), created);
    session.redo().unwrap();
    assert_eq!(session.bytes().unwrap(), changed);
}

#[test]
fn empty_noop_cancel_and_validation_preserve_both_sides_of_history() {
    let mut session = created();
    let before = session.bytes().unwrap();
    let selected = row(&session);
    session
        .update_free_text(
            &selected,
            &FreeTextUpdate {
                contents: "second",
                ..same_update(&selected)
            },
        )
        .unwrap();
    let after = session.bytes().unwrap();
    session.undo().unwrap();
    let selected = row(&session);
    let info = session.info();
    let _cancelled = FreeTextUpdate {
        contents: "cancelled",
        bounds: bounds(),
        style: Some(style()),
    };
    assert_eq!(
        session
            .update_free_text(&selected, &same_update(&selected))
            .unwrap(),
        FreeTextCommit::Unchanged
    );
    assert_eq!(
        session
            .create_free_text(
                0,
                &FreeTextDraft {
                    contents: "\u{2003}\r\n",
                    bounds: bounds(),
                    style: style()
                }
            )
            .unwrap(),
        FreeTextCommit::Unchanged
    );
    assert!(
        session
            .create_free_text(
                0,
                &FreeTextDraft {
                    contents: &" ".repeat(65_537),
                    bounds: bounds(),
                    style: style()
                }
            )
            .is_err()
    );
    assert!(
        session
            .update_free_text(
                &selected,
                &FreeTextUpdate {
                    contents: &" ".repeat(65_537),
                    ..same_update(&selected)
                }
            )
            .is_err()
    );
    assert_eq!(session.bytes().unwrap(), before);
    assert_eq!(session.info(), info);
    session.undo().unwrap();
    session.redo().unwrap();
    assert_eq!(session.bytes().unwrap(), before);
    session.redo().unwrap();
    assert_eq!(session.bytes().unwrap(), after);
}

#[test]
fn update_permission_guards_collect_every_blocked_action_before_asserting() {
    let mut outcomes = Vec::new();
    for flags in [64, 128, 512, 64 | 128, 64 | 512, 128 | 512, 64 | 128 | 512] {
        for action in ["body", "move", "resize", "style", "delete"] {
            let blocked = flags & 64 != 0
                || match action {
                    "body" => flags & 512 != 0,
                    _ => flags & 128 != 0,
                };
            if !blocked {
                continue;
            }
            let mut session = edited_fixture(|doc, id| {
                doc.get_object_mut(id)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .set("F", flags);
            });
            seed_two_sided_history(&mut session);
            let history = history_trace(&mut session);
            let selected = row(&session);
            let mut update = same_update(&selected);
            match action {
                "body" => update.contents = "new 中文",
                "move" => update.bounds.x += 1.,
                "resize" => update.bounds.width -= 1.,
                "style" => {
                    let mut s = style();
                    s.line_height += 0.1;
                    update.style = Some(s);
                }
                _ => update.contents = " ",
            }
            outcomes.push((
                flags,
                action,
                session.update_free_text(&selected, &update).is_err(),
            ));
            assert_eq!(history_trace(&mut session), history);
        }
    }
    assert!(
        outcomes.iter().all(|(_, _, rejected)| *rejected),
        "{outcomes:?}"
    );
}

#[test]
fn permission_actions_are_distinct_for_all_flag_combinations() {
    for flags in [
        0,
        64,
        128,
        512,
        64 | 128,
        64 | 512,
        128 | 512,
        64 | 128 | 512,
    ] {
        for action in ["body", "move", "resize", "style", "delete", "noop"] {
            let mut session = edited_fixture(|doc, id| {
                doc.get_object_mut(id)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .set("F", flags)
            });
            seed_two_sided_history(&mut session);
            let baseline_history = history_trace(&mut session);
            let selected = row(&session);
            let before = session.bytes().unwrap();
            let before_info = session.info();
            let mut update = same_update(&selected);
            match action {
                "body" => update.contents = "new 中文",
                "move" => update.bounds.x += 1.,
                "resize" => update.bounds.width -= 1.,
                "style" => {
                    let mut style = style();
                    style.alignment = TextAlignment::Right;
                    update.style = Some(style);
                }
                "delete" => update.contents = "  \r\n",
                _ => (),
            }
            let blocked = action != "noop"
                && (flags & 64 != 0
                    || match action {
                        "body" => flags & 512 != 0,
                        _ => flags & 128 != 0,
                    });
            let result = session.update_free_text(&selected, &update);
            assert_eq!(
                result.is_err(),
                blocked,
                "flags={flags}, action={action}, result={result:?}"
            );
            if blocked || action == "noop" {
                assert_eq!(session.bytes().unwrap(), before);
                assert_eq!(session.info(), before_info);
                assert_eq!(history_trace(&mut session), baseline_history);
            }
            if !blocked {
                if action == "noop" {
                    assert_eq!(result.unwrap(), FreeTextCommit::Unchanged);
                } else if action == "delete" {
                    assert_eq!(result.unwrap(), FreeTextCommit::Deleted);
                    assert!(session.free_texts(0).unwrap().annotations.is_empty());
                } else {
                    assert_eq!(result.unwrap(), FreeTextCommit::Updated);
                    let changed = row(&session);
                    assert_eq!(changed.contents, update.contents);
                    assert_eq!(changed.bounds, update.bounds);
                    if let Some(style) = &update.style {
                        assert_eq!(
                            session
                                .free_text_appearance_style(&changed)
                                .unwrap()
                                .as_ref(),
                            Some(style)
                        );
                    }
                    assert_ne!(session.bytes().unwrap(), before);
                }
            }
        }
    }
}

#[test]
fn existing_empty_noop_does_not_delete_and_changed_empty_deletes_once() {
    let mut session = edited_fixture(|doc, id| {
        doc.get_object_mut(id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Contents", lopdf::text_string(""))
    });
    let selected = row(&session);
    let original = session.bytes().unwrap();
    assert_eq!(
        session
            .update_free_text(&selected, &same_update(&selected))
            .unwrap(),
        FreeTextCommit::Unchanged
    );
    assert_eq!(session.bytes().unwrap(), original);
    let mut moved = same_update(&selected);
    moved.bounds.x += 1.;
    assert_eq!(
        session.update_free_text(&selected, &moved).unwrap(),
        FreeTextCommit::Deleted
    );
    session.undo().unwrap();
    assert_eq!(session.bytes().unwrap(), original);
    assert!(session.undo().is_err());
}

#[test]
fn new_blank_draft_is_unchanged_and_empty_cannot_bypass_validation() {
    let mut session = Session::blank(595., 842., 1).unwrap();
    let before = session.bytes().unwrap();
    let info = session.info();
    assert_eq!(
        session
            .create_free_text(
                0,
                &FreeTextDraft {
                    contents: "\t\r\n\u{2003}",
                    bounds: bounds(),
                    style: style()
                }
            )
            .unwrap(),
        FreeTextCommit::Unchanged
    );
    assert_eq!(session.bytes().unwrap(), before);
    assert_eq!(session.info(), info);
    assert!(session.undo().is_err());
    assert!(
        session
            .create_free_text(
                0,
                &FreeTextDraft {
                    contents: &" ".repeat(65_537),
                    bounds: bounds(),
                    style: style()
                }
            )
            .is_err()
    );
    let mut invalid = bounds();
    invalid.width = 0.;
    assert!(
        session
            .create_free_text(
                0,
                &FreeTextDraft {
                    contents: "",
                    bounds: invalid,
                    style: style()
                }
            )
            .is_err()
    );
    let mut invalid_style = style();
    invalid_style.color.g = f32::NAN;
    assert!(
        session
            .create_free_text(
                0,
                &FreeTextDraft {
                    contents: "",
                    bounds: bounds(),
                    style: invalid_style
                }
            )
            .is_err()
    );
    assert!(
        session
            .create_free_text(
                1,
                &FreeTextDraft {
                    contents: "",
                    bounds: bounds(),
                    style: style()
                }
            )
            .is_err()
    );
    assert_eq!(session.bytes().unwrap(), before);
    assert_eq!(session.info(), info);
}

#[test]
fn unknown_font_none_collects_body_and_resize_refusals_without_font_fallback() {
    let mut refused = Vec::new();
    for action in ["body", "resize"] {
        let mut session = edited_fixture(|doc, id| {
            doc.get_object_mut(id)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .remove(b"PDFMasterAppearance");
        });
        seed_two_sided_history(&mut session);
        let history = history_trace(&mut session);
        let selected = row(&session);
        let mut update = same_update(&selected);
        if action == "body" {
            update.contents = "new 中文";
        } else {
            update.bounds.width -= 1.;
        }
        refused.push(session.update_free_text(&selected, &update).is_err());
        assert_eq!(history_trace(&mut session), history);
    }
    assert_eq!(refused, vec![true; 2]);
}

#[test]
fn unknown_font_moves_without_reflow_but_needs_explicit_style_for_body_and_resize() {
    let mut session = edited_fixture(|doc, id| {
        doc.get_object_mut(id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .remove(b"PDFMasterAppearance");
    });
    let selected = row(&session);
    assert_eq!(session.free_text_appearance_style(&selected).unwrap(), None);
    let before = Document::load_mem(&session.bytes().unwrap()).unwrap();
    let old_ap = before
        .get_object(selected.object_id.unwrap())
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"AP")
        .unwrap()
        .clone();
    let mut moved = same_update(&selected);
    moved.bounds.x += 5.;
    assert_eq!(
        session.update_free_text(&selected, &moved).unwrap(),
        FreeTextCommit::Updated
    );
    let selected = row(&session);
    let after = Document::load_mem(&session.bytes().unwrap()).unwrap();
    assert_eq!(
        *after
            .get_object(selected.object_id.unwrap())
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"AP")
            .unwrap(),
        old_ap
    );
    let bytes = session.bytes().unwrap();
    for resize in [false, true] {
        let mut update = same_update(&selected);
        if resize {
            update.bounds.width -= 1.;
        } else {
            update.contents = "changed";
        }
        assert!(session.update_free_text(&selected, &update).is_err());
        assert_eq!(session.bytes().unwrap(), bytes);
    }
    assert_eq!(
        session
            .update_free_text(
                &selected,
                &FreeTextUpdate {
                    contents: "changed",
                    style: Some(style()),
                    ..same_update(&selected)
                }
            )
            .unwrap(),
        FreeTextCommit::Updated
    );
}

#[test]
fn all_snapshot_fields_and_stale_ap_or_metadata_are_rejected() {
    let mut session = created();
    seed_two_sided_history(&mut session);
    let baseline_history = history_trace(&mut session);
    let selected = row(&session);
    let before = session.bytes().unwrap();
    let mut cases = Vec::new();
    macro_rules! forged {
        ($field:ident,$value:expr) => {{
            let mut value = selected.clone();
            value.$field = $value;
            cases.push(value);
        }};
    }
    forged!(page_index, 1);
    forged!(annotation_index, 1);
    forged!(page_id, (999, 0));
    forged!(object_id, None);
    forged!(contents, "forged".into());
    forged!(flags, 128);
    forged!(revision, [0; 32]);
    let mut altered = selected.clone();
    altered.bounds.x += 1.;
    cases.push(altered);
    let mut altered = selected.clone();
    altered.bounds.y += 1.;
    cases.push(altered);
    let mut altered = selected.clone();
    altered.bounds.width -= 1.;
    cases.push(altered);
    let mut altered = selected.clone();
    altered.bounds.height -= 1.;
    cases.push(altered);
    let mut altered = selected.clone();
    altered.style.font_size += 1.;
    cases.push(altered);
    let mut altered = selected.clone();
    altered.style.color.r = 0.;
    cases.push(altered);
    let mut altered = selected.clone();
    altered.style.color.g = 0.;
    cases.push(altered);
    let mut altered = selected.clone();
    altered.style.color.b = 0.;
    cases.push(altered);
    let mut altered = selected.clone();
    altered.style.font_resource = b"other".to_vec();
    cases.push(altered);
    let mut altered = selected.clone();
    altered.style.alignment = TextAlignment::Center;
    cases.push(altered);
    for forged in cases {
        assert!(
            session
                .update_free_text(&forged, &same_update(&selected))
                .is_err()
        );
        assert!(session.free_text_appearance_style(&forged).is_err());
        assert_eq!(session.bytes().unwrap(), before);
        assert_eq!(history_trace(&mut session), baseline_history);
    }
    for key in ["AP", "PDFMasterAppearance"] {
        let mut doc = Document::load_mem(&before).unwrap();
        doc.get_object_mut(selected.object_id.unwrap())
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set(key, dictionary! {"Unknown"=>1});
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        let mut changed = Session::open(&bytes).unwrap();
        assert!(
            changed
                .update_free_text(&selected, &same_update(&selected))
                .is_err()
        );
        assert!(changed.free_text_appearance_style(&selected).is_err());
    }
}

#[test]
fn complex_visual_profiles_can_move_but_cannot_be_silently_reflowed() {
    for (key, value) in [
        ("C", Object::Array(vec![1.into(), 0.into(), 0.into()])),
        ("IC", Object::Array(vec![1.into(), 0.into(), 0.into()])),
        ("CA", 0.5.into()),
        ("ca", 0.5.into()),
        ("BM", Object::Name(b"Multiply".to_vec())),
        ("BE", dictionary! {"S"=>"C"}.into()),
        ("BS", dictionary! {"W"=>1}.into()),
        ("Border", Object::Array(vec![0.into(), 0.into(), 1.into()])),
    ] {
        let mut session = edited_fixture(|doc, id| {
            doc.get_object_mut(id)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set(key, value)
        });
        let selected = row(&session);
        let before = session.bytes().unwrap();
        assert!(
            session
                .update_free_text(
                    &selected,
                    &FreeTextUpdate {
                        contents: "changed",
                        ..same_update(&selected)
                    }
                )
                .is_err(),
            "{key}"
        );
        assert_eq!(session.bytes().unwrap(), before);
        let mut moved = same_update(&selected);
        moved.bounds.x += 1.;
        assert_eq!(
            session.update_free_text(&selected, &moved).unwrap(),
            FreeTextCommit::Updated
        );
    }
}

#[test]
fn effective_default_border_refuses_each_reflow_but_pure_move_preserves_appearance() {
    for action in ["body", "resize", "style"] {
        let mut session = edited_fixture(|doc, id| {
            let annotation = doc.get_object_mut(id).unwrap().as_dict_mut().unwrap();
            annotation.remove(b"BS");
            annotation.remove(b"Border");
        });
        seed_two_sided_history(&mut session);
        let baseline_history = history_trace(&mut session);
        let selected = row(&session);
        let before = Document::load_mem(&session.bytes().unwrap()).unwrap();
        let old_ap = annotation_dictionary(&before, &selected)
            .get(b"AP")
            .unwrap()
            .clone();
        assert_eq!(
            session
                .update_free_text(&selected, &same_update(&selected))
                .unwrap(),
            FreeTextCommit::Unchanged,
            "{action}"
        );
        assert_eq!(history_trace(&mut session), baseline_history, "{action}");
        let mut update = same_update(&selected);
        match action {
            "body" => update.contents = "changed source 中文",
            "resize" => update.bounds.width -= 7.,
            "style" => {
                let mut explicit = style();
                explicit.font_size += 1.;
                update.style = Some(explicit);
            }
            _ => unreachable!(),
        }
        assert_eq!(
            session
                .update_free_text(&selected, &update)
                .unwrap_err()
                .to_string(),
            "invalid operation: complex FreeText appearance cannot be reflowed",
            "{action}"
        );
        assert_eq!(history_trace(&mut session), baseline_history, "{action}");

        let mut moved = same_update(&selected);
        moved.bounds.x += 5.;
        assert_eq!(
            session.update_free_text(&selected, &moved).unwrap(),
            FreeTextCommit::Updated,
            "{action}"
        );
        let moved_bytes = session.bytes().unwrap();
        let moved_row = row(&session);
        assert_eq!(moved_row.contents, selected.contents, "{action}");
        assert_eq!(moved_row.bounds, moved.bounds, "{action}");
        let moved_doc = Document::load_mem(&moved_bytes).unwrap();
        assert_eq!(
            annotation_dictionary(&moved_doc, &moved_row)
                .get(b"AP")
                .unwrap(),
            &old_ap,
            "{action}"
        );
        session.undo().unwrap();
        assert_eq!(
            session.bytes().unwrap(),
            baseline_history.current,
            "{action}"
        );
        session.redo().unwrap();
        assert_eq!(session.bytes().unwrap(), moved_bytes, "{action}");
    }
}

#[test]
fn bs_zero_overrides_nonzero_border_for_actual_reflow() {
    let mut session = edited_fixture(|doc, id| {
        let annotation = doc.get_object_mut(id).unwrap().as_dict_mut().unwrap();
        annotation.set("BS", dictionary! {"W"=>0});
        annotation.set("Border", vec![Object::Integer(0), 0.into(), 9.into()]);
    });
    seed_two_sided_history(&mut session);
    let selected = row(&session);
    let before_bytes = session.bytes().unwrap();
    let before = Document::load_mem(&before_bytes).unwrap();
    let old_ap = annotation_dictionary(&before, &selected)
        .get(b"AP")
        .unwrap()
        .clone();
    let mut changed_bounds = selected.bounds;
    changed_bounds.width -= 7.;
    let mut changed_style = style();
    changed_style.font_size += 1.;
    let changed_source = "changed source 中文 𠮷";
    assert_eq!(
        session
            .update_free_text(
                &selected,
                &FreeTextUpdate {
                    contents: changed_source,
                    bounds: changed_bounds,
                    style: Some(changed_style.clone()),
                },
            )
            .unwrap(),
        FreeTextCommit::Updated
    );
    let changed_bytes = session.bytes().unwrap();
    let changed = row(&session);
    assert_eq!(changed.contents, changed_source);
    assert_eq!(changed.bounds, changed_bounds);
    assert_eq!(
        session.free_text_appearance_style(&changed).unwrap(),
        Some(changed_style)
    );
    let after = Document::load_mem(&changed_bytes).unwrap();
    assert_ne!(
        annotation_dictionary(&after, &changed).get(b"AP").unwrap(),
        &old_ap
    );
    session.undo().unwrap();
    assert_eq!(session.bytes().unwrap(), before_bytes);
    session.redo().unwrap();
    assert_eq!(session.bytes().unwrap(), changed_bytes);
}

#[test]
fn explicit_style_roundtrips_all_alignments_and_page_rotations() {
    for rotation in [0, 90, 180, 270] {
        for alignment in [
            TextAlignment::Left,
            TextAlignment::Center,
            TextAlignment::Right,
        ] {
            let mut blank = Session::blank(595., 842., 1).unwrap();
            let mut doc = Document::load_mem(&blank.bytes().unwrap()).unwrap();
            let page = *doc.get_pages().values().next().unwrap();
            doc.get_object_mut(page)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("Rotate", rotation);
            let mut bytes = Vec::new();
            doc.save_to(&mut bytes).unwrap();
            blank = Session::open(&bytes).unwrap();
            let mut appearance = style();
            appearance.alignment = alignment;
            blank
                .create_free_text(
                    0,
                    &FreeTextDraft {
                        contents: "English 中文 مرحبا हैलो 𠮷\r\nCafé e\u{301}",
                        bounds: bounds(),
                        style: appearance.clone(),
                    },
                )
                .unwrap();
            let reopened = Session::open(&blank.bytes().unwrap()).unwrap();
            let selected = row(&reopened);
            assert_eq!(selected.bounds, bounds());
            assert_eq!(
                reopened.free_text_appearance_style(&selected).unwrap(),
                Some(appearance.clone())
            );
            assert!(!reopened.render_png(0, 0.5).unwrap().is_empty());
            assert_normal_stream_matches_owned_carrier(&reopened, &selected, &appearance);
            assert_eq!(
                blank
                    .update_free_text(
                        &selected,
                        &FreeTextUpdate {
                            style: Some(appearance),
                            ..same_update(&selected)
                        }
                    )
                    .unwrap(),
                FreeTextCommit::Unchanged
            );
            let mut changed_style = style();
            changed_style.alignment = alignment;
            changed_style.font_size = 19.;
            changed_style.line_height = 1.4;
            let changed_text = "updated 中文 مرحبا हैलो\r\nCafé e\u{301} 𠮷";
            let mut changed_bounds = selected.bounds;
            changed_bounds.width -= 10.;
            assert_eq!(
                blank
                    .update_free_text(
                        &selected,
                        &FreeTextUpdate {
                            contents: changed_text,
                            bounds: changed_bounds,
                            style: Some(changed_style.clone()),
                        }
                    )
                    .unwrap(),
                FreeTextCommit::Updated
            );
            let reopened = Session::open(&blank.bytes().unwrap()).unwrap();
            let changed = row(&reopened);
            assert_eq!(changed.contents, changed_text);
            assert_eq!(changed.bounds, changed_bounds);
            assert_eq!(
                reopened.free_text_appearance_style(&changed).unwrap(),
                Some(changed_style.clone())
            );
            assert_normal_stream_matches_owned_carrier(&reopened, &changed, &changed_style);
            assert!(!reopened.render_png(0, 0.25).unwrap().is_empty());
        }
    }
}

fn save_session(doc: &mut Document) -> Session {
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    Session::open(&bytes).unwrap()
}
fn annotation_dictionary(
    doc: &Document,
    selected: &pdfmaster_core::FreeTextAnnotation,
) -> lopdf::Dictionary {
    doc.get_object(selected.object_id.unwrap())
        .unwrap()
        .as_dict()
        .unwrap()
        .clone()
}
fn resolve<'a>(doc: &'a Document, value: &'a Object) -> &'a Object {
    if let Object::Reference(id) = value {
        resolve(doc, doc.get_object(*id).unwrap())
    } else {
        value
    }
}

#[test]
fn real_embedded_font_scopes_and_old_page_objects_survive_two_annotations() {
    let mut session = Session::blank(595., 842., 1).unwrap();
    let before = Document::load_mem(&session.bytes().unwrap()).unwrap();
    let page = *before.get_pages().values().next().unwrap();
    let old_page = before.get_object(page).unwrap().as_dict().unwrap();
    for contents in ["Latin 中文 مرحبا हैलो 𠮷", "different 第二 مرحباً नमस्ते"]
    {
        session
            .create_free_text(
                0,
                &FreeTextDraft {
                    contents,
                    bounds: bounds(),
                    style: style(),
                },
            )
            .unwrap();
    }
    let after = Document::load_mem(&session.bytes().unwrap()).unwrap();
    let new_page = after.get_object(page).unwrap().as_dict().unwrap();
    for key in [b"Contents".as_slice(), b"Resources"] {
        assert_eq!(old_page.get(key).ok(), new_page.get(key).ok());
    }
    for (id, old) in &before.objects {
        if *id != page && !is_xref_storage(old) {
            assert_eq!(after.objects.get(id), Some(old), "old object {id:?}");
        }
    }
    let mut form_ids = Vec::new();
    for selected in session.free_texts(0).unwrap().annotations {
        let annotation = annotation_dictionary(&after, &selected);
        let ap = resolve(&after, annotation.get(b"AP").unwrap())
            .as_dict()
            .unwrap();
        form_ids.push(ap.get(b"N").unwrap().as_reference().unwrap());
        let form = resolve(&after, ap.get(b"N").unwrap()).as_stream().unwrap();
        assert_eq!(
            form.dict
                .get(b"BBox")
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_float().unwrap())
                .collect::<Vec<_>>(),
            vec![0., 0., 360., 240.]
        );
        let resources = resolve(&after, form.dict.get(b"Resources").unwrap())
            .as_dict()
            .unwrap();
        let fonts = resolve(&after, resources.get(b"Font").unwrap())
            .as_dict()
            .unwrap();
        let operators = lopdf::content::Content::decode_strict(&form.content).unwrap();
        let mut used = std::collections::BTreeSet::new();
        for operation in operators.operations {
            if operation.operator == "Tf" {
                let name = operation.operands[0].as_name().unwrap();
                used.insert(name.to_vec());
                let font = resolve(&after, fonts.get(name).unwrap()).as_dict().unwrap();
                assert_eq!(font.get(b"Subtype").unwrap().as_name().unwrap(), b"Type0");
                assert!(
                    resolve(&after, font.get(b"ToUnicode").unwrap())
                        .as_stream()
                        .is_ok()
                );
                let descendants = resolve(&after, font.get(b"DescendantFonts").unwrap())
                    .as_array()
                    .unwrap();
                let cid = resolve(&after, &descendants[0]).as_dict().unwrap();
                let descriptor = resolve(&after, cid.get(b"FontDescriptor").unwrap())
                    .as_dict()
                    .unwrap();
                assert!(
                    resolve(
                        &after,
                        descriptor
                            .get(b"FontFile2")
                            .or_else(|_| descriptor.get(b"FontFile3"))
                            .unwrap()
                    )
                    .as_stream()
                    .is_ok()
                );
            }
            assert_ne!(operation.operator, "Do");
        }
        assert_eq!(used.len(), 4);
        assert!(form.content.windows(10).any(|v| v == b"ActualText"));
        assert!(fonts.get(&selected.style.font_resource).is_ok());
    }
    assert_ne!(form_ids[0], form_ids[1]);
    assert!(!after.objects.values().any(|value| {
        value
            .as_stream()
            .is_ok_and(|v| v.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Image"))
    }));
}

#[test]
fn cow_shared_pages_and_popup_preserve_unknown_metadata_and_original_objects() {
    let base = created();
    let selected = row(&base);
    let mut doc = Document::load_mem(&base.bytes().unwrap()).unwrap();
    let page = selected.page_id;
    let root = selected.object_id.unwrap();
    let popup=doc.add_object(dictionary!{"Type"=>"Annot","Subtype"=>"Popup","Parent"=>root,"P"=>page,"Rect"=>vec![1.into(),1.into(),20.into(),20.into()],"UnknownPopup"=>77});
    let annotation = doc.get_object_mut(root).unwrap().as_dict_mut().unwrap();
    annotation.set("Popup", popup);
    annotation.set("UnknownVendor", dictionary! {"V"=>77});
    let ap = annotation.get_mut(b"AP").unwrap().as_dict_mut().unwrap();
    ap.set("UnknownAP", 88);
    let annots = doc.add_object(vec![Object::Reference(root), Object::Reference(popup)]);
    let original_page = doc.get_object_mut(page).unwrap().as_dict_mut().unwrap();
    original_page.set("Annots", annots);
    let copy = original_page.clone();
    let second_page = doc.add_object(copy);
    let pages = doc
        .catalog()
        .unwrap()
        .get(b"Pages")
        .unwrap()
        .as_reference()
        .unwrap();
    doc.get_object_mut(pages)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set(
            "Kids",
            vec![Object::Reference(page), Object::Reference(second_page)],
        );
    doc.get_object_mut(pages)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Count", 2);
    let mut session = save_session(&mut doc);
    let before = Document::load_mem(&session.bytes().unwrap()).unwrap();
    let selected = row(&session);
    session
        .update_free_text(
            &selected,
            &FreeTextUpdate {
                contents: "updated 中文 مرحبا हैलो",
                ..same_update(&selected)
            },
        )
        .unwrap();
    let after = Document::load_mem(&session.bytes().unwrap()).unwrap();
    let changed = row(&session);
    let annotation = annotation_dictionary(&after, &changed);
    assert_eq!(
        annotation.get(b"UnknownVendor").unwrap(),
        &dictionary! {"V"=>77}.into()
    );
    assert_eq!(
        resolve(&after, annotation.get(b"AP").unwrap())
            .as_dict()
            .unwrap()
            .get(b"UnknownAP")
            .unwrap()
            .as_i64()
            .unwrap(),
        88
    );
    let new_popup = annotation.get(b"Popup").unwrap().as_reference().unwrap();
    assert_ne!(new_popup, popup);
    assert_eq!(
        after
            .get_object(new_popup)
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"Parent")
            .unwrap()
            .as_reference()
            .unwrap(),
        changed.object_id.unwrap()
    );
    assert_eq!(
        session.free_texts(1).unwrap().annotations[0].contents,
        selected.contents
    );
    for (id, value) in &before.objects {
        if *id != page && !is_xref_storage(value) {
            assert_eq!(after.objects.get(id), Some(value), "{id:?}");
        }
    }
}
// lopdf regenerates only this actual serialization storage record on save.
fn is_xref_storage(value: &Object) -> bool {
    value.as_stream().is_ok_and(|stream| {
        stream.dict.get(b"Type").and_then(Object::as_name).ok() == Some(b"XRef")
    })
}

fn assert_normal_stream_matches_owned_carrier(
    session: &Session,
    selected: &pdfmaster_core::FreeTextAnnotation,
    style: &FreeTextAppearanceStyle,
) {
    let prepared = pdfmaster_core::text_appearance::prepare_text_appearance(
        &pdfmaster_core::text_appearance::TextAppearanceRequest {
            contents: &selected.contents,
            width: selected.bounds.width,
            height: selected.bounds.height,
            font: style.font,
            font_size: style.font_size,
            color: style.color,
            alignment: style.alignment,
            line_height: style.line_height,
        },
    )
    .unwrap();
    let source = Document::load_mem(prepared.pdf_bytes()).unwrap();
    let source_page = *source.get_pages().values().next().unwrap();
    let source_page = source.get_object(source_page).unwrap().as_dict().unwrap();
    let source_stream = resolve(&source, source_page.get(b"Contents").unwrap())
        .as_stream()
        .unwrap();
    let target = Document::load_mem(&session.bytes().unwrap()).unwrap();
    let annotation = annotation_dictionary(&target, selected);
    let appearance = resolve(&target, annotation.get(b"AP").unwrap())
        .as_dict()
        .unwrap();
    let form = resolve(&target, appearance.get(b"N").unwrap())
        .as_stream()
        .unwrap();
    assert_eq!(
        form.content, source_stream.content,
        "same Krilla operators including its sole y-flip must survive cross-document import"
    );
    assert_eq!(
        form.dict
            .get(b"Matrix")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_float().unwrap())
            .collect::<Vec<_>>(),
        vec![1., 0., 0., 1., 0., 0.]
    );
}

#[test]
fn invalid_relationships_fail_atomically_with_reachable_undo_and_redo() {
    for relationship in [
        "StructParent",
        "IRT",
        "incoming",
        "sharedPopup",
        "conflictingPopup",
    ] {
        let mut session = edited_fixture(|doc, id| {
            let page = *doc.get_pages().values().next().unwrap();
            match relationship {
                "StructParent" => doc
                    .get_object_mut(id)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .set("StructParent", 0),
                "IRT" => doc
                    .get_object_mut(id)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .set("IRT", id),
                "incoming" => {
                    let incoming=doc.add_object(dictionary!{"Subtype"=>"Text","Rect"=>vec![1.into(),1.into(),20.into(),20.into()],"IRT"=>id});
                    let annots =
                        doc.add_object(vec![Object::Reference(id), Object::Reference(incoming)]);
                    doc.get_object_mut(page)
                        .unwrap()
                        .as_dict_mut()
                        .unwrap()
                        .set("Annots", annots);
                }
                _ => {
                    let popup=doc.add_object(dictionary!{"Subtype"=>"Popup","Rect"=>vec![1.into(),1.into(),20.into(),20.into()],"Parent"=>id});
                    doc.get_object_mut(id)
                        .unwrap()
                        .as_dict_mut()
                        .unwrap()
                        .set("Popup", popup);
                    let other = if relationship == "sharedPopup" {
                        doc.add_object(dictionary!{"Subtype"=>"Text","Rect"=>vec![1.into(),1.into(),20.into(),20.into()],"Popup"=>popup})
                    } else {
                        doc.add_object(dictionary!{"Subtype"=>"Popup","Rect"=>vec![1.into(),1.into(),20.into(),20.into()],"Parent"=>id})
                    };
                    let annots = doc.add_object(vec![
                        Object::Reference(id),
                        Object::Reference(popup),
                        Object::Reference(other),
                    ]);
                    doc.get_object_mut(page)
                        .unwrap()
                        .as_dict_mut()
                        .unwrap()
                        .set("Annots", annots);
                }
            }
        });
        // Build history on another page, keeping the forbidden target untouched.
        session
            .create_free_text(
                0,
                &FreeTextDraft {
                    contents: "history one",
                    bounds: bounds(),
                    style: style(),
                },
            )
            .unwrap();
        let before = session.bytes().unwrap();
        session
            .create_free_text(
                0,
                &FreeTextDraft {
                    contents: "history two",
                    bounds: bounds(),
                    style: style(),
                },
            )
            .unwrap();
        let after = session.bytes().unwrap();
        session.undo().unwrap();
        let selected = row(&session);
        let info = session.info();
        assert!(
            session
                .update_free_text(
                    &selected,
                    &FreeTextUpdate {
                        contents: "changed",
                        ..same_update(&selected)
                    }
                )
                .is_err(),
            "{relationship}"
        );
        assert_eq!(session.bytes().unwrap(), before);
        assert_eq!(session.info(), info);
        session.undo().unwrap();
        session.redo().unwrap();
        assert_eq!(session.bytes().unwrap(), before);
        session.redo().unwrap();
        assert_eq!(session.bytes().unwrap(), after);
    }
}

#[test]
fn invalid_metadata_and_input_fail_before_any_transaction() {
    for metadata in [
        Object::Null,
        Object::Reference((999999, 0)),
        dictionary! {"V"=>2,"Font"=>"Sans","LineHeight"=>1.3}.into(),
        dictionary! {"V"=>1,"Font"=>"unknown","LineHeight"=>1.3}.into(),
        dictionary! {"V"=>1,"Font"=>"Sans","LineHeight"=>0.9}.into(),
        dictionary! {"V"=>1,"Font"=>Object::Reference((999999,0)),"LineHeight"=>1.3}.into(),
    ] {
        let session = edited_fixture(|doc, id| {
            doc.get_object_mut(id)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("PDFMasterAppearance", metadata)
        });
        let selected = row(&session);
        assert!(session.free_text_appearance_style(&selected).is_err());
    }
    let mut session = created();
    let selected = row(&session);
    let before = session.bytes().unwrap();
    for contents in ["\u{10ffff}", "x\0y", &"word ".repeat(10000)] {
        assert!(
            session
                .update_free_text(
                    &selected,
                    &FreeTextUpdate {
                        contents,
                        ..same_update(&selected)
                    }
                )
                .is_err()
        );
        assert_eq!(session.bytes().unwrap(), before);
    }
    for value in [f32::NAN, f32::INFINITY, 0., -1., 1001.] {
        let mut appearance = style();
        appearance.font_size = value;
        assert!(
            session
                .update_free_text(
                    &selected,
                    &FreeTextUpdate {
                        style: Some(appearance),
                        ..same_update(&selected)
                    }
                )
                .is_err()
        );
    }
    let mut invalid = bounds();
    invalid.x = -1.;
    assert!(
        session
            .create_free_text(
                0,
                &FreeTextDraft {
                    contents: "",
                    bounds: invalid,
                    style: style()
                }
            )
            .is_err()
    );
    assert!(
        session
            .create_free_text(
                1,
                &FreeTextDraft {
                    contents: "",
                    bounds: bounds(),
                    style: style()
                }
            )
            .is_err()
    );
    let mut invalid_style = style();
    invalid_style.line_height = 0.9;
    assert!(
        session
            .create_free_text(
                0,
                &FreeTextDraft {
                    contents: "",
                    bounds: bounds(),
                    style: invalid_style
                }
            )
            .is_err()
    );
    assert_eq!(session.bytes().unwrap(), before);
}

fn budget_fixture(count: usize, body: &str, unsupported: bool) -> Session {
    let blank = Session::blank(595., 842., 1).unwrap();
    let mut doc = Document::load_mem(&blank.bytes().unwrap()).unwrap();
    let page = *doc.get_pages().values().next().unwrap();
    let mut values = Vec::new();
    for _ in 0..count {
        let mut value = dictionary! {"Type"=>"Annot","Subtype"=>"FreeText","Rect"=>vec![31.into(),61.into(),391.into(),301.into()],"Contents"=>lopdf::text_string(body),"DA"=>Object::string_literal(b"/foreign 18 Tf 0 g".to_vec())};
        if unsupported {
            value.set("RC", Object::string_literal(b"rich".to_vec()));
        }
        values.push(Object::Reference(doc.add_object(value)));
    }
    let annots = doc.add_object(values);
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", annots);
    save_session(&mut doc)
}
#[test]
fn target_page_annotation_and_total_body_budgets_are_checked_before_empty_or_unsupported_shortcuts()
{
    let mut full = budget_fixture(16, &"a".repeat(65_536), false);
    let before = full.bytes().unwrap();
    assert_eq!(
        full.create_free_text(
            0,
            &FreeTextDraft {
                contents: "",
                bounds: bounds(),
                style: style()
            }
        )
        .unwrap(),
        FreeTextCommit::Unchanged
    );
    assert!(
        full.create_free_text(
            0,
            &FreeTextDraft {
                contents: "x",
                bounds: bounds(),
                style: style()
            }
        )
        .is_err()
    );
    assert_eq!(full.bytes().unwrap(), before);
    let mut count_boundary = budget_fixture(1024, "", false);
    let before = count_boundary.bytes().unwrap();
    assert_eq!(
        count_boundary
            .create_free_text(
                0,
                &FreeTextDraft {
                    contents: "",
                    bounds: bounds(),
                    style: style()
                }
            )
            .unwrap(),
        FreeTextCommit::Unchanged
    );
    assert!(
        count_boundary
            .create_free_text(
                0,
                &FreeTextDraft {
                    contents: "x",
                    bounds: bounds(),
                    style: style()
                }
            )
            .is_err()
    );
    assert_eq!(count_boundary.bytes().unwrap(), before);
    for (count, body, unsupported) in [
        (1025, "".to_string(), false),
        (1, "a".repeat(65_537), true),
        (17, "a".repeat(65_536), true),
    ] {
        let mut session = budget_fixture(count, &body, unsupported);
        let before = session.bytes().unwrap();
        let info = session.info();
        assert!(
            session
                .create_free_text(
                    0,
                    &FreeTextDraft {
                        contents: "",
                        bounds: bounds(),
                        style: style()
                    }
                )
                .is_err()
        );
        assert_eq!(session.bytes().unwrap(), before);
        assert_eq!(session.info(), info);
    }
}

#[test]
fn update_incremental_target_page_body_budget_is_atomic_with_two_sided_history() {
    let session = budget_fixture(16, &"a".repeat(65_536), false);
    let mut doc = Document::load_mem(&session.bytes().unwrap()).unwrap();
    let page = *doc.get_pages().values().next().unwrap();
    let annots = doc
        .get_object(page)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Annots")
        .unwrap()
        .as_reference()
        .unwrap();
    let added = doc.add_object(dictionary! {"Type"=>"Annot","Subtype"=>"FreeText","Rect"=>vec![31.into(),61.into(),391.into(),301.into()],"Contents"=>lopdf::text_string(""),"DA"=>Object::string_literal(b"/foreign 18 Tf 0 g".to_vec())});
    doc.get_object_mut(annots)
        .unwrap()
        .as_array_mut()
        .unwrap()
        .push(Object::Reference(added));
    let mut session = save_session(&mut doc);
    seed_two_sided_history(&mut session);
    let history = history_trace(&mut session);
    let selected = session.free_texts(0).unwrap().annotations.pop().unwrap();
    assert!(
        session
            .update_free_text(
                &selected,
                &FreeTextUpdate {
                    contents: "x",
                    bounds: selected.bounds,
                    style: Some(style())
                }
            )
            .is_err()
    );
    assert_eq!(history_trace(&mut session), history);
}

#[test]
fn same_page_total_budget_does_not_become_a_new_cross_document_limit() {
    let mut session = budget_fixture(16, &"a".repeat(65_536), false);
    let mut doc = Document::load_mem(&session.bytes().unwrap()).unwrap();
    let first = *doc.get_pages().values().next().unwrap();
    let copy = doc.get_object(first).unwrap().as_dict().unwrap().clone();
    let second = doc.add_object(copy);
    let pages = doc
        .catalog()
        .unwrap()
        .get(b"Pages")
        .unwrap()
        .as_reference()
        .unwrap();
    doc.get_object_mut(pages)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set(
            "Kids",
            vec![Object::Reference(first), Object::Reference(second)],
        );
    doc.get_object_mut(pages)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Count", 2);
    session = save_session(&mut doc);
    for page in [0, 1] {
        assert_eq!(
            session
                .create_free_text(
                    page,
                    &FreeTextDraft {
                        contents: "",
                        bounds: bounds(),
                        style: style()
                    }
                )
                .unwrap(),
            FreeTextCommit::Unchanged
        );
    }
}

#[test]
fn cross_page_incoming_reply_and_wrong_popup_parent_cannot_be_orphaned() {
    for relationship in ["crossPageReply", "wrongPopupParent"] {
        let mut session = edited_fixture(|doc, id| {
            let page = *doc.get_pages().values().next().unwrap();
            if relationship == "wrongPopupParent" {
                let wrong=doc.add_object(dictionary!{"Subtype"=>"Text","Rect"=>vec![1.into(),1.into(),20.into(),20.into()]});
                let popup=doc.add_object(dictionary!{"Subtype"=>"Popup","Rect"=>vec![1.into(),1.into(),20.into(),20.into()],"Parent"=>wrong});
                doc.get_object_mut(id)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .set("Popup", popup);
            } else {
                let reply=doc.add_object(dictionary!{"Subtype"=>"Text","Rect"=>vec![1.into(),1.into(),20.into(),20.into()],"IRT"=>id});
                let mut copy = doc.get_object(page).unwrap().as_dict().unwrap().clone();
                let annots = doc.add_object(vec![Object::Reference(reply)]);
                copy.set("Annots", annots);
                let second = doc.add_object(copy);
                let pages = doc
                    .catalog()
                    .unwrap()
                    .get(b"Pages")
                    .unwrap()
                    .as_reference()
                    .unwrap();
                doc.get_object_mut(pages)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .set(
                        "Kids",
                        vec![Object::Reference(page), Object::Reference(second)],
                    );
                doc.get_object_mut(pages)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap()
                    .set("Count", 2);
            }
        });
        seed_two_sided_history(&mut session);
        let baseline = history_trace(&mut session);
        let selected = row(&session);
        assert!(
            session
                .update_free_text(
                    &selected,
                    &FreeTextUpdate {
                        contents: "changed",
                        ..same_update(&selected)
                    }
                )
                .is_err(),
            "{relationship}"
        );
        assert_eq!(history_trace(&mut session), baseline);
    }
}
