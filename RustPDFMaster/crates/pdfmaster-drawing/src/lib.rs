use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}
pub const MAX_INPUT_POINTS: usize = 4096;
pub const MAX_PATH_POINTS: usize = 32768;
/// Midpoint quadratic smoothing, flattened before both preview and PDF InkList.
/// Method follows PDF Master's SwiftyDraw-derived path; see LICENSE for credit.
pub fn smooth(points: &[Point]) -> Result<Vec<Point>, &'static str> {
    if !(2..=MAX_INPUT_POINTS).contains(&points.len())
        || points.iter().any(|p| !p.x.is_finite() || !p.y.is_finite())
    {
        return Err("invalid or oversized ink stroke");
    }
    fn mid(a: Point, b: Point) -> Point {
        Point {
            x: a.x * 0.5 + b.x * 0.5,
            y: a.y * 0.5 + b.y * 0.5,
        }
    }
    fn flatten(
        out: &mut Vec<Point>,
        a: Point,
        c: Point,
        b: Point,
        depth: u8,
    ) -> Result<(), &'static str> {
        let m = mid(a, b);
        if (c.x - m.x).hypot(c.y - m.y) <= 0.15 || depth == 16 {
            if out.len() >= MAX_PATH_POINTS {
                return Err("flattened ink path exceeds budget");
            }
            out.push(b);
            return Ok(());
        }
        let l = mid(a, c);
        let r = mid(c, b);
        let center = mid(l, r);
        flatten(out, a, l, center, depth + 1)?;
        flatten(out, center, r, b, depth + 1)
    }
    let mut out = vec![points[0]];
    let mut current = points[0];
    for i in 0..points.len() {
        let end = if i + 1 < points.len() {
            mid(points[i], points[i + 1])
        } else {
            points[i]
        };
        flatten(&mut out, current, points[i], end, 0)?;
        current = end;
    }
    Ok(out)
}
pub fn page_point(
    u: f32,
    v: f32,
    w: f32,
    h: f32,
    rotation: i32,
    margin: f32,
) -> Result<Point, &'static str> {
    if [u, v, w, h, margin].iter().any(|v| !v.is_finite())
        || w <= 0.
        || h <= 0.
        || margin < 0.
        || rotation % 90 != 0
    {
        return Err("invalid page mapping");
    }
    let (u, v) = (u.clamp(0., 1.), v.clamp(0., 1.));
    let (x, y) = match rotation.rem_euclid(360) {
        0 => (u * w, (1. - v) * h),
        90 => (v * w, u * h),
        180 => ((1. - u) * w, v * h),
        _ => ((1. - v) * w, (1. - u) * h),
    };
    let m = margin.min(w.min(h) * 0.5);
    Ok(Point {
        x: x.clamp(m, w - m),
        y: y.clamp(m, h - m),
    })
}
pub fn display_point(p: Point, w: f32, h: f32, rotation: i32) -> Point {
    match rotation.rem_euclid(360) {
        0 => Point { x: p.x, y: h - p.y },
        90 => Point { x: p.y, y: p.x },
        180 => Point { x: w - p.x, y: p.y },
        _ => Point {
            x: h - p.y,
            y: w - p.x,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn smoothing_finishes_endpoints_flattens_curves_and_stays_in_convex_bounds() {
        let p = [
            Point { x: 20., y: 20. },
            Point { x: 100., y: 100. },
            Point { x: 180., y: 20. },
        ];
        let out = smooth(&p).unwrap();
        assert_eq!(out[0], p[0]);
        assert_eq!(*out.last().unwrap(), p[2]);
        assert!(out.len() > 3);
        assert!(
            out.iter()
                .all(|p| p.x >= 20. && p.x <= 180. && p.y >= 20. && p.y < 100.)
        );
        assert!(
            out.iter()
                .any(|p| (p.x - 100.).abs() < 0.1 && (p.y - 80.).abs() < 0.1)
        );
    }
    #[test]
    fn quarter_turn_coordinate_mapping_is_inverse_and_clamps_half_stroke_at_edges() {
        for rotation in [0, 90, 180, 270, -90] {
            for (u, v) in [(0., 0.), (1., 0.), (0., 1.), (1., 1.), (0.2, 0.7)] {
                let p = page_point(u, v, 200., 120., rotation, 0.).unwrap();
                let d = display_point(p, 200., 120., rotation);
                let (w, h) = if rotation.rem_euclid(180) == 0 {
                    (200., 120.)
                } else {
                    (120., 200.)
                };
                assert!((d.x / w - u).abs() < 0.0001);
                assert!((d.y / h - v).abs() < 0.0001);
                let c = page_point(u * 10., v * 10., 200., 120., rotation, 6.).unwrap();
                assert!(c.x >= 6. && c.y >= 6. && c.x <= 194. && c.y <= 114.);
            }
        }
        assert!(page_point(0., 0., 0., 120., 0, 0.).is_err());
        assert!(page_point(f32::NAN, 0., 200., 120., 0, 0.).is_err());
        assert!(page_point(0., 0., 200., 120., 45, 0.).is_err());
    }
    #[test]
    fn invalid_inputs_and_path_work_are_bounded() {
        assert!(smooth(&[]).is_err());
        assert!(smooth(&[Point { x: 0., y: 0. }]).is_err());
        assert!(smooth(&[Point { x: f32::NAN, y: 0. }, Point { x: 0., y: 0. }]).is_err());
        assert!(smooth(&vec![Point { x: 1., y: 1. }; MAX_INPUT_POINTS + 1]).is_err());
        let p = (0..MAX_INPUT_POINTS)
            .map(|i| Point {
                x: i as f32,
                y: if i % 2 == 0 { 0. } else { 20000. },
            })
            .collect::<Vec<_>>();
        assert!(smooth(&p).is_err());
    }
}
