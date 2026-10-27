use pdfmaster_core::{Session, prepare_imports};
use std::io::Cursor;

#[test]
fn image_tool_layout_matches_letter_landscape_margins_and_pixel_sized_pages() {
    use pdfmaster_core::{ImageImportLayout, ImagePaper, prepare_image_imports};
    let image = png();
    let layout = ImageImportLayout {
        paper: ImagePaper::Letter,
        landscape: true,
        margin: 30.,
    };
    let converted = prepare_image_imports(&[("test.png", &image)], layout).unwrap();
    let s = Session::open(&converted[0]).unwrap();
    assert_eq!(
        (s.info().pages[0].width, s.info().pages[0].height),
        (792., 612.)
    );
    let doc = lopdf::Document::load_mem(&converted[0]).unwrap();
    let id = *doc.get_pages().values().next().unwrap();
    let content = String::from_utf8(doc.get_page_content(id)).unwrap();
    assert!(
        content.contains("732 0 0 366 30 123 cm"),
        "aspect-fit/centering must use chosen margins: {content}"
    );
    let pixels = ImageImportLayout {
        paper: ImagePaper::Pixels,
        landscape: true,
        margin: 999.,
    };
    let converted = prepare_image_imports(&[("test.png", &image)], pixels).unwrap();
    let s = Session::open(&converted[0]).unwrap();
    assert_eq!(
        (s.info().pages[0].width, s.info().pages[0].height),
        (2., 1.),
        "pixel mode ignores orientation and margin, 1px=1pt"
    );
    for margin in [-1., f64::NAN, f64::INFINITY, 306.] {
        assert!(
            prepare_image_imports(
                &[("test.png", &image)],
                ImageImportLayout { margin, ..layout }
            )
            .is_err()
        );
    }
    let pdf = Session::blank(100., 100., 1).unwrap().bytes().unwrap();
    assert!(
        prepare_image_imports(&[("not-image.pdf", &pdf)], layout).is_err(),
        "image tool is not the mixed-file importer"
    );
}

#[test]
fn pixel_paper_uses_each_oriented_tiff_frame_dimensions_without_dpi_guessing() {
    use pdfmaster_core::{ImageImportLayout, ImagePaper, prepare_image_imports};
    let mut bytes = Cursor::new(Vec::new());
    {
        let mut e = tiff::encoder::TiffEncoder::new(&mut bytes).unwrap();
        let mut page = e.new_image::<tiff::encoder::colortype::RGB8>(3, 1).unwrap();
        page.encoder()
            .write_tag(tiff::tags::Tag::Orientation, 6u16)
            .unwrap();
        page.write_data(&[128; 9]).unwrap();
        e.write_image::<tiff::encoder::colortype::RGB8>(2, 4, &[64; 24])
            .unwrap();
    }
    let converted = prepare_image_imports(
        &[("two.tif", bytes.get_ref())],
        ImageImportLayout {
            paper: ImagePaper::Pixels,
            landscape: false,
            margin: 0.,
        },
    )
    .unwrap();
    let s = Session::open(&converted[0]).unwrap();
    assert_eq!(
        s.info()
            .pages
            .iter()
            .map(|p| (p.width, p.height))
            .collect::<Vec<_>>(),
        [(1., 3.), (2., 4.)]
    );
}

fn png() -> Vec<u8> {
    let mut bytes = vec![];
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[255, 0, 0, 255, 0, 0, 255, 0])
            .unwrap();
    }
    bytes
}

fn embedded_image(pdf: &[u8]) -> (i64, i64, Vec<u8>) {
    let d = lopdf::Document::load_mem(pdf).unwrap();
    let stream = d
        .objects
        .values()
        .filter_map(|o| o.as_stream().ok())
        .find(|s| s.dict.get(b"Subtype").and_then(lopdf::Object::as_name).ok() == Some(b"Image"))
        .unwrap();
    (
        stream.dict.get(b"Width").unwrap().as_i64().unwrap(),
        stream.dict.get(b"Height").unwrap().as_i64().unwrap(),
        stream.get_plain_content().unwrap(),
    )
}

