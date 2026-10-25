use std::path::{Path, PathBuf};

/// Holds the complete export payload alive until the native picker completes.
pub struct PreparedExport {
    _directory: tempfile::TempDir,
    pub path: PathBuf,
}

impl PreparedExport {
    pub fn new(cache: &Path, name: &str, bytes: &[u8]) -> Result<Self, String> {
        if bytes.is_empty() || bytes.len() > 32 * 1024 * 1024 {
            return Err("导出内容为空或超过预算".into());
        }
        let directory = tempfile::Builder::new()
            .prefix("pdfmaster-export-")
            .tempdir_in(cache)
            .map_err(|e| e.to_string())?;
        let name = Path::new(name)
            .file_name()
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| std::ffi::OsStr::new("PDF-Master-副本.pdf"));
        let path = directory.path().join(name);
        crate::storage::export_new(&path, bytes).map_err(|e| e.to_string())?;
        Ok(Self {
            _directory: directory,
            path,
        })
    }
}

pub fn verify_completion(
    completed: bool,
    readback: Option<&[u8]>,
    expected: &[u8],
) -> Result<bool, String> {
    if !completed {
        return Ok(false);
    }
    if readback.is_some_and(|bytes| bytes == expected) {
        Ok(true)
    } else {
        Err("系统已结束导出，但无法回读验证同一份 PDF。文档仍保留未保存状态。".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_pdf_exists_before_picker_and_is_confined() {
        let cache = tempfile::tempdir().unwrap();
        let staged =
            PreparedExport::new(cache.path(), "../../source.pdf", b"synthetic PDF").unwrap();
        assert!(staged.path.starts_with(cache.path()));
        assert_eq!(staged.path.file_name().unwrap(), "source.pdf");
        assert_eq!(std::fs::read(&staged.path).unwrap(), b"synthetic PDF");
    }
    #[test]
    fn cancelling_never_reports_a_verified_export() {
        assert!(!verify_completion(false, None, b"expected").unwrap());
    }
    #[test]
    fn empty_or_different_provider_result_does_not_clear_dirty() {
        assert!(verify_completion(true, Some(b""), b"expected").is_err());
        assert!(verify_completion(true, None, b"expected").is_err());
        assert!(verify_completion(true, Some(b"wrong"), b"expected").is_err());
        assert!(verify_completion(true, Some(b"expected"), b"expected").unwrap());
    }
}
