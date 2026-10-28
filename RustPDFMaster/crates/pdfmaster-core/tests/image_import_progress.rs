use pdfmaster_core::{
    ImageImportLayout, ImageImportProgress, ImagePaper, PdfMasterError, Session,
    prepare_image_imports, prepare_image_imports_cancellable, prepare_image_imports_observed,
};
use std::{cell::Cell, io::Cursor};

fn png() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[255, 0, 0, 255, 0, 0, 255, 128])
            .unwrap();
    }
    bytes
}

fn pixel_layout() -> ImageImportLayout {
    ImageImportLayout {
        paper: ImagePaper::Pixels,
        landscape: false,
        margin: 0.,
    }
}

fn source_layout() -> ImageImportLayout {
    ImageImportLayout {
        paper: ImagePaper::Source,
        landscape: false,
        margin: 0.,
    }
}

fn multipage_tiff() -> Vec<u8> {
    use tiff::{
        encoder::{Rational, colortype::RGB8},
        tags::Tag,
    };

    let mut bytes = Cursor::new(Vec::new());
    {
        let mut encoder = tiff::encoder::TiffEncoder::new(&mut bytes).unwrap();
        let mut first = encoder.new_image::<RGB8>(2, 1).unwrap();
        first
            .encoder()
            .write_tag(Tag::ResolutionUnit, 2u16)
            .unwrap();
        first
            .encoder()
            .write_tag(Tag::XResolution, Rational { n: 72, d: 1 })
            .unwrap();
        first
            .encoder()
            .write_tag(Tag::YResolution, Rational { n: 72, d: 1 })
            .unwrap();
        first.write_data(&[255, 0, 0, 0, 255, 0]).unwrap();
        encoder
            .write_image::<RGB8>(1, 2, &[0, 0, 255, 255, 255, 0])
            .unwrap();
    }
    bytes.into_inner()
}

fn preflight_error(
    files: &[(&str, &[u8])],
    layout: ImageImportLayout,
    cancelled: bool,
) -> PdfMasterError {
    let mut seen = Vec::new();
    let result =
        prepare_image_imports_observed(files, layout, || cancelled, |progress| seen.push(progress));

    assert!(seen.is_empty(), "preflight emitted {seen:?}");
    result.expect_err("the invalid batch unexpectedly succeeded")
}

#[test]
fn observed_image_import_reports_each_source_in_order() {
    let first = png();
    let second = png();
    let files = [
        ("first.png", first.as_slice()),
        ("second.png", second.as_slice()),
    ];
    let mut seen = Vec::new();

    let result = prepare_image_imports_observed(
        &files,
        pixel_layout(),
        || false,
        |progress| seen.push(progress),
    )
    .unwrap();

    assert_eq!(result.len(), 2);
    assert_eq!(
        result,
        prepare_image_imports(&files, pixel_layout()).unwrap()
    );
    assert_eq!(
        seen,
        [
            ImageImportProgress {
                current: 1,
                total: 2,
            },
            ImageImportProgress {
                current: 2,
                total: 2,
            },
        ]
    );
}

#[test]
fn multipage_tiff_reports_one_source_event_and_preserves_layout_page_semantics() {
    let tiff = multipage_tiff();

    for (layout, expected_pages, expected_sizes) in [
        (pixel_layout(), 2, vec![(2., 1.), (1., 2.)]),
        (source_layout(), 1, vec![(2., 1.)]),
    ] {
        let mut seen = Vec::new();
        let output = prepare_image_imports_observed(
            &[("two.tiff", tiff.as_slice())],
            layout,
            || false,
            |progress| seen.push(progress),
        )
        .unwrap();

        assert_eq!(
            seen,
            [ImageImportProgress {
                current: 1,
                total: 1
            }]
        );
        let info = Session::open(&output[0]).unwrap().info();
        assert_eq!(info.page_count, expected_pages);
        assert_eq!(
            info.pages
                .iter()
                .map(|page| (page.width, page.height))
                .collect::<Vec<_>>(),
            expected_sizes
        );
    }
}

