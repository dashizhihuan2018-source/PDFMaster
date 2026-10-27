//! Read-only, bounded annotation projection. Never follows an action or appearance.
use lopdf::{Dictionary, Document, Object};
use serde::{Deserialize, Serialize};

use crate::document::{Color, PageInfo, PdfMasterError, Rect, Result, Session, page_object_ids};
use crate::notes::{decode_contents, resolve, resolve_with_id};

const MAX_PAGE_ENTRIES: usize = 4096;
const MAX_ENTRIES: usize = 16384;
const MAX_ROWS: usize = 4096;
const MAX_TEXT: usize = 65536;
const MAX_TOTAL_TEXT: usize = 2 * 1024 * 1024;

pub(crate) fn modified_now() -> Result<Object> {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .ok_or(PdfMasterError::InvalidOperation(
            "system clock cannot provide a PDF modification date",
        ))?;
    utc_date(seconds)
}

fn utc_date(seconds: i64) -> Result<Object> {
    let date = time::OffsetDateTime::from_unix_timestamp(seconds)
        .map_err(|_| PdfMasterError::InvalidOperation("system clock is outside PDF date range"))?;
    if !(1..=9999).contains(&date.year()) {
        return Err(PdfMasterError::InvalidOperation(
            "system clock is outside PDF date range",
        ));
    }
    Ok(Object::string_literal(format!(
        "D:{:04}{:02}{:02}{:02}{:02}{:02}Z",
        date.year(),
        u8::from(date.month()),
        date.day(),
        date.hour(),
        date.minute(),
        date.second()
    )))
}

