use std::collections::{HashMap, HashSet};

use lopdf::{Dictionary, Document, Object, ObjectId, decode_text_string};
use serde::{Deserialize, Serialize};

use crate::{PdfMasterError, Result, Session, document::page_object_ids};

const MAX_ITEMS: usize = 4096;
const MAX_DEPTH: usize = 128;
const MAX_TEXT: usize = 8192;
const MAX_TOTAL_TEXT: usize = 1024 * 1024;

fn malformed(message: &str) -> PdfMasterError {
    PdfMasterError::Malformed(message.into())
}

fn resolve<'a>(doc: &'a Document, mut value: &'a Object) -> Result<(&'a Object, Option<ObjectId>)> {
    let mut seen = HashSet::new();
    let mut last = None;
    for _ in 0..64 {
        match value {
            Object::Reference(id) => {
                if !seen.insert(*id) {
                    return Err(malformed("outline object reference cycle"));
                }
                last = Some(*id);
                value = doc
                    .objects
                    .get(id)
                    .ok_or_else(|| malformed("missing outline object"))?;
            }
            _ => return Ok((value, last)),
        }
    }
    Err(PdfMasterError::Budget("outline reference depth"))
}

fn dict(value: &Object) -> Result<&Dictionary> {
    value
        .as_dict()
        .map_err(|_| malformed("outline dictionary expected"))
}

#[derive(Default)]
struct Names<'a> {
    legacy: HashMap<&'a [u8], &'a Object>,
    modern: HashMap<&'a [u8], &'a Object>,
    bytes: usize,
}

impl<'a> Names<'a> {
    fn add(&mut self, key: &'a [u8], value: &'a Object, modern: bool) -> Result<()> {
        if key.len() > MAX_TEXT || self.legacy.len() + self.modern.len() >= MAX_ITEMS {
            return Err(PdfMasterError::Budget("outline named destinations"));
        }
        self.bytes = self
            .bytes
            .checked_add(key.len())
            .ok_or(PdfMasterError::Budget("outline name bytes"))?;
        if self.bytes > MAX_TOTAL_TEXT {
            return Err(PdfMasterError::Budget("outline name bytes"));
        }
        if (if modern {
            &mut self.modern
        } else {
            &mut self.legacy
        })
        .insert(key, value)
        .is_some()
        {
            return Err(malformed("duplicate outline destination name"));
        }
        Ok(())
    }

    fn load(doc: &'a Document, catalog: &'a Dictionary) -> Result<Self> {
        let mut names = Self::default();
        if let Ok(legacy) = catalog.get(b"Dests") {
            for (key, value) in dict(resolve(doc, legacy)?.0)?.iter() {
                names.add(key, value, false)?;
            }
        }
        let Some(root) = catalog
            .get(b"Names")
            .ok()
            .map(|value| resolve(doc, value).and_then(|(value, _)| dict(value)))
            .transpose()?
            .and_then(|names| names.get(b"Dests").ok())
        else {
            return Ok(names);
        };
        let mut pending = vec![(root, 0)];
        let mut scheduled: usize = 1;
        let mut visited = HashSet::new();
        while let Some((value, depth)) = pending.pop() {
            if depth >= MAX_DEPTH {
                return Err(PdfMasterError::Budget("outline name tree depth"));
            }
            let (value, id) = resolve(doc, value)?;
            if id.is_some_and(|id| !visited.insert(id)) {
                return Err(malformed("outline name tree cycle or shared child"));
            }
            let node = dict(value)?;
            if node.has(b"Kids") && node.has(b"Names") {
                return Err(malformed("outline name tree mixes branches and leaves"));
            }
            if let Ok(kids) = node.get(b"Kids") {
                let kids = resolve(doc, kids)?
                    .0
                    .as_array()
                    .map_err(|_| malformed("outline name Kids array expected"))?;
                scheduled = scheduled
                    .checked_add(kids.len())
                    .ok_or(PdfMasterError::Budget("outline name tree nodes"))?;
                if scheduled > MAX_ITEMS {
                    return Err(PdfMasterError::Budget("outline name tree nodes"));
                }
                pending.extend(kids.iter().rev().map(|kid| (kid, depth + 1)));
            }
            if let Ok(entries) = node.get(b"Names") {
                let entries = resolve(doc, entries)?
                    .0
                    .as_array()
                    .map_err(|_| malformed("outline Names array expected"))?;
                if entries.len() % 2 != 0 || entries.len() / 2 > MAX_ITEMS {
                    return Err(malformed("invalid outline name pairs"));
                }
                for pair in entries.chunks_exact(2) {
                    let Object::String(key, _) = resolve(doc, &pair[0])?.0 else {
                        return Err(malformed("outline name key must be a byte string"));
                    };
                    names.add(key, &pair[1], true)?;
                }
            }
        }
        Ok(names)
    }
}

