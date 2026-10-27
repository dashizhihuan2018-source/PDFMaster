use std::collections::HashSet;
use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use flate2::read::ZlibDecoder;
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_interpret::InterpreterWarning;
use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_syntax::Pdf;
use hayro::kurbo::Affine;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::vello_cpu::{Pixmap, RasterizerSettings, RenderContext, Resources, TargetInit};
use hayro::{PixmapSettings, RenderCache, RenderSettings, render, render_into};
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

use crate::document::{
    PdfMasterError, Result, Session, inherited_optional, page_object_ids, validate_page_index,
};

const MAX_RENDER_PIXELS: u64 = 40_000_000;
const MAX_RENDER_CONTENT_BYTES: usize = 8 * 1024 * 1024;
const MAX_SOURCE_RESOURCE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SOURCE_IMAGE_PIXELS: u64 = 40_000_000;
const MAX_RESOURCE_OBJECTS: usize = 4_096;
const MAX_RESOURCE_DEPTH: usize = 64;

impl Session {
    pub fn render_png(&self, page: usize, scale: f32) -> Result<Vec<u8>> {
        self.render_pixmap(page, scale)?
            .into_png()
            .map_err(|error| PdfMasterError::Render(error.to_string()))
    }

    pub(crate) fn render_pixmap(
        &self,
        page: usize,
        scale: f32,
    ) -> Result<hayro::vello_cpu::Pixmap> {
        self.render_pixmap_scaled(page, scale, scale, None)
    }

    pub(crate) fn preflight_render_page(&self, page: usize) -> Result<()> {
        validate_page_index(page, self.pages.len())?;
        let page_ids = page_object_ids(&self.document)?;
        let page_id = *page_ids.get(page).ok_or_else(|| {
            PdfMasterError::Render("core did not expose the requested page".into())
        })?;
        preflight_render(&self.document, page_id)
    }

    pub(crate) fn export_render_dimensions(&self, page: usize, dpi: f64) -> Result<(u32, u32)> {
        validate_page_index(page, self.pages.len())?;
        let info = &self.pages[page];
        let (point_width, point_height) = if matches!(info.rotation, 90 | 270) {
            (f64::from(info.height), f64::from(info.width))
        } else {
            (f64::from(info.width), f64::from(info.height))
        };
        let width = (point_width * dpi / 72.0).ceil();
        let height = (point_height * dpi / 72.0).ceil();
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return Err(PdfMasterError::InvalidGeometry(
                "image export dimensions must be finite and positive",
            ));
        }
        if width > f64::from(u16::MAX) || height > f64::from(u16::MAX) {
            return Err(PdfMasterError::Budget("render side exceeds u16"));
        }
        Ok((width as u32, height as u32))
    }

    pub(crate) fn render_pixmap_exact(
        &self,
        page: usize,
        width: u32,
        height: u32,
        dpi: f64,
    ) -> Result<hayro::vello_cpu::Pixmap> {
        validate_page_index(page, self.pages.len())?;
        let scale = (dpi / 72.0) as f32;
        let pixmap = self.render_pixmap_scaled(page, scale, scale, Some((width, height)))?;
        if u32::from(pixmap.width()) != width || u32::from(pixmap.height()) != height {
            return Err(PdfMasterError::Render(
                "renderer output dimensions did not match export preflight".into(),
            ));
        }
        Ok(pixmap)
    }

    fn render_pixmap_scaled(
        &self,
        page: usize,
        x_scale: f32,
        y_scale: f32,
        target_dimensions: Option<(u32, u32)>,
    ) -> Result<hayro::vello_cpu::Pixmap> {
        validate_page_index(page, self.pages.len())?;
        if !x_scale.is_finite()
            || !y_scale.is_finite()
            || x_scale <= 0.0
            || y_scale <= 0.0
            || x_scale > 16.0
            || y_scale > 16.0
        {
            return Err(PdfMasterError::InvalidGeometry(
                "render scale must be finite, positive, and at most 16",
            ));
        }
        let page_ids = page_object_ids(&self.document)?;
        let page_id = *page_ids.get(page).ok_or_else(|| {
            PdfMasterError::Render("core did not expose the requested page".into())
        })?;
        preflight_render(&self.document, page_id)?;
        let pdf = Pdf::new(self.current_bytes.clone())
            .map_err(|error| PdfMasterError::Render(format!("hayro load failed: {error:?}")))?;
        if pdf.pages().len() != self.pages.len() {
            return Err(PdfMasterError::Render(format!(
                "parser page count mismatch: core={}, renderer={}",
                self.pages.len(),
                pdf.pages().len()
            )));
        }
        let rendered_page = pdf.pages().get(page).ok_or_else(|| {
            PdfMasterError::Render("renderer did not expose the requested page".into())
        })?;
        let (width, height) = rendered_page.render_dimensions();
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return Err(PdfMasterError::Render(
                "renderer reported invalid page dimensions".into(),
            ));
        }
        let (pixel_width, pixel_height) = target_dimensions.map_or_else(
            || ((width * x_scale) as u64, (height * y_scale) as u64),
            |(width, height)| (u64::from(width), u64::from(height)),
        );
        if pixel_width == 0 || pixel_height == 0 {
            return Err(PdfMasterError::Budget("render side is zero"));
        }
        if pixel_width > u16::MAX as u64 || pixel_height > u16::MAX as u64 {
            return Err(PdfMasterError::Budget("render side exceeds u16"));
        }
        if pixel_width.saturating_mul(pixel_height) > MAX_RENDER_PIXELS {
            return Err(PdfMasterError::Budget("render total pixels"));
        }

        let warning_bits = Arc::new(AtomicU8::new(0));
        let warning_capture = Arc::clone(&warning_bits);
        let interpreter_settings = InterpreterSettings {
            warning_sink: Arc::new(move |warning| {
                let bit = interpreter_warning_bit(warning);
                warning_capture.fetch_or(bit, Ordering::Relaxed);
            }),
            ..Default::default()
        };
        let cache = RenderCache::new();
        let render_settings = RenderSettings::default();
        let rendered = if let Some((target_width, target_height)) = target_dimensions {
            let target_width = u16::try_from(target_width)
                .map_err(|_| PdfMasterError::Budget("render side exceeds u16"))?;
            let target_height = u16::try_from(target_height)
                .map_err(|_| PdfMasterError::Budget("render side exceeds u16"))?;
            let mut context = RenderContext::new(target_width, target_height);
            let transform = Affine::scale_non_uniform(x_scale as f64, y_scale as f64)
                * rendered_page.initial_transform(true).to_kurbo();
            render_into(
                rendered_page,
                &cache,
                &interpreter_settings,
                &render_settings,
                &mut context,
                transform,
            );
            context.flush();
            let mut pixmap = Pixmap::new(target_width, target_height);
            context.render_with(
                &mut pixmap,
                &mut Resources::default(),
                RasterizerSettings {
                    target_init: TargetInit::Clear(WHITE),
                    ..Default::default()
                },
            );
            pixmap
        } else {
            render(
                rendered_page,
                &cache,
                &interpreter_settings,
                &render_settings,
                &PixmapSettings {
                    x_scale,
                    y_scale,
                    bg_color: WHITE,
                },
            )
        };
        let warnings = warning_bits.load(Ordering::Relaxed);
        if warnings != 0 {
            return Err(PdfMasterError::Render(format!(
                "material interpreter warning(s):{}{}{}{}",
                if warnings & 1 != 0 {
                    " unsupported-font"
                } else {
                    ""
                },
                if warnings & 2 != 0 {
                    " image-decode-failure"
                } else {
                    ""
                },
                if warnings & 4 != 0 {
                    " unresolved-annotation-appearance"
                } else {
                    ""
                },
                if warnings & 8 != 0 { " unknown" } else { "" },
            )));
        }
        Ok(rendered)
    }
}

