use super::*;
use std::path::PathBuf;
const FILE_BYTES: usize = 32 * 1024 * 1024;
const READ_CHUNK_BYTES: usize = 64 * 1024;

fn file_error() -> String {
    "仅接受普通本地文件，每份32MiB、整批64MiB；拒绝链接/文件夹或改变的文件。整批未导入。".to_owned()
}
fn cancellation_error() -> String {
    "图片导入已取消；整批未导入。".to_owned()
}
fn check_cancelled(cancelled: &dyn Fn() -> bool) -> Reply<()> {
    if cancelled() {
        Err(cancellation_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
pub(super) fn read_images(
    paths: &[PathBuf],
    layout: pdfmaster_core::ImageImportLayout,
) -> Reply<Vec<Vec<u8>>> {
    read_images_cancellable(paths, layout, || false)
}
pub(super) fn read_images_cancellable(
    paths: &[PathBuf],
    layout: pdfmaster_core::ImageImportLayout,
    cancelled: impl Fn() -> bool,
) -> Reply<Vec<Vec<u8>>> {
    read_images_observed(paths, layout, cancelled, |_| {})
}
pub(super) fn read_images_observed(
    paths: &[PathBuf],
    layout: pdfmaster_core::ImageImportLayout,
    cancelled: impl Fn() -> bool,
    mut progress: impl FnMut(pdfmaster_core::ImageImportProgress),
) -> Reply<Vec<Vec<u8>>> {
    read_with_layout(paths, Some(layout), &cancelled, &mut progress)
}
pub(super) fn read(paths: &[PathBuf]) -> Reply<Vec<Vec<u8>>> {
    read_with_layout(paths, None, &|| false, &mut |_| {})
}
fn read_file_cancellable(reader: &mut impl Read, cancelled: &dyn Fn() -> bool) -> Reply<Vec<u8>> {
    check_cancelled(cancelled)?;
    let mut bytes = Vec::new();
    let mut chunk = [0; READ_CHUNK_BYTES];
    loop {
        check_cancelled(cancelled)?;
        let remaining = (FILE_BYTES + 1).saturating_sub(bytes.len());
        if remaining == 0 {
            break;
        }
        let read = match reader.read(&mut chunk[..remaining.min(READ_CHUNK_BYTES)]) {
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                check_cancelled(cancelled)?;
                continue;
            }
            Err(_) => return Err(file_error()),
        };
        check_cancelled(cancelled)?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    Ok(bytes)
}
fn read_with_layout(
    paths: &[PathBuf],
    layout: Option<pdfmaster_core::ImageImportLayout>,
    cancelled: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(pdfmaster_core::ImageImportProgress),
) -> Reply<Vec<Vec<u8>>> {
    check_cancelled(cancelled)?;
    if paths.is_empty() || paths.len() > pdfmaster_core::MAX_INSERT_FILES {
        return Err(file_error());
    }
    let mut declared = 0u64;
    for path in paths {
        check_cancelled(cancelled)?;
        let m = std::fs::symlink_metadata(path).map_err(|_| file_error())?;
        check_cancelled(cancelled)?;
        if !m.is_file() || m.len() == 0 || m.len() > FILE_BYTES as u64 {
            return Err(file_error());
        }
        declared = declared.checked_add(m.len()).ok_or_else(file_error)?;
        if declared > pdfmaster_core::MAX_INSERT_BYTES as u64 {
            return Err(file_error());
        }
    }
    let mut owned = vec![];
    let mut names = vec![];
    let mut actual = 0usize;
    for path in paths {
        check_cancelled(cancelled)?;
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
        let mut file = options.open(path).map_err(|_| file_error())?;
        check_cancelled(cancelled)?;
        let before = file.metadata().map_err(|_| file_error())?;
        check_cancelled(cancelled)?;
        if !before.is_file() || before.len() > FILE_BYTES as u64 {
            return Err(file_error());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if before.file_attributes() & 0x400 != 0 {
                return Err(file_error());
            }
        }
        let bytes = read_file_cancellable(&mut file, cancelled)?;
        check_cancelled(cancelled)?;
        let after = file.metadata().map_err(|_| file_error())?;
        check_cancelled(cancelled)?;
        if bytes.len() > FILE_BYTES
            || bytes.len() as u64 != before.len()
            || before.len() != after.len()
            || before.modified().ok() != after.modified().ok()
        {
            return Err(file_error());
        }
        actual = actual.checked_add(bytes.len()).ok_or_else(file_error)?;
        if actual > pdfmaster_core::MAX_INSERT_BYTES {
            return Err(file_error());
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
    let converted = match layout {
        Some(layout) => {
            pdfmaster_core::prepare_image_imports_observed(&refs, layout, cancelled, progress)
        }
        None => pdfmaster_core::prepare_imports(&refs),
    };
    match converted {
        Err(pdfmaster_core::PdfMasterError::Cancelled) => Err(cancellation_error()),
        Err(error) => Err(format!(
            "整批未导入，原文件未修改：{error}。当前支持PDF、PNG/JPEG/BMP、静态GIF及受支持的多页TIFF；HEIC/文字/Office及更广图片色彩模型仍待迁移。"
        )),
        Ok(converted) => Ok(converted),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};

    struct CountingReader {
        bytes: Vec<u8>,
        offset: usize,
        reads: Rc<Cell<usize>>,
        largest_request: Rc<Cell<usize>>,
    }
    impl Read for CountingReader {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            self.reads.set(self.reads.get() + 1);
            self.largest_request
                .set(self.largest_request.get().max(output.len()));
            let count = output.len().min(self.bytes.len() - self.offset);
            output[..count].copy_from_slice(&self.bytes[self.offset..self.offset + count]);
            self.offset += count;
            Ok(count)
        }
    }

    struct InterruptingReader {
        bytes: Vec<u8>,
        offset: usize,
        interruptions_left: usize,
        reads: Rc<Cell<usize>>,
        interruptions: Rc<Cell<usize>>,
    }
    impl Read for InterruptingReader {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            self.reads.set(self.reads.get() + 1);
            if self.interruptions_left > 0 {
                self.interruptions_left -= 1;
                self.interruptions.set(self.interruptions.get() + 1);
                return Err(std::io::ErrorKind::Interrupted.into());
            }
            let count = output.len().min(self.bytes.len() - self.offset);
            output[..count].copy_from_slice(&self.bytes[self.offset..self.offset + count]);
            self.offset += count;
            Ok(count)
        }
    }

    struct ErrorReader {
        reads: Rc<Cell<usize>>,
    }
    impl Read for ErrorReader {
        fn read(&mut self, _output: &mut [u8]) -> std::io::Result<usize> {
            self.reads.set(self.reads.get() + 1);
            Err(std::io::ErrorKind::PermissionDenied.into())
        }
    }

    #[test]
    fn pre_cancelled_regular_image_has_no_output_and_controlled_reader_is_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("own.png");
        let source = Session::blank(80., 40., 1)
            .unwrap()
            .page_clipboard(&[0])
            .unwrap()
            .png;
        std::fs::write(&path, &source).unwrap();

        let error = read_images_cancellable(
            std::slice::from_ref(&path),
            pdfmaster_core::ImageImportLayout::default(),
            || true,
        )
        .unwrap_err();
        assert!(error.contains("取消") && !error.contains("own.png"));
        assert_eq!(std::fs::read(path).unwrap(), source);

        let reads = Rc::new(Cell::new(0));
        let largest_request = Rc::new(Cell::new(0));
        let mut reader = CountingReader {
            bytes: vec![7; 3 * 64 * 1024],
            offset: 0,
            reads: reads.clone(),
            largest_request,
        };
        assert!(read_file_cancellable(&mut reader, &|| true).is_err());
        assert_eq!(reads.get(), 0, "pre-cancellation must happen before Read");
    }

    #[test]
    fn controlled_reader_stops_after_the_first_bounded_chunk() {
        let reads = Rc::new(Cell::new(0));
        let largest_request = Rc::new(Cell::new(0));
        let mut reader = CountingReader {
            bytes: vec![11; 3 * 64 * 1024],
            offset: 0,
            reads: reads.clone(),
            largest_request: largest_request.clone(),
        };

        let error = read_file_cancellable(&mut reader, &|| reads.get() >= 1).unwrap_err();

        assert!(error.contains("取消"));
        assert_eq!(
            reads.get(),
            1,
            "no later chunk may be read after cancellation"
        );
        assert!(largest_request.get() <= 64 * 1024);
        assert_eq!(reader.offset, 64 * 1024);
    }

    #[test]
    fn controlled_reader_retries_one_or_multiple_interruptions() {
        for interruption_count in [1, 3] {
            let reads = Rc::new(Cell::new(0));
            let interruptions = Rc::new(Cell::new(0));
            let payload = vec![3, 1, 4, 1, 5];
            let mut reader = InterruptingReader {
                bytes: payload.clone(),
                offset: 0,
                interruptions_left: interruption_count,
                reads,
                interruptions: interruptions.clone(),
            };

            let output = read_file_cancellable(&mut reader, &|| false).unwrap();

            assert_eq!(output, payload);
            assert_eq!(interruptions.get(), interruption_count);
        }
    }

    #[test]
    fn cancellation_after_interrupted_read_prevents_another_read_attempt() {
        let reads = Rc::new(Cell::new(0));
        let interruptions = Rc::new(Cell::new(0));
        let mut reader = InterruptingReader {
            bytes: vec![9, 2, 6],
            offset: 0,
            interruptions_left: 1,
            reads: reads.clone(),
            interruptions: interruptions.clone(),
        };

        let result = read_file_cancellable(&mut reader, &|| interruptions.get() > 0);
        let error = match result {
            Err(error) => error,
            Ok(bytes) => panic!("expected cancellation, got {} bytes", bytes.len()),
        };

        assert!(error.contains("取消"));
        assert_eq!(
            reads.get(),
            1,
            "Interrupted must not be retried after cancellation"
        );
    }

    #[test]
    fn non_interrupted_read_errors_remain_terminal() {
        let reads = Rc::new(Cell::new(0));
        let mut reader = ErrorReader {
            reads: reads.clone(),
        };

        let result = read_file_cancellable(&mut reader, &|| false);
        let error = match result {
            Err(error) => error,
            Ok(bytes) => panic!("expected read failure, got {} bytes", bytes.len()),
        };

        assert!(error.contains("整批未导入"));
        assert_eq!(reads.get(), 1);
    }

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

    #[test]
    fn observed_image_reads_report_core_events_and_preserve_sources() {
        let dir = tempfile::tempdir().unwrap();
        let source = Session::blank(80., 40., 1)
            .unwrap()
            .page_clipboard(&[0])
            .unwrap()
            .png;
        let first = dir.path().join("first.png");
        let second = dir.path().join("second.png");
        std::fs::write(&first, &source).unwrap();
        std::fs::write(&second, &source).unwrap();
        let paths = [first.clone(), second.clone()];
        let layout = pdfmaster_core::ImageImportLayout::default();
        let mut seen = Vec::new();

        let observed =
            read_images_observed(&paths, layout, || false, |progress| seen.push(progress)).unwrap();

        assert_eq!(
            observed,
            read_images_cancellable(&paths, layout, || false).unwrap()
        );
        assert_eq!(
            seen.iter()
                .map(|progress| (progress.current, progress.total))
                .collect::<Vec<_>>(),
            [(1, 2), (2, 2)]
        );
        assert_eq!(std::fs::read(first).unwrap(), source);
        assert_eq!(std::fs::read(second).unwrap(), source);
    }

    #[test]
    fn observed_image_reads_emit_no_events_for_preflight_failures_or_pre_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        let source = Session::blank(80., 40., 1)
            .unwrap()
            .page_clipboard(&[0])
            .unwrap()
            .png;
        let good = dir.path().join("good.png");
        let invalid = dir.path().join("invalid.png");
        let missing = dir.path().join("missing.png");
        std::fs::write(&good, &source).unwrap();
        std::fs::write(&invalid, b"not an image").unwrap();
        let layout = pdfmaster_core::ImageImportLayout::default();

        for paths in [
            vec![good.clone(), invalid.clone()],
            vec![good.clone(), missing],
        ] {
            let mut seen = Vec::new();
            let result =
                read_images_observed(&paths, layout, || false, |progress| seen.push(progress));
            assert!(
                result.is_err(),
                "invalid native batch unexpectedly succeeded"
            );
            assert!(seen.is_empty(), "preflight emitted {seen:?}");
        }

        let mut seen = Vec::new();
        let result = read_images_observed(
            std::slice::from_ref(&good),
            layout,
            || true,
            |progress| seen.push(progress),
        );
        assert_eq!(result.unwrap_err(), cancellation_error());
        assert!(seen.is_empty());
        assert_eq!(std::fs::read(good).unwrap(), source);
    }

    #[test]
    fn observed_image_read_callback_cancellation_is_atomic_and_preserves_sources() {
        let dir = tempfile::tempdir().unwrap();
        let source = Session::blank(80., 40., 1)
            .unwrap()
            .page_clipboard(&[0])
            .unwrap()
            .png;
        let first = dir.path().join("first.png");
        let second = dir.path().join("second.png");
        std::fs::write(&first, &source).unwrap();
        std::fs::write(&second, &source).unwrap();
        let stop = Cell::new(false);
        let mut seen = Vec::new();

        let result = read_images_observed(
            &[first.clone(), second.clone()],
            pdfmaster_core::ImageImportLayout::default(),
            || stop.get(),
            |progress| {
                seen.push(progress);
                stop.set(true);
            },
        );

        assert_eq!(result.unwrap_err(), cancellation_error());
        assert_eq!(
            seen.iter()
                .map(|progress| (progress.current, progress.total))
                .collect::<Vec<_>>(),
            [(1, 2)]
        );
        assert_eq!(std::fs::read(first).unwrap(), source);
        assert_eq!(std::fs::read(second).unwrap(), source);
    }
}
