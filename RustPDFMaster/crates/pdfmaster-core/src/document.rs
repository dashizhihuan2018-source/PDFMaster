use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use lopdf::{Dictionary, Document, LoadOptions, Object, ObjectId, Stream, dictionary};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::edit::apply_edit;

pub(crate) const MAX_INPUT_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const MAX_PAGES: usize = 4_096;
pub(crate) const MAX_PAGE_DIMENSION: f32 = 20_000.0;
pub(crate) const MAX_HISTORY: usize = 32;
pub(crate) const MAX_HISTORY_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MAX_TEXT_BYTES: usize = 64 * 1024;
pub(crate) const MAX_DECOMPRESSED_TEXT_BYTES: usize = 8 * 1024 * 1024;

/// Errors deliberately exposed as stable, displayable failures to platform shells.
#[derive(Debug, Error)]
pub enum PdfMasterError {
    #[error("malformed PDF: {0}")]
    Malformed(String),
    #[error("encrypted PDFs cannot be edited")]
    Encrypted,
    #[error("signed PDFs cannot be edited without invalidating signatures")]
    Signed,
    #[error("resource budget exceeded: {0}")]
    Budget(&'static str),
    #[error("invalid page index {index}; document has {page_count} pages")]
    InvalidPage { index: usize, page_count: usize },
    #[error("invalid geometry: {0}")]
    InvalidGeometry(&'static str),
    #[error("invalid operation: {0}")]
    InvalidOperation(&'static str),
    #[error("no undo snapshot is available")]
    NoUndo,
    #[error("no redo snapshot is available")]
    NoRedo,
    #[error("PDF serialization failed: {0}")]
    Serialization(String),
    #[error("PDF rendering failed: {0}")]
    Render(String),
}

pub type Result<T> = std::result::Result<T, PdfMasterError>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageInfo {
    pub width: f32,
    pub height: f32,
    pub rotation: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DocumentInfo {
    pub page_count: usize,
    pub pages: Vec<PageInfo>,
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapeKind {
    Rectangle,
    Ellipse,
    Line,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Edit {
    Rotate {
        page: usize,
        degrees: i32,
    },
    RotatePages {
        pages: Vec<usize>,
        degrees: i32,
    },
    MovePage {
        from: usize,
        to: usize,
    },
    /// Move the selection, in source order, before a boundary in the original
    /// page sequence. `insertion == page_count` appends the selection.
    MovePages {
        pages: Vec<usize>,
        insertion: usize,
    },
    InsertBlank {
        index: usize,
        width: f32,
        height: f32,
    },
    DuplicatePage {
        page: usize,
        index: usize,
    },
    DeletePage {
        page: usize,
    },
    DeletePages {
        pages: Vec<usize>,
    },
    AddText {
        page: usize,
        bounds: Rect,
        text: String,
        font_size: f32,
        color: Color,
    },
    AddNote {
        page: usize,
        bounds: Rect,
        contents: String,
        color: Color,
    },
    AddShape {
        page: usize,
        bounds: Rect,
        shape: ShapeKind,
        stroke: Color,
        fill: Option<Color>,
        line_width: f32,
    },
    AddLine {
        page: usize,
        start: crate::lines::Point,
        end: crate::lines::Point,
        endings: [crate::lines::LineEnding; 2],
        stroke: Color,
        fill: Option<Color>,
        line_width: f32,
    },
    AddInk {
        page: usize,
        strokes: Vec<Vec<crate::Point>>,
        color: Color,
        line_width: f32,
        #[serde(default)]
        smoothed: bool,
    },
    UpdateShape {
        page: usize,
        expected: crate::shapes::ShapeAnnotation,
        bounds: Rect,
        stroke: Color,
        fill: Option<Color>,
        line_width: f32,
    },
    DeleteShape {
        page: usize,
        expected: crate::shapes::ShapeAnnotation,
    },
    UpdateNote {
        page: usize,
        annotation_index: usize,
        expected_contents: String,
        contents: String,
    },
    DeleteNote {
        page: usize,
        annotation_index: usize,
        expected_contents: String,
    },
    DeleteAnnotation {
        expected: Box<crate::annotations::AnnotationRow>,
    },
}

#[derive(Debug)]
pub struct Session {
    pub(crate) document: Document,
    pub(crate) current_bytes: Vec<u8>,
    pub(crate) pages: Vec<PageInfo>,
    saved_bytes: Vec<u8>,
    undo: Vec<Vec<u8>>,
    redo: Vec<Vec<u8>>,
    text_cache: OnceLock<TextReader>,
}

/// Immutable, reusable text-extraction view. This owns one document snapshot;
/// searching many pages must not clone the complete PDF for every page.
#[derive(Clone, Debug)]
pub struct TextReader {
    state: Arc<Mutex<TextState>>,
    page_count: usize,
}

#[derive(Debug)]
struct TextState {
    document: Document,
    page_ids: Vec<ObjectId>,
    normalized: HashSet<ObjectId>,
    direct_resources: HashMap<ObjectId, ObjectId>,
}

impl TextReader {
    pub fn text(&self, page: usize) -> Result<String> {
        validate_page_index(page, self.page_count)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| PdfMasterError::Malformed("text snapshot lock is unavailable".into()))?;
        state.normalize_page(page)?;
        state
            .document
            .extract_text_with_limit(&[(page + 1) as u32], MAX_DECOMPRESSED_TEXT_BYTES)
            .map_err(|error| PdfMasterError::Malformed(error.to_string()))
    }
}

impl TextState {
    fn normalize_page(&mut self, index: usize) -> Result<()> {
        let id = self.page_ids[index];
        if self.normalized.contains(&id) {
            return Ok(());
        }
        let mut owner = id;
        let mut seen = HashSet::new();
        let mut selected = None;
        for _ in 0..64 {
            if !seen.insert(owner) {
                return Err(PdfMasterError::Malformed(
                    "text resource inheritance cycle".into(),
                ));
            }
            let dictionary = self
                .document
                .get_object(owner)
                .and_then(Object::as_dict)
                .map_err(|error| PdfMasterError::Malformed(error.to_string()))?;
            let resources = dictionary.get(b"Resources").ok().cloned();
            let parent = dictionary.get(b"Parent").ok().cloned();
            if let Some(resources) = resources {
                let resolved = dereference(&self.document, &resources)?;
                if !matches!(resolved, Object::Null) {
                    resolved
                        .as_dict()
                        .map_err(|error| PdfMasterError::Malformed(error.to_string()))?;
                    let resource_id = if let Object::Reference(id) = resources {
                        id
                    } else if let Some(id) = self.direct_resources.get(&owner) {
                        *id
                    } else {
                        self.document
                            .max_id
                            .checked_add(1)
                            .ok_or(PdfMasterError::Budget("text normalization object IDs"))?;
                        let id = self.document.add_object(resources);
                        self.direct_resources.insert(owner, id);
                        id
                    };
                    selected = Some(Object::Reference(resource_id));
                    break;
                }
            }
            match parent {
                Some(parent) if !matches!(dereference(&self.document, &parent)?, Object::Null) => {
                    owner = parent
                        .as_reference()
                        .map_err(|error| PdfMasterError::Malformed(error.to_string()))?;
                }
                _ => {
                    selected = Some(Object::Dictionary(Dictionary::new()));
                    break;
                }
            }
        }
        let selected = selected.ok_or_else(|| {
            PdfMasterError::Malformed("text resource inheritance depth exceeded".into())
        })?;
        let page = self
            .document
            .get_object_mut(id)
            .and_then(Object::as_dict_mut)
            .map_err(|error| PdfMasterError::Malformed(error.to_string()))?;
        page.set("Resources", selected);
        // lopdf merges ancestor fonts, but PDF Resources inherits as a whole.
        // Only this never-saved snapshot is modified; Kids traversal is unchanged.
        page.remove(b"Parent");
        self.normalized.insert(id);
        Ok(())
    }
}

impl Session {
    pub fn open(bytes: &[u8]) -> Result<Self> {
        let (document, pages) = parse_document(bytes)?;
        Ok(Self {
            document,
            current_bytes: bytes.to_vec(),
            pages,
            saved_bytes: bytes.to_vec(),
            undo: Vec::new(),
            redo: Vec::new(),
            text_cache: OnceLock::new(),
        })
    }

    pub fn blank(width: f32, height: f32, pages: usize) -> Result<Self> {
        validate_page_size(width, height)?;
        if pages == 0 {
            return Err(PdfMasterError::InvalidOperation(
                "a document must contain at least one page",
            ));
        }
        if pages > MAX_PAGES {
            return Err(PdfMasterError::Budget("page count"));
        }

        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let mut page_ids = Vec::with_capacity(pages);
        for _ in 0..pages {
            page_ids.push(add_blank_page(&mut document, pages_id, width, height));
        }
        document.objects.insert(
            pages_id,
            dictionary! {
                "Type" => "Pages",
                "Kids" => page_ids.iter().copied().map(Object::Reference).collect::<Vec<_>>(),
                "Count" => pages as i64,
            }
            .into(),
        );
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog_id);
        let bytes = serialize(&document)?;
        Self::open(&bytes)
    }

    pub fn info(&self) -> DocumentInfo {
        DocumentInfo {
            page_count: self.pages.len(),
            pages: self.pages.clone(),
            dirty: self.is_dirty(),
            can_undo: !self.undo.is_empty(),
            can_redo: !self.redo.is_empty(),
        }
    }

    pub fn bytes(&self) -> Result<Vec<u8>> {
        Ok(self.current_bytes.clone())
    }

    pub fn apply(&mut self, edit: Edit) -> Result<()> {
        let mut candidate = self.document.clone();
        let structural = matches!(
            &edit,
            Edit::MovePage { .. }
                | Edit::MovePages { .. }
                | Edit::InsertBlank { .. }
                | Edit::DuplicatePage { .. }
                | Edit::DeletePage { .. }
                | Edit::DeletePages { .. }
        );
        let duplicate = match &edit {
            Edit::DuplicatePage { page, index } => Some((*page, *index)),
            _ => None,
        };
        apply_edit(&mut candidate, &self.pages, edit)?;
        if structural {
            crate::page_labels::migrate_edit_labels(&self.document, &mut candidate, duplicate)?;
        }
        self.commit_document(candidate)
    }

    pub(crate) fn commit_document(&mut self, candidate: Document) -> Result<()> {
        let bytes = serialize(&candidate)?;
        if bytes.len() > MAX_INPUT_BYTES {
            return Err(PdfMasterError::Budget("serialized document size"));
        }
        let (candidate, pages) = parse_document(&bytes)?;

        self.redo.clear();
        push_bounded(&mut self.undo, &mut self.redo, self.current_bytes.clone());
        self.document = candidate;
        self.current_bytes = bytes;
        self.pages = pages;
        self.text_cache.take();
        Ok(())
    }

    pub fn undo(&mut self) -> Result<()> {
        let bytes = self.undo.last().ok_or(PdfMasterError::NoUndo)?.clone();
        let (document, pages) = parse_document(&bytes)?;
        self.undo.pop();
        push_bounded(&mut self.redo, &mut self.undo, self.current_bytes.clone());
        self.document = document;
        self.current_bytes = bytes;
        self.pages = pages;
        self.text_cache.take();
        Ok(())
    }

    pub fn redo(&mut self) -> Result<()> {
        let bytes = self.redo.last().ok_or(PdfMasterError::NoRedo)?.clone();
        let (document, pages) = parse_document(&bytes)?;
        self.redo.pop();
        push_bounded(&mut self.undo, &mut self.redo, self.current_bytes.clone());
        self.document = document;
        self.current_bytes = bytes;
        self.pages = pages;
        self.text_cache.take();
        Ok(())
    }

    pub fn is_dirty(&self) -> bool {
        self.current_bytes != self.saved_bytes
    }

    pub fn mark_saved(&mut self) {
        self.saved_bytes.clone_from(&self.current_bytes);
    }

    pub fn text(&self, page: usize) -> Result<String> {
        validate_page_index(page, self.pages.len())?;
        self.text_reader()?.text(page)
    }

    pub fn text_reader(&self) -> Result<TextReader> {
        if let Some(reader) = self.text_cache.get() {
            return Ok(reader.clone());
        }
        let ids = page_object_ids(&self.document)?;
        let reader = TextReader {
            page_count: ids.len(),
            state: Arc::new(Mutex::new(TextState {
                document: self.document.clone(),
                page_ids: ids,
                normalized: HashSet::new(),
                direct_resources: HashMap::new(),
            })),
        };
        let _ = self.text_cache.set(reader.clone());
        Ok(self.text_cache.get().cloned().unwrap_or(reader))
    }
}

pub(crate) fn validate_page_index(page: usize, page_count: usize) -> Result<()> {
    if page >= page_count {
        return Err(PdfMasterError::InvalidPage {
            index: page,
            page_count,
        });
    }
    Ok(())
}

pub(crate) fn validate_page_size(width: f32, height: f32) -> Result<()> {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err(PdfMasterError::InvalidGeometry(
            "page dimensions must be finite and positive",
        ));
    }
    if width > MAX_PAGE_DIMENSION || height > MAX_PAGE_DIMENSION {
        return Err(PdfMasterError::Budget("page dimensions"));
    }
    Ok(())
}

pub(crate) fn add_blank_page(
    document: &mut Document,
    parent: ObjectId,
    width: f32,
    height: f32,
) -> ObjectId {
    let content_id = document.add_object(Stream::new(Dictionary::new(), Vec::new()));
    document.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => parent,
        "MediaBox" => vec![0.into(), 0.into(), width.into(), height.into()],
        "Resources" => Dictionary::new(),
        "Contents" => content_id,
    })
}

pub(crate) fn page_object_ids(document: &Document) -> Result<Vec<ObjectId>> {
    validate_page_tree(document)
}

pub(crate) fn serialize(document: &Document) -> Result<Vec<u8>> {
    let mut clone = document.clone();
    let mut bytes = Vec::new();
    clone
        .save_to(&mut bytes)
        .map_err(|error| PdfMasterError::Serialization(error.to_string()))?;
    Ok(bytes)
}

fn parse_document(bytes: &[u8]) -> Result<(Document, Vec<PageInfo>)> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(PdfMasterError::Budget("input size"));
    }
    let document = Document::load_mem_with_options(
        bytes,
        LoadOptions {
            strict: true,
            max_decompressed_size: Some(MAX_DECOMPRESSED_TEXT_BYTES),
            ..Default::default()
        },
    )
    .map_err(|error| PdfMasterError::Malformed(error.to_string()))?;
    if document.is_encrypted() || document.was_encrypted() {
        return Err(PdfMasterError::Encrypted);
    }
    if has_signature(&document) {
        return Err(PdfMasterError::Signed);
    }
    let ids = page_object_ids(&document)?;
    if ids.is_empty() {
        return Err(PdfMasterError::Malformed("document has no pages".into()));
    }
    if ids.len() > MAX_PAGES {
        return Err(PdfMasterError::Budget("page count"));
    }
    let pages = ids
        .into_iter()
        .map(|id| page_info(&document, id))
        .collect::<Result<Vec<_>>>()?;
    Ok((document, pages))
}

