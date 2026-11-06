use super::*;
use pdfmaster_drawing::Point;

#[derive(Clone, Debug)]
pub(super) struct NotePointer {
    pointer: i32,
    start: Point,
    frame: pdfmaster_ui::canvas_notes::Frame,
    source: String,
    document_id: u32,
    page: usize,
    ticket: Option<u64>,
    page_width: f32,
    page_height: f32,
    rotation: i32,
    moved: bool,
}

#[derive(Clone, Debug)]
pub(super) struct NoteRequest {
    pub(super) request: String,
    document_id: u32,
    page: usize,
    ticket: Option<u64>,
    source: String,
    cancelling: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct NoteCommitAttempt {
    id: u32,
    request: String,
    contents: String,
}

impl NoteCommitAttempt {
    fn for_draft(draft: &NoteDraft, contents: String) -> Self {
        Self {
            id: draft.id,
            request: draft.request.clone(),
            contents,
        }
    }

    fn owns(&self, draft: &NoteDraft) -> bool {
        self.id == draft.id && self.request == draft.request
    }
}

fn crypto_request() -> Option<String> {
    let mut random = [0u8; 16];
    let crypto = web_sys::window()?.crypto().ok()?;
    crypto.get_random_values_with_u8_array(&mut random).ok()?;
    Some(random.iter().map(|byte| format!("{byte:02x}")).collect())
}

impl Model {
    pub(super) fn refresh_note_anchor(self) {
        let Some(draft) = self.note_draft.get_untracked() else {
            return;
        };
        let Some(image) = annotation_view::decoded_page_image(self) else {
            return;
        };
        let Some(page) = self.document.with_untracked(|document| {
            document
                .as_ref()
                .and_then(|document| document.info.pages.get(draft.page))
                .cloned()
        }) else {
            return;
        };
        let rect = image.get_bounding_client_rect();
        self.note_anchor.set(
            pdfmaster_ui::canvas_notes::page_anchor(
                draft.anchor(),
                pdfmaster_ui::canvas_notes::Frame {
                    left: rect.left() as f32,
                    top: rect.top() as f32,
                    width: rect.width() as f32,
                    height: rect.height() as f32,
                },
                page.width,
                page.height,
                page.rotation,
            )
            .ok(),
        );
    }

    pub(super) fn note_pointer_down(
        self,
        event: &web_sys::PointerEvent,
        surface: NodeRef<leptos::html::Div>,
    ) -> bool {
        if self.chrome.get_untracked().active_tool != Some(ActiveTool::Note) {
            return false;
        }
        if self.blocked()
            || !self.image_ready.get_untracked()
            || !event.is_primary()
            || event.button() != 0
        {
            return true;
        }
        let Some(image) = annotation_view::decoded_page_image(self) else {
            self.message.set("页面图片尚未解码，未开始便笺。".into());
            return true;
        };
        let Some(document) = self.document.get_untracked() else {
            return true;
        };
        let page_index = self.page.get_untracked();
        let Some(page) = document.info.pages.get(page_index) else {
            return true;
        };
        let rect = image.get_bounding_client_rect();
        let frame = pdfmaster_ui::canvas_notes::Frame {
            left: rect.left() as f32,
            top: rect.top() as f32,
            width: rect.width() as f32,
            height: rect.height() as f32,
        };
        if !frame.valid() {
            self.message.set("页面图片尺寸无效，未开始便笺。".into());
            return true;
        }
        let Some(node) = surface.get() else {
            return true;
        };
        if node.set_pointer_capture(event.pointer_id()).is_err() {
            self.message.set("无法捕获便笺指针，未开始。".into());
            return true;
        }
        event.prevent_default();
        self.note_pointer.set(Some(NotePointer {
            pointer: event.pointer_id(),
            start: Point {
                x: event.client_x() as f32,
                y: event.client_y() as f32,
            },
            frame,
            source: self.image.get_untracked(),
            document_id: document.id,
            page: page_index,
            ticket: self.annotation_image_ticket.get_untracked(),
            page_width: page.width,
            page_height: page.height,
            rotation: page.rotation,
            moved: false,
        }));
        true
    }