#[cfg(test)]
mod timestamp_tests {
    use super::*;
    #[test]
    fn modification_dates_are_real_utc_calendar_values_and_reject_out_of_range_clock() {
        for (seconds, expected) in [
            (0, "D:19700101000000Z"),
            (951782400, "D:20000229000000Z"),
            (2147483647, "D:20380119031407Z"),
        ] {
            assert_eq!(
                utc_date(seconds).unwrap().as_str().unwrap(),
                expected.as_bytes()
            );
        }
        assert!(utc_date(i64::MAX).is_err());
        assert!(utc_date(253402300800).is_err());
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnnotationRow {
    pub page_index: usize,
    pub annotation_index: usize,
    pub page_id: (u32, u16),
    pub object_id: Option<(u32, u16)>,
    pub subtype: String,
    pub contents: String,
    pub color: Option<Color>,
    pub bounds: Option<Rect>,
    /// ISO local date-time, with an explicit offset only if present in the PDF.
    pub modified: Option<String>,
}

impl Session {
    pub fn annotations(&self) -> Result<Vec<AnnotationRow>> {
        enumerate_annotations(&self.document)
    }
}

pub(crate) fn delete_annotation(
    document: &mut Document,
    pages: &[PageInfo],
    expected: &AnnotationRow,
) -> Result<()> {
    // Compare the exact displayed row, including its original page and slot.
    // Filtered Widget/Link/Popup rows can never be forged into this operation.
    if !enumerate_annotations(document)?
        .iter()
        .any(|row| row == expected)
    {
        return Err(PdfMasterError::InvalidOperation(
            "annotation changed since it was selected",
        ));
    }
    let remaining = crate::notes::annots_without_annotation(
        document,
        pages,
        expected.page_index,
        expected.annotation_index,
    )?;
    // Detach only this page's membership array; preserve indirect objects,
    // embedded payloads, appearances and other pages. Not secure erasure.
    let array = document.add_object(remaining);
    document
        .get_object_mut(expected.page_id)
        .and_then(Object::as_dict_mut)
        .map_err(|error| PdfMasterError::Malformed(error.to_string()))?
        .set("Annots", array);
    Ok(())
}

fn enumerate_annotations(document: &Document) -> Result<Vec<AnnotationRow>> {
    let mut rows = Vec::new();
    let mut visited = 0usize;
    let mut text_bytes = 0usize;
    for (page_index, page_id) in page_object_ids(document)?.into_iter().enumerate() {
        let page = document
            .get_object(page_id)
            .and_then(Object::as_dict)
            .map_err(|e| PdfMasterError::Malformed(e.to_string()))?;
        let Ok(entries) = page.get(b"Annots") else {
            continue;
        };
        let entries = resolve(document, entries)?
            .as_array()
            .map_err(|_| PdfMasterError::Malformed("page Annots must be an array".into()))?;
        visited = visited
            .checked_add(entries.len())
            .ok_or(PdfMasterError::Budget("annotation projection entries"))?;
        if entries.len() > MAX_PAGE_ENTRIES || visited > MAX_ENTRIES {
            return Err(PdfMasterError::Budget("annotation projection entries"));
        }
        for (annotation_index, value) in entries.iter().enumerate() {
            let (value, object_id) = resolve_with_id(document, value)?;
            let annotation = value
                .as_dict()
                .map_err(|_| PdfMasterError::Malformed("annotation must be a dictionary".into()))?;
            let subtype = annotation
                .get(b"Subtype")
                .map_err(|_| PdfMasterError::Malformed("annotation is missing Subtype".into()))?;
            let subtype = resolve(document, subtype)?.as_name().map_err(|_| {
                PdfMasterError::Malformed("annotation Subtype must be a name".into())
            })?;
            if matches!(subtype, b"Widget" | b"Link" | b"Popup") {
                continue;
            }
            if rows.len() == MAX_ROWS || subtype.len() > 128 {
                return Err(PdfMasterError::Budget("annotation projection rows/type"));
            }
            let contents = match annotation.get(b"Contents") {
                Err(_) => String::new(),
                Ok(value) => {
                    let raw = resolve(document, value)?;
                    if raw.as_str().is_ok_and(|s| s.len() > MAX_TEXT * 2 + 3) {
                        return Err(PdfMasterError::Budget("annotation projection text"));
                    }
                    decode_contents(document, raw)?
                }
            };
            text_bytes = text_bytes
                .checked_add(contents.len())
                .ok_or(PdfMasterError::Budget("annotation projection text"))?;
            if contents.len() > MAX_TEXT || text_bytes > MAX_TOTAL_TEXT {
                return Err(PdfMasterError::Budget("annotation projection text"));
            }
            let modified = match annotation.get(b"M") {
                Ok(value) => {
                    let value = resolve(document, value)?;
                    value
                        .as_str()
                        .ok()
                        .filter(|s| s.len() <= 32)
                        .and_then(|s| std::str::from_utf8(s).ok())
                        .and_then(pdf_date)
                }
                Err(_) => None,
            };
            rows.push(AnnotationRow {
                page_index,
                annotation_index,
                page_id,
                object_id,
                subtype: String::from_utf8_lossy(subtype).into_owned(),
                contents,
                color: color(document, annotation)?,
                bounds: bounds(document, annotation)?,
                modified,
            });
        }
    }
    Ok(rows)
}

fn bounds(document: &Document, annotation: &Dictionary) -> Result<Option<Rect>> {
    let Ok(value) = annotation.get(b"Rect") else {
        return Ok(None);
    };
    let Ok(values) = resolve(document, value)?.as_array() else {
        return Ok(None);
    };
    if values.len() != 4 {
        return Ok(None);
    }
    let mut v = [0.; 4];
    for (i, value) in values.iter().enumerate() {
        let Ok(value) = resolve(document, value)?.as_float() else {
            return Ok(None);
        };
        if !value.is_finite() {
            return Ok(None);
        }
        v[i] = value;
    }
    let width = (v[2] - v[0]).abs();
    let height = (v[3] - v[1]).abs();
    if width <= 0. || height <= 0. || !width.is_finite() || !height.is_finite() {
        return Ok(None);
    }
    Ok(Some(Rect {
        x: v[0].min(v[2]),
        y: v[1].min(v[3]),
        width,
        height,
    }))
}

fn color(document: &Document, annotation: &Dictionary) -> Result<Option<Color>> {
    let Ok(value) = annotation.get(b"C") else {
        return Ok(None);
    };
    let values = resolve(document, value)?
        .as_array()
        .map_err(|_| PdfMasterError::Malformed("annotation C must be an array".into()))?;
    if !matches!(values.len(), 0 | 1 | 3 | 4) {
        return Err(PdfMasterError::Malformed(
            "annotation C must be Gray, RGB, CMYK or empty".into(),
        ));
    }
    let mut v = [0.; 4];
    for (i, value) in values.iter().enumerate() {
        v[i] = resolve(document, value)?
            .as_float()
            .map_err(|_| PdfMasterError::Malformed("annotation C must contain numbers".into()))?;
        if !v[i].is_finite() || !(0. ..=1.).contains(&v[i]) {
            return Err(PdfMasterError::InvalidGeometry(
                "annotation C must be finite and normalized",
            ));
        }
    }
    let (r, g, b) = match values.len() {
        0 => return Ok(None),
        1 => (v[0], v[0], v[0]),
        3 => (v[0], v[1], v[2]),
        _ => (
            1. - (v[0] + v[3]).min(1.),
            1. - (v[1] + v[3]).min(1.),
            1. - (v[2] + v[3]).min(1.),
        ),
    };
    Ok(Some(Color { r, g, b }))
}

// ISO 32000 date fields are optional only in trailing order; do not silently
// repair invalid calendars or invent a timezone for a floating local date.
fn pdf_date(raw: &str) -> Option<String> {
    let value = raw.strip_prefix("D:")?;
    let digits = value.bytes().take_while(u8::is_ascii_digit).count();
    if !matches!(digits, 4 | 6 | 8 | 10 | 12 | 14) {
        return None;
    }
    let mut parts = [1_u32, 1, 1, 0, 0, 0];
    parts[0] = value[..4].parse().ok()?;
    for (index, part) in parts.iter_mut().enumerate().skip(1) {
        let start = 4 + (index - 1) * 2;
        if start < digits {
            *part = value[start..start + 2].parse().ok()?;
        }
    }
    let [year, month, day, hour, minute, second] = parts;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        _ => return None,
    };
    if year == 0 || day == 0 || day > days || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let suffix = &value[digits..];
    let zone = if suffix.is_empty() {
        String::new()
    } else if suffix == "Z" {
        "Z".into()
    } else {
        let sign = suffix.as_bytes()[0];
        if !matches!(sign, b'+' | b'-') || suffix.len() < 3 {
            return None;
        }
        let h = suffix.get(1..3)?.parse::<u8>().ok()?;
        let tail = suffix.get(3..)?;
        let m = if tail.is_empty() || tail == "'" {
            0
        } else {
            let tail = tail.strip_prefix('\'')?;
            let tail = tail.strip_suffix('\'').unwrap_or(tail);
            if tail.len() != 2 || !tail.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            tail.parse::<u8>().ok()?
        };
        if h > 23 || m > 59 {
            return None;
        }
        format!("{}{h:02}:{m:02}", char::from(sign))
    };
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}{zone}"
    ))
}
