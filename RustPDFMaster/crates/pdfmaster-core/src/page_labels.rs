//! Read-only PDF 32000-1 sections 7.9.7/12.4.2; never rewrites catalog labels.
use std::collections::HashSet;

use lopdf::{Dictionary, Document, Object, ObjectId, decode_text_string};

use crate::{PdfMasterError, Result, Session, document::page_object_ids};

const MAX_DEPTH: usize = 64;
const MAX_NODES: usize = 8192;
const MAX_RANGES: usize = 4096;
const MAX_LABEL_BYTES: usize = 8192;
const MAX_TOTAL_BYTES: usize = 1024 * 1024;

fn malformed(message: &str) -> PdfMasterError {
    PdfMasterError::Malformed(format!("page labels: {message}"))
}

fn resolve<'a>(
    document: &'a Document,
    mut value: &'a Object,
) -> Result<(&'a Object, Option<ObjectId>)> {
    let mut visited = HashSet::new();
    let mut last = None;
    for _ in 0..=64 {
        match value {
            Object::Reference(id) => {
                if visited.len() >= 64 {
                    return Err(PdfMasterError::Budget("page label reference depth"));
                }
                if !visited.insert(*id) {
                    return Err(malformed("reference cycle"));
                }
                last = Some(*id);
                value = document
                    .objects
                    .get(id)
                    .ok_or_else(|| malformed("missing reference"))?;
            }
            _ => return Ok((value, last)),
        }
    }
    Err(PdfMasterError::Budget("page label reference depth"))
}

fn field<'a>(
    document: &'a Document,
    dictionary: &'a Dictionary,
    key: &[u8],
) -> Result<Option<&'a Object>> {
    let value = dictionary
        .get(key)
        .ok()
        .map(|v| resolve(document, v))
        .transpose()?
        .map(|v| v.0);
    Ok(value.filter(|value| !matches!(value, Object::Null)))
}

fn integer(value: &Object) -> Result<u64> {
    match value {
        Object::Integer(value) => u64::try_from(*value).map_err(|_| malformed("negative integer")),
        _ => Err(malformed("integer expected")),
    }
}

struct Rule {
    prefix: String,
    style: Option<u8>,
    start: u64,
}

struct Tree<'a> {
    document: &'a Document,
    page_count: usize,
    nodes: usize,
    seen: HashSet<ObjectId>,
    raw_prefix_bytes: usize,
    decoded_prefix_bytes: usize,
    ranges: Vec<(usize, Rule)>,
}

