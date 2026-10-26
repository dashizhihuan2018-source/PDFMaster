use pdfmaster_core::Session;
use serde::Serialize;

pub(crate) const MAX_PAGES: usize = 4_096;
pub(crate) const MAX_QUERY_BYTES: usize = 1_024;
pub(crate) const MAX_QUERY_CHARS: usize = 256;
pub(crate) const MAX_PAGE_TEXT_BYTES: usize = 1_024 * 1_024;
pub(crate) const MAX_PAGE_CHARS: usize = 262_144;
pub(crate) const MAX_FOLDED_TEXT_BYTES: usize = 4 * 1_024 * 1_024;
pub(crate) const MAX_FOLDED_CHARS: usize = MAX_PAGE_CHARS * 4;
pub(crate) const MAX_TOTAL_TEXT_BYTES: usize = 8 * 1_024 * 1_024;
pub(crate) const MAX_HITS: usize = 1_000;
pub(crate) const MAX_TOTAL_SNIPPET_BYTES: usize = 256 * 1_024;
const SNIPPET_BEFORE_CHARS: usize = 35;
const SNIPPET_AFTER_CHARS: usize = 50;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct SearchHit {
    pub page_index: usize,
    pub match_start: usize,
    pub match_end: usize,
    pub snippet: String,
    pub snippet_match_start: usize,
    pub snippet_match_end: usize,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct SearchResponse {
    pub query: String,
    pub case_sensitive: bool,
    pub hits: Vec<SearchHit>,
    pub pages_scanned: usize,
    pub searchable_pages: usize,
    pub truncated: bool,
    pub limitation: String,
}

struct FoldedPage {
    text: String,
    folded_char_bytes: Vec<usize>,
    folded_to_original: Vec<usize>,
}

impl FoldedPage {
    fn new(chars: &[char], case_sensitive: bool) -> Result<Self, String> {
        let mut text = String::new();
        let mut folded_char_bytes = Vec::with_capacity(chars.len() + 1);
        let mut folded_to_original = Vec::with_capacity(chars.len());
        for (original_index, character) in chars.iter().copied().enumerate() {
            if case_sensitive {
                if folded_to_original.len() >= MAX_FOLDED_CHARS
                    || text.len().saturating_add(character.len_utf8()) > MAX_FOLDED_TEXT_BYTES
                {
                    return Err("文字大小写映射超出搜索预算。".into());
                }
                folded_char_bytes.push(text.len());
                folded_to_original.push(original_index);
                text.push(character);
            } else {
                for folded in character.to_lowercase() {
                    if folded_to_original.len() >= MAX_FOLDED_CHARS
                        || text.len().saturating_add(folded.len_utf8()) > MAX_FOLDED_TEXT_BYTES
                    {
                        return Err("文字大小写映射超出搜索预算。".into());
                    }
                    folded_char_bytes.push(text.len());
                    folded_to_original.push(original_index);
                    text.push(folded);
                }
            }
        }
        folded_char_bytes.push(text.len());
        Ok(Self {
            text,
            folded_char_bytes,
            folded_to_original,
        })
    }

    fn original_range(&self, start_byte: usize, end_byte: usize) -> Option<(usize, usize)> {
        let start = self.folded_char_bytes.binary_search(&start_byte).ok()?;
        let end = self.folded_char_bytes.binary_search(&end_byte).ok()?;
        if start >= end || end > self.folded_to_original.len() {
            return None;
        }
        Some((
            self.folded_to_original[start],
            self.folded_to_original[end - 1] + 1,
        ))
    }
}

pub(crate) fn search_session(
    session: &Session,
    query: &str,
    case_sensitive: bool,
) -> Result<SearchResponse, String> {
    let page_count = session.info().page_count;
    let reader = session.text_reader().map_err(|error| error.to_string())?;
    search_with_loader(page_count, query, case_sensitive, |page| {
        reader.text(page).map_err(|error| error.to_string())
    })
}

fn bounded_prefix(text: &str, remaining_bytes: usize) -> (&str, bool) {
    let mut end = text.len().min(MAX_PAGE_TEXT_BYTES).min(remaining_bytes);
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], end < text.len())
}

