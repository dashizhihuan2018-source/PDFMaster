use super::{Model, bridge, canvas_intent::Intent};
use leptos::{prelude::*, task::spawn_local};
use pdfmaster_ui::{
    text_export::{Receipt, Request, TextExportFormat},
    workbench::Action,
};

impl Model {
    pub(super) fn export_text(self, format: TextExportFormat) {
        if self.defer_canvas_edit(Some(Intent::Strip(Action::ExportText(format)))) {
            return;
        }
        if self.blocked() {
            return;
        }
        self.busy.set(true);
        spawn_local(async move {
            let result =
                bridge::call::<Option<Receipt>>("export_text_document", &Request { format }).await;
            let status = match result {
                Ok(Some(receipt)) if receipt.valid_for(format) => {
                    let mut status = format!(
                        "已导出“{}”（{} 字节）。",
                        receipt.file_name, receipt.total_bytes
                    );
                    if let Some(warning) = receipt.warning.filter(|w| !w.is_empty()) {
                        status.push(' ');
                        status.push_str(&warning);
                    }
                    status
                }
                Ok(Some(_)) => "文本导出回复无效；现有文档和未保存状态保持不变。".into(),
                Ok(None) => "已取消文本导出；文档、页码、缩放和修改状态保持不变。".into(),
                Err(error) => format!("文本导出失败：{error}；现有文档和未保存状态保持不变。"),
            };
            self.message.set(status);
            self.busy.set(false);
        });
    }
}
