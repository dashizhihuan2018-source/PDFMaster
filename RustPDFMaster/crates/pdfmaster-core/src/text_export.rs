//! Immutable, bounded derivatives. These never enter PDF save/history ownership.
use crate::{
    PdfMasterError, Result, Session,
    notes::{decode_contents, resolve},
};
use lopdf::{Dictionary, Document, Object, StringFormat};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

const FIELD: usize = 64 * 1024;
const DECODED: usize = 2 * 1024 * 1024;
const OUTPUT: usize = 32 * 1024 * 1024;
const ENTRIES: usize = 16_384;
const HTML_HEAD: &str = "<!doctype html><html><head><meta charset=\"utf-8\"></head><body><table><thead><tr><th>Page</th><th>Type</th><th>Contents</th><th>Author</th></tr></thead><tbody>";
const HTML_TAIL: &str = "</tbody></table></body></html>";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextExportFormat {
    PlainText,
    SummaryText,
    SummaryMarkdown,
    SummaryHtml,
}

fn malformed() -> PdfMasterError {
    PdfMasterError::Malformed("invalid text-export annotation metadata".into())
}
fn budget() -> PdfMasterError {
    PdfMasterError::Budget("text export")
}
fn append(out: &mut String, text: &str) -> Result<()> {
    if text.len() > OUTPUT.saturating_sub(out.len()) {
        return Err(budget());
    }
    out.push_str(text);
    Ok(())
}
fn count_decoded(length: usize, aggregate: &mut usize) -> Result<()> {
    if length > FIELD || length > DECODED.saturating_sub(*aggregate) {
        return Err(budget());
    }
    *aggregate += length;
    Ok(())
}

// Preflight exact UTF-8 length before the existing decoder allocates a String.
fn field(doc: &Document, dict: &Dictionary, key: &[u8], aggregate: &mut usize) -> Result<String> {
    let Ok(value) = dict.get(key) else {
        return Ok(String::new());
    };
    let value = resolve(doc, value)?;
    let bytes = value.as_str().map_err(|_| malformed())?;
    if bytes.len() > FIELD * 2 + 3 {
        return Err(budget());
    }
    let length = if bytes.starts_with(&[0xfe, 0xff]) || bytes.starts_with(&[0xff, 0xfe]) {
        if (bytes.len() - 2) % 2 != 0 {
            return Err(malformed());
        }
        let little = bytes[0] == 0xff;
        let units = bytes[2..].chunks_exact(2).map(|b| {
            if little {
                u16::from_le_bytes([b[0], b[1]])
            } else {
                u16::from_be_bytes([b[0], b[1]])
            }
        });
        let mut length = 0;
        for ch in char::decode_utf16(units) {
            length += ch.map_err(|_| malformed())?.len_utf8();
        }
        length
    } else if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        std::str::from_utf8(&bytes[3..])
            .map_err(|_| malformed())?
            .len()
    } else {
        static LENGTHS: OnceLock<[usize; 256]> = OnceLock::new();
        let lengths = LENGTHS.get_or_init(|| {
            std::array::from_fn(|byte| {
                let object = Object::String(vec![byte as u8], StringFormat::Literal);
                decode_contents(&Document::new(), &object)
                    .map(|s| s.len())
                    .unwrap_or(0)
            })
        });
        bytes.iter().map(|b| lengths[*b as usize]).sum()
    };
    count_decoded(length, aggregate)?;
    decode_contents(doc, value)
}

fn escaped(out: &mut String, text: &str, markdown: bool) -> Result<()> {
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        let entity = match ch {
            '&' => Some("&amp;"),
            '<' => Some("&lt;"),
            '>' => Some("&gt;"),
            '"' => Some("&quot;"),
            '\'' => Some("&#39;"),
            _ => None,
        };
        if let Some(entity) = entity {
            append(out, entity)?;
        } else if markdown && matches!(ch, '\r' | '\n') {
            if ch == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            append(out, " ")?;
        } else {
            if markdown && matches!(ch, '\\' | '*' | '_' | '`' | '[' | ']' | '|') {
                append(out, "\\")?;
            }
            append(out, ch.encode_utf8(&mut [0; 4]))?;
        }
    }
    Ok(())
}

impl Session {
    pub fn export_text(&self, format: TextExportFormat) -> Result<String> {
        let mut output = String::new();
        if format == TextExportFormat::PlainText {
            let reader = self.text_reader()?;
            for page in 0..self.pages.len() {
                if page != 0 {
                    append(&mut output, "\n\n")?;
                }
                append(&mut output, &reader.text(page)?)?;
            }
            return Ok(output);
        }
        if format == TextExportFormat::SummaryHtml {
            append(&mut output, HTML_HEAD)?;
        }
        let mut visited = 0usize;
        let mut rows = 0usize;
        let mut aggregate = 0usize;
        for (index, page) in self.document.get_pages().values().enumerate() {
            let dict = self
                .document
                .get_object(*page)
                .and_then(Object::as_dict)
                .map_err(|_| malformed())?;
            let Ok(entries) = dict.get(b"Annots") else {
                continue;
            };
            let entries = resolve(&self.document, entries)?
                .as_array()
                .map_err(|_| malformed())?;
            if entries.len() > 4096 || entries.len() > ENTRIES.saturating_sub(visited) {
                return Err(budget());
            }
            visited += entries.len();
            for entry in entries {
                if rows == ENTRIES {
                    return Err(budget());
                }
                let dict = resolve(&self.document, entry)?
                    .as_dict()
                    .map_err(|_| malformed())?;
                let kind = match dict.get(b"Subtype") {
                    Ok(value) => std::str::from_utf8(
                        resolve(&self.document, value)?
                            .as_name()
                            .map_err(|_| malformed())?,
                    )
                    .map_err(|_| malformed())?,
                    Err(_) => "Unknown",
                };
                count_decoded(kind.len(), &mut aggregate)?;
                let contents = field(&self.document, dict, b"Contents", &mut aggregate)?;
                let author = field(&self.document, dict, b"T", &mut aggregate)?;
                if format == TextExportFormat::SummaryHtml {
                    append(&mut output, "<tr><td>")?;
                    append(&mut output, &(index + 1).to_string())?;
                    for text in [kind, contents.as_str(), author.as_str()] {
                        append(&mut output, "</td><td>")?;
                        escaped(&mut output, text, false)?;
                    }
                    append(&mut output, "</td></tr>")?;
                } else {
                    if rows != 0 {
                        append(&mut output, "\n")?;
                    }
                    if format == TextExportFormat::SummaryMarkdown {
                        append(&mut output, "- ")?;
                    }
                    append(&mut output, &format!("Page {} | ", index + 1))?;
                    for (field_index, text) in [kind, contents.as_str(), author.as_str()]
                        .into_iter()
                        .enumerate()
                    {
                        if field_index != 0 {
                            append(&mut output, " | ")?;
                        }
                        if format == TextExportFormat::SummaryMarkdown {
                            escaped(&mut output, text, true)?;
                        } else {
                            append(&mut output, text)?;
                        }
                    }
                }
                rows += 1;
            }
        }
        if format == TextExportFormat::SummaryHtml {
            append(&mut output, HTML_TAIL)?;
        }
        Ok(output)
    }
}