fn profile_png(profile: &[u8], gray: bool, sixteen: bool, samples: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(profile).unwrap();
    let mut iccp = b"Synthetic\0\0".to_vec();
    iccp.extend(z.finish().unwrap());
    let mut bytes = vec![];
    {
        let mut e = png::Encoder::new(&mut bytes, 1, 1);
        e.set_color(if gray {
            png::ColorType::GrayscaleAlpha
        } else {
            png::ColorType::Rgba
        });
        e.set_depth(if sixteen {
            png::BitDepth::Sixteen
        } else {
            png::BitDepth::Eight
        });
        let mut w = e.write_header().unwrap();
        w.write_chunk(png::chunk::iCCP, &iccp).unwrap();
        w.write_image_data(samples).unwrap();
    }
    bytes
}

fn linear_rgb_profile() -> Vec<u8> {
    let mut p = moxcms::ColorProfile::new_srgb();
    p.cicp = None;
    p.red_trc = Some(moxcms::curve_from_gamma(1.));
    p.green_trc = p.red_trc.clone();
    p.blue_trc = p.red_trc.clone();
    encoded_profile(p)
}

fn encoded_profile(p: moxcms::ColorProfile) -> Vec<u8> {
    // The pinned CMS writer does not align tag payloads; make our synthetic
    // ICC fixture conform to the four-byte offset rule, not the importer lax.
    let bytes = p.encode().unwrap();
    let count = u32::from_be_bytes(bytes[128..132].try_into().unwrap()) as usize;
    let end = 132 + count * 12;
    let mut out = bytes[..end].to_vec();
    for i in 0..count {
        let tag = 132 + i * 12;
        let start = u32::from_be_bytes(bytes[tag + 4..tag + 8].try_into().unwrap()) as usize;
        let size = u32::from_be_bytes(bytes[tag + 8..tag + 12].try_into().unwrap()) as usize;
        while !out.len().is_multiple_of(4) {
            out.push(0);
        }
        let offset = out.len() as u32;
        out[tag + 4..tag + 8].copy_from_slice(&offset.to_be_bytes());
        out.extend_from_slice(&bytes[start..start + size]);
    }
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
    let size = out.len() as u32;
    out[..4].copy_from_slice(&size.to_be_bytes());
    out
}

#[test]
fn matrix_icc_png_gray_rgb_and_16bit_are_normalized_before_alpha_composition() {
    for gray in [false, true] {
        let profile = if gray {
            encoded_profile(moxcms::ColorProfile::new_gray_with_gamma(1.))
        } else {
            linear_rgb_profile()
        };
        for sixteen in [false, true] {
            let mut samples = vec![];
            for value in if gray {
                vec![128u16 * 257, 128 * 257]
            } else {
                vec![128u16 * 257; 4]
            } {
                if sixteen {
                    samples.extend(value.to_be_bytes());
                } else {
                    samples.push((value >> 8) as u8);
                }
            }
            let bytes = profile_png(&profile, gray, sixteen, &samples);
            let converted = prepare_imports(&[("profile.png", &bytes)]).unwrap();
            let (_, _, rgb) = embedded_image(&converted[0]);
            assert!(
                rgb.iter().all(|v| (220..=223).contains(v)),
                "gray={gray} sixteen={sixteen}: {rgb:?}"
            );
        }
    }
    // A low 16-bit value must be transformed before reduction to 8 bits.
    let bytes = profile_png(
        &linear_rgb_profile(),
        false,
        true,
        &[0, 128, 0, 128, 0, 128, 255, 255],
    );
    let converted = prepare_imports(&[("dark.png", &bytes)]).unwrap();
    assert!(
        embedded_image(&converted[0])
            .2
            .iter()
            .all(|v| (5..=7).contains(v))
    );
}

