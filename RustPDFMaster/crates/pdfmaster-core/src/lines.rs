//! PDF /Line geometry stays in source-page coordinates, independent of /Rotate.
use crate::document::{
    Color, PageInfo, PdfMasterError, Rect, Result, page_object_ids, validate_page_index,
};
use crate::edit::{validate_color, validate_rect};
use crate::notes::{resolve, validate_notes_page, validated_annots_for_add};
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use serde::{Deserialize, Serialize};

pub use pdfmaster_drawing::Point;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineEnding {
    None,
    Square,
    Circle,
    Diamond,
    OpenArrow,
    ClosedArrow,
    Butt,
    ROpenArrow,
    RClosedArrow,
    Slash,
}
impl LineEnding {
    fn name(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Square => "Square",
            Self::Circle => "Circle",
            Self::Diamond => "Diamond",
            Self::OpenArrow => "OpenArrow",
            Self::ClosedArrow => "ClosedArrow",
            Self::Butt => "Butt",
            Self::ROpenArrow => "ROpenArrow",
            Self::RClosedArrow => "RClosedArrow",
            Self::Slash => "Slash",
        }
    }
    fn parse(name: &[u8]) -> Result<Self> {
        [
            Self::None,
            Self::Square,
            Self::Circle,
            Self::Diamond,
            Self::OpenArrow,
            Self::ClosedArrow,
            Self::Butt,
            Self::ROpenArrow,
            Self::RClosedArrow,
            Self::Slash,
        ]
        .into_iter()
        .find(|s| s.name().as_bytes() == name)
        .ok_or(PdfMasterError::InvalidOperation("unknown line ending"))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LineGeometry {
    pub start: Point,
    pub end: Point,
    pub endings: [LineEnding; 2],
    pub dash_pattern: Vec<f32>,
}

pub(crate) fn validate_width(width: f32) -> Result<()> {
    if width.is_finite() && (0.25..=32.).contains(&width) {
        Ok(())
    } else {
        Err(PdfMasterError::InvalidGeometry(
            "line width must be in 0.25..=32 pt",
        ))
    }
}

fn validate_geometry(line: &LineGeometry, bounds: Rect) -> Result<()> {
    let inside = |p: Point| {
        p.x.is_finite()
            && p.y.is_finite()
            && p.x >= bounds.x
            && p.y >= bounds.y
            && p.x <= bounds.x + bounds.width
            && p.y <= bounds.y + bounds.height
    };
    if !inside(line.start) || !inside(line.end) || line.start == line.end {
        return Err(PdfMasterError::InvalidGeometry(
            "line endpoints must be distinct and inside its bounds",
        ));
    }
    if line.dash_pattern.len() > 32
        || line
            .dash_pattern
            .iter()
            .any(|v| !v.is_finite() || *v < 0. || *v > 1000.)
        || (!line.dash_pattern.is_empty() && line.dash_pattern.iter().all(|v| *v == 0.))
    {
        return Err(PdfMasterError::InvalidOperation(
            "unsupported line dash pattern",
        ));
    }
    Ok(())
}

pub(crate) fn parse(document: &Document, a: &Dictionary, bounds: Rect) -> Result<LineGeometry> {
    // Caption, leaders, intent/measurement semantics cannot be reproduced just
    // by drawing /L; do not silently discard them on update.
    for key in [
        b"LL".as_slice(),
        b"LLE",
        b"LLO",
        b"Cap",
        b"CP",
        b"CO",
        b"IT",
        b"Measure",
        b"ca",
    ] {
        if a.has(key) {
            return Err(PdfMasterError::InvalidOperation(
                "extended line geometry is not yet editable",
            ));
        }
    }
    let p = resolve(
        document,
        a.get(b"L")
            .map_err(|e| PdfMasterError::Malformed(e.to_string()))?,
    )?
    .as_array()
    .map_err(|e| PdfMasterError::Malformed(e.to_string()))?;
    if p.len() != 4 {
        return Err(PdfMasterError::Malformed(
            "line L requires four coordinates".into(),
        ));
    }
    let number = |v: &Object| {
        resolve(document, v)?
            .as_float()
            .map_err(|e| PdfMasterError::Malformed(e.to_string()))
    };
    let mut endings = [LineEnding::None; 2];
    if let Ok(values) = a.get(b"LE") {
        let values = resolve(document, values)?
            .as_array()
            .map_err(|e| PdfMasterError::Malformed(e.to_string()))?;
        if values.len() != 2 {
            return Err(PdfMasterError::Malformed(
                "line LE requires two names".into(),
            ));
        }
        for (out, input) in endings.iter_mut().zip(values) {
            *out = LineEnding::parse(
                resolve(document, input)?
                    .as_name()
                    .map_err(|e| PdfMasterError::Malformed(e.to_string()))?,
            )?;
        }
    }
    let mut dash_pattern = Vec::new();
    if let Ok(bs) = a.get(b"BS") {
        let bs = resolve(document, bs)?
            .as_dict()
            .map_err(|e| PdfMasterError::Malformed(e.to_string()))?;
        let style = bs
            .get(b"S")
            .ok()
            .map(|v| {
                resolve(document, v)?
                    .as_name()
                    .map_err(|e| PdfMasterError::Malformed(e.to_string()))
            })
            .transpose()?
            .unwrap_or(b"S");
        if !matches!(style, b"S" | b"D") {
            return Err(PdfMasterError::InvalidOperation(
                "unsupported line border style",
            ));
        }
        if let Ok(d) = bs.get(b"D") {
            let values = resolve(document, d)?
                .as_array()
                .map_err(|e| PdfMasterError::Malformed(e.to_string()))?;
            if values.len() > 32 {
                return Err(PdfMasterError::Budget("line dash pattern"));
            }
            if style == b"D" {
                dash_pattern = values.iter().map(number).collect::<Result<Vec<_>>>()?;
            } else {
                return Err(PdfMasterError::InvalidOperation(
                    "solid line with conflicting dash metadata",
                ));
            }
        } else if style == b"D" {
            dash_pattern.push(3.);
        }
    } else if let Ok(border) = a.get(b"Border") {
        let border = resolve(document, border)?
            .as_array()
            .map_err(|e| PdfMasterError::Malformed(e.to_string()))?;
        if let Some(d) = border.get(3) {
            let values = resolve(document, d)?
                .as_array()
                .map_err(|e| PdfMasterError::Malformed(e.to_string()))?;
            if values.len() > 32 {
                return Err(PdfMasterError::Budget("line dash pattern"));
            }
            dash_pattern = values.iter().map(number).collect::<Result<Vec<_>>>()?;
        }
    }
    let line = LineGeometry {
        start: Point {
            x: number(&p[0])?,
            y: number(&p[1])?,
        },
        end: Point {
            x: number(&p[2])?,
            y: number(&p[3])?,
        },
        endings,
        dash_pattern,
    };
    validate_geometry(&line, bounds)?;
    Ok(line)
}

pub(crate) fn resized(line: &LineGeometry, old: Rect, new: Rect) -> Result<LineGeometry> {
    validate_geometry(line, old)?;
    let transform = |p: Point| Point {
        x: new.x + ((p.x - old.x) / old.width) * new.width,
        y: new.y + ((p.y - old.y) / old.height) * new.height,
    };
    let line = LineGeometry {
        start: transform(line.start),
        end: transform(line.end),
        ..line.clone()
    };
    validate_geometry(&line, new)?;
    Ok(line)
}

pub(crate) fn write_geometry(a: &mut Dictionary, line: &LineGeometry, width: f32) {
    a.set(
        "L",
        vec![
            line.start.x.into(),
            line.start.y.into(),
            line.end.x.into(),
            line.end.y.into(),
        ],
    );
    a.set(
        "LE",
        line.endings
            .map(|s| Object::Name(s.name().as_bytes().to_vec()))
            .to_vec(),
    );
    let mut bs = dictionary! {"Type"=>"Border","W"=>width,"S"=>if line.dash_pattern.is_empty(){"S"}else{"D"}};
    if !line.dash_pattern.is_empty() {
        bs.set(
            "D",
            line.dash_pattern
                .iter()
                .copied()
                .map(Object::Real)
                .collect::<Vec<_>>(),
        );
    }
    a.set("BS", bs);
}

/// Keep requested endpoint placement independent of appearance padding. Widening
/// or shrinking a line must not clip its shaft/endings to an obsolete /BBox.
pub(crate) fn expanded_bounds(
    line: &LineGeometry,
    page: &PageInfo,
    width: f32,
    requested: Option<Rect>,
) -> Rect {
    let padding = (width * 6.).max(1.);
    let mut x = (line.start.x.min(line.end.x) - padding).max(0.);
    let mut y = (line.start.y.min(line.end.y) - padding).max(0.);
    let mut right = (line.start.x.max(line.end.x) + padding).min(page.width);
    let mut top = (line.start.y.max(line.end.y) + padding).min(page.height);
    if let Some(r) = requested {
        x = x.min(r.x);
        y = y.min(r.y);
        right = right.max(r.x + r.width);
        top = top.max(r.y + r.height);
    }
    Rect {
        x,
        y,
        width: right - x,
        height: top - y,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn add_line(
    document: &mut Document,
    pages: &[PageInfo],
    page: usize,
    start: Point,
    end: Point,
    endings: [LineEnding; 2],
    stroke: Color,
    fill: Option<Color>,
    width: f32,
) -> Result<()> {
    validate_page_index(page, pages.len())?;
    validate_width(width)?;
    validate_color(stroke)?;
    if let Some(c) = fill {
        validate_color(c)?;
    }
    let page_rect = Rect {
        x: 0.,
        y: 0.,
        width: pages[page].width,
        height: pages[page].height,
    };
    let line = LineGeometry {
        start,
        end,
        endings,
        dash_pattern: Vec::new(),
    };
    validate_geometry(&line, page_rect)?;
    // Include arrow wings and stroke extents, clipped only to the visible page.
    let bounds = expanded_bounds(&line, &pages[page], width, None);
    let (x, y) = (bounds.x, bounds.y);
    validate_rect(bounds, &pages[page])?;
    let page_id = page_object_ids(document)?[page];
    let mut annots = validated_annots_for_add(document, page_id, &pages[page])?;
    let ap=document.add_object(Stream::new(dictionary!{"Type"=>"XObject","Subtype"=>"Form","FormType"=>1,"BBox"=>vec![0.into(),0.into(),bounds.width.into(),bounds.height.into()],"Resources"=>Dictionary::new()},appearance(&line,bounds,stroke,fill,width)?));
    let mut a = dictionary! {"Type"=>"Annot","Subtype"=>"Line","Rect"=>vec![x.into(),y.into(),(x+bounds.width).into(),(y+bounds.height).into()],"P"=>page_id,"F"=>4,"C"=>vec![stroke.r.into(),stroke.g.into(),stroke.b.into()],"Border"=>vec![0.into(),0.into(),width.into()],"AP"=>dictionary!{"N"=>ap}};
    write_geometry(&mut a, &line, width);
    if let Some(c) = fill {
        a.set("IC", vec![c.r.into(), c.g.into(), c.b.into()]);
    }
    let id = document.add_object(a);
    annots.push(id.into());
    document
        .get_object_mut(page_id)
        .and_then(Object::as_dict_mut)
        .map_err(|e| PdfMasterError::Malformed(e.to_string()))?
        .set("Annots", annots);
    validate_notes_page(document, page_id, &pages[page])
}

pub(crate) fn appearance(
    line: &LineGeometry,
    bounds: Rect,
    stroke: Color,
    fill: Option<Color>,
    width: f32,
) -> Result<Vec<u8>> {
    validate_geometry(line, bounds)?;
    validate_width(width)?;
    let mut ops = vec![
        Operation::new("q", vec![]),
        Operation::new(
            "RG",
            vec![stroke.r.into(), stroke.g.into(), stroke.b.into()],
        ),
        Operation::new("w", vec![width.into()]),
        Operation::new("J", vec![0.into()]),
        Operation::new("j", vec![1.into()]),
    ];
    if let Some(c) = fill {
        ops.push(Operation::new(
            "rg",
            vec![c.r.into(), c.g.into(), c.b.into()],
        ));
    }
    let s = Point {
        x: line.start.x - bounds.x,
        y: line.start.y - bounds.y,
    };
    let e = Point {
        x: line.end.x - bounds.x,
        y: line.end.y - bounds.y,
    };
    ops.push(Operation::new(
        "d",
        vec![
            Object::Array(
                line.dash_pattern
                    .iter()
                    .copied()
                    .map(Object::Real)
                    .collect(),
            ),
            0.into(),
        ],
    ));
    path(&mut ops, "m", s);
    path(&mut ops, "l", e);
    ops.push(Operation::new("S", vec![]));
    // Ending outlines themselves are solid, even on a dashed shaft.
    ops.push(Operation::new("d", vec![Object::Array(vec![]), 0.into()]));
    let length = (e.x - s.x).hypot(e.y - s.y);
    let dx = (e.x - s.x) / length;
    let dy = (e.y - s.y) / length;
    ending(
        &mut ops,
        s,
        -dx,
        -dy,
        line.endings[0],
        width,
        fill.is_some(),
    );
    ending(&mut ops, e, dx, dy, line.endings[1], width, fill.is_some());
    ops.push(Operation::new("Q", vec![]));
    Content { operations: ops }
        .encode()
        .map_err(|e| PdfMasterError::Serialization(e.to_string()))
}
fn path(ops: &mut Vec<Operation>, operator: &str, p: Point) {
    ops.push(Operation::new(operator, vec![p.x.into(), p.y.into()]));
}
fn ending(
    ops: &mut Vec<Operation>,
    tip: Point,
    dx: f32,
    dy: f32,
    kind: LineEnding,
    w: f32,
    filled: bool,
) {
    if kind == LineEnding::None {
        return;
    }
    let p = |along: f32, across: f32| Point {
        x: tip.x + along * dx - across * dy,
        y: tip.y + along * dy + across * dx,
    };
    let closed = match kind {
        LineEnding::OpenArrow
        | LineEnding::ClosedArrow
        | LineEnding::ROpenArrow
        | LineEnding::RClosedArrow => {
            let sign = if matches!(kind, LineEnding::ROpenArrow | LineEnding::RClosedArrow) {
                1.
            } else {
                -1.
            };
            path(ops, "m", p(sign * 4. * w, 2. * w));
            path(ops, "l", tip);
            path(ops, "l", p(sign * 4. * w, -2. * w));
            matches!(kind, LineEnding::ClosedArrow | LineEnding::RClosedArrow)
        }
        LineEnding::Square => {
            for (i, (a, b)) in [(-2., -2.), (2., -2.), (2., 2.), (-2., 2.)]
                .into_iter()
                .enumerate()
            {
                path(ops, if i == 0 { "m" } else { "l" }, p(a * w, b * w));
            }
            true
        }
        LineEnding::Diamond => {
            for (i, (a, b)) in [(-3., 0.), (0., 2.), (3., 0.), (0., -2.)]
                .into_iter()
                .enumerate()
            {
                path(ops, if i == 0 { "m" } else { "l" }, p(a * w, b * w));
            }
            true
        }
        LineEnding::Circle => {
            let r = 2. * w;
            let k = r * 0.552_284_8;
            path(
                ops,
                "m",
                Point {
                    x: tip.x + r,
                    y: tip.y,
                },
            );
            for v in [
                [tip.x + r, tip.y + k, tip.x + k, tip.y + r, tip.x, tip.y + r],
                [tip.x - k, tip.y + r, tip.x - r, tip.y + k, tip.x - r, tip.y],
                [tip.x - r, tip.y - k, tip.x - k, tip.y - r, tip.x, tip.y - r],
                [tip.x + k, tip.y - r, tip.x + r, tip.y - k, tip.x + r, tip.y],
            ] {
                ops.push(Operation::new("c", v.into_iter().map(Into::into).collect()));
            }
            true
        }
        LineEnding::Butt => {
            path(ops, "m", p(0., 2. * w));
            path(ops, "l", p(0., -2. * w));
            false
        }
        LineEnding::Slash => {
            path(ops, "m", p(w, 1.732_050_8 * w));
            path(ops, "l", p(-w, -1.732_050_8 * w));
            false
        }
        LineEnding::None => return,
    };
    if closed {
        ops.push(Operation::new("h", vec![]));
    }
    ops.push(Operation::new(
        if closed && filled { "B" } else { "S" },
        vec![],
    ));
}
