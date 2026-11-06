//! Validated layout preferences and the bounded simple reflow profile.
use crate::notes::resolve;
use crate::text_appearance::BundledFont;
use crate::{FreeTextAnnotation, FreeTextAppearanceStyle, PdfMasterError, Result};
use lopdf::{Dictionary, Document, Object, dictionary};

pub(crate) fn validate(style: &FreeTextAppearanceStyle) -> Result<()> {
    if !style.font_size.is_finite() || style.font_size <= 0. || style.font_size > 1000. {
        return Err(PdfMasterError::InvalidOperation("appearance font size"));
    }
    if !style.line_height.is_finite() || !(1. ..=4.).contains(&style.line_height) {
        return Err(PdfMasterError::InvalidOperation("appearance line height"));
    }
    crate::edit::validate_color(style.color)?;
    Ok(())
}
pub(crate) fn metadata(style: &FreeTextAppearanceStyle) -> Object {
    let font = match style.font {
        BundledFont::Sans => "Sans",
        BundledFont::CjkSc => "CjkSc",
        BundledFont::Arabic => "Arabic",
        BundledFont::Devanagari => "Devanagari",
    };
    dictionary! {"V"=>1,"Font"=>font,"LineHeight"=>style.line_height}.into()
}
pub(crate) fn read(
    document: &Document,
    annotation: &Dictionary,
    expected: &FreeTextAnnotation,
) -> Result<Option<FreeTextAppearanceStyle>> {
    let Ok(value) = annotation.get(b"PDFMasterAppearance") else {
        return Ok(None);
    };
    let invalid = || PdfMasterError::Malformed("invalid PDFMasterAppearance".into());
    let dict = resolve(document, value)?.as_dict().map_err(|_| invalid())?;
    let get = |key: &[u8]| -> Result<&Object> {
        resolve(document, dict.get(key).map_err(|_| invalid())?)
    };
    if get(b"V")?.as_i64().map_err(|_| invalid())? != 1 {
        return Err(invalid());
    }
    let font = match get(b"Font")?.as_name().map_err(|_| invalid())? {
        b"Sans" => BundledFont::Sans,
        b"CjkSc" => BundledFont::CjkSc,
        b"Arabic" => BundledFont::Arabic,
        b"Devanagari" => BundledFont::Devanagari,
        _ => return Err(invalid()),
    };
    let style = FreeTextAppearanceStyle {
        font,
        font_size: expected.style.font_size,
        color: expected.style.color,
        alignment: expected.style.alignment,
        line_height: get(b"LineHeight")?.as_float().map_err(|_| invalid())?,
    };
    validate(&style)?;
    Ok(Some(style))
}
pub(crate) fn validate_simple_profile(document: &Document, annotation: &Dictionary) -> Result<()> {
    let invalid =
        || PdfMasterError::InvalidOperation("complex FreeText appearance cannot be reflowed");
    for key in [b"C".as_slice(), b"IC"] {
        if let Ok(value) = annotation.get(key)
            && !resolve(document, value)?
                .as_array()
                .map_err(|_| invalid())?
                .is_empty()
        {
            return Err(invalid());
        }
    }
    for key in [b"CA".as_slice(), b"ca"] {
        if let Ok(value) = annotation.get(key)
            && resolve(document, value)?
                .as_float()
                .map_err(|_| invalid())?
                != 1.
        {
            return Err(invalid());
        }
    }
    if let Ok(value) = annotation.get(b"BM")
        && resolve(document, value)?.as_name().map_err(|_| invalid())? != b"Normal"
    {
        return Err(invalid());
    }
    if annotation.has(b"BE") {
        return Err(invalid());
    }
    if let Ok(value) = annotation.get(b"BS") {
        let bs = resolve(document, value)?.as_dict().map_err(|_| invalid())?;
        // An absent W has the PDF default width one, not zero.
        if resolve(document, bs.get(b"W").map_err(|_| invalid())?)?
            .as_float()
            .map_err(|_| invalid())?
            != 0.
        {
            return Err(invalid());
        }
    } else {
        // BS takes precedence over Border. Without BS, Border supplies the
        // effective width; if both are absent, PDF defaults that width to one.
        let value = annotation.get(b"Border").map_err(|_| invalid())?;
        let border = resolve(document, value)?
            .as_array()
            .map_err(|_| invalid())?;
        if border.len() != 3 {
            return Err(invalid());
        }
        for value in border {
            if resolve(document, value)?
                .as_float()
                .map_err(|_| invalid())?
                != 0.
            {
                return Err(invalid());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text_appearance::BundledFont;
    use crate::{Color, FreeTextStyle, Rect, TextAlignment};
    use lopdf::dictionary;
    fn style() -> FreeTextAppearanceStyle {
        FreeTextAppearanceStyle {
            font: BundledFont::Sans,
            font_size: 18.,
            color: Color {
                r: 1. / 7.,
                g: 2. / 7.,
                b: 3. / 7.,
            },
            alignment: TextAlignment::Left,
            line_height: 1.3,
        }
    }
    fn selected() -> FreeTextAnnotation {
        FreeTextAnnotation {
            page_index: 0,
            annotation_index: 0,
            page_id: (1, 0),
            object_id: Some((2, 0)),
            bounds: Rect {
                x: 0.,
                y: 0.,
                width: 100.,
                height: 100.,
            },
            contents: "text".into(),
            style: FreeTextStyle {
                font_resource: b"f0".to_vec(),
                font_size: 18.,
                color: style().color,
                alignment: TextAlignment::Left,
            },
            flags: 4,
            revision: [0; 32],
        }
    }
    #[test]
    fn free_text_style_rejects_invalid_size_height_and_each_rgb_component() {
        assert!(validate(&style()).is_ok());
        let mut rejected = Vec::new();
        for value in [0., -1., 1000.1, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut value_style = style();
            value_style.font_size = value;
            rejected.push(validate(&value_style).is_err());
        }
        for value in [0.99, 4.01, f32::NAN, f32::INFINITY] {
            let mut value_style = style();
            value_style.line_height = value;
            rejected.push(validate(&value_style).is_err());
        }
        for value in [-0.1, 1.1, f32::NAN, f32::INFINITY] {
            for channel in 0..3 {
                let mut value_style = style();
                match channel {
                    0 => value_style.color.r = value,
                    1 => value_style.color.g = value,
                    _ => value_style.color.b = value,
                };
                rejected.push(validate(&value_style).is_err());
            }
        }
        assert_eq!(rejected, vec![true; 22]);
        for (size, height) in [(0.001, 1.), (1000., 4.)] {
            let mut valid = style();
            valid.font_size = size;
            valid.line_height = height;
            assert!(validate(&valid).is_ok());
        }
    }
    #[test]
    fn free_text_metadata_roundtrips_four_preferences_but_uses_actual_da_q_projection() {
        let doc = Document::with_version("1.7");
        for font in [
            BundledFont::Sans,
            BundledFont::CjkSc,
            BundledFont::Arabic,
            BundledFont::Devanagari,
        ] {
            let mut value = style();
            value.font = font;
            let mut expected = selected();
            expected.style.font_size = 31.;
            expected.style.color = Color {
                r: 0.2,
                g: 0.3,
                b: 0.4,
            };
            expected.style.alignment = TextAlignment::Right;
            let annotation = dictionary! {"PDFMasterAppearance"=>metadata(&value)};
            let mut desired = value;
            desired.font_size = 31.;
            desired.color = expected.style.color;
            desired.alignment = TextAlignment::Right;
            assert_eq!(read(&doc, &annotation, &expected).unwrap(), Some(desired));
        }
        assert_eq!(read(&doc, &dictionary! {}, &selected()).unwrap(), None);
    }
    #[test]
    fn free_text_metadata_rejects_reference_types_versions_fonts_and_line_heights() {
        let doc = Document::with_version("1.7");
        let bad = [
            Object::Reference((9, 0)),
            Object::Null,
            Object::Integer(1),
            dictionary! {"V"=>2,"Font"=>"Sans","LineHeight"=>1.3}.into(),
            dictionary! {"V"=>1,"Font"=>"Serif","LineHeight"=>1.3}.into(),
            dictionary! {"V"=>1,"Font"=>Object::Reference((9,0)),"LineHeight"=>1.3}.into(),
            dictionary! {"V"=>1,"Font"=>"Sans","LineHeight"=>Object::Reference((9,0))}.into(),
            dictionary! {"V"=>1,"Font"=>"Sans","LineHeight"=>0.9}.into(),
            dictionary! {"V"=>1,"Font"=>"Sans","LineHeight"=>4.1}.into(),
            dictionary! {"V"=>"1","Font"=>"Sans","LineHeight"=>1.3}.into(),
            dictionary! {"V"=>1,"Font"=>"Sans"}.into(),
        ];
        let rejected: Vec<_> = bad
            .into_iter()
            .map(|value| {
                read(
                    &doc,
                    &dictionary! {"PDFMasterAppearance"=>value},
                    &selected(),
                )
                .is_err()
            })
            .collect();
        assert_eq!(rejected, vec![true; 11]);
    }
    #[test]
    fn free_text_metadata_accepts_bounded_indirect_values_and_rejects_missing_cycles_depth() {
        let mut doc = Document::with_version("1.7");
        let version = doc.add_object(1);
        let font = doc.add_object(Object::Name(b"Arabic".to_vec()));
        let height = doc.add_object(Object::Real(1.7));
        let preference =
            doc.add_object(dictionary! {"V"=>version,"Font"=>font,"LineHeight"=>height});
        let annotation = dictionary! {"PDFMasterAppearance"=>preference};
        let mut expected_style = style();
        expected_style.font = BundledFont::Arabic;
        expected_style.line_height = 1.7;
        assert_eq!(
            read(&doc, &annotation, &selected()).unwrap(),
            Some(expected_style)
        );
    }
    #[test]
    fn free_text_metadata_rejects_missing_cyclic_and_overdeep_indirect_values() {
        let mut doc = Document::with_version("1.7");
        doc.objects.insert((1, 0), Object::Reference((2, 0)));
        doc.objects.insert((2, 0), Object::Reference((1, 0)));
        for index in 100..166 {
            doc.objects
                .insert((index, 0), Object::Reference((index + 1, 0)));
        }
        doc.objects.insert(
            (166, 0),
            dictionary! {"V"=>1,"Font"=>"Sans","LineHeight"=>1.3}.into(),
        );
        let rejected: Vec<_> = [(999, 0), (1, 0), (100, 0)]
            .into_iter()
            .map(|id| {
                read(
                    &doc,
                    &dictionary! {"PDFMasterAppearance"=>Object::Reference(id)},
                    &selected(),
                )
                .is_err()
            })
            .collect();
        assert_eq!(rejected, vec![true; 3]);
    }
    #[test]
    fn free_text_profile_rejects_complex_visuals_and_accepts_only_simple_defaults() {
        let mut doc = Document::with_version("1.7");
        let indirect_bs = doc.add_object(dictionary! {"W"=>0});
        let indirect_border = doc.add_object(vec![Object::Integer(0); 3]);
        let accepted = [
            dictionary! {"Border"=>vec![0.into(),0.into(),0.into()]},
            dictionary! {"BS"=>dictionary!{"W"=>0}},
            dictionary! {"BS"=>dictionary!{"W"=>0},"Border"=>vec![0.into(),0.into(),9.into()]},
            dictionary! {"BS"=>indirect_bs,"Border"=>"ignored"},
            dictionary! {"Border"=>indirect_border},
            dictionary! {"C"=>Vec::<Object>::new(),"IC"=>Vec::<Object>::new(),"CA"=>1,"ca"=>1,"BM"=>"Normal","Border"=>vec![0.into(),0.into(),0.into()]},
        ];
        let accepted: Vec<_> = accepted
            .iter()
            .map(|value| validate_simple_profile(&doc, value).is_ok())
            .collect();
        assert_eq!(accepted, vec![true; 6]);
        let bad = [
            dictionary! {},
            dictionary! {"C"=>vec![1.into(),0.into(),0.into()]},
            dictionary! {"IC"=>vec![1.into()]},
            dictionary! {"C"=>"bad"},
            dictionary! {"CA"=>0.9},
            dictionary! {"ca"=>0.9},
            dictionary! {"BM"=>"Multiply"},
            dictionary! {"BE"=>dictionary!{"S"=>"C"}},
            dictionary! {"BS"=>dictionary!{}},
            dictionary! {"BS"=>dictionary!{"W"=>1}},
            dictionary! {"BS"=>"bad"},
            dictionary! {"BS"=>Object::Reference((999,0))},
            dictionary! {"Border"=>vec![0.into(),0.into(),1.into()]},
            dictionary! {"Border"=>vec![1.into(),0.into(),0.into()]},
            dictionary! {"Border"=>vec![0.into(),0.into()]},
            dictionary! {"Border"=>"bad"},
        ];
        let rejected: Vec<_> = bad
            .iter()
            .map(|value| validate_simple_profile(&doc, value).is_err())
            .collect();
        assert_eq!(rejected, vec![true; 16]);
    }
}