#[derive(Default)]
struct PreflightState {
    active_xobjects: HashSet<ObjectId>,
    validated_xobjects: HashSet<ObjectId>,
    resource_objects: usize,
    source_resource_bytes: u64,
}

#[derive(Default)]
struct AuditedResources {
    fonts: HashSet<Vec<u8>>,
    xobjects: HashSet<Vec<u8>>,
}

impl PreflightState {
    fn count_resource(&mut self) -> Result<()> {
        self.resource_objects = self.resource_objects.saturating_add(1);
        if self.resource_objects > MAX_RESOURCE_OBJECTS {
            return Err(PdfMasterError::Budget("source resource object count"));
        }
        Ok(())
    }

    fn charge_source_bytes(&mut self, bytes: u64) -> Result<()> {
        self.source_resource_bytes = self.source_resource_bytes.saturating_add(bytes);
        if self.source_resource_bytes > MAX_SOURCE_RESOURCE_BYTES {
            return Err(PdfMasterError::Budget("source resource bytes"));
        }
        Ok(())
    }
}

fn preflight_render(document: &Document, page_id: ObjectId) -> Result<()> {
    let mut state = PreflightState::default();
    let page = document
        .get_object(page_id)
        .and_then(Object::as_dict)
        .map_err(|error| PdfMasterError::Render(format!("content preflight failed: {error}")))?;
    let resources = match inherited_optional(document, page_id, b"Resources")? {
        Some(resources) => preflight_resources(document, resources, &mut state, 0)?,
        None => AuditedResources::default(),
    };
    if let Ok(contents) = page.get(b"Contents") {
        preflight_content_object(document, contents, &resources, &mut state, "page")?;
    }
    if let Ok(annots) = page.get(b"Annots") {
        preflight_annotations(document, annots, &mut state)?;
    }
    Ok(())
}

fn preflight_annotations(
    document: &Document,
    annots: &Object,
    state: &mut PreflightState,
) -> Result<()> {
    let (_, annots) = resolve_object(document, annots)?;
    let annots = annots.as_array().map_err(|_| {
        PdfMasterError::Render(
            "annotation appearance preflight warning: Annots is not an array".into(),
        )
    })?;
    for annot in annots {
        let (_, annot) = resolve_object(document, annot)?;
        let annot = annot.as_dict().map_err(|_| {
            PdfMasterError::Render(
                "annotation appearance preflight warning: annotation is not a dictionary".into(),
            )
        })?;
        let Ok(appearance) = annot.get(b"AP") else {
            continue;
        };
        let (_, appearance) = resolve_object(document, appearance)?;
        let appearance = appearance.as_dict().map_err(|_| {
            PdfMasterError::Render(
                "annotation appearance preflight warning: AP is not a dictionary".into(),
            )
        })?;
        let Ok(normal) = appearance.get(b"N") else {
            continue;
        };
        let (_, normal) = resolve_object(document, normal)?;
        match normal {
            Object::Stream(_) => preflight_xobject(document, normal, state, 1)?,
            Object::Dictionary(states) => {
                if states.is_empty() {
                    return Err(PdfMasterError::Render(
                        "annotation appearance preflight warning: empty normal appearance states"
                            .into(),
                    ));
                }
                for (_, appearance) in states.iter() {
                    let (_, appearance) = resolve_object(document, appearance)?;
                    if !matches!(appearance, Object::Stream(_)) {
                        return Err(PdfMasterError::Render(
                            "annotation appearance preflight warning: normal state is not a stream"
                                .into(),
                        ));
                    }
                    preflight_xobject(document, appearance, state, 1)?;
                }
            }
            _ => {
                return Err(PdfMasterError::Render(
                    "annotation appearance preflight warning: normal appearance is invalid".into(),
                ));
            }
        }
    }
    Ok(())
}