impl Tree<'_> {
    fn rule(&mut self, value: &Object) -> Result<Rule> {
        let dictionary = resolve(self.document, value)?
            .0
            .as_dict()
            .map_err(|_| malformed("label dictionary expected"))?;
        if let Some(value) = field(self.document, dictionary, b"Type")?
            && !matches!(value,Object::Name(name) if name==b"PageLabel")
        {
            return Err(malformed("invalid label type"));
        }
        let style = match field(self.document, dictionary, b"S")? {
            None => None,
            Some(Object::Name(name))
                if matches!(name.as_slice(), b"D" | b"R" | b"r" | b"A" | b"a") =>
            {
                Some(name[0])
            }
            _ => return Err(malformed("unknown numbering style")),
        };
        let start = field(self.document, dictionary, b"St")?
            .map(integer)
            .transpose()?
            .unwrap_or(1);
        if start == 0 {
            return Err(malformed("numbering must start at one or later"));
        }
        let prefix = match field(self.document, dictionary, b"P")? {
            None => String::new(),
            Some(value @ Object::String(bytes, _)) => {
                if bytes.len() > MAX_LABEL_BYTES {
                    return Err(PdfMasterError::Budget("page label prefix bytes"));
                }
                self.raw_prefix_bytes = self
                    .raw_prefix_bytes
                    .checked_add(bytes.len())
                    .ok_or(PdfMasterError::Budget("page label prefix bytes"))?;
                if self.raw_prefix_bytes > MAX_TOTAL_BYTES {
                    return Err(PdfMasterError::Budget("page label prefix bytes"));
                }
                // The dependency deliberately tolerates an odd UTF-16 trailing byte
                // and skips undefined PDFDocEncoding entries. Labels must not lose data.
                if bytes.starts_with(b"\xFE\xFF") && bytes.len() % 2 != 0 {
                    return Err(malformed("odd UTF-16 prefix length"));
                }
                let decoded =
                    decode_text_string(value).map_err(|_| malformed("invalid prefix text"))?;
                // PDFDocEncoding maps each defined byte to exactly one BMP scalar.
                // A shorter result means the dependency silently skipped undefined input.
                if !bytes.starts_with(b"\xFE\xFF")
                    && !bytes.starts_with(b"\xEF\xBB\xBF")
                    && decoded.chars().count() != bytes.len()
                {
                    return Err(malformed("undefined PDFDocEncoding prefix byte"));
                }
                decoded
            }
            _ => return Err(malformed("prefix must be text")),
        };
        self.decoded_prefix_bytes = self
            .decoded_prefix_bytes
            .checked_add(prefix.len())
            .ok_or(PdfMasterError::Budget("page label prefix bytes"))?;
        if prefix.len() > MAX_LABEL_BYTES || self.decoded_prefix_bytes > MAX_TOTAL_BYTES {
            return Err(PdfMasterError::Budget("page label prefix bytes"));
        }
        Ok(Rule {
            prefix,
            style,
            start,
        })
    }

    fn walk(&mut self, value: &Object, depth: usize) -> Result<()> {
        self.nodes += 1;
        if self.nodes > MAX_NODES || depth > MAX_DEPTH {
            return Err(PdfMasterError::Budget("page label tree nodes or depth"));
        }
        let (value, id) = resolve(self.document, value)?;
        if id.is_some_and(|id| !self.seen.insert(id)) {
            return Err(malformed("cycle or shared number-tree node"));
        }
        let dictionary = value
            .as_dict()
            .map_err(|_| malformed("number-tree dictionary expected"))?;
        let before = self.ranges.len();
        match (
            field(self.document, dictionary, b"Nums")?,
            field(self.document, dictionary, b"Kids")?,
        ) {
            (Some(Object::Array(nums)), None) => {
                if nums.len() > MAX_RANGES * 2 {
                    return Err(PdfMasterError::Budget("page label ranges"));
                }
                if nums.len() % 2 != 0 {
                    return Err(malformed("odd number-tree pairs"));
                }
                for pair in nums.chunks_exact(2) {
                    if self.ranges.len() >= MAX_RANGES {
                        return Err(PdfMasterError::Budget("page label ranges"));
                    }
                    let index = usize::try_from(integer(resolve(self.document, &pair[0])?.0)?)
                        .map_err(|_| malformed("invalid page index"))?;
                    if index >= self.page_count
                        || self.ranges.last().is_some_and(|(last, _)| *last >= index)
                    {
                        return Err(malformed(
                            "page indices must be local, unique and ascending",
                        ));
                    }
                    let rule = self.rule(&pair[1])?;
                    self.ranges.push((index, rule));
                }
            }
            (None, Some(Object::Array(kids))) => {
                if kids.len() > MAX_RANGES {
                    return Err(PdfMasterError::Budget("page label children"));
                }
                for child in kids {
                    if !matches!(child, Object::Reference(_)) {
                        return Err(malformed("child must be an indirect reference"));
                    }
                    self.walk(child, depth + 1)?;
                }
            }
            _ => return Err(malformed("exactly one Nums or Kids array is required")),
        }
        if self.ranges.len() == before {
            return Err(malformed("empty number-tree node"));
        }
        match field(self.document, dictionary, b"Limits")? {
            Some(Object::Array(limits)) if limits.len() == 2 => {
                let low = integer(resolve(self.document, &limits[0])?.0)?;
                let high = integer(resolve(self.document, &limits[1])?.0)?;
                if low != self.ranges[before].0 as u64
                    || high != self.ranges.last().unwrap().0 as u64
                {
                    return Err(malformed("number-tree limits do not match descendants"));
                }
            }
            None if depth == 0 => {}
            _ => return Err(malformed("non-root number-tree limits are required")),
        }
        Ok(())
    }
}

