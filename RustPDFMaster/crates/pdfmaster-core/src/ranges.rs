use std::collections::BTreeSet;

use thiserror::Error;

use crate::document::MAX_PAGES;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum PageRangeError {
    #[error("page range cannot be empty")]
    Empty,
    #[error("page numbers are one-based and cannot be zero")]
    Zero,
    #[error("page count must be positive")]
    InvalidPageCount,
    #[error("page {page} is outside the document's {page_count} pages")]
    OutOfBounds { page: usize, page_count: usize },
    #[error("page range is malformed")]
    Malformed,
    #[error("page number overflows the platform index type")]
    Overflow,
    #[error("page range input exceeds {max_bytes} bytes")]
    InputBudget { max_bytes: usize },
    #[error("page range selects more than {max_pages} pages")]
    SelectionBudget { max_pages: usize },
}

pub struct PageRange;

impl PageRange {
    pub const MAX_INPUT_BYTES: usize = 64 * 1024;

    pub fn parse(text: &str, page_count: usize) -> std::result::Result<Vec<usize>, PageRangeError> {
        if text.len() > Self::MAX_INPUT_BYTES {
            return Err(PageRangeError::InputBudget {
                max_bytes: Self::MAX_INPUT_BYTES,
            });
        }
        if page_count == 0 {
            return Err(PageRangeError::InvalidPageCount);
        }
        if text.trim().is_empty() {
            return Err(PageRangeError::Empty);
        }

        let mut result = BTreeSet::new();
        for raw_piece in text.split(',') {
            let piece = raw_piece.trim();
            if piece.is_empty() {
                return Err(PageRangeError::Malformed);
            }
            let (first, last) = match piece.split_once('-') {
                Some((first, last)) => {
                    if last.contains('-') {
                        return Err(PageRangeError::Malformed);
                    }
                    (parse_page_number(first)?, Some(parse_page_number(last)?))
                }
                None => (parse_page_number(piece)?, None),
            };
            let last = last.unwrap_or(first);
            for page in [first, last] {
                if page > page_count {
                    return Err(PageRangeError::OutOfBounds { page, page_count });
                }
            }
            let start = first.min(last);
            let end = first.max(last);
            let range_len = end - start + 1;
            if range_len > MAX_PAGES {
                return Err(PageRangeError::SelectionBudget {
                    max_pages: MAX_PAGES,
                });
            }
            for page in (start - 1)..end {
                result.insert(page);
                if result.len() > MAX_PAGES {
                    return Err(PageRangeError::SelectionBudget {
                        max_pages: MAX_PAGES,
                    });
                }
            }
        }

        if result.is_empty() {
            return Err(PageRangeError::Empty);
        }
        Ok(result.into_iter().collect())
    }
}

fn parse_page_number(text: &str) -> std::result::Result<usize, PageRangeError> {
    let text = text.trim();
    let digits = text.strip_prefix('+').unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(PageRangeError::Malformed);
    }
    let mut value = 0usize;
    for digit in digits.bytes().map(|byte| usize::from(byte - b'0')) {
        value = value
            .checked_mul(10)
            .and_then(|current| current.checked_add(digit))
            .ok_or(PageRangeError::Overflow)?;
    }
    if value == 0 {
        return Err(PageRangeError::Zero);
    }
    Ok(value)
}
