use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Stream, dictionary};

use crate::document::{
    Color, PageInfo, PdfMasterError, Rect, Result, ShapeKind, page_object_ids, validate_page_index,
};
use crate::edit::{validate_color, validate_rect};
use crate::notes::{validate_notes_page, validated_annots_for_add};

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
        "Type"=>"Annot", "Subtype"=>match shape {ShapeKind::Rectangle=>"Square", ShapeKind::Ellipse=>"Circle"},
        "Rect"=>vec![bounds.x.into(),bounds.y.into(),(bounds.x+bounds.width).into(),(bounds.y+bounds.height).into()],
        "P"=>page_id, "F"=>4_i64,
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
