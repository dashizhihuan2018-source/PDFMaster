//! JPEG container metadata is admitted independently: the pixel decoder's
//! optional ICC API intentionally conflates missing and malformed profiles.
use crate::{
    PdfMasterError, Result,
    import_color::{self, unsupported},
};
use image::metadata::Orientation;
use moxcms::ColorProfile;

pub(super) struct Metadata {
    pub profile: Option<Box<ColorProfile>>,
    pub orientation: Orientation,
    pub dpi: Option<f64>,
}
fn budget() -> PdfMasterError {
    PdfMasterError::Budget("JPEG metadata")
}

pub(super) fn preflight(bytes: &[u8], color_bytes: &mut usize) -> Result<Metadata> {
    if !bytes.starts_with(&[255, 216]) {
        return Err(unsupported());
    }
    let mut offset = 2usize;
    let mut entropy = false;
    let mut scans = 0usize;
    let mut channels = None;
    let mut fragments = [None; 256];
    let mut count = None;
    let mut profile_size = 0usize;
    let mut exif = None;
    let mut jfif = None;
    for _ in 0..65_536 {
        let marker = loop {
            if entropy {
                offset += bytes
                    .get(offset..)
                    .ok_or_else(unsupported)?
                    .iter()
                    .position(|b| *b == 255)
                    .ok_or_else(unsupported)?;
            }
            if bytes.get(offset) != Some(&255) {
                return Err(unsupported());
            }
            while bytes.get(offset) == Some(&255) {
                offset += 1;
            }
            let marker = *bytes.get(offset).ok_or_else(unsupported)?;
            offset += 1;
            if entropy && (marker == 0 || (208..=215).contains(&marker)) {
                continue;
            }
            break marker;
        };
        entropy = false;
        if marker == 217 {
            if offset != bytes.len() || scans == 0 {
                return Err(unsupported());
            }
            let gray = channels.ok_or_else(unsupported)? == 1;
            let profile = if let Some(count) = count {
                let mut profile = Vec::with_capacity(profile_size);
                for part in &fragments[1..=count] {
                    profile.extend_from_slice(part.ok_or_else(unsupported)?);
                }
                Some(import_color::icc(&profile, gray, color_bytes)?)
            } else {
                None
            };
            let (orientation, dcf, exif_dpi) = if let Some(data) = exif {
                exif_metadata(data, profile.is_some())?
            } else {
                (Orientation::NoTransforms, false, None)
            };
            let profile = if dcf {
                if gray {
                    return Err(unsupported());
                }
                // DCF optional color is close to, but not exactly Adobe RGB:
                // CP-3461B 4.5.4.4 specifies white (.313,.329) and gamma 2.2.
                let mut p = ColorProfile::new_srgb();
                p.cicp = None;
                p.red_trc = Some(moxcms::curve_from_gamma(2.2));
                p.green_trc = p.red_trc.clone();
                p.blue_trc = p.red_trc.clone();
                p.update_rgb_colorimetry(
                    moxcms::XyY::new(0.313, 0.329, 1.),
                    moxcms::ColorPrimaries {
                        red: moxcms::Chromaticity { x: 0.64, y: 0.33 },
                        green: moxcms::Chromaticity { x: 0.21, y: 0.71 },
                        blue: moxcms::Chromaticity { x: 0.15, y: 0.06 },
                    },
                );
                Some(Box::new(p))
            } else {
                profile
            };
            return Ok(Metadata {
                profile,
                orientation,
                dpi: exif_dpi.or(jfif.flatten()),
            });
        }
        if marker == 0 || marker == 216 || marker == 1 || (208..=215).contains(&marker) {
            return Err(unsupported());
        }
        let length = bytes.get(offset..offset + 2).ok_or_else(unsupported)?;
        let length = u16::from_be_bytes(length.try_into().map_err(|_| unsupported())?) as usize;
        if length < 2 {
            return Err(unsupported());
        }
        let end = offset.checked_add(length).ok_or_else(unsupported)?;
        let data = bytes.get(offset + 2..end).ok_or_else(unsupported)?;
        offset = end;
        match marker {
            224 if data.starts_with(b"JFIF\0") => {
                if scans != 0
                    || jfif.is_some()
                    || data.len() < 14
                    || data[7] > 2
                    || data.len() != 14 + usize::from(data[12]) * usize::from(data[13]) * 3
                {
                    return Err(unsupported());
                }
                let x = u16::from_be_bytes([data[8], data[9]]);
                let y = u16::from_be_bytes([data[10], data[11]]);
                jfif = Some(match data[7] {
                    1 | 2 if x != 0 && y != 0 => {
                        Some(f64::from(x) * if data[7] == 2 { 2.54 } else { 1. })
                    }
                    _ => None,
                });
            }
            192..=195 | 197..=199 | 201..=203 | 205..=207 => {
                if !matches!(marker, 192..=194)
                    || scans != 0
                    || channels.is_some()
                    || data.len() < 6
                    || data[0] != 8
                    || !matches!(data[5], 1 | 3)
                    || data.len() != 6 + 3 * usize::from(data[5])
                {
                    return Err(unsupported());
                }
                channels = Some(data[5]);
            }
            218 => {
                if channels.is_none() {
                    return Err(unsupported());
                }
                scans += 1;
                if scans > 256 {
                    return Err(budget());
                }
                entropy = true;
            }
            226 if data.starts_with(b"ICC_PROFILE") => {
                if scans != 0 || data.len() <= 14 || &data[..12] != b"ICC_PROFILE\0" {
                    return Err(unsupported());
                }
                let sequence = usize::from(data[12]);
                let total = usize::from(data[13]);
                if total == 0
                    || sequence == 0
                    || sequence > total
                    || count.is_some_and(|c| c != total)
                    || fragments[sequence].is_some()
                {
                    return Err(unsupported());
                }
                count = Some(total);
                profile_size = profile_size
                    .checked_add(data.len() - 14)
                    .ok_or_else(budget)?;
                if profile_size > import_color::PROFILE_BYTES {
                    return Err(budget());
                }
                fragments[sequence] = Some(&data[14..]);
            }
            225 if data.starts_with(b"Exif") => {
                if scans != 0 || exif.is_some() || !data.starts_with(b"Exif\0\0") {
                    return Err(unsupported());
                }
                exif = Some(&data[6..]);
            }
            // XMP, MPF/gain-map and alternate color containers require their
            // own semantics; do not silently render just an SDR/base image.
            225 if data.starts_with(b"http://ns.adobe.com/") => return Err(unsupported()),
            226 if data.starts_with(b"MPF\0") || data.starts_with(b"urn:iso:std:iso:ts:21496:") => {
                return Err(unsupported());
            }
            232 if data.starts_with(b"SPIFF\0") => return Err(unsupported()),
            235 if data.starts_with(b"JP") => return Err(unsupported()),
            _ => {}
        }
    }
    Err(budget())
}