fn preflight_content_object(
    document: &Document,
    object: &Object,
    resources: &AuditedResources,
    state: &mut PreflightState,
    owner: &str,
) -> Result<()> {
    let mut data = Vec::new();
    append_content_bytes(document, object, &mut data, owner)?;
    let decoded = Content::decode_strict(&data).map_err(|error| {
        PdfMasterError::Render(format!("content preflight failed for {owner}: {error}"))
    })?;
    state.charge_source_bytes(data.len() as u64)?;
    validate_operations(document, &decoded.operations, resources, state, owner)
}

fn validate_operations(
    document: &Document,
    operations: &[Operation],
    resources: &AuditedResources,
    state: &mut PreflightState,
    owner: &str,
) -> Result<()> {
    let mut graphics_stack = Vec::new();
    let mut in_text = false;
    let mut text_font_selected = false;
    let mut marked_content_depth = 0usize;
    for operation in operations {
        let operands = operation.operands.as_slice();
        match operation.operator.as_str() {
            "q" => {
                require_operand_count(operation, owner, 0)?;
                graphics_stack.push(text_font_selected);
                if graphics_stack.len() > MAX_RESOURCE_DEPTH {
                    return Err(PdfMasterError::Budget("graphics state depth"));
                }
            }
            "Q" => {
                require_operand_count(operation, owner, 0)?;
                text_font_selected = graphics_stack.pop().ok_or_else(|| {
                    operation_error(owner, operation, "unbalanced graphics state restore")
                })?;
            }
            "BT" => {
                require_operand_count(operation, owner, 0)?;
                if in_text {
                    return Err(operation_error(
                        owner,
                        operation,
                        "nested text objects are invalid",
                    ));
                }
                in_text = true;
                text_font_selected = false;
            }
            "ET" => {
                require_operand_count(operation, owner, 0)?;
                if !in_text {
                    return Err(operation_error(owner, operation, "text object is not open"));
                }
                in_text = false;
                text_font_selected = false;
            }
            "cm" | "c" => require_numbers(operation, owner, 6)?,
            "Tm" => {
                require_text_object(operation, owner, in_text)?;
                require_numbers(operation, owner, 6)?;
            }
            "v" | "y" => require_numbers(operation, owner, 4)?,
            "re" => require_numbers(operation, owner, 4)?,
            "m" | "l" | "Td" | "TD" => {
                require_numbers(operation, owner, 2)?;
                if matches!(operation.operator.as_str(), "Td" | "TD") {
                    require_text_object(operation, owner, in_text)?;
                }
            }
            "w" | "M" | "i" | "Tc" | "Tw" | "Tz" | "TL" | "Ts" => {
                let value = require_one_number(operation, owner)?;
                match operation.operator.as_str() {
                    "M" if value < 1.0 => {
                        return Err(operation_error(owner, operation, "must be at least 1"));
                    }
                    "i" if !(0.0..=100.0).contains(&value) => {
                        return Err(operation_error(owner, operation, "must be in 0..=100"));
                    }
                    "Tc" | "Tw" | "Tz" | "TL" | "Ts" => {
                        require_text_object(operation, owner, in_text)?;
                    }
                    _ => {}
                }
            }
            "J" | "j" => require_integer_in_range(operation, owner, 0, 2)?,
            "Tr" => {
                require_text_object(operation, owner, in_text)?;
                require_integer_in_range(operation, owner, 0, 7)?;
            }
            "d" => validate_dash_pattern(operation, owner)?,
            "G" | "g" => require_color_numbers(operation, owner, 1)?,
            "RG" | "rg" => require_color_numbers(operation, owner, 3)?,
            "K" | "k" => require_color_numbers(operation, owner, 4)?,
            "h" | "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "n" | "W" | "W*" => {
                require_operand_count(operation, owner, 0)?
            }
            "Tf" => {
                require_text_object(operation, owner, in_text)?;
                require_operand_count(operation, owner, 2)?;
                let font_name = operands[0].as_name().map_err(|_| {
                    operation_error(owner, operation, "first operand must be a font name")
                })?;
                let size = finite_number(&operands[1]).ok_or_else(|| {
                    operation_error(owner, operation, "font size must be a finite number")
                })?;
                if size <= 0.0 {
                    return Err(operation_error(
                        owner,
                        operation,
                        "font size must be positive",
                    ));
                }
                if !resources.fonts.contains(font_name) {
                    return Err(operation_error(
                        owner,
                        operation,
                        "font resource name is not linked",
                    ));
                }
                text_font_selected = true;
            }
            "Tj" | "'" => {
                require_text_object(operation, owner, in_text)?;
                require_operand_count(operation, owner, 1)?;
                if !matches!(operands[0], Object::String(_, _)) {
                    return Err(operation_error(
                        owner,
                        operation,
                        "operand must be a string",
                    ));
                }
                require_text_font(operation, owner, text_font_selected)?;
            }
            "TJ" => {
                require_text_object(operation, owner, in_text)?;
                require_operand_count(operation, owner, 1)?;
                let array = operands[0]
                    .as_array()
                    .map_err(|_| operation_error(owner, operation, "operand must be an array"))?;
                for value in array {
                    if !matches!(value, Object::String(_, _)) && finite_number(value).is_none() {
                        return Err(operation_error(
                            owner,
                            operation,
                            "array entries must be strings or finite numbers",
                        ));
                    }
                }
                require_text_font(operation, owner, text_font_selected)?;
            }
            "\"" => {
                require_text_object(operation, owner, in_text)?;
                require_operand_count(operation, owner, 3)?;
                if finite_number(&operands[0]).is_none()
                    || finite_number(&operands[1]).is_none()
                    || !matches!(operands[2], Object::String(_, _))
                {
                    return Err(operation_error(
                        owner,
                        operation,
                        "requires two finite numbers followed by a string",
                    ));
                }
                require_text_font(operation, owner, text_font_selected)?;
            }
            "T*" => {
                require_text_object(operation, owner, in_text)?;
                require_operand_count(operation, owner, 0)?;
            }
            "Do" => {
                require_operand_count(operation, owner, 1)?;
                let name = operands[0].as_name().map_err(|_| {
                    operation_error(owner, operation, "operand must be an XObject name")
                })?;
                if !resources.xobjects.contains(name) {
                    return Err(operation_error(
                        owner,
                        operation,
                        "XObject resource name is not linked",
                    ));
                }
            }
            "CS" | "cs" => {
                require_operand_count(operation, owner, 1)?;
                let name = operands[0].as_name().map_err(|_| {
                    operation_error(owner, operation, "operand must be a color-space name")
                })?;
                if !is_device_color_space(name) {
                    return Err(operation_error(
                        owner,
                        operation,
                        "only built-in device color spaces are supported",
                    ));
                }
            }
            "ri" | "MP" => {
                require_operand_count(operation, owner, 1)?;
                operands[0]
                    .as_name()
                    .map_err(|_| operation_error(owner, operation, "operand must be a name"))?;
            }
            "BMC" => {
                require_operand_count(operation, owner, 1)?;
                operands[0]
                    .as_name()
                    .map_err(|_| operation_error(owner, operation, "operand must be a name"))?;
                marked_content_depth = marked_content_depth
                    .checked_add(1)
                    .ok_or(PdfMasterError::Budget("marked content depth"))?;
                if marked_content_depth > MAX_RESOURCE_DEPTH {
                    return Err(PdfMasterError::Budget("marked content depth"));
                }
            }
            "EMC" => {
                require_operand_count(operation, owner, 0)?;
                marked_content_depth = marked_content_depth.checked_sub(1).ok_or_else(|| {
                    operation_error(owner, operation, "marked-content sequence is not open")
                })?;
            }
            "BI" => {
                require_operand_count(operation, owner, 1)?;
                let image = operands[0].as_stream().map_err(|_| {
                    operation_error(owner, operation, "operand must be an inline image")
                })?;
                state.count_resource()?;
                validate_image_stream(document, image, state, 0)?;
            }
            "gs" | "sh" | "SC" | "sc" | "SCN" | "scn" | "DP" | "BDC" | "BX" | "EX" => {
                return Err(operation_error(
                    owner,
                    operation,
                    "is unsupported in this foundation",
                ));
            }
            _ => {
                return Err(operation_error(
                    owner,
                    operation,
                    "is not in the audited allowlist",
                ));
            }
        }
    }
    if in_text {
        return Err(PdfMasterError::Render(format!(
            "content preflight failed for {owner}: unclosed BT operator"
        )));
    }
    if !graphics_stack.is_empty() {
        return Err(PdfMasterError::Render(format!(
            "content preflight failed for {owner}: unbalanced q operator"
        )));
    }
    if marked_content_depth != 0 {
        return Err(PdfMasterError::Render(format!(
            "content preflight failed for {owner}: unbalanced BMC operator"
        )));
    }
    Ok(())
}

