//! Native FreeText wire and page-local editing state. No PDF engine dependency.
pub use crate::annotations::Color;
pub use crate::canvas_notes::Frame;
use pdfmaster_drawing::Point;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Font {
    Sans,
    CjkSc,
    Arabic,
    Devanagari,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Alignment {
    Left,
    Center,
    Right,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Appearance {
    pub font: Font,
    pub font_size: f32,
    pub color: Color,
    pub alignment: Alignment,
    pub line_height: f32,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ImportedStyle {
    pub font_resource: Vec<u8>,
    pub font_size: f32,
    pub color: Color,
    pub alignment: Alignment,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Annotation {
    pub page_index: usize,
    pub annotation_index: usize,
    pub page_id: (u32, u16),
    pub object_id: Option<(u32, u16)>,
    pub bounds: Rect,
    pub contents: String,
    pub style: ImportedStyle,
    pub flags: u32,
    pub revision: [u8; 32],
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    Existing { original: Annotation },
    New { bounds: Rect, style: Appearance },
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Owner {
    pub id: u32,
    pub request: String,
    pub page: usize,
    pub target: Target,
    #[serde(default)]
    pub appearance_style: Option<Appearance>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Payload {
    pub id: u32,
    pub request: String,
    pub contents: String,
    pub bounds: Rect,
    pub style: Option<Appearance>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct List {
    pub annotations: Vec<Annotation>,
    pub unsupported_count: usize,
}

pub fn text_matrix(
    bounds: Rect,
    frame: Frame,
    width: f32,
    height: f32,
    rotation: i32,
) -> Result<[f32; 6], &'static str> {
    if !valid_rect(bounds, width, height) || !frame.valid() || rotation.rem_euclid(90) != 0 {
        return Err("invalid text plane");
    }
    let map =
        |x, y| crate::canvas_notes::page_anchor(Point { x, y }, frame, width, height, rotation);
    let origin = map(bounds.x, bounds.y + bounds.height)?;
    let right = map(bounds.x + bounds.width, bounds.y + bounds.height)?;
    let bottom = map(bounds.x, bounds.y)?;
    Ok([
        (right.x - origin.x) / bounds.width,
        (right.y - origin.y) / bounds.width,
        (bottom.x - origin.x) / bounds.height,
        (bottom.y - origin.y) / bounds.height,
        origin.x,
        origin.y,
    ])
}
pub fn local_delta(matrix: [f32; 6], delta: Point) -> Option<Point> {
    if matrix.iter().any(|v| !v.is_finite()) || !delta.x.is_finite() || !delta.y.is_finite() {
        return None;
    }
    let [a, b, c, d, _, _] = matrix;
    let det = a * d - b * c;
    if !det.is_finite() || det.abs() < f32::EPSILON {
        return None;
    }
    Some(Point {
        x: (d * delta.x - c * delta.y) / det,
        y: (a * delta.y - b * delta.x) / det,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    Move,
    Nw,
    N,
    Ne,
    E,
    Se,
    S,
    Sw,
    W,
}
pub fn drag_rect(
    bounds: Rect,
    delta: Point,
    handle: Handle,
    width: f32,
    height: f32,
) -> Option<Rect> {
    if !valid_rect(bounds, width, height) || !delta.x.is_finite() || !delta.y.is_finite() {
        return None;
    }
    if handle == Handle::Move {
        return Some(Rect {
            x: (bounds.x + delta.x).clamp(0., width - bounds.width),
            y: (bounds.y - delta.y).clamp(0., height - bounds.height),
            ..bounds
        });
    }
    let (mut left, mut right, mut top, mut bottom) = (
        bounds.x,
        bounds.x + bounds.width,
        height - bounds.y - bounds.height,
        height - bounds.y,
    );
    let min_width = bounds.width.min(1.);
    let min_height = bounds.height.min(1.);
    if matches!(handle, Handle::Nw | Handle::W | Handle::Sw) {
        left = (left + delta.x).clamp(0., right - min_width);
    }
    if matches!(handle, Handle::Ne | Handle::E | Handle::Se) {
        right = (right + delta.x).clamp(left + min_width, width);
    }
    if matches!(handle, Handle::Nw | Handle::N | Handle::Ne) {
        top = (top + delta.y).clamp(0., bottom - min_height);
    }
    if matches!(handle, Handle::Sw | Handle::S | Handle::Se) {
        bottom = (bottom + delta.y).clamp(top + min_height, height);
    }
    Some(Rect {
        x: left,
        y: height - bottom,
        width: right - left,
        height: bottom - top,
    })
}

pub fn valid_rect(r: Rect, width: f32, height: f32) -> bool {
    [r.x, r.y, r.width, r.height, width, height]
        .iter()
        .all(|v| v.is_finite())
        && r.x >= 0.
        && r.y >= 0.
        && r.width > 0.
        && r.height > 0.
        && r.x + r.width <= width
        && r.y + r.height <= height
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            font: Font::Sans,
            font_size: 18.,
            color: Color {
                r: 0.,
                g: 0.,
                b: 0.,
            },
            alignment: Alignment::Left,
            line_height: 1.3,
        }
    }
}
impl Owner {
    pub fn bounds(&self) -> Rect {
        match &self.target {
            Target::New { bounds, .. } => *bounds,
            Target::Existing { original } => original.bounds,
        }
    }
    pub fn contents(&self) -> String {
        match &self.target {
            Target::New { .. } => String::new(),
            Target::Existing { original } => original.contents.clone(),
        }
    }
    pub fn flags(&self) -> u32 {
        match &self.target {
            Target::New { .. } => 0,
            Target::Existing { original } => original.flags,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Draft {
    pub owner: Owner,
    pub contents: String,
    pub bounds: Rect,
    pub style: Option<Appearance>,
    pub manual_width: bool,
}
impl Draft {
    pub fn new(owner: Owner) -> Self {
        Self {
            contents: owner.contents(),
            bounds: owner.bounds(),
            style: owner.appearance_style.clone(),
            owner,
            manual_width: false,
        }
    }
    pub fn body_locked(&self) -> bool {
        self.owner.flags() & (64 | 512) != 0
    }
    pub fn geometry_locked(&self) -> bool {
        self.owner.flags() & (64 | 128) != 0
    }
    pub fn validate(&self, width: f32, height: f32) -> Result<(), String> {
        if self.contents.len() > 65_536 {
            return Err("文字超过 64 KiB UTF-8 预算；正文已保留，请删减后再提交。".into());
        }
        let original = self.owner.bounds();
        let body_changed = self.contents != self.owner.contents();
        let geometry_changed = self.bounds != original;
        let style_changed = self.style != self.owner.appearance_style;
        if !valid_rect(self.bounds, width, height) {
            return Err("文字范围须完整位于页面内。".into());
        }
        if self.body_locked() && body_changed {
            return Err("此文字正文已锁定。".into());
        }
        if self.geometry_locked() && (geometry_changed || style_changed) {
            return Err("此文字位置、尺寸与格式已锁定。".into());
        }
        if self.style.is_none()
            && (body_changed
                || self.bounds.width != original.width
                || self.bounds.height != original.height)
        {
            return Err("原文字体未知；修改正文或尺寸前请明确选择打包字体。".into());
        }
        Ok(())
    }
    pub fn payload(&self) -> Payload {
        Payload {
            id: self.owner.id,
            request: self.owner.request.clone(),
            contents: self.contents.clone(),
            bounds: self.bounds,
            style: self.style.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    Idle,
    BeginPending,
    Preparing,
    Editing,
    Committing,
    Uncertain,
    Cancelling,
    Handoff,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub generation: u64,
    pub document_id: u32,
    pub page: usize,
    pub request: String,
}
#[derive(Clone, Debug)]
pub struct Session<I> {
    pub phase: Phase,
    pub ticket: Option<Ticket>,
    pub draft: Option<Draft>,
    pub frozen: Option<Payload>,
    pub intent: Option<I>,
    pub composing: bool,
    pub finish_requested: bool,
    pub error: Option<String>,
    generation: u64,
    target: Option<Target>,
}
impl<I> Default for Session<I> {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            ticket: None,
            draft: None,
            frozen: None,
            intent: None,
            composing: false,
            finish_requested: false,
            error: None,
            generation: 0,
            target: None,
        }
    }
}
impl<I> Session<I> {
    pub fn active(&self) -> bool {
        self.phase != Phase::Idle
    }
    pub fn begin(
        &mut self,
        document_id: u32,
        page: usize,
        request: String,
        target: Target,
    ) -> Option<Ticket> {
        if self.active() {
            return None;
        }
        self.generation = self.generation.checked_add(1)?;
        let ticket = Ticket {
            generation: self.generation,
            document_id,
            page,
            request,
        };
        self.phase = Phase::BeginPending;
        self.ticket = Some(ticket.clone());
        self.target = Some(target);
        Some(ticket)
    }
    pub fn owns(&self, ticket: &Ticket) -> bool {
        self.ticket.as_ref() == Some(ticket)
    }
    pub fn matches_owner(&self, document_id: u32, owner: &Owner) -> bool {
        self.ticket.as_ref().is_some_and(|t| {
            t.document_id == document_id && t.page == owner.page && t.request == owner.request
        }) && self.draft.as_ref().is_some_and(|d| d.owner == *owner)
    }
    pub fn accept(&mut self, ticket: &Ticket, owner: Owner) -> bool {
        if !self.owns(ticket)
            || self.phase != Phase::BeginPending
            || owner.id == 0
            || owner.page != ticket.page
            || owner.request != ticket.request
            || Some(&owner.target) != self.target.as_ref()
        {
            return false;
        }
        self.draft = Some(Draft::new(owner));
        self.phase = Phase::Preparing;
        true
    }
    pub fn queue(&mut self, intent: Option<I>) {
        if self.intent.is_none() {
            self.intent = intent;
        }
        self.finish_requested = true;
    }
    pub fn freeze(&mut self, width: f32, height: f32) -> Result<Option<Payload>, String> {
        if self.phase != Phase::Editing || self.composing {
            return Ok(None);
        }
        let Some(draft) = &self.draft else {
            return Ok(None);
        };
        if let Err(error) = draft.validate(width, height) {
            self.reject_local(error.clone());
            return Err(error);
        }
        let payload = draft.payload();
        self.frozen = Some(payload.clone());
        self.phase = Phase::Committing;
        self.error = None;
        Ok(Some(payload))
    }
    /// A refused local attempt has no native result to recover or action to replay.
    pub fn reject_local(&mut self, error: String) {
        if self.phase == Phase::Editing && !self.composing {
            self.intent = None;
            self.finish_requested = false;
            self.error = Some(error);
        }
    }
    pub fn retry(&mut self) -> Option<Payload> {
        if self.phase != Phase::Uncertain {
            return None;
        }
        let payload = self.frozen.clone()?;
        self.phase = Phase::Committing;
        self.error = None;
        Some(payload)
    }
    pub fn uncertain(&mut self, ticket: &Ticket, error: String) {
        if self.owns(ticket) {
            self.phase = Phase::Uncertain;
            self.error = Some(error);
        }
    }
    pub fn reject(&mut self, ticket: &Ticket, error: String) {
        if self.owns(ticket) {
            self.phase = Phase::Editing;
            self.frozen = None;
            self.intent = None;
            self.finish_requested = false;
            self.error = Some(error);
        }
    }
    pub fn finish(&mut self, ticket: &Ticket, document_id: u32, owner_ended: bool) -> Option<I> {
        if !self.owns(ticket) || document_id != ticket.document_id || !owner_ended {
            return None;
        }
        let intent = self.intent.take();
        let generation = self.generation;
        *self = Self::default();
        self.generation = generation;
        intent
    }
}
