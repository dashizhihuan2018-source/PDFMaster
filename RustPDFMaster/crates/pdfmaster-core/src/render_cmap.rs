//! A finite declarative ToUnicode grammar, not a PostScript interpreter.
use super::fonts::CidSystemInfo;
use crate::{PdfMasterError, Result};
use std::collections::{BTreeSet, HashSet};

fn fail(message: impl std::fmt::Display) -> PdfMasterError {
    PdfMasterError::Render(format!("ToUnicode CMap audit failed: {message}"))
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Word(Vec<u8>),
    Name(Vec<u8>),
    String(Vec<u8>),
    Hex(Vec<u8>),
    OpenDict,
    CloseDict,
    OpenArray,
    CloseArray,
}
fn whitespace(c: u8) -> bool {
    matches!(c, 0 | 9 | 10 | 12 | 13 | 32)
}
fn delimiter(c: u8) -> bool {
    whitespace(c) || b"()<>[]{}/%".contains(&c)
}
fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

fn lex(bytes: &[u8]) -> Result<Vec<Token>> {
    if bytes.len() > 1024 * 1024 {
        return Err(fail("CMap decoded bytes"));
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if whitespace(bytes[i]) {
            i += 1;
            continue;
        }
        if bytes[i] == b'%' {
            while i < bytes.len() && !matches!(bytes[i], b'\r' | b'\n') {
                i += 1;
            }
            continue;
        }
        let start = i;
        let token = match bytes[i] {
            b'<' if bytes.get(i + 1) == Some(&b'<') => {
                i += 2;
                Token::OpenDict
            }
            b'>' if bytes.get(i + 1) == Some(&b'>') => {
                i += 2;
                Token::CloseDict
            }
            b'[' => {
                i += 1;
                Token::OpenArray
            }
            b']' => {
                i += 1;
                Token::CloseArray
            }
            b'<' => {
                i += 1;
                let mut value = Vec::new();
                let mut nibble = None;
                while i < bytes.len() && bytes[i] != b'>' {
                    let c = bytes[i];
                    i += 1;
                    if i - start > 256 {
                        return Err(fail("CMap token bytes"));
                    }
                    if whitespace(c) {
                        continue;
                    }
                    let n = hex(c).ok_or_else(|| fail("invalid hex token"))?;
                    if let Some(high) = nibble.take() {
                        value.push(high * 16 + n);
                    } else {
                        nibble = Some(n);
                    }
                }
                if bytes.get(i) != Some(&b'>') || nibble.is_some() {
                    return Err(fail("unterminated or odd hex token"));
                }
                i += 1;
                Token::Hex(value)
            }
            b'(' => {
                i += 1;
                let mut depth = 1usize;
                let mut value = Vec::new();
                while depth != 0 {
                    let c = *bytes
                        .get(i)
                        .ok_or_else(|| fail("unterminated literal string"))?;
                    i += 1;
                    if i - start > 256 {
                        return Err(fail("CMap token bytes"));
                    }
                    match c {
                        b'(' => {
                            depth += 1;
                            value.push(c);
                        }
                        b')' => {
                            depth -= 1;
                            if depth > 0 {
                                value.push(c);
                            }
                        }
                        b'\\' => {
                            let c = *bytes
                                .get(i)
                                .ok_or_else(|| fail("unterminated string escape"))?;
                            i += 1;
                            match c {
                                b'n' => value.push(b'\n'),
                                b'r' => value.push(b'\r'),
                                b't' => value.push(b'\t'),
                                b'b' => value.push(8),
                                b'f' => value.push(12),
                                b'\r' => {
                                    if bytes.get(i) == Some(&b'\n') {
                                        i += 1;
                                    }
                                }
                                b'\n' => {}
                                b'0'..=b'7' => {
                                    let mut n = u16::from(c - b'0');
                                    for _ in 0..2 {
                                        if let Some(c @ b'0'..=b'7') = bytes.get(i) {
                                            n = n * 8 + u16::from(*c - b'0');
                                            i += 1;
                                        } else {
                                            break;
                                        }
                                    }
                                    value.push(n as u8);
                                }
                                _ => value.push(c),
                            }
                        }
                        _ => value.push(c),
                    }
                }
                Token::String(value)
            }
            b'/' => {
                i += 1;
                let mut name = Vec::new();
                while i < bytes.len() && !delimiter(bytes[i]) {
                    if bytes[i] == b'#' {
                        let a = bytes
                            .get(i + 1)
                            .and_then(|c| hex(*c))
                            .ok_or_else(|| fail("invalid name escape"))?;
                        let b = bytes
                            .get(i + 2)
                            .and_then(|c| hex(*c))
                            .ok_or_else(|| fail("invalid name escape"))?;
                        name.push(a * 16 + b);
                        i += 3;
                    } else {
                        name.push(bytes[i]);
                        i += 1;
                    }
                    if i - start > 256 {
                        return Err(fail("CMap token bytes"));
                    }
                }
                Token::Name(name)
            }
            c if !delimiter(c) => {
                while i < bytes.len() && !delimiter(bytes[i]) {
                    i += 1;
                    if i - start > 256 {
                        return Err(fail("CMap token bytes"));
                    }
                }
                Token::Word(bytes[start..i].to_vec())
            }
            _ => return Err(fail("unknown delimiter")),
        };
        if i - start > 256 {
            return Err(fail("CMap token bytes"));
        }
        if out.len() == 65_536 {
            return Err(fail("CMap token count"));
        }
        out.push(token);
    }
    Ok(out)
}

