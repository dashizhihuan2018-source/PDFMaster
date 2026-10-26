//! Safe, transactional PDF primitives shared by every PDF Master shell.

mod document;
mod edit;
mod extract;
mod notes;
mod ranges;
mod render;

pub use document::{Color, DocumentInfo, Edit, PageInfo, PdfMasterError, Rect, Result, Session};
pub use notes::{NoteList, TextNote};
pub use ranges::{PageRange, PageRangeError};