fn destination<'a>(
    doc: &'a Document,
    catalog: &'a Dictionary,
    value: &'a Object,
    names: &mut Option<Names<'a>>,
    pages: &HashMap<ObjectId, usize>,
    lookup_bytes: &mut usize,
) -> Result<Option<usize>> {
    let mut value = resolve(doc, value)?.0;
    if matches!(value, Object::Name(key) | Object::String(key, _) if key.len() > MAX_TEXT) {
        return Err(PdfMasterError::Budget("outline destination lookup key"));
    }
    if let Object::Name(key) | Object::String(key, _) = value {
        *lookup_bytes = lookup_bytes
            .checked_add(key.len())
            .ok_or(PdfMasterError::Budget("outline destination lookup bytes"))?;
        if *lookup_bytes > MAX_TOTAL_TEXT {
            return Err(PdfMasterError::Budget("outline destination lookup bytes"));
        }
    }
    if matches!(value, Object::Name(_) | Object::String(_, _)) && names.is_none() {
        *names = Some(Names::load(doc, catalog)?);
    }
    let named = match value {
        Object::Name(key) => Some(
            names
                .as_ref()
                .and_then(|names| names.legacy.get(key.as_slice())),
        ),
        Object::String(key, _) => Some(
            names
                .as_ref()
                .and_then(|names| names.modern.get(key.as_slice())),
        ),
        _ => None,
    };
    if let Some(named) = named {
        let Some(named) = named else {
            return Ok(None);
        };
        value = resolve(doc, named)?.0;
    }
    if let Object::Dictionary(dictionary) = value {
        let Ok(target) = dictionary.get(b"D") else {
            return Ok(None);
        };
        value = resolve(doc, target)?.0;
    }
    let Object::Array(parts) = value else {
        return Ok(None);
    };
    if parts.len() < 2 || parts.len() > 6 {
        return Ok(None);
    }
    let Object::Reference(page) = &parts[0] else {
        return Ok(None);
    };
    let Some(page) = pages.get(page) else {
        return Ok(None);
    };
    let mode = resolve(doc, &parts[1])?.0.as_name().ok();
    let valid_length = match mode {
        Some(b"Fit" | b"FitB") => parts.len() == 2,
        Some(b"FitH" | b"FitV" | b"FitBH" | b"FitBV") => parts.len() == 3,
        Some(b"XYZ") => parts.len() == 5,
        Some(b"FitR") => parts.len() == 6,
        _ => false,
    };
    if !valid_length {
        return Ok(None);
    }
    for parameter in &parts[2..] {
        let parameter = resolve(doc, parameter)?.0;
        if matches!(parameter, Object::Null) && mode != Some(b"FitR") {
            continue;
        }
        if !parameter.as_float().ok().is_some_and(f32::is_finite) {
            return Ok(None);
        }
    }
    // Navigation intentionally projects only the local page, not its view/zoom.
    Ok(Some(*page))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutlineItem {
    pub title: String,
    pub depth: usize,
    pub path: Vec<usize>,
    pub page_index: Option<usize>,
}

impl Session {
    pub fn outline(&self) -> Result<Vec<OutlineItem>> {
        let doc = &self.document;
        let catalog = doc
            .catalog()
            .map_err(|_| malformed("document catalog unavailable"))?;
        let Ok(root) = catalog.get(b"Outlines") else {
            return Ok(Vec::new());
        };
        let (root, root_id) = resolve(doc, root)?;
        let root = dict(root)?;
        let Ok(first) = root.get(b"First") else {
            return Ok(Vec::new());
        };
        let mut names = None;
        let pages = page_object_ids(doc)?
            .into_iter()
            .enumerate()
            .map(|(index, id)| (id, index))
            .collect::<HashMap<_, _>>();
        let mut rows = Vec::new();
        let mut pending = vec![(first, vec![0])];
        let mut visited = root_id.into_iter().collect::<HashSet<_>>();
        let mut total_text: usize = 0;
        let mut lookup_bytes: usize = 0;
        while let Some((value, path)) = pending.pop() {
            if rows.len() >= MAX_ITEMS || path.len() > MAX_DEPTH {
                return Err(PdfMasterError::Budget("outline item count or depth"));
            }
            let (value, id) = resolve(doc, value)?;
            if id.is_some_and(|id| !visited.insert(id)) {
                return Err(malformed("outline cycle or shared item"));
            }
            let node = dict(value)?;
            let title = if let Ok(value) = node.get(b"Title") {
                let value = resolve(doc, value)?.0;
                if let Object::String(bytes, _) = value {
                    if bytes.len() > MAX_TEXT {
                        return Err(PdfMasterError::Budget("outline title bytes"));
                    }
                    decode_text_string(value)
                        .map_err(|_| malformed("invalid outline title string"))?
                } else {
                    "未命名".into()
                }
            } else {
                "未命名".into()
            };
            total_text = total_text
                .checked_add(title.len())
                .ok_or(PdfMasterError::Budget("outline title bytes"))?;
            if title.len() > MAX_TEXT || total_text > MAX_TOTAL_TEXT {
                return Err(PdfMasterError::Budget("outline title bytes"));
            }
            // Match the original reader's display-only whitespace normalization.
            // Charge raw decoded text above so blank input cannot evade budgets.
            let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
            let title = if title.is_empty() {
                "未命名".into()
            } else {
                title
            };
            let target = if node.has(b"Dest") && node.has(b"A") {
                None
            } else if let Ok(value) = node.get(b"Dest") {
                Some(value)
            } else if let Ok(action) = node.get(b"A") {
                let action = dict(resolve(doc, action)?.0)?;
                let kind = action
                    .get(b"S")
                    .ok()
                    .map(|value| resolve(doc, value))
                    .transpose()?;
                if kind.and_then(|(value, _)| value.as_name().ok()) == Some(b"GoTo") {
                    action.get(b"D").ok()
                } else {
                    None
                }
            } else {
                None
            };
            let page_index = target
                .map(|target| {
                    destination(doc, catalog, target, &mut names, &pages, &mut lookup_bytes)
                })
                .transpose()?
                .flatten();
            rows.push(OutlineItem {
                title,
                depth: path.len() - 1,
                path: path.clone(),
                page_index,
            });
            if let Ok(next) = node.get(b"Next") {
                let mut sibling = path.clone();
                *sibling.last_mut().unwrap() += 1;
                pending.push((next, sibling));
            }
            if let Ok(first) = node.get(b"First") {
                let mut child = path;
                child.push(0);
                pending.push((first, child));
            }
        }
        Ok(rows)
    }
}