#[test]
fn tiff_each_frame_uses_its_own_icc_after_unpremultiplication_and_orientation() {
    let mut bytes = Cursor::new(Vec::new());
    {
        let mut e = tiff::encoder::TiffEncoder::new(&mut bytes).unwrap();
        let mut p = e
            .new_image::<tiff::encoder::colortype::RGBA16>(1, 2)
            .unwrap();
        p.encoder()
            .write_tag(tiff::tags::Tag::IccProfile, linear_rgb_profile().as_slice())
            .unwrap();
        p.encoder()
            .write_tag(tiff::tags::Tag::ExtraSamples, &[1u16][..])
            .unwrap();
        p.encoder()
            .write_tag(tiff::tags::Tag::Orientation, 6u16)
            .unwrap();
        p.write_data(&[64 * 257, 64 * 257, 64 * 257, 128 * 257, 0, 0, 0, 0])
            .unwrap();
        e.write_image::<tiff::encoder::colortype::RGB8>(1, 1, &[64; 3])
            .unwrap();
    }
    let converted = prepare_imports(&[("profile.tif", bytes.get_ref())]).unwrap();
    let doc = lopdf::Document::load_mem(&converted[0]).unwrap();
    for (i, id) in doc.get_pages().values().enumerate() {
        let images = doc.get_page_images(*id).unwrap();
        let stream = doc.get_object(images[0].id).unwrap().as_stream().unwrap();
        let rgb = stream.get_plain_content().unwrap();
        if i == 0 {
            assert_eq!((images[0].width, images[0].height), (2, 1));
            assert_eq!(&rgb[..3], &[255; 3]);
            assert!(rgb[3..].iter().all(|v| (220..=223).contains(v)), "{rgb:?}");
        } else {
            assert_eq!(rgb, [64; 3]);
        }
    }
}

fn add_png_chunk(mut bytes: Vec<u8>, name: &[u8; 4], data: &[u8], late: bool) -> Vec<u8> {
    let mut chunk = (data.len() as u32).to_be_bytes().to_vec();
    chunk.extend(name);
    chunk.extend(data);
    chunk.extend(crc32fast::hash(&chunk[4..]).to_be_bytes());
    let offset = if late { bytes.len() - 12 } else { 33 };
    bytes.splice(offset..offset, chunk);
    bytes
}

#[test]
fn png_gamma_chromaticities_and_icc_priority_have_independent_pixel_oracles() {
    let rgba = [180u8, 100, 60, 255];
    let plain = {
        let mut b = vec![];
        {
            let mut e = png::Encoder::new(&mut b, 1, 1);
            e.set_color(png::ColorType::Rgba);
            e.write_header().unwrap().write_image_data(&rgba).unwrap();
        }
        b
    };
    let chroma: Vec<u8> = [31270u32, 32900, 68000, 32000, 26500, 69000, 15000, 6000]
        .into_iter()
        .flat_map(u32::to_be_bytes)
        .collect();
    let calibrated = add_png_chunk(
        add_png_chunk(plain, b"cHRM", &chroma, false),
        b"gAMA",
        &45455u32.to_be_bytes(),
        false,
    );
    let converted = prepare_imports(&[("p3-gamma.png", &calibrated)]).unwrap();
    // Independent Display-P3 -> sRGB linear matrix and the explicitly declared
    // PNG gamma (not moxcms itself) provide this oracle, allowing 2/255 rounding.
    let src = rgba[..3]
        .iter()
        .map(|v| (f64::from(*v) / 255.).powf(1. / 0.45455))
        .collect::<Vec<_>>();
    let matrix = [
        [1.224745, -0.224904, 0.],
        [-0.042058, 1.042081, 0.],
        [-0.019642, -0.078655, 1.098537],
    ];
    let expected: Vec<u8> = matrix
        .map(|row| {
            let linear = row
                .into_iter()
                .zip(&src)
                .map(|(m, v)| m * v)
                .sum::<f64>()
                .clamp(0., 1.);
            ((if linear <= 0.0031308 {
                linear * 12.92
            } else {
                1.055 * linear.powf(1. / 2.4) - 0.055
            }) * 255.)
                .round() as u8
        })
        .to_vec();
    let actual = embedded_image(&converted[0]).2;
    assert!(
        actual
            .iter()
            .zip(&expected)
            .all(|(a, b)| a.abs_diff(*b) <= 2),
        "actual={actual:?} expected={expected:?}"
    );
    let mut p3 = moxcms::ColorProfile::new_display_p3();
    p3.cicp = None;
    let icc = profile_png(&encoded_profile(p3), false, false, &rgba);
    let converted = prepare_imports(&[("p3-icc.png", &icc)]).unwrap();
    let src: Vec<f64> = rgba[..3]
        .iter()
        .map(|v| {
            let x = f64::from(*v) / 255.;
            if x <= 0.04045 {
                x / 12.92
            } else {
                ((x + 0.055) / 1.055).powf(2.4)
            }
        })
        .collect();
    let expected = matrix.map(|row| {
        let x = row
            .into_iter()
            .zip(&src)
            .map(|(m, v)| m * v)
            .sum::<f64>()
            .clamp(0., 1.);
        ((if x <= 0.0031308 {
            x * 12.92
        } else {
            1.055 * x.powf(1. / 2.4) - 0.055
        }) * 255.)
            .round() as u8
    });
    let actual = embedded_image(&converted[0]).2;
    assert!(
        actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 2),
        "ICC actual={actual:?} expected={expected:?}"
    );
    let linear = profile_png(&linear_rgb_profile(), false, false, &[128, 128, 128, 255]);
    let with_gamma = add_png_chunk(linear, b"gAMA", &45455u32.to_be_bytes(), false);
    let converted = prepare_imports(&[("icc-wins.png", &with_gamma)]).unwrap();
    assert!(
        embedded_image(&converted[0])
            .2
            .iter()
            .all(|v| (187..=189).contains(v))
    );
}

