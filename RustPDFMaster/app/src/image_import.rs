use serde::{Deserialize, Serialize};
#[derive(Clone, Deserialize)]
pub struct Selection {
    pub id: u32,
    pub names: Vec<String>,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Paper {
    A4,
    Letter,
    Pixels,
}
#[derive(Clone, Serialize)]
pub struct Layout {
    pub paper: Paper,
    pub landscape: bool,
    pub margin: f64,
}
#[derive(Clone)]
pub struct Configuration {
    pub selection: Selection,
    pub order: Vec<usize>,
    pub selected: Option<usize>,
    pub paper: Paper,
    pub landscape: bool,
    pub margin: String,
    pub running: bool,
    pub cancelling: bool,
    pub error: String,
}
impl Configuration {
    pub fn new(selection: Selection) -> Result<Self, String> {
        if selection.id == 0
            || selection.names.is_empty()
            || selection.names.len() > 32
            || selection
                .names
                .iter()
                .any(|n| n.is_empty() || n.len() > 8192 || n.contains(['/', '\\']))
        {
            return Err("图片选择回复无效；未创建文档。".into());
        }
        Ok(Self {
            order: (0..selection.names.len()).collect(),
            selected: Some(0),
            selection,
            paper: Paper::A4,
            landscape: false,
            margin: "24".into(),
            running: false,
            cancelling: false,
            error: String::new(),
        })
    }
    pub fn layout(&self) -> Result<Layout, String> {
        if self.order.is_empty() {
            return Err("请至少保留一张图片。".into());
        }
        let margin = if self.paper == Paper::Pixels {
            0.
        } else {
            let margin = self
                .margin
                .trim()
                .parse::<f64>()
                .map_err(|_| "边距须为有限非负数字。")?;
            let edge = if self.paper == Paper::A4 {
                595.276
            } else {
                612.
            };
            if !margin.is_finite() || margin < 0. || margin * 2. >= edge {
                return Err("边距须非负且小于纸张短边的一半。".into());
            }
            margin
        };
        Ok(Layout {
            paper: self.paper,
            landscape: self.landscape,
            margin,
        })
    }
    pub fn shift(&mut self, delta: isize) {
        if self.running || self.cancelling || ![-1, 1].contains(&delta) {
            return;
        }
        if let Some(i) = self.selected
            && let Some(j) = i.checked_add_signed(delta)
            && i < self.order.len()
            && j < self.order.len()
        {
            self.order.swap(i, j);
            self.selected = Some(j);
        }
    }
    pub fn remove(&mut self) {
        if self.running || self.cancelling {
            return;
        }
        if let Some(i) = self.selected
            && i < self.order.len()
        {
            self.order.remove(i);
            self.selected = (!self.order.is_empty()).then(|| i.min(self.order.len() - 1));
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Configuration {
        Configuration::new(Selection {
            id: 7,
            names: vec!["a.png".into(), "b.tif".into(), "c.jpg".into()],
        })
        .unwrap()
    }
    #[test]
    fn reordered_rows_keep_native_indices_and_selection_then_remove_only_the_selected_row() {
        let mut c = config();
        c.selected = Some(1);
        c.shift(-1);
        assert_eq!(c.order, [1, 0, 2]);
        assert_eq!(c.selected, Some(0));
        c.remove();
        assert_eq!(c.order, [0, 2]);
        assert_eq!(c.selected, Some(0));
        c.running = true;
        c.shift(1);
        c.remove();
        assert_eq!(c.order, [0, 2]);
    }
    fn selection_is_bounded_and_layout_matches_original_margin_and_pixel_contract() {
        assert!(
            Configuration::new(Selection {
                id: 0,
                names: vec!["x".into()]
            })
            .is_err()
        );
        assert!(
            Configuration::new(Selection {
                id: 1,
                names: vec![]
            })
            .is_err()
        );
        let mut c = config();
        assert_eq!(c.layout().unwrap().margin, 24.);
        c.paper = Paper::Letter;
        for s in ["306", "NaN", "inf", "-1", ""] {
            c.margin = s.into();
            assert!(c.layout().is_err());
        }
        c.paper = Paper::Pixels;
        assert_eq!(c.layout().unwrap().margin, 0.);
        c.order.clear();
        assert!(c.layout().is_err());
    }
    #[test]
    fn layout_validation_has_no_hidden_dpi_or_orientation_guess() {
        selection_is_bounded_and_layout_matches_original_margin_and_pixel_contract();
    }
}
