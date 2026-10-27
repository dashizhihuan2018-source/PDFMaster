//! Self-owned synthetic PNG/eXIf fixtures. No external probe paths are used.
use pdfmaster_core::{ImageImportLayout, Session, prepare_image_imports};

fn source() -> ImageImportLayout {
    serde_json::from_str(r#"{"paper":"source","landscape":false,"margin":0}"#).unwrap()
}

fn tiff_exif(
    orientation: u16,
    x: Option<(u32, u32)>,
    y: Option<(u32, u32)>,
    unit: Option<u16>,
    little: bool,
) -> Vec<u8> {
    let u16_bytes = |value: u16| {
        if little {
            value.to_le_bytes()
        } else {
            value.to_be_bytes()
        }
    };
    let u32_bytes = |value: u32| {
        if little {
            value.to_le_bytes()
        } else {
            value.to_be_bytes()
        }
    };
    let mut entries = Vec::new();
    if orientation != 0 {
        entries.push((0x112u16, 3u16, 1u32, u32::from(orientation)));
    }
    let count = usize::from(orientation != 0)
        + usize::from(x.is_some())
        + usize::from(y.is_some())
        + usize::from(unit.is_some());
    let values = 8 + 2 + count * 12 + 4;
    let mut next_value = values as u32;
    if x.is_some() {
        entries.push((282, 5, 1, next_value));
        next_value += 8;
    }
    if y.is_some() {
        entries.push((283, 5, 1, next_value));
    }
    if let Some(unit) = unit {
        entries.push((296, 3, 1, u32::from(unit)));
    }
    entries.sort_by_key(|e| e.0);

    let mut out = if little {
        b"II".to_vec()
    } else {
        b"MM".to_vec()
    };
    out.extend(u16_bytes(42));
    out.extend(u32_bytes(8));
    out.extend(u16_bytes(entries.len() as u16));
    for (tag, ty, n, value) in entries {
        out.extend(u16_bytes(tag));
        out.extend(u16_bytes(ty));
        out.extend(u32_bytes(n));
        if ty == 3 {
            out.extend(u16_bytes(value as u16));
            out.extend([0; 2]);
        } else {
            out.extend(u32_bytes(value));
        }
    }
    out.extend(u32_bytes(0));
    for rational in [x, y].into_iter().flatten() {
        out.extend(u32_bytes(rational.0));
        out.extend(u32_bytes(rational.1));
    }
    out
}

fn fixture_png_ordered(
    exif: Option<&[u8]>,
    phys: Option<(u32, u32, u8)>,
    phys_first: bool,
) -> (Vec<u8>, Vec<u8>) {
    let (width, height) = (8u32, 4u32);
    let rgb: Vec<u8> = (0..height)
        .flat_map(|y| {
            (0..width).flat_map(move |x| {
                [
                    (x * 17 + y * 3) as u8,
                    (y * 51 + x * 2) as u8,
                    (x * 11 + y * 29) as u8,
                ]
            })
        })
        .collect();
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    let write_phys = |writer: &mut png::Writer<&mut Vec<u8>>| {
        if let Some((x, y, unit)) = phys {
            let mut data = x.to_be_bytes().to_vec();
            data.extend(y.to_be_bytes());
            data.push(unit);
            writer.write_chunk(png::chunk::pHYs, &data).unwrap();
        }
    };
    if phys_first {
        write_phys(&mut writer);
    }
    if let Some(exif) = exif {
        writer
            .write_chunk(png::chunk::ChunkType(*b"eXIf"), exif)
            .unwrap();
    }
    if !phys_first {
        write_phys(&mut writer);
    }
    writer.write_image_data(&rgb).unwrap();
    drop(writer);
    (bytes, rgb)
}

fn fixture_png(exif: Option<&[u8]>, phys: Option<(u32, u32, u8)>) -> (Vec<u8>, Vec<u8>) {
    fixture_png_ordered(exif, phys, false)
}

fn embedded_rgb(pdf: &[u8]) -> (usize, usize, Vec<u8>) {
    let doc = lopdf::Document::load_mem(pdf).unwrap();
    let image = doc
        .objects
        .values()
        .filter_map(|object| object.as_stream().ok())
        .find(|stream| {
            stream
                .dict
                .get(b"Subtype")
                .and_then(lopdf::Object::as_name)
                .ok()
                == Some(b"Image")
        })
        .unwrap();
    (
        image.dict.get(b"Width").unwrap().as_i64().unwrap() as usize,
        image.dict.get(b"Height").unwrap().as_i64().unwrap() as usize,
        image.get_plain_content().unwrap(),
    )
}

fn oriented_expected(source: &[u8], orientation: u16) -> (usize, usize, Vec<u8>) {
    let (width, height) = (8usize, 4usize);
    let (out_width, out_height) = if orientation >= 5 {
        (height, width)
    } else {
        (width, height)
    };
    let mut out = vec![0; source.len()];
    for source_y in 0..height {
        for source_x in 0..width {
            let (x, y) = match orientation {
                1 => (source_x, source_y),
                2 => (width - 1 - source_x, source_y),
                3 => (width - 1 - source_x, height - 1 - source_y),
                4 => (source_x, height - 1 - source_y),
                5 => (source_y, source_x),
                6 => (height - 1 - source_y, source_x),
                7 => (height - 1 - source_y, width - 1 - source_x),
                8 => (source_y, width - 1 - source_x),
                _ => unreachable!(),
            };
            let from = (source_y * width + source_x) * 3;
            let to = (y * out_width + x) * 3;
            out[to..to + 3].copy_from_slice(&source[from..from + 3]);
        }
    }
    (out_width, out_height, out)
}

fn page_size(input: &[u8]) -> (f64, f64) {
    let output = prepare_image_imports(&[("own.png", input)], source()).unwrap();
    let info = Session::open(&output[0]).unwrap().info();
    (info.pages[0].width.into(), info.pages[0].height.into())
}

fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = (data.len() as u32).to_be_bytes().to_vec();
    out.extend(kind);
    out.extend(data);
    out.extend(crc32fast::hash(&out[4..]).to_be_bytes());
    out
}

