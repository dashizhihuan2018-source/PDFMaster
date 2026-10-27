//! Synthetic media only. Source-sized conversion is distinct from explicit
//! paper layouts and the mixed-file importer's all-frame compatibility mode.
use pdfmaster_core::{ImageImportLayout, Session, prepare_image_imports};
use std::io::Cursor;

fn source() -> ImageImportLayout {
    serde_json::from_str(r#"{"paper":"source","landscape":false,"margin":0}"#).unwrap()
}
fn size(name: &str, bytes: &[u8]) -> (usize, f64, f64) {
    let pdf = prepare_image_imports(&[(name, bytes)], source()).unwrap();
    let info = Session::open(&pdf[0]).unwrap().info();
    (
        info.page_count,
        info.pages[0].width.into(),
        info.pages[0].height.into(),
    )
}
fn png(density: Option<(u32, u32, u8)>) -> Vec<u8> {
    png_sized(8, 4, density)
}
fn png_sized(width: u32, height: u32, density: Option<(u32, u32, u8)>) -> Vec<u8> {
    let mut bytes = vec![];
    let mut e = png::Encoder::new(&mut bytes, width, height);
    e.set_color(png::ColorType::Rgb);
    let mut w = e.write_header().unwrap();
    if let Some((x, y, unit)) = density {
        let mut data = x.to_be_bytes().to_vec();
        data.extend(y.to_be_bytes());
        data.push(unit);
        w.write_chunk(png::chunk::pHYs, &data).unwrap();
    }
    w.write_image_data(&vec![128; width as usize * height as usize * 3])
        .unwrap();
    drop(w);
    bytes
}
fn jpeg(jfif: Option<(u8, u16, u16)>, exif: Option<(u32, u32)>) -> Vec<u8> {
    let mut bytes = vec![];
    image::codecs::jpeg::JpegEncoder::new(&mut bytes)
        .encode(&[128; 96], 8, 4, image::ExtendedColorType::Rgb8)
        .unwrap();
    // The encoder's single JFIF APP0 is replaced, not duplicated.
    let end = 4 + u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
    assert_eq!(&bytes[6..11], b"JFIF\0");
    bytes.drain(2..end);
    let mut metadata = vec![];
    if let Some((unit, x, y)) = jfif {
        let mut data = b"JFIF\0\x01\x02".to_vec();
        data.push(unit);
        data.extend(x.to_be_bytes());
        data.extend(y.to_be_bytes());
        data.extend([0, 0]);
        metadata.extend([255, 224]);
        metadata.extend(((data.len() + 2) as u16).to_be_bytes());
        metadata.extend(data);
    }
    if let Some((x, y)) = exif {
        let mut data = b"Exif\0\0II\x2a\0\x08\0\0\0".to_vec();
        data.extend(3u16.to_le_bytes());
        for (tag, ty, value) in [(282u16, 5u16, 50u32), (283, 5, 58), (296, 3, 2)] {
            data.extend(tag.to_le_bytes());
            data.extend(ty.to_le_bytes());
            data.extend(1u32.to_le_bytes());
            data.extend(value.to_le_bytes());
        }
        data.extend(0u32.to_le_bytes());
        for n in [x, 1, y, 1] {
            data.extend(n.to_le_bytes());
        }
        metadata.extend([255, 225]);
        metadata.extend(((data.len() + 2) as u16).to_be_bytes());
        metadata.extend(data);
    }
    bytes.splice(2..2, metadata);
    bytes
}
#[test]
fn source_without_absolute_density_has_no_paper_or_margin() {
    assert_eq!(size("plain.png", &png(None)), (1, 8., 4.));
    assert_eq!(size("ratio.png", &png(Some((144, 288, 0)))), (1, 8., 4.));
    assert_eq!(
        size("ratio.jpg", &jpeg(Some((0, 144, 288)), None)),
        (1, 8., 4.)
    );
}
#[test]
fn source_png_uses_rounded_horizontal_dpi_for_both_axes() {
    assert_eq!(
        size("density.png", &png(Some((5669, 11338, 1)))),
        (1, 4., 2.)
    );
    let (_, w, h) = size("rounded.png", &png(Some((5700, 11400, 1))));
    assert!((w - 8. * 72. / 145.).abs() < 0.00001);
    assert!((h - 4. * 72. / 145.).abs() < 0.00001);
}
#[test]
fn source_jpeg_uses_jfif_units_and_exif_precedence() {
    assert_eq!(
        size("inch.jpg", &jpeg(Some((1, 144, 288)), None)),
        (1, 4., 2.)
    );
    let (_, w, h) = size("cm.jpg", &jpeg(Some((2, 100, 200)), None));
    assert!((w - 8. * 72. / 254.).abs() < 0.00001);
    assert!((h - 4. * 72. / 254.).abs() < 0.00001);
    let (_, w, h) = size("fractional-cm.jpg", &jpeg(Some((2, 101, 202)), None));
    let dpi = f64::from(256.54f32);
    assert!((w - 8. * 72. / dpi).abs() < 0.00001);
    assert!((h - 4. * 72. / dpi).abs() < 0.00001);
    assert_eq!(
        size("conflict.jpg", &jpeg(Some((1, 300, 600)), Some((144, 288)))),
        (1, 4., 2.)
    );
}
#[test]
fn source_tiff_uses_only_first_oriented_frame_and_horizontal_density() {
    use tiff::{
        encoder::{Rational, TiffEncoder, colortype::RGB8},
        tags::Tag,
    };
    let mut bytes = Cursor::new(vec![]);
    {
        let mut e = TiffEncoder::new(&mut bytes).unwrap();
        let mut first = e.new_image::<RGB8>(8, 4).unwrap();
        first.encoder().write_tag(Tag::Orientation, 6u16).unwrap();
        first
            .encoder()
            .write_tag(Tag::ResolutionUnit, 2u16)
            .unwrap();
        first
            .encoder()
            .write_tag(Tag::XResolution, Rational { n: 144, d: 1 })
            .unwrap();
        first
            .encoder()
            .write_tag(Tag::YResolution, Rational { n: 288, d: 1 })
            .unwrap();
        first.write_data(&[128; 96]).unwrap();
        e.write_image::<RGB8>(2, 3, &[64; 18]).unwrap();
    }
    assert_eq!(size("two.tiff", bytes.get_ref()), (1, 2., 4.));
}
#[test]
fn source_invalid_geometry_rejects_the_entire_batch() {
    let good = png(None);
    let mut zero = jpeg(None, Some((144, 288)));
    // Exif XResolution denominator zero is malformed, not a valid fallback.
    let t = 12; // SOI + APP1 header + Exif prefix, no JFIF.
    assert_eq!(&zero[t..t + 2], b"II");
    zero[t + 54..t + 58].fill(0);
    assert!(prepare_image_imports(&[("good.png", &good), ("zero.jpg", &zero)], source()).is_err());
    let huge = jpeg(None, Some((1, 144)));
    // 8 pixels at 1dpi remains safe; large physical sizes are checked separately
    // by the same PDF dimension budget, never silently fitted to A4.
    assert_eq!(size("one.jpg", &huge), (1, 576., 288.));
}

#[test]
fn source_density_is_validated_and_full_page_drawing_preserves_pixels() {
    let good = png(Some((5669, 11338, 1)));
    let pdf = prepare_image_imports(&[("density.png", &good)], source()).unwrap();
    let doc = lopdf::Document::load_mem(&pdf[0]).unwrap();
    let page = *doc.get_pages().values().next().unwrap();
    let content = String::from_utf8(doc.get_page_content(page)).unwrap();
    assert!(
        content.contains("4 0 0 2 0 0 cm"),
        "no fit margin: {content}"
    );
    let image = doc
        .objects
        .values()
        .filter_map(|o| o.as_stream().ok())
        .find(|s| s.dict.get(b"Subtype").and_then(lopdf::Object::as_name).ok() == Some(b"Image"))
        .unwrap();
    assert_eq!(image.get_plain_content().unwrap(), [128; 96]);
    // Duplicate/unknown pHYs and corrupt CRC must never produce a partial batch.
    let mut duplicate = good.clone();
    let length = 21; // pHYs: length + type + 9 data + CRC, after IHDR (33).
    duplicate.splice(33..33, good[33..33 + length].iter().copied());
    let unknown = png(Some((5669, 11338, 2)));
    let mut corrupt = good.clone();
    corrupt[45] ^= 1;
    for bad in [duplicate, unknown, corrupt] {
        assert!(
            prepare_image_imports(&[("good.png", &good), ("bad.png", &bad)], source()).is_err()
        );
    }
}

#[test]
fn source_tiff_checks_declared_density_types_even_on_unused_axis_or_ratio() {
    use tiff::{
        encoder::{Rational, TiffEncoder, colortype::RGB8},
        tags::Tag,
    };
    for unit in [1u16, 2] {
        for wrong_type in [false, true] {
            let mut bytes = Cursor::new(vec![]);
            {
                let mut e = TiffEncoder::new(&mut bytes).unwrap();
                let mut first = e.new_image::<RGB8>(8, 4).unwrap();
                first
                    .encoder()
                    .write_tag(Tag::ResolutionUnit, unit)
                    .unwrap();
                first
                    .encoder()
                    .write_tag(Tag::XResolution, Rational { n: 144, d: 1 })
                    .unwrap();
                if wrong_type {
                    first.encoder().write_tag(Tag::YResolution, 288u16).unwrap();
                } else {
                    first
                        .encoder()
                        .write_tag(Tag::YResolution, Rational { n: 288, d: 0 })
                        .unwrap();
                }
                first.write_data(&[128; 96]).unwrap();
            }
            assert!(
                prepare_image_imports(&[("malformed.tif", bytes.get_ref())], source()).is_err(),
                "malformed YResolution must not be hidden by horizontal scaling: unit={unit}, wrong_type={wrong_type}"
            );
        }
    }
}

#[test]
fn source_physical_size_budget_does_not_fit_oversize_images_to_paper() {
    let good = png(None);
    let boundary = png_sized(20_000, 1, Some((3937, 3937, 1))); // 100dpi
    assert_eq!(size("boundary.png", &boundary).1, 14_400.);
    let overflow = png_sized(20_000, 1, Some((3898, 3898, 1))); // 99dpi
    assert!(
        prepare_image_imports(
            &[("good.png", &good), ("overflow.png", &overflow)],
            source()
        )
        .is_err()
    );
    let mut bytes = Cursor::new(vec![]);
    {
        use tiff::{
            encoder::{Rational, TiffEncoder, colortype::RGB8},
            tags::Tag,
        };
        let mut e = TiffEncoder::new(&mut bytes).unwrap();
        let mut first = e.new_image::<RGB8>(20_000, 1).unwrap();
        first.encoder().write_tag(Tag::Orientation, 6u16).unwrap();
        first
            .encoder()
            .write_tag(Tag::ResolutionUnit, 2u16)
            .unwrap();
        first
            .encoder()
            .write_tag(Tag::XResolution, Rational { n: 99, d: 1 })
            .unwrap();
        first.write_data(&vec![128; 60_000]).unwrap();
    }
    assert!(
        prepare_image_imports(
            &[
                ("good.png", &good),
                ("rotated-overflow.tif", bytes.get_ref())
            ],
            source()
        )
        .is_err()
    );
}

#[test]
fn source_exif_relative_and_single_axis_match_system_metadata_fallback() {
    let base = jpeg(Some((1, 300, 600)), Some((144, 288)));
    // Exif payload starts after SOI + 18-byte JFIF + 4-byte APP1 header + 6-byte prefix.
    let t = 30;
    assert_eq!(&base[t..t + 2], b"II");
    let mut relative = base.clone();
    relative[t + 42..t + 44].copy_from_slice(&1u16.to_le_bytes());
    assert_eq!(size("relative-exif.jpg", &relative), (1, 4., 2.));
    let mut only_y = base.clone();
    only_y[t + 10..t + 12].copy_from_slice(&65000u16.to_le_bytes());
    assert_eq!(size("only-y.jpg", &only_y), (1, 2., 1.));
}

#[test]
fn source_exif_zero_density_and_centimeter_rounding_follow_imageio() {
    assert_eq!(
        size("zero.jpg", &jpeg(Some((1, 300, 600)), Some((0, 288)))),
        (1, 8., 4.)
    );
    let mut centimeter = jpeg(None, Some((144, 288)));
    let t = 12;
    centimeter[t + 42..t + 44].copy_from_slice(&3u16.to_le_bytes());
    let (_, w, h) = size("cm-exif.jpg", &centimeter);
    assert!((w - 8. * 72. / 366.).abs() < 0.00001);
    assert!((h - 4. * 72. / 366.).abs() < 0.00001);
    let mut fractional = jpeg(None, Some((1441, 2881)));
    for offset in [t + 54, t + 62] {
        fractional[offset..offset + 4].copy_from_slice(&10u32.to_le_bytes());
    }
    let (_, w, h) = size("fractional-exif.jpg", &fractional);
    let dpi = f64::from(144.1f32);
    assert!((w - 8. * 72. / dpi).abs() < 0.00001);
    assert!((h - 4. * 72. / dpi).abs() < 0.00001);
}

#[test]
fn source_centimeter_zero_density_falls_back_before_unit_conversion() {
    let mut jpeg = jpeg(None, Some((0, 288)));
    let jpeg_tiff = 12;
    jpeg[jpeg_tiff + 42..jpeg_tiff + 44].copy_from_slice(&3u16.to_le_bytes());
    let (_, w, h) = size("zero-cm.jpg", &jpeg);
    assert!((w - 8. * 72. / 183.).abs() < 0.00001);
    assert!((h - 4. * 72. / 183.).abs() < 0.00001);

    use tiff::{
        encoder::{Rational, TiffEncoder, colortype::RGB8},
        tags::Tag,
    };
    let mut bytes = Cursor::new(vec![]);
    {
        let mut encoder = TiffEncoder::new(&mut bytes).unwrap();
        let mut image = encoder.new_image::<RGB8>(8, 4).unwrap();
        image
            .encoder()
            .write_tag(Tag::ResolutionUnit, 3u16)
            .unwrap();
        image
            .encoder()
            .write_tag(Tag::XResolution, Rational { n: 0, d: 1 })
            .unwrap();
        image
            .encoder()
            .write_tag(Tag::YResolution, Rational { n: 288, d: 1 })
            .unwrap();
        image.write_data(&[128; 96]).unwrap();
    }
    let (_, w, h) = size("zero-cm.tiff", bytes.get_ref());
    assert!((w - 8. * 72. / 183.).abs() < 0.00001);
    assert!((h - 4. * 72. / 183.).abs() < 0.00001);
}

#[test]
fn source_tiff_relative_single_axis_and_centimeter_metadata_follow_imageio() {
    use tiff::{
        encoder::{Rational, TiffEncoder, colortype::RGB8},
        tags::Tag,
    };
    for (unit, only_y, x, expected) in [
        (1, false, 144, 144.),
        (2, true, 144, 288.),
        (3, false, 144, 366.),
        (2, false, 0, 72.),
    ] {
        let mut bytes = Cursor::new(vec![]);
        {
            let mut e = TiffEncoder::new(&mut bytes).unwrap();
            let mut p = e.new_image::<RGB8>(8, 4).unwrap();
            p.encoder()
                .write_tag(Tag::ResolutionUnit, unit as u16)
                .unwrap();
            p.encoder()
                .write_tag(Tag::XResolution, Rational { n: x, d: 1 })
                .unwrap();
            p.encoder()
                .write_tag(Tag::YResolution, Rational { n: 288, d: 1 })
                .unwrap();
            p.write_data(&[128; 96]).unwrap();
        }
        let mut data = bytes.into_inner();
        if only_y {
            let ifd = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
            let count = u16::from_le_bytes(data[ifd..ifd + 2].try_into().unwrap()) as usize;
            for offset in (ifd + 2..ifd + 2 + count * 12).step_by(12) {
                if data[offset..offset + 2] == 282u16.to_le_bytes() {
                    data[offset..offset + 2].copy_from_slice(&65000u16.to_le_bytes());
                }
            }
        }
        let (_, w, h) = size("density.tif", &data);
        assert!(
            (w - 8. * 72. / expected).abs() < 0.00001,
            "unit{unit}, only_y{only_y}, x{x}: {w}"
        );
        assert!((h - 4. * 72. / expected).abs() < 0.00001);
    }
}
