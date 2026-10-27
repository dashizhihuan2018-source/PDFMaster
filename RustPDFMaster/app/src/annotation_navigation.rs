use crate::annotations::{Identity, Row};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Page {
    pub width: f64,
    pub height: f64,
    pub rotation: i32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub generation: u64,
    pub page: usize,
    pub identity: Identity,
    pub region: Region,
}
#[derive(Clone, Copy, Debug)]
pub struct Context {
    pub generation: Option<u64>,
    pub page: usize,
    pub selection: Option<Identity>,
}
impl Target {
    pub fn new(row: &Row, generation: Option<u64>, page: Page) -> Option<Self> {
        let generation = generation?;
        let b = row.bounds?;
        if [page.width, page.height]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.)
            || page.rotation % 90 != 0
            || [b.x, b.y, b.width, b.height].iter().any(|v| !v.is_finite())
            || b.width <= 0.
            || b.height <= 0.
        {
            return None;
        }
        // PDF bounds are f32; widen before addition/padding to avoid overflow.
        let (x, y) = (f64::from(b.x), f64::from(b.y));
        let (right, top) = (x + f64::from(b.width), y + f64::from(b.height));
        if right <= 0. || top <= 0. || x >= page.width || y >= page.height {
            return None;
        }
        let (left, bottom, right, top) = (
            (x - 12.).max(0.),
            (y - 12.).max(0.),
            (right + 12.).min(page.width),
            (top + 12.).min(page.height),
        );
        let (w, h) = (page.width, page.height);
        let region = match page.rotation.rem_euclid(360) {
            0 => Region {
                left: left / w,
                top: (h - top) / h,
                right: right / w,
                bottom: (h - bottom) / h,
            },
            90 => Region {
                left: bottom / h,
                top: left / w,
                right: top / h,
                bottom: right / w,
            },
            180 => Region {
                left: (w - right) / w,
                top: bottom / h,
                right: (w - left) / w,
                bottom: top / h,
            },
            _ => Region {
                left: (h - top) / h,
                top: (w - right) / w,
                right: (h - bottom) / h,
                bottom: (w - left) / w,
            },
        };
        Some(Self {
            generation,
            page: row.page_index,
            identity: row.identity(),
            region,
        })
    }
    fn current(self, context: Context) -> bool {
        context.generation == Some(self.generation)
            && context.page == self.page
            && context.selection == Some(self.identity)
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Axis {
    pub scroll: f64,
    pub viewport: f64,
    pub viewport_start: f64,
    pub border: f64,
    pub page_start: f64,
    pub page_extent: f64,
}
impl Axis {
    pub fn reveal(self, start: f64, end: f64) -> Option<i32> {
        if [
            self.scroll,
            self.viewport,
            self.viewport_start,
            self.border,
            self.page_start,
            self.page_extent,
            start,
            end,
        ]
        .iter()
        .any(|v| !v.is_finite())
            || self.viewport <= 0.
            || self.page_extent <= 0.
            || self.scroll < 0.
            || self.border < 0.
            || start < 0.
            || end > 1.
            || end <= start
        {
            return None;
        }
        let origin = self.scroll + self.page_start - self.viewport_start - self.border;
        let (start, end) = (
            origin + start * self.page_extent,
            origin + end * self.page_extent,
        );
        if !start.is_finite() || !end.is_finite() {
            return None;
        }
        Some(crate::reveal_scroll(self.scroll, start, end, self.viewport).round() as i32)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Waiting,
    Rendered,
    Scheduled,
}
#[derive(Clone, Copy, Debug)]
struct Pending {
    ticket: u64,
    target: Target,
    phase: Phase,
}
/// A single reveal, owned by one navigation and one successful raster reply.
/// No document bytes or image payloads are retained here.
#[derive(Clone, Debug, Default)]
pub struct Navigation {
    serial: u64,
    pending: Option<Pending>,
}
impl Navigation {
    pub fn begin(&mut self, target: Option<Target>) -> Option<u64> {
        self.cancel();
        let target = target?;
        self.serial = self.serial.checked_add(1)?;
        self.pending = Some(Pending {
            ticket: self.serial,
            target,
            phase: Phase::Waiting,
        });
        Some(self.serial)
    }
    pub fn ticket(&self) -> Option<u64> {
        self.pending.map(|p| p.ticket)
    }
    pub fn rendered(&mut self, ticket: u64) {
        if let Some(p) = self
            .pending
            .as_mut()
            .filter(|p| p.ticket == ticket && p.phase == Phase::Waiting)
        {
            p.phase = Phase::Rendered;
        }
    }
    pub fn schedule(&mut self, context: Context, decoded: bool) -> Option<u64> {
        let p = self.pending.as_mut()?;
        if decoded && p.phase == Phase::Rendered && p.target.current(context) {
            p.phase = Phase::Scheduled;
            Some(p.ticket)
        } else {
            None
        }
    }
    pub fn take(&mut self, ticket: u64, context: Context) -> Option<Target> {
        let p = self.pending.filter(|p| p.ticket == ticket)?;
        self.cancel();
        (p.phase == Phase::Scheduled && p.target.current(context)).then_some(p.target)
    }
    pub fn abandon(&mut self, ticket: u64) {
        if self.ticket() == Some(ticket) {
            self.cancel();
        }
    }
    pub fn cancel(&mut self) {
        self.pending = None;
    }
}
