pub fn adjacent_page(current: usize, delta: i32, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    current.saturating_add_signed(delta as isize).min(count - 1)
}
pub mod ink;

#[allow(clippy::too_many_arguments)]
pub fn line_edit(
    page: usize,
    width: f32,
    height: f32,
    points: &str,
    stroke: &str,
    fill: &str,
    line: &str,
    mode: &str,
) -> Result<serde_json::Value, String> {
    let endings = match mode {
        "line" => ["none", "none"],
        "arrow" => ["none", "open_arrow"],
        "double_arrow" => ["open_arrow", "open_arrow"],
        _ => return Err("直线类型无效。".into()),
    };
    if points.len() > 256 {
        return Err("端点输入过长。".into());
    }
    let values = points
        .split(',')
        .map(|v| v.trim().parse::<f32>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "端点须为 x1,y1,x2,y2。")?;
    let [x1, y1, x2, y2] = values.as_slice() else {
        return Err("端点需要四个逗号分隔的数值。".into());
    };
    if values.iter().any(|v| !v.is_finite())
        || *x1 < 0.
        || *x2 < 0.
        || *y1 < 0.
        || *y2 < 0.
        || *x1 > width
        || *x2 > width
        || *y1 > height
        || *y2 > height
        || (x1 == x2 && y1 == y2)
    {
        return Err("端点须不同且位于页面内。".into());
    }
    // Reuse page/style admission only; creation always sends actual endpoints,
    // never a bounding rectangle interpreted as a line direction.
    let mut edit = shape_parameters(
        page,
        width,
        height,
        "line",
        &format!("0,0,{width},{height}"),
        stroke,
        fill,
        line,
    )?;
    edit["kind"] = serde_json::json!("add_line");
    edit["start"] = serde_json::json!({"x":x1,"y":y1});
    edit["end"] = serde_json::json!({"x":x2,"y":y2});
    edit["endings"] = serde_json::json!(endings);
    let object = edit.as_object_mut().expect("shape parameters object");
    object.remove("shape");
    object.remove("bounds");
    Ok(edit)
}

#[allow(clippy::too_many_arguments)]
pub fn shape_edit(
    page: usize,
    width: f32,
    height: f32,
    shape: &str,
    bounds: &str,
    stroke: &str,
    fill: &str,
    line: &str,
) -> Result<serde_json::Value, String> {
    if !matches!(shape, "rectangle" | "ellipse") {
        return Err("添加直线须指定独立端点。".into());
    }
    shape_parameters(page, width, height, shape, bounds, stroke, fill, line)
}

#[allow(clippy::too_many_arguments)]
fn shape_parameters(
    page: usize,
    width: f32,
    height: f32,
    shape: &str,
    bounds: &str,
    stroke: &str,
    fill: &str,
    line: &str,
) -> Result<serde_json::Value, String> {
    if !matches!(shape, "rectangle" | "ellipse" | "line")
        || !width.is_finite()
        || !height.is_finite()
        || width <= 0.
        || height <= 0.
        || width > 20_000.
        || height > 20_000.
    {
        return Err("图形类型或页面尺寸无效。".into());
    }
    if [bounds, stroke, fill, line]
        .iter()
        .any(|value| value.len() > 256)
    {
        return Err("图形输入过长，未修改文档。".into());
    }
    let values = bounds
        .split(',')
        .map(|value| value.trim().parse::<f32>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "图形范围须为 x,y,宽,高，单位 pt。")?;
    let [x, y, w, h] = values.as_slice() else {
        return Err("图形范围需要四个逗号分隔的数值。".into());
    };
    if values.iter().any(|value| !value.is_finite())
        || *x < 0.
        || *y < 0.
        || *w <= 0.
        || *h <= 0.
        || x + w > width
        || y + h > height
    {
        return Err("图形须完整位于页面内，宽高必须为正数。".into());
    }
    let line = line.trim().parse::<f32>().map_err(|_| "请输入有效线宽。")?;
    if shape == "line" {
        if !line.is_finite() || !(0.25..=32.).contains(&line) {
            return Err("直线线宽须在 0.25–32 pt。".into());
        }
    } else if !line.is_finite() || line <= 0. || line >= w.min(*h) {
        return Err("线宽必须为正数且小于图形的宽和高。".into());
    }
    let parse_color = |value: &str| -> Result<serde_json::Value, String> {
        let value = value.trim();
        let hex = value
            .strip_prefix('#')
            .filter(|value| value.len() == 6 && value.bytes().all(|v| v.is_ascii_hexdigit()))
            .ok_or("颜色须为 #RRGGBB，填充留空表示透明。")?;
        let value = u32::from_str_radix(hex, 16).map_err(|_| "颜色无效。")?;
        Ok(
            serde_json::json!({"r":((value>>16)&255) as f32/255.,"g":((value>>8)&255) as f32/255.,"b":(value&255) as f32/255.}),
        )
    };
    let stroke = parse_color(stroke)?;
    let fill = if fill.trim().is_empty() {
        serde_json::Value::Null
    } else {
        parse_color(fill)?
    };
    Ok(
        serde_json::json!({"kind":"add_shape","page":page,"shape":shape,"bounds":{"x":x,"y":y,"width":w,"height":h},"stroke":stroke,"fill":fill,"line_width":line}),
    )
}