fn require_operand_count(operation: &Operation, owner: &str, expected: usize) -> Result<()> {
    if operation.operands.len() != expected {
        return Err(operation_error(
            owner,
            operation,
            &format!("expected {expected} operand(s)"),
        ));
    }
    Ok(())
}

fn require_numbers(operation: &Operation, owner: &str, expected: usize) -> Result<()> {
    require_operand_count(operation, owner, expected)?;
    if operation
        .operands
        .iter()
        .any(|value| finite_number(value).is_none())
    {
        return Err(operation_error(
            owner,
            operation,
            "all operands must be finite numbers",
        ));
    }
    Ok(())
}

fn require_one_number(operation: &Operation, owner: &str) -> Result<f32> {
    require_operand_count(operation, owner, 1)?;
    finite_number(&operation.operands[0])
        .ok_or_else(|| operation_error(owner, operation, "operand must be a finite number"))
}

fn require_integer_in_range(
    operation: &Operation,
    owner: &str,
    minimum: i64,
    maximum: i64,
) -> Result<()> {
    require_operand_count(operation, owner, 1)?;
    let value = operation.operands[0]
        .as_i64()
        .map_err(|_| operation_error(owner, operation, "operand must be an integer"))?;
    if !(minimum..=maximum).contains(&value) {
        return Err(operation_error(
            owner,
            operation,
            "integer operand is out of range",
        ));
    }
    Ok(())
}

fn validate_dash_pattern(operation: &Operation, owner: &str) -> Result<()> {
    require_operand_count(operation, owner, 2)?;
    let dashes = operation.operands[0]
        .as_array()
        .map_err(|_| operation_error(owner, operation, "first operand must be an array"))?;
    if dashes
        .iter()
        .any(|value| finite_number(value).is_none_or(|number| number < 0.0))
        || finite_number(&operation.operands[1]).is_none_or(|number| number < 0.0)
    {
        return Err(operation_error(
            owner,
            operation,
            "dash entries and phase must be finite non-negative numbers",
        ));
    }
    Ok(())
}

fn require_color_numbers(operation: &Operation, owner: &str, expected: usize) -> Result<()> {
    require_numbers(operation, owner, expected)?;
    if operation
        .operands
        .iter()
        .any(|value| finite_number(value).is_none_or(|number| !(0.0..=1.0).contains(&number)))
    {
        return Err(operation_error(
            owner,
            operation,
            "color operands must be in 0..=1",
        ));
    }
    Ok(())
}

fn require_text_object(operation: &Operation, owner: &str, in_text: bool) -> Result<()> {
    if !in_text {
        return Err(operation_error(
            owner,
            operation,
            "requires an open text object",
        ));
    }
    Ok(())
}

fn require_text_font(operation: &Operation, owner: &str, selected: bool) -> Result<()> {
    if !selected {
        return Err(operation_error(
            owner,
            operation,
            "requires an explicit validated Tf font selection in this text object",
        ));
    }
    Ok(())
}

