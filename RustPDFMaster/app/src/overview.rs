use std::{collections::BTreeSet, ops::Range};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    pages: BTreeSet<usize>,
    anchor: Option<usize>,
    lead: Option<usize>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Move {
    pub pages: Vec<usize>,
    pub insertion: usize,
    pub selected: Range<usize>,
}
impl Selection {
    pub fn current(&self) -> Option<usize> {
        self.lead
    }
    pub fn pages(&self) -> Vec<usize> {
        self.pages.iter().copied().collect()
    }
    pub fn contains(&self, page: usize) -> bool {
        self.pages.contains(&page)
    }
    pub fn reset(&mut self, current: usize, count: usize) {
        self.pages.clear();
        self.anchor = (count > 0).then_some(current.min(count.saturating_sub(1)));
        self.lead = self.anchor;
        if let Some(page) = self.anchor {
            self.pages.insert(page);
        }
    }
    pub fn restore(&mut self, pages: &[usize], count: usize) {
        self.pages = pages.iter().copied().filter(|&page| page < count).collect();
        self.anchor = self.pages.last().copied();
        self.lead = self.anchor;
    }
    pub fn click(&mut self, page: usize, count: usize, shift: bool, toggle: bool) {
        if page >= count {
            return;
        }
        self.lead = Some(page);
        if shift && let Some(anchor) = self.anchor.filter(|&anchor| anchor < count) {
            if !toggle {
                self.pages.clear();
            }
            self.pages.extend(anchor.min(page)..=anchor.max(page));
        } else {
            if !toggle {
                self.pages.clear();
            }
            if !self.pages.insert(page) {
                self.pages.remove(&page);
            }
            self.anchor = Some(page);
        }
    }
    pub fn movement(&self, insertion: usize, count: usize) -> Option<Move> {
        let first = *self.pages.first()?;
        let last = *self.pages.last()?;
        if insertion > count
            || last >= count
            || (last - first + 1 == self.pages.len() && (first..=last + 1).contains(&insertion))
        {
            return None;
        }
        let start = insertion - self.pages.range(..insertion).count();
        Some(Move {
            pages: self.pages(),
            insertion,
            selected: start..start + self.pages.len(),
        })
    }
}
pub fn scale(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.5, 2.)
    } else {
        1.
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tile {
    pub page: usize,
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    pub columns: usize,
    pub height: f64,
    pub tiles: Vec<Tile>,
    rows: Vec<(Range<usize>, f64, f64)>,
}
impl Layout {
    pub fn new(width: f64, value: f64, pages: &[(f64, f64, i32)]) -> Self {
        let width = if width.is_finite() {
            width.clamp(1., 20000.)
        } else {
            1.
        };
        let available = (width - 40.).max(1.);
        let columns = (((available + 16.) / (180. * scale(value) + 16.)).round() as usize).max(1);
        let tile_width = ((available - (columns - 1) as f64 * 16.) / columns as f64).max(72.);
        let mut layout = Self {
            columns,
            height: 40.,
            ..Default::default()
        };
        let mut top = 20.;
        for (row, chunk) in pages.chunks(columns).enumerate() {
            let start = layout.tiles.len();
            let mut row_height: f64 = 0.;
            for (column, &(w, h, rotation)) in chunk.iter().enumerate() {
                let aspect = if w.is_finite() && h.is_finite() && w > 0. && h > 0. {
                    if rotation.rem_euclid(180) == 90 {
                        w / h
                    } else {
                        h / w
                    }
                } else {
                    1.414
                };
                let height = tile_width * aspect.clamp(0.1, 10.) + 28.;
                row_height = row_height.max(height);
                layout.tiles.push(Tile {
                    page: row * columns + column,
                    left: 20. + column as f64 * (tile_width + 16.),
                    top,
                    width: tile_width,
                    height,
                });
            }
            layout
                .rows
                .push((start..layout.tiles.len(), top, top + row_height));
            top += row_height + 20.;
        }
        if !layout.tiles.is_empty() {
            layout.height = top;
        }
        layout
    }
    pub fn visible(&self, scroll: f64, height: f64) -> Range<usize> {
        if self.rows.is_empty() {
            return 0..0;
        }
        let height = if height.is_finite() && height > 0. {
            height
        } else {
            600.
        };
        let scroll = if scroll.is_finite() {
            scroll.max(0.).min((self.height - height).max(0.))
        } else {
            0.
        };
        let start = self
            .rows
            .partition_point(|(_, _, bottom)| *bottom < scroll)
            .saturating_sub(1);
        let end = (self
            .rows
            .partition_point(|(_, top, _)| *top <= scroll + height)
            + 1)
        .min(self.rows.len());
        self.rows[start].0.start..self.rows[end.saturating_sub(1).max(start)].0.end
    }
    pub fn insertion(&self, x: f64, y: f64) -> Option<usize> {
        if !x.is_finite() || !y.is_finite() || x < 0. || y < 0. {
            return None;
        }
        for (range, top, bottom) in &self.rows {
            if y >= *top && y <= *bottom {
                if let Some(tile) = self.tiles[range.clone()].iter().find(|tile| {
                    x >= tile.left && x <= tile.left + tile.width && y <= tile.top + tile.height
                }) {
                    return Some(tile.page + usize::from(x >= tile.left + tile.width / 2.));
                }
                return Some(self.tiles.len());
            }
        }
        Some(self.tiles.len())
    }
}