fn has_signature(document: &Document) -> bool {
    document.objects.values().any(|object| {
        let dictionary = match object {
            Object::Dictionary(dictionary) => dictionary,
            Object::Stream(stream) => &stream.dict,
            _ => return false,
        };
        dictionary.has(b"ByteRange")
            || dictionary
                .get(b"Type")
                .and_then(Object::as_name)
                .is_ok_and(|name| name == b"Sig")
            || dictionary
                .get(b"FT")
                .and_then(Object::as_name)
                .is_ok_and(|name| name == b"Sig")
    })
}

fn page_info(document: &Document, page_id: ObjectId) -> Result<PageInfo> {
    let media = page_box(
        document,
        inherited_value(document, page_id, b"MediaBox")?,
        "MediaBox",
    )?;
    if media[0] != 0.0 || media[1] != 0.0 {
        return Err(PdfMasterError::Malformed(
            "nonzero page origin is unsupported in this foundation".into(),
        ));
    }
    if media[2] <= media[0] || media[3] <= media[1] {
        return Err(PdfMasterError::Malformed(
            "page MediaBox must have increasing coordinates".into(),
        ));
    }
    if let Some(crop) = inherited_optional(document, page_id, b"CropBox")? {
        let crop = page_box(document, crop, "CropBox")?;
        if crop != media {
            return Err(PdfMasterError::Malformed(
                "a CropBox different from MediaBox is unsupported in this foundation".into(),
            ));
        }
    }
    let width = media[2] - media[0];
    let height = media[3] - media[1];
    validate_page_size(width, height)?;
    let rotation = match inherited_optional(document, page_id, b"Rotate")? {
        Some(object) => dereference(document, object)?
            .as_i64()
            .map_err(|_| PdfMasterError::Malformed("page rotation must be an integer".into()))?,
        None => 0,
    }
    .rem_euclid(360) as i32;
    if rotation % 90 != 0 {
        return Err(PdfMasterError::Malformed(
            "page rotation must be a multiple of 90 degrees".into(),
        ));
    }
    Ok(PageInfo {
        width,
        height,
        rotation,
    })
}

