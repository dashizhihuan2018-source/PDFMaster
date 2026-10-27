use std::cell::Cell;
use std::io::Cursor;

use image::ImageFormat;
use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use pdfmaster_core::{
    Color, Edit, ImageExportFormat, ImageExportLimits, ImageExportRequest, PdfMasterError, Rect,
    Session, ShapeKind,
};

fn fixture_pdf() -> Vec<u8> {
    let mut document = Document::with_version("1.7");
    let pages_id = document.new_object_id();
    let mut page_ids = Vec::new();
    for content in [Vec::new(), b"1 0 0 rg 10 10 30 20 re f\n".to_vec()] {
        let content_id = document.add_object(Stream::new(Dictionary::new(), content));
        page_ids.push(document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 100.into(), 60.into()],
            "Resources" => Dictionary::new(),
            "Contents" => content_id,
        }));
    }
    document.objects.insert(
        pages_id,
        dictionary! {
            "Type" => "Pages",
            "Kids" => page_ids.iter().copied().map(Object::Reference).collect::<Vec<_>>(),
            "Count" => page_ids.len() as i64,
        }
        .into(),
    );
    let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    document.trailer.set("Root", catalog_id);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

#[derive(Clone, Copy)]
struct PageSpec<'a> {
    width: f32,
    height: f32,
    crop: Option<[f32; 4]>,
    rotation: i64,
    content: &'a [u8],
}

fn custom_fixture(pages: &[PageSpec<'_>]) -> Vec<u8> {
    let mut document = Document::with_version("1.7");
    let pages_id = document.new_object_id();
    let mut page_ids = Vec::new();
    for page in pages {
        let content_id = document.add_object(Stream::new(Dictionary::new(), page.content.to_vec()));
        let mut dictionary = dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), page.width.into(), page.height.into()],
            "Resources" => Dictionary::new(),
            "Contents" => content_id,
        };
        if let Some([x0, y0, x1, y1]) = page.crop {
            dictionary.set("CropBox", vec![x0.into(), y0.into(), x1.into(), y1.into()]);
        }
        if page.rotation != 0 {
            dictionary.set("Rotate", page.rotation);
        }
        page_ids.push(document.add_object(dictionary));
    }
    document.objects.insert(
        pages_id,
        dictionary! {
            "Type" => "Pages",
            "Kids" => page_ids.iter().copied().map(Object::Reference).collect::<Vec<_>>(),
            "Count" => page_ids.len() as i64,
        }
        .into(),
    );
    let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    document.trailer.set("Root", catalog_id);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

fn one_page_request(format: ImageExportFormat, dpi: f64) -> ImageExportRequest {
    ImageExportRequest {
        format,
        dpi,
        pages: vec![0],
        ..Default::default()
    }
}

fn export_one(session: &Session, request: &ImageExportRequest) -> pdfmaster_core::ExportedImage {
    let mut files = Vec::new();
    session
        .export_images(
            request,
            || false,
            |file| {
                files.push(file);
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(files.len(), 1);
    files.pop().unwrap()
}

fn decode_png(bytes: &[u8]) -> (u32, u32, Vec<u8>, Option<png::PixelDimensions>) {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(
        png::Transformations::normalize_to_color8() | png::Transformations::ALPHA,
    );
    let mut reader = decoder.read_info().unwrap();
    let dimensions = reader.info().pixel_dims;
    let (width, height) = (reader.info().width, reader.info().height);
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let frame = reader.next_frame(&mut pixels).unwrap();
    pixels.truncate(frame.buffer_size());
    (width, height, pixels, dimensions)
}

fn jpeg_density(bytes: &[u8]) -> (u8, u16, u16) {
    assert_eq!(&bytes[..2], &[0xff, 0xd8]);
    let mut offset = 2;
    while offset + 4 <= bytes.len() {
        assert_eq!(bytes[offset], 0xff);
        let marker = bytes[offset + 1];
        let length = u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]) as usize;
        let end = offset + 2 + length;
        assert!(end <= bytes.len());
        if marker == 0xe0 && length >= 16 && &bytes[offset + 4..offset + 9] == b"JFIF\0" {
            return (
                bytes[offset + 11],
                u16::from_be_bytes([bytes[offset + 12], bytes[offset + 13]]),
                u16::from_be_bytes([bytes[offset + 14], bytes[offset + 15]]),
            );
        }
        offset = end;
    }
    panic!("JPEG lacks a JFIF density segment");
}

fn assert_session_unchanged(session: &Session, bytes: &[u8], info: &pdfmaster_core::DocumentInfo) {
    assert_eq!(session.bytes().unwrap(), bytes);
    assert_eq!(&session.info(), info);
}

