use crate::text_appearance::{
    BundledFont, TextAppearanceRequest, TextLayoutMetrics, TextLineMetrics,
};
use crate::text_fonts::{family, font_context, kind_for_blob};
use crate::{PdfMasterError, Result};
use parley::{
    Alignment, FontFamily, FontFamilyName, LayoutContext, LineHeight, PositionedLayoutItem,
    StyleProperty,
};
use std::borrow::Cow;
pub(crate) struct Shaped {
    pub metrics: TextLayoutMetrics,
    pub runs: Vec<ShapedRun>,
}
pub(crate) struct ShapedRun {
    pub font: BundledFont,
    pub x: f32,
    pub y: f32,
    pub glyphs: Vec<krilla::text::KrillaGlyph>,
}
pub(crate) fn shape(request: &TextAppearanceRequest<'_>) -> Result<Shaped> {
    let mut fonts = font_context()?;
    let mut context = LayoutContext::<()>::new();
    let mut order = vec![request.font];
    for kind in [
        BundledFont::Sans,
        BundledFont::CjkSc,
        BundledFont::Arabic,
        BundledFont::Devanagari,
    ] {
        if !order.contains(&kind) {
            order.push(kind);
        }
    }
    let families: Vec<_> = order
        .iter()
        .map(|kind| FontFamilyName::Named(Cow::Borrowed(family(*kind))))
        .collect();
    let mut shaped = Shaped {
        metrics: TextLayoutMetrics {
            width: 0.0,
            height: 0.0,
            glyph_count: 0,
            fonts: Vec::new(),
            lines: Vec::new(),
        },
        runs: Vec::new(),
    };
    let mut unique =
        std::collections::HashMap::<BundledFont, std::collections::HashSet<u32>>::new();
    for (start, end, separator_end) in hard_paragraphs(request.contents) {
        let text = &request.contents[start..end];
        let mut builder = context.ranged_builder(&mut fonts, text, 1.0, false);
        builder.push_default(StyleProperty::FontFamily(FontFamily::List(Cow::Borrowed(
            &families,
        ))));
        builder.push_default(StyleProperty::FontSize(request.font_size));
        builder.push_default(StyleProperty::LineHeight(LineHeight::FontSizeRelative(
            request.line_height,
        )));
        let mut layout = builder.build(text);
        layout.break_all_lines(Some(request.width));
        layout.align(
            match request.alignment {
                crate::TextAlignment::Left => Alignment::Left,
                crate::TextAlignment::Center => Alignment::Center,
                crate::TextAlignment::Right => Alignment::Right,
            },
            Default::default(),
        );
        let height_offset = shaped.metrics.height;
        let first_line = shaped.metrics.lines.len();
        for line in layout.lines() {
            let metrics = line.metrics();
            let range = line.text_range();
            let advance = metrics.advance - metrics.trailing_whitespace;
            shaped.metrics.width = shaped.metrics.width.max(advance);
            shaped.metrics.lines.push(TextLineMetrics {
                text_range: start + range.start..start + range.end,
                baseline: height_offset + metrics.baseline,
                offset: metrics.offset,
                advance,
            });
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(positioned) = item else {
                    continue;
                };
                let run = positioned.run();
                let kind = kind_for_blob(run.font().data.id(), run.font().index)?;
                let mut glyphs = Vec::new();
                let clusters: Vec<_> = run.visual_clusters().collect();
                let mut i = 0;
                while i < clusters.len() {
                    // Resolve the whole logical shaping cluster before emitting any member.
                    // Krilla's many-glyph ActualText association requires EXACT equal ranges.
                    let mut range = clusters[i].text_range();
                    let mut next = i + 1;
                    while next < clusters.len() && clusters[next].is_ligature_continuation() {
                        let continuation = clusters[next].text_range();
                        range.start = range.start.min(continuation.start);
                        range.end = range.end.max(continuation.end);
                        next += 1;
                    }
                    let global = start + range.start..start + range.end;
                    for cluster in &clusters[i..next] {
                        for glyph in cluster.glyphs() {
                            if glyph.id == 0
                                && request.contents[global.clone()].chars().any(visible_scalar)
                            {
                                return Err(PdfMasterError::InvalidOperation(
                                    "missing visible glyph",
                                ));
                            }
                            unique.entry(kind).or_default().insert(glyph.id);
                            glyphs.push(krilla::text::KrillaGlyph::new(
                                krilla::text::GlyphId::new(glyph.id),
                                glyph.advance / request.font_size,
                                glyph.x / request.font_size,
                                glyph.y / request.font_size,
                                0.0,
                                global.clone(),
                                None,
                            ));
                        }
                    }
                    i = next;
                }
                shaped.metrics.glyph_count += glyphs.len();
                if !glyphs.is_empty() {
                    shaped.runs.push(ShapedRun {
                        font: kind,
                        x: positioned.offset(),
                        y: height_offset + positioned.baseline(),
                        glyphs,
                    });
                }
            }
        }
        if shaped.metrics.lines.len() > first_line {
            shaped.metrics.lines.last_mut().unwrap().text_range.end = separator_end;
        }
        shaped.metrics.height += layout.height();
    }
    shaped.metrics.fonts = [
        BundledFont::Sans,
        BundledFont::CjkSc,
        BundledFont::Arabic,
        BundledFont::Devanagari,
    ]
    .into_iter()
    .filter(|kind| unique.contains_key(kind))
    .collect();
    check_budgets(
        shaped.metrics.lines.len(),
        shaped.metrics.glyph_count,
        unique.values().map(|ids| ids.len()).max().unwrap_or(0),
    )?;
    if shaped.metrics.width > request.width + 0.01 || shaped.metrics.height > request.height + 0.01
    {
        return Err(PdfMasterError::InvalidGeometry(
            "text exceeds appearance box",
        ));
    }
    validate_positions(request, &shaped)?;
    Ok(shaped)
}
fn validate_positions(request: &TextAppearanceRequest<'_>, shaped: &Shaped) -> Result<()> {
    let range_valid = |range: &std::ops::Range<usize>| {
        range.start <= range.end
            && range.end <= request.contents.len()
            && request.contents.is_char_boundary(range.start)
            && request.contents.is_char_boundary(range.end)
    };
    if !shaped.metrics.width.is_finite()
        || !shaped.metrics.height.is_finite()
        || shaped.metrics.lines.iter().any(|line| {
            ![line.baseline, line.offset, line.advance]
                .into_iter()
                .all(f32::is_finite)
                || !range_valid(&line.text_range)
        })
        || shaped.runs.iter().any(|run| {
            !run.x.is_finite()
                || !run.y.is_finite()
                || run.glyphs.iter().any(|glyph| {
                    ![
                        glyph.x_advance,
                        glyph.x_offset,
                        glyph.y_offset,
                        glyph.y_advance,
                    ]
                    .into_iter()
                    .all(f32::is_finite)
                        || !range_valid(&glyph.text_range)
                })
        })
    {
        return Err(PdfMasterError::InvalidOperation(
            "invalid shaped positions or source range",
        ));
    }
    Ok(())
}
fn hard_paragraphs(text: &str) -> Vec<(usize, usize, usize)> {
    let bytes = text.as_bytes();
    let mut result = Vec::new();
    let (mut start, mut i) = (0, 0);
    while i < bytes.len() {
        if matches!(bytes[i], b'\r' | b'\n') {
            let end = i;
            i += if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
                2
            } else {
                1
            };
            result.push((start, end, i));
            start = i;
        } else {
            i += 1;
        }
    }
    result.push((start, bytes.len(), bytes.len()));
    result
}
fn visible_scalar(ch: char) -> bool {
    !ch.is_whitespace()
        && !matches!(ch as u32, 0x00ad | 0x034f | 0x061c | 0x180e | 0x200b..=0x200f | 0x202a..=0x202e | 0x2060..=0x206f | 0xfe00..=0xfe0f | 0xfeff | 0xe0000..=0xe007f | 0xe0100..=0xe01ef)
}
fn check_budgets(lines: usize, glyphs: usize, unique: usize) -> Result<()> {
    if lines > 4_096 {
        return Err(PdfMasterError::Budget("text lines"));
    }
    if glyphs > 65_536 {
        return Err(PdfMasterError::Budget("text glyphs"));
    }
    if unique > 4_096 {
        return Err(PdfMasterError::Budget("font unique glyphs"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_layout_uses_actual_fonts_positions_and_complete_indic_ranges() {
        let request = TextAppearanceRequest {
            contents: "हैलो वर्ल्ड\r\n\r\nLatin 中文 مرحبا",
            width: 360.0,
            height: 400.0,
            font: BundledFont::Sans,
            font_size: 18.0,
            color: crate::Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
            },
            alignment: crate::TextAlignment::Center,
            line_height: 1.3,
        };
        let shaped = shape(&request).unwrap();
        assert_eq!(shaped.metrics.lines.len(), 3);
        assert_eq!(shaped.metrics.fonts.len(), 4);
        assert!(shaped.runs[0].x > 0.0);
        assert_eq!(shaped.runs[0].y, shaped.metrics.lines[0].baseline);
        assert!(shaped.runs[0].glyphs.len() > 1);
        assert_eq!(shaped.runs[0].glyphs[0].text_range, 0..6);
        assert_eq!(shaped.runs[0].glyphs[1].text_range, 0..6);
    }

    #[test]
    fn fresh_positions_validate_all_real_metrics_ranges_and_glyph_coordinates() {
        let request = TextAppearanceRequest {
            contents: "中文 valid",
            width: 360.0,
            height: 400.0,
            font: BundledFont::Sans,
            font_size: 18.0,
            color: crate::Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
            },
            alignment: crate::TextAlignment::Left,
            line_height: 1.3,
        };
        let mut results = Vec::new();
        for field in 0..12 {
            let mut shaped = shape(&request).unwrap();
            match field {
                0 => shaped.metrics.width = f32::NAN,
                1 => shaped.metrics.height = f32::INFINITY,
                2 => shaped.metrics.lines[0].baseline = f32::NAN,
                3 => shaped.metrics.lines[0].offset = f32::NAN,
                4 => shaped.metrics.lines[0].advance = f32::INFINITY,
                5 => shaped.metrics.lines[0].text_range.start = 1,
                6 => shaped.runs[0].x = f32::NAN,
                7 => shaped.runs[0].y = f32::INFINITY,
                8 => shaped.runs[0].glyphs[0].x_advance = f32::NAN,
                9 => shaped.runs[0].glyphs[0].x_offset = f32::INFINITY,
                10 => shaped.runs[0].glyphs[0].y_offset = f32::NAN,
                _ => shaped.runs[0].glyphs[0].text_range.end = request.contents.len() + 1,
            }
            results.push(matches!(
                validate_positions(&request, &shaped),
                Err(PdfMasterError::InvalidOperation(_))
            ));
        }
        assert_eq!(results, vec![true; 12]);
    }

    #[test]
    fn production_budget_helper_enforces_exact_boundaries() {
        assert!(check_budgets(4_096, 65_536, 4_096).is_ok());
        assert!(check_budgets(4_097, 1, 1).is_err());
        assert!(check_budgets(1, 65_537, 1).is_err());
        assert!(check_budgets(1, 1, 4_097).is_err());
    }

    #[test]
    fn fresh_budgets_report_each_specific_resource_as_budget() {
        let results: Vec<_> = [(4097, 1, 1), (1, 65537, 1), (1, 1, 4097)]
            .into_iter()
            .map(|(lines, glyphs, unique)| {
                matches!(
                    check_budgets(lines, glyphs, unique),
                    Err(PdfMasterError::Budget(_))
                )
            })
            .collect();
        assert_eq!(results, vec![true; 3]);
    }
}