fn dereference<'a>(document: &'a Document, object: &'a Object) -> Result<&'a Object> {
    let mut current = object;
    let mut seen = HashSet::new();
    for _ in 0..64 {
        match current {
            Object::Reference(id) => {
                if !seen.insert(*id) {
                    return Err(PdfMasterError::Malformed(
                        "indirect object reference cycle".into(),
                    ));
                }
                current = document
                    .get_object(*id)
                    .map_err(|error| PdfMasterError::Malformed(error.to_string()))?;
            }
            value => return Ok(value),
        }
    }
    Err(PdfMasterError::Malformed(
        "indirect object reference depth exceeded".into(),
    ))
}

pub(crate) fn inherited_optional<'a>(
    document: &'a Document,
    mut id: ObjectId,
    key: &[u8],
) -> Result<Option<&'a Object>> {
    let mut seen = HashSet::new();
    for _ in 0..64 {
        if !seen.insert(id) {
            return Err(PdfMasterError::Malformed("page inheritance cycle".into()));
        }
        let dictionary = document
            .get_object(id)
            .and_then(Object::as_dict)
            .map_err(|error| PdfMasterError::Malformed(error.to_string()))?;
        if let Ok(value) = dictionary.get(key) {
            return Ok(Some(value));
        }
        match dictionary.get(b"Parent") {
            Ok(parent) => {
                id = parent.as_reference().map_err(|_| {
                    PdfMasterError::Malformed("page Parent must be an indirect reference".into())
                })?;
            }
            Err(_) => return Ok(None),
        }
    }
    Err(PdfMasterError::Malformed(
        "page inheritance depth exceeded".into(),
    ))
}

