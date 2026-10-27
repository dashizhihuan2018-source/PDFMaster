use pdfmaster_core::{
    ImageImportLayout, ImagePaper, PdfMasterError, Session, prepare_image_imports,
    prepare_image_imports_cancellable,
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

fn multipage_tiff() -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    {
        let mut encoder = tiff::encoder::TiffEncoder::new(&mut bytes).unwrap();
        encoder
            .write_image::<tiff::encoder::colortype::RGB8>(2, 1, &[255, 0, 0, 0, 255, 0])
            .unwrap();
        encoder
            .write_image::<tiff::encoder::colortype::RGB8>(1, 2, &[0, 0, 255, 255, 255, 0])
            .unwrap();
    }
    bytes.into_inner()
}

fn pixel_layout() -> ImageImportLayout {
    ImageImportLayout {
        paper: ImagePaper::Pixels,
        landscape: false,
        margin: 0.,
    }
}

#[test]
fn pre_cancelled_image_batch_returns_cancelled() {
    let png = png();
    let result = prepare_image_imports_cancellable(&[("own.png", &png)], pixel_layout(), || true);

    assert!(matches!(result, Err(PdfMasterError::Cancelled)));
}

#[test]
fn disabled_cancellation_matches_legacy_bytes_and_geometry_for_png_and_tiff() {
    let png = png();
    let tiff = multipage_tiff();
    let files = [("own.png", png.as_slice()), ("two.tiff", tiff.as_slice())];

    let expected = prepare_image_imports(&files, pixel_layout()).unwrap();
    let actual = prepare_image_imports_cancellable(&files, pixel_layout(), || false).unwrap();

    assert_eq!(actual, expected);
    assert_eq!(Session::open(&actual[0]).unwrap().info().page_count, 1);
    let tiff_info = Session::open(&actual[1]).unwrap().info();
    assert_eq!(tiff_info.page_count, 2);
    assert_eq!(
        tiff_info
            .pages
            .iter()
            .map(|page| (page.width, page.height))
            .collect::<Vec<_>>(),
        [(2., 1.), (1., 2.)]
    );
}

#[test]
fn every_observed_checkpoint_cancels_the_entire_multifile_batch() {
    let png = png();
    let tiff = multipage_tiff();
    let files = [("own.png", png.as_slice()), ("two.tiff", tiff.as_slice())];
    let checks = Cell::new(0usize);

    let completed = prepare_image_imports_cancellable(&files, pixel_layout(), || {
        checks.set(checks.get() + 1);
        false
    })
    .unwrap();
    assert_eq!(completed.len(), 2);
    assert!(
        checks.get() > 1,
        "the pipeline must expose multiple safe checkpoints"
    );

    for cancel_at in 1..=checks.get() {
        let seen = Cell::new(0usize);
        let result = prepare_image_imports_cancellable(&files, pixel_layout(), || {
            seen.set(seen.get() + 1);
            seen.get() == cancel_at
        });
        assert!(
            matches!(result, Err(PdfMasterError::Cancelled)),
            "checkpoint {cancel_at} returned a usable partial batch"
        );
    }

    let seen = Cell::new(0usize);
    let result = prepare_image_imports_cancellable(&files, pixel_layout(), || {
        seen.set(seen.get() + 1);
        seen.get() == checks.get() + 1
    })
    .unwrap();
    assert_eq!(
        result.len(),
        2,
        "cancelling after the last checkpoint must finish"
    );
}