#[test]
fn malformed_profiles_conflicts_late_chunks_and_metadata_bombs_fail_the_whole_batch() {
    let profile = linear_rgb_profile();
    let valid = profile_png(&profile, false, false, &[128; 4]);
    let mut corrupted = valid.clone();
    corrupted[45] ^= 1; // leave the iCCP CRC incorrect
    let mut clut = profile.clone();
    clut[132..136].copy_from_slice(b"A2B0");
    let mut duplicate = profile.clone();
    duplicate[144..148].copy_from_slice(b"rXYZ");
    let mut outside = profile.clone();
    outside[136..140].copy_from_slice(&u32::MAX.to_be_bytes());
    let mut too_many = profile.clone();
    too_many[128..132].copy_from_slice(&65u32.to_be_bytes());
    let mut bomb = vec![0; 1024 * 1024 + 1];
    bomb[..4].copy_from_slice(&((1024u32 * 1024) + 1).to_be_bytes());
    let cases = vec![
        corrupted,
        profile_png(&profile[..profile.len() - 1], false, false, &[128; 4]),
        profile_png(&clut, false, false, &[128; 4]),
        profile_png(&duplicate, false, false, &[128; 4]),
        profile_png(&outside, false, false, &[128; 4]),
        profile_png(&too_many, false, false, &[128; 4]),
        profile_png(&bomb, false, false, &[128; 4]),
        profile_png(
            &encoded_profile(moxcms::ColorProfile::new_gray_with_gamma(1.)),
            false,
            false,
            &[128; 4],
        ),
        add_png_chunk(valid.clone(), b"sRGB", &[0], false),
        add_png_chunk(valid.clone(), b"PLTE", &[0; 3], false),
        add_png_chunk(png(), b"gAMA", &100000u32.to_be_bytes(), true),
        add_png_chunk(png(), b"gAMA", &[0; 4], false),
    ];
    for (i, bad) in cases.iter().enumerate() {
        assert!(
            prepare_imports(&[("first.png", &valid), ("late.png", bad)]).is_err(),
            "case {i}"
        );
    }
    let mut padded = profile;
    padded.resize(1024 * 1024, 0);
    padded[..4].copy_from_slice(&(1024u32 * 1024).to_be_bytes());
    let large = profile_png(&padded, false, false, &[128; 4]);
    assert!(matches!(
        prepare_imports(&vec![("profile.png", large.as_slice()); 9]),
        Err(pdfmaster_core::PdfMasterError::Budget(
            "image color metadata"
        ))
    ));
}

