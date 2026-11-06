use super::*;
use crate::free_text_editor::{
    CanvasFreeTextTarget, FreeTextAppearance, FreeTextEditor, FreeTextFont,
};

fn bounds() -> pdfmaster_core::Rect {
    pdfmaster_core::Rect {
        x: 20.0,
        y: 20.0,
        width: 200.0,
        height: 100.0,
    }
}

fn style() -> FreeTextAppearance {
    FreeTextAppearance {
        font: FreeTextFont::Sans,
        font_size: 12.0,
        color: pdfmaster_core::Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
        },
        alignment: pdfmaster_core::TextAlignment::Left,
        line_height: 1.2,
    }
}

fn cjk_style() -> FreeTextAppearance {
    FreeTextAppearance {
        font: FreeTextFont::CjkSc,
        font_size: 14.0,
        color: pdfmaster_core::Color {
            r: 0.1,
            g: 0.2,
            b: 0.3,
        },
        alignment: pdfmaster_core::TextAlignment::Center,
        line_height: 1.4,
    }
}

fn workspace() -> (Workspace, tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let journal_path = directory.path().join("draft.json");
    let mut session = Session::blank(300.0, 300.0, 1).unwrap();
    session.mark_saved();
    let workspace = Workspace {
        session: Some(session),
        active_document_id: 1,
        next_document_id: 1,
        document_epoch: 1,
        journal: Some(recovery::Journal::new(journal_path.clone())),
        ..Default::default()
    };
    (workspace, directory, journal_path)
}

fn workspace_with_note() -> (Workspace, tempfile::TempDir) {
    let (mut workspace, directory, _journal_path) = workspace();
    workspace
        .session
        .as_mut()
        .unwrap()
        .apply(Edit::AddNote {
            page: 0,
            bounds: pdfmaster_core::Rect {
                x: 5.0,
                y: 5.0,
                width: 10.0,
                height: 10.0,
            },
            contents: "existing note".into(),
            color: pdfmaster_core::Color {
                r: 1.0,
                g: 1.0,
                b: 0.0,
            },
        })
        .unwrap();
    workspace.session.as_mut().unwrap().mark_saved();
    (workspace, directory)
}

fn finish_new_free_text(
    workspace: &mut Workspace,
    request: &str,
    contents: &str,
    commit_style: Option<FreeTextAppearance>,
) -> FreeTextEditor {
    let editor = workspace
        .begin_canvas_free_text_editor(
            request,
            1,
            0,
            CanvasFreeTextTarget::New {
                bounds: bounds(),
                style: style(),
            },
        )
        .unwrap();
    workspace
        .commit_free_text_editor(
            editor.id,
            &editor.request,
            contents.into(),
            bounds(),
            commit_style,
        )
        .unwrap();
    assert!(workspace.free_text_editor.is_none());
    editor
}

fn core_style() -> pdfmaster_core::FreeTextAppearanceStyle {
    pdfmaster_core::FreeTextAppearanceStyle {
        font: pdfmaster_core::text_appearance::BundledFont::Sans,
        font_size: 12.0,
        color: pdfmaster_core::Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
        },
        alignment: pdfmaster_core::TextAlignment::Left,
        line_height: 1.2,
    }
}

fn workspace_with_existing(contents: &str) -> (Workspace, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let mut session = Session::blank(300.0, 300.0, 1).unwrap();
    session
        .create_free_text(
            0,
            &pdfmaster_core::FreeTextDraft {
                contents,
                bounds: bounds(),
                style: core_style(),
            },
        )
        .unwrap();
    session.mark_saved();
    let workspace = Workspace {
        session: Some(session),
        active_document_id: 1,
        next_document_id: 1,
        document_epoch: 1,
        journal: Some(recovery::Journal::new(directory.path().join("draft.json"))),
        ..Default::default()
    };
    (workspace, directory)
}

fn workspace_with_mutated_existing(
    contents: &str,
    mutate: impl FnOnce(&mut lopdf::Dictionary),
) -> (Workspace, tempfile::TempDir) {
    let (mut workspace, directory) = workspace_with_existing(contents);
    let selected = workspace
        .session
        .as_ref()
        .unwrap()
        .free_texts(0)
        .unwrap()
        .annotations
        .remove(0);
    let mut document =
        lopdf::Document::load_mem(&workspace.session.as_ref().unwrap().bytes().unwrap()).unwrap();
    mutate(
        document
            .get_object_mut(selected.object_id.unwrap())
            .unwrap()
            .as_dict_mut()
            .unwrap(),
    );
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    let mut session = Session::open(&bytes).unwrap();
    session.mark_saved();
    workspace.session = Some(session);
    (workspace, directory)
}

fn page_value(bytes: &[u8], key: &[u8]) -> Option<lopdf::Object> {
    let document = lopdf::Document::load_mem(bytes).unwrap();
    let page_id = *document.get_pages().values().next().unwrap();
    document
        .get_object(page_id)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(key)
        .ok()
        .cloned()
}

#[test]
fn free_text_editor_begin_replay_and_cancel_are_non_mutating_and_guard_owner() {
    let (mut workspace, _directory, journal_path) = workspace();
    let before = workspace.session.as_ref().unwrap().bytes().unwrap();
    let before_info = workspace.session.as_ref().unwrap().info();
    let target = CanvasFreeTextTarget::New {
        bounds: bounds(),
        style: style(),
    };

    let editor = workspace
        .begin_canvas_free_text_editor("free-request-0001", 1, 0, target.clone())
        .unwrap();
    assert!(workspace.has_unsaved_work());
    assert!(workspace.ensure_editor_idle().is_err());
    assert_eq!(workspace.session.as_ref().unwrap().info(), before_info);
    assert_eq!(
        workspace
            .begin_canvas_free_text_editor("free-request-0001", 1, 0, target)
            .unwrap()
            .id,
        editor.id
    );
    let json = serde_json::to_value(workspace.view().unwrap()).unwrap();
    assert_eq!(json["free_text_editor"]["id"], editor.id);
    assert!(json["free_text_editor"].get("snapshot").is_none());
    assert!(json["free_text_editor"].get("document_epoch").is_none());

    workspace
        .cancel_free_text_editor(editor.id, &editor.request)
        .unwrap();
    assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), before);
    assert_eq!(workspace.session.as_ref().unwrap().info(), before_info);
    assert!(!workspace.has_unsaved_work());
    assert!(!journal_path.exists());
}

