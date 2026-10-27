use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Page,
    Newest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Identity(pub (u32, u16), pub Option<(u32, u16)>, pub usize);

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
pub struct Bounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Row {
    pub page_index: usize,
    pub annotation_index: usize,
    pub page_id: (u32, u16),
    pub object_id: Option<(u32, u16)>,
    pub subtype: String,
    pub contents: String,
    pub color: Option<Color>,
    pub bounds: Option<Bounds>,
    pub modified: Option<String>,
    // Filled by the actual browser date parser. Floating PDF dates use the
    // local timezone at that date, not today's offset (which may have DST).
    #[serde(skip)]
    pub modified_millis: Option<i64>,
}

impl Row {
    pub fn identity(&self) -> Identity {
        Identity(self.page_id, self.object_id, self.annotation_index)
    }
    pub fn heading(&self) -> String {
        let title = match self.subtype.as_str() {
            "Highlight" => "高亮",
            "Underline" => "下划线",
            "StrikeOut" => "删除线",
            "Ink" => "手绘",
            "FreeText" => "文字",
            "Text" => "便笺",
            "Square" => "矩形",
            "Circle" => "椭圆",
            "Line" => "直线 / 箭头",
            other => other,
        };
        format!("第 {} 页 · {title}", self.page_index + 1)
    }
    pub fn detail(&self) -> String {
        self.contents
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }
    pub fn height(&self) -> usize {
        if self.contents.trim().is_empty() {
            44
        } else {
            60
        }
    }
    pub fn color_css(&self) -> String {
        self.color
            .filter(|c| {
                [c.r, c.g, c.b]
                    .iter()
                    .all(|v| v.is_finite() && (0. ..=1.).contains(v))
            })
            .map(|c| {
                format!(
                    "rgb({},{},{})",
                    (c.r * 255.).round() as u8,
                    (c.g * 255.).round() as u8,
                    (c.b * 255.).round() as u8
                )
            })
            .unwrap_or_else(|| "transparent".into())
    }
}

pub fn sort_rows(rows: &mut [Row], sort: Sort) {
    rows.sort_by(|a, b| {
        let date = if sort == Sort::Newest {
            b.modified_millis.cmp(&a.modified_millis)
        } else {
            std::cmp::Ordering::Equal
        };
        date.then_with(|| a.page_index.cmp(&b.page_index))
            .then_with(|| a.annotation_index.cmp(&b.annotation_index))
    });
}

pub fn summary(rows: &[Row]) -> String {
    rows.iter()
        .map(|r| {
            if r.contents.is_empty() {
                r.heading()
            } else {
                format!("{}\n{}", r.heading(), r.contents)
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn copy_status_is_current(
    started_generation: Option<u64>,
    current_generation: Option<u64>,
    started_message: &str,
    current_message: &str,
) -> bool {
    started_generation.is_some()
        && started_generation == current_generation
        && started_message == current_message
}
