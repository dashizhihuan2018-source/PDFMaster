use pdfmaster_core::{Color, Edit, Rect, Session, ShapeKind};
use std::io::Cursor;

fn fixture() -> Session {
    let mut s = Session::blank(100., 80., 2).unwrap();
    for (page, color) in [
        (
            0,
            Color {
                r: 1.,
                g: 0.,
                b: 0.,
            },
        ),
        (
            1,
            Color {
                r: 0.,
                g: 0.,
                b: 1.,
            },
        ),
    ] {
        s.apply(Edit::AddShape {
            page,
            bounds: Rect {
                x: 10.,
                y: 10.,
                width: 70.,
                height: 50.,
            },
            shape: ShapeKind::Rectangle,
            stroke: color,
            fill: Some(color),
            line_width: 2.,
        })
        .unwrap();
    }
    s.apply(Edit::Rotate {
        page: 1,
        degrees: 90,
    })
    .unwrap();
    s
}

#[test]
fn page_copy_provides_source_order_pdf_and_real_multiformat_annotation_pixels_without_editing() {
    let s = fixture();
    let before = s.bytes().unwrap();
    let info = s.info();
    let copy = s.page_clipboard(&[1, 0]).unwrap();
    assert_eq!(Session::open(&copy.pdf).unwrap().info().page_count, 2);
    assert_eq!(
        Session::open(&copy.pdf).unwrap().info().pages[1].rotation,
        90
    );
    assert_eq!((copy.width, copy.height), (300, 576));
    let mut reader = png::Decoder::new(Cursor::new(&copy.png))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let frame = reader.next_frame(&mut pixels).unwrap();
    assert_eq!((frame.width, frame.height), (copy.width, copy.height));
    let stride = frame.color_type.samples();
    let colored = |range: std::ops::Range<usize>, r: usize, b: usize| {
        pixels[range]
            .chunks_exact(stride)
            .any(|p| p[r] > 200 && p[1] < 60 && p[b] < 60)
    };
    let half = copy.width as usize * copy.height as usize / 2 * stride;
    assert!(
        colored(0..half, 0, 2),
        "red first-page annotation must be rasterized"
    );
    assert!(
        colored(half..pixels.len(), 2, 0),
        "blue rotated second page must not disappear"
    );
    assert_eq!(&copy.tiff[..4], b"II\x2a\0");
    assert_eq!(u32::from_le_bytes(copy.dib[..4].try_into().unwrap()), 40);
    assert_eq!(i32::from_le_bytes(copy.dib[4..8].try_into().unwrap()), 300);
    assert_eq!(
        i32::from_le_bytes(copy.dib[8..12].try_into().unwrap()),
        -576
    );
    assert_eq!(s.bytes().unwrap(), before);
    assert_eq!(s.info(), info);
}

#[test]
fn invalid_clipboard_selections_do_not_consume_history_or_redo() {
    let mut s = fixture();
    s.undo().unwrap();
    let before = s.bytes().unwrap();
    let info = s.info();
    for pages in [&[][..], &[0, 0], &[2], &[usize::MAX]] {
        assert!(s.page_clipboard(pages).is_err());
    }
    assert_eq!(s.bytes().unwrap(), before);
    assert_eq!(s.info(), info);
    s.redo().unwrap();
}

#[test]
fn clipboard_quantizes_scale_like_the_renderer_at_the_8192_pixel_boundary() {
    let s = Session::blank(933., 17841., 1).unwrap();
    let copy = s.page_clipboard(&[0]).unwrap();
    assert!(copy.width <= 8192 && copy.height <= 8192);
    assert!(u64::from(copy.width) * u64::from(copy.height) <= 16_777_216);
}