#[test]
fn png_export_sorts_deduplicates_and_preserves_source() {
    let session = Session::open(&fixture_pdf()).unwrap();
    let before = session.bytes().unwrap();
    let before_info = session.info();
    let request = ImageExportRequest {
        pages: vec![1, 0, 1],
        ..Default::default()
    };
    let mut files = Vec::new();
    let result = session.export_images(
        &request,
        || false,
        |file| {
            files.push(file);
            Ok(())
        },
    );
    assert!(
        result.is_ok(),
        "valid two-page PNG export must succeed: {result:?}"
    );
    let summary = result.unwrap();
    assert_eq!(summary.pages, 2);
    assert_eq!(
        summary.total_bytes,
        files.iter().map(|file| file.bytes.len()).sum::<usize>()
    );
    assert_eq!(
        files
            .iter()
            .map(|page| page.filename.as_str())
            .collect::<Vec<_>>(),
        ["page-0001.png", "page-0002.png"]
    );
    assert_eq!(session.bytes().unwrap(), before);
    assert_eq!(session.info(), before_info);
}

#[test]
fn defaults_and_format_serialization_are_stable() {
    let request = ImageExportRequest::default();
    assert_eq!(request.format, ImageExportFormat::Png);
    assert_eq!(request.dpi, 144.0);
    assert!(request.pages.is_empty());
    assert_eq!(request.jpeg_quality, 0.9);
    assert_eq!(request.limits, ImageExportLimits::default());
    assert_eq!(
        serde_json::to_string(&ImageExportFormat::Png).unwrap(),
        "\"png\""
    );
    assert_eq!(
        serde_json::to_string(&ImageExportFormat::Jpeg).unwrap(),
        "\"jpeg\""
    );
}

#[test]
fn invalid_requests_never_call_sink() {
    let session = Session::open(&fixture_pdf()).unwrap();
    let defaults = ImageExportLimits::default();
    let mut requests = vec![
        ImageExportRequest::default(),
        ImageExportRequest {
            pages: vec![2],
            ..Default::default()
        },
        ImageExportRequest {
            pages: vec![0],
            dpi: f64::NAN,
            ..Default::default()
        },
        ImageExportRequest {
            pages: vec![0],
            dpi: f64::INFINITY,
            ..Default::default()
        },
        ImageExportRequest {
            pages: vec![0],
            dpi: 35.9,
            ..Default::default()
        },
        ImageExportRequest {
            pages: vec![0],
            dpi: 600.1,
            ..Default::default()
        },
        ImageExportRequest {
            pages: vec![0],
            jpeg_quality: f32::NAN,
            ..Default::default()
        },
        ImageExportRequest {
            pages: vec![0],
            jpeg_quality: -0.01,
            ..Default::default()
        },
        ImageExportRequest {
            pages: vec![0],
            jpeg_quality: 1.01,
            ..Default::default()
        },
    ];
    for limits in [
        ImageExportLimits {
            page_pixels: 0,
            ..defaults
        },
        ImageExportLimits {
            total_pixels: 0,
            ..defaults
        },
        ImageExportLimits {
            output_bytes: 0,
            ..defaults
        },
        ImageExportLimits {
            page_pixels: defaults.page_pixels + 1,
            ..defaults
        },
        ImageExportLimits {
            total_pixels: defaults.total_pixels + 1,
            ..defaults
        },
        ImageExportLimits {
            output_bytes: defaults.output_bytes + 1,
            ..defaults
        },
    ] {
        requests.push(ImageExportRequest {
            pages: vec![0],
            limits,
            ..Default::default()
        });
    }
    for request in requests {
        let calls = Cell::new(0);
        assert!(
            session
                .export_images(
                    &request,
                    || false,
                    |_| {
                        calls.set(calls.get() + 1);
                        Ok(())
                    }
                )
                .is_err()
        );
        assert_eq!(
            calls.get(),
            0,
            "invalid request reached the sink: {request:?}"
        );
    }
}

