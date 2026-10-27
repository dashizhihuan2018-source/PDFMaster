use crate::Point;
use crate::document::{
    Color, PageInfo, PdfMasterError, Rect, Result, page_object_ids, validate_page_index,
};
use crate::edit::{validate_color, validate_rect};
use crate::notes::{validate_notes_page, validated_annots_for_add};
use lopdf::{
    Dictionary, Document, Object, Stream,
    content::{Content, Operation},
    dictionary,
};

pub(crate) fn add_ink(
    document: &mut Document,
    pages: &[PageInfo],
    page: usize,
    strokes: Vec<Vec<Point>>,
    color: Color,
    width: f32,
    smoothed: bool,
) -> Result<()> {
    validate_page_index(page, pages.len())?;
    validate_color(color)?;
    crate::lines::validate_width(width)?;
    if strokes.is_empty()
        || strokes.len() > 64
        || strokes
            .iter()
            .map(Vec::len)
            .try_fold(0usize, usize::checked_add)
            .is_none_or(|n| n > 8192)
    {
        return Err(PdfMasterError::Budget("ink stroke/input count"));
    }
    let p = &pages[page];
    let mut total = 0usize;
    let mut paths = Vec::new();
    for points in strokes {
        if !(2..=pdfmaster_drawing::MAX_INPUT_POINTS).contains(&points.len())
            || points.iter().any(|v| {
                !v.x.is_finite()
                    || !v.y.is_finite()
                    || v.x < 0.
                    || v.y < 0.
                    || v.x > p.width
                    || v.y > p.height
            })
        {
            return Err(PdfMasterError::InvalidGeometry(
                "ink points must be finite and inside page",
            ));
        }
        let points = if smoothed {
            pdfmaster_drawing::smooth(&points).map_err(PdfMasterError::InvalidGeometry)?
        } else {
            points
        };
        total += points.len();
        if total > pdfmaster_drawing::MAX_PATH_POINTS {
            return Err(PdfMasterError::Budget("flattened ink points"));
        }
        paths.push(points);
    }
    let page_id = page_object_ids(document)?[page];
    let mut annots = validated_annots_for_add(document, page_id, p)?;
    for points in paths {
        let x = points.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
        let y = points.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
        let right = points.iter().map(|p| p.x).fold(0., f32::max);
        let top = points.iter().map(|p| p.y).fold(0., f32::max);
        let bounds = Rect {
            x: (x - width).max(0.),
            y: (y - width).max(0.),
            width: (right + width).min(p.width) - (x - width).max(0.),
            height: (top + width).min(p.height) - (y - width).max(0.),
        };
        validate_rect(bounds, p)?;
        let mut operations = vec![
            Operation::new("q", vec![]),
            Operation::new("RG", vec![color.r.into(), color.g.into(), color.b.into()]),
            Operation::new("w", vec![width.into()]),
            Operation::new("J", vec![1.into()]),
            Operation::new("j", vec![1.into()]),
        ];
        for (i, point) in points.iter().enumerate() {
            operations.push(Operation::new(
                if i == 0 { "m" } else { "l" },
                vec![(point.x - bounds.x).into(), (point.y - bounds.y).into()],
            ));
        }
        operations.push(Operation::new("S", vec![]));
        operations.push(Operation::new("Q", vec![]));
        let content = Content { operations }
            .encode()
            .map_err(|e| PdfMasterError::Serialization(e.to_string()))?;
        let ap=document.add_object(Stream::new(dictionary!{"Type"=>"XObject","Subtype"=>"Form","FormType"=>1,"BBox"=>vec![0.into(),0.into(),bounds.width.into(),bounds.height.into()],"Resources"=>Dictionary::new()},content));
        let ink = points
            .iter()
            .flat_map(|p| [Object::Real(p.x), Object::Real(p.y)])
            .collect::<Vec<_>>();
        let mut a = dictionary! {"Type"=>"Annot","Subtype"=>"Ink","Rect"=>vec![bounds.x.into(),bounds.y.into(),(bounds.x+bounds.width).into(),(bounds.y+bounds.height).into()],"P"=>page_id,"F"=>4,"C"=>vec![color.r.into(),color.g.into(),color.b.into()],"BS"=>dictionary!{"W"=>width,"S"=>"S"},"Border"=>vec![0.into(),0.into(),width.into()],"InkList"=>vec![Object::Array(ink)],"AP"=>dictionary!{"N"=>ap}};
        a.set("M", crate::annotations::modified_now()?);
        annots.push(document.add_object(a).into());
    }
    document
        .get_object_mut(page_id)
        .and_then(Object::as_dict_mut)
        .map_err(|e| PdfMasterError::Malformed(e.to_string()))?
        .set("Annots", annots);
    validate_notes_page(document, page_id, p)
}
