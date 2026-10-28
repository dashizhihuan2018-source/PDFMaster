use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub struct ProgressNotice {
    pub id: u32,
    pub current: usize,
    pub total: usize,
}
#[derive(Clone, Deserialize)]
pub struct Selection {
    pub id: u32,
    pub names: Vec<String>,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Paper {
    Source,
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
    pub progress: Option<ProgressNotice>,
}
#[derive(Clone)]
pub enum Job {
    Configuration(Configuration),
    Recovery {
        id: u32,
        cancelling: bool,
        error: String,
    },
}
impl Job {
    pub fn id(&self) -> u32 {
        match self {
            Self::Configuration(configuration) => configuration.selection.id,
            Self::Recovery { id, .. } => *id,
        }
    }
    pub fn cancelling(&self) -> bool {
        match self {
            Self::Configuration(configuration) => configuration.cancelling,
            Self::Recovery { cancelling, .. } => *cancelling,
        }
    }
    pub fn set_cancelling(&mut self, value: bool) {
        match self {
            Self::Configuration(configuration) => configuration.cancelling = value,
            Self::Recovery { cancelling, .. } => *cancelling = value,
        }
    }
    pub fn set_error(&mut self, value: String) {
        match self {
            Self::Configuration(configuration) => configuration.error = value,
            Self::Recovery { error, .. } => *error = value,
        }
    }
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
            paper: Paper::Source,
            landscape: false,
            margin: "0".into(),
            running: false,
            cancelling: false,
            error: String::new(),
            progress: None,
        })
    }
    pub fn layout(&self) -> Result<Layout, String> {
        if self.order.is_empty() {
            return Err("请至少保留一张图片。".into());
        }
        let margin = if matches!(self.paper, Paper::Pixels | Paper::Source) {
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
    pub fn accept_progress(&mut self, notice: ProgressNotice) -> bool {
        if !self.running
            || self.cancelling
            || !self.error.is_empty()
            || notice.id != self.selection.id
            || !(1..=32).contains(&notice.total)
            || notice.total != self.order.len()
            || notice.current == 0
            || notice.current > notice.total
            || self
                .progress
                .is_some_and(|progress| notice.current <= progress.current)
        {
            return false;
        }
        self.progress = Some(notice);
        true
    }
    pub fn progress_text(&self) -> Option<String> {
        if !self.running || self.cancelling || !self.error.is_empty() {
            return None;
        }
        self.progress.map(|progress| {
            format!(
                "正在处理第 {} 张图 ，共 {} 张...",
                progress.current, progress.total
            )
        })
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
    fn automatic_picker_layout_preserves_selection_without_paper_configuration() {
        let c = config();
        assert_eq!(c.order, [0, 1, 2]);
        let value = serde_json::to_value(c.layout().unwrap()).unwrap();
        assert_eq!(value["paper"], "source");
        assert_eq!(value["margin"], 0.);
        assert_eq!(value["landscape"], false);
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
    #[test]
    fn progress_accepts_only_current_strictly_increasing_source_notices() {
        let mut c = config();
        assert_eq!(c.progress_text(), None);
        c.running = true;
        assert!(c.accept_progress(ProgressNotice {
            id: 7,
            current: 1,
            total: 3,
        }));
        assert_eq!(
            c.progress_text().as_deref(),
            Some("正在处理第 1 张图 ，共 3 张...")
        );
        assert!(!c.accept_progress(ProgressNotice {
            id: 7,
            current: 1,
            total: 3,
        }));
        assert!(!c.accept_progress(ProgressNotice {
            id: 8,
            current: 2,
            total: 3,
        }));
        assert!(c.accept_progress(ProgressNotice {
            id: 7,
            current: 3,
            total: 3,
        }));
        assert!(c.running);
        c.cancelling = true;
        assert_eq!(c.progress_text(), None);
    }
    #[test]
    fn progress_rejects_inactive_error_and_out_of_contract_notices() {
        let mut c = config();
        let first = ProgressNotice {
            id: 7,
            current: 1,
            total: 3,
        };
        assert!(!c.accept_progress(first));
        c.running = true;
        c.error = "failure".into();
        assert!(!c.accept_progress(first));
        c.error.clear();
        for notice in [
            ProgressNotice {
                id: 7,
                current: 1,
                total: 0,
            },
            ProgressNotice {
                id: 7,
                current: 1,
                total: 33,
            },
            ProgressNotice {
                id: 7,
                current: 1,
                total: 2,
            },
            ProgressNotice {
                id: 7,
                current: 0,
                total: 3,
            },
            ProgressNotice {
                id: 7,
                current: 4,
                total: 3,
            },
        ] {
            assert!(!c.accept_progress(notice));
        }
        assert!(c.accept_progress(first));
        c.running = false;
        assert_eq!(c.progress_text(), None);
        c.running = true;
        c.error = "failure".into();
        assert_eq!(c.progress_text(), None);
    }
    fn selection_is_bounded_and_layout_matches_candidate_default_and_pixel_contract() {
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
        assert_eq!(c.layout().unwrap().margin, 0.);
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
        selection_is_bounded_and_layout_matches_candidate_default_and_pixel_contract();
    }
}
