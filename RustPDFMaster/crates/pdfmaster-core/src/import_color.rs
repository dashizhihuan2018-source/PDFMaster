//! Bounded matrix/TRC image normalization. Unsupported metadata is never stripped
//! and then silently treated as sRGB. These limits are admission limits, not a
//! claim that the complete decoder or CMS has a fixed total heap footprint.
use crate::{PdfMasterError, Result};
use image::metadata::Orientation;
use moxcms::{
    ColorProfile, DataColorSpace, Layout, ParsingOptions, ProfileClass, TransformOptions,
};
use std::io::Read;

pub(super) const PROFILE_BYTES: usize = 1024 * 1024;
const BATCH_PROFILE_BYTES: usize = 8 * PROFILE_BYTES;

pub(super) fn unsupported() -> PdfMasterError {
    PdfMasterError::InvalidOperation("image color metadata is invalid or not yet supported")
}
fn budget() -> PdfMasterError {
    PdfMasterError::Budget("image color metadata")
}
pub(super) fn charge(size: usize, total: &mut usize) -> Result<()> {
    *total = total.checked_add(size).ok_or_else(budget)?;
    if size > PROFILE_BYTES || *total > BATCH_PROFILE_BYTES {
        return Err(budget());
    }
    Ok(())
}

pub(super) struct PngMetadata {
    pub profile: Option<Box<ColorProfile>>,
    pub orientation: Option<Orientation>,
    pub dpi: Option<f64>,
}
fn be32(bytes: &[u8]) -> Result<usize> {
    Ok(u32::from_be_bytes(bytes.try_into().map_err(|_| unsupported())?) as usize)
}

fn text_preflight(tag: &[u8], total: &mut usize) -> Result<()> {
    const TEXT_BYTES: usize = 64 * 1024;
    if tag.len() > TEXT_BYTES {
        return Err(budget());
    }
    let expanded = if &tag[..4] == b"mluc" {
        if tag.len() < 16 {
            return Err(unsupported());
        }
        let count = be32(&tag[8..12])?;
        if count > 16 {
            return Err(budget());
        }
        if count == 0 || be32(&tag[12..16])? != 12 {
            return Err(unsupported());
        }
        let end = 16 + count * 12;
        let records = tag.get(16..end).ok_or_else(unsupported)?;
        let mut size = 0usize;
        for record in records.chunks_exact(12) {
            let length = be32(&record[4..8])?;
            let offset = be32(&record[8..12])?;
            if length % 2 != 0
                || offset < end
                || offset.checked_add(length).is_none_or(|n| n > tag.len())
            {
                return Err(unsupported());
            }
            // Count even aliased ranges again: the pinned CMS allocates one
            // UTF-16 conversion and owned string per localized record.
            size = size.checked_add(length).ok_or_else(budget)?;
        }
        size
    } else {
        tag.len()
    };
    *total = total.checked_add(expanded).ok_or_else(budget)?;
    if *total > TEXT_BYTES {
        return Err(budget());
    }
    Ok(())
}

pub(super) fn icc(bytes: &[u8], gray: bool, total: &mut usize) -> Result<Box<ColorProfile>> {
    charge(bytes.len(), total)?;
    if bytes.len() < 132 || be32(&bytes[..4])? != bytes.len() || &bytes[36..40] != b"acsp" {
        return Err(unsupported());
    }
    let count = be32(&bytes[128..132])?;
    if count > 64 {
        return Err(budget());
    }
    let end = 132 + count * 12;
    let tags = bytes.get(132..end).ok_or_else(unsupported)?;
    let mut seen = std::collections::HashSet::new();
    let mut text_bytes = 0usize;
    for tag in tags.chunks_exact(12) {
        let name = &tag[..4];
        // This increment supports matrix shapers, not CLUTs, device links,
        // named colors or HDR processing. Reject before CMS allocation.
        if !matches!(
            name,
            b"rXYZ"
                | b"gXYZ"
                | b"bXYZ"
                | b"rTRC"
                | b"gTRC"
                | b"bTRC"
                | b"kTRC"
                | b"wtpt"
                | b"bkpt"
                | b"chad"
                | b"desc"
                | b"cprt"
                | b"dmnd"
                | b"dmdd"
                | b"vued"
                | b"tech"
                | b"chrm"
                | b"lumi"
        ) || !seen.insert(name)
        {
            return Err(unsupported());
        }
        let start = be32(&tag[4..8])?;
        let size = be32(&tag[8..12])?;
        if start < end
            || start % 4 != 0
            || size < 8
            || start.checked_add(size).is_none_or(|n| n > bytes.len())
        {
            return Err(unsupported());
        }
        if matches!(name, b"desc" | b"cprt" | b"dmnd" | b"dmdd" | b"vued") {
            text_preflight(&bytes[start..start + size], &mut text_bytes)?;
        }
    }
    let p = ColorProfile::new_from_slice_with_options(
        bytes,
        ParsingOptions {
            max_profile_size: PROFILE_BYTES + 1,
            max_allowed_clut_size: 0,
            max_allowed_trc_size: 16_384,
        },
    )
    .map_err(|_| unsupported())?;
    if p.pcs != DataColorSpace::Xyz
        || !matches!(
            p.profile_class,
            ProfileClass::InputDevice | ProfileClass::DisplayDevice | ProfileClass::OutputDevice
        )
        || p.color_space
            != if gray {
                DataColorSpace::Gray
            } else {
                DataColorSpace::Rgb
            }
        || (gray && p.gray_trc.is_none())
        || (!gray && !p.is_matrix_shaper())
    {
        return Err(unsupported());
    }
    Ok(Box::new(p))
}

