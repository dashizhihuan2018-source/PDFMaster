use crate::{PdfMasterError, Result, Session};
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, metadata::Orientation};
use lopdf::{Object, Stream, dictionary};
use std::io::Cursor;

const FRAME_PIXELS: u64 = 40_000_000;
const TOTAL_PIXELS: u64 = 100_000_000;
const MAX_PAGES: usize = 500;
const DECODE_BYTES: u64 = 320 * 1024 * 1024;
/// Identifies the source image whose conversion is about to start.
///
/// `current` is one-based and `total` is the number of source files in the
/// batch. This is not a completion count, page count, byte count or percent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageImportProgress {
    pub current: usize,
    pub total: usize,
}
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImagePaper {
    /// Automatic image-tool geometry: oriented pixels at source horizontal DPI.
    Source,
    A4,
    Letter,
    Pixels,
}
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImageImportLayout {
    pub paper: ImagePaper,
    pub landscape: bool,
    pub margin: f64,
}
impl Default for ImageImportLayout {
    fn default() -> Self {
        Self {
            paper: ImagePaper::A4,
            landscape: false,
            margin: 24.,
        }
    }
}
impl ImageImportLayout {
    pub fn validate(self) -> Result<()> {
        if !matches!(self.paper, ImagePaper::Pixels | ImagePaper::Source)
            && (!self.margin.is_finite()
                || self.margin < 0.
                || self.margin * 2.
                    >= match self.paper {
                        ImagePaper::A4 => 595.276,
                        _ => 612.,
                    })
        {
            return Err(PdfMasterError::InvalidGeometry("image paper margin"));
        }
        Ok(())
    }
    fn page_with_density(self, w: u32, h: u32, dpi: Option<f64>) -> Result<(f64, f64, f64)> {
        self.validate()?;
        if self.paper == ImagePaper::Source {
            // Match the source metadata's floatValue before the double division.
            let dpi = f64::from(dpi.unwrap_or(72.) as f32);
            let (w, h) = (f64::from(w) * 72. / dpi, f64::from(h) * 72. / dpi);
            if !dpi.is_finite()
                || dpi <= 0.
                || !w.is_finite()
                || !h.is_finite()
                || w <= 0.
                || h <= 0.
                || w > 14_400.
                || h > 14_400.
            {
                return Err(PdfMasterError::Budget("image PDF page dimensions"));
            }
            return Ok((w, h, 0.));
        }
        if self.paper == ImagePaper::Pixels {
            if w == 0 || h == 0 || w > 14_400 || h > 14_400 {
                return Err(PdfMasterError::Budget("image PDF page dimensions"));
            }
            return Ok((f64::from(w), f64::from(h), 0.));
        }
        let (w, h) = match self.paper {
            ImagePaper::A4 => (595.276, 841.89),
            _ => (612., 792.),
        };
        Ok(if self.landscape {
            (h, w, self.margin)
        } else {
            (w, h, self.margin)
        })
    }
}
pub fn prepare_image_imports(
    files: &[(&str, &[u8])],
    layout: ImageImportLayout,
) -> Result<Vec<Vec<u8>>> {
    prepare_image_imports_cancellable(files, layout, || false)
}
pub fn prepare_image_imports_cancellable(
    files: &[(&str, &[u8])],
    layout: ImageImportLayout,
    cancelled: impl Fn() -> bool,
) -> Result<Vec<Vec<u8>>> {
    prepare_image_imports_observed(files, layout, cancelled, |_| {})
}
/// Prepares images while observing which source image is about to be processed.
///
/// Events are one-based source positions, not completion notifications.
pub fn prepare_image_imports_observed(
    files: &[(&str, &[u8])],
    layout: ImageImportLayout,
    cancelled: impl Fn() -> bool,
    mut progress: impl FnMut(ImageImportProgress),
) -> Result<Vec<Vec<u8>>> {
    check_cancelled(&cancelled)?;
    if files.iter().any(|(name, _)| {
        !matches!(
            name.rsplit('.')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase()
                .as_str(),
            "png" | "jpg" | "jpeg" | "tif" | "tiff"
        )
    }) {
        return Err(invalid());
    }
    check_cancelled(&cancelled)?;
    prepare_with_layout(files, layout, &cancelled, &mut progress)
}
fn check_cancelled(cancelled: &dyn Fn() -> bool) -> Result<()> {
    if cancelled() {
        Err(PdfMasterError::Cancelled)
    } else {
        Ok(())
    }
}
enum Plan {
    Pdf,
    Image(
        ImageFormat,
        Vec<Option<Box<moxcms::ColorProfile>>>,
        Option<Orientation>,
        Vec<Option<f64>>,
    ),
}
fn invalid() -> PdfMasterError {
    PdfMasterError::InvalidOperation("image format, color model, animation or data is unsupported")
}
fn limits() -> image::Limits {
    let mut l = image::Limits::default();
    l.max_image_width = Some(40_000_000);
    l.max_image_height = Some(40_000_000);
    l.max_alloc = Some(DECODE_BYTES);
    l
}
fn decoder(bytes: &[u8], format: ImageFormat) -> Result<impl ImageDecoder + '_> {
    let mut r = ImageReader::with_format(Cursor::new(bytes), format);
    r.limits(limits());
    r.into_decoder().map_err(|_| invalid())
}
fn unsupported_color() -> PdfMasterError {
    crate::import_color::unsupported()
}
#[derive(Default)]
struct ImageMetadata {
    profile: Option<Box<moxcms::ColorProfile>>,
    orientation: Option<Orientation>,
    dpi: Option<f64>,
}
fn color_preflight(bytes: &[u8], format: ImageFormat, total: &mut usize) -> Result<ImageMetadata> {
    if format == ImageFormat::Png {
        return crate::import_color::png(bytes, total).map(|metadata| ImageMetadata {
            profile: metadata.profile,
            orientation: metadata.orientation,
            dpi: metadata.dpi,
        });
    }
    if format == ImageFormat::Bmp {
        let header = bytes.get(14..18).ok_or_else(invalid)?;
        if u32::from_le_bytes(header.try_into().map_err(|_| invalid())?) >= 108 {
            return Err(unsupported_color());
        }
    }
    if format == ImageFormat::Jpeg {
        let metadata = crate::import_jpeg::preflight(bytes, total)?;
        return Ok(ImageMetadata {
            profile: metadata.profile,
            orientation: Some(metadata.orientation),
            dpi: metadata.dpi,
        });
    }
    Ok(ImageMetadata::default())
}
fn pixels(w: u32, h: u32, total: &mut u64) -> Result<()> {
    let n = u64::from(w) * u64::from(h);
    if n == 0 || n > FRAME_PIXELS {
        return Err(PdfMasterError::Budget("image frame pixels"));
    }
    *total = total
        .checked_add(n)
        .ok_or(PdfMasterError::Budget("image total pixels"))?;
    if *total > TOTAL_PIXELS {
        return Err(PdfMasterError::Budget("image total pixels"));
    }
    Ok(())
}
fn tiff_decoder(bytes: &[u8]) -> Result<tiff::decoder::Decoder<Cursor<&[u8]>>> {
    let mut l = tiff::decoder::Limits::default();
    l.decoding_buffer_size = DECODE_BYTES as usize;
    l.intermediate_buffer_size = 64 * 1024 * 1024;
    tiff::decoder::Decoder::new(Cursor::new(bytes))
        .map(|d| d.with_limits(l))
        .map_err(|_| invalid())
}
fn tiff_properties(
    d: &mut tiff::decoder::Decoder<Cursor<&[u8]>>,
) -> Result<(u32, u32, usize, Orientation, bool)> {
    use tiff::{ColorType as C, tags::Tag};
    for tag in [
        Tag::ExifDirectory,
        Tag::Unknown(301),
        Tag::Unknown(318),
        Tag::Unknown(319),
        Tag::Unknown(342),
        Tag::Unknown(532),
    ] {
        if d.find_tag(tag).map_err(|_| invalid())?.is_some() {
            return Err(unsupported_color());
        }
    }
    if d.find_tag_unsigned::<u16>(Tag::PlanarConfiguration)
        .map_err(|_| invalid())?
        .unwrap_or(1)
        != 1
    {
        return Err(invalid());
    }
    let channels = match d.colortype().map_err(|_| invalid())? {
        C::Gray(8 | 16) => 1,
        C::GrayA(8 | 16) => 2,
        C::Multiband {
            bit_depth: 8 | 16,
            num_samples: 2,
        } if d
            .find_tag_unsigned::<u16>(Tag::PhotometricInterpretation)
            .map_err(|_| invalid())?
            == Some(1)
            && d.find_tag_unsigned::<u16>(Tag::SamplesPerPixel)
                .map_err(|_| invalid())?
                == Some(2) =>
        {
            2
        }
        C::RGB(8 | 16) => 3,
        C::RGBA(8 | 16) => 4,
        _ => return Err(invalid()),
    };
    let exif = d
        .find_tag_unsigned::<u8>(Tag::Orientation)
        .map_err(|_| invalid())?
        .unwrap_or(1);
    let orientation = Orientation::from_exif(exif).ok_or_else(invalid)?;
    let extra = d
        .find_tag_unsigned_vec::<u16>(Tag::ExtraSamples)
        .map_err(|_| invalid())?;
    let associated = if channels == 2 || channels == 4 {
        match extra.as_deref() {
            Some([1]) => true,
            Some([2]) => false,
            _ => return Err(invalid()),
        }
    } else {
        false
    };
    let (w, h) = d.dimensions().map_err(|_| invalid())?;
    Ok((w, h, channels, orientation, associated))
}
fn tiff_color(
    d: &mut tiff::decoder::Decoder<Cursor<&[u8]>>,
    gray: bool,
    total: &mut usize,
) -> Result<Option<Box<moxcms::ColorProfile>>> {
    use tiff::tags::{Tag, Type};
    let mut ifd = d.image_ifd();
    let Some(entry) = ifd.find_entry(Tag::IccProfile) else {
        return Ok(None);
    };
    if entry.count() > crate::import_color::PROFILE_BYTES as u64 {
        return Err(PdfMasterError::Budget("image color metadata"));
    }
    if !matches!(entry.field_type(), Type::BYTE | Type::UNDEFINED) {
        return Err(unsupported_color());
    }
    let mut bytes = vec![0; entry.count() as usize];
    if ifd
        .find_tag_bytes(Tag::IccProfile, &mut bytes, 0)
        .map_err(|_| invalid())?
        != Some(bytes.len())
    {
        return Err(invalid());
    }
    crate::import_color::icc(&bytes, gray, total).map(Some)
}
fn tiff_dpi(d: &mut tiff::decoder::Decoder<Cursor<&[u8]>>) -> Result<Option<f64>> {
    use tiff::{decoder::ifd::Value, tags::Tag};
    let unit = d
        .find_tag_unsigned::<u16>(Tag::ResolutionUnit)
        .map_err(|_| invalid())?
        .unwrap_or(2);
    if !matches!(unit, 1..=3) {
        return Err(invalid());
    }
    let mut x = None;
    let mut y = None;
    for tag in [Tag::XResolution, Tag::YResolution] {
        match d.find_tag(tag).map_err(|_| invalid())? {
            None => {}
            Some(Value::Rational(n, denominator)) if denominator != 0 => {
                let value = Some(f64::from(n) / f64::from(denominator));
                if tag == Tag::XResolution {
                    x = value;
                } else {
                    y = value;
                }
            }
            _ => return Err(invalid()),
        }
    }
    let base = if x == Some(0.) || y == Some(0.) {
        Some(72.)
    } else {
        x.or(y)
    };
    Ok(base.map(|x| if unit == 3 { (x * 2.54).round() } else { x }))
}
/// Preflights the whole batch before any pixel decoding or output publication.
/// Limits supplement decoder-specific limits, not a promise of total heap use.
pub fn prepare_imports(files: &[(&str, &[u8])]) -> Result<Vec<Vec<u8>>> {
    prepare_with_layout(files, ImageImportLayout::default(), &|| false, &mut |_| {})
}
fn prepare_with_layout(
    files: &[(&str, &[u8])],
    layout: ImageImportLayout,
    cancelled: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(ImageImportProgress),
) -> Result<Vec<Vec<u8>>> {
    check_cancelled(cancelled)?;
    layout.validate()?;
    if files.is_empty() || files.len() > crate::MAX_INSERT_FILES {
        return Err(PdfMasterError::Budget("import file count"));
    }
    let mut bytes_total = 0usize;
    let mut total_pixels = 0;
    let mut color_bytes = 0;
    let mut pages = 0usize;
    let mut plans = vec![];
    for (name, bytes) in files {
        check_cancelled(cancelled)?;
        bytes_total = bytes_total
            .checked_add(bytes.len())
            .ok_or(PdfMasterError::Budget("import input bytes"))?;
        if bytes.len() > crate::document::MAX_INPUT_BYTES || bytes_total > crate::MAX_INSERT_BYTES {
            return Err(PdfMasterError::Budget("import input bytes"));
        }
        let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        if ext == "pdf" {
            pages += Session::open(bytes)?.info().page_count;
            plans.push(Plan::Pdf);
        } else {
            let format = match ext.as_str() {
                "png" => ImageFormat::Png,
                "jpg" | "jpeg" => ImageFormat::Jpeg,
                "bmp" => ImageFormat::Bmp,
                "gif" => ImageFormat::Gif,
                "tif" | "tiff" => ImageFormat::Tiff,
                _ => return Err(invalid()),
            };
            let ImageMetadata {
                profile: color,
                orientation,
                dpi,
            } = color_preflight(bytes, format, &mut color_bytes)?;
            let mut colors = vec![];
            let mut densities = vec![];
            let mut count = 1;
            if format == ImageFormat::Tiff {
                let mut d = tiff_decoder(bytes)?;
                count = 0;
                loop {
                    check_cancelled(cancelled)?;
                    let (w, h, channels, orientation, _) = tiff_properties(&mut d)?;
                    colors.push(tiff_color(&mut d, channels <= 2, &mut color_bytes)?);
                    let dpi = if layout.paper == ImagePaper::Source {
                        tiff_dpi(&mut d)?
                    } else {
                        None
                    };
                    densities.push(dpi);
                    pixels(w, h, &mut total_pixels)?;
                    let (w, h) = oriented_size(w, h, orientation);
                    layout.page_with_density(w, h, dpi)?;
                    count += 1;
                    if count + pages > MAX_PAGES {
                        return Err(PdfMasterError::Budget("import pages"));
                    }
                    check_cancelled(cancelled)?;
                    if layout.paper == ImagePaper::Source || !d.more_images() {
                        break;
                    }
                    d.next_image().map_err(|_| invalid())?;
                }
            } else {
                if format == ImageFormat::Png
                    && image::codecs::png::PngDecoder::with_limits(Cursor::new(bytes), limits())
                        .map_err(|_| invalid())?
                        .is_apng()
                        .map_err(|_| invalid())?
                {
                    return Err(invalid());
                }
                if format == ImageFormat::Gif {
                    let mut options = gif::DecodeOptions::new();
                    options.check_frame_consistency(true);
                    options.skip_frame_decoding(true);
                    let mut gif = options
                        .read_info(Cursor::new(bytes))
                        .map_err(|_| invalid())?;
                    if gif.read_next_frame().map_err(|_| invalid())?.is_none()
                        || gif.read_next_frame().map_err(|_| invalid())?.is_some()
                    {
                        return Err(invalid());
                    }
                    if gif.icc_profile().is_some() {
                        return Err(unsupported_color());
                    }
                }
                let d = decoder(bytes, format)?;
                let (w, h) = d.dimensions();
                pixels(w, h, &mut total_pixels)?;
                let (w, h) = oriented_size(w, h, orientation.unwrap_or(Orientation::NoTransforms));
                layout.page_with_density(w, h, dpi)?;
                colors.push(color);
                densities.push(dpi);
            }
            pages += count;
            plans.push(Plan::Image(format, colors, orientation, densities));
        }
        if pages > MAX_PAGES {
            return Err(PdfMasterError::Budget("import pages"));
        }
        check_cancelled(cancelled)?;
    }
    let mut output = vec![];
    let mut output_bytes = 0usize;
    for (index, ((_, bytes), plan)) in files.iter().zip(plans).enumerate() {
        check_cancelled(cancelled)?;
        progress(ImageImportProgress {
            current: index + 1,
            total: files.len(),
        });
        check_cancelled(cancelled)?;
        let converted = match plan {
            Plan::Pdf => bytes.to_vec(),
            Plan::Image(format, colors, orientation, densities) => image_pdf(
                bytes,
                format,
                colors,
                orientation,
                densities,
                layout,
                cancelled,
            )?,
        };
        check_cancelled(cancelled)?;
        output_bytes = output_bytes
            .checked_add(converted.len())
            .ok_or(PdfMasterError::Budget("import output bytes"))?;
        if output_bytes > crate::MAX_INSERT_BYTES {
            return Err(PdfMasterError::Budget("import output bytes"));
        }
        output.push(converted);
    }
    check_cancelled(cancelled)?;
    Ok(output)
}
fn oriented_size(w: u32, h: u32, orientation: Orientation) -> (u32, u32) {
    match orientation {
        Orientation::Rotate90
        | Orientation::Rotate270
        | Orientation::Rotate90FlipH
        | Orientation::Rotate270FlipH => (h, w),
        _ => (w, h),
    }
}
fn image_pdf(
    bytes: &[u8],
    format: ImageFormat,
    colors: Vec<Option<Box<moxcms::ColorProfile>>>,
    orientation: Option<Orientation>,
    densities: Vec<Option<f64>>,
    layout: ImageImportLayout,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<u8>> {
    check_cancelled(cancelled)?;
    let count = colors.len();
    let mut session = Session::blank(595.276, 841.89, count)?;
    let ids = crate::document::page_object_ids(&session.document)?;
    let mut tiff = if format == ImageFormat::Tiff {
        Some(tiff_decoder(bytes)?)
    } else {
        None
    };
    for (index, page) in ids.into_iter().enumerate() {
        check_cancelled(cancelled)?;
        let image = if let Some(d) = &mut tiff {
            let (w, h, channels, orientation, associated) = tiff_properties(d)?;
            let raw = match d.read_image().map_err(|_| invalid())? {
                tiff::decoder::DecodingResult::U8(raw) => {
                    raw.into_iter().map(|v| u16::from(v) * 257).collect()
                }
                tiff::decoder::DecodingResult::U16(raw) => raw,
                _ => return Err(invalid()),
            };
            check_cancelled(cancelled)?;
            if raw.len() != w as usize * h as usize * channels {
                return Err(invalid());
            }
            let rgba = raw
                .chunks_exact(channels)
                .flat_map(|p| {
                    let mut rgba = match channels {
                        1 => [p[0], p[0], p[0], 65_535],
                        2 => [p[0], p[0], p[0], p[1]],
                        3 => [p[0], p[1], p[2], 65_535],
                        _ => [p[0], p[1], p[2], p[3]],
                    };
                    if associated {
                        let a = u64::from(rgba[3]);
                        for color in &mut rgba[..3] {
                            *color = (u64::from(*color) * 65_535 + a / 2)
                                .checked_div(a)
                                .unwrap_or(0)
                                .min(65_535) as u16;
                        }
                    }
                    rgba
                })
                .collect();
            let mut image = DynamicImage::ImageRgba16(
                image::ImageBuffer::from_raw(w, h, rgba).ok_or_else(invalid)?,
            );
            image.apply_orientation(orientation);
            check_cancelled(cancelled)?;
            if index + 1 < count {
                d.next_image().map_err(|_| invalid())?;
            }
            image
        } else {
            let mut d = decoder(bytes, format)?;
            let orientation = if let Some(orientation) = orientation {
                orientation
            } else {
                d.orientation().map_err(|_| invalid())?
            };
            let mut image = DynamicImage::from_decoder(d).map_err(|_| invalid())?;
            check_cancelled(cancelled)?;
            image.apply_orientation(orientation);
            check_cancelled(cancelled)?;
            image
        };
        let w = image.width();
        let h = image.height();
        let (paper_width, paper_height, margin) =
            layout.page_with_density(w, h, densities[index])?;
        let rgb: Vec<u8> = if let Some(profile) = &colors[index] {
            crate::import_color::normalize(image, profile)?
        } else {
            let rgba = image.into_rgba8().into_raw();
            rgba.chunks_exact(4)
                .flat_map(|p| {
                    let a = u32::from(p[3]);
                    [0, 1, 2].map(|i| ((u32::from(p[i]) * a + 255 * (255 - a) + 127) / 255) as u8)
                })
                .collect()
        };
        check_cancelled(cancelled)?;
        let mut stream = Stream::new(
            dictionary! {
                "Type"=>"XObject", "Subtype"=>"Image", "Width"=>i64::from(w), "Height"=>i64::from(h),
                "ColorSpace"=>"DeviceRGB", "BitsPerComponent"=>8,
            },
            rgb,
        );
        stream
            .compress()
            .map_err(|e| PdfMasterError::Serialization(e.to_string()))?;
        check_cancelled(cancelled)?;
        let image_id = session.document.add_object(stream);
        let scale = ((paper_width - 2. * margin) / f64::from(w))
            .min((paper_height - 2. * margin) / f64::from(h));
        let width = f64::from(w) * scale;
        let height = f64::from(h) * scale;
        let x = (paper_width - width) / 2.;
        let y = (paper_height - height) / 2.;
        let content = session.document.add_object(Stream::new(lopdf::Dictionary::new(),
            format!("q 1 1 1 rg 0 0 {paper_width} {paper_height} re f {width} 0 0 {height} {x} {y} cm /ImportedImage Do Q\n").into_bytes()));
        let page = session
            .document
            .get_object_mut(page)
            .and_then(Object::as_dict_mut)
            .map_err(|_| invalid())?;
        page.set("Contents", content);
        page.set(
            "MediaBox",
            vec![
                0.into(),
                0.into(),
                Object::Real(paper_width as f32),
                Object::Real(paper_height as f32),
            ],
        );
        page.set(
            "Resources",
            dictionary! {"XObject"=>dictionary!{"ImportedImage"=>image_id}},
        );
        check_cancelled(cancelled)?;
        let bytes = crate::document::serialize(&session.document)?;
        check_cancelled(cancelled)?;
        if bytes.len() > crate::document::MAX_INPUT_BYTES {
            return Err(PdfMasterError::Budget("image PDF bytes"));
        }
    }
    check_cancelled(cancelled)?;
    let bytes = crate::document::serialize(&session.document)?;
    check_cancelled(cancelled)?;
    Session::open(&bytes)?;
    check_cancelled(cancelled)?;
    Ok(bytes)
}