#[test]
fn free_text_editor_creates_real_unicode_annotation_and_exact_history() {
    let (mut workspace, _directory, _journal_path) = workspace();
    let original = workspace.session.as_ref().unwrap().bytes().unwrap();
    let original_contents = page_value(&original, b"Contents");
    let original_resources = page_value(&original, b"Resources");
    let target = CanvasFreeTextTarget::New {
        bounds: bounds(),
        style: style(),
    };
    let editor = workspace
        .begin_canvas_free_text_editor("free-request-0002", 1, 0, target)
        .unwrap();

    workspace
        .commit_free_text_editor(
            editor.id,
            &editor.request,
            "中文 Rust".into(),
            bounds(),
            None,
        )
        .unwrap();
    let saved = workspace.session.as_ref().unwrap().bytes().unwrap();
    let reopened = Session::open(&saved).unwrap();
    assert_eq!(
        reopened.free_texts(0).unwrap().annotations[0].contents,
        "中文 Rust"
    );
    assert_eq!(page_value(&saved, b"Contents"), original_contents);
    assert_eq!(page_value(&saved, b"Resources"), original_resources);
    assert!(workspace.free_text_editor.is_none());

    workspace.session.as_mut().unwrap().undo().unwrap();
    assert_eq!(
        workspace.session.as_ref().unwrap().bytes().unwrap(),
        original
    );
    workspace.session.as_mut().unwrap().redo().unwrap();
    assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), saved);
}

#[test]
fn free_text_editor_updates_real_existing_annotation_and_undoes_exactly() {
    let (mut workspace, _directory) = workspace_with_existing("中文 Rust");
    let before = workspace.session.as_ref().unwrap().bytes().unwrap();
    let original = workspace
        .session
        .as_ref()
        .unwrap()
        .free_texts(0)
        .unwrap()
        .annotations
        .remove(0);
    let editor = workspace
        .begin_canvas_free_text_editor(
            "free-request-0003",
            1,
            0,
            CanvasFreeTextTarget::Existing {
                original: original.clone(),
            },
        )
        .unwrap();
    assert_eq!(editor.appearance_style, Some(style()));

    workspace
        .commit_free_text_editor(
            editor.id,
            &editor.request,
            "修改 中文".into(),
            original.bounds,
            None,
        )
        .unwrap();
    assert_eq!(
        workspace
            .session
            .as_ref()
            .unwrap()
            .free_texts(0)
            .unwrap()
            .annotations[0]
            .contents,
        "修改 中文"
    );
    workspace.session.as_mut().unwrap().undo().unwrap();
    assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), before);
}

#[test]
fn free_text_editor_existing_noop_preserves_redo_branch_and_full_info() {
    let (mut workspace, _directory) = workspace_with_existing("中文 Rust");
    let session = workspace.session.as_mut().unwrap();
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
    let original = session.free_texts(0).unwrap().annotations.remove(0);
    let before = session.bytes().unwrap();
    let before_info = session.info();
    let editor = workspace
        .begin_canvas_free_text_editor(
            "free-request-0004",
            1,
            0,
            CanvasFreeTextTarget::Existing {
                original: original.clone(),
            },
        )
        .unwrap();

    workspace
        .commit_free_text_editor(
            editor.id,
            &editor.request,
            original.contents,
            original.bounds,
            None,
        )
        .unwrap();
    assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), before);
    assert_eq!(workspace.session.as_ref().unwrap().info(), before_info);
    assert!(workspace.free_text_editor.is_none());
}

#[test]
fn free_text_editor_cancel_keeps_existing_and_whitespace_commit_deletes_once() {
    let (mut workspace, _directory) = workspace_with_existing("中文 Rust");
    let before = workspace.session.as_ref().unwrap().bytes().unwrap();
    let original = workspace
        .session
        .as_ref()
        .unwrap()
        .free_texts(0)
        .unwrap()
        .annotations
        .remove(0);
    let cancelled = workspace
        .begin_canvas_free_text_editor(
            "free-request-0005",
            1,
            0,
            CanvasFreeTextTarget::Existing {
                original: original.clone(),
            },
        )
        .unwrap();
    workspace
        .cancel_free_text_editor(cancelled.id, &cancelled.request)
        .unwrap();
    assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), before);
    assert_eq!(
        workspace
            .session
            .as_ref()
            .unwrap()
            .free_texts(0)
            .unwrap()
            .annotations
            .len(),
        1
    );

    let deleting = workspace
        .begin_canvas_free_text_editor(
            "free-request-0006",
            1,
            0,
            CanvasFreeTextTarget::Existing {
                original: original.clone(),
            },
        )
        .unwrap();
    workspace
        .commit_free_text_editor(
            deleting.id,
            &deleting.request,
            " \n\t".into(),
            original.bounds,
            None,
        )
        .unwrap();
    assert!(
        workspace
            .session
            .as_ref()
            .unwrap()
            .free_texts(0)
            .unwrap()
            .annotations
            .is_empty()
    );
    workspace.session.as_mut().unwrap().undo().unwrap();
    assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), before);
}

#[test]
fn free_text_editor_rejects_invalid_request_before_allocating_or_owning() {
    for request in ["short", "free request 0007", "free-request-中文"] {
        let (mut workspace, _directory, _journal_path) = workspace();
        let before = workspace.session.as_ref().unwrap().bytes().unwrap();
        let before_info = workspace.session.as_ref().unwrap().info();
        assert!(
            workspace
                .begin_canvas_free_text_editor(
                    request,
                    1,
                    0,
                    CanvasFreeTextTarget::New {
                        bounds: bounds(),
                        style: style(),
                    },
                )
                .is_err()
        );
        assert!(workspace.free_text_editor.is_none());
        assert_eq!(workspace.next_editor_id, 0);
        assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), before);
        assert_eq!(workspace.session.as_ref().unwrap().info(), before_info);
    }
    let (mut workspace, _directory, _journal_path) = workspace();
    assert!(
        workspace
            .begin_canvas_free_text_editor(
                &"a".repeat(65),
                1,
                0,
                CanvasFreeTextTarget::New {
                    bounds: bounds(),
                    style: style(),
                },
            )
            .is_err()
    );
}

#[test]
fn free_text_editor_enforces_seen_request_budget_and_checked_id_allocation() {
    let (mut budgeted, _directory, _journal_path) = workspace();
    budgeted.seen_free_text_requests = (0..4096).map(|index| format!("seen-{index:04}")).collect();
    assert!(
        budgeted
            .begin_canvas_free_text_editor(
                "free-budget-0001",
                1,
                0,
                CanvasFreeTextTarget::New {
                    bounds: bounds(),
                    style: style(),
                },
            )
            .is_err()
    );
    assert!(budgeted.free_text_editor.is_none());
    assert_eq!(budgeted.next_editor_id, 0);

    let (mut exhausted, _directory, _journal_path) = workspace();
    exhausted.next_editor_id = u32::MAX;
    assert!(
        exhausted
            .begin_canvas_free_text_editor(
                "free-overflow-01",
                1,
                0,
                CanvasFreeTextTarget::New {
                    bounds: bounds(),
                    style: style(),
                },
            )
            .is_err()
    );
    assert!(exhausted.free_text_editor.is_none());
    assert_eq!(exhausted.next_editor_id, u32::MAX);
}

