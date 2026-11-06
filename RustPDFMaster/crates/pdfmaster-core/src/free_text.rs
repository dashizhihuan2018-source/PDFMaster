use lopdf::content::Content;
use lopdf::{Document, Object};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::document::{page_object_ids, validate_page_index};
use crate::notes::{decode_contents, parse_rect, resolve, resolve_with_id};
use crate::{Color, PdfMasterError, Rect, Result, Session};

const MAX_ANNOTATIONS_PER_PAGE: usize = 1_024;
const MAX_CONTENTS_SOURCE_BYTES: usize = 2 * 65_536 + 3;
const MAX_CONTENTS_BYTES: usize = 65_536;
const MAX_TOTAL_CONTENTS_BYTES: usize = 1024 * 1024;
const MAX_DA_BYTES: usize = 4_096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlignment {
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FreeTextStyle {
    pub font_resource: Vec<u8>,
    pub font_size: f32,
    pub color: Color,
    pub alignment: TextAlignment,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FreeTextAnnotation {
    pub page_index: usize,
    pub annotation_index: usize,
    pub page_id: (u32, u16),
    pub object_id: Option<(u32, u16)>,
    pub bounds: Rect,
    pub contents: String,
    pub style: FreeTextStyle,
    pub flags: u32,
    pub revision: [u8; 32],
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FreeTextList {
    pub annotations: Vec<FreeTextAnnotation>,
    pub unsupported_count: usize,
}

impl Session {
    pub fn free_texts(&self, page: usize) -> Result<FreeTextList> {
        validate_page_index(page, self.pages.len())?;
        let revision = Sha256::digest(&self.current_bytes).into();
        project_free_texts(&self.document, &self.pages, page, revision)
    }

    pub fn delete_free_text(&mut self, expected: &FreeTextAnnotation) -> Result<()> {
        let annotation = self.selected_free_text(expected)?;
        validate_free_text_delete_permission(&self.document, &annotation)?;

        let mut candidate = self.document.clone();
        let remaining = crate::notes::annots_without_annotation(
            &candidate,
            &self.pages,
            expected.page_index,
            expected.annotation_index,
        )?;
        let array = candidate.add_object(remaining);
        candidate
            .get_object_mut(expected.page_id)
            .and_then(Object::as_dict_mut)
            .map_err(|error| PdfMasterError::Malformed(error.to_string()))?
            .set("Annots", array);
        self.commit_document(candidate)
    }

    pub(crate) fn selected_free_text(
        &self,
        expected: &FreeTextAnnotation,
    ) -> Result<lopdf::Dictionary> {
        let revision: [u8; 32] = Sha256::digest(&self.current_bytes).into();
        if expected.revision != revision {
            return Err(PdfMasterError::InvalidOperation(
                "document changed since FreeText was selected",
            ));
        }
        let current =
            project_free_texts(&self.document, &self.pages, expected.page_index, revision)?;
        if !current.annotations.iter().any(|value| value == expected) {
            return Err(PdfMasterError::InvalidOperation(
                "FreeText changed since it was selected",
            ));
        }

        let page_id = page_object_ids(&self.document)?[expected.page_index];
        let page = self
            .document
            .get_object(page_id)
            .and_then(Object::as_dict)
            .map_err(|error| PdfMasterError::Malformed(error.to_string()))?;
        let annots = resolve(
            &self.document,
            page.get(b"Annots")
                .map_err(|_| PdfMasterError::InvalidOperation("page has no annotations"))?,
        )?
        .as_array()
        .map_err(|_| PdfMasterError::Malformed("page Annots must be an array".into()))?;
        let annotation = resolve(
            &self.document,
            annots
                .get(expected.annotation_index)
                .ok_or(PdfMasterError::InvalidOperation(
                    "annotation index is unavailable",
                ))?,
        )?
        .as_dict()
        .map_err(|_| PdfMasterError::Malformed("annotation must be a dictionary".into()))?;
        Ok(annotation.clone())
    }
}

pub(crate) fn validate_free_text_delete_permission(
    document: &Document,
    annotation: &lopdf::Dictionary,
) -> Result<()> {
    let subtype = annotation
        .get(b"Subtype")
        .map_err(|_| PdfMasterError::Malformed("annotation is missing Subtype".into()))?;
    let subtype = resolve(document, subtype)?
        .as_name()
        .map_err(|_| PdfMasterError::Malformed("annotation Subtype must be a name".into()))?;
    if subtype != b"FreeText" {
        return Ok(());
    }
    let flags = parse_flags(document, annotation)?;
    if flags & (64 | 128) != 0 {
        return Err(PdfMasterError::InvalidOperation(
            "read-only or locked FreeText cannot be deleted",
        ));
    }
    Ok(())
}

fn project_free_texts(
    document: &Document,
    pages: &[crate::PageInfo],
    page: usize,
    revision: [u8; 32],
) -> Result<FreeTextList> {
    validate_page_index(page, pages.len())?;
    let page_id = page_object_ids(document)?[page];
    let page_dictionary = document
        .get_object(page_id)
        .and_then(Object::as_dict)
        .map_err(|error| PdfMasterError::Malformed(error.to_string()))?;
    let Ok(entries) = page_dictionary.get(b"Annots") else {
        return Ok(FreeTextList::default());
    };
    let entries = resolve(document, entries)?
        .as_array()
        .map_err(|_| PdfMasterError::Malformed("page Annots must be an array".into()))?;
    if entries.len() > MAX_ANNOTATIONS_PER_PAGE {
        return Err(PdfMasterError::Budget("annotation count"));
    }

    let mut list = FreeTextList::default();
    let mut decoded_contents_bytes = 0usize;
    for (annotation_index, entry) in entries.iter().enumerate() {
        let (entry, object_id) = resolve_with_id(document, entry)?;
        let annotation = entry.as_dict().map_err(|_| {
            PdfMasterError::Malformed("annotation entry must be a dictionary".into())
        })?;
        let subtype = annotation
            .get(b"Subtype")
            .map_err(|_| PdfMasterError::Malformed("annotation is missing Subtype".into()))?;
        let subtype = resolve(document, subtype)?
            .as_name()
            .map_err(|_| PdfMasterError::Malformed("annotation Subtype must be a name".into()))?;
        if subtype != b"FreeText" {
            continue;
        }

        let bounds = parse_rect(document, annotation, &pages[page])?;
        let contents = match annotation.get(b"Contents") {
            Ok(value) => {
                let raw = resolve(document, value)?;
                if raw
                    .as_str()
                    .is_ok_and(|bytes| bytes.len() > MAX_CONTENTS_SOURCE_BYTES)
                {
                    return Err(PdfMasterError::Budget("FreeText contents source"));
                }
                decode_contents(document, raw)?
            }
            Err(_) => String::new(),
        };
        decoded_contents_bytes = decoded_contents_bytes
            .checked_add(contents.len())
            .ok_or(PdfMasterError::Budget("FreeText contents"))?;
        if contents.len() > MAX_CONTENTS_BYTES || decoded_contents_bytes > MAX_TOTAL_CONTENTS_BYTES
        {
            return Err(PdfMasterError::Budget("FreeText contents"));
        }
        let flags = parse_flags(document, annotation)?;
        let alignment = parse_optional_integer(document, annotation, b"Q", 0)?;
        let rotation = parse_optional_integer(document, annotation, b"Rotate", 0)?;
        let has_complex_style = [b"RC".as_slice(), b"DS", b"CL", b"LE", b"RD", b"IT"]
            .iter()
            .any(|key| annotation.has(key));
        let appearance = parse_da(document, annotation)?;
        let (alignment, (font_resource, font_size, color)) =
            match (alignment, rotation, has_complex_style, appearance) {
                (0, 0, false, Some(appearance)) => (TextAlignment::Left, appearance),
                (1, 0, false, Some(appearance)) => (TextAlignment::Center, appearance),
                (2, 0, false, Some(appearance)) => (TextAlignment::Right, appearance),
                _ => {
                    list.unsupported_count += 1;
                    continue;
                }
            };
        list.annotations.push(FreeTextAnnotation {
            page_index: page,
            annotation_index,
            page_id,
            object_id,
            bounds,
            contents,
            style: FreeTextStyle {
                font_resource,
                font_size,
                color,
                alignment,
            },
            flags,
            revision,
        });
    }
    Ok(list)
}

fn parse_flags(document: &Document, annotation: &lopdf::Dictionary) -> Result<u32> {
    let Ok(value) = annotation.get(b"F") else {
        return Ok(0);
    };
    let value = resolve(document, value)?
        .as_i64()
        .map_err(|_| PdfMasterError::Malformed("FreeText F must be an integer".into()))?;
    u32::try_from(value)
        .map_err(|_| PdfMasterError::Malformed("FreeText F must be a non-negative u32".into()))
}

fn parse_optional_integer(
    document: &Document,
    annotation: &lopdf::Dictionary,
    key: &[u8],
    default: i64,
) -> Result<i64> {
    match annotation.get(key) {
        Ok(value) => resolve(document, value)?.as_i64().map_err(|_| {
            PdfMasterError::Malformed(format!(
                "FreeText {} must be an integer",
                String::from_utf8_lossy(key)
            ))
        }),
        Err(_) => Ok(default),
    }
}

fn parse_da(
    document: &Document,
    annotation: &lopdf::Dictionary,
) -> Result<Option<(Vec<u8>, f32, Color)>> {
    let Ok(value) = annotation.get(b"DA") else {
        return Ok(None);
    };
    let raw = resolve(document, value)?
        .as_str()
        .map_err(|_| PdfMasterError::Malformed("FreeText DA must be a PDF string".into()))?;
    if raw.len() > MAX_DA_BYTES {
        return Err(PdfMasterError::Budget("FreeText DA"));
    }
    let content = match Content::decode_strict(raw) {
        Ok(content) => content,
        Err(_) => return Ok(None),
    };
    if content.operations.is_empty() || content.operations.len() > 2 {
        return Ok(None);
    }
    let mut font = None;
    let mut color = None;
    for operation in content.operations {
        match operation.operator.as_str() {
            "Tf" if font.is_none() => {
                let [Object::Name(resource), size] = operation.operands.as_slice() else {
                    return Ok(None);
                };
                let Ok(size) = size.as_float() else {
                    return Ok(None);
                };
                if resource.is_empty()
                    || resource.len() > 127
                    || !size.is_finite()
                    || !(0.0..=1000.0).contains(&size)
                    || size == 0.0
                {
                    return Ok(None);
                }
                font = Some((resource.clone(), size));
            }
            "g" if color.is_none() => {
                let [component] = operation.operands.as_slice() else {
                    return Ok(None);
                };
                let Ok(component) = normalized_component(component) else {
                    return Ok(None);
                };
                color = Some(Color {
                    r: component,
                    g: component,
                    b: component,
                });
            }
            "rg" if color.is_none() => {
                let [r, g, b] = operation.operands.as_slice() else {
                    return Ok(None);
                };
                let (Ok(r), Ok(g), Ok(b)) = (
                    normalized_component(r),
                    normalized_component(g),
                    normalized_component(b),
                ) else {
                    return Ok(None);
                };
                color = Some(Color { r, g, b });
            }
            _ => return Ok(None),
        }
    }
    let Some((font, size)) = font else {
        return Ok(None);
    };
    Ok(Some((
        font,
        size,
        color.unwrap_or(Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
        }),
    )))
}

fn normalized_component(value: &Object) -> std::result::Result<f32, ()> {
    let value = value.as_float().map_err(|_| ())?;
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(value)
    } else {
        Err(())
    }
}
