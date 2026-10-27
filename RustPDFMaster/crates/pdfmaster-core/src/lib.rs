//! Safe, transactional PDF primitives shared by every PDF Master shell.

mod annotations;
mod clipboard;
mod document;
mod edit;
mod extract;
mod image_export;
mod import;
mod import_color;
mod import_exif;
mod import_jpeg;
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
pub use clipboard::PageClipboard;
pub use document::{
    Color, DocumentInfo, Edit, PageInfo, PdfMasterError, Rect, Result, Session, ShapeKind,
    TextReader,
};
pub use image_export::{
    ExportedImage, ImageExportFormat, ImageExportLimits, ImageExportRequest, ImageExportSummary,
};
pub use import::{ImageImportLayout, ImagePaper, prepare_image_imports, prepare_imports};
pub use insert::{MAX_INSERT_BYTES, MAX_INSERT_FILES};
pub use lines::{LineEnding, LineGeometry, Point};
pub use notes::{NoteList, TextNote};
pub use outline::OutlineItem;
pub use ranges::{PageRange, PageRangeError};
pub use shapes::{ShapeAnnotation, ShapeList};