fn inherited_value<'a>(document: &'a Document, id: ObjectId, key: &[u8]) -> Result<&'a Object> {
    inherited_optional(document, id, key)?
        .ok_or_else(|| PdfMasterError::Malformed("missing inherited page property".into()))
}

fn page_box(document: &Document, object: &Object, name: &str) -> Result<[f32; 4]> {
    let values = dereference(document, object)?
        .as_array()
        .map_err(|_| PdfMasterError::Malformed(format!("page {name} must be an array")))?;
    if values.len() != 4 {
        return Err(PdfMasterError::Malformed(format!(
            "page {name} must contain four numbers"
        )));
    }
    let mut result = [0.0; 4];
    for (index, value) in values.iter().enumerate() {
        result[index] = dereference(document, value)?.as_float().map_err(|_| {
            PdfMasterError::Malformed(format!("page {name} values must be numbers"))
        })?;
        if !result[index].is_finite() {
            return Err(PdfMasterError::Malformed(format!(
                "page {name} values must be finite"
            )));
        }
    }
    Ok(result)
}

pub(crate) fn materialize_page_attributes(
    document: &mut Document,
    page_ids: &[ObjectId],
) -> Result<()> {
    for &page_id in page_ids {
        let media_box =
            dereference(document, inherited_value(document, page_id, b"MediaBox")?)?.clone();
        let crop_box = inherited_optional(document, page_id, b"CropBox")?
            .map(|value| dereference(document, value).cloned())
            .transpose()?;
        let rotation = inherited_optional(document, page_id, b"Rotate")?
            .map(|value| dereference(document, value).cloned())
            .transpose()?;
        let resources = inherited_optional(document, page_id, b"Resources")?
            .map(|value| dereference(document, value).cloned())
            .transpose()?
            .unwrap_or_else(|| Object::Dictionary(Dictionary::new()));
        let page = document
            .get_object_mut(page_id)
            .and_then(Object::as_dict_mut)
            .map_err(|error| PdfMasterError::Malformed(error.to_string()))?;
        page.set("MediaBox", media_box);
        page.set("Resources", resources);
        if let Some(crop_box) = crop_box {
            page.set("CropBox", crop_box);
        }
        if let Some(rotation) = rotation {
            page.set("Rotate", rotation);
        }
    }
    Ok(())
}