/// Scans every chunk, including after IDAT, rather than trusting the decoder's
/// optional metadata API (which deliberately ignores malformed iCCP payloads).
pub(super) fn png(bytes: &[u8], total: &mut usize) -> Result<PngMetadata> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(unsupported());
    }
    let mut offset = 8usize;
    let mut color = None;
    let mut gamma = None;
    let mut chroma = None;
    let mut profile = None;
    let mut srgb = false;
    let mut exif = false;
    let mut orientation = None;
    let mut exif_dpi = None;
    let mut phys = false;
    let mut phys_dpi = None;
    let mut before_palette = true;
    let mut before_data = true;
    for n in 0..65_536 {
        let header = bytes
            .get(offset..offset.saturating_add(8))
            .ok_or_else(unsupported)?;
        let size = be32(&header[..4])?;
        let end = offset
            .checked_add(size)
            .and_then(|n| n.checked_add(12))
            .ok_or_else(unsupported)?;
        let chunk = bytes.get(offset + 4..end).ok_or_else(unsupported)?;
        let data = &chunk[4..chunk.len() - 4];
        if crc32fast::hash(&chunk[..chunk.len() - 4]) != be32(&chunk[chunk.len() - 4..])? as u32 {
            return Err(unsupported());
        }
        let kind = &header[4..];
        match kind {
            b"IHDR" if n == 0 && size == 13 => {
                color = Some(data[9]);
            }
            b"IHDR" => return Err(unsupported()),
            b"PLTE" => before_palette = false,
            b"IDAT" => {
                before_palette = false;
                before_data = false;
            }
            b"gAMA" => {
                if !before_palette || gamma.is_some() || size != 4 {
                    return Err(unsupported());
                }
                let g = be32(data)? as f32 / 100_000.;
                if !(0.1..=10.).contains(&g) {
                    return Err(unsupported());
                }
                gamma = Some(g);
            }
            b"cHRM" => {
                if !before_palette || chroma.is_some() || size != 32 {
                    return Err(unsupported());
                }
                let mut xy = [0f32; 8];
                for (i, v) in data.chunks_exact(4).enumerate() {
                    xy[i] = be32(v)? as f32 / 100_000.;
                }
                if xy
                    .chunks_exact(2)
                    .any(|p| p[0] < 0. || p[1] <= 0. || p[0] + p[1] > 1.00001)
                {
                    return Err(unsupported());
                }
                chroma = Some(xy);
            }
            b"sRGB" => {
                if !before_palette || srgb || profile.is_some() || size != 1 || data[0] > 3 {
                    return Err(unsupported());
                }
                srgb = true;
            }
            b"iCCP" => {
                if !before_palette || profile.is_some() || srgb {
                    return Err(unsupported());
                }
                let name = data.iter().position(|v| *v == 0).ok_or_else(unsupported)?;
                if !(1..=79).contains(&name) || data.get(name + 1) != Some(&0) {
                    return Err(unsupported());
                }
                let compressed = data.get(name + 2..).ok_or_else(unsupported)?;
                let mut z = flate2::read::ZlibDecoder::new(compressed);
                let mut decoded = vec![];
                (&mut z)
                    .take(PROFILE_BYTES as u64 + 1)
                    .read_to_end(&mut decoded)
                    .map_err(|_| unsupported())?;
                if decoded.len() > PROFILE_BYTES {
                    return Err(budget());
                }
                if z.total_in() != compressed.len() as u64 {
                    return Err(unsupported());
                }
                profile = Some(icc(&decoded, matches!(color, Some(0 | 4)), total)?);
            }
            b"eXIf" => {
                if !before_data || exif {
                    return Err(unsupported());
                }
                exif = true;
                charge(data.len(), total)?;
                let metadata = crate::import_exif::parse(data)?;
                if metadata.dcf(false)? {
                    return Err(unsupported());
                }
                orientation = Some(metadata.orientation);
                exif_dpi = metadata.dpi;
            }
            b"pHYs" => {
                if !before_data || phys || data.len() != 9 || data[8] > 1 {
                    return Err(unsupported());
                }
                phys = true;
                let (x, y) = (be32(&data[..4])?, be32(&data[4..8])?);
                if data[8] == 1 && x != 0 && y != 0 {
                    phys_dpi = Some((x as f64 * 0.0254).round());
                }
            }
            b"cICP" | b"mDCV" | b"cLLI" => return Err(unsupported()),
            b"IEND" => {
                if end != bytes.len() || size != 0 || before_data {
                    return Err(unsupported());
                }
                let profile = if profile.is_some() || srgb || (gamma.is_none() && chroma.is_none())
                {
                    profile
                } else {
                    let mut p = ColorProfile::new_srgb();
                    p.cicp = None;
                    // Chromaticities without a transfer function do not define a
                    // complete source space. Keep this case explicitly unsupported.
                    let g = gamma.ok_or_else(unsupported)?;
                    p.red_trc = Some(moxcms::curve_from_gamma(1. / g));
                    p.green_trc = p.red_trc.clone();
                    p.blue_trc = p.red_trc.clone();
                    if let Some(xy) = chroma {
                        // PNG grayscale ignores the RGB primaries; adapting its
                        // white point needs a separate gray-space implementation.
                        if matches!(color, Some(0 | 4)) {
                            return Err(unsupported());
                        }
                        let white = moxcms::XyY::new(f64::from(xy[0]), f64::from(xy[1]), 1.);
                        let primaries = moxcms::ColorPrimaries {
                            red: moxcms::Chromaticity { x: xy[2], y: xy[3] },
                            green: moxcms::Chromaticity { x: xy[4], y: xy[5] },
                            blue: moxcms::Chromaticity { x: xy[6], y: xy[7] },
                        };
                        let matrix = ColorProfile::colorants_matrix(white, primaries);
                        let m = matrix.v;
                        let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
                            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
                            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
                        if !det.is_finite()
                            || det.abs() < 1e-8
                            || m.iter().flatten().any(|v| !v.is_finite() || v.abs() > 16.)
                        {
                            return Err(unsupported());
                        }
                        p.update_rgb_colorimetry(white, primaries);
                    }
                    Some(Box::new(p))
                };
                return Ok(PngMetadata {
                    profile,
                    orientation,
                    dpi: exif_dpi.or(phys_dpi),
                });
            }
            _ => {}
        }
        offset = end;
    }
    Err(budget())
}