fn assert_existing_begin_rejected(mutator: impl FnOnce(&mut pdfmaster_core::FreeTextAnnotation)) {
    let (mut workspace, _directory) = workspace_with_existing("中文 Rust");
    let before = workspace.session.as_ref().unwrap().bytes().unwrap();
    let before_info = workspace.session.as_ref().unwrap().info();
    let mut original = workspace
        .session
        .as_ref()
        .unwrap()
        .free_texts(0)
        .unwrap()
        .annotations
        .remove(0);
    mutator(&mut original);
    assert!(
        workspace
            .begin_canvas_free_text_editor(
                "free-snapshot-01",
                1,
                0,
                CanvasFreeTextTarget::Existing { original },
            )
            .is_err()
    );
    assert!(workspace.free_text_editor.is_none());
    assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), before);
    assert_eq!(workspace.session.as_ref().unwrap().info(), before_info);
}

#[test]
fn free_text_editor_rejects_wrong_document_page_and_full_existing_snapshot() {
    let (mut wrong_document, _directory, _journal_path) = workspace();
    assert!(
        wrong_document
            .begin_canvas_free_text_editor(
                "free-document-01",
                2,
                0,
                CanvasFreeTextTarget::New {
                    bounds: bounds(),
                    style: style(),
                },
            )
            .is_err()
    );
    assert!(wrong_document.free_text_editor.is_none());

    let (mut wrong_page, _directory, _journal_path) = workspace();
    assert!(
        wrong_page
            .begin_canvas_free_text_editor(
                "free-page-00001",
                1,
                1,
                CanvasFreeTextTarget::New {
                    bounds: bounds(),
                    style: style(),
                },
            )
            .is_err()
    );
    assert!(wrong_page.free_text_editor.is_none());

    assert_existing_begin_rejected(|original| original.revision[0] ^= 1);
    assert_existing_begin_rejected(|original| original.object_id = Some((u32::MAX, 0)));
    assert_existing_begin_rejected(|original| original.bounds.x += 1.0);
    assert_existing_begin_rejected(|original| original.style.font_size += 1.0);
}

fn existing_begin_error(mutator: impl FnOnce(&mut pdfmaster_core::FreeTextAnnotation)) -> String {
    let (mut workspace, _directory) = workspace_with_existing("中文 Rust");
    let mut original = workspace
        .session
        .as_ref()
        .unwrap()
        .free_texts(0)
        .unwrap()
        .annotations
        .remove(0);
    mutator(&mut original);
    workspace
        .begin_canvas_free_text_editor(
            "free-budget-0002",
            1,
            0,
            CanvasFreeTextTarget::Existing { original },
        )
        .unwrap_err()
}

#[test]
fn free_text_editor_preflights_existing_wire_budgets_before_projection() {
    assert!(
        existing_begin_error(|original| original.contents = "a".repeat(65_537)).contains("预算")
    );
    assert!(
        existing_begin_error(|original| original.style.font_resource = vec![b'A'; 4_097])
            .contains("预算")
    );
    assert!(existing_begin_error(|original| original.annotation_index = 1_024).contains("预算"));
}

#[test]
fn free_text_editor_applies_explicit_style_to_new_and_existing_transactions() {
    let (mut workspace, _directory, _journal_path) = workspace();
    let created = workspace
        .begin_canvas_free_text_editor(
            "free-style-0001",
            1,
            0,
            CanvasFreeTextTarget::New {
                bounds: bounds(),
                style: style(),
            },
        )
        .unwrap();
    workspace
        .commit_free_text_editor(
            created.id,
            &created.request,
            "中文".into(),
            bounds(),
            Some(cjk_style()),
        )
        .unwrap();
    let selected = workspace
        .session
        .as_ref()
        .unwrap()
        .free_texts(0)
        .unwrap()
        .annotations
        .remove(0);
    assert_eq!(
        workspace
            .session
            .as_ref()
            .unwrap()
            .free_text_appearance_style(&selected)
            .unwrap()
            .unwrap()
            .font,
        pdfmaster_core::text_appearance::BundledFont::CjkSc
    );

    let editing = workspace
        .begin_canvas_free_text_editor(
            "free-style-0002",
            1,
            0,
            CanvasFreeTextTarget::Existing {
                original: selected.clone(),
            },
        )
        .unwrap();
    workspace
        .commit_free_text_editor(
            editing.id,
            &editing.request,
            "修改 中文".into(),
            selected.bounds,
            Some(style()),
        )
        .unwrap();
    let updated = workspace
        .session
        .as_ref()
        .unwrap()
        .free_texts(0)
        .unwrap()
        .annotations
        .remove(0);
    assert_eq!(
        workspace
            .session
            .as_ref()
            .unwrap()
            .free_text_appearance_style(&updated)
            .unwrap()
            .unwrap()
            .font,
        pdfmaster_core::text_appearance::BundledFont::Sans
    );
}

fn assert_new_begin_rejected(bounds: pdfmaster_core::Rect, style: FreeTextAppearance) {
    let (mut workspace, _directory, _journal_path) = workspace();
    let before = workspace.session.as_ref().unwrap().bytes().unwrap();
    let before_info = workspace.session.as_ref().unwrap().info();
    assert!(
        workspace
            .begin_canvas_free_text_editor(
                "free-geometry-01",
                1,
                0,
                CanvasFreeTextTarget::New { bounds, style },
            )
            .is_err()
    );
    assert!(workspace.free_text_editor.is_none());
    assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), before);
    assert_eq!(workspace.session.as_ref().unwrap().info(), before_info);
}

#[test]
fn free_text_editor_rejects_nonfinite_outside_oversize_rect_and_invalid_style_on_begin() {
    assert_new_begin_rejected(
        pdfmaster_core::Rect {
            x: f32::NAN,
            ..bounds()
        },
        style(),
    );
    assert_new_begin_rejected(
        pdfmaster_core::Rect {
            x: 250.0,
            ..bounds()
        },
        style(),
    );
    assert_new_begin_rejected(
        pdfmaster_core::Rect {
            width: 14_401.0,
            ..bounds()
        },
        style(),
    );
    for invalid in [
        FreeTextAppearance {
            font_size: 0.0,
            ..style()
        },
        FreeTextAppearance {
            color: pdfmaster_core::Color {
                r: 1.1,
                g: 0.0,
                b: 0.0,
            },
            ..style()
        },
        FreeTextAppearance {
            line_height: 4.1,
            ..style()
        },
    ] {
        assert_new_begin_rejected(bounds(), invalid);
    }
}

