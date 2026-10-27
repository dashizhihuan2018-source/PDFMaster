use super::*;
use std::path::PathBuf;
pub(super) fn read(paths: &[PathBuf]) -> Reply<Vec<Vec<u8>>> {
    let error = || {
        "仅接受普通本地文件，每份32MiB、整批64MiB；拒绝链接/文件夹或改变的文件。整批未导入。"
            .to_owned()
    };
    if paths.is_empty() || paths.len() > pdfmaster_core::MAX_INSERT_FILES {
        return Err(error());
    }
    let mut declared = 0u64;
    for path in paths {
        let m = std::fs::symlink_metadata(path).map_err(|_| error())?;
        if !m.is_file() || m.len() == 0 || m.len() > 32 * 1024 * 1024 {
            return Err(error());
        }
        declared = declared.checked_add(m.len()).ok_or_else(error)?;
        if declared > pdfmaster_core::MAX_INSERT_BYTES as u64 {
            return Err(error());
        }
    }
    let mut owned = vec![];
    let mut names = vec![];
    let mut actual = 0usize;
    for path in paths {
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
        }
        let mut file = options.open(path).map_err(|_| error())?;
        let before = file.metadata().map_err(|_| error())?;
        if !before.is_file() || before.len() > 32 * 1024 * 1024 {
            return Err(error());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if before.file_attributes() & 0x400 != 0 {
                return Err(error());
            }
        }
        let mut bytes = vec![];
        (&mut file)
            .take(32 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| error())?;
        let after = file.metadata().map_err(|_| error())?;
        if bytes.len() > 32 * 1024 * 1024
            || bytes.len() as u64 != before.len()
            || before.len() != after.len()
            || before.modified().ok() != after.modified().ok()
        {
            return Err(error());
        }
        actual = actual.checked_add(bytes.len()).ok_or_else(error)?;
        if actual > pdfmaster_core::MAX_INSERT_BYTES {
            return Err(error());
        }
        names.push(
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        );
        owned.push(bytes);
    }
    let refs: Vec<_> = names
        .iter()
        .zip(&owned)
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect();
    pdfmaster_core::prepare_imports(&refs).map_err(|e|format!("整批未导入，原文件未修改：{e}。当前支持PDF、PNG/JPEG/BMP、静态GIF及受支持的多页TIFF；HEIC/文字/Office及更广图片色彩模型仍待迁移。"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_mixed_batch_uses_owned_bytes_in_order_without_writing_sources() {
        let dir = tempfile::tempdir().unwrap();
        let pdf = Session::blank(100., 200., 1).unwrap().bytes().unwrap();
        let image = Session::blank(80., 40., 1)
            .unwrap()
            .page_clipboard(&[0])
            .unwrap()
            .png;
        let a = dir.path().join("a.pdf");
        let b = dir.path().join("b.png");
        std::fs::write(&a, &pdf).unwrap();
        std::fs::write(&b, &image).unwrap();
        let result = read(&[a.clone(), b.clone(), a.clone()]).unwrap();
        assert_eq!(result[0], pdf);
        assert_eq!(result[2], pdf);
        assert_eq!(
            Session::open(&result[1]).unwrap().info().pages[0]
                .width
                .round(),
            595.
        );
        assert_eq!(std::fs::read(a).unwrap(), pdf);
        assert_eq!(std::fs::read(b).unwrap(), image);
    }
    #[test]
    fn non_regular_symlink_and_late_bad_files_never_produce_a_partial_batch_or_private_paths() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("private-source-name.pdf");
        std::fs::write(&a, Session::blank(100., 100., 1).unwrap().bytes().unwrap()).unwrap();
        let bad = dir.path().join("private-late-name.png");
        std::fs::write(&bad, b"bad").unwrap();
        for paths in [
            vec![],
            vec![a.clone(); 33],
            vec![dir.path().to_path_buf()],
            vec![a.clone(), bad],
        ] {
            let error = read(&paths).unwrap_err();
            assert!(!error.contains("private-source-name") && !error.contains("private-late-name"));
        }
        #[cfg(unix)]
        {
            let link = dir.path().join("link.pdf");
            std::os::unix::fs::symlink(&a, &link).unwrap();
            assert!(read(&[link]).is_err());
        }
    }
}