pub(super) fn normalize(image: image::DynamicImage, profile: &ColorProfile) -> Result<Vec<u8>> {
    let rgba = image.into_rgba16().into_raw();
    let gray = profile.color_space == DataColorSpace::Gray;
    let transform = profile
        .create_transform_16bit(
            if gray {
                Layout::GrayAlpha
            } else {
                Layout::Rgba
            },
            &ColorProfile::new_srgb(),
            Layout::Rgba,
            TransformOptions {
                allow_use_cicp_transfer: false,
                ..Default::default()
            },
        )
        .map_err(|_| unsupported())?;
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    for source in rgba.chunks(4096 * 4) {
        let input = if gray {
            source.chunks_exact(4).flat_map(|p| [p[0], p[3]]).collect()
        } else {
            source.to_vec()
        };
        let mut out = vec![0u16; source.len()];
        transform
            .transform(&input, &mut out)
            .map_err(|_| unsupported())?;
        for (p, original) in out.chunks_exact(4).zip(source.chunks_exact(4)) {
            let alpha = u64::from(original[3]);
            for color in &p[..3] {
                let value =
                    (u64::from(*color) * alpha + 65_535 * (65_535 - alpha) + 32_767) / 65_535;
                rgb.push(((value + 128) / 257) as u8);
            }
        }
    }
    Ok(rgb)
}
