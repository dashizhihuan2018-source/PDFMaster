//! Operation-specific native prompt lifecycle, after the shared canvas handoff.
use super::*;
use pdfmaster_ui::workbench::PageAction;
#[derive(Deserialize)]
struct PageActionReply {
    document: Option<Document>,
    target_page: Option<usize>,
}
impl Model {
    pub(super) fn prompt_page_action(self, operation: PageAction) {
        if self.defer_canvas_edit(Some(TextIntent::PageAction(operation))) {
            return;
        }
        if self.blocked() {
            return;
        }
        self.close_chrome_popups();
        self.remember_tab();
        self.busy.set(true);
        spawn_local(async move {
            let result = bridge::call::<PageActionReply>(
                "prompt_page_action",
                &json!({"operation":operation,"page":self.page.get_untracked()}),
            )
            .await;
            match result {
                Ok(PageActionReply {
                    document: Some(document),
                    target_page,
                }) => {
                    self.page.update(|p| {
                        *p = target_page
                            .unwrap_or(*p)
                            .min(document.info.page_count.saturating_sub(1))
                    });
                    self.page_jump
                        .set((self.page.get_untracked() + 1).to_string());
                    self.search_result.set(None);
                    self.message.set(
                        document
                            .warning
                            .clone()
                            .unwrap_or_else(|| "页面操作已完成。".into()),
                    );
                    self.replace_document_view(Some(document));
                    self.overview_selection
                        .update(|s| s.reset(self.page.get_untracked(), self.count()));
                    self.render().await;
                }
                Ok(_) => self.message.set("已取消，文档保持不变。".into()),
                Err(error) => self.message.set(error),
            }
            self.busy.set(false);
        });
    }
}
