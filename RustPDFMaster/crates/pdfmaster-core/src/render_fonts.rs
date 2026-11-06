//! Admission for the bounded horizontal CID CFF profile, scoped to one page preflight.
use super::{MAX_RESOURCE_DEPTH, MAX_RESOURCE_OBJECTS, PreflightState, finite_number};
use crate::{PdfMasterError, Result};
use lopdf::{Dictionary, Document, Object, ObjectId};
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Clone)]
pub(super) enum AuditedFont {
    Standard,
    IdentityH {
        glyph_cids: BTreeSet<u16>,
        unicode_cids: BTreeSet<u16>,
    },
}

#[derive(Default)]
pub(super) struct FontState {
    visited: HashMap<ObjectId, usize>,
    steps: usize,
    streams: HashMap<ObjectId, Vec<u8>>,
    fonts: HashMap<ObjectId, AuditedFont>,
    glyphs: usize,
    subfonts: usize,
    commands: usize,
}

fn fail(message: impl Into<String>) -> PdfMasterError {
    PdfMasterError::Render(message.into())
}

pub(super) fn resolve<'a>(
    doc: &'a Document,
    object: &'a Object,
) -> Result<(Option<ObjectId>, &'a Object)> {
    let mut current = object;
    // Cache by the terminal indirect object, not an alias that may have peers.
    let mut terminal_id = None;
    for _ in 0..=MAX_RESOURCE_DEPTH {
        match current {
            Object::Reference(id) => {
                terminal_id = Some(*id);
                current = doc
                    .objects
                    .get(id)
                    .ok_or_else(|| fail("font graph reference is missing"))?;
            }
            _ => return Ok((terminal_id, current)),
        }
    }
    Err(PdfMasterError::Budget("font graph depth"))
}

pub(super) fn audit_graph(
    doc: &Document,
    object: &Object,
    state: &mut PreflightState,
) -> Result<()> {
    fn visit(
        doc: &Document,
        object: &Object,
        state: &mut PreflightState,
        active: &mut HashSet<ObjectId>,
        depth: usize,
    ) -> Result<usize> {
        if depth > MAX_RESOURCE_DEPTH {
            return Err(PdfMasterError::Budget("font graph depth"));
        }
        if let Object::Reference(id) = object {
            if active.contains(id) {
                return Err(fail("font graph contains an indirect reference cycle"));
            }
            if let Some(height) = state.font_state.visited.get(id) {
                if depth.saturating_add(*height) > MAX_RESOURCE_DEPTH {
                    return Err(PdfMasterError::Budget("font graph depth"));
                }
                return Ok(*height);
            }
            state
                .count_resource()
                .map_err(|_| PdfMasterError::Budget("font graph object count"))?;
            let value = doc
                .objects
                .get(id)
                .ok_or_else(|| fail("font graph reference is missing"))?;
            active.insert(*id);
            let height = 1 + visit(doc, value, state, active, depth + 1)?;
            active.remove(id);
            state.font_state.visited.insert(*id, height);
            return Ok(height);
        }
        state.font_state.steps = state.font_state.steps.saturating_add(1);
        if state.font_state.steps > MAX_RESOURCE_OBJECTS {
            return Err(PdfMasterError::Budget("font graph direct traversal steps"));
        }
        let mut height = 0;
        match object {
            Object::Array(values) => {
                for v in values {
                    height = height.max(1 + visit(doc, v, state, active, depth + 1)?);
                }
            }
            Object::Dictionary(dict) => {
                for (_, v) in dict.iter() {
                    height = height.max(1 + visit(doc, v, state, active, depth + 1)?);
                }
            }
            Object::Stream(stream) => {
                for (_, v) in stream.dict.iter() {
                    height = height.max(1 + visit(doc, v, state, active, depth + 1)?);
                }
            }
            _ => {}
        }
        Ok(height)
    }
    visit(doc, object, state, &mut HashSet::new(), 0).map(|_| ())
}