#[test]
fn free_text_editor_preflights_commit_geometry_and_style_and_keeps_owner_on_error() {
    let (mut workspace, _directory, _journal_path) = workspace();
    let editor = workspace
        .begin_canvas_free_text_editor(
            "free-commit-guard",
            1,
            0,
            CanvasFreeTextTarget::New {
                bounds: bounds(),
                style: style(),
            },
        )
        .unwrap();
    let before = workspace.session.as_ref().unwrap().bytes().unwrap();
    let before_info = workspace.session.as_ref().unwrap().info();
    let geometry_error = workspace
        .commit_free_text_editor(
            editor.id,
            &editor.request,
            "中文".into(),
            pdfmaster_core::Rect {
                x: f32::NAN,
                ..bounds()
            },
            None,
        )
        .err()
        .unwrap();
    assert!(geometry_error.contains("几何"), "{geometry_error}");
    assert_eq!(workspace.free_text_editor.as_ref().unwrap().id, editor.id);
    assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), before);
    assert_eq!(workspace.session.as_ref().unwrap().info(), before_info);

    let style_error = workspace
        .commit_free_text_editor(
            editor.id,
            &editor.request,
            "中文".into(),
            bounds(),
            Some(FreeTextAppearance {
                font_size: f32::INFINITY,
                ..style()
            }),
        )
        .err()
        .unwrap();
    assert!(style_error.contains("样式"), "{style_error}");
    assert_eq!(workspace.free_text_editor.as_ref().unwrap().id, editor.id);
    assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), before);
    assert_eq!(workspace.session.as_ref().unwrap().info(), before_info);

    workspace
        .commit_free_text_editor(editor.id, &editor.request, "中文".into(), bounds(), None)
        .unwrap();
}

#[test]
fn free_text_editor_preflights_body_budget_and_keeps_owner_for_retry() {
    let (mut workspace, _directory, _journal_path) = workspace();
    let editor = workspace
        .begin_canvas_free_text_editor(
            "free-body-00001",
            1,
            0,
            CanvasFreeTextTarget::New {
                bounds: bounds(),
                style: style(),
            },
        )
        .unwrap();
    let before = workspace.session.as_ref().unwrap().bytes().unwrap();
    let before_info = workspace.session.as_ref().unwrap().info();
    let error = workspace
        .commit_free_text_editor(
            editor.id,
            &editor.request,
            "a".repeat(65_537),
            bounds(),
            None,
        )
        .err()
        .unwrap();
    assert!(error.contains("64KiB"), "{error}");
    assert_eq!(workspace.free_text_editor.as_ref().unwrap().id, editor.id);
    assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), before);
    assert_eq!(workspace.session.as_ref().unwrap().info(), before_info);
    workspace
        .commit_free_text_editor(editor.id, &editor.request, "retry".into(), bounds(), None)
        .unwrap();
}

#[test]
fn free_text_editor_commit_rejects_wrong_id_and_request_without_touching_owner() {
    let (mut wrong_id, _directory, _journal_path) = workspace();
    let editor = wrong_id
        .begin_canvas_free_text_editor(
            "free-owner-0001",
            1,
            0,
            CanvasFreeTextTarget::New {
                bounds: bounds(),
                style: style(),
            },
        )
        .unwrap();
    let before = wrong_id.session.as_ref().unwrap().bytes().unwrap();
    let before_info = wrong_id.session.as_ref().unwrap().info();
    assert!(
        wrong_id
            .commit_free_text_editor(
                editor.id + 1,
                &editor.request,
                "wrong id".into(),
                bounds(),
                None,
            )
            .is_err()
    );
    assert_eq!(wrong_id.free_text_editor.as_ref().unwrap().id, editor.id);
    assert_eq!(wrong_id.session.as_ref().unwrap().bytes().unwrap(), before);
    assert_eq!(wrong_id.session.as_ref().unwrap().info(), before_info);

    let (mut wrong_request, _directory, _journal_path) = workspace();
    let editor = wrong_request
        .begin_canvas_free_text_editor(
            "free-owner-0002",
            1,
            0,
            CanvasFreeTextTarget::New {
                bounds: bounds(),
                style: style(),
            },
        )
        .unwrap();
    let before = wrong_request.session.as_ref().unwrap().bytes().unwrap();
    let before_info = wrong_request.session.as_ref().unwrap().info();
    assert!(
        wrong_request
            .commit_free_text_editor(
                editor.id,
                "free-owner-wrong",
                "wrong request".into(),
                bounds(),
                None,
            )
            .is_err()
    );
    assert_eq!(
        wrong_request.free_text_editor.as_ref().unwrap().id,
        editor.id
    );
    assert_eq!(
        wrong_request.session.as_ref().unwrap().bytes().unwrap(),
        before
    );
    assert_eq!(wrong_request.session.as_ref().unwrap().info(), before_info);
}

#[test]
fn free_text_editor_commit_rejects_document_epoch_identity_and_bytes_drift() {
    let (mut epoch_drift, _directory, _journal_path) = workspace();
    let editor = epoch_drift
        .begin_canvas_free_text_editor(
            "free-stale-0001",
            1,
            0,
            CanvasFreeTextTarget::New {
                bounds: bounds(),
                style: style(),
            },
        )
        .unwrap();
    let before = epoch_drift.session.as_ref().unwrap().bytes().unwrap();
    let before_info = epoch_drift.session.as_ref().unwrap().info();
    epoch_drift.document_epoch += 1;
    assert!(
        epoch_drift
            .commit_free_text_editor(
                editor.id,
                &editor.request,
                "epoch drift".into(),
                bounds(),
                None,
            )
            .is_err()
    );
    assert_eq!(epoch_drift.free_text_editor.as_ref().unwrap().id, editor.id);
    assert_eq!(
        epoch_drift.session.as_ref().unwrap().bytes().unwrap(),
        before
    );
    assert_eq!(epoch_drift.session.as_ref().unwrap().info(), before_info);

    let (mut identity_drift, _directory, _journal_path) = workspace();
    let editor = identity_drift
        .begin_canvas_free_text_editor(
            "free-stale-0002",
            1,
            0,
            CanvasFreeTextTarget::New {
                bounds: bounds(),
                style: style(),
            },
        )
        .unwrap();
    identity_drift.active_document_id = 2;
    assert!(
        identity_drift
            .commit_free_text_editor(
                editor.id,
                &editor.request,
                "identity drift".into(),
                bounds(),
                None,
            )
            .is_err()
    );
    assert_eq!(
        identity_drift.free_text_editor.as_ref().unwrap().id,
        editor.id
    );

    let (mut bytes_drift, _directory, _journal_path) = workspace();
    let editor = bytes_drift
        .begin_canvas_free_text_editor(
            "free-stale-0003",
            1,
            0,
            CanvasFreeTextTarget::New {
                bounds: bounds(),
                style: style(),
            },
        )
        .unwrap();
    bytes_drift
        .session
        .as_mut()
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    let drifted = bytes_drift.session.as_ref().unwrap().bytes().unwrap();
    let drifted_info = bytes_drift.session.as_ref().unwrap().info();
    assert!(
        bytes_drift
            .commit_free_text_editor(
                editor.id,
                &editor.request,
                "bytes drift".into(),
                bounds(),
                None,
            )
            .is_err()
    );
    assert_eq!(bytes_drift.free_text_editor.as_ref().unwrap().id, editor.id);
    assert_eq!(
        bytes_drift.session.as_ref().unwrap().bytes().unwrap(),
        drifted
    );
    assert_eq!(bytes_drift.session.as_ref().unwrap().info(), drifted_info);
}

