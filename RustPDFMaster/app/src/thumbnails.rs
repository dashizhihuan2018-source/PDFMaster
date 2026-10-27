use std::{collections::BTreeMap, ops::Range};

pub const ROW_HEIGHT: f64 = 200.;
const MAX_ROWS: usize = 16;
const MAX_IMAGE_BYTES: usize = 262144;

/// Only these page rows exist in the DOM. Spacer heights retain the real
/// document scroll extent without allocating a control/image for every page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub start: usize,
    pub end: usize,
    count: usize,
}
impl Window {
    /// Grid viewport: at most 256 previews, each admitted image <=256KiB.
    pub fn for_pages(pages: Range<usize>, count: usize) -> Self {
        let start = pages.start.min(count);
        let end = pages
            .end
            .min(count)
            .min(start.saturating_add(256))
            .max(start);
        Self { start, end, count }
    }
    pub fn new(scroll: f64, height: f64, count: usize) -> Self {
        let height = if height.is_finite() && height > 0. {
            height
        } else {
            600.
        };
        let total = count as f64 * ROW_HEIGHT;
        let scroll = if scroll.is_finite() {
            scroll.max(0.).min((total - height).max(0.))
        } else {
            0.
        };
        let start = ((scroll / ROW_HEIGHT).floor() as usize)
            .saturating_sub(2)
            .min(count);
        let end = (((scroll + height) / ROW_HEIGHT).ceil() as usize)
            .saturating_add(2)
            .min(start.saturating_add(MAX_ROWS))
            .min(count);
        Self { start, end, count }
    }
    pub fn rows(self) -> Range<usize> {
        self.start..self.end
    }
    pub fn before(self) -> f64 {
        self.start as f64 * ROW_HEIGHT
    }
    pub fn after(self) -> f64 {
        self.count.saturating_sub(self.end) as f64 * ROW_HEIGHT
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Image {
    Ready(String),
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ticket {
    generation: u64,
    serial: u64,
    page: usize,
}

#[derive(Default)]
pub struct Cache {
    generation: u64,
    serial: u64,
    exhausted: bool,
    active: Option<Ticket>,
    entries: BTreeMap<usize, Image>,
}
impl Cache {
    pub fn suspend(&mut self, ticket: Ticket) {
        if self.active == Some(ticket) {
            self.exhausted = true;
        }
    }
    pub fn suspended(&self) -> bool {
        self.exhausted
    }
    pub fn next(&self, window: Window) -> Option<usize> {
        if self.active.is_some() || self.exhausted {
            return None;
        }
        window.rows().find(|page| !self.entries.contains_key(page))
    }
    pub fn begin(&mut self, page: usize, window: Window) -> Option<Ticket> {
        if self.active.is_some()
            || self.exhausted
            || !window.rows().contains(&page)
            || self.entries.contains_key(&page)
        {
            return None;
        }
        let Some(serial) = self.serial.checked_add(1) else {
            self.exhausted = true;
            return None;
        };
        self.entries.retain(|page, _| window.rows().contains(page));
        self.serial = serial;
        let ticket = Ticket {
            generation: self.generation,
            serial,
            page,
        };
        self.active = Some(ticket);
        Some(ticket)
    }
    pub fn invalidate(&mut self) -> bool {
        self.entries.clear();
        // Do not release the active RPC: it still occupies one native request
        // slot. Its eventual completion releases that slot but cannot repaint.
        let Some(generation) = self.generation.checked_add(1) else {
            self.exhausted = true;
            return false;
        };
        self.generation = generation;
        !self.exhausted
    }
    pub fn finish(
        &mut self,
        ticket: Ticket,
        window: Window,
        result: Result<String, String>,
    ) -> bool {
        if self.active != Some(ticket) {
            return false;
        }
        self.active = None;
        if self.exhausted
            || ticket.generation != self.generation
            || !window.rows().contains(&ticket.page)
        {
            return false;
        }
        self.entries.retain(|page, _| window.rows().contains(page));
        let image = match result {
            Ok(value)
                if value.len() <= MAX_IMAGE_BYTES
                    && value
                        .strip_prefix("data:image/png;base64,")
                        .is_some_and(|v| !v.is_empty()) =>
            {
                Image::Ready(value)
            }
            Ok(_) => Image::Failed("缩略图响应无效或超过大小预算。".into()),
            Err(message) => Image::Failed(if message.is_empty() {
                "此页暂无法生成缩略图。".into()
            } else {
                message.chars().take(240).collect()
            }),
        };
        self.entries.insert(ticket.page, image);
        true
    }
    pub fn image(&self, page: usize) -> Option<&Image> {
        self.entries.get(&page)
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn retry(&mut self, page: usize) -> bool {
        if self.exhausted {
            return false;
        }
        if matches!(self.entries.get(&page), Some(Image::Failed(_))) {
            self.entries.remove(&page);
            true
        } else {
            false
        }
    }
}

/// The original thumbnail's CSS box is 124x164; density affects raster pixels
/// only. A 2x cap bounds a thumbnail to at most 248x328 source pixels.
pub fn thumbnail_scale(width: f32, height: f32, rotation: i32, density: f32) -> Option<f32> {
    if !width.is_finite() || !height.is_finite() || width <= 0. || height <= 0. {
        return None;
    }
    let (width, height) = match rotation.rem_euclid(360) {
        0 | 180 => (width, height),
        90 | 270 => (height, width),
        _ => return None,
    };
    let density = if density.is_finite() {
        density.clamp(1., 2.)
    } else {
        1.
    };
    let scale = (124. * density / width)
        .min(164. * density / height)
        .min(3.);
    (scale >= 0.01).then_some(scale)
}