struct Parser {
    tokens: Vec<Token>,
    index: usize,
}
impl Parser {
    fn take(&mut self) -> Result<Token> {
        let token = self
            .tokens
            .get(self.index)
            .cloned()
            .ok_or_else(|| fail("unexpected end"))?;
        self.index += 1;
        Ok(token)
    }
    fn expect(&mut self, token: Token) -> Result<()> {
        if self.take()? != token {
            return Err(fail("unexpected declaration token"));
        }
        Ok(())
    }
    fn word(&mut self, word: &[u8]) -> Result<()> {
        self.expect(Token::Word(word.to_vec()))
    }
    fn name(&mut self, name: &[u8]) -> Result<()> {
        self.expect(Token::Name(name.to_vec()))
    }
    fn integer(&mut self) -> Result<i64> {
        let Token::Word(bytes) = self.take()? else {
            return Err(fail("expected integer"));
        };
        std::str::from_utf8(&bytes)
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| fail("invalid integer"))
    }
    fn source(&mut self) -> Result<u16> {
        let Token::Hex(bytes) = self.take()? else {
            return Err(fail("source code must be hex"));
        };
        if bytes.len() != 2 {
            return Err(fail("source code must contain two bytes"));
        }
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }
    fn target(&mut self) -> Result<Vec<u8>> {
        let Token::Hex(bytes) = self.take()? else {
            return Err(fail("target must be hex"));
        };
        validate_target(&bytes)?;
        Ok(bytes)
    }
    fn info(&mut self) -> Result<CidSystemInfo> {
        let literal = self.tokens.get(self.index) == Some(&Token::OpenDict);
        if literal {
            self.expect(Token::OpenDict)?;
        } else {
            if self.integer()? != 3 {
                return Err(fail("CIDSystemInfo must use 3 dict"));
            }
            self.word(b"dict")?;
            self.word(b"dup")?;
            self.word(b"begin")?;
        }
        let mut registry = None;
        let mut ordering = None;
        let mut supplement = None;
        for _ in 0..3 {
            match self.take()? {
                Token::Name(key) if key == b"Registry" && registry.is_none() => {
                    let Token::String(s) = self.take()? else {
                        return Err(fail("Registry must be a string"));
                    };
                    registry = Some(s);
                }
                Token::Name(key) if key == b"Ordering" && ordering.is_none() => {
                    let Token::String(s) = self.take()? else {
                        return Err(fail("Ordering must be a string"));
                    };
                    ordering = Some(s);
                }
                Token::Name(key) if key == b"Supplement" && supplement.is_none() => {
                    supplement = Some(
                        i32::try_from(self.integer()?)
                            .ok()
                            .filter(|n| *n >= 0)
                            .ok_or_else(|| fail("invalid Supplement"))?,
                    );
                }
                _ => return Err(fail("invalid CIDSystemInfo field")),
            }
            if !literal {
                self.word(b"def")?;
            }
        }
        if literal {
            self.expect(Token::CloseDict)?;
        } else {
            self.word(b"end")?;
        }
        self.word(b"def")?;
        let info = CidSystemInfo {
            registry: registry.ok_or_else(|| fail("missing Registry"))?,
            ordering: ordering.ok_or_else(|| fail("missing Ordering"))?,
            supplement: supplement.ok_or_else(|| fail("missing Supplement"))?,
        };
        if [&info.registry, &info.ordering]
            .iter()
            .any(|v| v.is_empty() || v.len() > 127)
        {
            return Err(fail("CIDSystemInfo string length"));
        }
        Ok(info)
    }
}

