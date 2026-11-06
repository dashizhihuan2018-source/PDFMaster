//! Atomic embedded-font FreeText creation and typed update transactions.
use crate::{Color, FreeTextAnnotation, PdfMasterError, Rect, Result, Session, TextAlignment};
use lopdf::dictionary;

fn allocation_room(document: &lopdf::Document) -> Result<()> {
    document
        .max_id
        .checked_add(4)
        .ok_or(PdfMasterError::Budget("FreeText object IDs"))?;
    Ok(())
}

fn validate_input(contents: &str, bounds: Rect, page: &crate::PageInfo) -> Result<()> {
    if contents.len() > 65_536 {
        return Err(PdfMasterError::Budget("FreeText contents"));
    }
    crate::edit::validate_rect(bounds, page)?;
    if bounds.width > 14_400. || bounds.height > 14_400. {
        return Err(PdfMasterError::InvalidGeometry("appearance dimensions"));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub struct FreeTextAppearanceStyle {
    pub font: crate::text_appearance::BundledFont,
    pub font_size: f32,
    pub color: Color,
    pub alignment: TextAlignment,
    pub line_height: f32,
}

#[derive(Clone, Debug)]
pub struct FreeTextDraft<'a> {
    pub contents: &'a str,
    pub bounds: Rect,
    pub style: FreeTextAppearanceStyle,
}
#[derive(Clone, Debug)]
pub struct FreeTextUpdate<'a> {
    pub contents: &'a str,
    pub bounds: Rect,
    pub style: Option<FreeTextAppearanceStyle>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreeTextCommit {
    Unchanged,
    Created,
    Updated,
    Deleted,
}

impl Session {
    pub fn create_free_text(
        &mut self,
        page: usize,
        draft: &FreeTextDraft<'_>,
    ) -> Result<FreeTextCommit> {
        crate::document::validate_page_index(page, self.pages.len())?;
        validate_input(draft.contents, draft.bounds, &self.pages[page])?;
        crate::free_text_edit_style::validate(&draft.style)?;
        self.free_texts(page)?;
        let page_id = crate::document::page_object_ids(&self.document)?[page];
        let mut annots =
            crate::notes::validated_annots_for_add(&self.document, page_id, &self.pages[page])?;
        if draft.contents.trim().is_empty() {
            return Ok(FreeTextCommit::Unchanged);
        }
        if annots.len() >= 1024 {
            return Err(PdfMasterError::Budget("annotation count"));
        }
        let total = page_body_bytes(&self.document, &annots)?;
        if total
            .checked_add(draft.contents.len())
            .is_none_or(|n| n > 1024 * 1024)
        {
            return Err(PdfMasterError::Budget("FreeText contents"));
        }
        allocation_room(&self.document)?;
        let prepared = prepare(draft.contents, draft.bounds, &draft.style)?;
        let mut candidate = self.document.clone();
        let (form, da) = crate::free_text_appearance::import(
            &mut candidate,
            &prepared,
            draft.bounds.width,
            draft.bounds.height,
            &draft.style,
        )?;
        let annotation = lopdf::dictionary! {
            "Type"=>"Annot", "Subtype"=>"FreeText", "P"=>page_id, "F"=>4,
            "Rect"=>rectangle(draft.bounds), "Contents"=>lopdf::text_string(draft.contents),
            "DA"=>da, "Q"=>alignment(draft.style.alignment), "Border"=>vec![0.into(),0.into(),0.into()],
            "AP"=>lopdf::dictionary!{"N"=>form}, "M"=>crate::annotations::modified_now()?,
            "PDFMasterAppearance"=>crate::free_text_edit_style::metadata(&draft.style),
        };
        let annotation_id = candidate
            .try_add_object(annotation)
            .map_err(|_| PdfMasterError::Budget("FreeText IDs"))?;
        annots.push(lopdf::Object::Reference(annotation_id));
        let array = candidate
            .try_add_object(annots)
            .map_err(|_| PdfMasterError::Budget("FreeText IDs"))?;
        candidate
            .get_object_mut(page_id)
            .and_then(lopdf::Object::as_dict_mut)
            .map_err(|e| PdfMasterError::Malformed(e.to_string()))?
            .set("Annots", array);
        self.commit_document(candidate)?;
        Ok(FreeTextCommit::Created)
    }
    pub fn update_free_text(
        &mut self,
        expected: &FreeTextAnnotation,
        update: &FreeTextUpdate<'_>,
    ) -> Result<FreeTextCommit> {
        let mut annotation = self.selected_free_text(expected)?;
        validate_input(
            update.contents,
            update.bounds,
            &self.pages[expected.page_index],
        )?;
        if let Some(style) = &update.style {
            crate::free_text_edit_style::validate(style)?;
        }
        let body_changed = update.contents != expected.contents;
        let geometry_changed = update.bounds != expected.bounds;
        let style_changed = match &update.style {
            Some(style) => {
                crate::free_text_edit_style::read(&self.document, &annotation, expected)?.as_ref()
                    != Some(style)
            }
            None => false,
        };
        if !body_changed && !geometry_changed && !style_changed {
            return Ok(FreeTextCommit::Unchanged);
        }
        allocation_room(&self.document)?;
        if update.contents.trim().is_empty() {
            self.delete_free_text(expected)?;
            return Ok(FreeTextCommit::Deleted);
        }
        if expected.flags & 64 != 0
            || (body_changed && expected.flags & 512 != 0)
            || ((geometry_changed || style_changed) && expected.flags & 128 != 0)
        {
            return Err(PdfMasterError::InvalidOperation(
                "FreeText action is locked",
            ));
        }
        let members = crate::notes::validated_annots_for_add(
            &self.document,
            expected.page_id,
            &self.pages[expected.page_index],
        )?;
        let unchanged_body_bytes =
            page_body_bytes(&self.document, &members)? - expected.contents.len();
        if update.contents.len() > (1024 * 1024) - unchanged_body_bytes {
            return Err(PdfMasterError::Budget("FreeText contents"));
        }
        // Reuse the audited relationship validator before producer work, as well as
        // the same COW replacement helper at the final candidate write.
        crate::notes::annots_without_annotation(
            &self.document,
            &self.pages,
            expected.page_index,
            expected.annotation_index,
        )?;
        let resize = update.bounds.width != expected.bounds.width
            || update.bounds.height != expected.bounds.height;
        let mut candidate = self.document.clone();
        if body_changed || resize || style_changed {
            crate::free_text_edit_style::validate_simple_profile(&self.document, &annotation)?;
            let style = match &update.style {
                Some(style) => style.clone(),
                None => {
                    match crate::free_text_edit_style::read(&self.document, &annotation, expected)?
                    {
                        Some(style) => style,
                        None => {
                            return Err(PdfMasterError::InvalidOperation(
                                "FreeText reflow requires explicit font selection",
                            ));
                        }
                    }
                }
            };
            let prepared = prepare(update.contents, update.bounds, &style)?;
            let (form, da) = crate::free_text_appearance::import(
                &mut candidate,
                &prepared,
                update.bounds.width,
                update.bounds.height,
                &style,
            )?;
            let mut ap = match annotation.get(b"AP") {
                Ok(value) => crate::notes::resolve(&self.document, value)?
                    .as_dict()
                    .map_err(|_| {
                        PdfMasterError::Malformed("FreeText AP must be a dictionary".into())
                    })?
                    .clone(),
                Err(_) => lopdf::Dictionary::new(),
            };
            ap.set("N", form);
            annotation.set("AP", ap);
            annotation.set("DA", da);
            annotation.set("Q", alignment(style.alignment));
            annotation.set(
                "PDFMasterAppearance",
                crate::free_text_edit_style::metadata(&style),
            );
        }
        annotation.set("Contents", lopdf::text_string(update.contents));
        annotation.set("Rect", rectangle(update.bounds));
        annotation.set("M", crate::annotations::modified_now()?);
        annotation.set("P", expected.page_id);
        crate::notes::replace_annotation(
            &mut candidate,
            &self.pages,
            expected.page_index,
            expected.annotation_index,
            annotation,
        )?;
        self.commit_document(candidate)?;
        Ok(FreeTextCommit::Updated)
    }
    pub fn free_text_appearance_style(
        &self,
        expected: &FreeTextAnnotation,
    ) -> Result<Option<FreeTextAppearanceStyle>> {
        let annotation = self.selected_free_text(expected)?;
        crate::free_text_edit_style::read(&self.document, &annotation, expected)
    }
}

fn rectangle(bounds: Rect) -> Vec<lopdf::Object> {
    vec![
        bounds.x.into(),
        bounds.y.into(),
        (bounds.x + bounds.width).into(),
        (bounds.y + bounds.height).into(),
    ]
}
fn page_body_bytes(document: &lopdf::Document, annots: &[lopdf::Object]) -> Result<usize> {
    let mut total = 0usize;
    for value in annots {
        let dictionary = crate::notes::resolve(document, value)?
            .as_dict()
            .map_err(|e| PdfMasterError::Malformed(e.to_string()))?;
        if crate::notes::resolve(
            document,
            dictionary
                .get(b"Subtype")
                .map_err(|e| PdfMasterError::Malformed(e.to_string()))?,
        )?
        .as_name()
        .ok()
            == Some(b"FreeText")
            && let Ok(value) = dictionary.get(b"Contents")
        {
            total = total
                .checked_add(crate::notes::decode_contents(document, value)?.len())
                .ok_or(PdfMasterError::Budget("FreeText contents"))?;
        }
    }
    Ok(total)
}
fn alignment(value: TextAlignment) -> i64 {
    match value {
        TextAlignment::Left => 0,
        TextAlignment::Center => 1,
        TextAlignment::Right => 2,
    }
}
fn prepare(
    contents: &str,
    bounds: Rect,
    style: &FreeTextAppearanceStyle,
) -> Result<crate::text_appearance::PreparedTextAppearance> {
    crate::text_appearance::prepare_text_appearance(
        &crate::text_appearance::TextAppearanceRequest {
            contents,
            width: bounds.width,
            height: bounds.height,
            font: style.font,
            font_size: style.font_size,
            color: style.color,
            alignment: style.alignment,
            line_height: style.line_height,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn free_text_allocation_preflight_rejects_exhausted_ids() {
        let mut doc = lopdf::Document::with_version("1.7");
        doc.max_id = u32::MAX;
        assert!(allocation_room(&doc).is_err());
    }
    #[test]
    fn free_text_exhausted_ids_create_move_resize_body_delete_are_errors_not_panics() {
        let bounds = Rect {
            x: 10.,
            y: 10.,
            width: 200.,
            height: 100.,
        };
        let style = FreeTextAppearanceStyle {
            font: crate::text_appearance::BundledFont::Sans,
            font_size: 18.,
            color: Color {
                r: 0.,
                g: 0.,
                b: 0.,
            },
            alignment: TextAlignment::Left,
            line_height: 1.3,
        };
        let mut rejected = Vec::new();
        for action in ["create", "move", "resize", "body", "delete"] {
            let mut session = Session::blank(595., 842., 1).unwrap();
            session
                .create_free_text(
                    0,
                    &FreeTextDraft {
                        contents: "original",
                        bounds,
                        style: style.clone(),
                    },
                )
                .unwrap();
            session
                .apply(crate::Edit::Rotate {
                    page: 0,
                    degrees: 90,
                })
                .unwrap();
            session.undo().unwrap();
            let selected = session.free_texts(0).unwrap().annotations.remove(0);
            let before = session.bytes().unwrap();
            let info = session.info();
            session.document.max_id = u32::MAX;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if action == "create" {
                    return session.create_free_text(
                        0,
                        &FreeTextDraft {
                            contents: "new",
                            bounds,
                            style: style.clone(),
                        },
                    );
                }
                let mut update = FreeTextUpdate {
                    contents: &selected.contents,
                    bounds,
                    style: None,
                };
                match action {
                    "move" => update.bounds.x += 1.,
                    "resize" => update.bounds.width -= 1.,
                    "body" => update.contents = "changed",
                    _ => update.contents = " ",
                }
                session.update_free_text(&selected, &update)
            }));
            rejected.push(result.is_ok_and(|r| r.is_err()));
            assert_eq!(session.bytes().unwrap(), before);
            assert_eq!(session.info(), info);
            session.redo().unwrap();
            session.undo().unwrap();
            assert_eq!(session.bytes().unwrap(), before);
            session.undo().unwrap();
            session.redo().unwrap();
            assert_eq!(session.bytes().unwrap(), before);
        }
        assert_eq!(rejected, vec![true; 5]);
    }
    #[test]
    fn free_text_input_validates_source_bytes_and_geometry_even_when_blank() {
        let page = crate::PageInfo {
            width: 20_000.,
            height: 20_000.,
            rotation: 0,
        };
        let bounds = Rect {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 100.,
        };
        assert!(validate_input(&"a".repeat(65_536), bounds, &page).is_ok());
        assert!(validate_input("", bounds, &page).is_ok());
        let mut rejected = vec![validate_input(&" ".repeat(65_537), bounds, &page).is_err()];
        for coordinate in 0..4 {
            for value in [f32::NAN, f32::INFINITY, -1.] {
                let mut invalid = bounds;
                match coordinate {
                    0 => invalid.x = value,
                    1 => invalid.y = value,
                    2 => invalid.width = value,
                    _ => invalid.height = value,
                };
                rejected.push(validate_input("", invalid, &page).is_err());
            }
        }
        for width in [0., 14_400.1, 20_001.] {
            let mut invalid = bounds;
            invalid.width = width;
            rejected.push(validate_input("", invalid, &page).is_err());
        }
        for height in [0., 14_400.1, 20_001.] {
            let mut invalid = bounds;
            invalid.height = height;
            rejected.push(validate_input("", invalid, &page).is_err());
        }
        assert_eq!(rejected, vec![true; 19]);
    }
}