fn finite_number(value: &Object) -> Option<f32> {
    value.as_float().ok().filter(|value| value.is_finite())
}

fn is_device_color_space(name: &[u8]) -> bool {
    matches!(
        name,
        b"DeviceGray" | b"G" | b"Gray" | b"DeviceRGB" | b"RGB" | b"DeviceCMYK" | b"CMYK"
    )
}

fn operation_error(owner: &str, operation: &Operation, detail: &str) -> PdfMasterError {
    PdfMasterError::Render(format!(
        "content preflight failed for {owner}: operator {} {detail}",
        operation.operator
    ))
}

fn append_content_bytes(
    document: &Document,
    object: &Object,
    output: &mut Vec<u8>,
    owner: &str,
) -> Result<()> {
    let (_, object) = resolve_object(document, object)?;
    match object {
        Object::Stream(stream) => {
            let remaining = MAX_RENDER_CONTENT_BYTES
                .checked_sub(output.len())
                .ok_or(PdfMasterError::Budget("render content bytes"))?;
            let filters = image_filters(document, &stream.dict)?;
            let decoded = match filters.as_slice() {
                [] => {
                    if stream.content.len() > remaining {
                        return Err(PdfMasterError::Budget("render content bytes"));
                    }
                    stream.content.clone()
                }
                [filter] if matches!(filter.as_slice(), b"FlateDecode" | b"Fl") => {
                    if stream.dict.has(b"DecodeParms") {
                        return Err(PdfMasterError::Render(format!(
                            "content preflight failed for {owner}: DecodeParms is unsupported on content streams"
                        )));
                    }
                    strict_zlib_decode(&stream.content, remaining, "content preflight")?
                }
                _ => {
                    return Err(PdfMasterError::Render(format!(
                        "content preflight failed for {owner}: unsupported content filter"
                    )));
                }
            };
            output.extend_from_slice(&decoded);
            output.push(b'\n');
            if output.len() > MAX_RENDER_CONTENT_BYTES {
                return Err(PdfMasterError::Budget("render content bytes"));
            }
            Ok(())
        }
        Object::Array(values) => {
            for value in values {
                append_content_bytes(document, value, output, owner)?;
            }
            Ok(())
        }
        _ => Err(PdfMasterError::Render(format!(
            "content preflight failed for {owner}: Contents must be a stream or stream array"
        ))),
    }
}

fn preflight_resources(
    document: &Document,
    resources: &Object,
    state: &mut PreflightState,
    depth: usize,
) -> Result<AuditedResources> {
    if depth > MAX_RESOURCE_DEPTH {
        return Err(PdfMasterError::Budget("source resource depth"));
    }
    let (_, resources) = resolve_object(document, resources)?;
    let resources = resources.as_dict().map_err(|_| {
        PdfMasterError::Render("source Resources must resolve to a dictionary".into())
    })?;
    let mut audited = AuditedResources::default();
    for (category, value) in resources.iter() {
        match category.as_slice() {
            b"Font" => preflight_fonts(document, value, state, &mut audited)?,
            b"XObject" => {
                let (_, xobjects) = resolve_object(document, value)?;
                let xobjects = xobjects.as_dict().map_err(|_| {
                    PdfMasterError::Render(
                        "source XObject resources must resolve to a dictionary".into(),
                    )
                })?;
                for (name, object) in xobjects.iter() {
                    audited.xobjects.insert(name.clone());
                    preflight_xobject(document, object, state, depth + 1)?;
                }
            }
            b"ProcSet" => validate_proc_set(document, value)?,
            b"ColorSpace" | b"Pattern" | b"Shading" | b"ExtGState" | b"Properties" => {
                return Err(PdfMasterError::Render(format!(
                    "resource category {} is unsupported in this foundation",
                    String::from_utf8_lossy(category)
                )));
            }
            _ => {
                return Err(PdfMasterError::Render(format!(
                    "unmodeled resource category {}",
                    String::from_utf8_lossy(category)
                )));
            }
        }
    }
    Ok(audited)
}

fn preflight_fonts(
    document: &Document,
    fonts: &Object,
    state: &mut PreflightState,
    audited: &mut AuditedResources,
) -> Result<()> {
    let (_, fonts) = resolve_object(document, fonts)?;
    let fonts = fonts.as_dict().map_err(|_| {
        PdfMasterError::Render("Font resources must resolve to a dictionary".into())
    })?;
    for (name, font) in fonts.iter() {
        state.count_resource()?;
        let (_, font) = resolve_object(document, font)?;
        let font = font
            .as_dict()
            .map_err(|_| PdfMasterError::Render("Font resource must be a dictionary".into()))?;
        validate_standard_font(document, font)?;
        audited.fonts.insert(name.clone());
    }
    Ok(())
}

