use std::collections::HashSet;

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat, dictionary};

use crate::document::{
    Color, Edit, MAX_PAGES, MAX_TEXT_BYTES, PageInfo, PdfMasterError, Rect, Result, add_blank_page,
    materialize_page_attributes, page_object_ids, validate_page_index, validate_page_size,
};
use crate::notes::{delete_note, update_note, validate_notes_page, validated_annots_for_add};

pub(crate) fn apply_edit(document: &mut Document, pages: &[PageInfo], edit: Edit) -> Result<()> {
    match edit {
        Edit::Rotate { page, degrees } => rotate(document, pages, page, degrees),
        Edit::RotatePages {
            pages: selected,
            degrees,
        } => rotate_pages(document, pages, &selected, degrees),
        Edit::MovePage { from, to } => move_page(document, pages, from, to),
        Edit::InsertBlank {
            index,
            width,
            height,
        } => insert_blank(document, pages, index, width, height),
        Edit::DuplicatePage { page, index } => duplicate_page(document, pages, page, index),
        Edit::DeletePage { page } => delete_page(document, pages, page),
        Edit::DeletePages { pages: selected } => delete_pages(document, pages, &selected),
        Edit::AddText {
            page,
            bounds,
            text,
            font_size,
            color,
        } => add_text(document, pages, page, bounds, &text, font_size, color),
        Edit::AddNote {
            page,
            bounds,
            contents,
            color,
        } => add_note(document, pages, page, bounds, &contents, color),
        Edit::UpdateNote {
            page,
            annotation_index,
            expected_contents,
            contents,
        } => update_note(
            document,
            pages,
            page,
            annotation_index,
            &expected_contents,
            &contents,
        ),
        Edit::DeleteNote {
            page,
            annotation_index,
            expected_contents,
        } => delete_note(document, pages, page, annotation_index, &expected_contents),
    }
}

fn rotate(document: &mut Document, pages: &[PageInfo], page: usize, degrees: i32) -> Result<()> {
    validate_page_index(page, pages.len())?;
    validate_rotation(degrees)?;
    let page_id = page_object_ids(document)?[page];
    let rotation = normalized_rotation(pages[page].rotation, degrees);
    document
        .get_object_mut(page_id)
        .and_then(Object::as_dict_mut)
        .map_err(|error| PdfMasterError::Malformed(error.to_string()))?
        .set("Rotate", rotation as i64);
    Ok(())
}

fn rotate_pages(
    document: &mut Document,
    pages: &[PageInfo],
    selected: &[usize],
    degrees: i32,
) -> Result<()> {
    validate_page_batch(selected, pages.len())?;
    validate_rotation(degrees)?;
    let ids = page_object_ids(document)?;
    for &page in selected {
        let rotation = normalized_rotation(pages[page].rotation, degrees);
        document
            .get_object_mut(ids[page])
            .and_then(Object::as_dict_mut)
            .map_err(|error| PdfMasterError::Malformed(error.to_string()))?
            .set("Rotate", rotation as i64);
    }
    Ok(())
}

fn validate_rotation(degrees: i32) -> Result<()> {
    let degrees = i64::from(degrees);
    if degrees == 0 || degrees % 90 != 0 || degrees.rem_euclid(360) == 0 {
        return Err(PdfMasterError::InvalidOperation(
            "rotation must change the page by a multiple of 90 degrees",
        ));
    }
    Ok(())
}

fn normalized_rotation(current: i32, degrees: i32) -> i32 {
    (i64::from(current) + i64::from(degrees)).rem_euclid(360) as i32
}

fn move_page(document: &mut Document, pages: &[PageInfo], from: usize, to: usize) -> Result<()> {
    validate_page_index(from, pages.len())?;
    validate_page_index(to, pages.len())?;
    if from == to {
        return Err(PdfMasterError::InvalidOperation(
            "page is already at destination",
        ));
    }
    let mut ids = page_object_ids(document)?;
    let id = ids.remove(from);
    ids.insert(to, id);
    flatten_page_tree(document, &ids)
}