fn field<'a>(doc: &'a Document, dict: &'a Dictionary, key: &[u8]) -> Result<&'a Object> {
    resolve(
        doc,
        dict.get(key).map_err(|_| {
            fail(format!(
                "font required field {} is missing",
                String::from_utf8_lossy(key)
            ))
        })?,
    )
    .map(|(_, v)| v)
}
fn keys(dict: &Dictionary, allowed: &[&[u8]], label: &str) -> Result<()> {
    for (key, _) in dict.iter() {
        if !allowed.contains(&key.as_slice()) {
            return Err(fail(format!(
                "{label} field {} is unsupported",
                String::from_utf8_lossy(key)
            )));
        }
    }
    Ok(())
}
fn name(doc: &Document, dict: &Dictionary, key: &[u8]) -> Result<Vec<u8>> {
    let value = field(doc, dict, key)?
        .as_name()
        .map_err(|_| fail("font field must be a name"))?;
    if value.is_empty() || value.len() > 127 {
        return Err(fail("font name length"));
    }
    Ok(value.to_vec())
}
fn expected_name(doc: &Document, dict: &Dictionary, key: &[u8], expected: &[u8]) -> Result<()> {
    if name(doc, dict, key)? != expected {
        return Err(fail(format!(
            "font {} must be {}",
            String::from_utf8_lossy(key),
            String::from_utf8_lossy(expected)
        )));
    }
    Ok(())
}
fn number(doc: &Document, dict: &Dictionary, key: &[u8]) -> Result<f64> {
    finite_number(field(doc, dict, key)?)
        .map(f64::from)
        .ok_or_else(|| fail("font metric must be finite"))
}
fn width(object: &Object) -> Result<()> {
    if !finite_number(object).is_some_and(|v| (0.0..=100_000.0).contains(&v)) {
        return Err(fail("CID width must be finite in 0..=100000"));
    }
    Ok(())
}
fn cid(object: &Object) -> Result<u16> {
    object
        .as_i64()
        .ok()
        .and_then(|v| u16::try_from(v).ok())
        .ok_or_else(|| fail("CID must be in 0..=65535"))
}

fn widths(doc: &Document, dict: &Dictionary) -> Result<()> {
    if dict.has(b"DW") {
        width(field(doc, dict, b"DW")?)?;
    }
    if !dict.has(b"W") {
        return Ok(());
    }
    let values = field(doc, dict, b"W")?
        .as_array()
        .map_err(|_| fail("CID W must be an array"))?;
    if values.is_empty() {
        return Err(fail("CID W must not be empty"));
    }
    let mut i = 0;
    let mut previous = None;
    let mut covered = 0u32;
    while i < values.len() {
        let start = cid(resolve(doc, &values[i])?.1)?;
        i += 1;
        let next = resolve(
            doc,
            values.get(i).ok_or_else(|| fail("CID W is truncated"))?,
        )?
        .1;
        let end = if let Object::Array(ws) = next {
            if ws.is_empty() {
                return Err(fail("CID W width array must not be empty"));
            }
            for w in ws {
                width(resolve(doc, w)?.1)?;
            }
            i += 1;
            u16::try_from(u32::from(start) + ws.len() as u32 - 1)
                .map_err(|_| fail("CID W range overflow"))?
        } else {
            let end = cid(next)?;
            width(
                resolve(
                    doc,
                    values
                        .get(i + 1)
                        .ok_or_else(|| fail("CID W is truncated"))?,
                )?
                .1,
            )?;
            i += 2;
            end
        };
        if end < start || previous.is_some_and(|p| start <= p) {
            return Err(fail("CID W ranges must be monotonic and non-overlapping"));
        }
        covered += u32::from(end) - u32::from(start) + 1;
        if covered > 65_536 {
            return Err(PdfMasterError::Budget("CID W coverage"));
        }
        previous = Some(end);
    }
    Ok(())
}

