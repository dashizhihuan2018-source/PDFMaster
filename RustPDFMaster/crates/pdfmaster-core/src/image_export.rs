use std::io::{self, Write};

use image::ExtendedColorType;
use image::codecs::jpeg::{JpegEncoder, PixelDensity};
use serde::{Deserialize, Serialize};

use crate::{PdfMasterError, Result, Session};

const MIN_DPI: f64 = 36.0;
const MAX_DPI: f64 = 600.0;
const MAX_PAGE_PIXELS: u64 = 32_000_000;
const MAX_TOTAL_PIXELS: u64 = 100_000_000;
const MAX_OUTPUT_BYTES: usize = 512 * 1024 * 1024;
const MAX_DIMENSION: u32 = 20_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageExportFormat {
    Png,
    Jpeg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageExportLimits {
    pub page_pixels: u64,
    pub total_pixels: u64,
    pub output_bytes: usize,
}

impl Default for ImageExportLimits {
    fn default() -> Self {
        Self {
            page_pixels: MAX_PAGE_PIXELS,
            total_pixels: MAX_TOTAL_PIXELS,
            output_bytes: MAX_OUTPUT_BYTES,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageExportRequest {
    pub format: ImageExportFormat,
    pub dpi: f64,
    pub pages: Vec<usize>,
    pub jpeg_quality: f32,
    pub limits: ImageExportLimits,
}

impl Default for ImageExportRequest {
    fn default() -> Self {
        Self {
            format: ImageExportFormat::Png,
            dpi: 144.0,
            pages: Vec::new(),
            jpeg_quality: 0.9,
            limits: ImageExportLimits::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportedImage {
    pub page: usize,
    pub filename: String,
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageExportSummary {
    pub pages: usize,
    pub total_bytes: usize,
}

impl Session {
    pub fn export_images(
        &self,
        request: &ImageExportRequest,
        cancelled: impl Fn() -> bool,
        mut emit: impl FnMut(ExportedImage) -> Result<()>,
    ) -> Result<ImageExportSummary> {
        if cancelled() {
            return Err(PdfMasterError::Cancelled);
        }
        validate_request(request)?;

        let mut pages = request.pages.clone();
        pages.sort_unstable();
        pages.dedup();

        let mut plans = Vec::with_capacity(pages.len());
        let mut total_pixels = 0_u64;
        for page in pages {
            if cancelled() {
                return Err(PdfMasterError::Cancelled);
            }
            self.preflight_render_page(page)?;
            let (width, height) = self.export_render_dimensions(page, request.dpi)?;
            if width > MAX_DIMENSION || height > MAX_DIMENSION {
                return Err(PdfMasterError::Budget("image export longest edge"));
            }
            let pixels = u64::from(width)
                .checked_mul(u64::from(height))
                .ok_or(PdfMasterError::Budget("image export page pixels"))?;
            if pixels > request.limits.page_pixels {
                return Err(PdfMasterError::Budget("image export page pixels"));
            }
            total_pixels = total_pixels
                .checked_add(pixels)
                .ok_or(PdfMasterError::Budget("image export total pixels"))?;
            if total_pixels > request.limits.total_pixels {
                return Err(PdfMasterError::Budget("image export total pixels"));
            }
            plans.push((page, width, height));
            if cancelled() {
                return Err(PdfMasterError::Cancelled);
            }
        }

        if cancelled() {
            return Err(PdfMasterError::Cancelled);
        }
        let page_count = plans.len();
        let mut total_bytes = 0_usize;
        for (page, width, height) in plans {
            if cancelled() {
                return Err(PdfMasterError::Cancelled);
            }
            let pixmap = self.render_pixmap_exact(page, width, height, request.dpi)?;
            if cancelled() {
                return Err(PdfMasterError::Cancelled);
            }
            let remaining = request
                .limits
                .output_bytes
                .checked_sub(total_bytes)
                .ok_or(PdfMasterError::Budget("image export output bytes"))?;
            let bytes = match request.format {
                ImageExportFormat::Png => encode_png(&pixmap, request.dpi, remaining)?,
                ImageExportFormat::Jpeg => {
                    encode_jpeg(&pixmap, request.dpi, request.jpeg_quality, remaining)?
                }
            };
            if cancelled() {
                return Err(PdfMasterError::Cancelled);
            }
            total_bytes = total_bytes
                .checked_add(bytes.len())
                .ok_or(PdfMasterError::Budget("image export output bytes"))?;
            let extension = match request.format {
                ImageExportFormat::Png => "png",
                ImageExportFormat::Jpeg => "jpg",
            };
            emit(ExportedImage {
                page,
                filename: format!("page-{:04}.{extension}", page + 1),
                width,
                height,
                bytes,
            })?;
            if cancelled() {
                return Err(PdfMasterError::Cancelled);
            }
        }
        if cancelled() {
            return Err(PdfMasterError::Cancelled);
        }
        Ok(ImageExportSummary {
            pages: page_count,
            total_bytes,
        })
    }
}

fn validate_request(request: &ImageExportRequest) -> Result<()> {
    if !request.dpi.is_finite() || !(MIN_DPI..=MAX_DPI).contains(&request.dpi) {
        return Err(PdfMasterError::InvalidGeometry(
            "image export DPI must be finite and in 36..=600",
        ));
    }
    if !request.jpeg_quality.is_finite() || !(0.0..=1.0).contains(&request.jpeg_quality) {
        return Err(PdfMasterError::InvalidGeometry(
            "JPEG quality must be finite and in 0..=1",
        ));
    }
    if request.pages.is_empty() {
        return Err(PdfMasterError::InvalidOperation(
            "image export page selection must not be empty",
        ));
    }
    let limits = request.limits;
    if limits.page_pixels == 0
        || limits.total_pixels == 0
        || limits.output_bytes == 0
        || limits.page_pixels > MAX_PAGE_PIXELS
        || limits.total_pixels > MAX_TOTAL_PIXELS
        || limits.output_bytes > MAX_OUTPUT_BYTES
    {
        return Err(PdfMasterError::InvalidOperation(
            "image export limits exceed the supported range",
        ));
    }
    Ok(())
}

fn encode_png(pixmap: &hayro::vello_cpu::Pixmap, dpi: f64, limit: usize) -> Result<Vec<u8>> {
    let mut sink = LimitedWriter::new(limit);
    let encoded = (|| -> std::result::Result<(), png::EncodingError> {
        let mut encoder = png::Encoder::new(
            &mut sink,
            u32::from(pixmap.width()),
            u32::from(pixmap.height()),
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let pixels_per_meter = (dpi / 0.0254).round() as u32;
        encoder.set_pixel_dims(Some(png::PixelDimensions {
            xppu: pixels_per_meter,
            yppu: pixels_per_meter,
            unit: png::Unit::Meter,
        }));
        let mut writer = encoder.write_header()?;
        writer.write_image_data(pixmap.data_as_u8_slice())?;
        writer.finish()?;
        Ok(())
    })();
    match encoded {
        Ok(()) => Ok(sink.into_inner()),
        Err(_) if sink.exceeded => Err(PdfMasterError::Budget("image export output bytes")),
        Err(error) => Err(PdfMasterError::Render(format!(
            "PNG encoding failed: {error}"
        ))),
    }
}

fn encode_jpeg(
    pixmap: &hayro::vello_cpu::Pixmap,
    dpi: f64,
    quality: f32,
    limit: usize,
) -> Result<Vec<u8>> {
    let mut rgb = Vec::with_capacity(pixmap.data_as_u8_slice().len() / 4 * 3);
    for rgba in pixmap.data_as_u8_slice().chunks_exact(4) {
        rgb.extend_from_slice(&rgba[..3]);
    }
    let mut sink = LimitedWriter::new(limit);
    let encoded = {
        let quality = (quality * 100.0).round().clamp(1.0, 100.0) as u8;
        let mut encoder = JpegEncoder::new_with_quality(&mut sink, quality);
        encoder.set_pixel_density(PixelDensity::dpi(dpi.round() as u16));
        encoder.encode(
            &rgb,
            u32::from(pixmap.width()),
            u32::from(pixmap.height()),
            ExtendedColorType::Rgb8,
        )
    };
    match encoded {
        Ok(()) => Ok(sink.into_inner()),
        Err(_) if sink.exceeded => Err(PdfMasterError::Budget("image export output bytes")),
        Err(error) => Err(PdfMasterError::Render(format!(
            "JPEG encoding failed: {error}"
        ))),
    }
}

struct LimitedWriter {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}

impl LimitedWriter {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(limit.min(64 * 1024)),
            limit,
            exceeded: false,
        }
    }

    fn into_inner(self) -> Vec<u8> {
        self.bytes
    }
}

impl Write for LimitedWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let remaining = self.limit.saturating_sub(self.bytes.len());
        if buffer.len() > remaining {
            self.exceeded = true;
            return Err(io::Error::other("image export output byte limit exceeded"));
        }
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