fn validate_standard_font(document: &Document, font: &Dictionary) -> Result<()> {
    for (key, value) in font.iter() {
        if !matches!(
            key.as_slice(),
            b"Type" | b"Subtype" | b"BaseFont" | b"Encoding" | b"Name"
        ) {
            return Err(PdfMasterError::Render(format!(
                "Font field {} is unsupported in this foundation",
                String::from_utf8_lossy(key)
            )));
        }
        let (_, value) = resolve_object(document, value)?;
        if matches!(value, Object::Stream(_)) {
            return Err(PdfMasterError::Render(
                "Font resource streams are unsupported in this foundation".into(),
            ));
        }
    }
    let font_type = font
        .get(b"Type")
        .and_then(Object::as_name)
        .map_err(|_| PdfMasterError::Render("Font Type must be Font".into()))?;
    if font_type != b"Font" {
        return Err(PdfMasterError::Render("Font Type must be Font".into()));
    }
    let subtype = font
        .get(b"Subtype")
        .and_then(Object::as_name)
        .map_err(|_| PdfMasterError::Render("Font Subtype must be Type1".into()))?;
    if subtype != b"Type1" {
        return Err(PdfMasterError::Render(
            "only audited standard Type1 Font resources are supported".into(),
        ));
    }
    let base_font = font
        .get(b"BaseFont")
        .and_then(Object::as_name)
        .map_err(|_| PdfMasterError::Render("Font BaseFont must be a name".into()))?;
    if !is_standard_14(base_font) {
        return Err(PdfMasterError::Render(
            "Font BaseFont is not one of the standard 14 fonts".into(),
        ));
    }
    if let Ok(encoding) = font.get(b"Encoding") {
        let (_, encoding) = resolve_object(document, encoding)?;
        let encoding = encoding
            .as_name()
            .map_err(|_| PdfMasterError::Render("Font Encoding must be a known name".into()))?;
        if !matches!(
            encoding,
            b"StandardEncoding" | b"WinAnsiEncoding" | b"MacRomanEncoding"
        ) {
            return Err(PdfMasterError::Render(
                "Font Encoding is unsupported in this foundation".into(),
            ));
        }
    }
    Ok(())
}

fn is_standard_14(name: &[u8]) -> bool {
    matches!(
        name,
        b"Times-Roman"
            | b"Times-Bold"
            | b"Times-Italic"
            | b"Times-BoldItalic"
            | b"Helvetica"
            | b"Helvetica-Bold"
            | b"Helvetica-Oblique"
            | b"Helvetica-BoldOblique"
            | b"Courier"
            | b"Courier-Bold"
            | b"Courier-Oblique"
            | b"Courier-BoldOblique"
            | b"Symbol"
            | b"ZapfDingbats"
    )
}

fn validate_proc_set(document: &Document, proc_set: &Object) -> Result<()> {
    let (_, proc_set) = resolve_object(document, proc_set)?;
    let proc_set = proc_set
        .as_array()
        .map_err(|_| PdfMasterError::Render("ProcSet resource must be an array of names".into()))?;
    for value in proc_set {
        let (_, value) = resolve_object(document, value)?;
        value.as_name().map_err(|_| {
            PdfMasterError::Render("ProcSet resource must contain only names".into())
        })?;
    }
    Ok(())
}

fn preflight_xobject(
    document: &Document,
    object: &Object,
    state: &mut PreflightState,
    depth: usize,
) -> Result<()> {
    if depth > MAX_RESOURCE_DEPTH {
        return Err(PdfMasterError::Budget("source resource depth"));
    }
    let root_id = match object {
        Object::Reference(id) => Some(*id),
        _ => None,
    };
    if let Some(id) = root_id {
        if state.active_xobjects.contains(&id) {
            return Err(PdfMasterError::Render("source resource cycle".into()));
        }
        if state.validated_xobjects.contains(&id) {
            return Ok(());
        }
        state.active_xobjects.insert(id);
    }
    state.count_resource()?;
    let (_, object) = resolve_object(document, object)?;
    let stream = object
        .as_stream()
        .map_err(|_| PdfMasterError::Render("source XObject must resolve to a stream".into()))?;
    let subtype = stream
        .dict
        .get(b"Subtype")
        .and_then(Object::as_name)
        .map_err(|_| PdfMasterError::Render("source XObject is missing Subtype".into()))?;
    match subtype {
        b"Image" => validate_image_stream(document, stream, state, depth)?,
        b"Form" => {
            validate_form_stream(document, stream)?;
            let resources = match stream.dict.get(b"Resources") {
                Ok(resources) => preflight_resources(document, resources, state, depth + 1)?,
                Err(_) => AuditedResources::default(),
            };
            preflight_content_object(
                document,
                &Object::Stream(stream.clone()),
                &resources,
                state,
                "Form XObject",
            )?;
        }
        _ => {
            return Err(PdfMasterError::Render(format!(
                "unsupported source XObject subtype {}",
                String::from_utf8_lossy(subtype)
            )));
        }
    }
    if let Some(id) = root_id {
        state.active_xobjects.remove(&id);
        state.validated_xobjects.insert(id);
    }
    Ok(())
}

fn validate_form_stream(document: &Document, stream: &Stream) -> Result<()> {
    for (key, _) in stream.dict.iter() {
        if !matches!(
            key.as_slice(),
            b"Type"
                | b"Subtype"
                | b"FormType"
                | b"BBox"
                | b"Matrix"
                | b"Resources"
                | b"Filter"
                | b"Length"
        ) {
            return Err(PdfMasterError::Render(format!(
                "Form XObject field {} is unsupported in this foundation",
                String::from_utf8_lossy(key)
            )));
        }
    }

    let form_type_name = stream
        .dict
        .get(b"Type")
        .map_err(|_| PdfMasterError::Render("Form XObject is missing Type".into()))?;
    let (_, form_type_name) = resolve_object(document, form_type_name)?;
    if form_type_name.as_name().ok() != Some(b"XObject") {
        return Err(PdfMasterError::Render(
            "Form XObject Type must be XObject".into(),
        ));
    }

    let bbox = stream
        .dict
        .get(b"BBox")
        .map_err(|_| PdfMasterError::Render("Form XObject is missing BBox".into()))?;
    validate_finite_number_array(document, bbox, 4, "Form XObject BBox")?;

    if let Ok(matrix) = stream.dict.get(b"Matrix") {
        validate_finite_number_array(document, matrix, 6, "Form XObject Matrix")?;
    }
    if let Ok(form_type) = stream.dict.get(b"FormType") {
        let (_, form_type) = resolve_object(document, form_type)?;
        if form_type.as_i64().ok() != Some(1) {
            return Err(PdfMasterError::Render(
                "Form XObject FormType must be the integer 1".into(),
            ));
        }
    }
    Ok(())
}

