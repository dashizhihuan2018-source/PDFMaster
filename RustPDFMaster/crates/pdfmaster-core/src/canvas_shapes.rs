//! Revision-bound source-coordinate shapes for detached canvas editing.
use crate::document::{page_object_ids, validate_page_index};
use crate::notes::{annots_without_annotation, resolve, resolve_with_id, validated_annots_for_add};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    Color, LineEnding, LineGeometry, PdfMasterError, Point, Rect, Result, Session, ShapeAnnotation,
    ShapeKind,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CanvasShapeSelection {
    pub page_index: usize,
    pub page_id: (u32, u16),
    pub object_id: Option<(u32, u16)>,
    pub revision: [u8; 32],
    pub flags: u32,
    #[serde(default = "opaque")]
    pub opacity: f32,
    pub original: ShapeAnnotation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CanvasShapeList {
    pub annotations: Vec<CanvasShapeSelection>,
    pub unsupported_count: usize,
    pub annotation_count: usize,
    pub revision: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CanvasShapeGeometry {
    Rectangle {
        bounds: Rect,
    },
    Ellipse {
        bounds: Rect,
    },
    Line {
        start: Point,
        end: Point,
        endings: [LineEnding; 2],
        dash_pattern: Vec<f32>,
        #[serde(default)]
        bounds: Option<Rect>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CanvasShapeDraft {
    pub geometry: CanvasShapeGeometry,
    pub stroke: Color,
    pub fill: Option<Color>,
    pub line_width: f32,
    #[serde(default = "opaque")]
    pub opacity: f32,
}

fn opaque() -> f32 {
    1.
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanvasShapeCommit {
    Unchanged,
    Updated,
}

impl Session {
    pub fn canvas_shapes(&self, page: usize) -> Result<CanvasShapeList> {
        validate_page_index(page, self.pages.len())?;
        let page_id = page_object_ids(&self.document)?[page];
        let entries = validated_annots_for_add(&self.document, page_id, &self.pages[page])?;
        let revision = Sha256::digest(&self.current_bytes).into();
        // Parse every flag, including unsupported subtypes, before projecting.
        let mut identities = Vec::with_capacity(entries.len());
        for entry in &entries {
            let (value, object_id) = resolve_with_id(&self.document, entry)?;
            let a = value
                .as_dict()
                .map_err(|e| PdfMasterError::Malformed(e.to_string()))?;
            let flags = match a.get(b"F") {
                Ok(f) => u32::try_from(
                    resolve(&self.document, f)?
                        .as_i64()
                        .map_err(|e| PdfMasterError::Malformed(e.to_string()))?,
                )
                .map_err(|_| PdfMasterError::Malformed("annotation flags out of range".into()))?,
                Err(_) => 0,
            };
            identities.push((object_id, flags));
        }
        let projected = crate::shapes::inspect_canvas_shapes(&self.document, &self.pages, page)?;
        let mut list = CanvasShapeList {
            annotations: Vec::new(),
            unsupported_count: projected.unsupported_count,
            annotation_count: entries.len(),
            revision,
        };
        for original in projected.shapes {
            let (object_id, flags) = identities[original.annotation_index];
            if flags & (1 | 2 | 32) != 0 {
                list.unsupported_count += 1;
                continue;
            }
            let a = resolve(&self.document, &entries[original.annotation_index])?
                .as_dict()
                .map_err(|e| PdfMasterError::Malformed(e.to_string()))?;
            let opacity = crate::shape_opacity::validate(&self.document, a, &original)?;
            list.annotations.push(CanvasShapeSelection {
                page_index: page,
                page_id,
                object_id,
                revision,
                flags,
                opacity,
                original,
            });
        }
        Ok(list)
    }

    pub fn validate_canvas_shape(&self, expected: &CanvasShapeSelection) -> Result<()> {
        let revision: [u8; 32] = Sha256::digest(&self.current_bytes).into();
        if expected.revision != revision
            || !self
                .canvas_shapes(expected.page_index)?
                .annotations
                .iter()
                .any(|s| s == expected)
        {
            return Err(PdfMasterError::InvalidOperation(
                "canvas shape selection changed",
            ));
        }
        if expected.flags & (64 | 128 | 512) != 0 {
            return Err(PdfMasterError::InvalidOperation(
                "canvas shape is locked or read-only",
            ));
        }
        // Reuse all relationship admission (tagging, replies and popup ownership).
        annots_without_annotation(
            &self.document,
            &self.pages,
            expected.page_index,
            expected.original.annotation_index,
        )?;
        Ok(())
    }

    pub fn create_canvas_shape(&mut self, page: usize, draft: &CanvasShapeDraft) -> Result<()> {
        let (shape, bounds, line) = self.canvas_geometry(page, draft)?;
        let mut candidate = self.document.clone();
        if let Some(line) = line {
            if matches!(
                draft.geometry,
                CanvasShapeGeometry::Line {
                    bounds: Some(_),
                    ..
                }
            ) {
                crate::lines::add_line_geometry_with_bounds(
                    &mut candidate,
                    &self.pages,
                    page,
                    &line,
                    bounds,
                    draft.stroke,
                    draft.fill,
                    draft.line_width,
                )?;
            } else {
                crate::lines::add_line_geometry(
                    &mut candidate,
                    &self.pages,
                    page,
                    &line,
                    draft.stroke,
                    draft.fill,
                    draft.line_width,
                )?;
            }
        } else {
            crate::shapes::add_shape(
                &mut candidate,
                &self.pages,
                page,
                bounds,
                shape,
                draft.stroke,
                draft.fill,
                draft.line_width,
            )?;
        }
        let page_id = page_object_ids(&candidate)?[page];
        let index = validated_annots_for_add(&candidate, page_id, &self.pages[page])?.len() - 1;
        crate::shape_opacity::apply_to_generated(
            &mut candidate,
            &self.pages,
            page,
            index,
            draft.opacity,
        )?;
        self.commit_document(candidate)
    }

    pub fn update_canvas_shape(
        &mut self,
        expected: &CanvasShapeSelection,
        draft: &CanvasShapeDraft,
    ) -> Result<CanvasShapeCommit> {
        self.validate_canvas_shape(expected)?;
        let (shape, bounds, line) = self.canvas_geometry(expected.page_index, draft)?;
        let old = &expected.original;
        if shape != old.shape {
            return Err(PdfMasterError::InvalidOperation(
                "canvas shape subtype cannot change",
            ));
        }
        let same_geometry = if shape == ShapeKind::Line {
            line == old.line
                && (!matches!(
                    draft.geometry,
                    CanvasShapeGeometry::Line {
                        bounds: Some(_),
                        ..
                    }
                ) || bounds == old.bounds)
        } else {
            bounds == old.bounds
        };
        if same_geometry
            && draft.stroke == old.stroke
            && draft.fill == old.fill
            && draft.line_width == old.line_width
            && draft.opacity == expected.opacity
        {
            return Ok(CanvasShapeCommit::Unchanged);
        }
        let mut candidate = self.document.clone();
        if draft.opacity != 1. {
            let entries = validated_annots_for_add(
                &candidate,
                expected.page_id,
                &self.pages[expected.page_index],
            )?;
            let a = resolve(&candidate, &entries[old.annotation_index])?
                .as_dict()
                .map_err(|e| PdfMasterError::Malformed(e.to_string()))?;
            crate::shape_opacity::require_normal_appearance_if_present(&candidate, a)?;
        }
        crate::shapes::update_canvas_shape_with_line(
            &mut candidate,
            &self.pages,
            expected.page_index,
            old,
            bounds,
            draft.stroke,
            draft.fill,
            draft.line_width,
            line.as_ref(),
            if matches!(
                draft.geometry,
                CanvasShapeGeometry::Line {
                    bounds: Some(_),
                    ..
                }
            ) {
                crate::shapes::LineBoundsPolicy::Exact
            } else {
                crate::shapes::LineBoundsPolicy::Expanded
            },
        )?;
        crate::shape_opacity::apply_to_generated(
            &mut candidate,
            &self.pages,
            expected.page_index,
            old.annotation_index,
            draft.opacity,
        )?;
        self.commit_document(candidate)?;
        Ok(CanvasShapeCommit::Updated)
    }

    pub fn delete_canvas_shape(&mut self, expected: &CanvasShapeSelection) -> Result<()> {
        self.validate_canvas_shape(expected)?;
        let mut candidate = self.document.clone();
        crate::shapes::delete_canvas_shape(
            &mut candidate,
            &self.pages,
            expected.page_index,
            &expected.original,
        )?;
        self.commit_document(candidate)
    }

    fn canvas_geometry(
        &self,
        page: usize,
        draft: &CanvasShapeDraft,
    ) -> Result<(ShapeKind, Rect, Option<LineGeometry>)> {
        validate_page_index(page, self.pages.len())?;
        if !draft.opacity.is_finite() || !(0. ..=1.).contains(&draft.opacity) {
            return Err(PdfMasterError::InvalidGeometry(
                "shape opacity must be finite and between zero and one",
            ));
        }
        crate::edit::validate_color(draft.stroke)?;
        if let Some(c) = draft.fill {
            crate::edit::validate_color(c)?;
        }
        let (shape, bounds, line) = match &draft.geometry {
            CanvasShapeGeometry::Rectangle { bounds } => (ShapeKind::Rectangle, *bounds, None),
            CanvasShapeGeometry::Ellipse { bounds } => (ShapeKind::Ellipse, *bounds, None),
            CanvasShapeGeometry::Line {
                start,
                end,
                endings,
                dash_pattern,
                bounds,
            } => {
                let line = LineGeometry {
                    start: *start,
                    end: *end,
                    endings: *endings,
                    dash_pattern: dash_pattern.clone(),
                };
                crate::lines::validate_width(draft.line_width)?;
                crate::lines::validate_geometry(
                    &line,
                    Rect {
                        x: 0.,
                        y: 0.,
                        width: self.pages[page].width,
                        height: self.pages[page].height,
                    },
                )?;
                let bounds = bounds.unwrap_or_else(|| {
                    crate::lines::expanded_bounds(&line, &self.pages[page], draft.line_width, None)
                });
                crate::edit::validate_rect(bounds, &self.pages[page])?;
                crate::lines::validate_geometry(&line, bounds)?;
                (ShapeKind::Line, bounds, Some(line))
            }
        };
        crate::edit::validate_rect(bounds, &self.pages[page])?;
        if shape != ShapeKind::Line
            && (!draft.line_width.is_finite()
                || draft.line_width <= 0.
                || draft.line_width >= bounds.width.min(bounds.height))
        {
            return Err(PdfMasterError::InvalidGeometry(
                "shape line width is unsupported",
            ));
        }
        Ok((shape, bounds, line))
    }

    pub fn render_canvas_shape_background_png(
        &self,
        expected: &CanvasShapeSelection,
        scale: f32,
    ) -> Result<Vec<u8>> {
        self.validate_canvas_shape(expected)?;
        self.preflight_render_page(expected.page_index)?;
        drop(self.render_png(expected.page_index, scale)?);
        let mut candidate = self.document.clone();
        let mut annots = validated_annots_for_add(
            &candidate,
            expected.page_id,
            &self.pages[expected.page_index],
        )?;
        annots.remove(expected.original.annotation_index);
        let array = candidate.add_object(annots);
        candidate
            .get_object_mut(expected.page_id)
            .and_then(lopdf::Object::as_dict_mut)
            .map_err(|e| PdfMasterError::Malformed(e.to_string()))?
            .set("Annots", array);
        let bytes = crate::document::serialize(&candidate)?;
        Session::open(&bytes)?.render_png(expected.page_index, scale)
    }
}