#[test]
fn preflights_every_selected_page_before_emitting() {
    let oversized = custom_fixture(&[
        PageSpec {
            width: 100.0,
            height: 100.0,
            crop: None,
            rotation: 0,
            content: b"",
        },
        PageSpec {
            width: 200.0,
            height: 200.0,
            crop: None,
            rotation: 0,
            content: b"",
        },
    ]);
    let session = Session::open(&oversized).unwrap();
    let request = ImageExportRequest {
        dpi: 72.0,
        pages: vec![0, 1],
        limits: ImageExportLimits {
            total_pixels: 20_000,
            ..Default::default()
        },
        ..Default::default()
    };
    let calls = Cell::new(0);
    assert!(
        session
            .export_images(
                &request,
                || false,
                |_| {
                    calls.set(calls.get() + 1);
                    Ok(())
                }
            )
            .is_err()
    );
    assert_eq!(calls.get(), 0);

    let unsupported = custom_fixture(&[
        PageSpec {
            width: 100.0,
            height: 60.0,
            crop: None,
            rotation: 0,
            content: b"",
        },
        PageSpec {
            width: 100.0,
            height: 60.0,
            crop: None,
            rotation: 0,
            content: b"1 0 0 1 0 0 cm /GS1 gs",
        },
    ]);
    let session = Session::open(&unsupported).unwrap();
    let request = ImageExportRequest {
        dpi: 72.0,
        pages: vec![0, 1],
        ..Default::default()
    };
    let calls = Cell::new(0);
    assert!(
        session
            .export_images(
                &request,
                || false,
                |_| {
                    calls.set(calls.get() + 1);
                    Ok(())
                }
            )
            .is_err()
    );
    assert_eq!(calls.get(), 0);
}

#[test]
fn png_exports_real_pixels_dimensions_and_physical_density() {
    let session = Session::open(&custom_fixture(&[PageSpec {
        width: 100.25,
        height: 60.25,
        crop: None,
        rotation: 0,
        content: b"1 0 0 rg 10 10 30 20 re f\n",
    }]))
    .unwrap();
    for dpi in [72.0, 144.0, 300.0] {
        let file = export_one(&session, &one_page_request(ImageExportFormat::Png, dpi));
        let (width, height, pixels, dimensions) = decode_png(&file.bytes);
        assert_eq!((file.width, file.height), (width, height));
        assert_eq!(width, (100.25_f64 * dpi / 72.0).ceil() as u32);
        assert_eq!(height, (60.25_f64 * dpi / 72.0).ceil() as u32);
        let dimensions = dimensions.expect("PNG must contain pHYs");
        assert_eq!(dimensions.unit, png::Unit::Meter);
        let expected_ppm = (dpi / 0.0254).round() as u32;
        assert_eq!(
            (dimensions.xppu, dimensions.yppu),
            (expected_ppm, expected_ppm)
        );
        assert!(
            pixels
                .chunks_exact(4)
                .any(|p| p[0] > 180 && p[1] < 80 && p[2] < 80)
        );
        assert!(pixels.chunks_exact(4).any(|p| p[..3] == [255, 255, 255]));
    }
}

#[test]
fn ceil_sized_canvas_preserves_uniform_requested_dpi_pixel_mapping() {
    let session = Session::open(&custom_fixture(&[PageSpec {
        width: 10.25,
        height: 6.25,
        crop: None,
        rotation: 0,
        content: b"1 0 0 rg 0 0 4 6 re f\n",
    }]))
    .unwrap();
    let exported = export_one(&session, &one_page_request(ImageExportFormat::Png, 72.0));
    let (width, height, pixels, _) = decode_png(&exported.bytes);
    let (reference_width, reference_height, reference, _) =
        decode_png(&session.render_png(0, 1.0).unwrap());
    assert_eq!((width, height), (11, 7));
    assert_eq!((reference_width, reference_height), (10, 6));
    for y in 0..reference_height as usize {
        for x in 0..reference_width as usize {
            let exported_offset = (y * width as usize + x) * 4;
            let reference_offset = (y * reference_width as usize + x) * 4;
            assert_eq!(
                &pixels[exported_offset..exported_offset + 4],
                &reference[reference_offset..reference_offset + 4],
                "ceil canvas must add only right/bottom extent at ({x}, {y})"
            );
        }
    }
}

#[test]
fn rotated_crop_box_controls_actual_output_geometry() {
    let session = Session::open(&custom_fixture(&[PageSpec {
        width: 100.0,
        height: 60.0,
        crop: Some([0.0, 0.0, 100.0, 60.0]),
        rotation: 90,
        content: b"0 0 1 rg 10 20 100 60 re f\n",
    }]))
    .unwrap();
    let file = export_one(&session, &one_page_request(ImageExportFormat::Png, 144.0));
    assert_eq!((file.width, file.height), (120, 200));
    let decoded = decode_png(&file.bytes);
    assert_eq!((decoded.0, decoded.1), (120, 200));
}