fn search_with_loader<F>(
    page_count: usize,
    query: &str,
    case_sensitive: bool,
    mut load_page: F,
) -> Result<SearchResponse, String>
where
    F: FnMut(usize) -> Result<String, String>,
{
    let query = query.trim();
    let query_chars = query.chars().count();
    if query.is_empty() {
        return Err("请输入搜索关键词。".into());
    }
    if query.len() > MAX_QUERY_BYTES || query_chars > MAX_QUERY_CHARS {
        return Err(format!(
            "搜索关键词过长（最多 {MAX_QUERY_CHARS} 个字符 / {MAX_QUERY_BYTES} 字节）。"
        ));
    }
    if page_count == 0 || page_count > MAX_PAGES {
        return Err(format!("文档页数超出搜索范围（1–{MAX_PAGES} 页）。"));
    }
    let needle = if case_sensitive {
        query.to_owned()
    } else {
        query.to_lowercase()
    };
    let mut response = SearchResponse {
        query: query.to_owned(),
        case_sensitive,
        hits: Vec::new(),
        pages_scanned: 0,
        searchable_pages: 0,
        truncated: false,
        limitation: "页内纯文本匹配；不跨页，不提供字形坐标或高亮选区。".into(),
    };
    let mut processed_text_bytes = 0usize;
    let mut snippet_bytes = 0usize;

    for page_index in 0..page_count {
        if processed_text_bytes >= MAX_TOTAL_TEXT_BYTES {
            response.truncated = true;
            break;
        }
        let text = load_page(page_index)
            .map_err(|error| format!("第 {} 页文字无法安全提取：{error}", page_index + 1))?;
        response.pages_scanned += 1;
        let remaining = MAX_TOTAL_TEXT_BYTES - processed_text_bytes;
        let (prefix, bytes_truncated) = bounded_prefix(&text, remaining);
        processed_text_bytes = processed_text_bytes.saturating_add(prefix.len());
        response.truncated |= bytes_truncated;

        let mut chars: Vec<char> = prefix.chars().collect();
        if chars.len() > MAX_PAGE_CHARS {
            chars.truncate(MAX_PAGE_CHARS);
            response.truncated = true;
        }
        if chars.iter().any(|character| !character.is_whitespace()) {
            response.searchable_pages += 1;
        } else {
            continue;
        }

        let folded = FoldedPage::new(&chars, case_sensitive)
            .map_err(|error| format!("第 {} 页{error}", page_index + 1))?;
        for (start_byte, matched) in folded.text.match_indices(&needle) {
            let Some((match_start, match_end)) =
                folded.original_range(start_byte, start_byte + matched.len())
            else {
                return Err(format!(
                    "第 {} 页文字大小写映射无法安全定位。",
                    page_index + 1
                ));
            };
            let snippet_start = match_start.saturating_sub(SNIPPET_BEFORE_CHARS);
            let snippet_end = chars
                .len()
                .min(match_end.saturating_add(SNIPPET_AFTER_CHARS));
            let snippet: String = chars[snippet_start..snippet_end]
                .iter()
                .copied()
                .map(|character| {
                    if character == '\n' || character == '\r' {
                        ' '
                    } else {
                        character
                    }
                })
                .collect();
            if snippet_bytes.saturating_add(snippet.len()) > MAX_TOTAL_SNIPPET_BYTES {
                response.truncated = true;
                return Ok(response);
            }
            snippet_bytes += snippet.len();
            response.hits.push(SearchHit {
                page_index,
                match_start,
                match_end,
                snippet,
                snippet_match_start: match_start - snippet_start,
                snippet_match_end: match_end - snippet_start,
            });
            if response.hits.len() >= MAX_HITS {
                response.truncated = true;
                return Ok(response);
            }
        }
    }
    if response.searchable_pages == 0 && !response.truncated {
        return Err("文档没有可搜索文字；扫描页需要先经过 OCR。".into());
    }
    Ok(response)
}

