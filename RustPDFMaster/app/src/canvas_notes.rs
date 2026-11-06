use pdfmaster_drawing::{Point, display_point, page_point};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

impl Frame {
    pub fn valid(self) -> bool {
        [self.left, self.top, self.width, self.height]
            .iter()
            .all(|value| value.is_finite())
            && self.width > 0.
            && self.height > 0.
    }

    pub fn changed_from(self, other: Self) -> bool {
        (self.left - other.left).abs() > 0.5
            || (self.top - other.top).abs() > 0.5
            || (self.width - other.width).abs() > 0.5
            || (self.height - other.height).abs() > 0.5
    }
}

pub fn is_click(start: Point, end: Point) -> bool {
    [start.x, start.y, end.x, end.y]
        .iter()
        .all(|value| value.is_finite())
        && (start.x - end.x).hypot(start.y - end.y) <= 4.
}

pub fn click_page_point(
    client: Point,
    frame: Frame,
    page_width: f32,
    page_height: f32,
    rotation: i32,
) -> Result<Point, &'static str> {
    if !frame.valid()
        || !client.x.is_finite()
        || !client.y.is_finite()
        || client.x < frame.left
        || client.y < frame.top
        || client.x > frame.left + frame.width
        || client.y > frame.top + frame.height
    {
        return Err("click is outside the decoded page image");
    }
    page_point(
        (client.x - frame.left) / frame.width,
        (client.y - frame.top) / frame.height,
        page_width,
        page_height,
        rotation,
        0.,
    )
}

pub fn page_anchor(
    point: Point,
    frame: Frame,
    page_width: f32,
    page_height: f32,
    rotation: i32,
) -> Result<Point, &'static str> {
    if !frame.valid()
        || !point.x.is_finite()
        || !point.y.is_finite()
        || point.x < 0.
        || point.y < 0.
        || point.x > page_width
        || point.y > page_height
    {
        return Err("invalid note anchor");
    }
    let display = display_point(point, page_width, page_height, rotation);
    let (display_width, display_height) = if rotation.rem_euclid(180) == 0 {
        (page_width, page_height)
    } else {
        (page_height, page_width)
    };
    Ok(Point {
        x: frame.left + display.x / display_width * frame.width,
        y: frame.top + display.y / display_height * frame.height,
    })
}

pub fn note_contains(bounds: [f32; 4], point: Point) -> bool {
    let [x, y, width, height] = bounds;
    [x, y, width, height, point.x, point.y]
        .iter()
        .all(|value| value.is_finite())
        && width > 0.
        && height > 0.
        && point.x >= x
        && point.x <= x + width
        && point.y >= y
        && point.y <= y + height
}

pub fn icon_bounds(point: Point, page_width: f32, page_height: f32) -> Option<[f32; 4]> {
    if [point.x, point.y, page_width, page_height]
        .iter()
        .any(|value| !value.is_finite())
        || page_width <= 0.
        || page_height <= 0.
        || point.x < 0.
        || point.y < 0.
        || point.x > page_width
        || point.y > page_height
    {
        return None;
    }
    let width = page_width.min(24.);
    let height = page_height.min(24.);
    Some([
        (point.x - width * 0.5).clamp(0., page_width - width),
        (point.y - height * 0.5).clamp(0., page_height - height),
        width,
        height,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Point, b: Point) {
        assert!((a.x - b.x).abs() < 0.001, "x: {a:?} != {b:?}");
        assert!((a.y - b.y).abs() < 0.001, "y: {a:?} != {b:?}");
    }

    #[test]
    fn maps_real_image_frame_through_shared_drawing_geometry() {
        let frame = Frame {
            left: 10.,
            top: 20.,
            width: 100.,
            height: 200.,
        };
        close(
            click_page_point(Point { x: 60., y: 120. }, frame, 595., 842., 0).unwrap(),
            Point { x: 297.5, y: 421. },
        );
        for rotation in [0, 90, 180, 270] {
            for client in [
                Point { x: 10., y: 20. },
                Point { x: 110., y: 20. },
                Point { x: 10., y: 220. },
                Point { x: 110., y: 220. },
                Point { x: 37., y: 143. },
            ] {
                let page = click_page_point(client, frame, 595., 842., rotation).unwrap();
                close(
                    page_anchor(page, frame, 595., 842., rotation).unwrap(),
                    client,
                );
            }
        }
    }

    #[test]
    fn rejects_bad_frames_outside_points_and_non_clicks() {
        let frame = Frame {
            left: 10.,
            top: 20.,
            width: 100.,
            height: 200.,
        };
        for bad in [
            Frame { width: 0., ..frame },
            Frame {
                width: -1.,
                ..frame
            },
            Frame {
                left: f32::NAN,
                ..frame
            },
        ] {
            assert!(click_page_point(Point { x: 60., y: 120. }, bad, 595., 842., 0).is_err());
        }
        for point in [
            Point { x: 9., y: 20. },
            Point { x: 111., y: 20. },
            Point {
                x: f32::NAN,
                y: 20.,
            },
        ] {
            assert!(click_page_point(point, frame, 595., 842., 0).is_err());
        }
        assert!(is_click(Point { x: 1., y: 1. }, Point { x: 5., y: 1. }));
        assert!(!is_click(Point { x: 1., y: 1. }, Point { x: 5.01, y: 1. }));
        assert!(frame.changed_from(Frame {
            left: 10.51,
            ..frame
        }));
        assert!(!frame.changed_from(Frame {
            left: 10.5,
            ..frame
        }));
    }

    #[test]
    fn contains_hits_and_small_page_icons_stay_inside() {
        assert!(note_contains(
            [10., 20., 24., 24.],
            Point { x: 34., y: 44. }
        ));
        assert!(!note_contains(
            [10., 20., 24., 24.],
            Point { x: 35., y: 44. }
        ));
        assert_eq!(
            icon_bounds(Point { x: 0., y: 0. }, 10., 8.),
            Some([0., 0., 10., 8.])
        );
        assert_eq!(
            icon_bounds(Point { x: 99., y: 99. }, 100., 100.),
            Some([76., 76., 24., 24.])
        );
        assert!(icon_bounds(Point { x: -1., y: 0. }, 100., 100.).is_none());
    }
}
