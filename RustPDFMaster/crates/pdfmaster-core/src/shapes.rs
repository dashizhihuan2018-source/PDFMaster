use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use serde::{Deserialize, Serialize};

use crate::document::{
    Color, PageInfo, PdfMasterError, Rect, Result, Session, ShapeKind, page_object_ids,
    validate_page_index,
};
use crate::edit::{validate_color, validate_rect};
use crate::notes::{
    annots_without_annotation, parse_color, parse_rect, resolve, resolve_with_id,
    validate_notes_page, validated_annots_for_add,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShapeAnnotation {
    pub annotation_index: usize,
    pub shape: ShapeKind,
    pub bounds: Rect,
    pub stroke: Color,
    pub fill: Option<Color>,
    pub line_width: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<crate::lines::LineGeometry>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ShapeList {
    pub shapes: Vec<ShapeAnnotation>,
    pub unsupported_count: usize,
}

impl Session {
    pub fn shapes(&self, page: usize) -> Result<ShapeList> {
        inspect_shapes(&self.document, &self.pages, page)
    }
}

fn inspect_shapes(document: &Document, pages: &[PageInfo], page: usize) -> Result<ShapeList> {
    validate_page_index(page, pages.len())?;
    let page_id = page_object_ids(document)?[page];
    let annots = validated_annots_for_add(document, page_id, &pages[page])?;
    let mut list = ShapeList::default();
    for (annotation_index, entry) in annots.iter().enumerate() {
        let a = resolve(document, entry)?
            .as_dict()
            .map_err(|e| PdfMasterError::Malformed(e.to_string()))?;
        let shape = match resolve(
            document,
            a.get(b"Subtype")
                .map_err(|e| PdfMasterError::Malformed(e.to_string()))?,
        )?
        .as_name()
        .map_err(|e| PdfMasterError::Malformed(e.to_string()))?
        {
            b"Square" => ShapeKind::Rectangle,
            b"Circle" => ShapeKind::Ellipse,
            b"Line" => ShapeKind::Line,
            _ => {
                list.unsupported_count += 1;
                continue;
            }
        };
        let bounds = parse_rect(document, a, &pages[page])?;
        // Non-solid/alpha/special-effect imported styles are not silently
        // normalized into a different shape when applying the numeric editor.
        let style = (|| -> Result<(Color, Option<Color>, f32)> {
            if a.has(b"BE") || a.has(b"RD") || a.has(b"CA") || a.has(b"Rotate") {
                return Err(PdfMasterError::InvalidOperation(
                    "extended shape style is not yet editable",
                ));
            }
            let stroke = parse_color(document, a)?.unwrap_or(Color {
                r: 0.,
                g: 0.,
                b: 0.,
            });
            let fill = match a.get(b"IC") {
                Ok(c) => parse_color(document, &dictionary! {"C"=>c.clone()})?,
                Err(_) => None,
            };
            let line = if let Ok(bs) = a.get(b"BS") {
                let bs = resolve(document, bs)?
                    .as_dict()
                    .map_err(|e| PdfMasterError::Malformed(e.to_string()))?;
                if shape != ShapeKind::Line
                    && (bs.has(b"D")
                        || bs.get(b"S").ok().is_some_and(|s| {
                            resolve(document, s).ok().and_then(|s| s.as_name().ok()) != Some(b"S")
                        }))
                {
                    return Err(PdfMasterError::InvalidOperation(
                        "non-solid shape border is not yet editable",
                    ));
                }
                match bs.get(b"W") {
                    Ok(w) => resolve(document, w)?
                        .as_float()
                        .map_err(|e| PdfMasterError::Malformed(e.to_string()))?,
                    Err(_) => 1.,
                }
            } else if let Ok(border) = a.get(b"Border") {
                let border = resolve(document, border)?
                    .as_array()
                    .map_err(|e| PdfMasterError::Malformed(e.to_string()))?;
                if !(border.len() == 3 || (shape == ShapeKind::Line && border.len() == 4))
                    || resolve(document, &border[0])?.as_float().ok() != Some(0.)
                    || resolve(document, &border[1])?.as_float().ok() != Some(0.)
                {
                    return Err(PdfMasterError::InvalidOperation(
                        "extended shape border is not yet editable",
                    ));
                }
                resolve(document, &border[2])?
                    .as_float()
                    .map_err(|e| PdfMasterError::Malformed(e.to_string()))?
            } else {
                1.
            };
            if shape == ShapeKind::Line {
                crate::lines::validate_width(line)?;
            } else if !line.is_finite() || line <= 0. || line >= bounds.width.min(bounds.height) {
                return Err(PdfMasterError::InvalidGeometry(
                    "shape line width is unsupported",
                ));
            }
            Ok((stroke, fill, line))
        })();
        match style {
            Ok((stroke, fill, line_width)) => {
                let line = if shape == ShapeKind::Line {
                    match crate::lines::parse(document, a, bounds) {
                        Ok(line) => Some(line),
                        Err(_) => {
                            list.unsupported_count += 1;
                            continue;
                        }
                    }
                } else {
                    None
                };
                list.shapes.push(ShapeAnnotation {
                    annotation_index,
                    shape,
                    bounds,
                    stroke,
                    fill,
                    line_width,
                    line,
                });
            }
            Err(_) => list.unsupported_count += 1,
        }
    }
    Ok(list)
}

fn verify_selected(
    document: &Document,
    pages: &[PageInfo],
    page: usize,
    expected: &ShapeAnnotation,
) -> Result<()> {
    let current = inspect_shapes(document, pages, page)?
        .shapes
        .into_iter()
        .find(|s| s.annotation_index == expected.annotation_index)
        .ok_or(PdfMasterError::InvalidOperation(
            "editable shape is unavailable",
        ))?;
    if &current != expected {
        return Err(PdfMasterError::InvalidOperation(
            "shape changed since it was selected",
        ));
    }
    Ok(())
}

pub(crate) fn delete_shape(
    document: &mut Document,
    pages: &[PageInfo],
    page: usize,
    expected: &ShapeAnnotation,
) -> Result<()> {
    verify_selected(document, pages, page, expected)?;
    let remaining = annots_without_annotation(document, pages, page, expected.annotation_index)?;
    let page_id = page_object_ids(document)?[page];
    document
        .get_object_mut(page_id)
        .and_then(Object::as_dict_mut)
        .map_err(|e| PdfMasterError::Malformed(e.to_string()))?
        .set("Annots", remaining);
    validate_notes_page(document, page_id, &pages[page])
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_shape(
    document: &mut Document,
    pages: &[PageInfo],
    page: usize,
    expected: &ShapeAnnotation,
    bounds: Rect,
    stroke: Color,
    fill: Option<Color>,
    line_width: f32,
) -> Result<()> {
    verify_selected(document, pages, page, expected)?;
    validate_rect(bounds, &pages[page])?;
    validate_color(stroke)?;
    if let Some(c) = fill {
        validate_color(c)?;
    }
    if expected.shape == ShapeKind::Line {
        crate::lines::validate_width(line_width)?;
    } else if !line_width.is_finite()
        || line_width <= 0.
        || line_width >= bounds.width.min(bounds.height)
    {
        return Err(PdfMasterError::InvalidGeometry(
            "shape line width is unsupported",
        ));
    }
    annots_without_annotation(document, pages, page, expected.annotation_index)?;
    let page_id = page_object_ids(document)?[page];
    let mut annots = validated_annots_for_add(document, page_id, &pages[page])?;
    let (old, old_id) = resolve_with_id(document, &annots[expected.annotation_index])?;
    let mut annotation = old
        .as_dict()
        .map_err(|e| PdfMasterError::Malformed(e.to_string()))?
        .clone();
    let resized_line = expected
        .line
        .as_ref()
        .map(|line| crate::lines::resized(line, expected.bounds, bounds))
        .transpose()?;
    let bounds = if let Some(line) = &resized_line {
        crate::lines::expanded_bounds(line, &pages[page], line_width, Some(bounds))
    } else {
        bounds
    };
    let content = if let Some(line) = &resized_line {
        crate::lines::appearance(line, bounds, stroke, fill, line_width)?
    } else {
        appearance(
            bounds.width,
            bounds.height,
            expected.shape,
            stroke,
            fill,
            line_width,
        )?
    };
    let ap=document.add_object(Stream::new(dictionary!{"Type"=>"XObject","Subtype"=>"Form","FormType"=>1_i64,"BBox"=>vec![0.into(),0.into(),bounds.width.into(),bounds.height.into()],"Resources"=>Dictionary::new()},content));
    annotation.set(
        "Rect",
        vec![
            bounds.x.into(),
            bounds.y.into(),
            (bounds.x + bounds.width).into(),
            (bounds.y + bounds.height).into(),
        ],
    );
    annotation.set("C", vec![stroke.r.into(), stroke.g.into(), stroke.b.into()]);
    annotation.set("M", crate::annotations::modified_now()?);
    annotation.set(
        "BS",
        dictionary! {"Type"=>"Border","W"=>line_width,"S"=>"S"},
    );
    annotation.set("Border", vec![0.into(), 0.into(), line_width.into()]);
    if let Some(line) = &resized_line {
        crate::lines::write_geometry(&mut annotation, line, line_width);
    }
    annotation.set("AP", dictionary! {"N"=>ap});
    if let Some(c) = fill {
        annotation.set("IC", vec![c.r.into(), c.g.into(), c.b.into()]);
    } else {
        annotation.remove(b"IC");
    }
    annotation.set("P", page_id);
    let root = document.new_object_id();
    let mut popups = std::collections::HashSet::new();
    if let Ok(p) = annotation.get(b"Popup") {
        popups.insert(
            resolve_with_id(document, p)?
                .1
                .ok_or(PdfMasterError::InvalidOperation(
                    "direct popup is unsupported",
                ))?,
        );
    }
    for entry in &annots {
        let (value, id) = resolve_with_id(document, entry)?;
        let a = value
            .as_dict()
            .map_err(|e| PdfMasterError::Malformed(e.to_string()))?;
        if resolve(
            document,
            a.get(b"Subtype")
                .map_err(|e| PdfMasterError::Malformed(e.to_string()))?,
        )?
        .as_name()
        .ok()
            == Some(b"Popup")
            && let Ok(parent) = a.get(b"Parent")
            && old_id.is_some()
            && resolve_with_id(document, parent)?.1 == old_id
        {
            popups.insert(id.ok_or(PdfMasterError::InvalidOperation(
                "direct popup is unsupported",
            ))?);
        }
    }
    if popups.len() > 1 {
        return Err(PdfMasterError::Malformed(
            "conflicting popup ownership".into(),
        ));
    }
    if let Some(old_popup) = popups.into_iter().next() {
        let mut popup = document
            .get_object(old_popup)
            .and_then(Object::as_dict)
            .map_err(|e| PdfMasterError::Malformed(e.to_string()))?
            .clone();
        popup.set("Parent", root);
        if popup.has(b"P") {
            popup.set("P", page_id);
        }
        let new_popup = document.add_object(popup);
        annotation.set("Popup", new_popup);
        for entry in &mut annots {
            if resolve_with_id(document, entry)?.1 == Some(old_popup) {
                *entry = Object::Reference(new_popup);
            }
        }
    }
    document.objects.insert(root, annotation.into());
    annots[expected.annotation_index] = root.into();
    document
        .get_object_mut(page_id)
        .and_then(Object::as_dict_mut)
        .map_err(|e| PdfMasterError::Malformed(e.to_string()))?
        .set("Annots", annots);
    validate_notes_page(document, page_id, &pages[page])
}

/// Add an independent PDF annotation, not paint flattened into page Contents.
/// Bounds are unrotated source-page coordinates; /Rotate remains untouched.
#[allow(clippy::too_many_arguments)]
pub(crate) fn add_shape(
    document: &mut Document,
    pages: &[PageInfo],
    page: usize,
    bounds: Rect,
    shape: ShapeKind,
    stroke: Color,
    fill: Option<Color>,
    line_width: f32,
) -> Result<()> {
    if shape == ShapeKind::Line {
        return Err(PdfMasterError::InvalidOperation(
            "use add_line with explicit endpoints",
        ));
    }
    validate_page_index(page, pages.len())?;
    validate_rect(bounds, &pages[page])?;
    validate_color(stroke)?;
    if let Some(fill) = fill {
        validate_color(fill)?;
    }
    if !line_width.is_finite() || line_width <= 0. || line_width >= bounds.width.min(bounds.height)
    {
        return Err(PdfMasterError::InvalidGeometry(
            "shape line width must be finite, positive and smaller than its bounds",
        ));
    }
    let page_id = page_object_ids(document)?[page];
    let mut annots = validated_annots_for_add(document, page_id, &pages[page])?;
    let content = appearance(bounds.width, bounds.height, shape, stroke, fill, line_width)?;
    let appearance = document.add_object(Stream::new(
        dictionary! {
            "Type"=>"XObject", "Subtype"=>"Form", "FormType"=>1_i64,
            "BBox"=>vec![0.into(),0.into(),bounds.width.into(),bounds.height.into()],
            "Resources"=>Dictionary::new(),
        },
        content,
    ));
    let mut annotation = dictionary! {
        "Type"=>"Annot", "Subtype"=>match shape {ShapeKind::Rectangle=>"Square", ShapeKind::Ellipse=>"Circle", ShapeKind::Line=>return Err(PdfMasterError::InvalidOperation("use add_line with explicit endpoints"))},
        "Rect"=>vec![bounds.x.into(),bounds.y.into(),(bounds.x+bounds.width).into(),(bounds.y+bounds.height).into()],
        "P"=>page_id, "F"=>4_i64,
        "M"=>crate::annotations::modified_now()?,
        "C"=>vec![stroke.r.into(),stroke.g.into(),stroke.b.into()],
        "BS"=>dictionary! {"Type"=>"Border", "W"=>line_width, "S"=>"S"},
        "Border"=>vec![0.into(),0.into(),line_width.into()],
        "AP"=>dictionary! {"N"=>appearance},
    };
    if let Some(fill) = fill {
        annotation.set("IC", vec![fill.r.into(), fill.g.into(), fill.b.into()]);
    }
    let id = document.add_object(annotation);
    annots.push(id.into());
    document
        .get_object_mut(page_id)
        .and_then(lopdf::Object::as_dict_mut)
        .map_err(|error| PdfMasterError::Malformed(error.to_string()))?
        .set("Annots", annots);
    // Includes the shared per-page count budget after appending. All changes
    // still belong to Session's disposable candidate until commit/reparse.
    validate_notes_page(document, page_id, &pages[page])
}

fn appearance(
    width: f32,
    height: f32,
    shape: ShapeKind,
    stroke: Color,
    fill: Option<Color>,
    line: f32,
) -> Result<Vec<u8>> {
    let mut operations = vec![
        Operation::new("q", vec![]),
        Operation::new(
            "RG",
            vec![stroke.r.into(), stroke.g.into(), stroke.b.into()],
        ),
        Operation::new("w", vec![line.into()]),
    ];
    if let Some(fill) = fill {
        operations.push(Operation::new(
            "rg",
            vec![fill.r.into(), fill.g.into(), fill.b.into()],
        ));
    }
    // Inset the centerline so every stroke pixel remains inside /Rect and BBox.
    let inset = line / 2.;
    let w = width - line;
    let h = height - line;
    match shape {
        ShapeKind::Line => {
            return Err(PdfMasterError::InvalidOperation(
                "line appearance requires endpoints",
            ));
        }
        ShapeKind::Rectangle => operations.push(Operation::new(
            "re",
            vec![inset.into(), inset.into(), w.into(), h.into()],
        )),
        ShapeKind::Ellipse => {
            let cx = width / 2.;
            let cy = height / 2.;
            let rx = w / 2.;
            let ry = h / 2.;
            let k = 0.552_284_8;
            let dx = rx * k;
            let dy = ry * k;
            operations.push(Operation::new("m", vec![(cx + rx).into(), cy.into()]));
            for values in [
                [cx + rx, cy + dy, cx + dx, cy + ry, cx, cy + ry],
                [cx - dx, cy + ry, cx - rx, cy + dy, cx - rx, cy],
                [cx - rx, cy - dy, cx - dx, cy - ry, cx, cy - ry],
                [cx + dx, cy - ry, cx + rx, cy - dy, cx + rx, cy],
            ] {
                operations.push(Operation::new(
                    "c",
                    values.into_iter().map(Into::into).collect(),
                ));
            }
            operations.push(Operation::new("h", vec![]));
        }
    }
    operations.push(Operation::new(
        if fill.is_some() { "B" } else { "S" },
        vec![],
    ));
    operations.push(Operation::new("Q", vec![]));
    Content { operations }
        .encode()
        .map_err(|error| PdfMasterError::Serialization(error.to_string()))
}