fn insert_blank(
    document: &mut Document,
    pages: &[PageInfo],
    index: usize,
    width: f32,
    height: f32,
) -> Result<()> {
    if index > pages.len() {
        return Err(PdfMasterError::InvalidPage {
            index,
            page_count: pages.len(),
        });
    }
    if pages.len() == MAX_PAGES {
        return Err(PdfMasterError::Budget("page count"));
    }
    validate_page_size(width, height)?;
    let root = root_pages_id(document)?;
    let new_page = add_blank_page(document, root, width, height);
    let mut ids = page_object_ids(document)?;
    ids.insert(index, new_page);
    flatten_page_tree(document, &ids)
}

fn duplicate_page(
    document: &mut Document,
    pages: &[PageInfo],
    page: usize,
    index: usize,
) -> Result<()> {
    validate_page_index(page, pages.len())?;
    if index > pages.len() {
        return Err(PdfMasterError::InvalidPage {
            index,
            page_count: pages.len(),
        });
    }
    if pages.len() == MAX_PAGES {
        return Err(PdfMasterError::Budget("page count"));
    }
    let ids = page_object_ids(document)?;
    materialize_page_attributes(document, &[ids[page]])?;
    let source = document
        .get_object(ids[page])
        .and_then(Object::as_dict)
        .map_err(|error| PdfMasterError::Malformed(error.to_string()))?;
    if source.has(b"Annots") || source.has(b"StructParents") {
        return Err(PdfMasterError::InvalidOperation(
            "duplicating pages with annotations or structure ownership is unsupported",
        ));
    }
    let mut dictionary = document
        .get_object(ids[page])
        .and_then(Object::as_dict)
        .map_err(|error| PdfMasterError::Malformed(error.to_string()))?
        .clone();
    dictionary.remove(b"Parent");
    let duplicate = document.add_object(dictionary);
    let mut reordered = ids;
    reordered.insert(index, duplicate);
    flatten_page_tree(document, &reordered)
}

fn delete_page(document: &mut Document, pages: &[PageInfo], page: usize) -> Result<()> {
    validate_page_index(page, pages.len())?;
    if pages.len() == 1 {
        return Err(PdfMasterError::InvalidOperation(
            "cannot delete the last page",
        ));
    }
    let mut ids = page_object_ids(document)?;
    let page_dictionary = document
        .get_object(ids[page])
        .and_then(Object::as_dict)
        .map_err(|error| PdfMasterError::Malformed(error.to_string()))?;
    if page_dictionary.has(b"Annots") || page_dictionary.has(b"StructParents") {
        return Err(PdfMasterError::InvalidOperation(
            "deleting pages with annotations or structure ownership is unsupported",
        ));
    }
    if document
        .catalog()
        .map_err(|error| PdfMasterError::Malformed(error.to_string()))?
        .has(b"AcroForm")
    {
        return Err(PdfMasterError::InvalidOperation(
            "deleting pages from documents with an AcroForm is unsupported",
        ));
    }
    ids.remove(page);
    flatten_page_tree(document, &ids)
}

fn delete_pages(document: &mut Document, pages: &[PageInfo], selected: &[usize]) -> Result<()> {
    validate_page_batch(selected, pages.len())?;
    if selected.len() == pages.len() {
        return Err(PdfMasterError::InvalidOperation("cannot delete every page"));
    }
    if document
        .catalog()
        .map_err(|error| PdfMasterError::Malformed(error.to_string()))?
        .has(b"AcroForm")
    {
        return Err(PdfMasterError::InvalidOperation(
            "deleting pages from documents with an AcroForm is unsupported",
        ));
    }

    let mut ids = page_object_ids(document)?;
    for &page in selected {
        let page_dictionary = document
            .get_object(ids[page])
            .and_then(Object::as_dict)
            .map_err(|error| PdfMasterError::Malformed(error.to_string()))?;
        if page_dictionary.has(b"Annots") || page_dictionary.has(b"StructParents") {
            return Err(PdfMasterError::InvalidOperation(
                "deleting pages with annotations or structure ownership is unsupported",
            ));
        }
    }

    let mut descending = selected.to_vec();
    descending.sort_unstable_by(|left, right| right.cmp(left));
    for page in descending {
        ids.remove(page);
    }
    flatten_page_tree(document, &ids)
}

