use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);
struct StagedFile(PathBuf);
impl Drop for StagedFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[cfg(unix)]
fn sync_parent(parent: &Path) -> io::Result<()> {
    fs::File::open(parent)?.sync_all()
}

#[cfg(unix)]
fn publish_unix<F>(temp: &Path, target: &Path, parent: &Path, sync: F) -> io::Result<()>
where
    F: FnOnce(&Path) -> io::Result<()>,
{
    fs::hard_link(temp, target).map_err(|e| {
        io::Error::new(e.kind(), format!("不能创建新副本（不会覆盖已有文件）：{e}"))
    })?;
    sync(parent)
}

#[cfg(windows)]
pub(crate) fn move_file_write_through(
    source: &Path,
    destination: &Path,
    replace: bool,
) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let flags = MOVEFILE_WRITE_THROUGH
        | if replace {
            MOVEFILE_REPLACE_EXISTING
        } else {
            0
        };
    // SAFETY: both UTF-16 buffers are NUL terminated and remain alive for the
    // complete Win32 call. MoveFileExW does not retain either pointer.
    let moved = unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), flags) };
    if moved == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Commits a complete new PDF without replacing an existing destination.
/// Unix uses a same-directory hard link plus directory sync; Windows uses a
/// same-volume write-through move without `MOVEFILE_REPLACE_EXISTING`.
pub fn export_new(destination: &Path, bytes: &[u8]) -> io::Result<()> {
    if !destination
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "目标必须是新的 .pdf 文件",
        ));
    }
    let parent = destination
        .parent()
        .ok_or_else(|| io::Error::other("没有目标目录"))?
        .canonicalize()?;
    let filename = destination
        .file_name()
        .ok_or_else(|| io::Error::other("没有目标文件名"))?;
    let target = parent.join(filename);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    let temp = parent.join(format!(
        ".pdfmaster-{stamp}-{}-{}.tmp",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp)?;
    let _cleanup = StagedFile(temp.clone());
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    #[cfg(unix)]
    publish_unix(&temp, &target, &parent, sync_parent)?;
    #[cfg(windows)]
    move_file_write_through(&temp, &target, false).map_err(|e| {
        io::Error::new(e.kind(), format!("不能创建新副本（不会覆盖已有文件）：{e}"))
    })?;
    #[cfg(not(any(unix, windows)))]
    fs::hard_link(&temp, &target).map_err(|e| {
        io::Error::new(e.kind(), format!("不能创建新副本（不会覆盖已有文件）：{e}"))
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "pdfmaster-storage-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    #[test]
    fn export_creates_new_unicode_file_with_exact_bytes() {
        let t = Temp::new();
        let p = t.0.join("批注副本.pdf");
        export_new(&p, b"%PDF-test").unwrap();
        assert_eq!(fs::read(p).unwrap(), b"%PDF-test");
        assert_eq!(fs::read_dir(&t.0).unwrap().count(), 1);
    }
    #[test]
    fn existing_file_is_never_overwritten() {
        let t = Temp::new();
        let p = t.0.join("source.pdf");
        fs::write(&p, b"keep").unwrap();
        assert!(export_new(&p, b"replace").is_err());
        assert_eq!(fs::read(p).unwrap(), b"keep");
        assert_eq!(fs::read_dir(&t.0).unwrap().count(), 1);
    }
    #[test]
    fn missing_parent_does_not_create_or_write_anything() {
        let t = Temp::new();
        assert!(export_new(&t.0.join("absent/copy.pdf"), b"x").is_err());
        assert_eq!(fs::read_dir(&t.0).unwrap().count(), 0);
    }
    #[test]
    fn non_pdf_destination_rejected() {
        let t = Temp::new();
        assert!(export_new(&t.0.join("file.txt"), b"x").is_err());
        assert_eq!(fs::read_dir(&t.0).unwrap().count(), 0);
    }
    #[cfg(unix)]
    #[test]
    fn symlink_destination_cannot_overwrite_target() {
        let t = Temp::new();
        let p = t.0.join("source.pdf");
        fs::write(&p, b"private").unwrap();
        let link = t.0.join("alias.pdf");
        std::os::unix::fs::symlink(&p, &link).unwrap();
        assert!(export_new(&link, b"replace").is_err());
        assert_eq!(fs::read(p).unwrap(), b"private");
        assert_eq!(fs::read_dir(&t.0).unwrap().count(), 2);
    }
    #[cfg(unix)]
    #[test]
    fn directory_sync_failure_is_reported_after_no_clobber_publish() {
        let t = Temp::new();
        let staged = t.0.join("staged.tmp");
        let target = t.0.join("copy.pdf");
        fs::write(&staged, b"complete pdf").unwrap();
        let error = publish_unix(&staged, &target, &t.0, |_| {
            Err(io::Error::other("injected directory sync failure"))
        })
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("injected directory sync failure")
        );
        assert_eq!(fs::read(target).unwrap(), b"complete pdf");
    }
}