fn validate_page_tree(document: &Document) -> Result<Vec<ObjectId>> {
    let root = document
        .catalog()
        .and_then(|catalog| catalog.get(b"Pages"))
        .and_then(Object::as_reference)
        .map_err(|_| {
            PdfMasterError::Malformed("Catalog Pages must be an indirect reference".into())
        })?;
    let root_dictionary = document
        .get_object(root)
        .and_then(Object::as_dict)
        .map_err(|error| PdfMasterError::Malformed(error.to_string()))?;
    let root_type = root_dictionary
        .get(b"Type")
        .and_then(Object::as_name)
        .map_err(|_| PdfMasterError::Malformed("page tree root must have Type Pages".into()))?;
    if root_type != b"Pages" {
        return Err(PdfMasterError::Malformed(
            "page tree root must have Type Pages".into(),
        ));
    }
    if root_dictionary.has(b"Parent") {
        return Err(PdfMasterError::Malformed(
            "page tree root must not have a Parent".into(),
        ));
    }
    let mut seen = HashSet::new();
    let mut active = HashSet::new();
    let mut pages = Vec::new();
    let count = visit_page_tree(document, root, None, 0, &mut seen, &mut active, &mut pages)?;
    if count == 0 {
        return Err(PdfMasterError::Malformed("document has no pages".into()));
    }
    Ok(pages)
}

