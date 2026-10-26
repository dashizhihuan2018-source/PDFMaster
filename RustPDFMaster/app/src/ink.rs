use pdfmaster_drawing::Point;
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}
#[derive(Clone, Debug)]
pub struct InkGesture {
    pub request: String,
    pub pointer: i32,
    pub page: usize,
    pub page_width: f32,
    pub page_height: f32,
    pub rotation: i32,
    pub frame: Frame,
    pub points: Vec<Point>,
    pub color: serde_json::Value,
    pub stroke: String,
    pub width: f32,
    pub guard: Option<u32>,
    pub ended: bool,
    pub committing: bool,
    pub cancelled: bool,
    pub begin_pending: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub enum InkAction {
    Wait,
    Commit { id: u32, points: Vec<Point> },
    Cancel { id: u32 },
    CancelRequest { request: String },
}
impl InkGesture {
    #[allow(clippy::too_many_arguments)]
    pub fn begin(
        pointer: i32,
        page: usize,
        w: f32,
        h: f32,
        rotation: i32,
        frame: Frame,
        x: f64,
        y: f64,
        stroke: &str,
        width: &str,
    ) -> Result<Self, String> {
        let style = crate::shape_parameters(
            page,
            w,
            h,
            "line",
            &format!("0,0,{w},{h}"),
            stroke,
            "",
            width,
        )?;
        let width = style["line_width"].as_f64().ok_or("线宽无效。")? as f32;
        let color = style["stroke"].clone();
        let stroke = crate::shape_color_hex(&color).ok_or("颜色无效。")?;
        let mut s = Self {
            request: String::new(),
            pointer,
            page,
            page_width: w,
            page_height: h,
            rotation,
            frame,
            points: Vec::new(),
            color,
            stroke,
            width,
            guard: None,
            ended: false,
            committing: false,
            cancelled: false,
            begin_pending: true,
        };
        s.points.push(s.map(frame, x, y)?);
        Ok(s)
    }
    fn map(&self, frame: Frame, x: f64, y: f64) -> Result<Point, String> {
        if [frame.left, frame.top, frame.width, frame.height, x, y]
            .iter()
            .any(|v| !v.is_finite())
            || frame.width <= 0.
            || frame.height <= 0.
        {
            return Err("页面尚未就绪，未添加手绘。".into());
        }
        if [
            (frame.left - self.frame.left).abs(),
            (frame.top - self.frame.top).abs(),
            (frame.width - self.frame.width).abs(),
            (frame.height - self.frame.height).abs(),
        ]
        .iter()
        .any(|v| *v > 0.5)
        {
            return Err("页面位置或缩放已改变，本笔已取消，请重新绘制。".into());
        }
        pdfmaster_drawing::page_point(
            ((x - frame.left) / frame.width) as f32,
            ((y - frame.top) / frame.height) as f32,
            self.page_width,
            self.page_height,
            self.rotation,
            self.width * 0.5,
        )
        .map_err(|e| e.to_string())
    }
    pub fn sample(&mut self, pointer: i32, frame: Frame, x: f64, y: f64) -> Result<bool, String> {
        if pointer != self.pointer || self.ended || self.committing {
            return Ok(false);
        }
        let p = self.map(frame, x, y)?;
        if self.points.last() == Some(&p) {
            return Ok(false);
        }
        if self.points.len() >= pdfmaster_drawing::MAX_INPUT_POINTS {
            return Err("本笔采样点过多，已取消；请分成较短笔画。".into());
        }
        self.points.push(p);
        Ok(true)
    }
    pub fn preview(&self) -> String {
        if !self.moved() {
            return String::new();
        }
        pdfmaster_drawing::smooth(&self.points)
            .map(|points| {
                points
                    .into_iter()
                    .map(|p| {
                        let d = pdfmaster_drawing::display_point(
                            p,
                            self.page_width,
                            self.page_height,
                            self.rotation,
                        );
                        format!("{},{}", d.x, d.y)
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default()
    }
    pub fn moved(&self) -> bool {
        self.points
            .first()
            .is_some_and(|first| self.points.iter().any(|p| p != first))
    }
    pub fn acknowledge(&mut self, id: u32) -> InkAction {
        self.begin_pending = false;
        self.guard = Some(id);
        self.retry()
    }
    pub fn finish(&mut self, pointer: i32) -> InkAction {
        if pointer != self.pointer {
            return InkAction::Wait;
        }
        self.ended = true;
        self.retry()
    }
    pub fn cancel(&mut self) -> InkAction {
        self.cancelled = true;
        self.ended = true;
        self.retry()
    }
    pub fn retry(&mut self) -> InkAction {
        if self.committing || !self.ended {
            return InkAction::Wait;
        }
        let Some(id) = self.guard else {
            return InkAction::Wait;
        };
        self.committing = true;
        if self.cancelled || !self.moved() {
            InkAction::Cancel { id }
        } else {
            InkAction::Commit {
                id,
                points: self.points.clone(),
            }
        }
    }
}
#[derive(Clone, Debug)]
pub struct InkImageHandoff {
    pub previous: String,
    pub next: String,
    pub revision: u32,
    pub decoded: bool,
}
impl InkImageHandoff {
    pub fn decoded(&mut self, revision: u32) -> Option<String> {
        if self.revision != revision {
            return None;
        }
        self.decoded = true;
        Some(self.next.clone())
    }
    pub fn loaded(&self, revision: u32, source: &str) -> bool {
        self.revision == revision && self.decoded && self.next == source
    }
    pub fn failed(&self, revision: u32) -> Option<String> {
        (self.revision == revision).then(|| self.previous.clone())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_handoff_stages_decode_preserves_old_image_on_failure_and_ignores_stale_events() {
        let mut h = InkImageHandoff {
            previous: "old".into(),
            next: "new".into(),
            revision: 9,
            decoded: false,
        };
        assert!(!h.loaded(9, "new"));
        assert!(h.decoded(8).is_none());
        assert_eq!(h.failed(9), Some("old".into()));
        assert!(h.failed(8).is_none());
        assert_eq!(h.decoded(9), Some("new".into()));
        assert!(!h.loaded(9, "old"));
        assert!(!h.loaded(8, "new"));
        assert!(h.loaded(9, "new"));
    }
    fn frame() -> Frame {
        Frame {
            left: 25.,
            top: 40.,
            width: 400.,
            height: 240.,
        }
    }
    #[test]
    fn pointer_capture_uses_css_page_coordinates_not_retina_pixels_and_owns_one_pointer() {
        let mut s =
            InkGesture::begin(7, 2, 200., 120., 0, frame(), 65., 80., "#FF0000", "4").unwrap();
        assert_eq!(s.points, vec![Point { x: 20., y: 100. }]);
        assert!(!s.moved());
        assert!(!s.sample(8, frame(), 225., 160.).unwrap());
        assert!(s.sample(7, frame(), 225., 160.).unwrap());
        assert!(s.moved());
        assert_eq!(s.points[1], Point { x: 100., y: 60. });
        assert!(!s.preview().is_empty());
        assert!(
            s.sample(
                7,
                Frame {
                    left: 26.,
                    ..frame()
                },
                225.,
                160.
            )
            .is_err()
        );
    }
    #[test]
    fn rotated_touch_is_clamped_to_page_and_duplicate_samples_do_not_draw_a_dot() {
        let mut s =
            InkGesture::begin(1, 0, 200., 120., 90, frame(), 25., 40., "#0000FF", "12").unwrap();
        assert_eq!(s.points[0], Point { x: 6., y: 6. });
        s.sample(1, frame(), 25., 40.).unwrap();
        assert!(!s.moved());
        s.sample(1, frame(), 10000., 10000.).unwrap();
        assert_eq!(*s.points.last().unwrap(), Point { x: 194., y: 114. });
        assert!(s.moved());
        assert!(InkGesture::begin(1, 0, 200., 120., 0, frame(), 10., 10., "bad", "12").is_err());
    }
    #[test]
    fn release_before_native_ack_commits_once_and_failure_can_retry_frozen_input() {
        let mut s =
            InkGesture::begin(7, 2, 200., 120., 0, frame(), 65., 80., "#FF0000", "4").unwrap();
        s.sample(7, frame(), 225., 160.).unwrap();
        assert_eq!(s.finish(8), InkAction::Wait);
        assert!(!s.ended);
        assert_eq!(s.finish(7), InkAction::Wait);
        assert!(s.ended);
        let expected = InkAction::Commit {
            id: 42,
            points: s.points.clone(),
        };
        assert_eq!(s.acknowledge(42), expected);
        assert!(s.committing);
        assert_eq!(s.finish(7), InkAction::Wait);
        assert_eq!(s.acknowledge(42), InkAction::Wait);
        s.committing = false;
        assert_eq!(s.retry(), expected);
    }
    #[test]
    fn cancellation_before_ack_and_stationary_tap_release_the_native_guard_without_a_dot() {
        let mut s =
            InkGesture::begin(1, 0, 200., 120., 0, frame(), 65., 80., "#0000FF", "2").unwrap();
        assert_eq!(s.cancel(), InkAction::Wait);
        assert!(s.cancelled);
        assert_eq!(s.acknowledge(8), InkAction::Cancel { id: 8 });
        assert_eq!(s.cancel(), InkAction::Wait);
        let mut tap =
            InkGesture::begin(2, 0, 200., 120., 0, frame(), 65., 80., "#0000FF", "2").unwrap();
        assert_eq!(tap.acknowledge(9), InkAction::Wait);
        assert_eq!(tap.finish(2), InkAction::Cancel { id: 9 });
    }
    #[test]
    fn workspace_wiring_uses_pointer_capture_native_guard_and_image_load_handoff() {
        // Source contract supplements, not replaces, compiled/runtime checks.
        let s = include_str!("workspace.rs");
        for token in [
            "on:pointerdown",
            "on:pointermove",
            "on:pointerup",
            "on:pointercancel",
            "on:lostpointercapture",
            "set_pointer_capture",
            "release_pointer_capture",
            "begin_ink_document",
            "commit_ink_document",
            "cancel_ink_document",
            "ink_handoff",
            "on:load",
            "重试提交手绘",
            "取消／同步手绘",
            "on:error",
            "InkImageHandoff",
            "ink_image_failed",
            "cancel_ink_request_document",
            "call_bounded",
        ] {
            assert!(s.contains(token), "missing {token}");
        }
        assert!(
            include_str!("../styles.css").contains(".drawing-surface.inking{touch-action:none")
        );
        assert!(include_str!("bridge.rs").contains("clear_timeout_with_handle"));
        assert!(
            s.contains("s.begin_pending"),
            "cancel must retain a capture until begin acknowledgement, even on repeated cancel"
        );
    }
}