    pub(super) fn note_pointer_move(self, event: &web_sys::PointerEvent) -> bool {
        let mut owned = false;
        self.note_pointer.update(|pointer| {
            if let Some(pointer) = pointer
                && pointer.pointer == event.pointer_id()
            {
                owned = true;
                let current = Point {
                    x: event.client_x() as f32,
                    y: event.client_y() as f32,
                };
                pointer.moved |= !pdfmaster_ui::canvas_notes::is_click(pointer.start, current);
            }
        });
        if owned {
            event.prevent_default();
        }
        owned
    }

    pub(super) fn note_pointer_up(
        self,
        event: &web_sys::PointerEvent,
        surface: NodeRef<leptos::html::Div>,
    ) -> bool {
        let Some(pointer) = self.note_pointer.get_untracked() else {
            return false;
        };
        if pointer.pointer != event.pointer_id() {
            return true;
        }
        self.note_pointer.set(None);
        if let Some(node) = surface.get() {
            let _ = node.release_pointer_capture(event.pointer_id());
        }
        let Some(image) = annotation_view::decoded_page_image(self) else {
            self.message.set("页面图片已改变，未建立便笺。".into());
            return true;
        };
        let rect = image.get_bounding_client_rect();
        let frame = pdfmaster_ui::canvas_notes::Frame {
            left: rect.left() as f32,
            top: rect.top() as f32,
            width: rect.width() as f32,
            height: rect.height() as f32,
        };
        let end = Point {
            x: event.client_x() as f32,
            y: event.client_y() as f32,
        };
        let current_document = self
            .document
            .with_untracked(|document| document.as_ref().map(|document| document.id));
        if pointer.moved
            || !pdfmaster_ui::canvas_notes::is_click(pointer.start, end)
            || frame.changed_from(pointer.frame)
            || self.chrome.get_untracked().active_tool != Some(ActiveTool::Note)
            || self.blocked()
            || self.page.get_untracked() != pointer.page
            || current_document != Some(pointer.document_id)
            || self.annotation_image_ticket.get_untracked() != pointer.ticket
            || self.image.get_untracked() != pointer.source
        {
            self.message
                .set("便笺点击已移动或页面已改变，未建立草稿。".into());
            return true;
        }
        let point = match pdfmaster_ui::canvas_notes::click_page_point(
            end,
            frame,
            pointer.page_width,
            pointer.page_height,
            pointer.rotation,
        ) {
            Ok(point) => point,
            Err(error) => {
                self.message.set(error.into());
                return true;
            }
        };
        let target = match self.notes.get_untracked() {
            Some(Ok(notes)) => notes
                .notes
                .into_iter()
                .filter(|note| {
                    pdfmaster_ui::canvas_notes::note_contains(
                        [note.bounds.x, note.bounds.y, note.bounds.width, note.bounds.height],
                        point,
                    )
                })
                .max_by_key(|note| note.annotation_index)
                .map_or_else(
                    || {
                        json!({"kind":"new","x":point.x,"y":point.y,"page_width":pointer.page_width,"page_height":pointer.page_height,"rotation":pointer.rotation})
                    },
                    |note| {
                        json!({"kind":"existing","annotation_index":note.annotation_index,"expected_contents":note.contents})
                    },
                ),
            Some(Err(error)) => {
                self.message
                    .set(format!("当前页便笺读取失败：{error}；未猜测新增位置。"));
                return true;
            }
            None => {
                self.message.set("当前页便笺尚未读取，未猜测新增位置。".into());
                return true;
            }
        };
        let Some(request) = crypto_request() else {
            self.message.set("无法建立便笺请求身份，未开始。".into());
            return true;
        };
        let pending = NoteRequest {
            request: request.clone(),
            document_id: pointer.document_id,
            page: pointer.page,
            ticket: pointer.ticket,
            source: pointer.source,
            cancelling: false,
        };
        self.note_request.set(Some(pending.clone()));
        self.note_anchor.set(Some(point));
        self.message.set("正在保护便笺编辑会话…".into());
        let args = json!({"request":request,"document_id":pointer.document_id,"page":pointer.page,"target":target});
        spawn_local(async move {
            match bridge::call_bounded::<NoteDraft>("begin_canvas_note_editor", &args).await {
                Ok(editor) => {
                    let current = self.note_request.with_untracked(|request| {
                        request.as_ref().is_some_and(|request| {
                            !request.cancelling
                                && request.request == editor.request
                                && request.document_id == pointer.document_id
                                && request.page == self.page.get_untracked()
                                && request.ticket == self.annotation_image_ticket.get_untracked()
                                && request.source == self.image.get_untracked()
                        })
                    });
                    if current {
                        self.note_edit_text.set(editor.initial_contents());
                        self.note_commit.set(None);
                        self.note_anchor.set(
                            pdfmaster_ui::canvas_notes::page_anchor(
                                editor.anchor(),
                                pointer.frame,
                                pointer.page_width,
                                pointer.page_height,
                                pointer.rotation,
                            )
                            .ok(),
                        );
                        self.note_draft.set(Some(editor));
                        self.note_request.set(None);
                        self.message
                            .set("编辑便笺正文；完成后仍需导出 PDF。".into());
                    } else {
                        let _ = bridge::call_bounded::<Document>(
                            "cancel_canvas_note_request",
                            &json!({"request":editor.request}),
                        )
                        .await;
                    }
                }
                Err(error) => self.message.set(format!(
                    "{error}；请求状态未知，可在底栏取消／同步，当前页面不再自动重发。"
                )),
            }
        });
        true
    }

