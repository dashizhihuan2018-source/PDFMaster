//! Native-compatible shape values and source-page geometry; no engine or UI ownership.
pub use crate::{annotations::Color, canvas_notes::Frame, canvas_text::Rect};
use pdfmaster_drawing::Point;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Page {
    pub index: usize,
    pub width: f32,
    pub height: f32,
    pub rotation: i32,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShapeKind {
    Rectangle,
    Ellipse,
    Line,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
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
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LineGeometry {
    pub start: Point,
    pub end: Point,
    pub endings: [LineEnding; 2],
    pub dash_pattern: Vec<f32>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Annotation {
    pub annotation_index: usize,
    pub shape: ShapeKind,
    pub bounds: Rect,
    pub stroke: Color,
    pub fill: Option<Color>,
    pub line_width: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<LineGeometry>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Selection {
    pub page_index: usize,
    pub page_id: (u32, u16),
    #[serde(deserialize_with = "required_object_id")]
    pub object_id: Option<(u32, u16)>,
    pub revision: [u8; 32],
    pub flags: u32,
    #[serde(default = "opaque")]
    pub opacity: f32,
    pub original: Annotation,
}
fn required_object_id<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<(u32, u16)>, D::Error> {
    Option::deserialize(deserializer)
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct List {
    pub annotations: Vec<Selection>,
    pub unsupported_count: usize,
    pub annotation_count: usize,
    pub revision: [u8; 32],
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Geometry {
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
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Draft {
    pub geometry: Geometry,
    pub stroke: Color,
    pub fill: Option<Color>,
    pub line_width: f32,
    #[serde(default = "opaque")]
    pub opacity: f32,
}
fn opaque() -> f32 {
    1.
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    Existing { original: Selection },
    New { draft: Draft, revision: [u8; 32] },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Owner {
    pub id: u32,
    pub request: String,
    pub page: usize,
    pub target: Target,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Payload {
    pub id: u32,
    pub request: String,
    pub draft: Draft,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Rectangle,
    Ellipse,
    Line,
    Arrow,
    DoubleArrow,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub stroke: Color,
    pub fill: Option<Color>,
    pub line_width: f32,
    pub opacity: f32,
}
impl Default for Style {
    fn default() -> Self {
        Self {
            stroke: Color {
                r: 1.,
                g: 66. / 255.,
                b: 69. / 255.,
            },
            fill: None,
            line_width: 2.,
            opacity: 1.,
        }
    }
}
pub const WIDTH_PRESETS: [f32; 7] = [0.5, 1., 2., 3., 5., 8., 12.];
pub const FILL_SWATCH: Color = Color {
    r: 1.,
    g: 214. / 255.,
    b: 0.,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    Move,
    Sw,
    Se,
    Ne,
    Nw,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    Handle(Handle),
    Existing(Selection),
}
impl Kind {
    pub fn supports_fill(self) -> bool {
        matches!(self, Self::Rectangle | Self::Ellipse)
    }
    pub fn endings(self) -> [LineEnding; 2] {
        match self {
            Self::Arrow => [LineEnding::None, LineEnding::OpenArrow],
            Self::DoubleArrow => [LineEnding::OpenArrow; 2],
            _ => [LineEnding::None; 2],
        }
    }
}
pub fn transparency(value: f32) -> Result<f32, String> {
    if value.is_finite() && (0. ..=100.).contains(&value) {
        Ok(1. - value / 100.)
    } else {
        Err("transparency must be in 0..=100".into())
    }
}
impl Page {
    pub(crate) fn validate(self) -> Result<(), String> {
        if self.width.is_finite()
            && self.height.is_finite()
            && self.width > 0.
            && self.height > 0.
            && self.rotation % 90 == 0
        {
            Ok(())
        } else {
            Err("invalid source page".into())
        }
    }
}
fn point_inside(p: Point, b: Rect) -> bool {
    p.x.is_finite()
        && p.y.is_finite()
        && p.x >= b.x
        && p.y >= b.y
        && p.x <= b.x + b.width
        && p.y <= b.y + b.height
}
pub(crate) fn valid_rect(b: Rect, page: Page) -> Result<(), String> {
    page.validate()?;
    if [b.x, b.y, b.width, b.height, b.x + b.width, b.y + b.height]
        .iter()
        .all(|v| v.is_finite())
        && b.x >= 0.
        && b.y >= 0.
        && b.width > 0.
        && b.height > 0.
        && b.x + b.width <= page.width
        && b.y + b.height <= page.height
    {
        Ok(())
    } else {
        Err("invalid shape frame".into())
    }
}
fn page_rect(p: Page) -> Rect {
    Rect {
        x: 0.,
        y: 0.,
        width: p.width,
        height: p.height,
    }
}
fn expanded(start: Point, end: Point, pad: f32, page: Page) -> Rect {
    let x = (start.x.min(end.x) - pad).max(0.);
    let y = (start.y.min(end.y) - pad).max(0.);
    Rect {
        x,
        y,
        width: (start.x.max(end.x) + pad).min(page.width) - x,
        height: (start.y.max(end.y) + pad).min(page.height) - y,
    }
}
impl Draft {
    pub fn validate(&self, page: Page) -> Result<(), String> {
        page.validate()?;
        let color = |c: Color| {
            [c.r, c.g, c.b]
                .iter()
                .all(|v| v.is_finite() && (0. ..=1.).contains(v))
        };
        if !color(self.stroke)
            || self.fill.is_some_and(|c| !color(c))
            || !self.opacity.is_finite()
            || !(0. ..=1.).contains(&self.opacity)
        {
            return Err("invalid shape color or opacity".into());
        }
        let b = self.bounds(page)?;
        match &self.geometry {
            Geometry::Rectangle { .. } | Geometry::Ellipse { .. } => {
                if !self.line_width.is_finite()
                    || self.line_width <= 0.
                    || self.line_width >= b.width.min(b.height)
                {
                    return Err("area stroke must be positive and smaller than its frame".into());
                }
            }
            Geometry::Line {
                start,
                end,
                dash_pattern,
                ..
            } => {
                if !self.line_width.is_finite() || !(0.25..=32.).contains(&self.line_width) {
                    return Err("line width must be in 0.25..=32".into());
                }
                if !point_inside(*start, b) || !point_inside(*end, b) || start == end {
                    return Err("line endpoints must be distinct and inside its frame".into());
                }
                if dash_pattern.len() > 32
                    || dash_pattern
                        .iter()
                        .any(|v| !v.is_finite() || !(0. ..=1000.).contains(v))
                    || (!dash_pattern.is_empty() && dash_pattern.iter().all(|v| *v == 0.))
                {
                    return Err("unsupported line dash pattern".into());
                }
            }
        }
        Ok(())
    }
    pub fn bounds(&self, page: Page) -> Result<Rect, String> {
        page.validate()?;
        let b = match &self.geometry {
            Geometry::Rectangle { bounds } | Geometry::Ellipse { bounds } => *bounds,
            Geometry::Line {
                start, end, bounds, ..
            } => match bounds {
                Some(b) => *b,
                None => {
                    if !self.line_width.is_finite()
                        || !(0.25..=32.).contains(&self.line_width)
                        || !point_inside(*start, page_rect(page))
                        || !point_inside(*end, page_rect(page))
                    {
                        return Err("invalid historical line frame".into());
                    }
                    expanded(*start, *end, 6. * self.line_width, page)
                }
            },
        };
        valid_rect(b, page)?;
        Ok(b)
    }
}
impl Selection {
    pub fn draft(&self) -> Result<Draft, String> {
        let a = &self.original;
        let geometry = match a.shape {
            ShapeKind::Rectangle => Geometry::Rectangle { bounds: a.bounds },
            ShapeKind::Ellipse => Geometry::Ellipse { bounds: a.bounds },
            ShapeKind::Line => {
                let l = a
                    .line
                    .as_ref()
                    .ok_or("line annotation has no line geometry")?;
                Geometry::Line {
                    start: l.start,
                    end: l.end,
                    endings: l.endings,
                    dash_pattern: l.dash_pattern.clone(),
                    bounds: Some(a.bounds),
                }
            }
        };
        Ok(Draft {
            geometry,
            stroke: a.stroke,
            fill: a.fill,
            line_width: a.line_width,
            opacity: self.opacity,
        })
    }
}
pub fn new_draft(
    kind: Kind,
    start: Point,
    end: Point,
    style: Style,
    page: Page,
) -> Result<Draft, String> {
    page.validate()?;
    if !point_inside(start, page_rect(page)) || !point_inside(end, page_rect(page)) {
        return Err("initial shape points outside source page".into());
    }
    let bounds = Rect {
        x: start.x.min(end.x),
        y: start.y.min(end.y),
        width: (start.x - end.x).abs(),
        height: (start.y - end.y).abs(),
    };
    let geometry = match kind {
        Kind::Rectangle => Geometry::Rectangle { bounds },
        Kind::Ellipse => Geometry::Ellipse { bounds },
        _ => Geometry::Line {
            start,
            end,
            endings: kind.endings(),
            dash_pattern: vec![],
            bounds: Some(expanded(start, end, style.line_width.max(1.), page)),
        },
    };
    let d = Draft {
        geometry,
        stroke: style.stroke,
        fill: style.fill,
        line_width: style.line_width,
        opacity: style.opacity,
    };
    d.validate(page)?;
    Ok(d)
}
pub fn transform(draft: &Draft, bounds: Rect, page: Page) -> Result<Draft, String> {
    draft.validate(page)?;
    valid_rect(bounds, page)?;
    let old = draft.bounds(page)?;
    let map = |p: Point| Point {
        x: (p.x - old.x) / old.width * bounds.width + bounds.x,
        y: (p.y - old.y) / old.height * bounds.height + bounds.y,
    };
    let geometry = match &draft.geometry {
        Geometry::Rectangle { .. } => Geometry::Rectangle { bounds },
        Geometry::Ellipse { .. } => Geometry::Ellipse { bounds },
        Geometry::Line {
            start,
            end,
            endings,
            dash_pattern,
            ..
        } => Geometry::Line {
            start: map(*start),
            end: map(*end),
            endings: *endings,
            dash_pattern: dash_pattern.clone(),
            bounds: Some(bounds),
        },
    };
    let result = Draft {
        geometry,
        ..draft.clone()
    };
    result.validate(page)?;
    Ok(result)
}
pub fn drag_bounds(
    original: Rect,
    anchor: Point,
    point: Point,
    handle: Handle,
    page: Page,
) -> Result<Rect, String> {
    valid_rect(original, page)?;
    if [anchor.x, anchor.y, point.x, point.y]
        .iter()
        .any(|v| !v.is_finite())
    {
        return Err("invalid drag points".into());
    }
    let b = if handle == Handle::Move {
        let dx = (point.x - anchor.x).clamp(-original.x, page.width - original.x - original.width);
        let dy =
            (point.y - anchor.y).clamp(-original.y, page.height - original.y - original.height);
        Rect {
            x: original.x + dx,
            y: original.y + dy,
            ..original
        }
    } else {
        if !point_inside(point, page_rect(page)) {
            return Err("resize outside source page".into());
        }
        let opposite = match handle {
            Handle::Sw => Point {
                x: original.x + original.width,
                y: original.y + original.height,
            },
            Handle::Se => Point {
                x: original.x,
                y: original.y + original.height,
            },
            Handle::Ne => Point {
                x: original.x,
                y: original.y,
            },
            Handle::Nw => Point {
                x: original.x + original.width,
                y: original.y,
            },
            Handle::Move => unreachable!(),
        };
        Rect {
            x: point.x.min(opposite.x),
            y: point.y.min(opposite.y),
            width: (point.x - opposite.x).abs(),
            height: (point.y - opposite.y).abs(),
        }
    };
    valid_rect(b, page)?;
    Ok(b)
}
pub fn display_corners(
    bounds: Rect,
    frame: Frame,
    page: Page,
) -> Result<[(Handle, Point); 4], String> {
    valid_rect(bounds, page)?;
    if !frame.valid() {
        return Err("invalid decoded image frame".into());
    }
    let source = [
        (
            Handle::Sw,
            Point {
                x: bounds.x,
                y: bounds.y,
            },
        ),
        (
            Handle::Se,
            Point {
                x: bounds.x + bounds.width,
                y: bounds.y,
            },
        ),
        (
            Handle::Ne,
            Point {
                x: bounds.x + bounds.width,
                y: bounds.y + bounds.height,
            },
        ),
        (
            Handle::Nw,
            Point {
                x: bounds.x,
                y: bounds.y + bounds.height,
            },
        ),
    ];
    let mut result = source;
    for (_, p) in &mut result {
        *p = crate::canvas_notes::page_anchor(*p, frame, page.width, page.height, page.rotation)
            .map_err(String::from)?;
        if !p.x.is_finite() || !p.y.is_finite() {
            return Err("display corner overflow".into());
        }
    }
    Ok(result)
}
fn validate_list(list: &List, page: Page) -> Result<(), String> {
    page.validate()?;
    if list.annotation_count > 1024
        || list.annotations.len().checked_add(list.unsupported_count) != Some(list.annotation_count)
    {
        return Err("invalid shape list count".into());
    }
    let mut indices = std::collections::HashSet::new();
    let mut page_id = None;
    for s in &list.annotations {
        if s.page_index != page.index
            || s.revision != list.revision
            || s.original.annotation_index >= list.annotation_count
            || !indices.insert(s.original.annotation_index)
            || page_id.is_some_and(|id| id != s.page_id)
        {
            return Err("invalid shape list identity or revision".into());
        }
        page_id = Some(s.page_id);
        s.draft()?.validate(page)?;
    }
    Ok(())
}
fn row_matches(row: &crate::annotations::Row, s: &Selection) -> bool {
    row.page_index == s.page_index
        && row.page_id == s.page_id
        && row.object_id == s.object_id
        && row.annotation_index == s.original.annotation_index
        && row.subtype
            == match s.original.shape {
                ShapeKind::Rectangle => "Square",
                ShapeKind::Ellipse => "Circle",
                ShapeKind::Line => "Line",
            }
}
// Generic hit evidence is not an editable Draft. Preserve off-page rectangles
// without weakening the strict supported-shape/new-draft page boundary.
fn hit_corners(b: Rect, frame: Frame, page: Page) -> Result<[(Handle, Point); 4], String> {
    if ![b.x, b.y, b.width, b.height, b.x + b.width, b.y + b.height]
        .iter()
        .all(|v| v.is_finite())
        || b.width <= 0.
        || b.height <= 0.
    {
        return Err("annotation hit bounds unavailable".into());
    }
    let (w, h) = if page.rotation.rem_euclid(180) == 0 {
        (page.width, page.height)
    } else {
        (page.height, page.width)
    };
    let mut corners = [
        (Handle::Sw, Point { x: b.x, y: b.y }),
        (
            Handle::Se,
            Point {
                x: b.x + b.width,
                y: b.y,
            },
        ),
        (
            Handle::Ne,
            Point {
                x: b.x + b.width,
                y: b.y + b.height,
            },
        ),
        (
            Handle::Nw,
            Point {
                x: b.x,
                y: b.y + b.height,
            },
        ),
    ];
    for (_, point) in &mut corners {
        let p = pdfmaster_drawing::display_point(*point, page.width, page.height, page.rotation);
        *point = Point {
            x: frame.left + p.x / w * frame.width,
            y: frame.top + p.y / h * frame.height,
        };
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err("display corner overflow".into());
        }
    }
    Ok(corners)
}
pub fn hit_test(
    list: &List,
    rows: &[crate::annotations::Row],
    selected: Option<&Selection>,
    client: Point,
    frame: Frame,
    page: Page,
) -> Result<Option<Hit>, String> {
    page.validate()?;
    if !frame.valid() || !client.x.is_finite() || !client.y.is_finite() {
        return Err("invalid hit plane".into());
    }
    validate_list(list, page)?;
    if let Some(s) = selected {
        if !list.annotations.contains(s) {
            return Err("selected occurrence is stale".into());
        }
        for (handle, p) in display_corners(s.draft()?.bounds(page)?, frame, page)? {
            if (p.x - client.x).abs() <= 7. && (p.y - client.y).abs() <= 7. {
                if s.flags & (64 | 128 | 512) != 0 {
                    return Err("selected annotation is locked".into());
                }
                return Ok(Some(Hit::Handle(handle)));
            }
        }
    }
    crate::canvas_notes::click_page_point(client, frame, page.width, page.height, page.rotation)
        .map_err(String::from)?;
    let mut current: Vec<_> = rows.iter().filter(|r| r.page_index == page.index).collect();
    let mut indices = std::collections::HashSet::new();
    for r in &current {
        if r.annotation_index >= list.annotation_count || !indices.insert(r.annotation_index) {
            return Err("invalid generic annotation occurrence".into());
        }
    }
    // Generic projections can omit non-shape subtypes. A gap is uncertainty,
    // not evidence that only harmless Links were filtered out.
    if indices.len() != list.annotation_count {
        return Err("generic annotation occurrences incomplete".into());
    }
    for s in &list.annotations {
        if !current.iter().any(|r| row_matches(r, s)) {
            return Err("generic/typed annotation identity mismatch".into());
        }
    }
    // An unbounded non-Link annotation might cover any blank-space click.
    for r in &current {
        if r.subtype != "Link" {
            let b = r.bounds.ok_or("annotation hit bounds unavailable")?;
            hit_corners(
                Rect {
                    x: b.x,
                    y: b.y,
                    width: b.width,
                    height: b.height,
                },
                frame,
                page,
            )?;
        }
    }
    current.sort_by_key(|r| std::cmp::Reverse(r.annotation_index));
    for r in current {
        if r.subtype == "Link" {
            continue;
        }
        let b = r.bounds.ok_or("annotation hit bounds unavailable")?;
        let corners = hit_corners(
            Rect {
                x: b.x,
                y: b.y,
                width: b.width,
                height: b.height,
            },
            frame,
            page,
        )?;
        let min_x = corners
            .iter()
            .map(|(_, p)| p.x)
            .fold(f32::INFINITY, f32::min);
        let max_x = corners
            .iter()
            .map(|(_, p)| p.x)
            .fold(f32::NEG_INFINITY, f32::max);
        let min_y = corners
            .iter()
            .map(|(_, p)| p.y)
            .fold(f32::INFINITY, f32::min);
        let max_y = corners
            .iter()
            .map(|(_, p)| p.y)
            .fold(f32::NEG_INFINITY, f32::max);
        if client.x >= min_x - 4.
            && client.x <= max_x + 4.
            && client.y >= min_y - 4.
            && client.y <= max_y + 4.
        {
            let s = list
                .annotations
                .iter()
                .find(|s| row_matches(r, s))
                .ok_or("topmost annotation is unsupported")?;
            if s.flags & (64 | 128 | 512) != 0 {
                return Err("topmost annotation is locked".into());
            }
            return Ok(Some(Hit::Existing(s.clone())));
        }
    }
    Ok(None)
}
pub fn reconcile(
    list: &List,
    index: usize,
    expected: &Draft,
    page: Page,
) -> Result<Selection, String> {
    validate_list(list, page)?;
    expected.validate(page)?;
    let s = list
        .annotations
        .iter()
        .find(|s| s.original.annotation_index == index)
        .ok_or("expected shape occurrence is absent")?;
    let mut normalized = expected.clone();
    let b = expected.bounds(page)?;
    let b = Rect {
        width: (b.x + b.width) - b.x,
        height: (b.y + b.height) - b.y,
        ..b
    };
    match &mut normalized.geometry {
        Geometry::Rectangle { bounds } | Geometry::Ellipse { bounds } => *bounds = b,
        Geometry::Line { bounds, .. } => *bounds = Some(b),
    }
    if s.draft()? != normalized {
        return Err("saved shape does not match expected occurrence".into());
    }
    Ok(s.clone())
}
