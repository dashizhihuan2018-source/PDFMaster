use pdfmaster_core::PdfMasterError;
use pdfmaster_core::text_appearance::{
    BundledFont, TextAppearanceRequest, prepare_text_appearance,
};
use pdfmaster_core::{Color, TextAlignment};

fn request(contents: &str) -> TextAppearanceRequest<'_> {
    TextAppearanceRequest {
        contents,
        width: 220.,
        height: 700.,
        font: BundledFont::Sans,
        font_size: 18.,
        color: Color {
            r: 0.,
            g: 0.,
            b: 0.,
        },
        alignment: TextAlignment::Left,
        line_height: 1.3,
    }
}

#[test]
fn records_the_fixed_font_measurement_behind_the_220_point_wrap_fixture() {
    let text = "Latin 中文 مرحبا بالعالم हैलो वर्ल्ड\nCafé e\u{301} office 𠮷";
    let mut measured = request(text);
    measured.width = 240.0;
    let at_240 = prepare_text_appearance(&measured).unwrap();
    measured.width = 700.0;
    let at_700 = prepare_text_appearance(&measured).unwrap();
    assert_eq!(at_240.metrics().lines.len(), at_700.metrics().lines.len());
    assert!((at_240.metrics().lines[0].advance - 236.807_98).abs() < 0.001);
}

#[test]
fn missing_visible_glyph_and_nonfinite_geometry_are_errors_not_tofu_or_clamps() {
    let missing = request("\u{10ffff}");
    assert!(matches!(
        prepare_text_appearance(&missing),
        Err(PdfMasterError::InvalidOperation(_))
    ));
    let mut invalid = request("valid");
    invalid.width = f32::NAN;
    assert!(matches!(
        prepare_text_appearance(&invalid),
        Err(PdfMasterError::InvalidGeometry(_))
    ));
}

#[test]
fn rejects_invalid_input_boundaries_without_clamping() {
    for contents in ["", " \t\r\n", "ok\0bad", "ok\u{7}bad"] {
        assert!(matches!(
            prepare_text_appearance(&request(contents)),
            Err(PdfMasterError::InvalidOperation(_))
        ));
    }

    for width in [0.0, -1.0, 14_400.1, f32::INFINITY] {
        let mut invalid = request("valid");
        invalid.width = width;
        assert!(matches!(
            prepare_text_appearance(&invalid),
            Err(PdfMasterError::InvalidGeometry(_))
        ));
    }

    for font_size in [0.0, -1.0, 1_000.1, f32::INFINITY] {
        let mut invalid = request("valid");
        invalid.font_size = font_size;
        assert!(matches!(
            prepare_text_appearance(&invalid),
            Err(PdfMasterError::InvalidOperation(_))
        ));
    }

    for line_height in [0.99, 4.01, f32::NAN] {
        let mut invalid = request("valid");
        invalid.line_height = line_height;
        assert!(matches!(
            prepare_text_appearance(&invalid),
            Err(PdfMasterError::InvalidOperation(_))
        ));
    }

    for color in [
        Color {
            r: -0.1,
            g: 0.0,
            b: 0.0,
        },
        Color {
            r: 0.0,
            g: 1.1,
            b: 0.0,
        },
        Color {
            r: 0.0,
            g: 0.0,
            b: f32::NAN,
        },
    ] {
        let mut invalid = request("valid");
        invalid.color = color;
        assert!(matches!(
            prepare_text_appearance(&invalid),
            Err(PdfMasterError::InvalidOperation(_))
        ));
    }
}

#[test]
fn accepts_exact_geometry_style_and_color_boundaries() {
    let mut valid = request("A");
    valid.width = 14_400.0;
    valid.height = 14_400.0;
    valid.font_size = 1_000.0;
    valid.line_height = 4.0;
    valid.color = Color {
        r: 0.0,
        g: 1.0,
        b: 0.5,
    };
    assert!(prepare_text_appearance(&valid).is_ok());

    valid.font_size = 1.0;
    valid.line_height = 1.0;
    assert!(prepare_text_appearance(&valid).is_ok());
}