    pub(super) fn note_pointer_cancel(self, pointer_id: i32) -> bool {
        if self.note_pointer.with_untracked(|pointer| {
            pointer
                .as_ref()
                .is_some_and(|pointer| pointer.pointer == pointer_id)
        }) {
            self.note_pointer.set(None);
            self.message.set("便笺点击已取消。".into());
            true
        } else {
            false
        }
    }

    pub(super) fn finish_note(self, delete: bool) {
        if self.busy.get_untracked() {
            return;
        }
        let Some(draft) = self.note_draft.get_untracked() else {
            return;
        };
        let attempt = match self.note_commit.get_untracked() {
            Some(attempt) if attempt.owns(&draft) => attempt,
            Some(_) => {
                self.message
                    .set("已有另一个便笺终态待核对，未替换原请求。".into());
                return;
            }
            None => {
                let contents = if delete {
                    String::new()
                } else {
                    self.note_edit_text.get_untracked()
                };
                NoteCommitAttempt::for_draft(&draft, contents)
            }
        };
        let contents = attempt.contents.clone();
        let unchanged_existing = matches!(
            &draft.target,
            NoteDraftTarget::Existing { original } if original.contents == contents
        );
        if !unchanged_existing && contents.len() > 64 * 1024 {
            self.message.set("便笺内容不能超过64KiB。".into());
            return;
        }
        self.note_commit.set(Some(attempt.clone()));
        let expected_document = self
            .document
            .with_untracked(|document| document.as_ref().map(|document| document.id));
        self.busy.set(true);
        spawn_local(async move {
            match bridge::call_bounded::<Document>(
                "commit_note_editor",
                &json!({"id":draft.id,"contents":contents}),
            )
            .await
            {
                Ok(document) => {
                    let current = self.note_draft.with_untracked(|active| {
                        active.as_ref().is_some_and(|active| attempt.owns(active))
                    }) && self
                        .note_commit
                        .with_untracked(|active| active.as_ref() == Some(&attempt));
                    if !current || Some(document.id) != expected_document {
                        self.message
                            .set("便笺完成回复已过期；当前正文与owner保持不变。".into());
                    } else if let Some(owner) = document.note_editor.clone() {
                        let same_owner = owner.id == draft.id && owner.request == draft.request;
                        self.replace_document_view(Some(document));
                        self.page.set(owner.page);
                        self.page_jump.set((owner.page + 1).to_string());
                        if !same_owner {
                            self.note_edit_text.set(owner.initial_contents());
                            self.note_commit.set(None);
                        }
                        self.note_draft.set(Some(owner));
                        self.note_anchor.set(None);
                        self.message.set(if same_owner {
                            "原生便笺事务仍未结束；正文和owner已保留。".into()
                        } else {
                            "原生返回了另一个便笺owner；已恢复该owner，未清空。".into()
                        });
                        self.render().await;
                        self.refresh_note_anchor();
                    } else {
                        self.replace_document_view(Some(document));
                        self.note_draft.set(None);
                        self.note_request.set(None);
                        self.note_commit.set(None);
                        self.note_anchor.set(None);
                        self.note_edit_text.set(String::new());
                        self.search_result.set(None);
                        self.message.set("便笺事务已完成；请导出副本保存。".into());
                        self.render().await;
                    }
                }
                Err(error) => self
                    .message
                    .set(format!("{error}；正文与原生ID已保留，可重试完成。")),
            }
            self.busy.set(false);
        });
    }

