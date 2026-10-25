pub fn adjacent_page(current: usize, delta: i32, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    current.saturating_add_signed(delta as isize).min(count - 1)
}
pub fn valid_zoom(zoom: f32) -> f32 {
    if !zoom.is_finite() {
        1.0
    } else {
        zoom.clamp(0.25, 3.0)
    }
}

pub fn command_status_message(command: &str, restored: bool, warning: Option<&str>) -> String {
    if let Some(warning) = warning {
        warning.to_owned()
    } else if restored {
        "已恢复上次未保存的本地草稿，请导出副本。".into()
    } else if command == "export_document" {
        "副本已导出并回读验证。".into()
    } else {
        String::new()
    }
}

const MIN_FIT_ZOOM: f32 = 0.01;

pub fn step_zoom(current: f32, factor: f32) -> f32 {
    if !factor.is_finite() || factor <= 0. {
        return render_zoom(current);
    }
    render_zoom(current * factor)
}

pub fn visible_status(warning: Option<&str>, message: &str, busy: bool) -> String {
    let status = if busy { "处理中…" } else { message };
    match warning {
        Some(warning) if !status.is_empty() && warning != status => format!("{warning}；{status}"),
        Some(warning) => warning.to_owned(),
        None => status.to_owned(),
    }
}

fn render_zoom(zoom: f32) -> f32 {
    if !zoom.is_finite() {
        1.0
    } else {
        zoom.clamp(MIN_FIT_ZOOM, 3.0)
    }
}

pub struct RasterPlan {
    pub css_width: f32,
    pub css_height: f32,
    pub scale: f32,
}

impl RasterPlan {
    /// Let the raster's intrinsic ratio determine height even when its width changes.
    pub fn image_style(&self) -> String {
        format!("width:{}px;height:auto", self.css_width)
    }
}

pub fn fit_width_zoom(width: f32, height: f32, rotation: i32, available: f32) -> f32 {
    let width = if rotation.rem_euclid(180) == 0 {
        width
    } else {
        height
    };
    if !available.is_finite() || available <= 0. || !width.is_finite() || width <= 0. {
        1.
    } else {
        render_zoom(available / width)
    }
}

/// Dimensions are validated PDF page points. CSS zoom and device pixel density
/// are separate, so a Retina display cannot double the document's layout size.
pub fn raster_plan(width: f32, height: f32, rotation: i32, zoom: f32, density: f32) -> RasterPlan {
    // Fit-to-width may intentionally choose a zoom below the interactive
    // control floor. Re-clamping it here would make large pages overflow.
    let zoom = render_zoom(zoom);
    let density = if density.is_finite() {
        density.clamp(1., 3.)
    } else {
        1.
    };
    let (width, height) = if rotation.rem_euclid(180) == 0 {
        (width, height)
    } else {
        (height, width)
    };
    let mut scale = zoom * density;
    let pixels = |s: f32| ((width * s).ceil() as u64, (height * s).ceil() as u64);
    let (w, h) = pixels(scale);
    if w > 65535 || h > 65535 || w.saturating_mul(h) > 40_000_000 {
        let bound = (40_000_000.0_f64 / f64::from(width) / f64::from(height))
            .sqrt()
            .min(65535. / f64::from(width))
            .min(65535. / f64::from(height));
        scale = scale.min((bound * 0.999) as f32);
    }
    RasterPlan {
        css_width: width * zoom,
        css_height: height * zoom,
        scale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zoom_out_from_fit_never_increases_zoom() {
        let fitted = fit_width_zoom(2384., 1684., 0, 350.);
        assert!(step_zoom(fitted, 0.8) < fitted);
        assert!(step_zoom(fitted, 1.25) > fitted);
        assert_eq!(step_zoom(3., 1.25), 3.);
    }
    #[test]
    fn recovery_warning_survives_render_error_and_busy_status() {
        assert_eq!(
            visible_status(Some("草稿写入失败"), "渲染失败", false),
            "草稿写入失败；渲染失败"
        );
        assert!(visible_status(Some("草稿写入失败"), "", true).contains("草稿写入失败"));
    }
    #[test]
    fn page_navigation_clamps_at_edges() {
        assert_eq!(adjacent_page(0, -1, 5), 0);
        assert_eq!(adjacent_page(3, 1, 5), 4);
        assert_eq!(adjacent_page(4, 1, 5), 4);
        assert_eq!(adjacent_page(99, -1, 0), 0);
    }
    #[test]
    fn zoom_rejects_nan_and_bounds_memory_budget() {
        assert_eq!(valid_zoom(f32::NAN), 1.0);
        assert_eq!(valid_zoom(0.01), 0.25);
        assert_eq!(valid_zoom(9.0), 3.0);
        assert_eq!(valid_zoom(1.5), 1.5);
    }
    #[test]
    fn retina_pixels_do_not_change_layout_size() {
        let plan = raster_plan(595., 842., 0, 1., 2.);
        assert_eq!(
            (plan.css_width, plan.css_height, plan.scale),
            (595., 842., 2.)
        );
        let plan = raster_plan(595., 842., 90, 1.5, 2.);
        assert_eq!(
            (plan.css_width, plan.css_height, plan.scale),
            (1263., 892.5, 3.)
        );
    }
    #[test]
    fn raster_scale_fits_budget_and_handles_invalid_density() {
        assert_eq!(raster_plan(595., 842., 0, 1., f32::NAN).scale, 1.);
        let p = raster_plan(20_000., 20_000., 0, 3., 3.);
        let w = (20_000. * p.scale).ceil() as u64;
        assert!(w * w <= 40_000_000);
        assert_eq!(raster_plan(595., 842., 0, 1., 100.).scale, 3.);
    }
    #[test]
    fn viewport_reserves_phone_notch_and_home_indicator() {
        let css = include_str!("../styles.css");
        assert!(css.contains("env(safe-area-inset-top"));
        assert!(css.contains("env(safe-area-inset-bottom"));
    }
    #[test]
    fn fit_width_uses_rotated_width_and_handles_invalid_viewport() {
        assert!((fit_width_zoom(595., 842., 0, 350.) - 350. / 595.).abs() < 0.0001);
        assert!((fit_width_zoom(595., 842., 90, 350.) - 350. / 842.).abs() < 0.0001);
        assert_eq!(fit_width_zoom(595., 842., 0, f32::NAN), 1.);
        assert_eq!(fit_width_zoom(595., 842., 0, 0.), 1.);
        assert_eq!(fit_width_zoom(595., 842., 0, 10000.), 3.);
    }
    #[test]
    fn fit_width_preserves_large_page_zoom_below_manual_minimum() {
        let zoom = fit_width_zoom(2384., 1684., 0, 350.);
        assert!((zoom - 350. / 2384.).abs() < 0.0001);
        let plan = raster_plan(2384., 1684., 0, zoom, 2.);
        assert!((plan.css_width - 350.).abs() < 0.01);
        assert!((plan.scale - zoom * 2.).abs() < 0.0001);
    }
    #[test]
    fn image_style_keeps_intrinsic_aspect_under_width_changes() {
        assert_eq!(
            raster_plan(595., 842., 90, 1., 2.).image_style(),
            "width:842px;height:auto"
        );
    }
    #[test]
    fn persistence_warning_wins_over_export_success_status() {
        assert_eq!(
            command_status_message("export_document", false, Some("本地草稿写入失败。")),
            "本地草稿写入失败。"
        );
    }
}
