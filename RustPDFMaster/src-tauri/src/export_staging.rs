use std::path::Path;
#[cfg(any(target_os = "ios", test))]
use std::path::PathBuf;
use std::{fs::File, io::Read};

/// Only called for a native picker/coordinator URL; never takes a web path.
pub fn readback_pdf(path: &Path) -> Result<Vec<u8>, String> {
    // Windows refuses File::open on a directory before handle metadata can be
    // inspected. Classify it consistently, then recheck the opened handle to
    // avoid trusting only this path-based preflight.
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("无法检查导出回读文件（{:?}）", error.kind()))?;
    if !metadata.is_file() {
        return Err("导出提供器返回的不是普通文件；未保存状态仍保留。".into());
    }
    let file =
        File::open(path).map_err(|error| format!("无法打开导出回读文件（{:?}）", error.kind()))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("无法检查导出回读文件（{:?}）", error.kind()))?;
    if !metadata.is_file() {
        return Err("导出提供器返回的不是普通文件；未保存状态仍保留。".into());
    }
    if metadata.len() > 32 * 1024 * 1024 {
        return Err("导出回读超过32MiB预算".into());
    }
    let mut bytes = Vec::new();
    file.take(32 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("回读导出文件失败（{:?}）", error.kind()))?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err("导出回读超过32MiB预算".into());
    }
    Ok(bytes)
}

/// Holds the complete export payload alive until the native picker completes.
#[cfg(any(target_os = "ios", test))]
pub struct PreparedExport {
    _directory: tempfile::TempDir,
    pub path: PathBuf,
}

#[cfg(any(target_os = "ios", test))]
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

#[cfg(any(target_os = "ios", test))]
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
    #[test]
    fn provider_readback_requires_a_regular_file_without_leaking_paths() {
        let directory = tempfile::tempdir().unwrap();
        let error = readback_pdf(directory.path()).unwrap_err();
        assert!(error.contains("普通文件"));
        assert!(!error.contains(directory.path().to_str().unwrap()));
        let absent = directory.path().join("private-fixture-name.pdf");
        let error = readback_pdf(&absent).unwrap_err();
        assert!(error.contains("NotFound"));
        assert!(!error.contains("private-fixture-name"));
        let staged = PreparedExport::new(directory.path(), "test.pdf", b"synthetic PDF").unwrap();
        assert_eq!(readback_pdf(&staged.path).unwrap(), b"synthetic PDF");
    }
    #[test]
    fn provider_readback_checks_the_size_budget_before_allocating() {
        let directory = tempfile::tempdir().unwrap();
        let staged = PreparedExport::new(directory.path(), "test.pdf", b"synthetic PDF").unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&staged.path)
            .unwrap()
            .set_len(32 * 1024 * 1024 + 1)
            .unwrap();
        assert!(readback_pdf(&staged.path).unwrap_err().contains("预算"));
    }
}