#[test]
fn icc_localized_text_counts_reused_payload_for_each_record_before_cms_allocation() {
    for (records, string_bytes, record_size) in [(17u32, 2u32, 12u32), (16, 5000, 12), (1, 2, 13)] {
        let mut profile = linear_rgb_profile();
        let mut text = b"mluc\0\0\0\0".to_vec();
        text.extend(records.to_be_bytes());
        text.extend(record_size.to_be_bytes());
        let start = 16 + records * 12;
        for _ in 0..records {
            text.extend(b"enUS");
            text.extend(string_bytes.to_be_bytes());
            text.extend(start.to_be_bytes());
        }
        text.extend(std::iter::repeat_n([0u8, b'a'], string_bytes as usize / 2).flatten());
        let count = u32::from_be_bytes(profile[128..132].try_into().unwrap()) as usize;
        let tag = (0..count)
            .map(|i| 132 + i * 12)
            .find(|i| &profile[*i..*i + 4] == b"desc")
            .unwrap();
        let offset = profile.len() as u32;
        profile[tag + 4..tag + 8].copy_from_slice(&offset.to_be_bytes());
        profile[tag + 8..tag + 12].copy_from_slice(&(text.len() as u32).to_be_bytes());
        profile.extend(text);
        let size = profile.len() as u32;
        profile[..4].copy_from_slice(&size.to_be_bytes());
        let png = profile_png(&profile, false, false, &[128; 4]);
        assert!(
            prepare_imports(&[("text-profile.png", &png)]).is_err(),
            "records={records} bytes={string_bytes} size={record_size}"
        );
    }
}

#[test]
fn icc_cicp_hdr_or_competing_transfer_declarations_are_not_silently_ignored() {
    for p in [
        moxcms::ColorProfile::new_bt2020_pq(),
        moxcms::ColorProfile::new_bt2020_hlg(),
        moxcms::ColorProfile::new_srgb(),
    ] {
        let bytes = profile_png(&encoded_profile(p), false, false, &[128; 4]);
        assert!(prepare_imports(&[("competing.png", &bytes)]).is_err());
    }
}

#[test]
fn gray_alpha_tiff_8_and_16_bit_preserve_associated_and_straight_alpha() {
    struct GrayAlpha8;
    impl tiff::encoder::colortype::ColorType for GrayAlpha8 {
        type Inner = u8;
        const TIFF_VALUE: tiff::tags::PhotometricInterpretation =
            tiff::tags::PhotometricInterpretation::BlackIsZero;
        const BITS_PER_SAMPLE: &'static [u16] = &[8, 8];
        const SAMPLE_FORMAT: &'static [tiff::tags::SampleFormat] =
            &[tiff::tags::SampleFormat::Uint; 2];
        fn horizontal_predict(_: &[u8], _: &mut Vec<u8>) {
            unreachable!()
        }
    }
    struct GrayAlpha16;
    impl tiff::encoder::colortype::ColorType for GrayAlpha16 {
        type Inner = u16;
        const TIFF_VALUE: tiff::tags::PhotometricInterpretation =
            tiff::tags::PhotometricInterpretation::BlackIsZero;
        const BITS_PER_SAMPLE: &'static [u16] = &[16, 16];
        const SAMPLE_FORMAT: &'static [tiff::tags::SampleFormat] =
            &[tiff::tags::SampleFormat::Uint; 2];
        fn horizontal_predict(_: &[u16], _: &mut Vec<u16>) {
            unreachable!()
        }
    }
    for associated in [false, true] {
        for sixteen in [false, true] {
            let mut bytes = Cursor::new(Vec::new());
            {
                let mut encoder = tiff::encoder::TiffEncoder::new(&mut bytes).unwrap();
                let extra = [if associated { 1u16 } else { 2u16 }];
                if sixteen {
                    let mut page = encoder.new_image::<GrayAlpha16>(1, 1).unwrap();
                    page.encoder()
                        .write_tag(tiff::tags::Tag::ExtraSamples, &extra[..])
                        .unwrap();
                    page.write_data(&[64 * 257, 128 * 257]).unwrap();
                } else {
                    let mut page = encoder.new_image::<GrayAlpha8>(1, 1).unwrap();
                    page.encoder()
                        .write_tag(tiff::tags::Tag::ExtraSamples, &extra[..])
                        .unwrap();
                    page.write_data(&[64, 128]).unwrap();
                }
            }
            let result = prepare_imports(&[("gray-alpha.tif", bytes.get_ref())]).unwrap();
            let (_, _, rgb) = embedded_image(&result[0]);
            assert_eq!(rgb, [if associated { 191 } else { 159 }; 3]);
        }
    }
}

