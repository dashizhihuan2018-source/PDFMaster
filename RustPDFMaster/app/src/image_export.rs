use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Png,
    Jpeg,
}

impl Format {
    pub const fn from_jpeg(jpeg: bool) -> Self {
        if jpeg { Self::Jpeg } else { Self::Png }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Jpeg => "JPEG",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrepareRequest {
    pub format: Format,
}

impl PrepareRequest {
    pub const fn new(format: Format) -> Self {
        Self { format }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRequest {
    pub id: u32,
}

impl TaskRequest {
    pub const fn new(id: u32) -> Self {
        Self { id }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub id: u32,
    pub pages: usize,
    pub total_bytes: u64,
    pub folder_name: String,
    pub warning: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    id: u32,
    format: Format,
    cancelling: bool,
    error: String,
}

impl Job {
    pub fn new(id: u32, format: Format) -> Self {
        Self {
            id,
            format,
            cancelling: false,
            error: String::new(),
        }
    }

    pub const fn id(&self) -> u32 {
        self.id
    }

    pub const fn format(&self) -> Format {
        self.format
    }

    pub const fn cancelling(&self) -> bool {
        self.cancelling
    }

    pub fn error(&self) -> &str {
        &self.error
    }

    pub fn begin_cancel(&mut self) -> bool {
        if self.cancelling {
            return false;
        }
        self.cancelling = true;
        self.error.clear();
        true
    }

    pub fn cancel_failed(&mut self, id: u32) -> bool {
        if id != self.id || !self.cancelling {
            return false;
        }
        self.cancelling = false;
        true
    }

    pub fn set_error(&mut self, error: String) {
        self.error = error;
    }

    pub fn accept_receipt(&self, receipt: Receipt) -> Option<Receipt> {
        (receipt.id == self.id).then_some(receipt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_wire_roundtrip_preserves_formats_task_and_receipt_warning() {
        assert_eq!(
            serde_json::to_value(PrepareRequest::new(Format::Png)).unwrap(),
            serde_json::json!({"format":"png"})
        );
        assert_eq!(
            serde_json::to_value(PrepareRequest::new(Format::Jpeg)).unwrap(),
            serde_json::json!({"format":"jpeg"})
        );
        let task: Task = serde_json::from_value(serde_json::json!({"id":17})).unwrap();
        assert_eq!(task.id, 17);
        let receipt: Receipt = serde_json::from_value(serde_json::json!({
            "id":17,
            "pages":3,
            "total_bytes":4096,
            "folder_name":"sample-images",
            "warning":"Windows 目录持久性尚未确认"
        }))
        .unwrap();
        assert_eq!(receipt.id, 17);
        assert_eq!(receipt.pages, 3);
        assert_eq!(receipt.total_bytes, 4096);
        assert_eq!(receipt.folder_name, "sample-images");
        assert_eq!(
            receipt.warning.as_deref(),
            Some("Windows 目录持久性尚未确认")
        );
        assert_eq!(
            serde_json::to_value(TaskRequest::new(17)).unwrap(),
            serde_json::json!({"id":17})
        );
    }

    #[test]
    fn matching_cancel_can_retry_after_failure_and_rejects_stale_reply() {
        let mut job = Job::new(7, Format::Png);
        assert!(job.begin_cancel());
        assert!(!job.begin_cancel());
        assert!(!job.cancel_failed(6));
        assert!(job.cancelling());
        assert!(job.cancel_failed(7));
        assert!(!job.cancelling());
        assert!(job.begin_cancel());
    }

    #[test]
    fn completion_after_cancel_preserves_matching_published_receipt() {
        let mut job = Job::new(7, Format::Jpeg);
        assert!(job.begin_cancel());
        let receipt = Receipt {
            id: 7,
            pages: 2,
            total_bytes: 900,
            folder_name: "published".into(),
            warning: Some("目录警告".into()),
        };
        assert_eq!(job.accept_receipt(receipt.clone()), Some(receipt));
        assert_eq!(
            job.accept_receipt(Receipt {
                id: 8,
                pages: 1,
                total_bytes: 1,
                folder_name: "stale".into(),
                warning: None,
            }),
            None
        );
    }

    #[test]
    fn catalog_enables_only_existing_pdf_to_image_entry() {
        assert_eq!(
            crate::shell::tool("pdf-to-image").unwrap().workbench_target,
            Some("image-export")
        );
        for id in [
            "edit",
            "sign",
            "forms",
            "ocr",
            "compress",
            "compare",
            "pdf-to-word",
            "pdf-to-excel",
            "pdf-to-powerpoint",
        ] {
            assert_eq!(
                crate::shell::tool(id).unwrap().workbench_target,
                None,
                "unrelated unavailable tool became live: {id}"
            );
        }
    }

    #[test]
    fn export_mode_dispatches_direct_png_image_action() {
        let image = crate::workbench::tools(crate::workbench::Mode::Export)
            .iter()
            .find(|tool| tool.id == "export-image")
            .expect("Export strip image action");
        assert_eq!(image.title, "PNG 图片");
        assert_eq!(
            image.action,
            Some(crate::workbench::Action::ExportImages(false))
        );
    }
}