#[test]
fn free_text_editor_begin_replay_rejects_epoch_and_bytes_drift() {
    let target = CanvasFreeTextTarget::New {
        bounds: bounds(),
        style: style(),
    };
    let (mut epoch_drift, _directory, _journal_path) = workspace();
    let editor = epoch_drift
        .begin_canvas_free_text_editor("free-replay-0001", 1, 0, target.clone())
        .unwrap();
    epoch_drift.document_epoch += 1;
    assert!(
        epoch_drift
            .begin_canvas_free_text_editor("free-replay-0001", 1, 0, target.clone())
            .is_err()
    );
    assert_eq!(epoch_drift.free_text_editor.as_ref().unwrap().id, editor.id);

    let (mut bytes_drift, _directory, _journal_path) = workspace();
    let editor = bytes_drift
        .begin_canvas_free_text_editor("free-replay-0002", 1, 0, target.clone())
        .unwrap();
    bytes_drift
        .session
        .as_mut()
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    assert!(
        bytes_drift
            .begin_canvas_free_text_editor("free-replay-0002", 1, 0, target)
            .is_err()
    );
    assert_eq!(bytes_drift.free_text_editor.as_ref().unwrap().id, editor.id);
}

#[test]
fn free_text_editor_exact_commit_retry_is_idempotent_and_changed_payload_rejects() {
    let (mut workspace, _directory, _journal_path) = workspace();
    let editor = workspace
        .begin_canvas_free_text_editor(
            "free-retry-0001",
            1,
            0,
            CanvasFreeTextTarget::New {
                bounds: bounds(),
                style: style(),
            },
        )
        .unwrap();
    workspace
        .commit_free_text_editor(
            editor.id,
            &editor.request,
            "中文 Rust".into(),
            bounds(),
            None,
        )
        .unwrap();
    let once = workspace.session.as_ref().unwrap().bytes().unwrap();
    let once_info = workspace.session.as_ref().unwrap().info();
    workspace
        .commit_free_text_editor(
            editor.id,
            &editor.request,
            "中文 Rust".into(),
            bounds(),
            None,
        )
        .unwrap();
    assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), once);
    assert_eq!(workspace.session.as_ref().unwrap().info(), once_info);
    assert!(
        workspace
            .commit_free_text_editor(
                editor.id,
                &editor.request,
                "different".into(),
                bounds(),
                None,
            )
            .is_err()
    );
    assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), once);
    assert_eq!(workspace.session.as_ref().unwrap().info(), once_info);
    workspace
        .cancel_free_text_editor(editor.id, &editor.request)
        .unwrap();
    assert!(
        workspace
            .begin_canvas_free_text_editor(
                &editor.request,
                1,
                0,
                CanvasFreeTextTarget::New {
                    bounds: bounds(),
                    style: style(),
                },
            )
            .is_err()
    );
}

#[test]
fn free_text_editor_cancel_retry_and_old_terminal_cannot_affect_new_owner() {
    let (mut workspace, _directory, _journal_path) = workspace();
    let first = workspace
        .begin_canvas_free_text_editor(
            "free-cancel-0001",
            1,
            0,
            CanvasFreeTextTarget::New {
                bounds: bounds(),
                style: style(),
            },
        )
        .unwrap();
    workspace
        .cancel_free_text_editor(first.id, &first.request)
        .unwrap();
    let cancelled_bytes = workspace.session.as_ref().unwrap().bytes().unwrap();
    let cancelled_info = workspace.session.as_ref().unwrap().info();
    workspace
        .cancel_free_text_editor(first.id, &first.request)
        .unwrap();
    assert_eq!(
        workspace.session.as_ref().unwrap().bytes().unwrap(),
        cancelled_bytes
    );
    assert_eq!(workspace.session.as_ref().unwrap().info(), cancelled_info);
    assert!(
        workspace
            .commit_free_text_editor(first.id, &first.request, "cancelled".into(), bounds(), None,)
            .is_err()
    );

    let second = workspace
        .begin_canvas_free_text_editor(
            "free-cancel-0002",
            1,
            0,
            CanvasFreeTextTarget::New {
                bounds: bounds(),
                style: style(),
            },
        )
        .unwrap();
    assert!(
        workspace
            .cancel_free_text_editor(first.id, &first.request)
            .is_err()
    );
    assert_eq!(workspace.free_text_editor.as_ref().unwrap().id, second.id);
    assert!(
        workspace
            .commit_free_text_editor(
                first.id,
                &first.request,
                "old terminal".into(),
                bounds(),
                None,
            )
            .is_err()
    );
    assert_eq!(workspace.free_text_editor.as_ref().unwrap().id, second.id);
}

#[test]
fn free_text_editor_request_cancel_tombstones_before_begin_and_cancels_only_owner() {
    let (mut workspace, _directory, _journal_path) = workspace();
    let before = workspace.session.as_ref().unwrap().bytes().unwrap();
    workspace
        .cancel_canvas_free_text_request("free-tombstone-1", 1)
        .unwrap();
    assert!(
        workspace
            .begin_canvas_free_text_editor(
                "free-tombstone-1",
                1,
                0,
                CanvasFreeTextTarget::New {
                    bounds: bounds(),
                    style: style(),
                },
            )
            .is_err()
    );
    assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), before);

    let editor = workspace
        .begin_canvas_free_text_editor(
            "free-tombstone-2",
            1,
            0,
            CanvasFreeTextTarget::New {
                bounds: bounds(),
                style: style(),
            },
        )
        .unwrap();
    assert!(
        workspace
            .cancel_canvas_free_text_request("free-tombstone-3", 1)
            .is_err()
    );
    assert_eq!(workspace.free_text_editor.as_ref().unwrap().id, editor.id);
    workspace
        .cancel_canvas_free_text_request(&editor.request, 1)
        .unwrap();
    assert!(workspace.free_text_editor.is_none());
    assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), before);

    assert!(
        workspace
            .cancel_canvas_free_text_request("free-tombstone-4", 2)
            .is_err()
    );
}

