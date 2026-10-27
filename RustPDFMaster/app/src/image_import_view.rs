use super::{Document, Model, bridge};
use leptos::{prelude::*, task::spawn_local};
use pdfmaster_ui::{
    image_import::{Configuration, Job, Selection},
    shell::Route,
};
use serde_json::json;
use wasm_bindgen::JsCast;

impl Model {
    pub(super) fn prepare_images(self) {
        if self.busy.get_untracked()
            || self.image_import.get_untracked().is_some()
            || self.has_note_draft()
            || self.has_ink()
        {
            return;
        }
        self.busy.set(true);
        spawn_local(async move {
            match bridge::call::<Option<Selection>>("prepare_image_import", &json!({})).await {
                Ok(Some(selection)) => {
                    let id = selection.id;
                    match Configuration::new(selection) {
                        Ok(c) => {
                            self.image_import.set(Some(Job::Configuration(c)));
                            self.busy.set(false);
                            self.convert_images();
                            return;
                        }
                        Err(e) => {
                            self.image_import.set(Some(Job::Recovery {
                                id,
                                cancelling: true,
                                error: format!("{e} 正在释放本地任务…"),
                            }));
                            let release =
                                bridge::call::<bool>("cancel_image_import", &json!({"id":id}))
                                    .await;
                            if !self.image_current(id) {
                                return;
                            }
                            match release {
                                Ok(true) => {
                                    self.image_import.set(None);
                                    self.message
                                        .set(format!("{e} 本地任务已释放，可重新选择。"));
                                    focus("image-tool-entry");
                                }
                                Ok(false) => self.image_import.update(|job| {
                                    if let Some(job) = job {
                                        job.set_cancelling(false);
                                        job.set_error(format!("{e} 取消未确认，请重试取消。"));
                                    }
                                }),
                                Err(cancel_error) => self.image_import.update(|job| {
                                    if let Some(job) = job {
                                        job.set_cancelling(false);
                                        job.set_error(format!(
                                            "{e} 取消未确认：{cancel_error}；请重试取消。"
                                        ));
                                    }
                                }),
                            }
                            self.busy.set(false);
                            return;
                        }
                    }
                }
                Ok(None) => self
                    .message
                    .set("已取消选择图片；现有文档保持不变。".into()),
                Err(e) => self.message.set(e),
            }
            self.busy.set(false);
        });
    }

    fn image_current(self, id: u32) -> bool {
        self.image_import
            .with_untracked(|job| job.as_ref().is_some_and(|job| job.id() == id))
    }

    pub(super) fn image_import_status(self) -> Option<String> {
        self.image_import.with(|job| {
            job.as_ref().map(|job| match job {
                Job::Configuration(configuration) if configuration.cancelling => {
                    "正在取消图片转换…".into()
                }
                Job::Configuration(configuration) if !configuration.error.is_empty() => {
                    configuration.error.clone()
                }
                Job::Configuration(configuration) if configuration.running => {
                    "正在转换图片 · 不覆盖来源图片或当前文档".into()
                }
                Job::Configuration(_) => "图片转换仍占用本地任务；请取消后重新选择。".into(),
                Job::Recovery {
                    cancelling: true, ..
                } => "正在取消无效的图片选择…".into(),
                Job::Recovery { error, .. } => error.clone(),
            })
        })
    }