/// Read only orientation and primary-image color declarations. Validate every
/// field range, but never interpret/emit GPS, maker notes or thumbnail pixels.
fn exif_metadata(bytes: &[u8], has_icc: bool) -> Result<(Orientation, bool, Option<f64>)> {
    let little = match bytes.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return Err(unsupported()),
    };
    let u16_at = |offset: usize| -> Result<u16> {
        let b = bytes
            .get(offset..offset.checked_add(2).ok_or_else(unsupported)?)
            .ok_or_else(unsupported)?;
        let b = b.try_into().map_err(|_| unsupported())?;
        Ok(if little {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        })
    };
    let u32_at = |offset: usize| -> Result<usize> {
        let b = bytes
            .get(offset..offset.checked_add(4).ok_or_else(unsupported)?)
            .ok_or_else(unsupported)?;
        let b = b.try_into().map_err(|_| unsupported())?;
        Ok(if little {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        } as usize)
    };
    if u16_at(2)? != 42 {
        return Err(unsupported());
    }
    // kind: 0 primary, 1 Exif, 2 interoperability, 3 thumbnail, 4 GPS.
    let mut pending = vec![(u32_at(4)?, 0)];
    let mut visited = std::collections::HashSet::new();
    let mut entries = 0usize;
    let mut orientation = Orientation::NoTransforms;
    let mut color = None;
    let mut interop = None;
    let mut dcf_fields = 0u8;
    let mut dcf_values = true;
    let mut resolution = None;
    let mut resolution_y = None;
    let mut resolution_unit = 2;
    while let Some((offset, kind)) = pending.pop() {
        if offset < 8 || !visited.insert(offset) {
            return Err(unsupported());
        }
        if visited.len() > 8 {
            return Err(budget());
        }
        let count = usize::from(u16_at(offset)?);
        entries += count;
        if entries > 512 {
            return Err(budget());
        }
        let end = offset.checked_add(2 + count * 12).ok_or_else(unsupported)?;
        bytes
            .get(offset..end.checked_add(4).ok_or_else(unsupported)?)
            .ok_or_else(unsupported)?;
        let mut tags = std::collections::HashSet::new();
        for entry in (offset + 2..end).step_by(12) {
            let tag = u16_at(entry)?;
            if !tags.insert(tag) {
                return Err(unsupported());
            }
            let ty = u16_at(entry + 2)?;
            let count = u32_at(entry + 4)?;
            let unit = match ty {
                1 | 2 | 6 | 7 => 1,
                3 | 8 => 2,
                4 | 9 | 11 | 13 => 4,
                5 | 10 | 12 => 8,
                _ => return Err(unsupported()),
            };
            let size = count.checked_mul(unit).ok_or_else(unsupported)?;
            let value = if size <= 4 {
                entry + 8
            } else {
                u32_at(entry + 8)?
            };
            let data = bytes
                .get(value..value.checked_add(size).ok_or_else(unsupported)?)
                .ok_or_else(unsupported)?;
            match (kind, tag) {
                (0, 282 | 283) => {
                    if ty != 5 || count != 1 {
                        return Err(unsupported());
                    }
                    let denominator = u32_at(value + 4)?;
                    if denominator == 0 {
                        return Err(unsupported());
                    }
                    if tag == 282 {
                        resolution = Some(u32_at(value)? as f64 / denominator as f64);
                    } else {
                        resolution_y = Some(u32_at(value)? as f64 / denominator as f64);
                    }
                }
                (0, 296) => {
                    if ty != 3 || count != 1 {
                        return Err(unsupported());
                    }
                    resolution_unit = u16_at(value)?;
                    if !(1..=3).contains(&resolution_unit) {
                        return Err(unsupported());
                    }
                }
                (0, 0x112) => {
                    if ty != 3 || count != 1 {
                        return Err(unsupported());
                    }
                    orientation = Orientation::from_exif(
                        u16_at(value)?.try_into().map_err(|_| unsupported())?,
                    )
                    .ok_or_else(unsupported)?;
                }
                (0, 0x8769) | (0, 0x8825) | (1, 0xa005) => {
                    if ty != 4 || count != 1 {
                        return Err(unsupported());
                    }
                    pending.push((
                        u32_at(value)?,
                        match tag {
                            0x8769 => 1,
                            0xa005 => 2,
                            _ => 4,
                        },
                    ));
                }
                (1, 0xa001) => {
                    if ty != 3 || count != 1 {
                        return Err(unsupported());
                    }
                    color = Some(u16_at(value)?);
                }
                (2, 1) => {
                    if ty != 2 || count != 4 {
                        return Err(unsupported());
                    }
                    interop = Some(data);
                }
                (0, 318 | 319 | 529) | (1, 0xa500) => {
                    let (bit, expected): (u8, &[(u32, u32)]) = match tag {
                        318 => (1, &[(313, 1000), (329, 1000)]),
                        319 => (
                            2,
                            &[
                                (64, 100),
                                (33, 100),
                                (21, 100),
                                (71, 100),
                                (15, 100),
                                (6, 100),
                            ],
                        ),
                        529 => (4, &[(299, 1000), (587, 1000), (114, 1000)]),
                        _ => (8, &[(22, 10)]),
                    };
                    if ty != 5 || count != expected.len() {
                        return Err(unsupported());
                    }
                    for (i, (n, d)) in expected.iter().enumerate() {
                        let actual_n = u32_at(value + i * 8)? as u64;
                        let actual_d = u32_at(value + i * 8 + 4)? as u64;
                        if actual_d == 0 {
                            return Err(unsupported());
                        }
                        dcf_values &= actual_n * u64::from(*d) == actual_d * u64::from(*n);
                    }
                    dcf_fields |= bit;
                }
                (0, 301 | 532) => return Err(unsupported()),
                (0, 330) => return Err(unsupported()), // unimplemented SubIFDs
                _ => {}
            }
        }
        let next = u32_at(end)?;
        if next != 0 {
            if kind != 0 {
                return Err(unsupported());
            }
            pending.push((next, 3));
        }
    }
    // ImageIO treats Exif unit1 (or no unit) as inch here, and synthesizes
    // horizontal density from Y when X is absent. JFIF is only a fallback.
    let dpi = if resolution == Some(0.) || resolution_y == Some(0.) {
        Some(72.)
    } else {
        resolution.or(resolution_y).map(|x| {
            if resolution_unit == 3 {
                (x * 2.54).round()
            } else {
                x
            }
        })
    };
    if has_icc {
        return Ok((orientation, false, dpi));
    }
    let dcf = match (color, interop) {
        (None | Some(1), None | Some(b"R98\0")) if dcf_fields == 0 => false,
        (Some(65535), Some(b"R03\0")) if dcf_fields == 15 && dcf_values => true,
        _ => return Err(unsupported()),
    };
    Ok((orientation, dcf, dpi))
}
