use super::*;

const MAX_SEEN_REQUESTS: usize = 4096;
const MAX_ORIGINAL_BYTES: usize = 1024 * 1024;
const MAX_CONTENTS_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum CanvasNoteTarget {
    Existing {
        annotation_index: usize,
        expected_contents: String,
    },
    New {
        x: f32,
        y: f32,
        page_width: f32,
        page_height: f32,
        rotation: i32,
    },
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum NoteEditorTarget {
    Existing {
        original: pdfmaster_core::TextNote,
    },
    New {
        bounds: pdfmaster_core::Rect,
        color: pdfmaster_core::Color,
    },
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct NoteEditor {
    pub(crate) id: u32,
    pub(crate) page: usize,
    pub(crate) request: String,
    pub(crate) target: NoteEditorTarget,
    #[serde(skip)]
    document_id: u32,
    #[serde(skip)]
    document_epoch: u64,
    #[serde(skip)]
    snapshot: std::sync::Arc<[u8]>,
    #[serde(skip)]
    source_target: CanvasNoteTarget,
}

#[derive(Clone, Debug)]
pub(crate) struct FinishedNote {
    id: u32,
    epoch: u64,
    outcome: FinishedNoteOutcome,
}

#[derive(Clone, Debug)]
enum FinishedNoteOutcome {
    Cancelled,
    Committed(String),
}

// Fresh lifecycle implementation follows test-first from the contract.

impl Workspace {
    pub(super) fn begin_note_editor(
        &mut self,
        page: usize,
        annotation_index: usize,
        expected_contents: &str,
    ) -> Reply<NoteEditor> {
        let next = self
            .next_editor_id
            .checked_add(1)
            .ok_or("便笺会话编号已耗尽，请安全保存后重启。")?;
        let request = format!("legacy-note-{next:08x}");
        self.begin_canvas_note_editor(
            &request,
            self.active_document_id,
            page,
            CanvasNoteTarget::Existing {
                annotation_index,
                expected_contents: expected_contents.to_owned(),
            },
        )
    }

    pub(super) fn begin_canvas_note_editor(
        &mut self,
        request: &str,
        document_id: u32,
        page: usize,
        target: CanvasNoteTarget,
    ) -> Reply<NoteEditor> {
        Self::validate_note_request(request)?;
        if document_id != self.active_document_id {
            return Err("便笺请求不属于当前文档，请同步状态。".into());
        }
        let wire_target = self.validate_note_target(page, &target)?;

        if let Some(active) = &self.note_editor {
            if active.request == request
                && active.document_id == document_id
                && active.page == page
                && active.source_target == target
            {
                return Ok(active.clone());
            }
            return Err("已有其他便笺编辑正在进行，未替换当前内容。".into());
        }
        if self.seen_note_requests.contains(request) {
            return Err("该便笺请求已结束或取消，未重新开始。".into());
        }
        if self.seen_note_requests.len() >= MAX_SEEN_REQUESTS {
            return Err("便笺恢复请求预算已用尽，请先保存文档并正常重启。".into());
        }
        self.ensure_editor_idle()?;
        let snapshot: std::sync::Arc<[u8]> = self
            .session
            .as_ref()
            .ok_or("请先打开 PDF")?
            .bytes()
            .map_err(|error| error.to_string())?
            .into();
        let id = self
            .next_editor_id
            .checked_add(1)
            .ok_or("便笺会话编号已耗尽，请安全保存后重启。")?;
        let editor = NoteEditor {
            id,
            page,
            request: request.to_owned(),
            target: wire_target,
            document_id,
            document_epoch: self.document_epoch,
            snapshot,
            source_target: target,
        };
        self.seen_note_requests.insert(request.to_owned());
        self.next_editor_id = id;
        self.note_editor = Some(editor.clone());
        Ok(editor)
    }

    pub(super) fn cancel_canvas_note_request(&mut self, request: &str) -> Reply<DocumentView> {
        Self::validate_note_request(request)?;
        if !self.seen_note_requests.contains(request)
            && self.seen_note_requests.len() >= MAX_SEEN_REQUESTS
        {
            return Err("便笺恢复请求预算已用尽，请先保存文档并正常重启。".into());
        }
        self.seen_note_requests.insert(request.to_owned());
        if let Some(editor) = &self.note_editor
            && editor.request == request
        {
            return self.cancel_note_editor(editor.id);
        }
        self.view()
    }

    pub(super) fn commit_note_editor(&mut self, id: u32, contents: String) -> Reply<DocumentView> {
        if self.note_editor.is_none() {
            if let Some(finished) = &self.finished_note
                && finished.id == id
                && finished.epoch == self.document_epoch
                && matches!(&finished.outcome, FinishedNoteOutcome::Committed(saved) if saved == &contents)
            {
                return self.view();
            }
            return Err("便笺会话已改变，请同步状态。".into());
        }
        let editor = self
            .note_editor
            .as_ref()
            .filter(|editor| editor.id == id)
            .ok_or("便笺会话已改变，请同步状态。")?
            .clone();
        let session = self.session.as_mut().ok_or("请先打开 PDF")?;
        if editor.document_id != self.active_document_id
            || editor.document_epoch != self.document_epoch
            || session
                .bytes()
                .map_err(|error| error.to_string())?
                .as_slice()
                != editor.snapshot.as_ref()
        {
            return Err("便笺编辑期间文档已改变，未提交内容。".into());
        }

        let operation = match &editor.target {
            NoteEditorTarget::Existing { original } if contents == original.contents => None,
            NoteEditorTarget::New { .. } if contents.trim().is_empty() => None,
            _ if contents.len() > MAX_CONTENTS_BYTES => {
                return Err("便笺正文不得超过 64KiB。".into());
            }
            NoteEditorTarget::Existing { original } if contents.trim().is_empty() => {
                Some(Edit::DeleteNote {
                    page: editor.page,
                    annotation_index: original.annotation_index,
                    expected_contents: original.contents.clone(),
                })
            }
            NoteEditorTarget::Existing { original } => Some(Edit::UpdateNote {
                page: editor.page,
                annotation_index: original.annotation_index,
                expected_contents: original.contents.clone(),
                contents: contents.clone(),
            }),
            NoteEditorTarget::New { bounds, color } => Some(Edit::AddNote {
                page: editor.page,
                bounds: *bounds,
                contents: contents.clone(),
                color: *color,
            }),
        };

        let mutated = operation.is_some();
        if let Some(operation) = operation {
            session
                .apply(operation)
                .map_err(|error| error.to_string())?;
        }
        self.note_editor = None;
        self.finished_note = Some(FinishedNote {
            id,
            epoch: self.document_epoch,
            outcome: FinishedNoteOutcome::Committed(contents),
        });
        if mutated {
            self.checkpoint()
        } else {
            self.view()
        }
    }

    pub(super) fn cancel_note_editor(&mut self, id: u32) -> Reply<DocumentView> {
        if self.note_editor.is_none() {
            if self
                .finished_note
                .as_ref()
                .is_some_and(|finished| finished.id == id && finished.epoch == self.document_epoch)
            {
                return self.view();
            }
            return Err("便笺会话已改变，未取消新内容。".into());
        }
        if self
            .note_editor
            .as_ref()
            .is_none_or(|editor| editor.id != id)
        {
            return Err("便笺会话已改变，未取消新内容。".into());
        }
        self.note_editor = None;
        self.finished_note = Some(FinishedNote {
            id,
            epoch: self.document_epoch,
            outcome: FinishedNoteOutcome::Cancelled,
        });
        self.view()
    }

    fn validate_note_request(request: &str) -> Reply<()> {
        if !(8..=64).contains(&request.len())
            || !request
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err("便笺请求身份无效。".into());
        }
        Ok(())
    }

    fn validate_note_target(
        &self,
        page: usize,
        target: &CanvasNoteTarget,
    ) -> Reply<NoteEditorTarget> {
        let session = self.session.as_ref().ok_or("请先打开 PDF")?;
        let info = session.info();
        let page_info = info.pages.get(page).ok_or("便笺页码无效。")?;
        match target {
            CanvasNoteTarget::Existing {
                annotation_index,
                expected_contents,
            } => {
                if expected_contents.len() > MAX_ORIGINAL_BYTES {
                    return Err("便笺原始内容超出预算。".into());
                }
                let original = workspace_notes(self, page)?
                    .notes
                    .into_iter()
                    .find(|note| note.annotation_index == *annotation_index)
                    .filter(|note| note.contents == *expected_contents)
                    .ok_or("便笺已改变或不受支持，请刷新后重试。")?;
                Ok(NoteEditorTarget::Existing { original })
            }
            CanvasNoteTarget::New {
                x,
                y,
                page_width,
                page_height,
                rotation,
            } => {
                let values = [*x, *y, *page_width, *page_height];
                if values.iter().any(|value| !value.is_finite())
                    || *page_width <= 0.0
                    || *page_height <= 0.0
                    || *page_width != page_info.width
                    || *page_height != page_info.height
                    || rotation.rem_euclid(360) != page_info.rotation.rem_euclid(360)
                    || !matches!(rotation.rem_euclid(360), 0 | 90 | 180 | 270)
                    || !(0.0..=*page_width).contains(x)
                    || !(0.0..=*page_height).contains(y)
                {
                    return Err("便笺位置或页面几何无效。".into());
                }
                let width = page_width.min(24.0);
                let height = page_height.min(24.0);
                let bounds = pdfmaster_core::Rect {
                    x: (x - width / 2.0).clamp(0.0, page_width - width),
                    y: (y - height / 2.0).clamp(0.0, page_height - height),
                    width,
                    height,
                };
                Ok(NoteEditorTarget::New {
                    bounds,
                    color: pdfmaster_core::Color {
                        r: 1.0,
                        g: 0.85,
                        b: 0.2,
                    },
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{Dictionary, Document as PdfDocument, Object, Stream, dictionary, text_string};

    fn pdf_with_note(contents: &str, threaded_reply: bool) -> Vec<u8> {
        let mut document = PdfDocument::with_version("1.7");
        let pages_id = document.new_object_id();
        let page_count = if threaded_reply { 2 } else { 1 };
        let page_ids = (0..page_count)
            .map(|_| {
                let stream = document.add_object(Stream::new(Dictionary::new(), Vec::new()));
                document.add_object(dictionary! {
                    "Type" => "Page",
                    "Parent" => pages_id,
                    "MediaBox" => vec![0.into(), 0.into(), 100.into(), 80.into()],
                    "Resources" => Dictionary::new(),
                    "Contents" => stream,
                })
            })
            .collect::<Vec<_>>();
        document.objects.insert(
            pages_id,
            dictionary! {
                "Type" => "Pages",
                "Kids" => page_ids.iter().copied().map(Object::Reference).collect::<Vec<_>>(),
                "Count" => page_count as i64,
            }
            .into(),
        );
        let catalog = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog);

        let note = document.add_object(dictionary! {
            "Type" => "Annot",
            "Subtype" => "Text",
            "Rect" => vec![10.into(), 10.into(), 30.into(), 30.into()],
            "Contents" => text_string(contents),
        });
        document
            .get_object_mut(page_ids[0])
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Annots", vec![Object::Reference(note)]);
        if threaded_reply {
            let reply = dictionary! {
                "Type" => "Annot",
                "Subtype" => "Text",
                "Rect" => vec![40.into(), 10.into(), 60.into(), 30.into()],
                "Contents" => text_string("reply"),
                "IRT" => note,
            };
            document
                .get_object_mut(page_ids[1])
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("Annots", vec![Object::Dictionary(reply)]);
        }
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).unwrap();
        bytes
    }

    fn workspace_from_pdf(bytes: &[u8]) -> Workspace {
        Workspace {
            active_document_id: 7,
            session: Some(Session::open(bytes).unwrap()),
            ..Default::default()
        }
    }

    fn workspace(with_note: bool) -> Workspace {
        let mut session = Session::blank(100., 80., 1).unwrap();
        if with_note {
            session
                .apply(Edit::AddNote {
                    page: 0,
                    bounds: pdfmaster_core::Rect {
                        x: 10.,
                        y: 10.,
                        width: 20.,
                        height: 20.,
                    },
                    contents: "原便笺 🦀".into(),
                    color: pdfmaster_core::Color {
                        r: 1.,
                        g: 0.85,
                        b: 0.2,
                    },
                })
                .unwrap();
            session.mark_saved();
        }
        Workspace {
            active_document_id: 7,
            session: Some(session),
            ..Default::default()
        }
    }

    fn new_target(x: f32, y: f32) -> CanvasNoteTarget {
        CanvasNoteTarget::New {
            x,
            y,
            page_width: 100.,
            page_height: 80.,
            rotation: 0,
        }
    }

    fn existing_target(contents: &str) -> CanvasNoteTarget {
        CanvasNoteTarget::Existing {
            annotation_index: 0,
            expected_contents: contents.into(),
        }
    }

    #[test]
    fn new_target_is_explicit_and_begin_does_not_write_pdf() {
        let mut workspace = workspace(false);
        let bytes = workspace.session.as_ref().unwrap().bytes().unwrap();
        let editor = workspace
            .begin_canvas_note_editor(
                "00112233445566778899aabbccddeeff",
                7,
                0,
                new_target(50., 40.),
            )
            .unwrap();
        let wire = serde_json::to_value(editor).unwrap();
        assert_eq!(wire["target"]["kind"], "new");
        assert!(wire["target"].get("original").is_none());
        assert_eq!(wire["target"]["bounds"]["x"], 38.);
        assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), bytes);
        assert!(!workspace.dirty());
    }

    #[test]
    fn request_tombstone_replay_and_active_payload_binding_are_strict() {
        let mut workspace = workspace(false);
        let cancelled = "cancel-before-begin";
        workspace.cancel_canvas_note_request(cancelled).unwrap();
        assert!(
            workspace
                .begin_canvas_note_editor(cancelled, 7, 0, new_target(10., 10.))
                .is_err()
        );

        let request = "00112233445566778899aabbccddeeff";
        let first = workspace
            .begin_canvas_note_editor(request, 7, 0, new_target(10., 10.))
            .unwrap();
        assert_eq!(
            workspace
                .begin_canvas_note_editor(request, 7, 0, new_target(10., 10.))
                .unwrap()
                .id,
            first.id
        );
        assert!(
            workspace
                .begin_canvas_note_editor(request, 7, 0, new_target(11., 10.))
                .is_err()
        );
        workspace.cancel_canvas_note_request(request).unwrap();
        assert!(
            workspace
                .begin_canvas_note_editor(request, 7, 0, new_target(10., 10.))
                .is_err()
        );
    }

    #[test]
    fn begin_validates_document_epoch_and_request_boundaries_without_losing_the_guard() {
        let mut workspace = workspace(false);
        let bytes = workspace.session.as_ref().unwrap().bytes().unwrap();
        let info = workspace.session.as_ref().unwrap().info();
        let seen = workspace.seen_note_requests.clone();
        assert_eq!(
            workspace
                .begin_canvas_note_editor("wrong-document", 8, 0, new_target(10., 10.))
                .unwrap_err(),
            "便笺请求不属于当前文档，请同步状态。"
        );
        assert!(workspace.note_editor.is_none());
        assert_eq!(workspace.seen_note_requests, seen);
        assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), bytes);
        assert_eq!(workspace.session.as_ref().unwrap().info(), info);

        for invalid in [
            "1234567".to_owned(),
            "请求身份八字".to_owned(),
            "a".repeat(65),
        ] {
            assert_eq!(
                workspace
                    .begin_canvas_note_editor(&invalid, 7, 0, new_target(10., 10.))
                    .unwrap_err(),
                "便笺请求身份无效。"
            );
            assert!(workspace.note_editor.is_none());
        }
        for accepted in ["a".repeat(8), "b".repeat(64)] {
            let editor = workspace
                .begin_canvas_note_editor(&accepted, 7, 0, new_target(10., 10.))
                .unwrap();
            assert_eq!(editor.request, accepted);
            workspace.cancel_note_editor(editor.id).unwrap();
        }

        let editor = workspace
            .begin_canvas_note_editor("epoch-mismatch", 7, 0, new_target(10., 10.))
            .unwrap();
        let epoch = editor.document_epoch;
        workspace.document_epoch = epoch + 1;
        assert_eq!(
            workspace
                .commit_note_editor(editor.id, "未提交".into())
                .err()
                .unwrap(),
            "便笺编辑期间文档已改变，未提交内容。"
        );
        let active = workspace.note_editor.as_ref().unwrap();
        assert_eq!(active.id, editor.id);
        assert_eq!(active.request, editor.request);
        assert_eq!(active.snapshot, editor.snapshot);
        assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), bytes);
        assert_eq!(workspace.session.as_ref().unwrap().info(), info);
    }

    #[test]
    fn new_empty_is_noop_and_unicode_add_retries_exactly_once() {
        let mut workspace = workspace(false);
        let before = workspace.session.as_ref().unwrap().bytes().unwrap();
        let empty = workspace
            .begin_canvas_note_editor("new-empty-request", 7, 0, new_target(1., 1.))
            .unwrap();
        workspace
            .commit_note_editor(empty.id, String::new())
            .unwrap();
        assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), before);
        assert!(workspace_notes(&workspace, 0).unwrap().notes.is_empty());

        let added = workspace
            .begin_canvas_note_editor("new-unicode-request", 7, 0, new_target(99., 79.))
            .unwrap();
        workspace
            .commit_note_editor(added.id, "新增 🦀\n正文".into())
            .unwrap();
        let once = workspace.session.as_ref().unwrap().bytes().unwrap();
        workspace
            .commit_note_editor(added.id, "新增 🦀\n正文".into())
            .unwrap();
        assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), once);
        let notes = workspace_notes(&workspace, 0).unwrap().notes;
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].contents, "新增 🦀\n正文");
        assert_eq!(notes[0].bounds.x, 76.);
        assert_eq!(notes[0].bounds.y, 56.);
    }

    #[test]
    fn replacement_lifecycle_replays_only_the_exact_terminal_payload_and_preserves_undo() {
        let mut workspace = workspace(false);
        let before = workspace.session.as_ref().unwrap().bytes().unwrap();
        let editor = workspace
            .begin_canvas_note_editor("replacement-terminal-payload", 7, 0, new_target(40., 30.))
            .expect("a valid request must acquire the single native owner");

        workspace
            .commit_note_editor(editor.id, "原始终态 🦀".into())
            .expect("the first exact payload must commit");
        let committed = workspace.session.as_ref().unwrap().bytes().unwrap();
        assert_ne!(committed, before);
        assert_eq!(workspace_notes(&workspace, 0).unwrap().notes.len(), 1);

        workspace
            .commit_note_editor(editor.id, "原始终态 🦀".into())
            .expect("a lost success reply may replay the exact payload");
        assert_eq!(
            workspace.session.as_ref().unwrap().bytes().unwrap(),
            committed
        );
        assert!(
            workspace
                .commit_note_editor(editor.id, "后来可见文本".into())
                .is_err(),
            "a changed visible value must not reinterpret the finished operation"
        );
        assert_eq!(
            workspace.session.as_ref().unwrap().bytes().unwrap(),
            committed
        );

        workspace.session.as_mut().unwrap().undo().unwrap();
        assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), before);
        assert!(workspace.session.as_mut().unwrap().undo().is_err());
    }

    #[test]
    fn replacement_lifecycle_terminal_identity_cannot_clear_a_newer_owner() {
        let mut workspace = workspace(false);
        let first = workspace
            .begin_canvas_note_editor("replacement-first-owner", 7, 0, new_target(20., 20.))
            .unwrap();
        workspace.cancel_note_editor(first.id).unwrap();

        let second = workspace
            .begin_canvas_note_editor("replacement-second-owner", 7, 0, new_target(30., 30.))
            .unwrap();
        assert!(workspace.cancel_note_editor(first.id).is_err());
        assert!(
            workspace
                .commit_note_editor(first.id, String::new())
                .is_err()
        );
        assert_eq!(workspace.note_editor.as_ref().unwrap().id, second.id);
        workspace.cancel_note_editor(second.id).unwrap();
    }

    #[test]
    fn cancelled_id_cannot_be_reinterpreted_as_an_empty_commit() {
        let mut workspace = workspace(false);
        let editor = workspace
            .begin_canvas_note_editor("cancelled-new-note", 7, 0, new_target(20., 20.))
            .unwrap();
        workspace.cancel_note_editor(editor.id).unwrap();
        assert!(
            workspace
                .commit_note_editor(editor.id, String::new())
                .is_err()
        );
        assert!(workspace_notes(&workspace, 0).unwrap().notes.is_empty());
    }

    #[test]
    fn exact_one_mib_existing_original_closes_unchanged_before_replacement_budget() {
        let contents = "a".repeat(MAX_ORIGINAL_BYTES);
        let source = pdf_with_note(&contents, false);
        let mut workspace = workspace_from_pdf(&source);
        let editor = workspace
            .begin_canvas_note_editor("exact-one-mib-original", 7, 0, existing_target(&contents))
            .unwrap();
        workspace.commit_note_editor(editor.id, contents).expect(
            "an exact unchanged 1MiB original is a no-op before the 64KiB replacement gate",
        );
        assert!(workspace.note_editor.is_none());
        assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), source);
        assert!(!workspace.dirty());
    }

    #[test]
    fn exact_64k_new_and_replacement_are_accepted_and_over_limit_keeps_the_guard() {
        let exact = "z".repeat(MAX_CONTENTS_BYTES);

        let mut added = workspace(false);
        let add = added
            .begin_canvas_note_editor("exact-64k-new", 7, 0, new_target(20., 20.))
            .unwrap();
        added.commit_note_editor(add.id, exact.clone()).unwrap();
        assert_eq!(workspace_notes(&added, 0).unwrap().notes[0].contents, exact);

        let mut replaced = workspace(true);
        let replace = replaced
            .begin_canvas_note_editor("exact-64k-replacement", 7, 0, existing_target("原便笺 🦀"))
            .unwrap();
        replaced
            .commit_note_editor(replace.id, "r".repeat(MAX_CONTENTS_BYTES))
            .unwrap();
        assert_eq!(
            workspace_notes(&replaced, 0).unwrap().notes[0]
                .contents
                .len(),
            MAX_CONTENTS_BYTES
        );

        let mut rejected = workspace(true);
        let reject = rejected
            .begin_canvas_note_editor("over-64k-replacement", 7, 0, existing_target("原便笺 🦀"))
            .unwrap();
        let bytes = rejected.session.as_ref().unwrap().bytes().unwrap();
        let info = rejected.session.as_ref().unwrap().info();
        assert_eq!(
            rejected
                .commit_note_editor(reject.id, "x".repeat(MAX_CONTENTS_BYTES + 1))
                .err()
                .unwrap(),
            "便笺正文不得超过 64KiB。"
        );
        let active = rejected.note_editor.as_ref().unwrap();
        assert_eq!(active.id, reject.id);
        assert_eq!(active.request, reject.request);
        assert_eq!(active.snapshot, reject.snapshot);
        assert_eq!(rejected.session.as_ref().unwrap().bytes().unwrap(), bytes);
        assert_eq!(rejected.session.as_ref().unwrap().info(), info);
    }

    #[test]
    fn mismatched_request_cancel_tombstones_only_itself() {
        let mut workspace = workspace(false);
        let editor = workspace
            .begin_canvas_note_editor("current-note-owner", 7, 0, new_target(20., 20.))
            .unwrap();
        workspace
            .cancel_canvas_note_request("different-request")
            .unwrap();
        assert_eq!(workspace.note_editor.as_ref().unwrap().id, editor.id);
        assert!(workspace.seen_note_requests.contains("different-request"));
        workspace.cancel_note_editor(editor.id).unwrap();
    }

    #[test]
    fn existing_unchanged_updates_and_empty_deletes_are_distinct_terminal_paths() {
        let mut workspace = workspace(true);
        let before = workspace.session.as_ref().unwrap().bytes().unwrap();
        let unchanged = workspace
            .begin_canvas_note_editor("existing-unchanged", 7, 0, existing_target("原便笺 🦀"))
            .unwrap();
        workspace
            .commit_note_editor(unchanged.id, "原便笺 🦀".into())
            .unwrap();
        assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), before);

        let update = workspace
            .begin_canvas_note_editor("existing-update", 7, 0, existing_target("原便笺 🦀"))
            .unwrap();
        workspace
            .commit_note_editor(update.id, "修改后的正文".into())
            .unwrap();
        assert_eq!(
            workspace_notes(&workspace, 0).unwrap().notes[0].contents,
            "修改后的正文"
        );

        let delete = workspace
            .begin_canvas_note_editor("existing-delete", 7, 0, existing_target("修改后的正文"))
            .unwrap();
        workspace
            .commit_note_editor(delete.id, String::new())
            .unwrap();
        assert!(workspace_notes(&workspace, 0).unwrap().notes.is_empty());
    }

    #[test]
    fn threaded_core_rejection_preserves_owner_snapshot_history_and_checkpoint() {
        let source = pdf_with_note("thread root", true);
        let mut session = Session::open(&source).unwrap();
        session
            .apply(Edit::Rotate {
                page: 0,
                degrees: 90,
            })
            .unwrap();
        let guarded_bytes = session.bytes().unwrap();
        session
            .apply(Edit::Rotate {
                page: 1,
                degrees: 90,
            })
            .unwrap();
        let redo_bytes = session.bytes().unwrap();
        session.undo().unwrap();
        assert_eq!(session.bytes().unwrap(), guarded_bytes);
        assert!(session.info().can_undo && session.info().can_redo && session.info().dirty);

        let directory = tempfile::tempdir().unwrap();
        let checkpoint = directory.path().join("draft.json");
        let mut workspace = Workspace {
            active_document_id: 7,
            session: Some(session),
            journal: Some(recovery::Journal::new(checkpoint.clone())),
            ..Default::default()
        };
        workspace.persist().unwrap();
        let checkpoint_before = std::fs::read(&checkpoint).unwrap();
        let editor = workspace
            .begin_canvas_note_editor(
                "threaded-core-rejection",
                7,
                0,
                existing_target("thread root"),
            )
            .unwrap();
        assert_eq!(editor.snapshot.as_ref(), guarded_bytes.as_slice());
        let info = workspace.session.as_ref().unwrap().info();
        let dirty = workspace.dirty();

        assert_eq!(
            workspace
                .commit_note_editor(editor.id, "changed".into())
                .err()
                .unwrap(),
            "invalid operation: threaded annotation lifecycle is not yet supported"
        );
        let active = workspace.note_editor.as_ref().unwrap();
        assert_eq!(active.id, editor.id);
        assert_eq!(active.request, editor.request);
        assert_eq!(active.page, editor.page);
        assert_eq!(active.source_target, editor.source_target);
        assert_eq!(active.snapshot, editor.snapshot);
        match &active.target {
            NoteEditorTarget::Existing { original } => {
                assert_eq!(original.contents, "thread root")
            }
            NoteEditorTarget::New { .. } => panic!("fixture must retain the existing note owner"),
        }
        assert_eq!(
            workspace.session.as_ref().unwrap().bytes().unwrap(),
            guarded_bytes
        );
        assert_eq!(workspace.session.as_ref().unwrap().info(), info);
        assert_eq!(workspace.dirty(), dirty);
        assert!(workspace.finished_note.is_none());
        assert_eq!(std::fs::read(&checkpoint).unwrap(), checkpoint_before);

        let session = workspace.session.as_mut().unwrap();
        session.redo().unwrap();
        assert_eq!(session.bytes().unwrap(), redo_bytes);
        session.undo().unwrap();
        assert_eq!(session.bytes().unwrap(), guarded_bytes);
        session.undo().unwrap();
        assert_eq!(session.bytes().unwrap(), source);
    }

    #[test]
    fn snapshot_conflict_and_identity_budget_preserve_the_owner() {
        let mut workspace = workspace(true);
        assert!(
            workspace
                .begin_canvas_note_editor("short", 7, 0, existing_target("原便笺 🦀"))
                .is_err()
        );
        let editor = workspace
            .begin_canvas_note_editor("snapshot-conflict", 7, 0, existing_target("原便笺 🦀"))
            .unwrap();
        workspace
            .session
            .as_mut()
            .unwrap()
            .apply(Edit::UpdateNote {
                page: 0,
                annotation_index: 0,
                expected_contents: "原便笺 🦀".into(),
                contents: "外部变化".into(),
            })
            .unwrap();
        let bytes = workspace.session.as_ref().unwrap().bytes().unwrap();
        assert!(
            workspace
                .commit_note_editor(editor.id, "不能覆盖".into())
                .is_err()
        );
        assert_eq!(workspace.note_editor.as_ref().unwrap().id, editor.id);
        assert_eq!(workspace.session.as_ref().unwrap().bytes().unwrap(), bytes);

        workspace.cancel_note_editor(editor.id).unwrap();
        workspace.seen_note_requests.clear();
        for index in 0..MAX_SEEN_REQUESTS {
            workspace
                .seen_note_requests
                .insert(format!("request-{index:04}"));
        }
        assert!(
            workspace
                .cancel_canvas_note_request("request-over-budget")
                .is_err()
        );
        assert_eq!(workspace.seen_note_requests.len(), MAX_SEEN_REQUESTS);
    }
}
