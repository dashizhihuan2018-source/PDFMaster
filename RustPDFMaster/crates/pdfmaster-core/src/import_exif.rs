//! Bounded TIFF/Exif metadata shared by JPEG APP1 and PNG eXIf containers.
//! Container framing and color-profile precedence stay with their callers.
use crate::{PdfMasterError, Result, import_color::unsupported};
use image::metadata::Orientation;

pub(super) struct Metadata<'a> {
    pub orientation: Orientation,
    pub dpi: Option<f64>,
    color: Option<u16>,
    interop: Option<&'a [u8]>,
    dcf_fields: u8,
    dcf_values: bool,
}

impl Metadata<'_> {
    /// JPEG ICC takes precedence over the legacy Exif color declaration.
    /// PNG calls this with `false` so unknown/DCF declarations are rejected.
    pub(super) fn dcf(&self, has_icc: bool) -> Result<bool> {
        if has_icc {
            return Ok(false);
        }
        match (self.color, self.interop) {
            (None | Some(1), None | Some(b"R98\0")) if self.dcf_fields == 0 => Ok(false),
            (Some(65535), Some(b"R03\0")) if self.dcf_fields == 15 && self.dcf_values => Ok(true),
            _ => Err(unsupported()),
        }
    }
}

fn budget() -> PdfMasterError {
    PdfMasterError::Budget("Exif metadata")
}

/// Read only orientation, primary-image density and color declarations.
/// Validate every traversed field, but never interpret GPS, maker notes or pixels.
pub(super) fn parse(bytes: &[u8]) -> Result<Metadata<'_>> {
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
    // Unit 1 (or missing) is treated as inch. Any explicit zero axis first
    // selects the 72 base, then centimeter conversion/rounding is applied.
    let base = if resolution == Some(0.) || resolution_y == Some(0.) {
        Some(72.)
    } else {
        resolution.or(resolution_y)
    };
    let dpi = base.map(|x| {
        if resolution_unit == 3 {
            (x * 2.54).round()
        } else {
            x
        }
    });
    Ok(Metadata {
        orientation,
        dpi,
        color,
        interop,
        dcf_fields,
        dcf_values,
    })
}