#[cfg(test)]
fn search_text_pages<const N: usize>(
    pages: [&str; N],
    query: &str,
    case_sensitive: bool,
) -> Result<SearchResponse, String> {
    search_with_loader(N, query, case_sensitive, |page| Ok(pages[page].to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_match_positions_and_snippet_offsets_are_character_based() {
        let result = search_text_pages(["零甲乙甲尾"], "乙甲", true).unwrap();
        assert_eq!(result.hits.len(), 1);
        let hit = &result.hits[0];
        assert_eq!((hit.match_start, hit.match_end), (2, 4));
        assert_eq!(
            hit.snippet
                .chars()
                .skip(hit.snippet_match_start)
                .take(hit.snippet_match_end - hit.snippet_match_start)
                .collect::<String>(),
            "乙甲"
        );
    }

    #[test]
    fn plain_search_supports_explicit_unicode_case_sensitivity() {
        assert_eq!(
            search_text_pages(["Rust rust RUST"], "rust", false)
                .unwrap()
                .hits
                .len(),
            3
        );
        assert_eq!(
            search_text_pages(["Rust rust RUST"], "rust", true)
                .unwrap()
                .hits
                .len(),
            1
        );
        assert_eq!(
            search_text_pages(["Été ÉTÉ"], "été", false)
                .unwrap()
                .hits
                .len(),
            2
        );
    }

    #[test]
    fn hit_budget_sets_truncation_instead_of_allocating_without_bound() {
        let text = "a ".repeat(MAX_HITS + 20);
        let result = search_text_pages([text.as_str()], "a", true).unwrap();
        assert_eq!(result.hits.len(), MAX_HITS);
        assert!(result.truncated);
    }

    #[test]
    fn extraction_failure_and_all_empty_pages_are_not_false_zero_results() {
        let error = search_with_loader(2, "x", true, |page| {
            if page == 1 {
                Err("bad text stream".into())
            } else {
                Ok("x".into())
            }
        })
        .unwrap_err();
        assert!(error.contains("第 2 页"));
        assert!(
            search_text_pages(["", " \n"], "x", true)
                .unwrap_err()
                .contains("可搜索文字")
        );
    }

    #[test]
    fn query_and_page_budgets_are_explicitly_rejected() {
        assert!(search_text_pages(["text"], "", true).is_err());
        assert!(search_text_pages(["text"], &"x".repeat(MAX_QUERY_CHARS + 1), true).is_err());
        assert!(search_with_loader(MAX_PAGES + 1, "x", true, |_| Ok("x".into())).is_err());
    }

    #[test]
    fn exhausted_text_budget_does_not_claim_unscanned_pages_need_ocr() {
        let result = search_with_loader(9, "needle", true, |page| {
            if page < 8 {
                Ok(" ".repeat(MAX_PAGE_TEXT_BYTES))
            } else {
                Ok("needle".into())
            }
        })
        .unwrap();
        assert!(result.truncated);
        assert_eq!(result.pages_scanned, 8);
        assert_eq!(result.searchable_pages, 0);
        assert!(result.hits.is_empty());
    }

    #[test]
    fn unicode_expansion_utf8_prefix_and_snippet_budget_remain_bounded() {
        let expanded = search_text_pages(["İ"], "i\u{307}", false).unwrap();
        assert_eq!(
            (expanded.hits[0].match_start, expanded.hits[0].match_end),
            (0, 1)
        );

        let utf8 = "界".repeat(MAX_PAGE_TEXT_BYTES / 3 + 10);
        let prefixed = search_text_pages([utf8.as_str()], "不存在", true).unwrap();
        assert!(prefixed.truncated);

        let query = "x".repeat(MAX_QUERY_CHARS);
        let many = format!("{} ", query).repeat(MAX_HITS);
        let snippets = search_text_pages([many.as_str()], &query, true).unwrap();
        assert!(snippets.truncated);
        assert!(snippets.hits.len() < MAX_HITS);
        assert!(
            snippets
                .hits
                .iter()
                .map(|hit| hit.snippet.len())
                .sum::<usize>()
                <= MAX_TOTAL_SNIPPET_BYTES
        );
    }
}
