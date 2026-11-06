use super::*;
use crate::free_text_editor::{CanvasFreeTextTarget, FreeTextAppearance, FreeTextFont};

fn bounds() -> pdfmaster_core::Rect {
    pdfmaster_core::Rect {
        x: 20.0,
        y: 20.0,
        width: 120.0,
        height: 40.0,
    }
}

fn core_style() -> pdfmaster_core::FreeTextAppearanceStyle {
    pdfmaster_core::FreeTextAppearanceStyle {
        font: pdfmaster_core::text_appearance::BundledFont::Sans,
        font_size: 16.0,
        color: pdfmaster_core::Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
        },
        alignment: pdfmaster_core::TextAlignment::Left,
        line_height: 1.2,
    }
}

fn appearance() -> FreeTextAppearance {
    FreeTextAppearance {
        font: FreeTextFont::Sans,
        font_size: 16.0,
        color: pdfmaster_core::Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
        },
        alignment: pdfmaster_core::TextAlignment::Left,
        line_height: 1.2,
    }
}

fn existing_workspace() -> (Workspace, tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let journal_path = directory.path().join("draft.json");
    let journal = recovery::Journal::new(journal_path.clone());
    let mut session = Session::blank(180.0, 140.0, 1).unwrap();
    session
        .create_free_text(
            0,
            &pdfmaster_core::FreeTextDraft {
                contents: "selected",
                bounds: bounds(),
                style: core_style(),
            },
        )
        .unwrap();
    session.mark_saved();
    journal
        .save("preview.pdf", &session.bytes().unwrap(), false)
        .unwrap();
    let workspace = Workspace {
        session: Some(session),
        active_document_id: 1,
        next_document_id: 1,
        document_epoch: 7,
        journal: Some(journal),
        ..Default::default()
    };
    (workspace, directory, journal_path)
}

fn begun_new_workspace(
    request: &str,
) -> (
    Workspace,
    crate::free_text_editor::FreeTextEditor,
    tempfile::TempDir,
    std::path::PathBuf,
) {
    let (mut workspace, directory, journal_path) = existing_workspace();
    let editor = workspace
        .begin_canvas_free_text_editor(
            request,
            1,
            0,
            CanvasFreeTextTarget::New {
                bounds: bounds(),
                style: appearance(),
            },
        )
        .unwrap();
    (workspace, editor, directory, journal_path)
}

fn assert_preview_rejected_without_state_change(
    workspace: &Workspace,
    journal_path: &std::path::Path,
    id: u32,
    request: &str,
) {
    let before_bytes = workspace.session.as_ref().unwrap().bytes().unwrap();
    let before_info = workspace.session.as_ref().unwrap().info();
    let before_editor = workspace.free_text_editor.clone();
    let before_journal = std::fs::read(journal_path).unwrap();
    assert!(
        workspace
            .free_text_editor_background(id, request, 1.0)
            .is_err()
    );
    assert_eq!(
        workspace.session.as_ref().unwrap().bytes().unwrap(),
        before_bytes
    );
    assert_eq!(workspace.session.as_ref().unwrap().info(), before_info);
    assert_eq!(workspace.free_text_editor, before_editor);
    assert_eq!(std::fs::read(journal_path).unwrap(), before_journal);
}

#[test]
fn existing_owner_uses_core_background_and_preserves_workspace_and_journal() {
    let (mut workspace, _directory, journal_path) = existing_workspace();
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
            "preview-existing-0001",
            1,
            0,
            CanvasFreeTextTarget::Existing {
                original: original.clone(),
            },
        )
        .unwrap();
    let expected = workspace
        .session
        .as_ref()
        .unwrap()
        .render_free_text_background_png(&original, 1.0)
        .unwrap();
    let normal = workspace
        .session
        .as_ref()
        .unwrap()
        .render_png(0, 1.0)
        .unwrap();
    assert_ne!(expected, normal);
    let before_bytes = workspace.session.as_ref().unwrap().bytes().unwrap();
    let before_info = workspace.session.as_ref().unwrap().info();
    let before_editor = workspace.free_text_editor.clone();
    let before_journal = std::fs::read(&journal_path).unwrap();

    assert_eq!(
        workspace
            .free_text_editor_background(editor.id, &editor.request, 1.0)
            .unwrap(),
        expected
    );
    assert_eq!(
        workspace.session.as_ref().unwrap().bytes().unwrap(),
        before_bytes
    );
    assert_eq!(workspace.session.as_ref().unwrap().info(), before_info);
    assert_eq!(workspace.free_text_editor, before_editor);
    assert_eq!(std::fs::read(&journal_path).unwrap(), before_journal);
}