pub fn shape_color_hex(color: &serde_json::Value) -> Option<String> {
    let channel = |key| {
        color[key]
            .as_f64()
            .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
            .map(|v| (v * 255.).round() as u8)
    };
    Some(format!(
        "#{:02X}{:02X}{:02X}",
        channel("r")?,
        channel("g")?,
        channel("b")?
    ))
}

#[allow(clippy::too_many_arguments)]
pub fn update_shape_edit(
    page: usize,
    width: f32,
    height: f32,
    expected: &serde_json::Value,
    bounds: &str,
    stroke: &str,
    fill: &str,
    line: &str,
) -> Result<serde_json::Value, String> {
    let shape = expected["shape"].as_str().ok_or("选中图形类型无效。")?;
    if shape == "line" && !expected["line"].is_object() {
        return Err("选中直线缺少端点快照。".into());
    }
    let mut edit = shape_parameters(page, width, height, shape, bounds, stroke, fill, line)?;
    // The hex form is a display/editor convenience, not permission to round
    // imported RGB floats when the user has not changed a color field.
    if shape_color_hex(&expected["stroke"]).is_some_and(|c| c.eq_ignore_ascii_case(stroke.trim())) {
        edit["stroke"] = expected["stroke"].clone();
    }
    if shape_color_hex(&expected["fill"]).is_some_and(|c| c.eq_ignore_ascii_case(fill.trim())) {
        edit["fill"] = expected["fill"].clone();
    }
    edit["kind"] = serde_json::json!("update_shape");
    edit["expected"] = expected.clone();
    edit.as_object_mut()
        .expect("shape edit object")
        .remove("shape");
    Ok(edit)
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
    } else if command == "extract_pages_document" {
        if restored {
            "页面已提取并回读验证；恢复的原文草稿仍未保存，请另行导出完整副本。".into()
        } else {
            "页面已提取并回读验证；原文与未保存状态保持不变。".into()
        }
    } else if command == "insert_pdfs_document" {
        "所选 PDF 已按文件面板返回顺序插入，可整批一次撤销；请导出完整副本保存修改。".into()
    } else if command == "insert_pdf_document" {
        "PDF 页面已插入，可一次撤销；请导出完整副本保存修改。".into()
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
    #[test]
    fn line_request_uses_explicit_endpoints_and_validates_edge_geometry() {
        for mode in ["line", "arrow", "double_arrow"] {
            let edit = super::line_edit(2, 200., 120., "200,0,200,120", "#FF0000", "", "32", mode)
                .unwrap();
            assert_eq!(edit["kind"], "add_line");
            assert_eq!(edit["page"], 2);
            assert_eq!(edit["start"], serde_json::json!({"x":200.,"y":0.}));
            assert_eq!(edit["end"], serde_json::json!({"x":200.,"y":120.}));
            assert_eq!(
                edit["endings"],
                match mode {
                    "arrow" => serde_json::json!(["none", "open_arrow"]),
                    "double_arrow" => serde_json::json!(["open_arrow", "open_arrow"]),
                    _ => serde_json::json!(["none", "none"]),
                }
            );
        }
        for points in [
            "0,0,0,0",
            "-1,2,10,20",
            "0,0,201,20",
            "NaN,0,10,10",
            "0,0,10",
            "0,0,10,inf",
        ] {
            assert!(super::line_edit(0, 200., 120., points, "#000000", "", "1", "line").is_err());
        }
        for width in ["0", "0.1", "32.1", "NaN", "inf"] {
            assert!(
                super::line_edit(0, 200., 120., "0,0,10,10", "#000000", "", width, "line").is_err()
            );
        }
        assert!(super::line_edit(0, 200., 120., "0,0,10,10", "bad", "", "1", "line").is_err());
        assert!(super::line_edit(0, 200., 120., "0,0,10,10", "#000000", "", "1", "bad").is_err());
    }

    #[test]
    fn line_update_retains_geometry_snapshot_and_allows_width_greater_than_bounds() {
        let expected = serde_json::json!({"annotation_index":0,"shape":"line","bounds":{"x":190.,"y":0.,"width":10.,"height":120.},"stroke":{"r":0.1,"g":0.4,"b":0.8},"fill":null,"line_width":32.,"line":{"start":{"x":200.,"y":0.},"end":{"x":200.,"y":120.},"endings":["diamond","closed_arrow"],"dash_pattern":[3.,2.]}});
        let edit = super::update_shape_edit(
            0,
            200.,
            120.,
            &expected,
            "190,0,10,120",
            "#1A66CC",
            "",
            "32",
        )
        .unwrap();
        assert_eq!(edit["expected"], expected);
        assert_eq!(edit["stroke"], expected["stroke"]);
        assert_eq!(edit["kind"], "update_shape");
        assert!(
            super::shape_edit(0, 200., 120., "line", "190,0,10,120", "#000000", "", "2").is_err()
        );
    }

    #[test]
    fn line_workspace_exposes_separate_endpoints_and_three_creation_modes() {
        let source = include_str!("workspace.rs");
        assert!(source.contains("line_edit("));
        assert!(source.contains("直线端点 x1,y1,x2,y2"));
        for mode in ["line", "arrow", "double_arrow"] {
            assert!(source.contains(&format!("m.add_line(\"{mode}\")")));
        }
        assert!(source.contains("端点样式与虚线"));
        assert!(!source.contains("拖拽和线条／手绘仍在迁移"));
    }
    #[test]
    fn shape_update_preserves_unmodified_imported_colors_and_snapshot() {
        let expected = serde_json::json!({"annotation_index":4,"shape":"ellipse","bounds":{"x":10.,"y":20.,"width":100.,"height":60.},"stroke":{"r":0.1,"g":0.4,"b":0.8},"fill":null,"line_width":2.});
        let edit =
            super::update_shape_edit(0, 200., 120., &expected, "20,30,110,70", "#1A66CC", "", "4")
                .unwrap();
        assert_eq!(edit["kind"], "update_shape");
        assert_eq!(edit["expected"], expected);
        assert_eq!(edit["stroke"], expected["stroke"]);
        assert_eq!(edit["fill"], serde_json::Value::Null);
        assert!(edit.get("shape").is_none());
        assert_eq!(edit["bounds"]["width"], 110.);
        assert!(
            super::update_shape_edit(0, 200., 120., &expected, "20,30,110,70", "#1A66CC", "", "0")
                .is_err()
        );
        let mut bad = expected.clone();
        bad["shape"] = serde_json::json!("line");
        assert!(
            super::update_shape_edit(0, 200., 120., &bad, "20,30,110,70", "#1A66CC", "", "4")
                .is_err()
        );
    }

    #[test]
    fn shape_lifecycle_workspace_wires_list_update_and_native_delete_and_clears_selection() {
        let source = include_str!("workspace.rs");
        assert!(source.contains("\"shapes_document\""));
        assert!(source.contains("\"delete_shape_document\""));
        assert!(source.contains("update_shape_edit("));
        assert!(source.contains("self.selected_shape.set(None)"));
        assert!(source.contains("当前页图形批注"));
        assert!(source.contains("应用图形修改"));
    }

    #[test]
    fn shape_request_uses_source_geometry_and_full_rgb_style() {
        let edit = super::shape_edit(
            3,
            200.,
            120.,
            "ellipse",
            "20, 20, 100, 60",
            "#fF0000",
            "#0000ff",
            "4",
        )
        .unwrap();
        assert_eq!(edit["kind"], "add_shape");
        assert_eq!(edit["shape"], "ellipse");
        assert_eq!(edit["page"], 3);
        assert_eq!(edit["bounds"]["width"], 100.);
        assert_eq!(edit["stroke"]["r"], 1.);
        assert_eq!(edit["fill"]["b"], 1.);
        let edit = super::shape_edit(0, 200., 120., "rectangle", "0,0,100,60", "#123456", "", "1")
            .unwrap();
        assert!(edit["fill"].is_null());
    }
    #[test]
    fn shape_request_rejects_bad_geometry_style_and_unbounded_inputs() {
        for bounds in [
            "",
            "1,2,3",
            "1,2,3,4,5",
            "NaN,0,10,10",
            "0,0,inf,10",
            "-1,0,10,10",
            "190,0,20,10",
        ] {
            assert!(
                super::shape_edit(0, 200., 120., "rectangle", bounds, "#000000", "", "1").is_err()
            );
        }
        for line in ["NaN", "inf", "0", "-1", "60"] {
            assert!(
                super::shape_edit(
                    0,
                    200.,
                    120.,
                    "rectangle",
                    "20,20,100,60",
                    "#000000",
                    "",
                    line
                )
                .is_err()
            );
        }
        for color in ["red", "#abc", "#12345678", "123456", "#zz1122"] {
            assert!(
                super::shape_edit(0, 200., 120., "rectangle", "20,20,100,60", color, "", "1")
                    .is_err()
            );
            assert!(
                super::shape_edit(
                    0,
                    200.,
                    120.,
                    "rectangle",
                    "20,20,100,60",
                    "#000000",
                    color,
                    "1"
                )
                .is_err()
            );
        }
        assert!(
            super::shape_edit(
                0,
                f32::NAN,
                120.,
                "rectangle",
                "20,20,100,60",
                "#000000",
                "",
                "1"
            )
            .is_err()
        );
        assert!(
            super::shape_edit(
                0,
                200.,
                120.,
                "triangle",
                "20,20,100,60",
                "#000000",
                "",
                "1"
            )
            .is_err()
        );
        assert!(
            super::shape_edit(
                0,
                200.,
                120.,
                "ellipse",
                &"0".repeat(1000),
                "#000000",
                "",
                "1"
            )
            .is_err()
        );
    }
    #[test]
    fn workspace_exposes_explicit_shape_inputs_and_both_shapes() {
        let source = include_str!("workspace.rs");
        for label in [
            "图形范围 x,y,宽,高",
            "图形描边颜色",
            "图形填充颜色",
            "图形线宽",
            "添加矩形批注",
            "添加椭圆批注",
        ] {
            assert!(source.contains(label), "missing {label}");
        }
        assert!(source.contains("m.add_shape(\"rectangle\")"));
        assert!(source.contains("m.add_shape(\"ellipse\")"));
        assert!(source.contains("其他类型批注未列入便笺列表"));
        assert!(!source.contains("其他类型批注尚不支持显示"));
    }
    #[test]
    fn batch_insertion_ui_preserves_single_entry_and_exposes_one_transaction_status() {
        let source = include_str!("workspace.rs");
        assert!(source.contains(
            "m.command(\"insert_pdfs_document\",json!({\"index\":m.page.get_untracked()+1}))"
        ));
        assert!(source.contains("批量插入 PDF"));
        assert!(source.contains("| \"insert_pdfs_document\""));
        assert_eq!(
            super::command_status_message("insert_pdfs_document", false, None),
            "所选 PDF 已按文件面板返回顺序插入，可整批一次撤销；请导出完整副本保存修改。"
        );
        assert_eq!(
            super::command_status_message("insert_pdfs_document", true, Some("草稿故障")),
            "草稿故障"
        );
        assert!(source.contains("在当前页后插入 PDF"));
    }
    #[test]
    fn insertion_ui_uses_current_page_native_picker_and_marks_unsaved() {
        let source = include_str!("workspace.rs");
        assert!(source.contains(
            "m.command(\"insert_pdf_document\",json!({\"index\":m.page.get_untracked()+1}))"
        ));
        assert!(source.contains("在当前页后插入 PDF"));
        assert_eq!(
            super::command_status_message("insert_pdf_document", false, None),
            "PDF 页面已插入，可一次撤销；请导出完整副本保存修改。"
        );
        assert_eq!(
            super::command_status_message("insert_pdf_document", true, Some("草稿故障")),
            "草稿故障"
        );
        assert!(source.contains("| \"insert_pdf_document\""));
    }
    #[test]
    fn extraction_ui_uses_native_range_command_and_never_claims_source_saved() {
        let source = include_str!("workspace.rs");
        assert!(source.contains(
            "m.command(\"extract_pages_document\",json!({\"range\":m.page_range.get_untracked()}))"
        ));
        assert!(source.contains("提取页面到新 PDF"));
        assert_eq!(
            super::command_status_message("extract_pages_document", false, None),
            "页面已提取并回读验证；原文与未保存状态保持不变。"
        );
        assert_eq!(
            super::command_status_message("extract_pages_document", true, None),
            "页面已提取并回读验证；恢复的原文草稿仍未保存，请另行导出完整副本。"
        );
        assert_eq!(
            super::command_status_message("extract_pages_document", true, Some("恢复警告")),
            "恢复警告"
        );
    }
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
        assert!(source.contains("m.begin_note_edit(edit_note.clone())"));
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
    fn note_delete_uses_the_native_confirmed_route_not_a_generic_edit_or_js_confirm() {
        let source = include_str!("workspace.rs");
        assert!(source.contains("m.delete_note(note.clone())"));
        assert!(source.contains("\"delete_note_document\""));
        assert!(!source.contains("confirm_with_message"));
        assert!(source.contains("删除便笺"));
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
    fn mobile_tools_use_a_single_vertical_flow_without_clipped_groups() {
        // Source contract only. Real WebKit scrolling and hit targets need simulator QA.
        let css = include_str!("../styles.css");
        let mobile = css
            .split("/* Mobile tools: one vertical scroll region. */")
            .nth(1)
            .expect("mobile tools must not remain a clipped horizontal strip");
        assert!(mobile.contains(".workspace .inspector{display:block;"));
        assert!(
            mobile.contains("max-height:36dvh;min-height:0;overflow-x:hidden;overflow-y:auto;")
        );
        assert!(mobile.contains(".workspace .tool-group{min-width:0;"));
        assert!(mobile.contains(".workspace .inspector h2,.workspace .tool-group h3,.workspace .tool-group .muted{display:block;"));
        assert!(mobile.contains(".workspace .note-list{max-height:none;overflow:visible}"));
        assert!(
            mobile.contains(
                ".workspace .tool-group button{white-space:normal;overflow-wrap:anywhere}"
            )
        );
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
