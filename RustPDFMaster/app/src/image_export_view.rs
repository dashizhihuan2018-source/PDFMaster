use super::{Model, bridge, canvas_intent::Intent};
use leptos::{prelude::*, task::spawn_local};
use pdfmaster_ui::image_export::{Format, Job, PrepareRequest, Receipt, Task, TaskRequest};

impl Model {
    pub(super) fn export_images(self, jpeg: bool) {
        if self.defer_canvas_edit(Some(Intent::Strip(
            pdfmaster_ui::workbench::Action::ExportImages(jpeg),
        ))) {
            return;
        }
        if self.busy.get_untracked()
            || self.image_import.get_untracked().is_some()
            || self.image_export.get_untracked().is_some()
            || self.pending_image_export.get_untracked().is_some()
            || self.has_note_draft()
            || self.has_ink()
        {
            return;
        }
        if !self.loaded() {
            self.pending_image_export.set(Some(jpeg));
            self.command("open_document", serde_json::json!({}));
            return;
        }
        let format = Format::from_jpeg(jpeg);
        self.busy.set(true);
        spawn_local(async move {
            let prepared =
                bridge::call::<Option<Task>>("prepare_image_export", &PrepareRequest::new(format))
                    .await;
            let task = match prepared {
                Ok(Some(task)) if task.id != 0 => task,
                Ok(Some(_)) => {
                    self.message
                        .set("图片导出任务回复无效；现有文档保持不变。".into());
                    self.busy.set(false);
                    return;
                }
                Ok(None) => {
                    self.message
                        .set("已取消图片导出；现有文档保持不变。".into());
                    self.busy.set(false);
                    return;
                }
                Err(error) => {
                    self.message.set(format!("无法准备图片导出：{error}"));
                    self.busy.set(false);
                    return;
                }
            };
            self.image_export.set(Some(Job::new(task.id, format)));
            let result = bridge::call::<Option<Receipt>>(
                "export_images_document",
                &TaskRequest::new(task.id),
            )
            .await;
            if !self.image_export_current(task.id) {
                return;
            }
            match result {
                Ok(Some(receipt)) => {
                    let accepted = self.image_export.with_untracked(|current| {
                        current.as_ref().and_then(|job| {
                            job.accept_receipt(receipt)
                                .map(|receipt| (job.format(), receipt))
                        })
                    });
                    if let Some((format, receipt)) = accepted {
                        self.image_export.set(None);
                        let mut status = format!(
                            "已导出 {} 页 {} 图片到文件夹“{}”（{} 字节）。",
                            receipt.pages,
                            format.label(),
                            receipt.folder_name,
                            receipt.total_bytes
                        );
                        if let Some(warning) = receipt.warning.filter(|warning| !warning.is_empty())
                        {
                            status.push(' ');
                            status.push_str(&warning);
                        }
                        self.message.set(status);
                    }
                }
                Ok(None) => {
                    self.image_export.set(None);
                    self.message
                        .set("已取消图片导出；文档、页码、缩放和修改状态保持不变。".into());
                }
                Err(error) => {
                    self.image_export.set(None);
                    self.message.set(format!(
                        "图片导出失败：{error}；现有文档和未保存状态保持不变。"
                    ));
                }
            }
            self.busy.set(false);
        });
    }

    fn image_export_current(self, id: u32) -> bool {
        self.image_export
            .with_untracked(|job| job.as_ref().is_some_and(|job| job.id() == id))
    }

    pub(super) fn image_export_status(self) -> Option<String> {
        self.image_export.with(|job| {
            job.as_ref().map(|job| {
                if !job.error().is_empty() {
                    job.error().to_owned()
                } else if job.cancelling() {
                    "正在取消图片导出；等待原生任务结束…".into()
                } else {
                    format!("正在导出 {} 图片…", job.format().label())
                }
            })
        })
    }

    pub(super) fn cancel_image_export(self) {
        let mut id = None;
        self.image_export.update(|job| {
            if let Some(job) = job
                && job.begin_cancel()
            {
                id = Some(job.id());
            }
        });
        let Some(id) = id else {
            return;
        };
        spawn_local(async move {
            let result = bridge::call::<bool>("cancel_image_export", &TaskRequest::new(id)).await;
            if !self.image_export_current(id) {
                return;
            }
            match result {
                Ok(true) => self
                    .message
                    .set("已请求取消图片导出；正在等待原生任务结束。".into()),
                Ok(false) => self
                    .message
                    .set("取消未确认；导出可能已发布，正在等待最终回执。".into()),
                Err(error) => self.image_export.update(|job| {
                    if let Some(job) = job
                        && job.cancel_failed(id)
                    {
                        job.set_error(format!(
                            "取消图片导出未确认：{error}；可重试取消或等待结果。"
                        ));
                    }
                }),
            }
        });
    }
}