    pub(super) fn cancel_canvas_note(self) {
        if self.busy.get_untracked() {
            return;
        }
        let draft = self.note_draft.get_untracked();
        let pending = self.note_request.get_untracked();
        if draft.is_none() && pending.is_none() {
            return;
        }
        let expected_document = self
            .document
            .with_untracked(|document| document.as_ref().map(|document| document.id));
        let expected_id = draft.as_ref().map(|draft| draft.id);
        let expected_request = draft
            .as_ref()
            .map(|draft| draft.request.clone())
            .or_else(|| pending.as_ref().map(|pending| pending.request.clone()));
        let expected_commit = self
            .note_commit
            .get_untracked()
            .filter(|attempt| draft.as_ref().is_some_and(|draft| attempt.owns(draft)));
        if draft.is_none() {
            self.note_request.update(|request| {
                if let Some(request) = request
                    && Some(&request.request) == expected_request.as_ref()
                {
                    request.cancelling = true;
                }
            });
        }
        self.busy.set(true);
        spawn_local(async move {
            let result = if let Some(draft) = draft.as_ref() {
                bridge::call_bounded::<Document>("cancel_note_editor", &json!({"id":draft.id}))
                    .await
            } else {
                bridge::call_bounded::<Document>(
                    "cancel_canvas_note_request",
                    &json!({"request":pending.as_ref().expect("pending request").request}),
                )
                .await
            };
            match result {
                Ok(document) => {
                    let current = match expected_id {
                        Some(id) => self.note_draft.with_untracked(|active| {
                            active.as_ref().is_some_and(|active| {
                                active.id == id
                                    && Some(&active.request) == expected_request.as_ref()
                            })
                        }),
                        None => self.note_request.with_untracked(|active| {
                            active.as_ref().is_some_and(|active| {
                                Some(&active.request) == expected_request.as_ref()
                            })
                        }),
                    };
                    if !current || Some(document.id) != expected_document {
                        self.message
                            .set("便笺取消回复已过期；当前owner保持不变。".into());
                    } else if let Some(owner) = document.note_editor.clone() {
                        self.replace_document_view(Some(document));
                        self.page.set(owner.page);
                        self.page_jump.set((owner.page + 1).to_string());
                        let same_owner = expected_id == Some(owner.id)
                            && Some(&owner.request) == expected_request.as_ref();
                        if !same_owner {
                            self.note_edit_text.set(owner.initial_contents());
                            self.note_commit.set(None);
                        } else if self
                            .note_commit
                            .with_untracked(|attempt| attempt.as_ref() != expected_commit.as_ref())
                        {
                            self.message
                                .set("便笺取消回复已过期；当前终态请求保持不变。".into());
                            self.busy.set(false);
                            return;
                        }
                        self.note_draft.set(Some(owner));
                        self.note_request.set(None);
                        self.note_anchor.set(None);
                        self.message
                            .set("原生仍有便笺owner；已恢复该owner，未假报取消。".into());
                        self.render().await;
                        self.refresh_note_anchor();
                    } else {
                        self.replace_document_view(Some(document));
                        self.note_draft.set(None);
                        self.note_request.set(None);
                        if self
                            .note_commit
                            .with_untracked(|attempt| attempt.as_ref() == expected_commit.as_ref())
                        {
                            self.note_commit.set(None);
                        }
                        self.note_anchor.set(None);
                        self.note_edit_text.set(String::new());
                        self.message.set("已取消便笺正文编辑。".into());
                        self.render().await;
                    }
                }
                Err(error) => {
                    self.note_request.update(|request| {
                        if let Some(request) = request
                            && Some(&request.request) == expected_request.as_ref()
                        {
                            request.cancelling = false;
                        }
                    });
                    self.message.set(format!("{error}；便笺恢复入口仍保留。"));
                }
            }
            self.busy.set(false);
        });
    }
}

#[component]
pub(super) fn Editor(model: Model) -> impl IntoView {
    let m = model;
    let input = NodeRef::<leptos::html::Textarea>::new();
    Effect::new(move |_| {
        if m.note_draft.get().is_some() {
            leptos::leptos_dom::helpers::request_animation_frame(move || {
                if let Some(input) = input.get() {
                    let options = web_sys::FocusOptions::new();
                    options.set_prevent_scroll(true);
                    let _ = input.focus_with_options(&options);
                }
            });
        }
    });
    view! {
        <Show when=move||m.note_draft.get().is_some()>
            <div class="canvas-note-editor" aria-label="便笺正文编辑" style=move||{
                let point=m.note_anchor.get().unwrap_or(Point{x:24.,y:24.});
                format!("--note-x:{}px;--note-y:{}px",point.x,point.y)
            }>
                <button class="canvas-note-complete-x" aria-label="完成便笺" title="完成" disabled=move||m.busy.get() on:click=move|_|m.finish_note(false)>"×"</button>
                <textarea node_ref=input aria-label="便笺正文" maxlength=(64*1024).to_string() disabled=move||m.busy.get()
                    prop:value=move||m.note_edit_text.get()
                    on:input=move|event|m.note_edit_text.set(event_target_value(&event))
                    on:keydown=move|event:web_sys::KeyboardEvent|{
                        if event.is_composing() {return;}
                        if event.key()=="Escape" {event.prevent_default();m.cancel_canvas_note();}
                        else if event.key()=="Enter"&&(event.ctrl_key()||event.meta_key()) {event.prevent_default();m.finish_note(false);}
                    }></textarea>
                <div class="canvas-note-actions">
                    <button disabled=move||m.busy.get() on:click=move|_|m.finish_note(false)>"完成"</button>
                    <button disabled=move||m.busy.get() on:click=move|_|m.cancel_canvas_note()>"取消"</button>
                    <Show when=move||m.note_draft.with(|draft|draft.as_ref().is_some_and(|draft|matches!(draft.target,NoteDraftTarget::Existing{..})))>
                        <button class="danger" aria-label="删除便笺" disabled=move||m.busy.get() on:click=move|_|m.finish_note(true)>"删除"</button>
                    </Show>
                </div>
            </div>
        </Show>
    }
}

#[component]
pub(super) fn Recovery(model: Model) -> impl IntoView {
    let m = model;
    view! {
        <Show when=move||m.note_draft.get().is_some()||m.note_request.get().is_some()>
            <span class="note-recovery">
                <span>{move||if m.note_draft.get().is_some(){"便笺正文尚未结束"}else{"便笺请求待同步"}}</span>
                <Show when=move||m.note_draft.get().is_some()>
                    <button aria-label="重试完成便笺" disabled=move||m.busy.get() on:click=move|_|m.finish_note(false)>"重试完成"</button>
                </Show>
                <button aria-label="取消或同步便笺" disabled=move||m.busy.get() on:click=move|_|m.cancel_canvas_note()>"取消／同步"</button>
            </span>
        </Show>
    }
}