#[derive(Debug, PartialEq)]
pub(super) struct CidSystemInfo {
    pub registry: Vec<u8>,
    pub ordering: Vec<u8>,
    pub supplement: i32,
}
fn system_info(doc: &Document, object: &Object) -> Result<CidSystemInfo> {
    let dict = resolve(doc, object)?
        .1
        .as_dict()
        .map_err(|_| fail("CIDSystemInfo must be a dictionary"))?;
    keys(
        dict,
        &[b"Registry", b"Ordering", b"Supplement"],
        "CIDSystemInfo",
    )?;
    let string = |key| -> Result<Vec<u8>> {
        let s = field(doc, dict, key)?
            .as_str()
            .map_err(|_| fail("CIDSystemInfo value must be a string"))?;
        if s.is_empty() || s.len() > 127 {
            return Err(fail("CIDSystemInfo string length"));
        }
        Ok(s.to_vec())
    };
    let supplement = field(doc, dict, b"Supplement")?
        .as_i64()
        .ok()
        .and_then(|n| i32::try_from(n).ok())
        .filter(|n| *n >= 0)
        .ok_or_else(|| fail("CIDSystemInfo Supplement must be non-negative i32"))?;
    Ok(CidSystemInfo {
        registry: string(b"Registry")?,
        ordering: string(b"Ordering")?,
        supplement,
    })
}

fn stream(
    doc: &Document,
    object: &Object,
    state: &mut PreflightState,
    label: &str,
    raw_limit: usize,
    decoded_limit: usize,
) -> Result<Vec<u8>> {
    let (id, object) = resolve(doc, object)?;
    let stream = object
        .as_stream()
        .map_err(|_| fail(format!("{label} must be a stream")))?;
    let extra: &[&[u8]] = match label {
        "FontFile3" => &[b"Length", b"Filter", b"Subtype"],
        "ToUnicode" => &[b"Length", b"Filter", b"Type", b"WMode"],
        _ => &[b"Length", b"Filter"],
    };
    keys(&stream.dict, extra, &format!("{label} stream"))?;
    if label == "FontFile3" {
        expected_name(doc, &stream.dict, b"Subtype", b"CIDFontType0C")?;
    }
    if label == "ToUnicode" {
        if stream.dict.has(b"Type") {
            expected_name(doc, &stream.dict, b"Type", b"CMap")?;
        }
        if stream.dict.has(b"WMode") && field(doc, &stream.dict, b"WMode")?.as_i64().ok() != Some(0)
        {
            return Err(fail("ToUnicode WMode must be 0"));
        }
    }
    if field(doc, &stream.dict, b"Length")?
        .as_i64()
        .ok()
        .and_then(|n| usize::try_from(n).ok())
        != Some(stream.content.len())
    {
        return Err(fail(format!("{label} Length mismatch")));
    }
    let budget_label = match label {
        "FontFile3" => "CFF",
        "ToUnicode" => "CMap",
        _ => "CIDSet",
    };
    if stream.content.len() > raw_limit {
        return Err(fail(format!("{budget_label} raw bytes")));
    }
    let flate = if stream.dict.has(b"Filter") {
        let filter = field(doc, &stream.dict, b"Filter")?;
        let filter = match filter {
            Object::Name(_) => filter,
            Object::Array(values) if values.len() == 1 => resolve(doc, &values[0])?.1,
            _ => return Err(fail("stream Filter must be a name or one-element array")),
        };
        if !matches!(filter.as_name().ok(), Some(b"FlateDecode" | b"Fl")) {
            return Err(fail("font stream Filter is unsupported"));
        }
        true
    } else {
        false
    };
    if let Some(decoded) = id.and_then(|id| state.font_state.streams.get(&id)) {
        if decoded.len() > decoded_limit {
            return Err(fail(format!("{budget_label} decoded bytes")));
        }
        return Ok(decoded.clone());
    }
    state.charge_source_bytes(stream.content.len() as u64)?;
    // Bound decompression before parsing or allocation driven by the font program.
    let decoded = if flate {
        super::strict_zlib_decode(&stream.content, decoded_limit, label).map_err(|e| match e {
            PdfMasterError::Budget(_) => fail(format!("{budget_label} decoded bytes")),
            _ => e,
        })?
    } else {
        stream.content.clone()
    };
    if decoded.len() > decoded_limit {
        return Err(fail(format!("{budget_label} decoded bytes")));
    }
    state.charge_source_bytes(decoded.len() as u64)?;
    if let Some(id) = id {
        state.font_state.streams.insert(id, decoded.clone());
    }
    Ok(decoded)
}