#[test]
fn new_owner_uses_normal_rendering_without_mutation() {
    let (mut workspace, _directory, journal_path) = existing_workspace();
    let editor = workspace
        .begin_canvas_free_text_editor(
            "preview-new-00000001",
            1,
            0,
            CanvasFreeTextTarget::New {
                bounds: bounds(),
                style: appearance(),
            },
        )
        .unwrap();
    let expected = workspace
        .session
        .as_ref()
        .unwrap()
        .render_png(0, 1.0)
        .unwrap();
    let before_bytes = workspace.session.as_ref().unwrap().bytes().unwrap();
    let before_info = workspace.session.as_ref().unwrap().info();
    let before_editor = workspace.free_text_editor.clone();
    let before_journal = std::fs::read(&journal_path).unwrap();

    assert_eq!(
        workspace
            .free_text_editor_background(editor.id, &editor.request, 1.0)
            .unwrap(),
        expected
    );
    assert_eq!(
        workspace.session.as_ref().unwrap().bytes().unwrap(),
        before_bytes
    );
    assert_eq!(workspace.session.as_ref().unwrap().info(), before_info);
    assert_eq!(workspace.free_text_editor, before_editor);
    assert_eq!(std::fs::read(&journal_path).unwrap(), before_journal);
}

#[test]
fn active_owner_rejects_invalid_request_format_without_mutation() {
    let (workspace, editor, _directory, journal_path) =
        begun_new_workspace("preview-guard-request-01");
    assert_preview_rejected_without_state_change(&workspace, &journal_path, editor.id, "bad!");
}

#[test]
fn active_owner_rejects_wrong_id_without_mutation() {
    let (workspace, editor, _directory, journal_path) =
        begun_new_workspace("preview-guard-id-000001");
    assert_preview_rejected_without_state_change(
        &workspace,
        &journal_path,
        editor.id + 1,
        &editor.request,
    );
}

#[test]
fn active_owner_rejects_wrong_valid_request_without_mutation() {
    let (workspace, editor, _directory, journal_path) =
        begun_new_workspace("preview-guard-request-01");
    assert_preview_rejected_without_state_change(
        &workspace,
        &journal_path,
        editor.id,
        "preview-guard-request-02",
    );
}

#[test]
fn active_owner_rejects_document_drift_without_mutation() {
    let (mut document, editor, _directory, journal_path) =
        begun_new_workspace("preview-guard-document");
    document.active_document_id += 1;
    assert_preview_rejected_without_state_change(
        &document,
        &journal_path,
        editor.id,
        &editor.request,
    );
}

#[test]
fn active_owner_rejects_epoch_drift_without_mutation() {
    let (mut epoch, editor, _directory, journal_path) =
        begun_new_workspace("preview-guard-epoch-001");
    epoch.document_epoch += 1;
    assert_preview_rejected_without_state_change(&epoch, &journal_path, editor.id, &editor.request);
}

#[test]
fn active_owner_rejects_bytes_drift_without_mutation() {
    let (mut bytes, editor, _directory, journal_path) =
        begun_new_workspace("preview-guard-bytes-001");
    bytes
        .session
        .as_mut()
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    assert_preview_rejected_without_state_change(&bytes, &journal_path, editor.id, &editor.request);
}

#[test]
fn active_owner_rejects_conflicting_export_owner() {
    let (mut export, editor, _directory, journal_path) =
        begun_new_workspace("preview-guard-export01");
    export.export_reservation = Some(77);
    assert_preview_rejected_without_state_change(
        &export,
        &journal_path,
        editor.id,
        &editor.request,
    );
}

#[test]
fn active_owner_rejects_conflicting_note_owner() {
    let (mut note, editor, _directory, journal_path) =
        begun_new_workspace("preview-guard-note-001");
    let (mut note_source, _source_directory, _) = existing_workspace();
    note_source
        .begin_canvas_note_editor(
            "preview-note-owner-01",
            1,
            0,
            note_editor::CanvasNoteTarget::New {
                x: 10.0,
                y: 10.0,
                page_width: 180.0,
                page_height: 140.0,
                rotation: 0,
            },
        )
        .unwrap();
    note.note_editor = note_source.note_editor.take();
    assert_preview_rejected_without_state_change(&note, &journal_path, editor.id, &editor.request);
}