fn exif_color_space(value: u16) -> Vec<u8> {
    let mut out = b"II\x2a\0\x08\0\0\0\x01\0".to_vec();
    out.extend(0x8769u16.to_le_bytes());
    out.extend(4u16.to_le_bytes());
    out.extend(1u32.to_le_bytes());
    out.extend(26u32.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(0xa001u16.to_le_bytes());
    out.extend(3u16.to_le_bytes());
    out.extend(1u32.to_le_bytes());
    out.extend(value.to_le_bytes());
    out.extend([0; 2]);
    out.extend(0u32.to_le_bytes());
    out
}

#[test]
fn source_png_exif_orientation_and_density_are_applied() {
    let exif = tiff_exif(6, Some((144, 1)), Some((288, 1)), Some(2), true);
    let (input, source_rgb) = fixture_png(Some(&exif), Some((11_811, 11_811, 1)));
    let output = prepare_image_imports(&[("own.png", &input)], source()).unwrap();
    let info = Session::open(&output[0]).unwrap().info();
    assert_eq!((info.pages[0].width, info.pages[0].height), (2., 4.));
    assert_eq!(embedded_rgb(&output[0]), oriented_expected(&source_rgb, 6));
}

#[test]
fn png_exif_both_endiannesses_apply_all_eight_orientations_to_pixels() {
    for little in [false, true] {
        for orientation in 1..=8 {
            let exif = tiff_exif(orientation, None, None, None, little);
            let (input, source_rgb) = fixture_png(Some(&exif), None);
            let output = prepare_image_imports(
                &[("oriented.png", &input)],
                serde_json::from_str(r#"{"paper":"pixels","landscape":false,"margin":0}"#).unwrap(),
            )
            .unwrap();
            assert_eq!(
                embedded_rgb(&output[0]),
                oriented_expected(&source_rgb, orientation),
                "orientation={orientation}, little={little}"
            );
        }
    }
}

#[test]
fn png_exif_density_precedes_phys_in_either_chunk_order_and_uses_single_axes() {
    for phys_first in [false, true] {
        let exif = tiff_exif(1, Some((144, 1)), Some((288, 1)), Some(2), true);
        let (input, _) = fixture_png_ordered(Some(&exif), Some((11_811, 11_811, 1)), phys_first);
        assert_eq!(page_size(&input), (4., 2.));
    }
    for (x, y, expected_width) in [(Some((144, 1)), None, 4.), (None, Some((288, 1)), 2.)] {
        let exif = tiff_exif(1, x, y, None, false);
        let (input, _) = fixture_png(Some(&exif), Some((11_811, 11_811, 1)));
        assert_eq!(page_size(&input).0, expected_width);
    }
}

#[test]
fn png_exif_without_density_keeps_phys_and_cm_zero_converts_after_fallback() {
    for exif in [
        tiff_exif(1, None, None, None, true),
        tiff_exif(0, None, None, Some(3), true),
        tiff_exif(0, None, None, None, true),
    ] {
        let (input, _) = fixture_png(Some(&exif), Some((11_811, 11_811, 1)));
        let (w, h) = page_size(&input);
        assert!((w - 8. * 72. / 300.).abs() < 0.00001);
        assert!((h - 4. * 72. / 300.).abs() < 0.00001);
    }
    let cm = tiff_exif(1, Some((144, 1)), None, Some(3), true);
    let (input, _) = fixture_png(Some(&cm), None);
    let (w, h) = page_size(&input);
    assert!((w - 8. * 72. / 366.).abs() < 0.00001);
    assert!((h - 4. * 72. / 366.).abs() < 0.00001);

    let zero_cm = tiff_exif(1, Some((0, 1)), Some((288, 1)), Some(3), true);
    let (input, _) = fixture_png(Some(&zero_cm), Some((11_811, 11_811, 1)));
    let (w, h) = page_size(&input);
    assert!((w - 8. * 72. / 183.).abs() < 0.00001);
    assert!((h - 4. * 72. / 183.).abs() < 0.00001);
}

#[test]
fn malformed_duplicate_late_and_corrupt_png_exif_fail_the_whole_batch() {
    let valid_exif = tiff_exif(1, Some((144, 1)), None, Some(2), true);
    let (good, _) = fixture_png(None, None);
    let (valid, _) = fixture_png(Some(&valid_exif), None);

    let mut bad_offset = valid_exif.clone();
    bad_offset[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    let mut bad_type = valid_exif.clone();
    bad_type[12..14].copy_from_slice(&4u16.to_le_bytes());
    let mut bad_count = valid_exif.clone();
    bad_count[14..18].copy_from_slice(&2u32.to_le_bytes());
    let mut denominator_zero = valid_exif.clone();
    let rational = denominator_zero.len() - 8;
    denominator_zero[rational + 4..rational + 8].fill(0);
    let cycle = b"II\x2a\0\x08\0\0\0\0\0\x08\0\0\0".to_vec();
    assert_eq!(cycle.len(), 14);

    let mut duplicate_tag = b"II\x2a\0\x08\0\0\0\x02\0".to_vec();
    for _ in 0..2 {
        duplicate_tag.extend(0x112u16.to_le_bytes());
        duplicate_tag.extend(3u16.to_le_bytes());
        duplicate_tag.extend(1u32.to_le_bytes());
        duplicate_tag.extend(1u16.to_le_bytes());
        duplicate_tag.extend([0; 2]);
    }
    duplicate_tag.extend(0u32.to_le_bytes());

    let mut too_many_entries = b"II\x2a\0\x08\0\0\0".to_vec();
    too_many_entries.extend(513u16.to_le_bytes());
    too_many_entries.resize(8 + 2 + 513 * 12 + 4, 0);

    let mut too_many_ifds = b"II\x2a\0\x08\0\0\0".to_vec();
    for index in 0..9u32 {
        too_many_ifds.extend(0u16.to_le_bytes());
        too_many_ifds.extend(if index == 8 {
            0u32.to_le_bytes()
        } else {
            (14 + index * 6).to_le_bytes()
        });
    }

    let mut cases = Vec::new();
    for exif in [
        bad_offset,
        bad_type,
        bad_count,
        denominator_zero,
        cycle,
        duplicate_tag,
        too_many_entries,
        too_many_ifds,
    ] {
        cases.push(fixture_png(Some(&exif), None).0);
    }
    let mut corrupt_crc = valid.clone();
    corrupt_crc[45] ^= 1;
    cases.push(corrupt_crc);

    let mut duplicate = valid.clone();
    duplicate.splice(33..33, chunk(b"eXIf", &valid_exif));
    cases.push(duplicate);

    let mut late = good.clone();
    let before_iend = late.len() - 12;
    late.splice(before_iend..before_iend, chunk(b"eXIf", &valid_exif));
    cases.push(late);

    for (index, bad) in cases.iter().enumerate() {
        assert!(
            prepare_image_imports(&[("good.png", &good), ("bad.png", bad)], source()).is_err(),
            "case {index}"
        );
    }
}

#[test]
fn png_exif_unknown_color_declaration_is_not_treated_as_decoder_default_srgb() {
    let declared_srgb = exif_color_space(1);
    let (valid, _) = fixture_png(Some(&declared_srgb), None);
    assert!(prepare_image_imports(&[("srgb-exif.png", &valid)], source()).is_ok());

    let unknown = exif_color_space(2);
    let (bad, _) = fixture_png(Some(&unknown), None);
    assert!(prepare_image_imports(&[("unknown-exif.png", &bad)], source()).is_err());
}

#[test]
fn png_exif_per_chunk_and_shared_batch_budgets_are_enforced() {
    let mut oversized = tiff_exif(1, None, None, None, true);
    oversized.resize(1024 * 1024 + 1, 0);
    let (oversized, _) = fixture_png(Some(&oversized), None);
    assert!(prepare_image_imports(&[("oversized.png", &oversized)], source()).is_err());

    let mut one_megabyte = tiff_exif(1, None, None, None, true);
    one_megabyte.resize(1024 * 1024, 0);
    let (large, _) = fixture_png(Some(&one_megabyte), None);
    let boundary: Vec<_> = (0..8).map(|_| ("large.png", large.as_slice())).collect();
    assert!(prepare_image_imports(&boundary, source()).is_ok());
    let files: Vec<_> = (0..9).map(|_| ("large.png", large.as_slice())).collect();
    assert!(matches!(
        prepare_image_imports(&files, source()),
        Err(pdfmaster_core::PdfMasterError::Budget(
            "image color metadata"
        ))
    ));
}