fn validate_target(bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() || bytes.len() > 64 || !bytes.len().is_multiple_of(2) {
        return Err(fail(
            "target length must be nonempty even and at most 64 bytes",
        ));
    }
    if char::decode_utf16(
        bytes
            .chunks_exact(2)
            .map(|p| u16::from_be_bytes([p[0], p[1]])),
    )
    .any(|v| v.is_err())
    {
        return Err(fail("target contains malformed UTF-16"));
    }
    Ok(())
}

fn insert(mapping: &mut BTreeSet<u16>, cids: &BTreeSet<u16>, cid: u16) -> Result<()> {
    if !cids.contains(&cid) {
        return Err(fail(format!("source CID {cid} is absent from CFF charset")));
    }
    if !mapping.insert(cid) {
        return Err(fail(format!("source CID {cid} is duplicated")));
    }
    if mapping.len() > 4096 {
        return Err(fail("CMap mapping count"));
    }
    Ok(())
}

pub(super) fn audit(
    bytes: &[u8],
    info: &CidSystemInfo,
    cids: &BTreeSet<u16>,
) -> Result<BTreeSet<u16>> {
    let mut p = Parser {
        tokens: lex(bytes)?,
        index: 0,
    };
    p.name(b"CIDInit")?;
    p.name(b"ProcSet")?;
    p.word(b"findresource")?;
    p.word(b"begin")?;
    if p.integer()? != 12 {
        return Err(fail("prolog requires 12 dict"));
    }
    p.word(b"dict")?;
    p.word(b"begin")?;
    p.word(b"begincmap")?;
    let mut headers = HashSet::new();
    while let Some(Token::Name(_)) = p.tokens.get(p.index) {
        let Token::Name(key) = p.take()? else {
            unreachable!()
        };
        if !headers.insert(key.clone()) {
            return Err(fail("duplicate CMap header"));
        }
        match key.as_slice() {
            b"CIDSystemInfo" => {
                if p.info()? != *info {
                    return Err(fail("CIDSystemInfo does not match descendant"));
                }
            }
            b"CMapName" => {
                let Token::Name(name) = p.take()? else {
                    return Err(fail("CMapName must be a name"));
                };
                if name.is_empty() {
                    return Err(fail("CMapName must not be empty"));
                }
                p.word(b"def")?;
            }
            b"CMapVersion" => {
                let Token::Word(bytes) = p.take()? else {
                    return Err(fail("invalid CMapVersion"));
                };
                if !std::str::from_utf8(&bytes)
                    .ok()
                    .and_then(|s| s.parse::<f64>().ok())
                    .is_some_and(|n| n.is_finite() && n >= 0.0)
                {
                    return Err(fail("invalid CMapVersion"));
                }
                p.word(b"def")?;
            }
            b"CMapType" => {
                if !matches!(p.integer()?, 0 | 2) {
                    return Err(fail("CMapType must be 0 or 2"));
                }
                p.word(b"def")?;
            }
            b"WMode" => {
                if p.integer()? != 0 {
                    return Err(fail("WMode must be 0"));
                }
                p.word(b"def")?;
            }
            _ => return Err(fail("unknown or missing CMap header declaration")),
        }
    }
    if [b"CIDSystemInfo".as_slice(), b"CMapName", b"CMapType"]
        .iter()
        .any(|key| !headers.contains(*key))
    {
        return Err(fail("unknown or missing CMap header declaration"));
    }
    if p.integer()? != 1 {
        return Err(fail("requires exactly one codespace"));
    }
    p.word(b"begincodespacerange")?;
    if p.source()? != 0 || p.source()? != 65535 {
        return Err(fail("requires Identity-H codespace"));
    }
    p.word(b"endcodespacerange")?;
    let mut mapping = BTreeSet::new();
    let mut blocks = 0;
    while p.tokens.get(p.index) != Some(&Token::Word(b"endcmap".to_vec())) {
        let count = p.integer()?;
        if !(0..=100).contains(&count) {
            return Err(fail("block count must be 0..=100"));
        }
        blocks += 1;
        let block = p.take()?;
        match block {
            Token::Word(word) if word == b"beginbfchar" => {
                for _ in 0..count {
                    let cid = p.source()?;
                    p.target()?;
                    insert(&mut mapping, cids, cid)?;
                }
                p.word(b"endbfchar")?;
            }
            Token::Word(word) if word == b"beginbfrange" => {
                for _ in 0..count {
                    let first = p.source()?;
                    let last = p.source()?;
                    if first > last {
                        return Err(fail("reversed bfrange"));
                    }
                    // Reserve the complete interval before parsing a target or expanding it.
                    let additional = usize::from(last) - usize::from(first) + 1;
                    if additional > 4096 - mapping.len() {
                        return Err(fail("CMap mapping count"));
                    }
                    if p.tokens.get(p.index) == Some(&Token::OpenArray) {
                        p.expect(Token::OpenArray)?;
                        for cid in first..=last {
                            p.target()?;
                            insert(&mut mapping, cids, cid)?;
                        }
                        p.expect(Token::CloseArray)?;
                    } else {
                        let mut target = p.target()?;
                        for cid in first..=last {
                            validate_target(&target)?;
                            insert(&mut mapping, cids, cid)?;
                            if cid != last {
                                let mut carry = true;
                                for byte in target.iter_mut().rev() {
                                    let (next, overflow) = byte.overflowing_add(1);
                                    *byte = next;
                                    carry = overflow;
                                    if !carry {
                                        break;
                                    }
                                }
                                if carry {
                                    return Err(fail("bfrange target overflow"));
                                }
                            }
                        }
                    }
                }
                p.word(b"endbfrange")?;
            }
            _ => return Err(fail("unknown mapping block")),
        }
    }
    if blocks == 0 || mapping.is_empty() {
        return Err(fail("CMap requires nonempty mapping"));
    }
    p.word(b"endcmap")?;
    p.word(b"CMapName")?;
    p.word(b"currentdict")?;
    p.name(b"CMap")?;
    p.word(b"defineresource")?;
    p.word(b"pop")?;
    p.word(b"end")?;
    p.word(b"end")?;
    if p.index != p.tokens.len() {
        return Err(fail("trailing executable token"));
    }
    Ok(mapping)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn program(blocks: &str) -> Vec<u8> {
        format!("/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (R) /Ordering (O) /Supplement 0 >> def /CMapName /T def /CMapType 2 def 1 begincodespacerange <0000> <FFFF> endcodespacerange {blocks} endcmap CMapName currentdict /CMap defineresource pop end end").into_bytes()
    }
    fn parse(blocks: &str) -> Result<BTreeSet<u16>> {
        audit(
            &program(blocks),
            &CidSystemInfo {
                registry: b"R".to_vec(),
                ordering: b"O".to_vec(),
                supplement: 0,
            },
            &(0..=65535).collect(),
        )
    }
    fn characters(count: usize) -> String {
        let mut blocks = String::new();
        for start in (0..count).step_by(100) {
            let end = count.min(start + 100);
            blocks.push_str(&format!("{} beginbfchar ", end - start));
            for cid in start..end {
                blocks.push_str(&format!("<{cid:04X}> <0041> "));
            }
            blocks.push_str("endbfchar ");
        }
        blocks
    }
    #[test]
    fn mapping_admission_boundary_uses_actual_4096_and_4097_mappings() {
        assert_eq!(parse(&characters(4096)).unwrap().len(), 4096);
        assert!(
            parse(&characters(4097))
                .unwrap_err()
                .to_string()
                .contains("CMap mapping count")
        );
    }
    #[test]
    fn range_mapping_boundary_and_preexpansion_gate() {
        assert_eq!(
            parse("1 beginbfrange <0000> <0FFF> <00410000> endbfrange")
                .unwrap()
                .len(),
            4096
        );
        assert!(
            parse("1 beginbfrange <0000> <1000> <00410000> endbfrange")
                .unwrap_err()
                .to_string()
                .contains("CMap mapping count")
        );
    }
    #[test]
    fn excessive_range_is_rejected_before_target_parsing() {
        // Reject the cardinality before reading/expanding an invalid target.
        assert!(
            parse("1 beginbfrange <0000> <FFFF> <D800> endbfrange")
                .unwrap_err()
                .to_string()
                .contains("CMap mapping count")
        );
    }
}
