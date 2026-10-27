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
                let metadata = crate::import_exif::parse(data)?;
                (
                    metadata.orientation,
                    metadata.dcf(profile.is_some())?,
                    metadata.dpi,
                )
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