#[test]
fn jpeg_exports_real_pixels_and_jfif_density() {
    let session = Session::open(&custom_fixture(&[PageSpec {
        width: 100.0,
        height: 60.0,
        crop: None,
        rotation: 0,
        content: b"1 0 0 rg 10 10 30 20 re f\n",
    }]))
    .unwrap();
    let file = export_one(&session, &one_page_request(ImageExportFormat::Jpeg, 300.0));
    assert_eq!(file.filename, "page-0001.jpg");
    assert_eq!(jpeg_density(&file.bytes), (1, 300, 300));
    let image = image::load_from_memory_with_format(&file.bytes, ImageFormat::Jpeg)
        .unwrap()
        .to_rgb8();
    assert_eq!(image.dimensions(), (417, 250));
    assert!(
        image
            .pixels()
            .any(|p| p[0] > 150 && p[1] < 120 && p[2] < 120)
    );
    assert!(
        image
            .pixels()
            .any(|p| p.0.iter().all(|channel| *channel > 240))
    );
}

#[test]
fn supported_annotation_pixels_match_the_shared_renderer() {
    let mut session = Session::open(&custom_fixture(&[PageSpec {
        width: 100.0,
        height: 60.0,
        crop: None,
        rotation: 0,
        content: b"",
    }]))
    .unwrap();
    session
        .apply(Edit::AddShape {
            page: 0,
            bounds: Rect {
                x: 15.0,
                y: 10.0,
                width: 35.0,
                height: 25.0,
            },
            shape: ShapeKind::Rectangle,
            stroke: Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
            },
            fill: Some(Color {
                r: 0.0,
                g: 0.6,
                b: 0.1,
            }),
            line_width: 2.0,
        })
        .unwrap();
    let exported = export_one(&session, &one_page_request(ImageExportFormat::Png, 72.0));
    let shared = session.render_png(0, 1.0).unwrap();
    assert_eq!(decode_png(&exported.bytes).2, decode_png(&shared).2);
}

#[test]
fn cancellation_and_sink_failures_propagate_without_mutation() {
    let session = Session::open(&fixture_pdf()).unwrap();
    let before = session.bytes().unwrap();
    let info = session.info();
    let request = ImageExportRequest {
        dpi: 72.0,
        pages: vec![0, 1],
        ..Default::default()
    };

    let calls = Cell::new(0);
    let error = session
        .export_images(
            &request,
            || true,
            |_| {
                calls.set(calls.get() + 1);
                Ok(())
            },
        )
        .unwrap_err();
    assert!(matches!(error, PdfMasterError::Cancelled));
    assert_eq!(calls.get(), 0);
    assert_session_unchanged(&session, &before, &info);

    let stopped = Cell::new(false);
    let emitted = Cell::new(0);
    let error = session
        .export_images(
            &request,
            || stopped.get(),
            |_| {
                emitted.set(emitted.get() + 1);
                stopped.set(true);
                Ok(())
            },
        )
        .unwrap_err();
    assert!(matches!(error, PdfMasterError::Cancelled));
    assert_eq!(emitted.get(), 1);
    assert_session_unchanged(&session, &before, &info);

    let error = session
        .export_images(
            &request,
            || false,
            |_| Err(PdfMasterError::InvalidOperation("sink rejected image")),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        PdfMasterError::InvalidOperation("sink rejected image")
    ));
    assert_session_unchanged(&session, &before, &info);
}

#[test]
fn byte_limits_stop_encoding_and_bound_the_aggregate() {
    let session = Session::open(&fixture_pdf()).unwrap();
    let before = session.bytes().unwrap();
    let info = session.info();
    let tiny = ImageExportRequest {
        dpi: 72.0,
        pages: vec![0],
        limits: ImageExportLimits {
            output_bytes: 16,
            ..Default::default()
        },
        ..Default::default()
    };
    let emitted = Cell::new(0);
    assert!(
        session
            .export_images(
                &tiny,
                || false,
                |_| {
                    emitted.set(emitted.get() + 1);
                    Ok(())
                }
            )
            .is_err()
    );
    assert_eq!(emitted.get(), 0);

    let baseline_request = ImageExportRequest {
        dpi: 72.0,
        pages: vec![0, 1],
        ..Default::default()
    };
    let mut baseline = Vec::new();
    session
        .export_images(
            &baseline_request,
            || false,
            |file| {
                baseline.push(file.bytes.len());
                Ok(())
            },
        )
        .unwrap();
    let request = ImageExportRequest {
        limits: ImageExportLimits {
            output_bytes: baseline[0] + baseline[1] - 1,
            ..Default::default()
        },
        ..baseline_request
    };
    let emitted = Cell::new(0);
    assert!(
        session
            .export_images(
                &request,
                || false,
                |_| {
                    emitted.set(emitted.get() + 1);
                    Ok(())
                }
            )
            .is_err()
    );
    assert_eq!(emitted.get(), 1);
    assert_session_unchanged(&session, &before, &info);
}
