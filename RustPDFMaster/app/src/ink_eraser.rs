use crate::canvas_notes::{Frame, click_page_point, is_click};
use pdfmaster_drawing::Point;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq)]
pub struct Context {
    pub document_id: u32,
    pub page: usize,
    pub ticket: Option<u64>,
    pub source: String,
    pub frame: Frame,
    pub page_width: f32,
    pub page_height: f32,
    pub rotation: i32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Capability {
    pub id: u32,
    pub document_id: u32,
    pub page: usize,
}
impl Capability {
    pub fn matches(&self, context: &Context) -> bool {
        self.id != 0 && self.document_id == context.document_id && self.page == context.page
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Arguments {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub tolerance_x: f32,
    pub tolerance_y: f32,
}
impl Context {
    pub fn arguments(&self, id: u32, client: Point) -> Result<Arguments, &'static str> {
        let point = click_page_point(
            client,
            self.frame,
            self.page_width,
            self.page_height,
            self.rotation,
        )?;
        let (display_x, display_y) = if self.rotation.rem_euclid(180) == 0 {
            (self.frame.width, self.frame.height)
        } else {
            (self.frame.height, self.frame.width)
        };
        let tolerance_x = 4. * self.page_width / display_x;
        let tolerance_y = 4. * self.page_height / display_y;
        if id == 0
            || !tolerance_x.is_finite()
            || !tolerance_y.is_finite()
            || tolerance_x > 200.
            || tolerance_y > 200.
        {
            return Err("橡皮点击或图片尺寸无效。");
        }
        Ok(Arguments {
            id,
            x: point.x,
            y: point.y,
            tolerance_x,
            tolerance_y,
        })
    }
}
#[derive(Clone, Debug)]
pub struct Pointer {
    pub id: i32,
    pub context: Context,
    start: Point,
    invalid: bool,
}
impl Pointer {
    pub fn begin(
        id: i32,
        primary: bool,
        button: i16,
        start: Point,
        context: Context,
    ) -> Option<Self> {
        if !primary || button != 0 || context.arguments(1, start).is_err() {
            return None;
        }
        Some(Self {
            id,
            context,
            start,
            invalid: false,
        })
    }
    pub fn sample(&mut self, id: i32, point: Point) {
        if self.id == id {
            self.invalid |= !is_click(self.start, point);
        }
    }
    pub fn cancel(&mut self, id: i32) {
        if self.id == id {
            self.invalid = true;
        }
    }
    pub fn accepts(&self, id: i32, end: Point, current: &Context) -> bool {
        !self.invalid
            && self.id == id
            && self.context == *current
            && is_click(self.start, end)
            && current.arguments(1, end).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas_notes::Frame;
    use pdfmaster_drawing::Point;

    fn context() -> Context {
        Context {
            document_id: 7,
            page: 1,
            ticket: Some(23),
            source: "decoded".into(),
            frame: Frame {
                left: 10.,
                top: 20.,
                width: 400.,
                height: 100.,
            },
            page_width: 200.,
            page_height: 100.,
            rotation: 0,
        }
    }
    #[test]
    fn ink_eraser_maps_all_rotations_and_four_css_pixel_axes() {
        for (rotation, expected, tx, ty) in [
            (0, (50., 75.), 2., 4.),
            (90, (50., 25.), 8., 1.),
            (180, (150., 25.), 2., 4.),
            (270, (150., 75.), 8., 1.),
        ] {
            let mut c = context();
            c.rotation = rotation;
            let a = c.arguments(9, Point { x: 110., y: 45. }).unwrap();
            assert_eq!((a.x, a.y), expected);
            assert_eq!((a.tolerance_x, a.tolerance_y), (tx, ty));
            assert_eq!(
                serde_json::to_value(a).unwrap(),
                serde_json::json!({"id":9,"x":expected.0,"y":expected.1,"tolerance_x":tx,"tolerance_y":ty})
            );
        }
        assert!(context().arguments(1, Point { x: 9., y: 30. }).is_err());
        assert!(
            context()
                .arguments(
                    1,
                    Point {
                        x: f32::NAN,
                        y: 30.
                    }
                )
                .is_err()
        );
    }
    #[test]
    fn ink_eraser_click_rejects_pointer_cancel_drag_and_every_stale_context() {
        let c = context();
        let start = Point { x: 110., y: 45. };
        assert!(Pointer::begin(1, false, 0, start, c.clone()).is_none());
        assert!(Pointer::begin(1, true, 1, start, c.clone()).is_none());
        let p = Pointer::begin(1, true, 0, start, c.clone()).unwrap();
        assert!(p.accepts(1, start, &c));
        assert!(!p.accepts(2, start, &c));
        assert!(!p.accepts(1, Point { x: 115., y: 45. }, &c));
        let mut moved = p.clone();
        moved.sample(1, Point { x: 120., y: 45. });
        assert!(!moved.accepts(1, start, &c));
        let mut cancelled = p.clone();
        cancelled.cancel(1);
        assert!(!cancelled.accepts(1, start, &c));
        for i in 0..9 {
            let mut stale = c.clone();
            match i {
                0 => stale.document_id += 1,
                1 => stale.page += 1,
                2 => stale.ticket = Some(24),
                3 => stale.source.push('!'),
                4 => stale.frame.left += 1.,
                5 => stale.frame.width += 1.,
                6 => stale.rotation = 90,
                7 => stale.page_width += 1.,
                _ => stale.page_height += 1.,
            }
            assert!(!p.accepts(1, start, &stale), "stale {i}");
        }
        assert!(
            Capability {
                id: 1,
                document_id: 7,
                page: 1
            }
            .matches(&c)
        );
        assert!(
            !Capability {
                id: 0,
                document_id: 7,
                page: 1
            }
            .matches(&c)
        );
        assert!(
            !Capability {
                id: 1,
                document_id: 8,
                page: 1
            }
            .matches(&c)
        );
        assert!(
            !Capability {
                id: 1,
                document_id: 7,
                page: 2
            }
            .matches(&c)
        );
    }
    #[test]
    fn ink_eraser_is_real_exclusive_available_action_with_honest_status() {
        use crate::workbench::*;
        for mode in [Mode::Annotation, Mode::Drawing] {
            assert_eq!(
                tools(mode)
                    .iter()
                    .find(|t| t.id == "eraser")
                    .unwrap()
                    .action,
                Some(Action::Activate(ActiveTool::Eraser))
            );
            let c = Chrome::default()
                .select_mode(mode, false)
                .activate_tool(ActiveTool::Ink, false)
                .activate_tool(ActiveTool::Eraser, false);
            assert_eq!(c.active_tool, Some(ActiveTool::Eraser));
            assert_eq!(c.panel, None);
            assert_eq!(c.mode, Some(mode));
            assert_eq!(c.activate_tool(ActiveTool::Eraser, false).active_tool, None);
        }
        assert_eq!(
            crate::command_status_message("erase_ink_document", false, None),
            "已擦除整条手绘笔迹，可撤销。"
        );
        assert_eq!(
            crate::command_status_message("erase_ink_document", false, Some("warning")),
            "warning"
        );
    }
    #[test]
    fn ink_eraser_genuine_pointer_uses_prepare_then_ordinary_document_command() {
        let workspace = include_str!("workspace.rs");
        for hook in [
            "eraser_pointer_down",
            "eraser_pointer_move",
            "eraser_pointer_up",
            "eraser_pointer_cancel",
        ] {
            assert!(workspace.contains(hook), "missing connected {hook}");
        }
        let controller = include_str!("ink_eraser_view.rs");
        for hook in [
            "decoded_page_image",
            "get_bounding_client_rect",
            "annotation_image_ticket",
            "\"prepare_ink_eraser\"",
            "self.command_at_page",
            "\"erase_ink_document\"",
        ] {
            assert!(controller.contains(hook), "missing real lifecycle {hook}");
        }
        assert!(!controller.contains("replace_document_view"));
    }
}