#[test]
fn unsupported_image_color_metadata_is_rejected_not_silently_discarded() {
    let mut linear = vec![];
    {
        let mut e = png::Encoder::new(&mut linear, 1, 1);
        e.set_color(png::ColorType::Rgb);
        e.set_depth(png::BitDepth::Eight);
        e.set_source_gamma(png::ScaledFloat::new(1.0));
        e.write_header()
            .unwrap()
            .write_image_data(&[128; 3])
            .unwrap();
    }
    let converted = prepare_imports(&[("linear.png", &linear)]).unwrap();
    let (_, _, rgb) = embedded_image(&converted[0]);
    assert!(rgb.iter().all(|v| (187..=189).contains(v)), "{rgb:?}");
    for (kind, data) in [
        (*b"cICP", vec![9, 16, 0, 1]),
        (*b"cHRM", vec![0; 32]),
        (*b"eXIf", b"unhandled-exif".to_vec()),
    ] {
        let mut bytes = vec![];
        {
            let mut e = png::Encoder::new(&mut bytes, 1, 1);
            e.set_color(png::ColorType::Rgb);
            let mut w = e.write_header().unwrap();
            w.write_chunk(png::chunk::ChunkType(kind), &data).unwrap();
            w.write_image_data(&[128; 3]).unwrap();
        }
        assert!(prepare_imports(&[("calibrated.png", &bytes)]).is_err());
    }
    let mut srgb = vec![];
    {
        let mut e = png::Encoder::new(&mut srgb, 1, 1);
        e.set_color(png::ColorType::Rgb);
        e.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        e.write_header()
            .unwrap()
            .write_image_data(&[128; 3])
            .unwrap();
    }
    assert!(prepare_imports(&[("srgb.png", &srgb)]).is_ok());
    let mut gif = vec![];
    {
        let mut e = gif::Encoder::new(&mut gif, 1, 1, &[128; 3]).unwrap();
        e.write_raw_extension(gif::AnyExtension(0xff), &[b"ICCRGBG1012", &[1, 2, 3]])
            .unwrap();
        e.write_frame(&gif::Frame {
            width: 1,
            height: 1,
            buffer: std::borrow::Cow::Borrowed(&[0]),
            ..gif::Frame::default()
        })
        .unwrap();
    }
    assert!(prepare_imports(&[("profile.gif", &gif)]).is_err());
    let mut tiff = Cursor::new(Vec::new());
    {
        let mut e = tiff::encoder::TiffEncoder::new(&mut tiff).unwrap();
        let mut page = e.new_image::<tiff::encoder::colortype::RGB8>(1, 1).unwrap();
        page.encoder()
            .write_tag(tiff::tags::Tag::IccProfile, &[1u8, 2, 3][..])
            .unwrap();
        page.write_data(&[128; 3]).unwrap();
    }
    assert!(prepare_imports(&[("profile.tif", tiff.get_ref())]).is_err());
}

