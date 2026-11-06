//! Genuine decoded-image pointer click; mutation/render use ordinary commands.
use super::*;
use pdfmaster_drawing::Point;
use pdfmaster_ui::{
    canvas_notes::Frame,
    ink_eraser::{Capability, Context, Pointer},
};

impl Model {
    fn eraser_context(self, own_busy: bool) -> Option<Context> {
        if self.chrome.get_untracked().active_tool != Some(ActiveTool::Eraser)
            || !self.image_ready.get_untracked()
            || (self.busy.get_untracked() && !own_busy)
            || self.has_note_draft()
            || self.has_ink()
            || self.has_free_text_draft()
            || self.has_shape_draft()
            || self.image_import.with_untracked(|v| v.is_some())
            || self.image_export.with_untracked(|v| v.is_some())
            || self.overview_visible.get_untracked()
        {
            return None;
        }
        let image = annotation_view::decoded_page_image(self)?;
        let document = self.document.get_untracked()?;
        let page_index = self.page.get_untracked();
        let page = document.info.pages.get(page_index)?;
        let rect = image.get_bounding_client_rect();
        let frame = Frame {
            left: rect.left() as f32,
            top: rect.top() as f32,
            width: rect.width() as f32,
            height: rect.height() as f32,
        };
        if !frame.valid() {
            return None;
        }
        Some(Context {
            document_id: document.id,
            page: page_index,
            ticket: self.annotation_image_ticket.get_untracked(),
            source: self.image.get_untracked(),
            frame,
            page_width: page.width,
            page_height: page.height,
            rotation: page.rotation,
        })
    }

    pub(super) fn eraser_pointer_down(
        self,
        event: &web_sys::PointerEvent,
        surface: NodeRef<leptos::html::Div>,
    ) -> bool {
        if self.chrome.get_untracked().active_tool != Some(ActiveTool::Eraser) {
            return false;
        }
        let Some(context) = self.eraser_context(false) else {
            return true;
        };
        let Some(pointer) = Pointer::begin(
            event.pointer_id(),
            event.is_primary(),
            event.button(),
            Point {
                x: event.client_x() as f32,
                y: event.client_y() as f32,
            },
            context,
        ) else {
            return true;
        };
        let Some(node) = surface.get() else {
            return true;
        };
        if node.set_pointer_capture(event.pointer_id()).is_err() {
            self.message.set("无法捕获橡皮指针，未擦除。".into());
            return true;
        }
        // Do not prevent default: scrolling must remain ordinary scrolling.
        self.eraser_pointer.set(Some(pointer));
        true
    }
    pub(super) fn eraser_pointer_move(self, event: &web_sys::PointerEvent) -> bool {
        let mut owned = false;
        self.eraser_pointer.update(|p| {
            if let Some(p) = p
                && p.id == event.pointer_id()
            {
                p.sample(
                    event.pointer_id(),
                    Point {
                        x: event.client_x() as f32,
                        y: event.client_y() as f32,
                    },
                );
                owned = true;
            }
        });
        owned
    }
    pub(super) fn eraser_pointer_cancel(self, id: i32) -> bool {
        let owned = self
            .eraser_pointer
            .with_untracked(|p| p.as_ref().is_some_and(|p| p.id == id));
        if owned {
            self.eraser_pointer.set(None);
        }
        owned
    }
    pub(super) fn eraser_pointer_up(
        self,
        event: &web_sys::PointerEvent,
        surface: NodeRef<leptos::html::Div>,
    ) -> bool {
        let Some(pointer) = self.eraser_pointer.get_untracked() else {
            return false;
        };
        if pointer.id != event.pointer_id() {
            return true;
        }
        self.eraser_pointer.set(None);
        if let Some(node) = surface.get() {
            let _ = node.release_pointer_capture(event.pointer_id());
        }
        let Some(context) = self.eraser_context(false) else {
            return true;
        };
        let end = Point {
            x: event.client_x() as f32,
            y: event.client_y() as f32,
        };
        if !event.is_primary()
            || event.button() != 0
            || !pointer.accepts(event.pointer_id(), end, &context)
        {
            return true;
        }
        let Some(owner) = self.eraser_sequence.get_untracked().checked_add(1) else {
            self.message
                .set("橡皮点击编号已耗尽，请安全保存后重启。".into());
            return true;
        };
        self.eraser_sequence.set(owner);
        self.eraser_preparing.set(Some(owner));
        self.busy.set(true);
        spawn_local(async move {
            let result: Result<Capability, String> = bridge::call(
                "prepare_ink_eraser",
                &json!({"document_id":context.document_id,"page":context.page}),
            )
            .await;
            if self.eraser_preparing.get_untracked() != Some(owner) {
                return;
            }
            let current = self.eraser_context(true);
            // Release only this read-only preparation's busy ownership, then
            // synchronously hand off to the existing command/render lifecycle.
            self.eraser_preparing.set(None);
            self.busy.set(false);
            match result {
                Ok(capability)
                    if capability.matches(&context) && current.as_ref() == Some(&context) =>
                {
                    match context
                        .arguments(capability.id, end)
                        .and_then(|args| serde_json::to_value(args).map_err(|_| "橡皮参数无效。"))
                    {
                        Ok(args) => {
                            self.command_at_page("erase_ink_document", args, Some(context.page))
                        }
                        Err(error) => self.message.set(error.into()),
                    }
                }
                Ok(_) => self
                    .message
                    .set("橡皮点击期间页面已改变，未擦除；请重新点击。".into()),
                Err(error) => self.message.set(error),
            }
        });
        true
    }
}