fn validate_finite_number_array(
    document: &Document,
    object: &Object,
    expected: usize,
    label: &str,
) -> Result<()> {
    let (_, object) = resolve_object(document, object)?;
    let values = object
        .as_array()
        .map_err(|_| PdfMasterError::Render(format!("{label} must be an array")))?;
    if values.len() != expected {
        return Err(PdfMasterError::Render(format!(
            "{label} must contain exactly {expected} numbers"
        )));
    }
    for value in values {
        let (_, value) = resolve_object(document, value)?;
        if finite_number(value).is_none() {
            return Err(PdfMasterError::Render(format!(
                "{label} entries must be finite numbers"
            )));
        }
    }
    Ok(())
}

fn validate_image_stream(
    document: &Document,
    stream: &Stream,
    state: &mut PreflightState,
    depth: usize,
) -> Result<()> {
    if depth > MAX_RESOURCE_DEPTH {
        return Err(PdfMasterError::Budget("source resource depth"));
    }
    let filters = image_filters(document, &stream.dict)?;
    if filters
        .iter()
        .any(|filter| matches!(filter.as_slice(), b"DCTDecode" | b"DCT"))
    {
        return Err(PdfMasterError::Render(
            "source image DCTDecode is unsupported until encoded dimensions are safely validated"
                .into(),
        ));
    }
    if filters
        .iter()
        .any(|filter| matches!(filter.as_slice(), b"JPXDecode"))
    {
        return Err(PdfMasterError::Render(
            "source image JPXDecode is unsupported in this foundation".into(),
        ));
    }
    if filters.len() > 1
        || filters
            .first()
            .is_some_and(|filter| !matches!(filter.as_slice(), b"FlateDecode" | b"Fl"))
    {
        return Err(PdfMasterError::Render(
            "source image filter is unsupported in this foundation".into(),
        ));
    }

    let image_mask = dict_value(&stream.dict, b"ImageMask", b"IM")
        .map(|value| resolve_object(document, value))
        .transpose()?
        .map(|(_, value)| value.as_bool())
        .transpose()
        .map_err(|_| PdfMasterError::Render("source image ImageMask must be boolean".into()))?
        .unwrap_or(false);
    let width = positive_usize(document, &stream.dict, b"Width", b"W", "Width")?;
    let height = positive_usize(document, &stream.dict, b"Height", b"H", "Height")?;
    let bits = if image_mask {
        1usize
    } else {
        positive_usize(
            document,
            &stream.dict,
            b"BitsPerComponent",
            b"BPC",
            "BitsPerComponent",
        )?
    };
    if !matches!(bits, 1 | 2 | 4 | 8 | 16) {
        return Err(PdfMasterError::Render(
            "source image BitsPerComponent is unsupported".into(),
        ));
    }
    let components = if image_mask {
        1usize
    } else {
        image_components(document, &stream.dict)?
    };
    let pixels = (width as u64)
        .checked_mul(height as u64)
        .ok_or(PdfMasterError::Budget("source image pixels"))?;
    if pixels > MAX_SOURCE_IMAGE_PIXELS {
        return Err(PdfMasterError::Budget("source image pixels"));
    }
    let row_bits = width
        .checked_mul(bits)
        .and_then(|value| value.checked_mul(components))
        .ok_or(PdfMasterError::Budget("source image bytes"))?;
    let source_bytes = row_bits
        .div_ceil(8)
        .checked_mul(height)
        .ok_or(PdfMasterError::Budget("source image bytes"))?;
    let working_bytes = pixels
        .checked_mul(4)
        .and_then(|rgba| rgba.checked_add(source_bytes as u64))
        .ok_or(PdfMasterError::Budget("source image bytes"))?;
    if working_bytes > MAX_SOURCE_RESOURCE_BYTES {
        return Err(PdfMasterError::Budget("source image bytes"));
    }
    state.charge_source_bytes(working_bytes)?;
    if filters
        .first()
        .is_some_and(|filter| matches!(filter.as_slice(), b"FlateDecode" | b"Fl"))
    {
        let strict_limit = source_bytes
            .checked_add(height)
            .ok_or(PdfMasterError::Budget("source image bytes"))?;
        strict_zlib_decode(&stream.content, strict_limit, "source image preflight")?;
    }
    let decoded = stream
        .get_plain_content_with_limit(source_bytes)
        .map_err(|error| {
            PdfMasterError::Render(format!("source image decode preflight failed: {error}"))
        })?;
    if decoded.len() != source_bytes {
        return Err(PdfMasterError::Render(format!(
            "source image decoded length mismatch: declared {source_bytes}, found {}",
            decoded.len()
        )));
    }

    for key in [b"SMask".as_slice(), b"Mask".as_slice()] {
        if let Ok(mask) = stream.dict.get(key)
            && !matches!(mask, Object::Array(_))
        {
            preflight_xobject(document, mask, state, depth + 1)?;
        }
    }
    Ok(())
}

fn image_filters(document: &Document, dictionary: &Dictionary) -> Result<Vec<Vec<u8>>> {
    let Some(filter) = dictionary.get(b"Filter").ok() else {
        return Ok(Vec::new());
    };
    let (_, filter) = resolve_object(document, filter)?;
    match filter {
        Object::Name(name) => Ok(vec![name.clone()]),
        Object::Array(values) => values
            .iter()
            .map(|value| {
                let (_, value) = resolve_object(document, value)?;
                value.as_name().map(Vec::from).map_err(|_| {
                    PdfMasterError::Render("source image Filter entries must be names".into())
                })
            })
            .collect(),
        _ => Err(PdfMasterError::Render(
            "source image Filter must be a name or array".into(),
        )),
    }
}