fn visit_page_tree(
    document: &Document,
    id: ObjectId,
    expected_parent: Option<ObjectId>,
    depth: usize,
    seen: &mut HashSet<ObjectId>,
    active: &mut HashSet<ObjectId>,
    pages: &mut Vec<ObjectId>,
) -> Result<usize> {
    if depth > 64 {
        return Err(PdfMasterError::Malformed("page tree depth exceeded".into()));
    }
    if active.contains(&id) {
        return Err(PdfMasterError::Malformed("page tree cycle".into()));
    }
    if !seen.insert(id) {
        return Err(PdfMasterError::Malformed(
            "duplicate page tree reference".into(),
        ));
    }
    active.insert(id);
    let dictionary = document
        .get_object(id)
        .and_then(Object::as_dict)
        .map_err(|error| PdfMasterError::Malformed(error.to_string()))?;
    if let Some(parent) = expected_parent {
        let actual = dictionary
            .get(b"Parent")
            .and_then(Object::as_reference)
            .map_err(|_| PdfMasterError::Malformed("page tree node has invalid Parent".into()))?;
        if actual != parent {
            return Err(PdfMasterError::Malformed(
                "page tree Parent mismatch".into(),
            ));
        }
    }
    let node_type = dictionary
        .get(b"Type")
        .and_then(Object::as_name)
        .map_err(|_| PdfMasterError::Malformed("page tree node has invalid Type".into()))?;
    let count = match node_type {
        b"Page" => {
            if pages.len() == MAX_PAGES {
                return Err(PdfMasterError::Budget("page count"));
            }
            pages.push(id);
            1
        }
        b"Pages" => {
            let declared = dictionary
                .get(b"Count")
                .and_then(Object::as_i64)
                .map_err(|_| PdfMasterError::Malformed("Pages Count must be an integer".into()))?;
            if declared < 0 {
                return Err(PdfMasterError::Malformed(
                    "Pages Count cannot be negative".into(),
                ));
            }
            if declared as usize > MAX_PAGES {
                return Err(PdfMasterError::Budget("page count"));
            }
            let kids = dereference(
                document,
                dictionary
                    .get(b"Kids")
                    .map_err(|_| PdfMasterError::Malformed("Pages node is missing Kids".into()))?,
            )?
            .as_array()
            .map_err(|_| PdfMasterError::Malformed("Pages Kids must be an array".into()))?;
            let mut actual = 0usize;
            for kid in kids {
                let kid = kid.as_reference().map_err(|_| {
                    PdfMasterError::Malformed("page tree Kids must be indirect references".into())
                })?;
                actual = actual
                    .checked_add(visit_page_tree(
                        document,
                        kid,
                        Some(id),
                        depth + 1,
                        seen,
                        active,
                        pages,
                    )?)
                    .ok_or(PdfMasterError::Budget("page count"))?;
                if actual > MAX_PAGES {
                    return Err(PdfMasterError::Budget("page count"));
                }
            }
            if declared as usize != actual {
                return Err(PdfMasterError::Malformed(format!(
                    "Pages Count mismatch: declared {declared}, found {actual}"
                )));
            }
            actual
        }
        _ => {
            return Err(PdfMasterError::Malformed(
                "unknown page tree node Type".into(),
            ));
        }
    };
    active.remove(&id);
    Ok(count)
}