    fn convert_images(self) {
        let Some(Job::Configuration(c)) = self.image_import.get_untracked() else {
            return;
        };
        if c.running || c.cancelling {
            return;
        }
        let layout = match c.layout() {
            Ok(l) => l,
            Err(e) => {
                self.image_import.update(|c| {
                    if let Some(Job::Configuration(c)) = c {
                        c.error = e;
                    }
                });
                return;
            }
        };
        let id = c.selection.id;
        self.remember_tab();
        self.image_import.update(|c| {
            if let Some(Job::Configuration(c)) = c {
                c.running = true;
                c.error.clear();
            }
        });
        self.busy.set(true);
        spawn_local(async move {
            let result = bridge::call::<Document>(
                "convert_image_import",
                &json!({"id":id,"order":c.order,"layout":layout}),
            )
            .await;
            if !self.image_current(id) {
                return;
            }
            match result {
                Ok(doc) => {
                    // Publication won the native lock before a possible cancel.
                    // Render that authoritative document, never hide a new tab.
                    self.image_import.set(None);
                    self.restore_tab(&doc);
                    self.page.set(0);
                    self.page_jump.set("1".into());
                    self.search_result.set(None);
                    self.chrome
                        .update(|c| *c = c.document_opened(false, false, true));
                    self.route.set(Route::Document);
                    self.replace_document_view(Some(doc));
                    let mut message =
                        "已创建独立 PDF；来源图片和已有文档保持不变，请导出副本保存。".to_string();
                    if !self.render().await {
                        message.push_str(&format!(" 预览失败：{}", self.message.get_untracked()));
                    }
                    self.message.set(message);
                }
                Err(e) => {
                    self.image_import.update(|configuration| {
                        if let Some(Job::Configuration(configuration)) = configuration
                            && configuration.selection.id == id
                        {
                            configuration.running = false;
                            configuration.cancelling = true;
                            configuration.error = format!("图片转换失败：{e}；正在释放本地任务…");
                        }
                    });
                    let release =
                        bridge::call::<bool>("cancel_image_import", &json!({"id":id})).await;
                    if !self.image_current(id) {
                        return;
                    }
                    match release {
                        Ok(true) => {
                            self.image_import.set(None);
                            self.message
                                .set(format!("图片转换失败：{e}；本地任务已释放，可重新选择。"));
                            focus("image-tool-entry");
                        }
                        Ok(false) => self.image_import.update(|configuration| {
                            if let Some(Job::Configuration(configuration)) = configuration {
                                configuration.cancelling = false;
                                configuration.error =
                                    format!("图片转换失败：{e}；取消未确认，请重试取消。");
                            }
                        }),
                        Err(cancel_error) => self.image_import.update(|configuration| {
                            if let Some(Job::Configuration(configuration)) = configuration {
                                configuration.cancelling = false;
                                configuration.error = format!(
                                    "图片转换失败：{e}；取消未确认：{cancel_error}；请重试取消。"
                                );
                            }
                        }),
                    }
                }
            }
            self.busy.set(false);
        });
    }

    pub(super) fn cancel_images(self) {
        let Some(job) = self.image_import.get_untracked() else {
            return;
        };
        if job.cancelling() {
            return;
        }
        let id = job.id();
        let recovery = matches!(job, Job::Recovery { .. });
        let conversion_error = match &job {
            Job::Configuration(configuration)
                if configuration.error.starts_with("图片转换失败：") =>
            {
                Some(
                    configuration
                        .error
                        .split("；取消未确认")
                        .next()
                        .unwrap_or(&configuration.error)
                        .to_string(),
                )
            }
            _ => None,
        };
        self.image_import.update(|job| {
            if let Some(job) = job {
                job.set_cancelling(true);
            }
        });
        spawn_local(async move {
            let result = bridge::call::<bool>("cancel_image_import", &json!({"id":id})).await;
            if !self.image_current(id) {
                return;
            }
            match result {
                Ok(true) => {
                    self.image_import.set(None);
                    self.busy.set(false);
                    self.message.set(if recovery {
                        "图片选择回复无效；本地任务已释放，可重新选择。".into()
                    } else if let Some(error) = &conversion_error {
                        format!("{error}；本地任务已释放，可重新选择。")
                    } else {
                        "已取消图片转换；未创建文档。".into()
                    });
                    focus("image-tool-entry");
                }
                Ok(false) => self.image_import.update(|job| {
                    if let Some(job) = job {
                        job.set_cancelling(false);
                        job.set_error(if recovery {
                            "图片选择回复无效；取消未确认，请重试取消。".into()
                        } else if let Some(error) = &conversion_error {
                            format!("{error}；取消未确认，请重试取消。")
                        } else {
                            "取消未确认；请等待当前任务结果。".into()
                        });
                    }
                }),
                Err(e) => self.image_import.update(|job| {
                    if let Some(job) = job {
                        job.set_cancelling(false);
                        job.set_error(if recovery {
                            format!("图片选择回复无效；取消未确认：{e}；请重试取消。")
                        } else if let Some(error) = &conversion_error {
                            format!("{error}；取消未确认：{e}；请重试取消。")
                        } else {
                            format!("取消未确认：{e}；请重试取消或等待结果。")
                        });
                    }
                }),
            }
        });
    }
}

fn focus(id: &str) {
    if let Some(e) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id(id))
        .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = e.focus();
    }
}
