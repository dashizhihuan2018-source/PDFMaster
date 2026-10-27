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

    // Share a read-only ownership audit without cloning the entire PDF again.
    annots_without_note(document, pages, page, annotation_index, expected_contents)?;
    let (_, old_id) = resolve_with_id(document, &annotation_object)?;
    let mut annotation = resolve(document, &annotation_object)?
        .as_dict()
        .map_err(|_| PdfMasterError::Malformed("annotation entry must be a dictionary".into()))?
        .clone();
    annotation.set("Contents", text_string(contents));
    annotation.set("M", crate::annotations::modified_now()?);
    if annotation.has(b"P") {
        annotation.set("P", page_id);
    }
    let annotation_id = document.new_object_id();
    let mut popup_ids = HashSet::new();
    if let Ok(popup) = annotation.get(b"Popup") {
        let id = resolve_with_id(document, popup)?
            .1
            .ok_or_else(|| PdfMasterError::Malformed("annotation Popup must be indirect".into()))?;
        popup_ids.insert(id);
    }
    for value in &annots {
        let (value, id) = resolve_with_id(document, value)?;
        let dictionary = value
            .as_dict()
            .map_err(|_| PdfMasterError::Malformed("annotation must be a dictionary".into()))?;
        if resolve(
            document,
            dictionary
                .get(b"Subtype")
                .map_err(|_| PdfMasterError::Malformed("annotation is missing Subtype".into()))?,
        )?
        .as_name()
        .ok()
            == Some(b"Popup")
            && let Ok(parent) = dictionary.get(b"Parent")
            && old_id.is_some()
            && resolve_with_id(document, parent)?.1 == old_id
        {
            popup_ids.insert(id.ok_or(PdfMasterError::InvalidOperation(
                "direct popup rebinding is not supported",
            ))?);
        }
    }
    if popup_ids.len() > 1 {
        return Err(PdfMasterError::Malformed(
            "conflicting popup ownership".into(),
        ));
    }
    if let Some(old_popup) = popup_ids.into_iter().next() {
        let mut popup = document
            .get_object(old_popup)
            .and_then(Object::as_dict)
            .map_err(|error| PdfMasterError::Malformed(error.to_string()))?
            .clone();
        popup.set("Parent", annotation_id);
        if popup.has(b"P") {
            popup.set("P", page_id);
        }
        let new_popup = document.add_object(popup);
        annotation.set("Popup", new_popup);
        for value in &mut annots {
            if resolve_with_id(document, value)?.1 == Some(old_popup) {
                *value = Object::Reference(new_popup);
            }
        }
    }
    document.objects.insert(annotation_id, annotation.into());
    annots[annotation_index] = Object::Reference(annotation_id);
    let annots_id = document.add_object(annots);
    document
        .get_object_mut(page_id)
        .and_then(Object::as_dict_mut)
        .map_err(|error| PdfMasterError::Malformed(error.to_string()))?
        .set("Annots", annots_id);
    validate_notes_page(document, page_id, &pages[page])
}

/// Remove page membership, not the shared indirect objects or historical bytes.
/// This is an undoable annotation operation, never a secure-erasure primitive.
pub(crate) fn delete_note(
    document: &mut Document,
    pages: &[PageInfo],
    page: usize,
    annotation_index: usize,
    expected_contents: &str,
) -> Result<()> {
    let remaining =
        annots_without_note(document, pages, page, annotation_index, expected_contents)?;
    let page_id = page_object_ids(document)?[page];
    // Copy the array so another page sharing it remains byte-for-byte semantic.
    let annots_id = document.add_object(remaining);
    document
        .get_object_mut(page_id)
        .and_then(Object::as_dict_mut)
        .map_err(|error| PdfMasterError::Malformed(error.to_string()))?
        .set("Annots", annots_id);
    validate_notes_page(document, page_id, &pages[page])
}