fn numeral(style: u8, mut number: u64) -> Result<String> {
    match style {
        b'D' => Ok(number.to_string()),
        b'A' | b'a' => {
            let count = (number - 1) / 26 + 1;
            if count > MAX_LABEL_BYTES as u64 {
                return Err(PdfMasterError::Budget("page label expansion"));
            }
            let letter = (if style == b'A' { b'A' } else { b'a' }) + ((number - 1) % 26) as u8;
            Ok((letter as char).to_string().repeat(count as usize))
        }
        b'R' | b'r' => {
            const TOKENS: [(u64, &str); 13] = [
                (1000, "M"),
                (900, "CM"),
                (500, "D"),
                (400, "CD"),
                (100, "C"),
                (90, "XC"),
                (50, "L"),
                (40, "XL"),
                (10, "X"),
                (9, "IX"),
                (5, "V"),
                (4, "IV"),
                (1, "I"),
            ];
            let mut result = String::new();
            for (value, token) in TOKENS {
                let repeat = number / value;
                if repeat > (MAX_LABEL_BYTES - result.len()) as u64 / token.len() as u64 {
                    return Err(PdfMasterError::Budget("page label expansion"));
                }
                for _ in 0..repeat {
                    result.push_str(token);
                }
                number %= value;
            }
            if style == b'r' {
                result.make_ascii_lowercase();
            }
            Ok(result)
        }
        _ => Err(malformed("unknown numbering style")),
    }
}

impl Session {
    /// Read-only labels for current catalog page indices, or physical numbers if absent/null.
    /// Malformed trees/text return an error, not fabricated numeric labels. Reads are
    /// bounded by 64 tree/reference levels, 8192 nodes, 4096 ranges, 8192 bytes per
    /// prefix/label and separate 1MiB raw-prefix, decoded-prefix and output totals.
    /// Structural page edits do not migrate catalog ranges; stale ranges can fail here.
    pub fn page_labels(&self) -> Result<Vec<String>> {
        let count = page_object_ids(&self.document)?.len();
        let catalog = self
            .document
            .catalog()
            .map_err(|_| malformed("catalog unavailable"))?;
        let Some(root) = field(&self.document, catalog, b"PageLabels")? else {
            return Ok((1..=count).map(|page| page.to_string()).collect());
        };
        let mut tree = Tree {
            document: &self.document,
            page_count: count,
            nodes: 0,
            seen: HashSet::new(),
            raw_prefix_bytes: 0,
            decoded_prefix_bytes: 0,
            ranges: Vec::new(),
        };
        tree.walk(root, 0)?;
        if tree.ranges.first().map(|(index, _)| *index) != Some(0) {
            return Err(malformed("first label range must start at page zero"));
        }
        let mut labels = Vec::with_capacity(count);
        let mut range = 0;
        let mut total = 0usize;
        for page in 0..count {
            while range + 1 < tree.ranges.len() && tree.ranges[range + 1].0 <= page {
                range += 1;
            }
            let (first, rule) = &tree.ranges[range];
            let suffix = if let Some(style) = rule.style {
                let number = rule
                    .start
                    .checked_add((page - first) as u64)
                    .ok_or(PdfMasterError::Budget("page label counter"))?;
                numeral(style, number)?
            } else {
                String::new()
            };
            let length = rule.prefix.len() + suffix.len();
            total = total
                .checked_add(length)
                .ok_or(PdfMasterError::Budget("page label output bytes"))?;
            if length > MAX_LABEL_BYTES || total > MAX_TOTAL_BYTES {
                return Err(PdfMasterError::Budget("page label output bytes"));
            }
            labels.push(format!("{}{}", rule.prefix, suffix));
        }
        Ok(labels)
    }
}