fn validate_page_batch(selected: &[usize], page_count: usize) -> Result<()> {
    if selected.is_empty() {
        return Err(PdfMasterError::InvalidOperation(
            "page batch cannot be empty",
        ));
    }
    if selected.len() > MAX_PAGES {
        return Err(PdfMasterError::Budget("page batch length"));
    }
    let mut seen = HashSet::with_capacity(selected.len());
    for &page in selected {
        validate_page_index(page, page_count)?;
        if !seen.insert(page) {
            return Err(PdfMasterError::InvalidOperation(
                "page batch contains duplicate indices",
            ));
        }
    }
    Ok(())
}

fn add_text(
    document: &mut Document,
    pages: &[PageInfo],
    page: usize,
    bounds: Rect,
    text: &str,
    font_size: f32,
    color: Color,
) -> Result<()> {
    validate_page_index(page, pages.len())?;
    validate_rect(bounds, &pages[page])?;
    validate_color(color)?;
    if text.is_empty() {
        return Err(PdfMasterError::InvalidOperation("text cannot be empty"));
    }
    if text.len() > MAX_TEXT_BYTES {
        return Err(PdfMasterError::Budget("text length"));
    }
    if !font_size.is_finite() || font_size <= 0.0 || font_size > 512.0 {
        return Err(PdfMasterError::InvalidGeometry("invalid font size"));
    }
    if !text.is_ascii() {
        return Err(PdfMasterError::InvalidOperation(
            "AddText supports ASCII only until an embedded Unicode font is available",
        ));
    }
    reject_rotated_edit(&pages[page])?;

    let page_id = page_object_ids(document)?[page];
    let font_name = install_font(document, page_id)?;
    let baseline = bounds.y + (bounds.height - font_size).max(0.0);
    let content = Content {
        operations: vec![
            Operation::new("q", vec![]),
            Operation::new("rg", vec![color.r.into(), color.g.into(), color.b.into()]),
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec![Object::Name(font_name), font_size.into()]),
            Operation::new("Td", vec![bounds.x.into(), baseline.into()]),
            Operation::new("Tj", vec![Object::string_literal(text)]),
            Operation::new("ET", vec![]),
            Operation::new("Q", vec![]),
        ],
    }
    .encode()
    .map_err(|error| PdfMasterError::Serialization(error.to_string()))?;
    append_page_contents(document, page_id, content)
}

fn add_note(
    document: &mut Document,
    pages: &[PageInfo],
    page: usize,
    bounds: Rect,
    contents: &str,
    color: Color,
) -> Result<()> {
    validate_page_index(page, pages.len())?;
    validate_rect(bounds, &pages[page])?;
    validate_color(color)?;
    if contents.is_empty() {
        return Err(PdfMasterError::InvalidOperation(
            "annotation contents cannot be empty",
        ));
    }
    if contents.len() > MAX_TEXT_BYTES {
        return Err(PdfMasterError::Budget("annotation text length"));
    }
    reject_rotated_edit(&pages[page])?;

    let page_id = page_object_ids(document)?[page];
    let mut annots = validated_annots_for_add(document, page_id, &pages[page])?;

    let appearance_content = format!(
        "q {} {} {} rg 0 0 {} {} re f 0 0 0 RG 1 w 0.5 0.5 {} {} re S Q\n",
        color.r,
        color.g,
        color.b,
        bounds.width,
        bounds.height,
        (bounds.width - 1.0).max(0.0),
        (bounds.height - 1.0).max(0.0),
    )
    .into_bytes();
    let appearance = Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "FormType" => 1_i64,
            "BBox" => vec![0.into(), 0.into(), bounds.width.into(), bounds.height.into()],
            "Resources" => Dictionary::new(),
        },
        appearance_content,
    );
    let appearance_id = document.add_object(appearance);
    let annotation_id = document.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Text",
        "Rect" => vec![
            bounds.x.into(),
            bounds.y.into(),
            (bounds.x + bounds.width).into(),
            (bounds.y + bounds.height).into(),
        ],
        "Contents" => pdf_text_string(contents),
        "C" => vec![color.r.into(), color.g.into(), color.b.into()],
        "F" => 4_i64,
        "Name" => "Comment",
        "AP" => dictionary! { "N" => appearance_id },
    });

    annots.push(annotation_id.into());
    document
        .get_object_mut(page_id)
        .and_then(Object::as_dict_mut)
        .map_err(|error| PdfMasterError::Malformed(error.to_string()))?
        .set("Annots", annots);
    validate_notes_page(document, page_id, &pages[page])
}

