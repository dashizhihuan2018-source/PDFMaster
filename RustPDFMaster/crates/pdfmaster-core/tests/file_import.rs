use pdfmaster_core::{Session, prepare_imports};
use std::io::Cursor;

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
    assert!(
        prepare_imports(&[("linear.png", &linear)]).is_err(),
        "linear samples cannot be emitted unchanged as device RGB"
    );
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