#[test]
fn active_owner_rejects_conflicting_ink_owner() {
    let (mut ink, editor, _directory, journal_path) = begun_new_workspace("preview-guard-ink-0001");
    ink.ink_editor = Some(InkEditor {
        id: 88,
        request: "preview-ink-owner-01".into(),
        page: 0,
        color: pdfmaster_core::Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
        },
        line_width: 2.0,
        document_epoch: ink.document_epoch,
        snapshot: ink.session.as_ref().unwrap().bytes().unwrap(),
    });
    assert_preview_rejected_without_state_change(&ink, &journal_path, editor.id, &editor.request);
}

#[test]
fn cancelled_and_committed_terminal_receipts_never_reopen_preview() {
    let (mut cancelled, editor, _directory, journal_path) =
        begun_new_workspace("preview-terminal-cancel");
    cancelled
        .cancel_free_text_editor(editor.id, &editor.request)
        .unwrap();
    assert!(cancelled.free_text_editor.is_none());
    assert!(cancelled.finished_free_text.is_some());
    let before_bytes = cancelled.session.as_ref().unwrap().bytes().unwrap();
    let before_info = cancelled.session.as_ref().unwrap().info();
    let before_terminal = format!("{:?}", cancelled.finished_free_text);
    let before_journal = std::fs::read(&journal_path).unwrap();
    assert!(
        cancelled
            .free_text_editor_background(editor.id, &editor.request, 1.0)
            .is_err()
    );
    assert_eq!(
        cancelled.session.as_ref().unwrap().bytes().unwrap(),
        before_bytes
    );
    assert_eq!(cancelled.session.as_ref().unwrap().info(), before_info);
    assert_eq!(
        format!("{:?}", cancelled.finished_free_text),
        before_terminal
    );
    assert_eq!(std::fs::read(&journal_path).unwrap(), before_journal);

    let (mut committed, editor, _directory, journal_path) =
        begun_new_workspace("preview-terminal-commit");
    committed
        .commit_free_text_editor(editor.id, &editor.request, "saved".into(), bounds(), None)
        .unwrap();
    assert!(committed.free_text_editor.is_none());
    assert!(committed.finished_free_text.is_some());
    let before_bytes = committed.session.as_ref().unwrap().bytes().unwrap();
    let before_info = committed.session.as_ref().unwrap().info();
    let before_terminal = format!("{:?}", committed.finished_free_text);
    let before_journal = std::fs::read(&journal_path).unwrap();
    assert!(
        committed
            .free_text_editor_background(editor.id, &editor.request, 1.0)
            .is_err()
    );
    assert_eq!(
        committed.session.as_ref().unwrap().bytes().unwrap(),
        before_bytes
    );
    assert_eq!(committed.session.as_ref().unwrap().info(), before_info);
    assert_eq!(
        format!("{:?}", committed.finished_free_text),
        before_terminal
    );
    assert_eq!(std::fs::read(&journal_path).unwrap(), before_journal);
}

#[test]
fn native_preview_propagates_render_scale_guards_without_mutation() {
    let (workspace, editor, _directory, journal_path) =
        begun_new_workspace("preview-scale-guard-01");
    let before_bytes = workspace.session.as_ref().unwrap().bytes().unwrap();
    let before_info = workspace.session.as_ref().unwrap().info();
    let before_editor = workspace.free_text_editor.clone();
    let before_journal = std::fs::read(&journal_path).unwrap();
    for scale in [0.0, f32::NAN, 17.0] {
        assert!(
            workspace
                .free_text_editor_background(editor.id, &editor.request, scale)
                .is_err()
        );
    }
    assert_eq!(
        workspace.session.as_ref().unwrap().bytes().unwrap(),
        before_bytes
    );
    assert_eq!(workspace.session.as_ref().unwrap().info(), before_info);
    assert_eq!(workspace.free_text_editor, before_editor);
    assert_eq!(std::fs::read(&journal_path).unwrap(), before_journal);
}