pub(super) fn audit_type0_cached(
    doc: &Document,
    id: Option<ObjectId>,
    font: &Dictionary,
    state: &mut PreflightState,
) -> Result<AuditedFont> {
    if let Some(value) = id.and_then(|id| state.font_state.fonts.get(&id)) {
        return Ok(value.clone());
    }
    let value = audit_type0(doc, font, state)?;
    if let Some(id) = id {
        state.font_state.fonts.insert(id, value.clone());
    }
    Ok(value)
}
pub(super) fn audit_type0(
    doc: &Document,
    font: &Dictionary,
    state: &mut PreflightState,
) -> Result<AuditedFont> {
    keys(
        font,
        &[
            b"Type",
            b"Subtype",
            b"BaseFont",
            b"Encoding",
            b"DescendantFonts",
            b"ToUnicode",
            b"Name",
        ],
        "Type0 Font",
    )?;
    expected_name(doc, font, b"Type", b"Font")?;
    expected_name(doc, font, b"Subtype", b"Type0")?;
    expected_name(doc, font, b"Encoding", b"Identity-H")?;
    name(doc, font, b"BaseFont")?;
    if font.has(b"Name") {
        name(doc, font, b"Name")?;
    }
    let descendants = field(doc, font, b"DescendantFonts")?
        .as_array()
        .map_err(|_| fail("DescendantFonts must be an array"))?;
    if descendants.len() != 1 {
        return Err(fail("DescendantFonts must contain exactly one font"));
    }
    let descendant = resolve(doc, &descendants[0])?
        .1
        .as_dict()
        .map_err(|_| fail("descendant must be a dictionary"))?;
    keys(
        descendant,
        &[
            b"Type",
            b"Subtype",
            b"BaseFont",
            b"CIDSystemInfo",
            b"FontDescriptor",
            b"DW",
            b"W",
        ],
        "CIDFont",
    )?;
    expected_name(doc, descendant, b"Type", b"Font")?;
    expected_name(doc, descendant, b"Subtype", b"CIDFontType0")?;
    name(doc, descendant, b"BaseFont")?;
    widths(doc, descendant)?;
    let info = system_info(doc, field(doc, descendant, b"CIDSystemInfo")?)?;
    let descriptor = field(doc, descendant, b"FontDescriptor")?
        .as_dict()
        .map_err(|_| fail("FontDescriptor must be a dictionary"))?;
    keys(
        descriptor,
        &[
            b"Type",
            b"FontName",
            b"Flags",
            b"FontBBox",
            b"ItalicAngle",
            b"Ascent",
            b"Descent",
            b"CapHeight",
            b"StemV",
            b"CIDSet",
            b"FontFile3",
        ],
        "FontDescriptor",
    )?;
    expected_name(doc, descriptor, b"Type", b"FontDescriptor")?;
    name(doc, descriptor, b"FontName")?;
    if field(doc, descriptor, b"Flags")?
        .as_i64()
        .ok()
        .and_then(|n| u32::try_from(n).ok())
        .is_none()
    {
        return Err(fail("FontDescriptor Flags must be a non-negative u32"));
    }
    let bbox = field(doc, descriptor, b"FontBBox")?
        .as_array()
        .map_err(|_| fail("FontBBox must be an array"))?;
    let bbox = bbox
        .iter()
        .map(|v| resolve(doc, v).ok().and_then(|(_, v)| finite_number(v)))
        .collect::<Option<Vec<_>>>()
        .filter(|v| v.len() == 4)
        .ok_or_else(|| fail("FontBBox must contain four finite numbers"))?;
    if bbox[2] <= bbox[0] || bbox[3] <= bbox[1] {
        return Err(fail("FontBBox must have positive width and height"));
    }
    for key in [
        b"ItalicAngle".as_slice(),
        b"Ascent",
        b"Descent",
        b"CapHeight",
        b"StemV",
    ] {
        let n = number(doc, descriptor, key)?;
        if n.abs() > 100_000.0 || (key == b"StemV" && n < 0.0) {
            return Err(fail("FontDescriptor metric outside bounds"));
        }
    }
    let bytes = stream(
        doc,
        descriptor
            .get(b"FontFile3")
            .map_err(|_| fail("FontFile3 is required"))?,
        state,
        "FontFile3",
        8 * 1024 * 1024,
        16 * 1024 * 1024,
    )?;
    let cids = audit_cff(&bytes, state)?;
    if let Ok(object) = descriptor.get(b"CIDSet") {
        let bits = stream(doc, object, state, "CIDSet", 16 * 1024, 8192)?;
        for cid in &cids {
            if bits
                .get(usize::from(*cid) / 8)
                .is_none_or(|b| b & (0x80 >> (*cid % 8)) == 0)
            {
                return Err(fail("CIDSet omits a CFF charset CID"));
            }
        }
    }
    let cmap = stream(
        doc,
        font.get(b"ToUnicode")
            .map_err(|_| fail("ToUnicode is required"))?,
        state,
        "ToUnicode",
        1024 * 1024,
        1024 * 1024,
    )?;
    let unicode_cids = super::cmap::audit(&cmap, &info, &cids)?;
    Ok(AuditedFont::IdentityH {
        glyph_cids: cids,
        unicode_cids,
    })
}

