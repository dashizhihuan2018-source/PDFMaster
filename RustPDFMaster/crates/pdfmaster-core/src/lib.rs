//! Safe, transactional PDF primitives shared by every PDF Master shell.

mod document;
mod edit;
mod render;

pub use document::{Color, DocumentInfo, Edit, PageInfo, PdfMasterError, Rect, Result, Session};