#[test]
fn physical_alignment_is_preserved_for_latin_and_rtl_paragraphs() {
    for text in [
        "physical alignment",
        "\u{2067}\u{0645}\u{0631}\u{062d}\u{0628}\u{0627} \u{0628}\u{0627}\u{0644}\u{0639}\u{0627}\u{0644}\u{0645}\u{2069}",
    ] {
        let offsets = [
            TextAlignment::Left,
            TextAlignment::Center,
            TextAlignment::Right,
        ]
        .map(|alignment| {
            let mut aligned = request(text);
            aligned.alignment = alignment;
            prepare_text_appearance(&aligned).unwrap().metrics().lines[0].offset
        });
        assert!(offsets[0].abs() < 0.001, "{text:?}: {offsets:?}");
        assert!(offsets[1] > offsets[0], "{text:?}: {offsets:?}");
        assert!(offsets[2] > offsets[1], "{text:?}: {offsets:?}");
    }
}

#[test]
fn preserves_hard_breaks_empty_paragraphs_and_source_byte_ranges() {
    let text = "  alpha\r\n\r\n中文  ";
    let prepared = prepare_text_appearance(&request(text)).unwrap();
    assert_eq!(prepared.metrics().lines.len(), 3);
    assert_eq!(
        prepared.metrics().lines.first().unwrap().text_range.start,
        0
    );
    assert_eq!(
        prepared.metrics().lines.last().unwrap().text_range.end,
        text.len()
    );
    for line in &prepared.metrics().lines {
        assert!(text.is_char_boundary(line.text_range.start));
        assert!(text.is_char_boundary(line.text_range.end));
    }
}

#[test]
fn coalesces_crlf_and_preserves_lf_cr_blank_and_trailing_paragraphs() {
    let text = "A\r\n\nB\r";
    let prepared = prepare_text_appearance(&request(text)).unwrap();
    let lines = &prepared.metrics().lines;
    assert_eq!(lines.len(), 4);
    assert_eq!(
        lines
            .iter()
            .map(|line| line.text_range.clone())
            .collect::<Vec<_>>(),
        vec![0..3, 3..4, 4..6, 6..6]
    );
    assert!(
        lines
            .windows(2)
            .all(|pair| pair[0].baseline < pair[1].baseline)
    );
    assert!((prepared.metrics().height - 4.0 * 18.0 * 1.3).abs() < 0.01);
}

#[test]
fn wraps_cjk_without_spaces_and_rejects_height_overflow() {
    let text = "中文段落排版必须在没有空格时也能真实换行";
    let mut narrow = request(text);
    narrow.width = 72.0;
    assert!(
        prepare_text_appearance(&narrow)
            .unwrap()
            .metrics()
            .lines
            .len()
            > 1
    );
    narrow.height = 10.0;
    assert!(matches!(
        prepare_text_appearance(&narrow),
        Err(PdfMasterError::InvalidGeometry(_))
    ));
}

#[test]
fn shapes_real_multiscript_paragraph_and_wraps_without_missing_glyphs() {
    let text = "Latin 中文 مرحبا بالعالم हैलो वर्ल्ड\nCafé e\u{301} office 𠮷";
    let narrow = prepare_text_appearance(&request(text)).unwrap();
    let mut wide_request = request(text);
    wide_request.width = 700.;
    let wide = prepare_text_appearance(&wide_request).unwrap();
    assert!(
        narrow.metrics().lines.len() > wide.metrics().lines.len(),
        "narrow={:?} wide={:?}",
        narrow.metrics(),
        wide.metrics()
    );
    assert_eq!(
        narrow.metrics().fonts,
        vec![
            BundledFont::Sans,
            BundledFont::CjkSc,
            BundledFont::Arabic,
            BundledFont::Devanagari
        ]
    );
    assert!(narrow.metrics().glyph_count > 0);
    assert!(narrow.pdf_bytes().starts_with(b"%PDF-"));
    assert!(narrow.metrics().height <= 700.);
}