fn audit_cff(bytes: &[u8], state: &mut PreflightState) -> Result<BTreeSet<u16>> {
    use skrifa::{
        GlyphId,
        raw::{
            FontRead,
            ps::cff::{CffFontRef, v1::Cff},
        },
    };
    let cff = Cff::read(bytes.into()).map_err(|e| fail(format!("CFF parse failed: {e:?}")))?;
    if cff.header().major() != 1 || cff.names().count() != 1 || cff.top_dicts().count() != 1 {
        return Err(fail(
            "CFF parse failed: expected version 1 and one name/topdict",
        ));
    }
    let font = CffFontRef::new_cff(bytes, 0, None)
        .map_err(|e| fail(format!("CFF parse failed: {e:?}")))?;
    if !font.is_cid() {
        return Err(fail("CFF font must be CID-keyed"));
    }
    let glyphs = font.num_glyphs() as usize;
    let subfonts = usize::from(font.num_subfonts());
    if glyphs == 0 || glyphs > 4096 || state.font_state.glyphs.saturating_add(glyphs) > 4096 {
        return Err(PdfMasterError::Budget("CFF glyph budget"));
    }
    if subfonts == 0 || subfonts > 256 || state.font_state.subfonts.saturating_add(subfonts) > 256 {
        return Err(PdfMasterError::Budget("CFF subfont budget"));
    }
    state.font_state.glyphs += glyphs;
    state.font_state.subfonts += subfonts;
    let charset = font
        .charset()
        .ok_or_else(|| fail("CFF charset parse failed"))?;
    let mut cids = BTreeSet::new();
    let mut mappings = charset.iter();
    for index in 0..glyphs {
        let (gid, sid) = mappings
            .next()
            .ok_or_else(|| fail("CFF charset is truncated"))?;
        let cid = sid.to_u16();
        if gid.to_u32() as usize != index || (index == 0 && cid != 0) || !cids.insert(cid) {
            return Err(fail(
                "CFF charset must be continuous with unique CIDs and GID0 CID0",
            ));
        }
    }
    if mappings.next().is_some() {
        return Err(fail("CFF charset has excess entries"));
    }
    let subfonts = (0..font.num_subfonts())
        .map(|i| {
            font.subfont(i, &[])
                .map_err(|e| fail(format!("CFF subfont parse failed: {e:?}")))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut pen = CountingPen {
        commands: state.font_state.commands,
        invalid: false,
    };
    for index in 0..glyphs {
        let gid = GlyphId::new(index as u32);
        let subfont = font
            .subfont_index(gid)
            .and_then(|i| subfonts.get(usize::from(i)))
            .ok_or_else(|| fail("CFF subfont index is missing or out of range"))?;
        font.draw(subfont, gid, &[], Some(1000.0), &mut pen)
            .map_err(|e| fail(format!("CFF charstring draw failed: {e:?}")))?;
        if pen.invalid {
            return Err(fail(
                "CFF outline coordinate is nonfinite or outside bounds",
            ));
        }
        if pen.commands > 1_000_000 {
            return Err(PdfMasterError::Budget("CFF outline command budget"));
        }
    }
    state.font_state.commands = pen.commands;
    Ok(cids)
}

struct CountingPen {
    commands: usize,
    invalid: bool,
}
impl CountingPen {
    fn command(&mut self, coordinates: &[f32]) {
        self.commands = self.commands.saturating_add(1);
        self.invalid |= coordinates
            .iter()
            .any(|v| !v.is_finite() || v.abs() > 1_000_000.0);
    }
}
impl skrifa::outline::OutlinePen for CountingPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.command(&[x, y]);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.command(&[x, y]);
    }
    fn quad_to(&mut self, x0: f32, y0: f32, x1: f32, y1: f32) {
        self.command(&[x0, y0, x1, y1]);
    }
    fn curve_to(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, x2: f32, y2: f32) {
        self.command(&[x0, y0, x1, y1, x2, y2]);
    }
    fn close(&mut self) {
        self.command(&[]);
    }
}