#[test]
fn tiff_orientation_and_associated_alpha_are_applied_before_white_background_composition() {
    let mut oriented = Cursor::new(Vec::new());
    {
        let mut encoder = tiff::encoder::TiffEncoder::new(&mut oriented).unwrap();
        let mut page = encoder
            .new_image::<tiff::encoder::colortype::RGB8>(2, 1)
            .unwrap();
        page.encoder()
            .write_tag(tiff::tags::Tag::Orientation, 6u16)
            .unwrap();
        page.write_data(&[255, 0, 0, 0, 0, 255]).unwrap();
    }
    let converted = prepare_imports(&[("camera.tif", oriented.get_ref())]).unwrap();
    let (w, h, rgb) = embedded_image(&converted[0]);
    assert_eq!((w, h), (1, 2));
    assert_eq!(rgb, [255, 0, 0, 0, 0, 255]);
    let mut alpha = Cursor::new(Vec::new());
    {
        let mut encoder = tiff::encoder::TiffEncoder::new(&mut alpha).unwrap();
        let mut page = encoder
            .new_image::<tiff::encoder::colortype::RGBA8>(1, 1)
            .unwrap();
        page.encoder()
            .write_tag(tiff::tags::Tag::ExtraSamples, &[1u16][..])
            .unwrap();
        page.write_data(&[128, 0, 0, 128]).unwrap();
    }
    let converted = prepare_imports(&[("premultiplied.tiff", alpha.get_ref())]).unwrap();
    let (_, _, rgb) = embedded_image(&converted[0]);
    assert_eq!(
        rgb,
        [255, 127, 127],
        "associated RGB must not be multiplied by alpha twice"
    );
}

#[test]
fn image_pixel_budgets_preflight_the_entire_batch_before_pixel_decode() {
    let mut bytes = vec![];
    image::codecs::bmp::BmpEncoder::new(&mut bytes)
        .encode(&[255, 0, 0], 1, 1, image::ExtendedColorType::Rgb8)
        .unwrap();
    bytes[18..22].copy_from_slice(&10_000i32.to_le_bytes());
    bytes[22..26].copy_from_slice(&10_000i32.to_le_bytes());
    assert!(matches!(
        prepare_imports(&[("huge.bmp", &bytes)]),
        Err(pdfmaster_core::PdfMasterError::Budget("image frame pixels"))
    ));
    bytes[18..22].copy_from_slice(&4_000i32.to_le_bytes());
    assert!(
        matches!(
            prepare_imports(&[("a.bmp", &bytes), ("b.bmp", &bytes), ("c.bmp", &bytes)]),
            Err(pdfmaster_core::PdfMasterError::Budget("image total pixels"))
        ),
        "no first-frame decode may precede full metadata admission"
    );
}

#[test]
fn new_merge_is_unsaved_without_placeholder_undo_or_first_file_admission_bypass() {
    let a = Session::blank(123., 200., 1).unwrap().bytes().unwrap();
    let b = Session::blank(321., 400., 2).unwrap().bytes().unwrap();
    let s = Session::from_imports(&[&a, &b]).unwrap();
    assert_eq!(
        s.info().pages.iter().map(|p| p.width).collect::<Vec<_>>(),
        [123., 321., 321.]
    );
    assert!(s.is_dirty());
    assert!(!s.info().can_undo && !s.info().can_redo);
    let mut tagged = lopdf::Document::load_mem(&a).unwrap();
    tagged
        .catalog_mut()
        .unwrap()
        .set("MarkInfo", lopdf::Dictionary::new());
    let mut bytes = vec![];
    tagged.save_to(&mut bytes).unwrap();
    assert!(Session::from_imports(&[&bytes, &b]).is_err());
    assert!(Session::from_imports(&[&b, &bytes]).is_err());
}