#[test]
fn free_text_editor_is_mutually_exclusive_with_note_ink_export_and_other_free_text() {
    let target = CanvasFreeTextTarget::New {
        bounds: bounds(),
        style: style(),
    };
    let (mut note_owned, _directory, _journal_path) = workspace();
    let note = note_owned
        .begin_canvas_note_editor(
            "note-owner-0001",
            1,
            0,
            note_editor::CanvasNoteTarget::New {
                x: 30.0,
                y: 30.0,
                page_width: 300.0,
                page_height: 300.0,
                rotation: 0,
            },
        )
        .unwrap();
    assert!(
        note_owned
            .begin_canvas_free_text_editor("free-cross-0001", 1, 0, target.clone())
            .is_err()
    );
    assert_eq!(note_owned.note_editor.as_ref().unwrap().id, note.id);

    let (mut ink_owned, _directory, _journal_path) = workspace();
    let ink = ink_owned
        .begin_ink(
            "ink-owner-00001",
            0,
            pdfmaster_core::Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
            },
            2.0,
        )
        .unwrap();
    assert!(
        ink_owned
            .begin_canvas_free_text_editor("free-cross-0002", 1, 0, target.clone())
            .is_err()
    );
    assert_eq!(ink_owned.ink_editor.as_ref().unwrap().id, ink.id);

    let (mut export_owned, _directory, _journal_path) = workspace();
    let export_id = export_owned.begin_export().unwrap().0;
    assert!(
        export_owned
            .begin_canvas_free_text_editor("free-cross-0003", 1, 0, target.clone())
            .is_err()
    );
    assert_eq!(export_owned.export_reservation, Some(export_id));

    let (mut free_owned, _directory, _journal_path) = workspace();
    let editor = free_owned
        .begin_canvas_free_text_editor("free-cross-0004", 1, 0, target.clone())
        .unwrap();
    assert!(
        free_owned
            .begin_canvas_free_text_editor("free-cross-0005", 1, 0, target)
            .is_err()
    );
    assert_eq!(free_owned.free_text_editor.as_ref().unwrap().id, editor.id);
    assert!(
        free_owned
            .begin_canvas_note_editor(
                "note-owner-0002",
                1,
                0,
                note_editor::CanvasNoteTarget::New {
                    x: 30.0,
                    y: 30.0,
                    page_width: 300.0,
                    page_height: 300.0,
                    rotation: 0,
                },
            )
            .is_err()
    );
    assert!(
        free_owned
            .begin_ink(
                "ink-owner-00002",
                0,
                pdfmaster_core::Color {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                },
                2.0,
            )
            .is_err()
    );
    assert!(free_owned.begin_export().is_err());
    assert_eq!(free_owned.free_text_editor.as_ref().unwrap().id, editor.id);
}

#[test]
fn free_text_editor_cancel_and_terminal_receipts_reject_identity_epoch_and_bytes_drift() {
    let (mut active_epoch, _directory, _journal_path) = workspace();
    let editor = active_epoch
        .begin_canvas_free_text_editor(
            "free-cancel-stale-1",
            1,
            0,
            CanvasFreeTextTarget::New {
                bounds: bounds(),
                style: style(),
            },
        )
        .unwrap();
    active_epoch.document_epoch += 1;
    assert!(
        active_epoch
            .cancel_free_text_editor(editor.id, &editor.request)
            .is_err()
    );
    assert_eq!(
        active_epoch.free_text_editor.as_ref().unwrap().id,
        editor.id
    );

    let (mut active_bytes, _directory, _journal_path) = workspace();
    let editor = active_bytes
        .begin_canvas_free_text_editor(
            "free-cancel-stale-2",
            1,
            0,
            CanvasFreeTextTarget::New {
                bounds: bounds(),
                style: style(),
            },
        )
        .unwrap();
    active_bytes
        .session
        .as_mut()
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    let drifted = active_bytes.session.as_ref().unwrap().bytes().unwrap();
    assert!(
        active_bytes
            .cancel_free_text_editor(editor.id, &editor.request)
            .is_err()
    );
    assert_eq!(
        active_bytes.free_text_editor.as_ref().unwrap().id,
        editor.id
    );
    assert_eq!(
        active_bytes.session.as_ref().unwrap().bytes().unwrap(),
        drifted
    );

    let (mut terminal, _directory, _journal_path) = workspace();
    let editor = terminal
        .begin_canvas_free_text_editor(
            "free-terminal-stale",
            1,
            0,
            CanvasFreeTextTarget::New {
                bounds: bounds(),
                style: style(),
            },
        )
        .unwrap();
    terminal
        .commit_free_text_editor(editor.id, &editor.request, "saved".into(), bounds(), None)
        .unwrap();
    terminal.document_epoch += 1;
    assert!(
        terminal
            .commit_free_text_editor(editor.id, &editor.request, "saved".into(), bounds(), None,)
            .is_err()
    );
    assert!(
        terminal
            .cancel_free_text_editor(editor.id, &editor.request)
            .is_err()
    );
}

