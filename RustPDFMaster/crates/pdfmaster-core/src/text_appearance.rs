//! Pure in-memory, embedded-font paragraph appearance carrier.
use crate::{Color, PdfMasterError, Result, TextAlignment};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BundledFont {
    Sans,
    CjkSc,
    Arabic,
    Devanagari,
}
#[derive(Clone, Debug)]
pub struct TextAppearanceRequest<'a> {
    pub contents: &'a str,
    pub width: f32,
    pub height: f32,
    pub font: BundledFont,
    pub font_size: f32,
    pub color: Color,
    pub alignment: TextAlignment,
    pub line_height: f32,
}
#[derive(Clone, Debug, PartialEq)]
pub struct TextLineMetrics {
    pub text_range: Range<usize>,
    pub baseline: f32,
    pub offset: f32,
    pub advance: f32,
}
#[derive(Clone, Debug, PartialEq)]
pub struct TextLayoutMetrics {
    pub width: f32,
    pub height: f32,
    pub glyph_count: usize,
    pub fonts: Vec<BundledFont>,
    pub lines: Vec<TextLineMetrics>,
}
#[derive(Clone, Debug)]
pub struct PreparedTextAppearance {
    pdf: Vec<u8>,
    metrics: TextLayoutMetrics,
}
impl PreparedTextAppearance {
    pub fn pdf_bytes(&self) -> &[u8] {
        &self.pdf
    }
    pub fn metrics(&self) -> &TextLayoutMetrics {
        &self.metrics
    }
}
pub fn prepare_text_appearance(
    request: &TextAppearanceRequest<'_>,
) -> Result<PreparedTextAppearance> {
    validate_request(request)?;
    let shaped = crate::text_layout::shape(request)?;
    let pdf = emit(request, &shaped)?;
    Ok(PreparedTextAppearance {
        pdf,
        metrics: shaped.metrics,
    })
}
fn validate_request(request: &TextAppearanceRequest<'_>) -> Result<()> {
    if request.contents.len() > 65_536 {
        return Err(PdfMasterError::Budget("text source bytes"));
    }
    if request.contents.trim().is_empty()
        || request
            .contents
            .chars()
            .any(|ch| ch <= '\u{1f}' && !matches!(ch, '\t' | '\r' | '\n'))
    {
        return Err(PdfMasterError::InvalidOperation("empty or controlled text"));
    }
    if [request.width, request.height]
        .into_iter()
        .any(|value| !value.is_finite() || value <= 0.0 || value > 14_400.0)
    {
        return Err(PdfMasterError::InvalidGeometry("appearance dimensions"));
    }
    if !request.font_size.is_finite() || request.font_size <= 0.0 || request.font_size > 1_000.0 {
        return Err(PdfMasterError::InvalidOperation("appearance font size"));
    }
    if !request.line_height.is_finite() || !(1.0..=4.0).contains(&request.line_height) {
        return Err(PdfMasterError::InvalidOperation("appearance line height"));
    }
    if [request.color.r, request.color.g, request.color.b]
        .into_iter()
        .any(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
    {
        return Err(PdfMasterError::InvalidOperation("appearance RGB"));
    }
    Ok(())
}
fn emit(
    request: &TextAppearanceRequest<'_>,
    shaped: &crate::text_layout::Shaped,
) -> Result<Vec<u8>> {
    let mut document = krilla::Document::new_with(krilla::SerializeSettings {
        compress_content_streams: false,
        no_device_cs: false,
        ..Default::default()
    });
    let settings = krilla::page::PageSettings::from_wh(request.width, request.height)
        .ok_or(PdfMasterError::InvalidGeometry("appearance page size"))?;
    let mut page = document.start_page_with(settings);
    let mut surface = page.surface();
    surface.set_fill(Some(krilla::paint::Fill {
        paint: krilla::color::rgb::Color::new(1, 2, 3).into(),
        opacity: krilla::num::NormalizedF32::ONE,
        rule: Default::default(),
    }));
    for run in &shaped.runs {
        let embedded = crate::text_fonts::embedded_fonts()
            .fonts
            .iter()
            .find(|font| font.kind == run.font)
            .ok_or_else(|| PdfMasterError::Serialization("unknown shaped font".into()))?;
        let font = krilla::text::Font::new(embedded.blob.clone().into_raw_parts().0.into(), 0)
            .ok_or_else(|| PdfMasterError::Serialization("embedded PDF font".into()))?;
        surface.draw_glyphs(
            krilla::geom::Point::from_xy(run.x, run.y),
            &run.glyphs,
            font,
            request.contents,
            request.font_size,
            false,
        );
    }
    surface.finish();
    page.finish();
    let bytes = document.finish().map_err(|error| {
        PdfMasterError::Serialization(format!("appearance emission: {error:?}"))
    })?;
    check_output_size(bytes.len())?;
    rewrite_owned_color(&bytes, request.color)
}
const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
fn check_output_size(size: usize) -> Result<()> {
    if size > MAX_OUTPUT_BYTES {
        Err(PdfMasterError::Budget("text appearance output bytes"))
    } else {
        Ok(())
    }
}
fn rewrite_owned_color(bytes: &[u8], color: Color) -> Result<Vec<u8>> {
    check_output_size(bytes.len())?;
    let mut document = lopdf::Document::load_mem_with_options(
        bytes,
        lopdf::LoadOptions {
            strict: true,
            max_decompressed_size: Some(MAX_OUTPUT_BYTES),
            ..Default::default()
        },
    )
    .map_err(|error| PdfMasterError::Serialization(error.to_string()))?;
    let pages = document.get_pages();
    if pages.len() != 1 {
        return Err(PdfMasterError::Serialization(
            "owned carrier must have one page".into(),
        ));
    }
    let page_id = *pages
        .values()
        .next()
        .ok_or_else(|| PdfMasterError::Serialization("no carrier page".into()))?;
    let page = document
        .get_object(page_id)
        .and_then(lopdf::Object::as_dict)
        .map_err(|error| PdfMasterError::Serialization(error.to_string()))?;
    let (stream, stream_id) = crate::notes::resolve_with_id(
        &document,
        page.get(b"Contents")
            .map_err(|error| PdfMasterError::Serialization(error.to_string()))?,
    )
    .map_err(|_| PdfMasterError::Serialization("owned content reference".into()))?;
    let stream = stream.as_stream().map_err(|_| {
        PdfMasterError::Serialization("owned carrier must have one content stream".into())
    })?;
    if stream.dict.has(b"Filter") {
        return Err(PdfMasterError::Serialization(
            "owned content must be unfiltered".into(),
        ));
    }
    check_output_size(stream.content.len())?;
    let mut content = lopdf::content::Content::decode_strict(&stream.content)
        .map_err(|error| PdfMasterError::Serialization(error.to_string()))?;
    let mut rgb_count = 0;
    for operation in &mut content.operations {
        if operation.operator == "rg" {
            let values: std::result::Result<Vec<_>, _> = operation
                .operands
                .iter()
                .map(lopdf::Object::as_float)
                .collect();
            if values.ok().as_deref() != Some(&[1.0f32 / 255.0, 2.0f32 / 255.0, 3.0f32 / 255.0]) {
                return Err(PdfMasterError::Serialization(
                    "owned RGB sentinel mismatch".into(),
                ));
            }
            rgb_count += 1;
            operation.operands = vec![color.r.into(), color.g.into(), color.b.into()];
        } else if matches!(
            operation.operator.as_str(),
            "g" | "G" | "RG" | "k" | "K" | "cs" | "CS" | "sc" | "SC" | "scn" | "SCN"
        ) {
            return Err(PdfMasterError::Serialization(
                "unexpected owned color operation".into(),
            ));
        }
    }
    if rgb_count == 0 {
        return Err(PdfMasterError::Serialization(
            "missing owned RGB sentinel".into(),
        ));
    }
    let encoded = content
        .encode()
        .map_err(|error| PdfMasterError::Serialization(error.to_string()))?;
    check_output_size(encoded.len())?;
    if let Some(id) = stream_id {
        document
            .get_object_mut(id)
            .and_then(lopdf::Object::as_stream_mut)
            .map_err(|error| PdfMasterError::Serialization(error.to_string()))?
            .set_content(encoded);
    } else {
        document
            .get_object_mut(page_id)
            .and_then(lopdf::Object::as_dict_mut)
            .and_then(|page| page.get_mut(b"Contents"))
            .and_then(lopdf::Object::as_stream_mut)
            .map_err(|error| PdfMasterError::Serialization(error.to_string()))?
            .set_content(encoded);
    }
    let mut output = Vec::new();
    document
        .save_to(&mut output)
        .map_err(|error| PdfMasterError::Serialization(error.to_string()))?;
    check_output_size(output.len())?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(contents: &str) -> TextAppearanceRequest<'_> {
        TextAppearanceRequest {
            contents,
            width: 100.0,
            height: 100.0,
            font: BundledFont::Sans,
            font_size: 12.0,
            color: Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
            },
            alignment: TextAlignment::Left,
            line_height: 1.0,
        }
    }

    #[test]
    fn fresh_output_budget_exact_boundary_uses_production_helper() {
        assert!(check_output_size(MAX_OUTPUT_BYTES).is_ok());
        assert!(matches!(
            check_output_size(MAX_OUTPUT_BYTES + 1),
            Err(PdfMasterError::Budget(_))
        ));
    }

    fn carrier_fixture(content: &[u8], pages: usize, streams: usize, filter: bool) -> Vec<u8> {
        use lopdf::{Document, Object, Stream, dictionary};
        let mut doc = Document::with_version("1.7");
        let pages_id = doc.new_object_id();
        let mut kids = Vec::new();
        for _ in 0..pages {
            let mut ids = Vec::new();
            for _ in 0..streams {
                let mut stream = Stream::new(dictionary! {}, content.to_vec());
                if filter {
                    stream.compress().unwrap();
                    assert!(stream.dict.has(b"Filter"));
                }
                ids.push(Object::Reference(doc.add_object(stream)));
            }
            let contents = if streams == 1 {
                ids.pop().unwrap()
            } else {
                Object::Array(ids)
            };
            let page = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()], "Resources" => dictionary!{}, "Contents" => contents });
            kids.push(Object::Reference(page));
        }
        doc.objects.insert(
            pages_id,
            Object::Dictionary(
                dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => pages as i64 },
            ),
        );
        let root = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", root);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        bytes
    }

    fn sentinel_content() -> Vec<u8> {
        use lopdf::content::{Content, Operation};
        Content {
            operations: vec![
                Operation::new(
                    "rg",
                    vec![
                        (1.0f32 / 255.0).into(),
                        (2.0f32 / 255.0).into(),
                        (3.0f32 / 255.0).into(),
                    ],
                ),
                Operation::new("q", vec![]),
                Operation::new("Q", vec![]),
            ],
        }
        .encode()
        .unwrap()
    }

    #[test]
    fn fresh_owned_rewrite_rejects_wrong_missing_or_malformed_sentinel() {
        let cases: &[&[u8]] = &[b"0 0 0 rg\nq Q", b"q Q", b"1 2 rg", b"/bad 0 0 rg"];
        let results: Vec<_> = cases
            .iter()
            .map(|content| {
                matches!(
                    rewrite_owned_color(&carrier_fixture(content, 1, 1, false), input("x").color),
                    Err(PdfMasterError::Serialization(_))
                )
            })
            .collect();
        assert_eq!(results, vec![true; cases.len()]);
    }

    #[test]
    fn fresh_owned_rewrite_rejects_all_other_color_operations() {
        let results: Vec<_> = [
            "g", "G", "RG", "k", "K", "cs", "CS", "sc", "SC", "scn", "SCN",
        ]
        .into_iter()
        .map(|operator| {
            let mut content = sentinel_content();
            content.extend_from_slice(format!("\n0 {operator}\n").as_bytes());
            matches!(
                rewrite_owned_color(&carrier_fixture(&content, 1, 1, false), input("x").color),
                Err(PdfMasterError::Serialization(_))
            )
        })
        .collect();
        assert_eq!(results, vec![true; 11]);
    }

    #[test]
    fn fresh_owned_rewrite_rejects_filter_extra_page_and_extra_content_stream() {
        let mut content = sentinel_content();
        content.extend_from_slice(b"q Q\n".repeat(100).as_slice());
        let results: Vec<_> = [(1, 1, true), (2, 1, false), (1, 2, false)]
            .into_iter()
            .map(|(pages, streams, filter)| {
                matches!(
                    rewrite_owned_color(
                        &carrier_fixture(&content, pages, streams, filter),
                        input("x").color
                    ),
                    Err(PdfMasterError::Serialization(_))
                )
            })
            .collect();
        assert_eq!(results, vec![true; 3]);
    }

    #[test]
    fn fresh_validator_rejects_text_controls_before_layout() {
        let results: Vec<_> = ["", " \t\r\n", "ok\0bad", "ok\u{7}bad", "ok\u{1f}bad"]
            .into_iter()
            .map(|text| {
                matches!(
                    validate_request(&input(text)),
                    Err(PdfMasterError::InvalidOperation(_))
                )
            })
            .collect();
        assert_eq!(results, vec![true; 5]);
    }

    #[test]
    fn fresh_validator_rejects_nonfinite_and_out_of_range_geometry_before_layout() {
        let mut results = Vec::new();
        for value in [
            0.0,
            -1.0,
            14400.1,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
        ] {
            let mut request = input("valid");
            request.width = value;
            results.push(matches!(
                validate_request(&request),
                Err(PdfMasterError::InvalidGeometry(_))
            ));
            request = input("valid");
            request.height = value;
            results.push(matches!(
                validate_request(&request),
                Err(PdfMasterError::InvalidGeometry(_))
            ));
        }
        assert_eq!(results, vec![true; 12]);
    }

    #[test]
    fn fresh_validator_rejects_nonfinite_and_out_of_range_style_before_layout() {
        let mut results = Vec::new();
        for value in [0.0, -1.0, 1000.1, f32::NAN, f32::INFINITY] {
            let mut request = input("valid");
            request.font_size = value;
            results.push(matches!(
                validate_request(&request),
                Err(PdfMasterError::InvalidOperation(_))
            ));
        }
        for value in [0.99, 4.01, f32::NAN, f32::INFINITY] {
            let mut request = input("valid");
            request.line_height = value;
            results.push(matches!(
                validate_request(&request),
                Err(PdfMasterError::InvalidOperation(_))
            ));
        }
        for value in [-0.1, 1.1, f32::NAN, f32::INFINITY] {
            for channel in 0..3 {
                let mut request = input("valid");
                match channel {
                    0 => request.color.r = value,
                    1 => request.color.g = value,
                    _ => request.color.b = value,
                }
                results.push(matches!(
                    validate_request(&request),
                    Err(PdfMasterError::InvalidOperation(_))
                ));
            }
        }
        assert_eq!(results, vec![true; 21]);
    }

    #[test]
    fn exact_input_boundaries_are_enforced_by_the_production_validator() {
        let base = |contents| TextAppearanceRequest {
            contents,
            width: 100.0,
            height: 100.0,
            font: BundledFont::Sans,
            font_size: 12.0,
            color: Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
            },
            alignment: TextAlignment::Left,
            line_height: 1.0,
        };
        let maximum = "a".repeat(65_536);
        let oversized = "a".repeat(65_537);
        assert!(validate_request(&base(&maximum)).is_ok());
        assert!(matches!(
            validate_request(&base(&oversized)),
            Err(PdfMasterError::Budget(_))
        ));
    }
}
