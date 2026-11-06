//! Safe, transactional PDF primitives shared by every PDF Master shell.

mod annotations;
mod canvas_shapes;
mod clipboard;
mod document;
mod edit;
mod extract;
mod free_text;
mod free_text_appearance;
mod free_text_edit;
mod free_text_edit_style;
mod free_text_preview;
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
mod shape_opacity;
mod shapes;
pub mod text_appearance;
mod text_export;
mod text_fonts;
mod text_layout;

pub use annotations::AnnotationRow;
pub use canvas_shapes::{
    CanvasShapeCommit, CanvasShapeDraft, CanvasShapeGeometry, CanvasShapeList, CanvasShapeSelection,
};
pub use clipboard::PageClipboard;
pub use document::{
    Color, DocumentInfo, Edit, PageInfo, PdfMasterError, Rect, Result, Session, ShapeKind,
    TextReader,
};
pub use free_text::{FreeTextAnnotation, FreeTextList, FreeTextStyle, TextAlignment};
pub use free_text_edit::{FreeTextAppearanceStyle, FreeTextCommit, FreeTextDraft, FreeTextUpdate};
pub use image_export::{
    ExportedImage, ImageExportFormat, ImageExportLimits, ImageExportRequest, ImageExportSummary,
};
pub use import::{
    ImageImportLayout, ImageImportProgress, ImagePaper, prepare_image_imports,
    prepare_image_imports_cancellable, prepare_image_imports_observed, prepare_imports,
};
pub use insert::{MAX_INSERT_BYTES, MAX_INSERT_FILES};
pub use lines::{LineEnding, LineGeometry, Point};
pub use notes::{NoteList, TextNote};
pub use outline::OutlineItem;
pub use ranges::{PageRange, PageRangeError};
pub use shapes::{ShapeAnnotation, ShapeList};
pub use text_export::TextExportFormat;