#[test]
fn free_text_editor_terminal_receipts_reject_document_bytes_and_other_owner_drift() {
    let (mut document_drift, _directory, _journal_path) = workspace();
    let document_editor =
        finish_new_free_text(&mut document_drift, "free-terminal-doc", "terminal", None);
    let document_bytes = document_drift.session.as_ref().unwrap().bytes().unwrap();
    let document_info = document_drift.session.as_ref().unwrap().info();
    let document_epoch = document_drift.document_epoch;
    document_drift.active_document_id += 1;
    assert!(
        document_drift
            .commit_free_text_editor(
                document_editor.id,
                &document_editor.request,
                "terminal".into(),
                bounds(),
                None,
            )
            .is_err()
    );
    assert!(
        document_drift
            .cancel_free_text_editor(document_editor.id, &document_editor.request)
            .is_err()
    );
    assert_eq!(document_drift.document_epoch, document_epoch);
    assert_eq!(
        document_drift.session.as_ref().unwrap().bytes().unwrap(),
        document_bytes
    );
    assert_eq!(
        document_drift.session.as_ref().unwrap().info(),
        document_info
    );
    assert!(document_drift.free_text_editor.is_none());

    let (mut bytes_drift, _directory, _journal_path) = workspace();
    let bytes_editor =
        finish_new_free_text(&mut bytes_drift, "free-terminal-bytes", "terminal", None);
    let bytes_epoch = bytes_drift.document_epoch;
    bytes_drift
        .session
        .as_mut()
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    let drifted_bytes = bytes_drift.session.as_ref().unwrap().bytes().unwrap();
    let drifted_info = bytes_drift.session.as_ref().unwrap().info();
    assert!(
        bytes_drift
            .commit_free_text_editor(
                bytes_editor.id,
                &bytes_editor.request,
                "terminal".into(),
                bounds(),
                None,
            )
            .is_err()
    );
    assert!(
        bytes_drift
            .cancel_free_text_editor(bytes_editor.id, &bytes_editor.request)
            .is_err()
    );
    assert_eq!(bytes_drift.document_epoch, bytes_epoch);
    assert_eq!(
        bytes_drift.session.as_ref().unwrap().bytes().unwrap(),
        drifted_bytes
    );
    assert_eq!(bytes_drift.session.as_ref().unwrap().info(), drifted_info);
    assert!(bytes_drift.free_text_editor.is_none());

    let (mut note_owned, _directory) = workspace_with_note();
    let note_editor = finish_new_free_text(&mut note_owned, "free-terminal-note", "terminal", None);
    let note_bytes = note_owned.session.as_ref().unwrap().bytes().unwrap();
    let note_info = note_owned.session.as_ref().unwrap().info();
    let note_epoch = note_owned.document_epoch;
    let note_owner = note_owned.begin_note_editor(0, 0, "existing note").unwrap();
    assert!(
        note_owned
            .commit_free_text_editor(
                note_editor.id,
                &note_editor.request,
                "terminal".into(),
                bounds(),
                None,
            )
            .is_err()
    );
    assert!(
        note_owned
            .cancel_free_text_editor(note_editor.id, &note_editor.request)
            .is_err()
    );
    assert_eq!(note_owned.document_epoch, note_epoch);
    assert_eq!(
        note_owned.session.as_ref().unwrap().bytes().unwrap(),
        note_bytes
    );
    assert_eq!(note_owned.session.as_ref().unwrap().info(), note_info);
    assert_eq!(note_owned.note_editor.as_ref().unwrap().id, note_owner.id);
    assert!(note_owned.free_text_editor.is_none());

    let (mut ink_owned, _directory, _journal_path) = workspace();
    let ink_editor = finish_new_free_text(&mut ink_owned, "free-terminal-ink", "terminal", None);
    let ink_bytes = ink_owned.session.as_ref().unwrap().bytes().unwrap();
    let ink_info = ink_owned.session.as_ref().unwrap().info();
    let ink_epoch = ink_owned.document_epoch;
    let ink_owner = ink_owned
        .begin_ink(
            "ink-terminal-owner",
            0,
            pdfmaster_core::Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
            },
            2.0,
        )
        .unwrap();
    assert!(
        ink_owned
            .commit_free_text_editor(
                ink_editor.id,
                &ink_editor.request,
                "terminal".into(),
                bounds(),
                None,
            )
            .is_err()
    );
    assert!(
        ink_owned
            .cancel_free_text_editor(ink_editor.id, &ink_editor.request)
            .is_err()
    );
    assert_eq!(ink_owned.document_epoch, ink_epoch);
    assert_eq!(
        ink_owned.session.as_ref().unwrap().bytes().unwrap(),
        ink_bytes
    );
    assert_eq!(ink_owned.session.as_ref().unwrap().info(), ink_info);
    assert_eq!(ink_owned.ink_editor.as_ref().unwrap().id, ink_owner.id);
    assert!(ink_owned.free_text_editor.is_none());

    let (mut export_owned, _directory, _journal_path) = workspace();
    let export_editor =
        finish_new_free_text(&mut export_owned, "free-terminal-export", "terminal", None);
    let export_bytes = export_owned.session.as_ref().unwrap().bytes().unwrap();
    let export_info = export_owned.session.as_ref().unwrap().info();
    let export_epoch = export_owned.document_epoch;
    let export_token = export_owned.begin_export().unwrap().0;
    assert!(
        export_owned
            .commit_free_text_editor(
                export_editor.id,
                &export_editor.request,
                "terminal".into(),
                bounds(),
                None,
            )
            .is_err()
    );
    assert!(
        export_owned
            .cancel_free_text_editor(export_editor.id, &export_editor.request)
            .is_err()
    );
    assert_eq!(export_owned.document_epoch, export_epoch);
    assert_eq!(
        export_owned.session.as_ref().unwrap().bytes().unwrap(),
        export_bytes
    );
    assert_eq!(export_owned.session.as_ref().unwrap().info(), export_info);
    assert_eq!(export_owned.export_reservation, Some(export_token));
    assert!(export_owned.free_text_editor.is_none());
}

#[test]
fn free_text_editor_terminal_payload_rejects_bounds_option_and_full_style_drift() {
    let (mut none_payload, _directory, _journal_path) = workspace();
    let none_editor =
        finish_new_free_text(&mut none_payload, "free-payload-none", "terminal", None);
    let none_bytes = none_payload.session.as_ref().unwrap().bytes().unwrap();
    let none_info = none_payload.session.as_ref().unwrap().info();
    let changed_bounds = pdfmaster_core::Rect {
        x: bounds().x + 1.0,
        ..bounds()
    };
    assert!(
        none_payload
            .commit_free_text_editor(
                none_editor.id,
                &none_editor.request,
                "terminal".into(),
                changed_bounds,
                None,
            )
            .is_err()
    );
    assert_eq!(
        none_payload.session.as_ref().unwrap().bytes().unwrap(),
        none_bytes
    );
    assert_eq!(none_payload.session.as_ref().unwrap().info(), none_info);
    assert!(none_payload.free_text_editor.is_none());
    none_payload
        .commit_free_text_editor(
            none_editor.id,
            &none_editor.request,
            "terminal".into(),
            bounds(),
            None,
        )
        .unwrap();
    assert!(
        none_payload
            .commit_free_text_editor(
                none_editor.id,
                &none_editor.request,
                "terminal".into(),
                bounds(),
                Some(style()),
            )
            .is_err()
    );
    assert_eq!(
        none_payload.session.as_ref().unwrap().bytes().unwrap(),
        none_bytes
    );
    assert_eq!(none_payload.session.as_ref().unwrap().info(), none_info);
    assert!(none_payload.free_text_editor.is_none());
    none_payload
        .commit_free_text_editor(
            none_editor.id,
            &none_editor.request,
            "terminal".into(),
            bounds(),
            None,
        )
        .unwrap();

    let (mut some_payload, _directory, _journal_path) = workspace();
    let committed_style = cjk_style();
    let some_editor = finish_new_free_text(
        &mut some_payload,
        "free-payload-some",
        "中文",
        Some(committed_style.clone()),
    );
    let some_bytes = some_payload.session.as_ref().unwrap().bytes().unwrap();
    let some_info = some_payload.session.as_ref().unwrap().info();
    some_payload
        .commit_free_text_editor(
            some_editor.id,
            &some_editor.request,
            "中文".into(),
            bounds(),
            Some(committed_style.clone()),
        )
        .unwrap();
    let changed_style = FreeTextAppearance {
        font_size: committed_style.font_size + 1.0,
        ..committed_style.clone()
    };
    assert!(
        some_payload
            .commit_free_text_editor(
                some_editor.id,
                &some_editor.request,
                "中文".into(),
                bounds(),
                Some(changed_style),
            )
            .is_err()
    );
    assert_eq!(
        some_payload.session.as_ref().unwrap().bytes().unwrap(),
        some_bytes
    );
    assert_eq!(some_payload.session.as_ref().unwrap().info(), some_info);
    assert!(some_payload.free_text_editor.is_none());
    some_payload
        .commit_free_text_editor(
            some_editor.id,
            &some_editor.request,
            "中文".into(),
            bounds(),
            Some(committed_style),
        )
        .unwrap();
}