fn push_bounded(target: &mut Vec<Vec<u8>>, other: &mut Vec<Vec<u8>>, bytes: Vec<u8>) {
    target.push(bytes);
    while target.len() > MAX_HISTORY {
        target.remove(0);
    }
    while history_bytes(target, other) > MAX_HISTORY_BYTES {
        if !other.is_empty() {
            other.remove(0);
        } else if target.len() > 1 {
            target.remove(0);
        } else {
            break;
        }
    }
}

fn history_bytes(first: &[Vec<u8>], second: &[Vec<u8>]) -> usize {
    first
        .iter()
        .chain(second)
        .fold(0usize, |total, bytes| total.saturating_add(bytes.len()))
}

#[cfg(test)]
mod text_reader_tests {
    use super::*;
    #[test]
    fn text_snapshot_is_cached_per_successful_document_revision() {
        let mut session = Session::blank(100., 100., 1).unwrap();
        let first = session.text_reader().unwrap();
        session.text(0).unwrap();
        let second = session.text_reader().unwrap();
        assert!(std::sync::Arc::ptr_eq(&first.state, &second.state));
        assert!(session.apply(Edit::DeletePage { page: 2 }).is_err());
        session.mark_saved();
        assert!(std::sync::Arc::ptr_eq(
            &first.state,
            &session.text_reader().unwrap().state
        ));
        session
            .apply(Edit::Rotate {
                page: 0,
                degrees: 90,
            })
            .unwrap();
        let edited = session.text_reader().unwrap();
        assert!(!std::sync::Arc::ptr_eq(&first.state, &edited.state));
        session.undo().unwrap();
        let undone = session.text_reader().unwrap();
        assert!(!std::sync::Arc::ptr_eq(&edited.state, &undone.state));
        session.redo().unwrap();
        assert!(!std::sync::Arc::ptr_eq(
            &undone.state,
            &session.text_reader().unwrap().state
        ));
        assert_eq!(first.text(0).unwrap(), "");
    }
    #[test]
    fn exhausted_object_counter_cannot_panic_during_read_only_normalization() {
        let mut session = Session::blank(100., 100., 1).unwrap();
        session.document.max_id = u32::MAX;
        let before = session.bytes().unwrap();
        assert!(session.text(0).is_err());
        assert_eq!(session.bytes().unwrap(), before);
    }
}