pub(super) fn validate_text(
    font: &AuditedFont,
    bytes: &[u8],
    op: &lopdf::content::Operation,
    owner: &str,
    inside_actual_text: bool,
) -> Result<()> {
    if let AuditedFont::IdentityH {
        glyph_cids,
        unicode_cids,
    } = font
    {
        if !bytes.len().is_multiple_of(2) {
            return Err(super::operation_error(
                owner,
                op,
                "Identity-H string must have even byte length",
            ));
        }
        for pair in bytes.chunks_exact(2) {
            let cid = u16::from_be_bytes([pair[0], pair[1]]);
            if !glyph_cids.contains(&cid) || (!inside_actual_text && !unicode_cids.contains(&cid)) {
                return Err(super::operation_error(
                    owner,
                    op,
                    &format!("Identity-H CID {cid} is not admitted by CFF and ToUnicode"),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_text_context_accepts_unmapped_audited_glyph_for_each_text_show() {
        let font = AuditedFont::IdentityH {
            glyph_cids: BTreeSet::from([1, 2]),
            unicode_cids: BTreeSet::from([1]),
        };
        let results: Vec<_> = ["Tj", "TJ", "'", "\""]
            .into_iter()
            .map(|operator| {
                validate_text(
                    &font,
                    &[0, 2],
                    &lopdf::content::Operation::new(operator, vec![]),
                    "test",
                    true,
                )
                .is_ok()
            })
            .collect();
        assert_eq!(results, vec![true; 4]);
    }

    #[test]
    fn actual_text_context_never_admits_absent_or_odd_cids_or_leaks_outside() {
        let font = AuditedFont::IdentityH {
            glyph_cids: BTreeSet::from([1, 2]),
            unicode_cids: BTreeSet::from([1]),
        };
        for operator in ["Tj", "TJ", "'", "\""] {
            let op = lopdf::content::Operation::new(operator, vec![]);
            for inside in [false, true] {
                assert!(validate_text(&font, &[0, 3], &op, "test", inside).is_err());
                assert!(validate_text(&font, &[0], &op, "test", inside).is_err());
                assert!(validate_text(&font, &[0, 1], &op, "test", inside).is_ok());
            }
            assert!(validate_text(&font, &[0, 2], &op, "test", false).is_err());
        }
    }
    fn program() -> Vec<u8> {
        let doc =
            Document::load_mem(include_bytes!("../tests/fixtures/unicode-ap/chinese.pdf")).unwrap();
        doc.objects
            .values()
            .find_map(|v| {
                v.as_stream()
                    .ok()
                    .filter(|s| {
                        s.dict.get(b"Subtype").and_then(Object::as_name).ok()
                            == Some(b"CIDFontType0C")
                    })
                    .map(|s| s.decompressed_content().unwrap())
            })
            .unwrap()
    }
    #[test]
    fn cff_cumulative_glyph_and_subfont_boundaries() {
        let bytes = program();
        let font = skrifa::raw::ps::cff::CffFontRef::new_cff(&bytes, 0, None).unwrap();
        let mut state = PreflightState::default();
        state.font_state.glyphs = 4096 - font.num_glyphs() as usize;
        state.font_state.subfonts = 256 - usize::from(font.num_subfonts());
        assert!(audit_cff(&bytes, &mut state).is_ok());
        assert!(
            audit_cff(&bytes, &mut state)
                .unwrap_err()
                .to_string()
                .contains("CFF glyph budget")
        );
        let mut state = PreflightState::default();
        state.font_state.subfonts = 257 - usize::from(font.num_subfonts());
        assert!(
            audit_cff(&bytes, &mut state)
                .unwrap_err()
                .to_string()
                .contains("CFF subfont budget")
        );
    }
    #[test]
    fn cff_outline_budget_counts_real_draw_commands() {
        let bytes = program();
        let mut state = PreflightState::default();
        assert!(audit_cff(&bytes, &mut state).is_ok());
        let count = state.font_state.commands;
        assert!(count > 0);
        let mut exact = PreflightState::default();
        exact.font_state.commands = 1_000_000 - count;
        assert!(audit_cff(&bytes, &mut exact).is_ok());
        let mut over = PreflightState::default();
        over.font_state.commands = 1_000_001 - count;
        assert!(
            audit_cff(&bytes, &mut over)
                .unwrap_err()
                .to_string()
                .contains("CFF outline command budget")
        );
    }
    #[test]
    fn cached_graph_still_enforces_full_path_depth() {
        let mut doc = Document::with_version("1.7");
        let mut leaf = Object::Integer(1);
        for _ in 0..10 {
            leaf = Object::Array(vec![leaf]);
        }
        let shared = doc.add_object(leaf);
        let mut state = PreflightState::default();
        audit_graph(&doc, &Object::Reference(shared), &mut state).unwrap();
        let mut chain = shared;
        for _ in 0..60 {
            chain = doc.add_object(Object::Reference(chain));
        }
        assert!(
            audit_graph(&doc, &Object::Reference(chain), &mut state)
                .unwrap_err()
                .to_string()
                .contains("font graph depth")
        );
    }
    #[test]
    fn stream_cache_cannot_skip_context_dictionary_validation() {
        let mut doc = Document::with_version("1.7");
        let id = doc.add_object(lopdf::Stream::new(
            lopdf::dictionary! {"Subtype" => "CIDFontType0C"},
            vec![0],
        ));
        let mut state = PreflightState::default();
        stream(
            &doc,
            &Object::Reference(id),
            &mut state,
            "FontFile3",
            1024,
            1024,
        )
        .unwrap();
        assert!(
            stream(
                &doc,
                &Object::Reference(id),
                &mut state,
                "CIDSet",
                1024,
                1024
            )
            .unwrap_err()
            .to_string()
            .contains("Subtype is unsupported")
        );
    }
    #[test]
    fn actual_stream_with_inconsistent_length_reaches_length_gate() {
        let doc = Document::with_version("1.7");
        let mut value = lopdf::Stream::new(
            lopdf::dictionary! {"Subtype" => "CIDFontType0C"},
            vec![0, 1],
        );
        value.dict.set("Length", 1);
        let error = stream(
            &doc,
            &Object::Stream(value),
            &mut PreflightState::default(),
            "FontFile3",
            1024,
            1024,
        )
        .unwrap_err();
        assert!(error.to_string().contains("FontFile3 Length mismatch"));
    }
    #[test]
    fn shared_stream_aliases_are_charged_once() {
        let mut doc = Document::with_version("1.7");
        let id = doc.add_object(lopdf::Stream::new(
            lopdf::dictionary! {"Subtype" => "CIDFontType0C"},
            vec![0; 20],
        ));
        let alias = doc.add_object(Object::Reference(id));
        let mut state = PreflightState::default();
        stream(
            &doc,
            &Object::Reference(id),
            &mut state,
            "FontFile3",
            1024,
            1024,
        )
        .unwrap();
        stream(
            &doc,
            &Object::Reference(alias),
            &mut state,
            "FontFile3",
            1024,
            1024,
        )
        .unwrap();
        assert_eq!(state.source_resource_bytes, 40);
    }
}
