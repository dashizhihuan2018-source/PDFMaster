pub fn adjacent_page(current: usize, delta: i32, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    current.saturating_add_signed(delta as isize).min(count - 1)
}

/// A page move has a destination only for an existing adjacent page. Return
/// its new index as well so selection follows the moved page, not its neighbor.
pub fn page_move_edit(
    current: usize,
    delta: i32,
    count: usize,
) -> Option<(serde_json::Value, usize)> {
    if current >= count || !matches!(delta, -1 | 1) {
        return None;
    }
    let target = current.checked_add_signed(delta as isize)?;
    (target < count).then(|| {
        (
            serde_json::json!({"kind":"move_page","from":current,"to":target}),
            target,
        )
    })
}
/// Unicode note contents remain PDF metadata, not embedded page-text fonts.
pub fn note_edit(
    page: usize,
    width: f32,
    height: f32,
    rotation: i32,
    contents: &str,
) -> Result<serde_json::Value, String> {
    if contents.trim().is_empty() {
        return Err("请先输入便笺内容。".into());
    }
    if contents.len() > 64 * 1024 {
        return Err("便笺内容不能超过 64 KiB。".into());
    }
    if !width.is_finite()
        || !height.is_finite()
        || width <= 0.
        || height <= 0.
        || width > 20_000.
        || height > 20_000.
    {
        return Err("页面尺寸无效，未添加便笺。".into());
    }
    if rotation.rem_euclid(360) != 0 {
        return Err("旋转页面的便笺定位仍在迁移，请先恢复 0°。".into());
    }
    let icon_width = width.min(24.);
    let icon_height = height.min(24.);
    let x = (width - icon_width).min(36.);
    let y = (height - icon_height - 36.).max(0.);
    Ok(
        serde_json::json!({"kind":"add_note","page":page,"bounds":{"x":x,"y":y,"width":icon_width,"height":icon_height},"contents":contents,"color":{"r":1.,"g":0.85,"b":0.2}}),
    )
}