fn validate_rect(bounds: Rect, page: &PageInfo) -> Result<()> {
    if !bounds.x.is_finite()
        || !bounds.y.is_finite()
        || !bounds.width.is_finite()
        || !bounds.height.is_finite()
        || bounds.x < 0.0
        || bounds.y < 0.0
        || bounds.width <= 0.0
        || bounds.height <= 0.0
        || bounds.x + bounds.width > page.width
        || bounds.y + bounds.height > page.height
    {
        return Err(PdfMasterError::InvalidGeometry(
            "rectangle must be finite, positive, and inside its page",
        ));
    }
    Ok(())
}

fn reject_rotated_edit(page: &PageInfo) -> Result<()> {
    if page.rotation != 0 {
        return Err(PdfMasterError::InvalidOperation(
            "adding content to rotated pages is unsupported in this foundation",
        ));
    }
    Ok(())
}

fn validate_color(color: Color) -> Result<()> {
    if [color.r, color.g, color.b]
        .into_iter()
        .any(|component| !component.is_finite() || !(0.0..=1.0).contains(&component))
    {
        return Err(PdfMasterError::InvalidGeometry(
            "color components must be between zero and one",
        ));
    }
    Ok(())
}

fn root_pages_id(document: &Document) -> Result<ObjectId> {
    document
        .catalog()
        .and_then(|catalog| catalog.get(b"Pages"))
        .and_then(Object::as_reference)
        .map_err(|error| PdfMasterError::Malformed(error.to_string()))
}

pub(crate) fn flatten_page_tree(document: &mut Document, ids: &[ObjectId]) -> Result<()> {
    let root = root_pages_id(document)?;
    materialize_page_attributes(document, ids)?;
    for id in ids {
        document
            .get_object_mut(*id)
            .and_then(Object::as_dict_mut)
            .map_err(|error| PdfMasterError::Malformed(error.to_string()))?
            .set("Parent", root);
    }
    let root_dictionary = document
        .get_object_mut(root)
        .and_then(Object::as_dict_mut)
        .map_err(|error| PdfMasterError::Malformed(error.to_string()))?;
    root_dictionary.set("Type", "Pages");
    root_dictionary.set(
        "Kids",
        ids.iter()
            .copied()
            .map(Object::Reference)
            .collect::<Vec<_>>(),
    );
    root_dictionary.set("Count", ids.len() as i64);
    Ok(())
}

fn install_font(document: &mut Document, page_id: ObjectId) -> Result<Vec<u8>> {
    let resources = effective_resources(document, page_id)?;
    let font_id = document.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let mut resources = resources;
    let mut fonts = match resources.get(b"Font") {
        Ok(Object::Dictionary(dictionary)) => dictionary.clone(),
        Ok(Object::Reference(id)) => document
            .get_object(*id)
            .and_then(Object::as_dict)
            .map_err(|error| PdfMasterError::Malformed(error.to_string()))?
            .clone(),
        Ok(_) => {
            return Err(PdfMasterError::Malformed(
                "Font resource is not a dictionary".into(),
            ));
        }
        Err(_) => Dictionary::new(),
    };
    let mut suffix = 0usize;
    let name = loop {
        let candidate = if suffix == 0 {
            b"FPM".to_vec()
        } else {
            format!("FPM{suffix}").into_bytes()
        };
        if !fonts.has(&candidate) {
            break candidate;
        }
        suffix += 1;
    };
    fonts.set(name.clone(), font_id);
    resources.set("Font", fonts);
    document
        .get_object_mut(page_id)
        .and_then(Object::as_dict_mut)
        .map_err(|error| PdfMasterError::Malformed(error.to_string()))?
        .set("Resources", resources);
    Ok(name)
}

