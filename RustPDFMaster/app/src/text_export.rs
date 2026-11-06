use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextExportFormat {
    PlainText,
    SummaryText,
    SummaryMarkdown,
    SummaryHtml,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Request {
    pub format: TextExportFormat,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Receipt {
    pub format: TextExportFormat,
    pub file_name: String,
    pub total_bytes: u64,
    pub warning: Option<String>,
}
impl Receipt {
    pub fn valid_for(&self, format: TextExportFormat) -> bool {
        self.format == format
            && self.total_bytes <= 32 * 1024 * 1024
            && !self.file_name.is_empty()
            && self.file_name.len() <= 1024
            && !matches!(self.file_name.as_str(), "." | "..")
            && !self.file_name.chars().any(|c| {
                c.is_control() || matches!(c, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*')
            })
            && self
                .warning
                .as_ref()
                .is_none_or(|warning| warning.len() <= 4096)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_export_wire_and_receipt_are_typed_pathfree_bounded() {
        for (format, wire) in [
            (TextExportFormat::PlainText, "plain_text"),
            (TextExportFormat::SummaryText, "summary_text"),
            (TextExportFormat::SummaryMarkdown, "summary_markdown"),
            (TextExportFormat::SummaryHtml, "summary_html"),
        ] {
            assert_eq!(
                serde_json::to_value(Request { format }).unwrap(),
                serde_json::json!({"format":wire})
            );
            let receipt: Receipt=serde_json::from_value(serde_json::json!({"format":wire,"file_name":"摘要.txt","total_bytes":0,"warning":null})).unwrap();
            assert!(receipt.valid_for(format));
            let mut bad = receipt.clone();
            bad.file_name = "C:\\private\\摘要.txt".into();
            assert!(!bad.valid_for(format));
            bad = receipt.clone();
            bad.file_name = "../摘要.txt".into();
            assert!(!bad.valid_for(format));
            bad = receipt.clone();
            bad.total_bytes = 32 * 1024 * 1024 + 1;
            assert!(!bad.valid_for(format));
            bad = receipt;
            bad.format = if format == TextExportFormat::PlainText {
                TextExportFormat::SummaryText
            } else {
                TextExportFormat::PlainText
            };
            assert!(!bad.valid_for(format));
        }
        assert!(
            serde_json::from_value::<Receipt>(
                serde_json::json!({"file_name":"x.txt","total_bytes":0,"warning":null})
            )
            .is_err()
        );
    }

    #[test]
    fn text_export_existing_menu_strip_and_no_document_reply_route() {
        let view = include_str!("workbench_view.rs");
        for label in [
            "导出纯文本…",
            "导出批注摘要 TXT…",
            "导出批注摘要 Markdown…",
            "导出批注摘要 HTML…",
        ] {
            assert!(view.contains(label), "{label}");
        }
        let strip = crate::workbench::tools(crate::workbench::Mode::Export);
        assert!(strip.iter().any(|tool| tool.id == "export-text"
            && tool.action
                == Some(crate::workbench::Action::ExportText(
                    TextExportFormat::PlainText
                ))));
        let controller = include_str!("text_export_view.rs");
        assert!(controller.contains("export_text_document"));
        for forbidden in [
            "document_info",
            "render_page",
            "self.document.set",
            "self.command(",
            "mark_saved",
        ] {
            assert!(!controller.contains(forbidden), "{forbidden}");
        }
    }
}
