//! Analytic paths matching the current saved shape producer.
use crate::canvas_shapes::{Draft, Geometry, LineEnding, Page};
use pdfmaster_drawing::Point;
use std::fmt::Write;
#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    pub d: String,
    pub fill: bool,
    pub dash_pattern: Vec<f32>,
    pub round_join: bool,
}
fn vertex(d: &mut String, command: &str, p: Point) {
    if !d.is_empty() {
        d.push(' ');
    }
    write!(d, "{command} {} {}", p.x, p.y).expect("writing to String");
}
fn polygon(points: &[Point], closed: bool) -> String {
    let mut d = String::new();
    for (i, p) in points.iter().enumerate() {
        vertex(&mut d, if i == 0 { "M" } else { "L" }, *p);
    }
    if closed {
        d.push_str(" Z");
    }
    d
}
fn ellipse(x: f32, y: f32, rx: f32, ry: f32) -> String {
    let kx = rx * 0.552_284_8;
    let ky = ry * 0.552_284_8;
    let mut d = String::new();
    vertex(&mut d, "M", Point { x: x + rx, y });
    for v in [
        [x + rx, y + ky, x + kx, y + ry, x, y + ry],
        [x - kx, y + ry, x - rx, y + ky, x - rx, y],
        [x - rx, y - ky, x - kx, y - ry, x, y - ry],
        [x + kx, y - ry, x + rx, y - ky, x + rx, y],
    ] {
        write!(
            d,
            " C {} {} {} {} {} {}",
            v[0], v[1], v[2], v[3], v[4], v[5]
        )
        .expect("writing to String");
    }
    d.push_str(" Z");
    d
}
fn ending(tip: Point, dx: f32, dy: f32, kind: LineEnding, w: f32, filled: bool) -> Option<Path> {
    let p = |a: f32, b: f32| Point {
        x: tip.x + a * dx - b * dy,
        y: tip.y + a * dy + b * dx,
    };
    let (d, closed) = match kind {
        LineEnding::None => return None,
        LineEnding::OpenArrow
        | LineEnding::ClosedArrow
        | LineEnding::ROpenArrow
        | LineEnding::RClosedArrow => {
            let sign = if matches!(kind, LineEnding::ROpenArrow | LineEnding::RClosedArrow) {
                1.
            } else {
                -1.
            };
            let closed = matches!(kind, LineEnding::ClosedArrow | LineEnding::RClosedArrow);
            (
                polygon(
                    &[p(sign * 4. * w, 2. * w), tip, p(sign * 4. * w, -2. * w)],
                    closed,
                ),
                closed,
            )
        }
        LineEnding::Square => (
            polygon(
                &[
                    p(-2. * w, -2. * w),
                    p(2. * w, -2. * w),
                    p(2. * w, 2. * w),
                    p(-2. * w, 2. * w),
                ],
                true,
            ),
            true,
        ),
        LineEnding::Diamond => (
            polygon(
                &[p(-3. * w, 0.), p(0., 2. * w), p(3. * w, 0.), p(0., -2. * w)],
                true,
            ),
            true,
        ),
        LineEnding::Circle => (ellipse(tip.x, tip.y, 2. * w, 2. * w), true),
        LineEnding::Butt => (polygon(&[p(0., 2. * w), p(0., -2. * w)], false), false),
        LineEnding::Slash => (
            polygon(&[p(w, 1.732_050_8 * w), p(-w, -1.732_050_8 * w)], false),
            false,
        ),
    };
    Some(Path {
        d,
        fill: closed && filled,
        dash_pattern: vec![],
        round_join: true,
    })
}
pub fn paths(draft: &Draft, page: Page) -> Result<Vec<Path>, String> {
    draft.validate(page)?;
    let mut paths = vec![];
    match &draft.geometry {
        Geometry::Rectangle { bounds } | Geometry::Ellipse { bounds } => {
            let inset = draft.line_width / 2.;
            let (x, y) = (bounds.x + inset, bounds.y + inset);
            let (w, h) = (
                bounds.width - draft.line_width,
                bounds.height - draft.line_width,
            );
            let d = if matches!(draft.geometry, Geometry::Rectangle { .. }) {
                polygon(
                    &[
                        Point { x, y },
                        Point { x: x + w, y },
                        Point { x: x + w, y: y + h },
                        Point { x, y: y + h },
                    ],
                    true,
                )
            } else {
                ellipse(x + w / 2., y + h / 2., w / 2., h / 2.)
            };
            paths.push(Path {
                d,
                fill: draft.fill.is_some(),
                dash_pattern: vec![],
                round_join: false,
            });
        }
        Geometry::Line {
            start,
            end,
            endings,
            dash_pattern,
            ..
        } => {
            paths.push(Path {
                d: polygon(&[*start, *end], false),
                fill: false,
                dash_pattern: dash_pattern.clone(),
                round_join: true,
            });
            let length = (end.x - start.x).hypot(end.y - start.y);
            if !length.is_finite() || length == 0. {
                return Err("line preview direction is not finite".into());
            }
            let dx = (end.x - start.x) / length;
            let dy = (end.y - start.y) / length;
            if let Some(p) = ending(
                *start,
                -dx,
                -dy,
                endings[0],
                draft.line_width,
                draft.fill.is_some(),
            ) {
                paths.push(p);
            }
            if let Some(p) = ending(
                *end,
                dx,
                dy,
                endings[1],
                draft.line_width,
                draft.fill.is_some(),
            ) {
                paths.push(p);
            }
        }
    }
    Ok(paths)
}