#[test]
fn a4_multipage_tiff_reports_one_source_event_and_matches_legacy_bytes() {
    let tiff = multipage_tiff();
    let files = [("two.tiff", tiff.as_slice())];
    let layout = ImageImportLayout::default();
    let mut seen = Vec::new();

    let observed =
        prepare_image_imports_observed(&files, layout, || false, |progress| seen.push(progress))
            .unwrap();
    let cancellable = prepare_image_imports_cancellable(&files, layout, || false).unwrap();
    let legacy = prepare_image_imports(&files, layout).unwrap();

    assert_eq!(
        seen,
        [ImageImportProgress {
            current: 1,
            total: 1
        }]
    );
    assert_eq!(observed, cancellable);
    assert_eq!(observed, legacy);
    let info = Session::open(&observed[0]).unwrap().info();
    assert_eq!(info.page_count, 2);
    for page in info.pages {
        assert!((page.width - 595.276).abs() < 0.01, "width={}", page.width);
        assert!(
            (page.height - 841.89).abs() < 0.01,
            "height={}",
            page.height
        );
    }
}

#[test]
fn cancellation_requested_by_first_event_aborts_the_entire_batch() {
    let first = png();
    let second = png();
    let files = [
        ("first.png", first.as_slice()),
        ("second.png", second.as_slice()),
    ];
    let stop = Cell::new(false);
    let mut seen = Vec::new();

    let result = prepare_image_imports_observed(
        &files,
        pixel_layout(),
        || stop.get(),
        |progress| {
            seen.push(progress);
            stop.set(true);
        },
    );

    assert!(matches!(result, Err(PdfMasterError::Cancelled)));
    assert_eq!(
        seen,
        [ImageImportProgress {
            current: 1,
            total: 2
        }]
    );
}

#[test]
fn preflight_failures_and_pre_cancellation_emit_no_events() {
    let good = png();
    let bad = b"not an image";
    let too_many = vec![("same.png", good.as_slice()); 33];
    let invalid_layout = ImageImportLayout {
        paper: ImagePaper::A4,
        landscape: false,
        margin: 300.,
    };
    let single = [("good.png", good.as_slice())];
    let late_bad = [("good.png", good.as_slice()), ("bad.png", bad.as_slice())];
    let unsupported = [("good.gif", good.as_slice())];

    assert!(matches!(
        preflight_error(&single, pixel_layout(), true),
        PdfMasterError::Cancelled
    ));
    assert!(matches!(
        preflight_error(&single, invalid_layout, false),
        PdfMasterError::InvalidGeometry("image paper margin")
    ));
    assert!(matches!(
        preflight_error(&late_bad, pixel_layout(), false),
        PdfMasterError::InvalidOperation(_)
    ));
    assert!(matches!(
        preflight_error(&unsupported, pixel_layout(), false),
        PdfMasterError::InvalidOperation(_)
    ));
    assert!(matches!(
        preflight_error(&too_many, pixel_layout(), false),
        PdfMasterError::Budget("import file count")
    ));
}

#[test]
fn observed_cancellable_and_legacy_outputs_are_byte_identical() {
    let png = png();
    let tiff = multipage_tiff();
    let files = [("own.png", png.as_slice()), ("two.tiff", tiff.as_slice())];

    for layout in [pixel_layout(), source_layout()] {
        let observed = prepare_image_imports_observed(&files, layout, || false, |_| {}).unwrap();
        let cancellable = prepare_image_imports_cancellable(&files, layout, || false).unwrap();
        let legacy = prepare_image_imports(&files, layout).unwrap();

        assert_eq!(observed, cancellable);
        assert_eq!(observed, legacy);
        assert_eq!(Session::open(&observed[0]).unwrap().info().page_count, 1);
        assert_eq!(
            Session::open(&observed[1]).unwrap().info().page_count,
            if layout.paper == ImagePaper::Source {
                1
            } else {
                2
            }
        );
    }
}