fn annots_without_note(
    document: &Document,
    pages: &[PageInfo],
    page: usize,
    annotation_index: usize,
    expected_contents: &str,
) -> Result<Vec<Object>> {
    validate_page_index(page, pages.len())?;
    if expected_contents.len() > MAX_DECODED_CONTENTS_BYTES {
        return Err(PdfMasterError::Budget("expected annotation contents"));
    }
    let page_id = page_object_ids(document)?[page];
    let notes = enumerate_notes(document, page_id, &pages[page])?;
    let current = notes
        .notes
        .iter()
        .find(|note| note.annotation_index == annotation_index)
        .ok_or(PdfMasterError::InvalidOperation(
            "Text annotation index is unavailable",
        ))?;
    if current.contents != expected_contents {
        return Err(PdfMasterError::InvalidOperation(
            "annotation contents changed since it was read",
        ));
    }
    annots_without_annotation(document, pages, page, annotation_index)
}

/// Ownership audit shared by Text and shape lifecycle. Removing membership
/// remains copy-on-write; this never deletes shared indirect objects.
pub(crate) fn annots_without_annotation(
    document: &Document,
    pages: &[PageInfo],
    page: usize,
    annotation_index: usize,
) -> Result<Vec<Object>> {
    validate_page_index(page, pages.len())?;
    let page_id = page_object_ids(document)?[page];
    let annots = validated_annots_for_add(document, page_id, &pages[page])?;
    let target = annots
        .get(annotation_index)
        .ok_or(PdfMasterError::InvalidOperation(
            "annotation index is unavailable",
        ))?;
    let (target, target_id) = resolve_with_id(document, target)?;
    let target = target
        .as_dict()
        .map_err(|_| PdfMasterError::Malformed("annotation must be a dictionary".into()))?;
    if target.has(b"StructParent") {
        return Err(PdfMasterError::InvalidOperation(
            "tagged annotation removal is not yet supported",
        ));
    }
    if target.has(b"IRT") {
        return Err(PdfMasterError::InvalidOperation(
            "threaded annotation lifecycle is not yet supported",
        ));
    }
    if let Some(target_id) = target_id {
        // Replies may live on another page and may be direct dictionaries.
        // Do not orphan them or silently redirect them to an obsolete COW root.
        let mut scanned = 0usize;
        for id in page_object_ids(document)? {
            let page = document
                .get_object(id)
                .and_then(Object::as_dict)
                .map_err(|error| PdfMasterError::Malformed(error.to_string()))?;
            let Ok(entries) = page.get(b"Annots") else {
                continue;
            };
            let entries = resolve(document, entries)?
                .as_array()
                .map_err(|_| PdfMasterError::Malformed("page Annots must be an array".into()))?;
            scanned = scanned
                .checked_add(entries.len())
                .ok_or(PdfMasterError::Budget("annotation relationship scan"))?;
            if scanned > 16_384 {
                return Err(PdfMasterError::Budget("annotation relationship scan"));
            }
            for entry in entries {
                let dictionary = resolve(document, entry)?.as_dict().map_err(|_| {
                    PdfMasterError::Malformed("annotation must be a dictionary".into())
                })?;
                if let Ok(parent) = dictionary.get(b"IRT") {
                    parent.as_reference().map_err(|_| {
                        PdfMasterError::Malformed("annotation IRT must be indirect".into())
                    })?;
                    if resolve_with_id(document, parent)?.1 == Some(target_id) {
                        return Err(PdfMasterError::InvalidOperation(
                            "threaded annotation lifecycle is not yet supported",
                        ));
                    }
                }
            }
        }
    }
    let popup_id = match target.get(b"Popup") {
        Ok(popup) => {
            popup.as_reference().map_err(|_| {
                PdfMasterError::Malformed("annotation Popup must be indirect".into())
            })?;
            let (popup, popup_id) = resolve_with_id(document, popup)?;
            let popup = popup.as_dict().map_err(|_| {
                PdfMasterError::Malformed("annotation Popup must be a dictionary".into())
            })?;
            if resolve(
                document,
                popup
                    .get(b"Subtype")
                    .map_err(|_| PdfMasterError::Malformed("Popup is missing Subtype".into()))?,
            )?
            .as_name()
            .ok()
                != Some(b"Popup")
            {
                return Err(PdfMasterError::Malformed(
                    "annotation Popup subtype is invalid".into(),
                ));
            }
            if popup.has(b"StructParent") {
                return Err(PdfMasterError::InvalidOperation(
                    "tagged popup removal is not yet supported",
                ));
            }
            if let Ok(parent) = popup.get(b"Parent") {
                parent.as_reference().map_err(|_| {
                    PdfMasterError::Malformed("Popup Parent must be indirect".into())
                })?;
                if target_id.is_none() || resolve_with_id(document, parent)?.1 != target_id {
                    return Err(PdfMasterError::Malformed(
                        "Popup belongs to another annotation".into(),
                    ));
                }
            }
            popup_id
        }
        Err(_) => None,
    };
    let mut remove = HashSet::from([annotation_index]);
    let mut owned_popups = popup_id.into_iter().collect::<HashSet<_>>();
    let mut direct_popups = 0usize;
    for (index, value) in annots.iter().enumerate() {
        if index == annotation_index {
            continue;
        }
        let (value, id) = resolve_with_id(document, value)?;
        let dictionary = value
            .as_dict()
            .map_err(|_| PdfMasterError::Malformed("annotation must be a dictionary".into()))?;
        if resolve(
            document,
            dictionary
                .get(b"Subtype")
                .map_err(|_| PdfMasterError::Malformed("annotation is missing Subtype".into()))?,
        )?
        .as_name()
        .ok()
            == Some(b"Popup")
        {
            let belongs_to_target = if let Ok(parent) = dictionary.get(b"Parent") {
                parent.as_reference().map_err(|_| {
                    PdfMasterError::Malformed("Popup Parent must be indirect".into())
                })?;
                target_id.is_some() && resolve_with_id(document, parent)?.1 == target_id
            } else {
                false
            };
            if popup_id.is_some() && id == popup_id || belongs_to_target {
                if dictionary.has(b"StructParent") {
                    return Err(PdfMasterError::InvalidOperation(
                        "tagged popup removal is not yet supported",
                    ));
                }
                if popup_id.is_some() && id != popup_id {
                    return Err(PdfMasterError::Malformed(
                        "conflicting popup ownership".into(),
                    ));
                }
                remove.insert(index);
                if let Some(id) = id {
                    owned_popups.insert(id);
                } else {
                    direct_popups += 1;
                }
                if owned_popups.len() + direct_popups > 1 {
                    return Err(PdfMasterError::Malformed(
                        "conflicting popup ownership".into(),
                    ));
                }
            }
        }
    }
    for (index, value) in annots.iter().enumerate() {
        if !remove.contains(&index)
            && let Ok(other_popup) = resolve(document, value)?
                .as_dict()
                .and_then(|value| value.get(b"Popup"))
            && resolve_with_id(document, other_popup)?
                .1
                .is_some_and(|id| owned_popups.contains(&id))
        {
            return Err(PdfMasterError::InvalidOperation(
                "popup is shared with another annotation",
            ));
        }
    }
    let remaining = annots
        .into_iter()
        .enumerate()
        .filter_map(|(index, value)| (!remove.contains(&index)).then_some(value))
        .collect::<Vec<_>>();
    Ok(remaining)
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

pub(crate) fn parse_rect(
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

pub(crate) fn parse_color(
    document: &Document,
    annotation: &lopdf::Dictionary,
) -> Result<Option<Color>> {
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

pub(crate) fn decode_contents(document: &Document, object: &Object) -> Result<String> {
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

pub(crate) fn resolve<'a>(document: &'a Document, object: &'a Object) -> Result<&'a Object> {
    resolve_with_id(document, object).map(|(value, _)| value)
}

pub(crate) fn resolve_with_id<'a>(
    document: &'a Document,
    object: &'a Object,
) -> Result<(&'a Object, Option<ObjectId>)> {
    let mut current = object;
    let mut resolved_id = None;
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
                resolved_id = Some(*id);
            }
            value => return Ok((value, resolved_id)),
        }
    }
}