/// Preserve the exact note snapshot so the core can reject stale annotation edits.
pub fn update_note_edit(
    page: usize,
    annotation_index: usize,
    expected_contents: &str,
    contents: &str,
) -> Result<serde_json::Value, String> {
    if contents.trim().is_empty() {
        return Err("便笺内容不能为空。".into());
    }
    if contents.len() > 64 * 1024 || expected_contents.len() > 1024 * 1024 {
        return Err("便笺内容超过编辑预算，请缩短内容。".into());
    }
    if contents == expected_contents {
        return Err("便笺内容未改变；可取消编辑。".into());
    }
    Ok(
        serde_json::json!({"kind":"update_note","page":page,"annotation_index":annotation_index,"expected_contents":expected_contents,"contents":contents}),
    )
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

pub fn page_jump_index(input: &str, page_count: usize) -> Result<usize, String> {
    let page = input
        .trim()
        .parse::<usize>()
        .map_err(|_| "页码无效，请输入现有页面。".to_string())?;
    if page == 0 || page > page_count {
        Err("页码无效，请输入现有页面。".into())
    } else {
        Ok(page - 1)
    }
}

const PAGE_JUMP_ERROR: &str = "页码无效，请输入现有页面。";

pub fn page_jump_success_message(message: &str) -> String {
    if message == PAGE_JUMP_ERROR {
        String::new()
    } else {
        message.to_owned()
    }
}

pub fn page_batch_args(range: &str, operation: &str) -> serde_json::Value {
    serde_json::json!({
        "range": range,
        "operation": operation,
    })
}

pub fn search_status_message(query: &str, count: usize, truncated: bool) -> String {
    if count == 0 {
        if truncated {
            "搜索已截断，尚未扫描全部文字；请细化关键词。".into()
        } else {
            format!("未找到“{query}”")
        }
    } else if truncated {
        format!("找到 {count} 项 · 结果已截断，请细化关键词")
    } else {
        format!("找到 {count} 项")
    }
}

pub fn search_conditions_message(message: &str, previous_summary: Option<&str>) -> String {
    if previous_summary == Some(message) {
        String::new()
    } else {
        message.to_owned()
    }
}

pub fn search_response_is_current(
    requested_query: &str,
    requested_case_sensitive: bool,
    response_query: &str,
    response_case_sensitive: bool,
) -> bool {
    requested_query.trim() == response_query && requested_case_sensitive == response_case_sensitive
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
    fn note_update_preserves_snapshot_identity_and_unicode_without_trimming() {
        let edit = update_note_edit(3, 9, " 原文\n🦀 ", " 新文\n<script>x</script> ").unwrap();
        assert_eq!(
            edit,
            serde_json::json!({"kind":"update_note","page":3,"annotation_index":9,"expected_contents":" 原文\n🦀 ","contents":" 新文\n<script>x</script> "})
        );
        assert!(update_note_edit(0, 0, "", "new").is_ok());
    }
    #[test]
    fn note_update_checks_utf8_budgets_and_rejects_empty_or_unchanged_input() {
        for bad in ["", " \n\t", "old"] {
            assert!(update_note_edit(0, 0, "old", bad).is_err());
        }
        assert!(update_note_edit(0, 0, "old", &"a".repeat(65536)).is_ok());
        assert!(update_note_edit(0, 0, "old", &"a".repeat(65537)).is_err());
        assert!(update_note_edit(0, 0, "old", &"🦀".repeat(16384)).is_ok());
        assert!(update_note_edit(0, 0, "old", &"🦀".repeat(16385)).is_err());
        assert!(update_note_edit(0, 0, &"a".repeat(1024 * 1024), "new").is_ok());
        assert!(update_note_edit(0, 0, &"a".repeat(1024 * 1024 + 1), "new").is_err());
    }
    #[test]
    fn note_editor_exposes_explicit_save_cancel_and_blocks_conflicting_commands() {
        let source = include_str!("workspace.rs");
        assert!(source.contains("m.save_note()"));
        assert!(source.contains("m.cancel_note_edit()"));
        assert!(source.contains("m.begin_note_edit(note.clone())"));
        assert!(source.contains("self.has_note_draft()"));
        assert!(source.contains("aria-label=\"编辑便笺内容\""));
        assert!(!source.contains("inner_html"));
    }
    #[test]
    fn note_editor_acquires_native_guard_before_exposing_input_and_releases_by_id() {
        // Source-wiring regression only; native unit and GUI tests verify behavior.
        let source = include_str!("workspace.rs");
        let compact: String = source.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(compact.contains("bridge::call::<NoteDraft>(\"begin_note_editor\""));
        assert!(compact.contains("bridge::call::<Document>(\"cancel_note_editor\""));
        assert!(compact.contains("bridge::call::<Document>(\"commit_note_editor\""));
        assert!(compact.contains("\"id\":draft.id"));
        assert!(source.contains("note_editor: Option<NoteDraft>"));
        assert!(source.contains("self.note_draft.set(doc.note_editor.clone())"));
    }
    #[test]
    fn lost_editor_ack_has_an_explicit_sync_route_without_replacing_local_input() {
        // UI wiring only: the native view carries the active editor and its ID.
        // Hide this route while local input exists so syncing cannot discard it.
        let source = include_str!("workspace.rs");
        let compact: String = source.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(compact.contains("<Showwhen=move||m.loaded()&&!m.has_note_draft()>"));
        assert!(compact.contains("on:click=move|_|m.command(\"document_info\",json!({}))"));
        assert!(source.contains("同步便笺编辑状态"));
    }
    #[test]
    fn note_creation_supports_unicode_and_adapts_to_small_pages_without_clipping() {
        let edit = note_edit(2, 595., 842., 0, " 中文便笺 🦀 ").unwrap();
        assert_eq!(edit["kind"], "add_note");
        assert_eq!(edit["contents"], " 中文便笺 🦀 ");
        assert_eq!(edit["page"], 2);
        let tiny = note_edit(0, 8., 9., 0, "x").unwrap();
        assert_eq!(tiny["bounds"]["width"], 8.);
        assert_eq!(tiny["bounds"]["height"], 9.);
        assert_eq!(tiny["bounds"]["x"], 0.);
        assert_eq!(tiny["bounds"]["y"], 0.);
        for bad in ["", " \n\t"] {
            assert!(note_edit(0, 100., 100., 0, bad).is_err());
        }
        assert!(note_edit(0, 100., 100., 90, "x").is_err());
        assert!(note_edit(0, f32::NAN, 100., 0, "x").is_err());
        assert!(note_edit(0, 100., -1., 0, "x").is_err());
        assert!(note_edit(0, 100., 100., 0, &"x".repeat(65537)).is_err());
    }
    #[test]
    fn note_sidebar_is_rust_rendered_text_and_refreshes_from_current_session() {
        let source = include_str!("workspace.rs");
        assert!(source.contains("\"notes_document\""));
        assert!(source.contains("self.notes.set(Some("));
        assert!(source.contains("m.add_note()"));
        assert!(!source.contains("inner_html"));
    }
    #[test]
    fn page_move_is_adjacent_bounded_and_keeps_selected_page_identity() {
        assert_eq!(
            page_move_edit(1, -1, 3),
            Some((serde_json::json!({"kind":"move_page","from":1,"to":0}), 0))
        );
        assert_eq!(
            page_move_edit(1, 1, 3),
            Some((serde_json::json!({"kind":"move_page","from":1,"to":2}), 2))
        );
        for (page, delta, count) in [
            (0, -1, 3),
            (2, 1, 3),
            (0, 1, 0),
            (3, -1, 3),
            (0, 2, 3),
            (0, 0, 3),
            (usize::MAX, 1, usize::MAX),
        ] {
            assert_eq!(page_move_edit(page, delta, count), None);
        }
    }
    #[test]
    fn current_page_delete_uses_the_same_native_confirmation_as_batch_delete() {
        let source = include_str!("workspace.rs");
        assert!(source.contains("m.delete_current_page()"));
        assert!(
            source.contains(
                "self.batch_range((self.page.get_untracked() + 1).to_string(), \"delete\")"
            )
        );
        assert!(!source.contains("\"kind\":\"delete_page\""));
    }
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
    #[test]
    fn page_jump_is_strictly_one_based_and_never_clamps() {
        assert_eq!(page_jump_index(" 2 ", 3), Ok(1));
        for invalid in ["", "0", "4", "-1", "1.5", "999999999999999999999"] {
            assert!(page_jump_index(invalid, 3).is_err(), "{invalid}");
        }
        assert!(page_jump_index("1", 0).is_err());
    }
    #[test]
    fn successful_page_jump_clears_only_the_obsolete_page_error() {
        assert_eq!(page_jump_success_message("页码无效，请输入现有页面。"), "");
        assert_eq!(
            page_jump_success_message("本地草稿写入失败。"),
            "本地草稿写入失败。"
        );
    }

    #[test]
    fn changing_search_conditions_clears_only_the_previous_search_summary() {
        assert_eq!(
            search_conditions_message("找到 2 项", Some("找到 2 项")),
            ""
        );
        assert_eq!(
            search_conditions_message("文档渲染失败", Some("找到 2 项")),
            "文档渲染失败"
        );
        assert_eq!(
            search_conditions_message("本地草稿写入失败。", None),
            "本地草稿写入失败。"
        );
    }

    #[test]
    fn mobile_footer_reserves_a_full_row_for_status_and_wraps_controls() {
        let css = include_str!("../styles.css");
        assert!(css.contains(".statusbar{flex-wrap:wrap"));
        assert!(css.contains(".statusbar [role=status]{flex:1 0 100%"));
        assert!(css.contains(".statusbar>.flex{display:none}"));
    }
    #[test]
    fn mobile_editable_fields_do_not_trigger_small_font_focus_zoom() {
        let css = include_str!("../styles.css");
        assert!(css.contains(".tool-group>input{width:100%"));
        assert!(css.contains(".workspace .tool-group>input,.workspace .page-jump-label input{font-size:16px;min-height:44px}"));
    }
    #[test]
    fn bounded_search_status_discloses_truncation() {
        assert_eq!(search_status_message("测试", 0, false), "未找到“测试”");
        assert!(search_status_message("a", 1000, true).contains("结果已截断"));
        let partial = search_status_message("needle", 0, true);
        assert!(partial.contains("尚未扫描全部文字"));
        assert!(!partial.contains("未找到"));
    }
    #[test]
    fn workspace_exposes_search_results_and_direct_page_jump_controls() {
        let source = include_str!("workspace.rs");
        assert!(source.contains("search_document"));
        assert!(source.contains("搜索当前文档"));
        assert!(source.contains("m.navigate(target)"));
        assert!(source.contains("跳转页码"));
        assert!(source.contains("page_jump_index"));
        assert!(source.contains("<input aria-label=\"搜索文档文字\" disabled=move||m.busy.get()"));
        assert!(source.contains("<input type=\"checkbox\" disabled=move||m.busy.get()"));
        assert!(source.contains(
            "m.search_case_sensitive.set(event_target_checked(&e));m.invalidate_search()"
        ));
        assert!(source.contains("\"case_sensitive\":case_sensitive"));
    }
    #[test]
    fn workspace_exposes_native_page_batch_with_busy_and_confirmation_contracts() {
        let source = include_str!("workspace.rs");
        assert!(source.contains("batch_pages"));
        assert!(!source.contains("confirm_with_message"));
        assert!(source.contains("aria-label=\"批量页码范围\" disabled=move||m.busy.get()"));
        assert!(source.contains("批量顺时针旋转 90°"));
        assert!(source.contains("批量删除"));
        assert_eq!(
            page_batch_args("1-3", "delete"),
            serde_json::json!({
                "range": "1-3",
                "operation": "delete",
            })
        );
    }
    #[test]
    fn stale_search_response_identity_is_rejected() {
        assert!(search_response_is_current(" 测试 ", false, "测试", false));
        assert!(!search_response_is_current("新", false, "旧", false));
        assert!(!search_response_is_current("same", true, "same", false));
    }
}
