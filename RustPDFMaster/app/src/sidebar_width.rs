pub const DEFAULT: f64 = 176.;
pub const MIN: f64 = 152.;
pub const MAX: f64 = 480.;
pub const READER_MIN: f64 = 200.;
pub const STORAGE_KEY: &str = "pdfmaster.reader-sidebar-width.v1";

#[derive(Clone, Debug)]
struct Drag {
    pointer: i32,
    x: f64,
    width: f64,
    preferred: f64,
}
#[derive(Clone, Debug)]
pub struct Width {
    preferred: f64,
    drag: Option<Drag>,
}
impl Default for Width {
    fn default() -> Self {
        Self::restore(None)
    }
}
impl Width {
    pub fn restore(saved: Option<&str>) -> Self {
        let preferred = saved
            .filter(|s| s.len() <= 32)
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|n| n.is_finite() && *n > 0.)
            .map(|n| n.clamp(MIN, MAX))
            .unwrap_or(DEFAULT);
        Self {
            preferred,
            drag: None,
        }
    }
    pub fn preferred(&self) -> f64 {
        self.preferred
    }
    pub fn effective(&self, viewport: f64) -> f64 {
        let (min, max) = Self::bounds(viewport);
        self.preferred.clamp(min, max)
    }
    pub fn bounds(viewport: f64) -> (f64, f64) {
        let max = if viewport.is_finite() && viewport >= 0. {
            (viewport - READER_MIN).clamp(0., MAX)
        } else {
            MIN
        };
        (MIN.min(max), max)
    }
    pub fn begin(&mut self, pointer: i32, x: f64, viewport: f64) -> bool {
        if self.drag.is_some() || !x.is_finite() {
            return false;
        }
        self.drag = Some(Drag {
            pointer,
            x,
            width: self.effective(viewport),
            preferred: self.preferred,
        });
        true
    }
    pub fn drag(&mut self, pointer: i32, x: f64, viewport: f64) -> bool {
        let Some(drag) = &self.drag else {
            return false;
        };
        if drag.pointer != pointer || !x.is_finite() {
            return false;
        }
        let (min, max) = Self::bounds(viewport);
        self.preferred = (drag.width + x - drag.x).clamp(min, max).clamp(MIN, MAX);
        true
    }
    pub fn finish(&mut self, pointer: i32) -> bool {
        if self.drag.as_ref().is_none_or(|d| d.pointer != pointer) {
            return false;
        }
        self.drag = None;
        true
    }
    pub fn cancel(&mut self, pointer: i32) -> bool {
        if self.drag.as_ref().is_none_or(|d| d.pointer != pointer) {
            return false;
        }
        self.cancel_active();
        true
    }
    pub fn cancel_active(&mut self) {
        if let Some(drag) = self.drag.take() {
            self.preferred = drag.preferred;
        }
    }
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }
    pub fn key(&mut self, key: &str, viewport: f64) -> bool {
        if self.drag.is_some() {
            return false;
        }
        let (min, max) = Self::bounds(viewport);
        let width = self.effective(viewport);
        let next = match key {
            "ArrowLeft" => width - 10.,
            "ArrowRight" => width + 10.,
            "Home" => min,
            "End" => max,
            _ => return false,
        };
        self.preferred = next.clamp(min, max).clamp(MIN, MAX);
        true
    }
    pub fn saved(&self) -> String {
        self.preferred.to_string()
    }
}
