use crate::{PdfMasterError, Result, Session};
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, metadata::Orientation};
use lopdf::{Object, Stream, dictionary};
use std::io::Cursor;

const FRAME_PIXELS: u64 = 40_000_000;
const TOTAL_PIXELS: u64 = 100_000_000;
const MAX_PAGES: usize = 500;
const DECODE_BYTES: u64 = 320 * 1024 * 1024;
enum Plan {
    Pdf,
    Image(ImageFormat, usize),
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
    PdfMasterError::InvalidOperation(
        "image color metadata requires color management not yet supported",
    )
}
// Do not reinterpret calibrated/profiled samples as uncalibrated RGB. The
// initial importer accepts untagged samples and the PNG sRGB declaration only;
// broader color management must be implemented before expanding this gate.
fn color_preflight(bytes: &[u8], format: ImageFormat) -> Result<()> {
    if format == ImageFormat::Png {
        if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Err(invalid());
        }
        let mut offset = 8usize;
        while let Some(header) = bytes.get(offset..offset.saturating_add(8)) {
            let size = u32::from_be_bytes(header[..4].try_into().map_err(|_| invalid())?) as usize;
            let end = offset
                .checked_add(size)
                .and_then(|n| n.checked_add(12))
                .ok_or_else(invalid)?;
            if end > bytes.len() {
                return Err(invalid());
            }
            if matches!(
                &header[4..],
                b"gAMA" | b"cHRM" | b"iCCP" | b"cICP" | b"mDCV" | b"cLLI" | b"eXIf"
            ) {
                return Err(unsupported_color());
            }
            if &header[4..] == b"IEND" {
                return if end == bytes.len() {
                    Ok(())
                } else {
                    Err(invalid())
                };
            }
            offset = end;
        }
        return Err(invalid());
    }
    if format == ImageFormat::Bmp {
        let header = bytes.get(14..18).ok_or_else(invalid)?;
        if u32::from_le_bytes(header.try_into().map_err(|_| invalid())?) >= 108 {
            return Err(unsupported_color());
        }
    }
    if format == ImageFormat::Jpeg {
        let mut d = decoder(bytes, format)?;
        // Exif may declare a non-sRGB color space. Until parsed/normalized,
        // reject it explicitly, rather than only applying its orientation.
        if d.icc_profile().map_err(|_| invalid())?.is_some()
            || d.exif_metadata().map_err(|_| invalid())?.is_some()
        {
            return Err(unsupported_color());
        }
    }
    Ok(())
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
        Tag::IccProfile,
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
/// Preflights the whole batch before any pixel decoding or output publication.
/// Limits supplement decoder-specific limits, not a promise of total heap use.
pub fn prepare_imports(files: &[(&str, &[u8])]) -> Result<Vec<Vec<u8>>> {
    if files.is_empty() || files.len() > crate::MAX_INSERT_FILES {
        return Err(PdfMasterError::Budget("import file count"));
    }
    let mut bytes_total = 0usize;
    let mut total_pixels = 0;
    let mut pages = 0usize;
    let mut plans = vec![];
    for (name, bytes) in files {
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
            color_preflight(bytes, format)?;
            let mut count = 1;
            if format == ImageFormat::Tiff {
                let mut d = tiff_decoder(bytes)?;
                count = 0;
                loop {
                    let (w, h, _, _, _) = tiff_properties(&mut d)?;
                    pixels(w, h, &mut total_pixels)?;
                    count += 1;
                    if count + pages > MAX_PAGES {
                        return Err(PdfMasterError::Budget("import pages"));
                    }
                    if !d.more_images() {
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
            }
            pages += count;
            plans.push(Plan::Image(format, count));
        }
        if pages > MAX_PAGES {
            return Err(PdfMasterError::Budget("import pages"));
        }
    }
    let mut output = vec![];
    let mut output_bytes = 0usize;
    for ((_, bytes), plan) in files.iter().zip(plans) {
        let converted = match plan {
            Plan::Pdf => bytes.to_vec(),
            Plan::Image(format, count) => image_pdf(bytes, format, count)?,
        };
        output_bytes = output_bytes
            .checked_add(converted.len())
            .ok_or(PdfMasterError::Budget("import output bytes"))?;
        if output_bytes > crate::MAX_INSERT_BYTES {
            return Err(PdfMasterError::Budget("import output bytes"));
        }
        output.push(converted);
    }
    Ok(output)
}
fn image_pdf(bytes: &[u8], format: ImageFormat, count: usize) -> Result<Vec<u8>> {
    let mut session = Session::blank(595.276, 841.89, count)?;
    let ids = crate::document::page_object_ids(&session.document)?;
    let mut tiff = if format == ImageFormat::Tiff {
        Some(tiff_decoder(bytes)?)
    } else {
        None
    };
    for (index, page) in ids.into_iter().enumerate() {
        let image = if let Some(d) = &mut tiff {
            let (w, h, channels, orientation, associated) = tiff_properties(d)?;
            let raw = match d.read_image().map_err(|_| invalid())? {
                tiff::decoder::DecodingResult::U8(raw) => raw,
                tiff::decoder::DecodingResult::U16(raw) => {
                    raw.into_iter().map(|v| (v >> 8) as u8).collect()
                }
                _ => return Err(invalid()),
            };
            if raw.len() != w as usize * h as usize * channels {
                return Err(invalid());
            }
            let rgba = raw
                .chunks_exact(channels)
                .flat_map(|p| {
                    let mut rgba = match channels {
                        1 => [p[0], p[0], p[0], 255],
                        2 => [p[0], p[0], p[0], p[1]],
                        3 => [p[0], p[1], p[2], 255],
                        _ => [p[0], p[1], p[2], p[3]],
                    };
                    if associated {
                        let a = u32::from(rgba[3]);
                        for color in &mut rgba[..3] {
                            *color = (u32::from(*color) * 255 + a / 2)
                                .checked_div(a)
                                .unwrap_or(0)
                                .min(255) as u8;
                        }
                    }
                    rgba
                })
                .collect();
            let mut image = DynamicImage::ImageRgba8(
                image::RgbaImage::from_raw(w, h, rgba).ok_or_else(invalid)?,
            );
            image.apply_orientation(orientation);
            if index + 1 < count {
                d.next_image().map_err(|_| invalid())?;
            }
            image
        } else {
            let mut d = decoder(bytes, format)?;
            let orientation = d.orientation().map_err(|_| invalid())?;
            let mut image = DynamicImage::from_decoder(d).map_err(|_| invalid())?;
            image.apply_orientation(orientation);
            image
        };
        let w = image.width();
        let h = image.height();
        let rgba = image.into_rgba8().into_raw();
        let rgb: Vec<u8> = rgba
            .chunks_exact(4)
            .flat_map(|p| {
                let a = u32::from(p[3]);
                [0, 1, 2].map(|i| ((u32::from(p[i]) * a + 255 * (255 - a) + 127) / 255) as u8)
            })
            .collect();
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
        let image_id = session.document.add_object(stream);
        let scale = ((595.276_f64 - 48.) / f64::from(w)).min((841.89_f64 - 48.) / f64::from(h));
        let width = f64::from(w) * scale;
        let height = f64::from(h) * scale;
        let x = (595.276 - width) / 2.;
        let y = (841.89 - height) / 2.;
        let content = session.document.add_object(Stream::new(lopdf::Dictionary::new(),
            format!("q 1 1 1 rg 0 0 595.276 841.89 re f {width} 0 0 {height} {x} {y} cm /ImportedImage Do Q\n").into_bytes()));
        let page = session
            .document
            .get_object_mut(page)
            .and_then(Object::as_dict_mut)
            .map_err(|_| invalid())?;
        page.set("Contents", content);
        page.set(
            "Resources",
            dictionary! {"XObject"=>dictionary!{"ImportedImage"=>image_id}},
        );
        let bytes = crate::document::serialize(&session.document)?;
        if bytes.len() > crate::document::MAX_INPUT_BYTES {
            return Err(PdfMasterError::Budget("image PDF bytes"));
        }
    }
    let bytes = crate::document::serialize(&session.document)?;
    Session::open(&bytes)?;
    Ok(bytes)
}