#[test]
fn mixed_batch_keeps_pdf_bytes_and_a4_white_backed_image_pixels_in_source_order() {
    let pdf = Session::blank(111., 222., 2).unwrap().bytes().unwrap();
    let image = png();
    let converted = prepare_imports(&[("first.PDF", &pdf), ("alpha.PNG", &image)]).unwrap();
    assert_eq!(converted[0], pdf);
    let converted_image = Session::open(&converted[1]).unwrap();
    assert_eq!(converted_image.info().page_count, 1);
    assert!((converted_image.info().pages[0].width - 595.276).abs() < 0.01);
    assert!((converted_image.info().pages[0].height - 841.89).abs() < 0.01);
    let rendered = converted_image.render_png(0, 1.).unwrap();
    let mut r = png::Decoder::new(Cursor::new(rendered))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; r.output_buffer_size().unwrap()];
    let info = r.next_frame(&mut pixels).unwrap();
    let channels = info.color_type.samples();
    let at = |x: usize, y: usize| &pixels[(y * info.width as usize + x) * channels..][..3];
    assert!(
        at(100, 420)[0] > 240 && at(100, 420)[1] < 20,
        "left half stays red"
    );
    assert!(
        at(450, 420).iter().all(|v| *v > 240),
        "transparent blue flattens to white"
    );
    assert!(
        at(10, 10).iter().all(|v| *v > 240),
        "24pt A4 margins remain white"
    );
    let merged =
        Session::from_imports(&converted.iter().map(Vec::as_slice).collect::<Vec<_>>()).unwrap();
    assert_eq!(
        merged
            .info()
            .pages
            .iter()
            .map(|p| p.width.round() as u32)
            .collect::<Vec<_>>(),
        [111, 111, 595]
    );
    assert!(merged.is_dirty());
    assert!(
        !merged.info().can_undo,
        "new merge has no placeholder/assembly Undo"
    );
    assert!(!merged.info().can_redo);
    assert_eq!(
        Session::open(&merged.bytes().unwrap())
            .unwrap()
            .info()
            .page_count,
        3
    );
    assert_eq!(image, png());
}

#[test]
fn invalid_late_file_rejects_entire_batch_and_new_merge_uses_same_admission_for_first_pdf() {
    let image = png();
    for late in [
        ("bad.png", b"broken".as_slice()),
        ("secret.docx", b"unimplemented"),
        ("wrong.pdf", &image),
    ] {
        assert!(prepare_imports(&[("ok.png", &image), late]).is_err());
    }
    assert!(prepare_imports(&[]).is_err());
    assert!(prepare_imports(&vec![("ok.png", image.as_slice()); 33]).is_err());
    assert!(Session::from_imports(&[]).is_err());
    let pdf = Session::blank(100., 100., 1).unwrap().bytes().unwrap();
    assert!(Session::from_imports(&[b"broken", &pdf]).is_err());
    assert!(Session::from_imports(&[&pdf, b"broken"]).is_err());
}

#[test]
fn every_tiff_frame_and_static_gif_are_imported_but_animation_is_not_silently_truncated() {
    let mut bytes = Cursor::new(Vec::new());
    {
        let mut encoder = tiff::encoder::TiffEncoder::new(&mut bytes).unwrap();
        encoder
            .write_image::<tiff::encoder::colortype::RGB8>(2, 1, &[255, 0, 0, 255, 0, 0])
            .unwrap();
        encoder
            .write_image::<tiff::encoder::colortype::RGB8>(1, 2, &[0, 0, 255, 0, 0, 255])
            .unwrap();
    }
    let batch = prepare_imports(&[("two.tiff", bytes.get_ref())]).unwrap();
    assert_eq!(Session::open(&batch[0]).unwrap().info().page_count, 2);
    let gif = |count| {
        let mut bytes = vec![];
        {
            let mut encoder = gif::Encoder::new(&mut bytes, 1, 1, &[255, 0, 0]).unwrap();
            for _ in 0..count {
                let frame = gif::Frame {
                    width: 1,
                    height: 1,
                    buffer: std::borrow::Cow::Borrowed(&[0]),
                    ..gif::Frame::default()
                };
                encoder.write_frame(&frame).unwrap();
            }
        }
        bytes
    };
    assert!(prepare_imports(&[("one.gif", &gif(1))]).is_ok());
    assert!(prepare_imports(&[("animated.gif", &gif(2))]).is_err());
}