fn append_page_contents(
    document: &mut Document,
    page_id: ObjectId,
    content: Vec<u8>,
) -> Result<()> {
    let existing = document
        .get_object(page_id)
        .and_then(Object::as_dict)
        .map_err(|error| PdfMasterError::Malformed(error.to_string()))?
        .get(b"Contents")
        .ok()
        .cloned();
    let mut streams = match existing {
        None | Some(Object::Null) => Vec::new(),
        Some(Object::Reference(id)) => match document
            .get_object(id)
            .map_err(|error| PdfMasterError::Malformed(error.to_string()))?
        {
            Object::Stream(_) => vec![Object::Reference(id)],
            Object::Array(values) => normalize_content_entries(document, values.clone())?,
            _ => {
                return Err(PdfMasterError::Malformed(
                    "indirect Contents must resolve to a stream or stream array".into(),
                ));
            }
        },
        Some(Object::Stream(stream)) => {
            let id = document.add_object(stream);
            vec![Object::Reference(id)]
        }
        Some(Object::Array(values)) => normalize_content_entries(document, values)?,
        Some(_) => {
            return Err(PdfMasterError::Malformed(
                "Contents must be a stream or stream array".into(),
            ));
        }
    };
    let content_id = document.add_object(Stream::new(Dictionary::new(), content));
    streams.push(Object::Reference(content_id));
    document
        .get_object_mut(page_id)
        .and_then(Object::as_dict_mut)
        .map_err(|error| PdfMasterError::Malformed(error.to_string()))?
        .set("Contents", streams);
    Ok(())
}

fn normalize_content_entries(document: &mut Document, values: Vec<Object>) -> Result<Vec<Object>> {
    let mut normalized = Vec::with_capacity(values.len());
    for value in values {
        match value {
            Object::Reference(id) => {
                document
                    .get_object(id)
                    .and_then(Object::as_stream)
                    .map_err(|_| {
                        PdfMasterError::Malformed(
                            "Contents array references must resolve to streams".into(),
                        )
                    })?;
                normalized.push(Object::Reference(id));
            }
            Object::Stream(stream) => {
                let id = document.add_object(stream);
                normalized.push(Object::Reference(id));
            }
            _ => {
                return Err(PdfMasterError::Malformed(
                    "Contents array entries must be streams".into(),
                ));
            }
        }
    }
    Ok(normalized)
}

fn pdf_text_string(value: &str) -> Object {
    if value.is_ascii() {
        return Object::string_literal(value);
    }
    let mut bytes = vec![0xFE, 0xFF];
    for unit in value.encode_utf16() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    Object::String(bytes, StringFormat::Hexadecimal)
}

fn effective_resources(document: &Document, mut id: ObjectId) -> Result<Dictionary> {
    for _ in 0..64 {
        let dictionary = document
            .get_object(id)
            .and_then(Object::as_dict)
            .map_err(|error| PdfMasterError::Malformed(error.to_string()))?;
        if let Ok(resources) = dictionary.get(b"Resources") {
            return match resources {
                Object::Dictionary(resources) => Ok(resources.clone()),
                Object::Reference(id) => document
                    .get_object(*id)
                    .and_then(Object::as_dict)
                    .cloned()
                    .map_err(|error| PdfMasterError::Malformed(error.to_string())),
                _ => Err(PdfMasterError::Malformed(
                    "Resources entry is not a dictionary".into(),
                )),
            };
        }
        match dictionary.get(b"Parent").and_then(Object::as_reference) {
            Ok(parent) => id = parent,
            Err(_) => return Ok(Dictionary::new()),
        }
    }
    Err(PdfMasterError::Malformed(
        "page resource inheritance depth exceeded".into(),
    ))
}