#[test]
fn free_text_editor_checkpoint_failure_keeps_committed_memory_and_retry_does_not_reapply() {
    let (mut workspace, directory, _journal_path) = workspace();
    workspace.journal = Some(recovery::Journal::new(
        directory.path().join("missing-parent").join("draft.json"),
    ));
    let editor = workspace
        .begin_canvas_free_text_editor(
            "free-checkpoint-1",
            1,
            0,
            CanvasFreeTextTarget::New {
                bounds: bounds(),
                style: style(),
            },
        )
        .unwrap();
    let view = workspace
        .commit_free_text_editor(
            editor.id,
            &editor.request,
            "checkpoint".into(),
            bounds(),
            None,
        )
        .unwrap();
    assert!(view.warning.is_some());
    assert!(workspace.recovery_fault);
    assert_eq!(
        workspace
            .session
            .as_ref()
            .unwrap()
            .free_texts(0)
            .unwrap()
            .annotations
            .len(),
        1
    );
    let once = workspace.session.as_ref().unwrap().bytes().unwrap();
    let once_info = workspace.session.as_ref().unwrap().info();
    let retry = workspace
        .commit_free_text_editor(
            editor.id,
            &editor.request,
            "checkpoint".into(),
            bounds(),
            None,
        )
        .unwrap();
    assert!(retry.warning.is_some());
    assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), once);
    assert_eq!(workspace.session.as_ref().unwrap().info(), once_info);
    assert_eq!(
        workspace
            .session
            .as_ref()
            .unwrap()
            .free_texts(0)
            .unwrap()
            .annotations
            .len(),
        1
    );
}

#[test]
fn free_text_editor_unknown_owned_style_rejects_reflow_but_allows_pure_move() {
    let (mut workspace, _directory) = workspace_with_mutated_existing("owned text", |annotation| {
        annotation.remove(b"PDFMasterAppearance");
        annotation.set("DA", lopdf::Object::string_literal("/Unknown 12 Tf 0 g"));
    });
    let original = workspace
        .session
        .as_ref()
        .unwrap()
        .free_texts(0)
        .unwrap()
        .annotations
        .remove(0);
    let before = workspace.session.as_ref().unwrap().bytes().unwrap();
    let before_info = workspace.session.as_ref().unwrap().info();
    let editor = workspace
        .begin_canvas_free_text_editor(
            "free-unknown-001",
            1,
            0,
            CanvasFreeTextTarget::Existing {
                original: original.clone(),
            },
        )
        .unwrap();
    assert_eq!(editor.appearance_style, None);
    assert!(
        workspace
            .commit_free_text_editor(
                editor.id,
                &editor.request,
                "changed text".into(),
                original.bounds,
                None,
            )
            .is_err()
    );
    assert_eq!(workspace.free_text_editor.as_ref().unwrap().id, editor.id);
    assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), before);
    assert_eq!(workspace.session.as_ref().unwrap().info(), before_info);
    assert!(
        workspace
            .commit_free_text_editor(
                editor.id,
                &editor.request,
                original.contents.clone(),
                pdfmaster_core::Rect {
                    width: original.bounds.width - 1.0,
                    ..original.bounds
                },
                None,
            )
            .is_err()
    );
    assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), before);

    workspace
        .cancel_free_text_editor(editor.id, &editor.request)
        .unwrap();
    let editor = workspace
        .begin_canvas_free_text_editor(
            "free-unknown-002",
            1,
            0,
            CanvasFreeTextTarget::Existing {
                original: original.clone(),
            },
        )
        .unwrap();
    let moved = pdfmaster_core::Rect {
        x: original.bounds.x + 5.0,
        ..original.bounds
    };
    workspace
        .commit_free_text_editor(editor.id, &editor.request, original.contents, moved, None)
        .unwrap();
    assert_eq!(
        workspace
            .session
            .as_ref()
            .unwrap()
            .free_texts(0)
            .unwrap()
            .annotations[0]
            .bounds,
        moved
    );
}

#[test]
fn free_text_editor_actual_locked_core_refusal_preserves_draft_and_document() {
    let (mut workspace, _directory) = workspace_with_mutated_existing("locked", |annotation| {
        annotation.set("F", 64i64);
    });
    let original = workspace
        .session
        .as_ref()
        .unwrap()
        .free_texts(0)
        .unwrap()
        .annotations
        .remove(0);
    let editor = workspace
        .begin_canvas_free_text_editor(
            "free-locked-0001",
            1,
            0,
            CanvasFreeTextTarget::Existing {
                original: original.clone(),
            },
        )
        .unwrap();
    let before = workspace.session.as_ref().unwrap().bytes().unwrap();
    let before_info = workspace.session.as_ref().unwrap().info();
    assert!(
        workspace
            .commit_free_text_editor(
                editor.id,
                &editor.request,
                "blocked by flag".into(),
                original.bounds,
                None,
            )
            .is_err()
    );
    assert_eq!(workspace.free_text_editor.as_ref().unwrap().id, editor.id);
    assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), before);
    assert_eq!(workspace.session.as_ref().unwrap().info(), before_info);
}

#[test]
fn free_text_editor_new_whitespace_is_noop_and_preserves_redo_branch() {
    let (mut workspace, _directory, _journal_path) = workspace();
    let session = workspace.session.as_mut().unwrap();
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
    let before = session.bytes().unwrap();
    let before_info = session.info();
    let editor = workspace
        .begin_canvas_free_text_editor(
            "free-blank-00001",
            1,
            0,
            CanvasFreeTextTarget::New {
                bounds: bounds(),
                style: style(),
            },
        )
        .unwrap();
    workspace
        .commit_free_text_editor(editor.id, &editor.request, " \r\n\t".into(), bounds(), None)
        .unwrap();
    assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), before);
    assert_eq!(workspace.session.as_ref().unwrap().info(), before_info);
    assert!(
        workspace
            .session
            .as_ref()
            .unwrap()
            .free_texts(0)
            .unwrap()
            .annotations
            .is_empty()
    );
}

#[test]
fn free_text_editor_wire_rejects_invalid_enums_and_serializes_no_internal_snapshot() {
    let invalid_font = serde_json::json!({
        "font": "system",
        "font_size": 12.0,
        "color": {"r": 0.0, "g": 0.0, "b": 0.0},
        "alignment": "left",
        "line_height": 1.2
    });
    assert!(serde_json::from_value::<FreeTextAppearance>(invalid_font).is_err());
    let invalid_alignment = serde_json::json!({
        "font": "sans",
        "font_size": 12.0,
        "color": {"r": 0.0, "g": 0.0, "b": 0.0},
        "alignment": "justify",
        "line_height": 1.2
    });
    assert!(serde_json::from_value::<FreeTextAppearance>(invalid_alignment).is_err());

    let (mut workspace, _directory, _journal_path) = workspace();
    let editor = workspace
        .begin_canvas_free_text_editor(
            "free-wire-00001",
            1,
            0,
            CanvasFreeTextTarget::New {
                bounds: bounds(),
                style: style(),
            },
        )
        .unwrap();
    let json = serde_json::to_string(&editor).unwrap();
    assert!(!json.contains("snapshot"));
    assert!(!json.contains("document_epoch"));
    assert!(!json.contains("document_id"));
}
