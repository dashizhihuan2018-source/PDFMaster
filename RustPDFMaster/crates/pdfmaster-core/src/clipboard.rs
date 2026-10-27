use crate::{PdfMasterError, Result, Session};
use hayro::vello_cpu::Pixmap;

const MAX_PIXELS: u64 = 16_777_216;
const MAX_SIDE: f64 = 8192.;
const MAX_OUTPUT: usize = 224 * 1024 * 1024;

pub struct PageClipboard {
    pub pdf: Vec<u8>,
    pub png: Vec<u8>,
    pub tiff: Vec<u8>,
    pub dib: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl Session {
    /// Builds every representation before a platform may replace its clipboard.
    /// Source ordering/relationship admission is shared with read-only extraction.
    pub fn page_clipboard(&self, pages: &[usize]) -> Result<PageClipboard> {
        let pdf = self.extract_pages(pages)?;
        let selected = Session::open(&pdf)?;
        let sizes: Vec<_> = selected
            .info()
            .pages
            .iter()
            .map(|p| {
                if p.rotation.rem_euclid(180) == 90 {
                    (f64::from(p.height), f64::from(p.width))
                } else {
                    (f64::from(p.width), f64::from(p.height))
                }
            })
            .collect();
        let (width, height, scale) = dimensions(&sizes)?;
        let mut image = Pixmap::new(width as u16, height as u16);
        image.data_as_u8_slice_mut().fill(255);
        let mut top = 0.;
        for (page, &(_, h)) in sizes.iter().enumerate() {
            let rendered = selected.render_pixmap(page, scale as f32)?;
            let (w, h_px) = (
                usize::from(rendered.width()),
                usize::from(rendered.height()),
            );
            let x = (width as usize).saturating_sub(w) / 2;
            let y = (top * scale).floor() as usize;
            if x + w > width as usize || y + h_px > height as usize {
                return Err(PdfMasterError::Render(
                    "clipboard renderer dimensions disagree with page geometry".into(),
                ));
            }
            for row in 0..h_px {
                let start = ((y + row) * width as usize + x) * 4;
                image.data_as_u8_slice_mut()[start..start + w * 4]
                    .copy_from_slice(&rendered.data_as_u8_slice()[row * w * 4..(row + 1) * w * 4]);
            }
            top += h + 12.;
        }
        let rgba = image.data_as_u8_slice();
        if rgba.chunks_exact(4).any(|p| p[3] != 255) {
            return Err(PdfMasterError::Render(
                "clipboard white-background raster is not opaque".into(),
            ));
        }
        let tiff = tiff_rgb(width, height, rgba);
        let dib = dib_bgra(width, height, rgba);
        let png = image
            .into_png()
            .map_err(|e| PdfMasterError::Render(e.to_string()))?;
        if [pdf.len(), png.len(), tiff.len(), dib.len()]
            .into_iter()
            .sum::<usize>()
            > MAX_OUTPUT
        {
            return Err(PdfMasterError::Budget("clipboard encoded representations"));
        }
        Ok(PageClipboard {
            pdf,
            png,
            tiff,
            dib,
            width,
            height,
        })
    }
}

fn dimensions(sizes: &[(f64, f64)]) -> Result<(u32, u32, f64)> {
    if sizes.is_empty()
        || sizes.len() > 4096
        || sizes
            .iter()
            .any(|&(w, h)| !w.is_finite() || !h.is_finite() || w <= 0. || h <= 0.)
    {
        return Err(PdfMasterError::InvalidGeometry(
            "invalid clipboard page dimensions",
        ));
    }
    let width = sizes.iter().map(|p| p.0).fold(0., f64::max);
    let height = sizes.iter().map(|p| p.1).sum::<f64>() + (sizes.len() - 1) as f64 * 12.;
    let ideal = 3_f64
        .min(MAX_SIDE / width.max(height))
        .min((MAX_PIXELS as f64 / (width * height)).sqrt());
    let mut scale = ideal as f32;
    if f64::from(scale) > ideal {
        scale = scale.next_down();
    }
    // Match the renderer's f32 multiplication, including its truncation. A
    // theoretical f64 floor can be one pixel smaller than the actual raster.
    for _ in 0..8 {
        let mut w = 1_u32;
        let mut h = 1_u32;
        let mut top = 0.;
        for &(page_width, page_height) in sizes {
            w = w.max((page_width as f32 * scale) as u32);
            h = h
                .max((top * f64::from(scale)).floor() as u32 + (page_height as f32 * scale) as u32);
            top += page_height + 12.;
        }
        if w <= 8192 && h <= 8192 && u64::from(w) * u64::from(h) <= MAX_PIXELS {
            return Ok((w, h, f64::from(scale)));
        }
        let reduction = (MAX_SIDE / f64::from(w.max(h)))
            .min((MAX_PIXELS as f64 / (f64::from(w) * f64::from(h))).sqrt())
            .min(1.);
        scale = ((f64::from(scale) * reduction) as f32).next_down();
    }
    Err(PdfMasterError::Budget("clipboard raster dimensions"))
}

// Baseline TIFF 6.0: one uncompressed, top-to-bottom chunky RGB strip.
// Lossless but intentionally not byte-identical to AppKit's LZW encoder.
fn tiff_rgb(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let entries = [
        (256, 4, 1, width),
        (257, 4, 1, height),
        (258, 3, 3, 146),
        (259, 3, 1, 1),
        (262, 3, 1, 2),
        (273, 4, 1, 152),
        (274, 3, 1, 1),
        (277, 3, 1, 3),
        (278, 4, 1, height),
        (279, 4, 1, width * height * 3),
        (284, 3, 1, 1),
    ];
    let mut out = Vec::with_capacity(152 + width as usize * height as usize * 3);
    out.extend_from_slice(b"II\x2a\0\x08\0\0\0");
    out.extend_from_slice(&11_u16.to_le_bytes());
    for (tag, kind, count, value) in entries {
        out.extend_from_slice(&(tag as u16).to_le_bytes());
        out.extend_from_slice(&(kind as u16).to_le_bytes());
        out.extend_from_slice(&(count as u32).to_le_bytes());
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend_from_slice(&0_u32.to_le_bytes());
    out.extend_from_slice(&[8, 0, 8, 0, 8, 0]);
    for p in rgba.chunks_exact(4) {
        out.extend_from_slice(&p[..3]);
    }
    out
}
// CF_DIB uses a BITMAPINFOHEADER (no BMP file header), top-down BGRA rows.
fn dib_bgra(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = vec![0; 40];
    out[0..4].copy_from_slice(&40_u32.to_le_bytes());
    out[4..8].copy_from_slice(&(width as i32).to_le_bytes());
    out[8..12].copy_from_slice(&(-(height as i32)).to_le_bytes());
    out[12..14].copy_from_slice(&1_u16.to_le_bytes());
    out[14..16].copy_from_slice(&32_u16.to_le_bytes());
    out[20..24].copy_from_slice(&(width * height * 4).to_le_bytes());
    out.reserve(rgba.len());
    for p in rgba.chunks_exact(4) {
        out.extend_from_slice(&[p[2], p[1], p[0], 255]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_geometry_bounds_extreme_and_mixed_pages() {
        for sizes in [
            vec![(20_000., 20_000.); 4096],
            vec![(20_000., 1.)],
            vec![(1., 20_000.)],
            vec![(20_000., 20_000.)],
        ] {
            let (w, h, s) = dimensions(&sizes).unwrap();
            assert!(w <= 8192 && h <= 8192);
            assert!(u64::from(w) * u64::from(h) <= MAX_PIXELS);
            assert!(s > 0. && s <= 3.);
        }
        assert!(dimensions(&[]).is_err());
        assert!(dimensions(&[(f64::NAN, 1.)]).is_err());
    }
}