fn strict_zlib_decode(content: &[u8], limit: usize, label: &str) -> Result<Vec<u8>> {
    let read_limit = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    let mut decoder = ZlibDecoder::new(content).take(read_limit);
    let mut output = Vec::with_capacity(content.len().min(limit));
    decoder.read_to_end(&mut output).map_err(|error| {
        PdfMasterError::Render(format!("{label} failed strict FlateDecode: {error}"))
    })?;
    if output.len() > limit {
        return Err(PdfMasterError::Budget("decoded stream bytes"));
    }
    Ok(output)
}

fn image_components(document: &Document, dictionary: &Dictionary) -> Result<usize> {
    let color_space = dict_value(dictionary, b"ColorSpace", b"CS")
        .ok_or_else(|| PdfMasterError::Render("source image is missing ColorSpace".into()))?;
    let (_, color_space) = resolve_object(document, color_space)?;
    match color_space {
        Object::Name(name) => device_color_components(name).ok_or_else(|| {
            PdfMasterError::Render(
                "source image ColorSpace is unsupported in this foundation".into(),
            )
        }),
        Object::Array(values) => validate_indexed_color_space(document, values),
        _ => Err(PdfMasterError::Render(
            "source image ColorSpace is unsupported in this foundation".into(),
        )),
    }
}

fn validate_indexed_color_space(document: &Document, values: &[Object]) -> Result<usize> {
    if values.len() != 4
        || !matches!(
            values.first().and_then(|value| value.as_name().ok()),
            Some(b"Indexed" | b"I")
        )
    {
        return Err(PdfMasterError::Render(
            "source image Indexed ColorSpace must contain exactly four entries".into(),
        ));
    }
    let (_, base) = resolve_object(document, &values[1])?;
    let base = base.as_name().map_err(|_| {
        PdfMasterError::Render(
            "source image Indexed base must be a built-in device ColorSpace".into(),
        )
    })?;
    let components = device_color_components(base).ok_or_else(|| {
        PdfMasterError::Render(
            "source image Indexed base must be DeviceGray, DeviceRGB, or DeviceCMYK".into(),
        )
    })?;
    let (_, high) = resolve_object(document, &values[2])?;
    let high = high.as_i64().map_err(|_| {
        PdfMasterError::Render("source image Indexed hival must be an integer".into())
    })?;
    if !(0..=255).contains(&high) {
        return Err(PdfMasterError::Render(
            "source image Indexed hival must be in 0..=255".into(),
        ));
    }
    let lookup = match &values[3] {
        Object::String(bytes, _) => bytes,
        _ => {
            return Err(PdfMasterError::Render(
                "source image Indexed requires a bounded direct lookup string".into(),
            ));
        }
    };
    let entries = usize::try_from(high)
        .ok()
        .and_then(|value| value.checked_add(1))
        .ok_or(PdfMasterError::Budget("Indexed lookup bytes"))?;
    let expected = entries
        .checked_mul(components)
        .ok_or(PdfMasterError::Budget("Indexed lookup bytes"))?;
    if lookup.len() != expected {
        return Err(PdfMasterError::Render(format!(
            "source image Indexed lookup length mismatch: expected {expected}, found {}",
            lookup.len()
        )));
    }
    Ok(1)
}

fn device_color_components(name: &[u8]) -> Option<usize> {
    match name {
        b"DeviceGray" | b"G" | b"Gray" => Some(1),
        b"DeviceRGB" | b"RGB" => Some(3),
        b"DeviceCMYK" | b"CMYK" => Some(4),
        _ => None,
    }
}

fn positive_usize(
    document: &Document,
    dictionary: &Dictionary,
    key: &[u8],
    short_key: &[u8],
    label: &str,
) -> Result<usize> {
    let value = dict_value(dictionary, key, short_key)
        .ok_or_else(|| PdfMasterError::Render(format!("source image is missing {label}")))?;
    let (_, value) = resolve_object(document, value)?;
    let value = value
        .as_i64()
        .map_err(|_| PdfMasterError::Render(format!("source image {label} must be an integer")))?;
    usize::try_from(value)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| PdfMasterError::Render(format!("source image {label} must be positive")))
}

fn dict_value<'a>(dictionary: &'a Dictionary, key: &[u8], short_key: &[u8]) -> Option<&'a Object> {
    dictionary
        .get(key)
        .or_else(|_| dictionary.get(short_key))
        .ok()
}

fn resolve_object<'a>(
    document: &'a Document,
    object: &'a Object,
) -> Result<(Option<ObjectId>, &'a Object)> {
    let mut current = object;
    let mut first_id = None;
    let mut seen = HashSet::new();
    for _ in 0..MAX_RESOURCE_DEPTH {
        match current {
            Object::Reference(id) => {
                first_id.get_or_insert(*id);
                if !seen.insert(*id) {
                    return Err(PdfMasterError::Render(
                        "source indirect reference cycle".into(),
                    ));
                }
                current = document.get_object(*id).map_err(|error| {
                    PdfMasterError::Render(format!("source object resolution failed: {error}"))
                })?;
            }
            value => return Ok((first_id, value)),
        }
    }
    Err(PdfMasterError::Budget("source reference depth"))
}

fn interpreter_warning_bit(warning: InterpreterWarning) -> u8 {
    match warning {
        InterpreterWarning::UnsupportedFont => 1,
        InterpreterWarning::ImageDecodeFailure => 2,
        InterpreterWarning::UnresolvedAnnotationAppearance => 4,
        _ => 8,
    }
}

#[cfg(test)]
mod tests {
    use super::interpreter_warning_bit;
    use hayro::hayro_interpret::InterpreterWarning;

    #[test]
    fn warning_capture_covers_unsupported_font_and_image_decode() {
        assert_eq!(
            interpreter_warning_bit(InterpreterWarning::UnsupportedFont),
            1
        );
        assert_eq!(
            interpreter_warning_bit(InterpreterWarning::ImageDecodeFailure),
            2
        );
        assert_eq!(
            interpreter_warning_bit(InterpreterWarning::UnresolvedAnnotationAppearance),
            4
        );
    }
}
