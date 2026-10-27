//! Safe, transactional PDF primitives shared by every PDF Master shell.

mod annotations;
mod document;
mod edit;
mod extract;
mod ink;
mod insert;
mod lines;
mod notes;
mod outline;
mod page_labels;
mod ranges;
mod render;
mod shapes;

pub use annotations::AnnotationRow;
pub use document::{
    Color, DocumentInfo, Edit, PageInfo, PdfMasterError, Rect, Result, Session, ShapeKind,
    TextReader,
};
pub use insert::{MAX_INSERT_BYTES, MAX_INSERT_FILES};
pub use lines::{LineEnding, LineGeometry, Point};
pub use notes::{NoteList, TextNote};
pub use outline::OutlineItem;
pub use ranges::{PageRange, PageRangeError};
pub use shapes::{ShapeAnnotation, ShapeList};
