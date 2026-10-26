use std::collections::HashSet;

use lopdf::{Document, Object, ObjectId, decode_text_string, text_string};
use serde::{Deserialize, Serialize};

use crate::document::{
    Color, MAX_TEXT_BYTES, PageInfo, PdfMasterError, Rect, Result, Session, page_object_ids,
    validate_page_index,
};

const MAX_ANNOTATIONS_PER_PAGE: usize = 1_024;
const MAX_DECODED_CONTENTS_BYTES: usize = 1024 * 1024;
const MAX_CONTENTS_SOURCE_BYTES: usize = MAX_DECODED_CONTENTS_BYTES * 2 + 3;
const MAX_REFERENCE_DEPTH: usize = 64;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextNote {
    pub annotation_index: usize,
    pub bounds: Rect,
    pub contents: String,
    pub color: Option<Color>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NoteList {
    pub notes: Vec<TextNote>,
    pub unsupported_count: usize,
}

impl Session {
    pub fn notes(&self, page: usize) -> Result<NoteList> {
        validate_page_index(page, self.pages.len())?;
        let page_id = page_object_ids(&self.document)?[page];
        enumerate_notes(&self.document, page_id, &self.pages[page])
    }
}

pub(crate) fn update_note(
    document: &mut Document,
    pages: &[PageInfo],
    page: usize,
    annotation_index: usize,
    expected_contents: &str,
    contents: &str,
) -> Result<()> {
    validate_page_index(page, pages.len())?;
    if contents.trim().is_empty() {
        return Err(PdfMasterError::InvalidOperation(
            "annotation contents cannot be blank",
        ));
    }
    if contents.len() > MAX_TEXT_BYTES {
        return Err(PdfMasterError::Budget("annotation text length"));
    }
    if expected_contents.len() > MAX_DECODED_CONTENTS_BYTES {
        return Err(PdfMasterError::Budget("expected annotation contents"));
    }

    let page_id = page_object_ids(document)?[page];
    let notes = enumerate_notes(document, page_id, &pages[page])?;
    let page_dictionary = document
        .get_object(page_id)
        .and_then(Object::as_dict)
        .map_err(|error| PdfMasterError::Malformed(error.to_string()))?;
    let annots_object = page_dictionary
        .get(b"Annots")
        .map_err(|_| PdfMasterError::InvalidOperation("page has no annotations"))?;
    let mut annots = resolve(document, annots_object)?
        .as_array()
        .map_err(|_| PdfMasterError::Malformed("page Annots must be an array".into()))?
        .clone();
    let annotation_object = annots
        .get(annotation_index)
        .ok_or(PdfMasterError::InvalidOperation(
            "annotation index is out of range",
        ))?
        .clone();
    let current = notes
        .notes
        .iter()
        .find(|note| note.annotation_index == annotation_index)
        .ok_or(PdfMasterError::InvalidOperation(
            "annotation subtype is unsupported for content updates",
        ))?;
    if current.contents != expected_contents {
        return Err(PdfMasterError::InvalidOperation(
            "annotation contents changed since it was read",
        ));
    }

    let mut annotation = resolve(document, &annotation_object)?
        .as_dict()
        .map_err(|_| PdfMasterError::Malformed("annotation entry must be a dictionary".into()))?
        .clone();
    annotation.set("Contents", text_string(contents));
    let annotation_id = document.add_object(annotation);
    annots[annotation_index] = Object::Reference(annotation_id);
    let annots_id = document.add_object(annots);
    document
        .get_object_mut(page_id)
        .and_then(Object::as_dict_mut)
        .map_err(|error| PdfMasterError::Malformed(error.to_string()))?
        .set("Annots", annots_id);
    validate_notes_page(document, page_id, &pages[page])
}

pub(crate) fn validated_annots_for_add(
    document: &Document,
    page_id: ObjectId,
    page: &PageInfo,
) -> Result<Vec<Object>> {
    validate_notes_page(document, page_id, page)?;
    let page_dictionary = document
        .get_object(page_id)
        .and_then(Object::as_dict)
        .map_err(|error| PdfMasterError::Malformed(error.to_string()))?;
    match page_dictionary.get(b"Annots") {
        Ok(annots) => resolve(document, annots)?
            .as_array()
            .cloned()
            .map_err(|_| PdfMasterError::Malformed("page Annots must be an array".into())),
        Err(_) => Ok(Vec::new()),
    }
}

pub(crate) fn validate_notes_page(
    document: &Document,
    page_id: ObjectId,
    page: &PageInfo,
) -> Result<()> {
    enumerate_notes(document, page_id, page).map(|_| ())
}

fn enumerate_notes(document: &Document, page_id: ObjectId, page: &PageInfo) -> Result<NoteList> {
    let page_dictionary = document
        .get_object(page_id)
        .and_then(Object::as_dict)
        .map_err(|error| PdfMasterError::Malformed(error.to_string()))?;
    let Ok(annots) = page_dictionary.get(b"Annots") else {
        return Ok(NoteList::default());
    };
    let annots = resolve(document, annots)?
        .as_array()
        .map_err(|_| PdfMasterError::Malformed("page Annots must be an array".into()))?;
    if annots.len() > MAX_ANNOTATIONS_PER_PAGE {
        return Err(PdfMasterError::Budget("annotation count"));
    }

    let mut result = NoteList::default();
    let mut decoded_contents_bytes = 0usize;
    for (annotation_index, object) in annots.iter().enumerate() {
        let annotation = resolve(document, object)?.as_dict().map_err(|_| {
            PdfMasterError::Malformed("annotation entry must be a dictionary".into())
        })?;
        let subtype = annotation
            .get(b"Subtype")
            .map_err(|_| PdfMasterError::Malformed("annotation is missing Subtype".into()))?;
        let subtype = resolve(document, subtype)?
            .as_name()
            .map_err(|_| PdfMasterError::Malformed("annotation Subtype must be a name".into()))?;
        let bounds = parse_rect(document, annotation, page)?;
        if subtype != b"Text" {
            result.unsupported_count += 1;
            continue;
        }

        let color = parse_color(document, annotation)?;
        let contents = match annotation.get(b"Contents") {
            Ok(value) => decode_contents(document, value)?,
            Err(_) => String::new(),
        };
        decoded_contents_bytes = decoded_contents_bytes
            .checked_add(contents.len())
            .ok_or(PdfMasterError::Budget("annotation contents"))?;
        if decoded_contents_bytes > MAX_DECODED_CONTENTS_BYTES {
            return Err(PdfMasterError::Budget("annotation contents"));
        }
        result.notes.push(TextNote {
            annotation_index,
            bounds,
            contents,
            color,
        });
    }
    Ok(result)
}

fn parse_rect(
    document: &Document,
    annotation: &lopdf::Dictionary,
    page: &PageInfo,
) -> Result<Rect> {
    let rect = annotation
        .get(b"Rect")
        .map_err(|_| PdfMasterError::Malformed("annotation is missing Rect".into()))?;
    let values = resolve(document, rect)?
        .as_array()
        .map_err(|_| PdfMasterError::Malformed("annotation Rect must be an array".into()))?;
    if values.len() != 4 {
        return Err(PdfMasterError::Malformed(
            "annotation Rect must contain four numbers".into(),
        ));
    }
    let mut coordinates = [0.0_f32; 4];
    for (index, value) in values.iter().enumerate() {
        coordinates[index] = resolve(document, value)?.as_float().map_err(|_| {
            PdfMasterError::Malformed("annotation Rect values must be numbers".into())
        })?;
        if !coordinates[index].is_finite() {
            return Err(PdfMasterError::Malformed(
                "annotation Rect values must be finite".into(),
            ));
        }
    }
    let [x1, y1, x2, y2] = coordinates;
    if x1 < 0.0 || y1 < 0.0 || x2 <= x1 || y2 <= y1 || x2 > page.width || y2 > page.height {
        return Err(PdfMasterError::InvalidGeometry(
            "annotation Rect must be increasing and inside its page",
        ));
    }
    Ok(Rect {
        x: x1,
        y: y1,
        width: x2 - x1,
        height: y2 - y1,
    })
}

fn parse_color(document: &Document, annotation: &lopdf::Dictionary) -> Result<Option<Color>> {
    let Ok(color) = annotation.get(b"C") else {
        return Ok(None);
    };
    let values = resolve(document, color)?
        .as_array()
        .map_err(|_| PdfMasterError::Malformed("Text annotation C must be an array".into()))?;
    if values.len() != 3 {
        return Err(PdfMasterError::Malformed(
            "Text annotation C must contain three RGB components".into(),
        ));
    }
    let mut components = [0.0_f32; 3];
    for (index, value) in values.iter().enumerate() {
        components[index] = resolve(document, value)?.as_float().map_err(|_| {
            PdfMasterError::Malformed("Text annotation C values must be numbers".into())
        })?;
        if !components[index].is_finite() || !(0.0..=1.0).contains(&components[index]) {
            return Err(PdfMasterError::InvalidGeometry(
                "Text annotation color components must be finite and between zero and one",
            ));
        }
    }
    Ok(Some(Color {
        r: components[0],
        g: components[1],
        b: components[2],
    }))
}

fn decode_contents(document: &Document, object: &Object) -> Result<String> {
    let object = resolve(document, object)?;
    let bytes = object.as_str().map_err(|_| {
        PdfMasterError::Malformed("Text annotation Contents must be a PDF string".into())
    })?;
    if bytes.len() > MAX_CONTENTS_SOURCE_BYTES {
        return Err(PdfMasterError::Budget("annotation contents"));
    }

    if bytes.starts_with(&[0xff, 0xfe]) {
        let encoded = &bytes[2..];
        if encoded.len() % 2 != 0 {
            return Err(PdfMasterError::Malformed(
                "Text annotation Contents has malformed UTF-16LE".into(),
            ));
        }
        let units = encoded
            .chunks_exact(2)
            .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
            .collect::<Vec<_>>();
        return String::from_utf16(&units).map_err(|_| {
            PdfMasterError::Malformed("Text annotation Contents has malformed UTF-16LE".into())
        });
    }
    if bytes.starts_with(&[0xfe, 0xff]) && (bytes.len() - 2) % 2 != 0 {
        return Err(PdfMasterError::Malformed(
            "Text annotation Contents has malformed UTF-16BE".into(),
        ));
    }
    decode_text_string(object).map_err(|error| {
        PdfMasterError::Malformed(format!(
            "Text annotation Contents cannot be decoded: {error}"
        ))
    })
}

fn resolve<'a>(document: &'a Document, object: &'a Object) -> Result<&'a Object> {
    let mut current = object;
    let mut seen = HashSet::new();
    let mut reference_count = 0usize;
    loop {
        match current {
            Object::Reference(id) => {
                if reference_count == MAX_REFERENCE_DEPTH {
                    return Err(PdfMasterError::Malformed(
                        "annotation object reference depth exceeded".into(),
                    ));
                }
                reference_count += 1;
                if !seen.insert(*id) {
                    return Err(PdfMasterError::Malformed(
                        "annotation object reference cycle".into(),
                    ));
                }
                current = document.objects.get(id).ok_or_else(|| {
                    PdfMasterError::Malformed("annotation object reference is missing".into())
                })?;
            }
            value => return Ok(value),
        }
    }
}
