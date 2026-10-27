mod control;
mod platform;
#[cfg(test)]
mod tests;

pub(crate) use control::ImageExportControl;

use control::PublicationGateError;
use pdfmaster_core::{
    ExportedImage, ImageExportRequest, ImageExportSummary, PdfMasterError, Session,
};
use std::{
    cell::RefCell,
    collections::BTreeSet,
    ffi::{OsStr, OsString},
    fs,
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
};

const COPY_CHUNK: usize = 64 * 1024;

#[derive(Clone)]
struct ExpectedFile {
    name: OsString,
    len: usize,
}

pub(crate) struct PreparedImageExport {
    parent: platform::ParentDirectory,
    _root: tempfile::TempDir,
    payload: PathBuf,
    folder_name: OsString,
    summary: ImageExportSummary,
    files: Vec<ExpectedFile>,
    control: ImageExportControl,
}

pub(crate) struct DisposableImageDirectory {
    _root: tempfile::TempDir,
    payload: PathBuf,
}

pub(crate) struct ImageExportPublication {
    pub pages: usize,
    pub total_bytes: usize,
    pub durability_warning: Option<String>,
}

impl PreparedImageExport {
    pub(crate) fn render_in(
        parent: &Path,
        folder_name: &OsStr,
        session: &Session,
        request: &ImageExportRequest,
        control: &ImageExportControl,
    ) -> Result<Self, String> {
        validate_basename(folder_name)?;
        ensure_active(control)?;
        let parent = platform::ParentDirectory::capture(parent)
            .map_err(|error| storage_error(error.kind()))?;
        ensure_active(control)?;
        let root = tempfile::Builder::new()
            .prefix(".pdfmaster-images-")
            .tempdir_in(parent.canonical())
            .map_err(|error| storage_error(error.kind()))?;
        set_private_root_mode(root.path())?;
        let payload = root.path().join("payload");
        platform::create_private_directory(&payload)
            .map_err(|error| storage_error(error.kind()))?;
        ensure_active(control)?;

        let files = RefCell::new(Vec::new());
        let sink_failure = RefCell::new(None);
        let export_result = session.export_images(
            request,
            || control.is_cancelled(),
            |image| match write_emitted_image(&payload, image, control) {
                Ok(file) => {
                    files.borrow_mut().push(file);
                    Ok(())
                }
                Err(error) => {
                    *sink_failure.borrow_mut() = Some(error);
                    Err(PdfMasterError::Render("image staging sink failed".into()))
                }
            },
        );
        if let Some(error) = sink_failure.into_inner() {
            return Err(error);
        }
        let summary = match export_result {
            Ok(summary) => summary,
            Err(PdfMasterError::Cancelled) => return Err(cancelled_error()),
            Err(_) => return Err("图片导出失败；未创建公开目标。".into()),
        };
        let files = files.into_inner();
        let tracked_bytes = files.iter().try_fold(0_usize, |total, file| {
            total
                .checked_add(file.len)
                .ok_or_else(|| "图片导出文件大小超出限制；未创建公开目标。".to_string())
        })?;
        if summary.pages != files.len() || summary.total_bytes != tracked_bytes {
            return Err("图片导出暂存不完整；未创建公开目标。".into());
        }
        verify_directory(&payload, &payload, &files, control)?;
        ensure_active(control)?;
        Ok(Self {
            parent,
            _root: root,
            payload,
            folder_name: folder_name.to_os_string(),
            summary,
            files,
            control: control.clone(),
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.payload
    }

    pub(crate) fn summary(&self) -> ImageExportSummary {
        self.summary
    }

    pub(crate) fn verify_copy(&self, actual: &Path) -> Result<(), String> {
        verify_directory(&self.payload, actual, &self.files, &self.control)
    }

    pub(crate) fn disposable_copy_in(
        &self,
        cache: &Path,
    ) -> Result<DisposableImageDirectory, String> {
        ensure_active(&self.control)?;
        let cache = platform::ParentDirectory::capture(cache)
            .map_err(|error| storage_error(error.kind()))?;
        let root = tempfile::Builder::new()
            .prefix(".pdfmaster-images-copy-")
            .tempdir_in(cache.canonical())
            .map_err(|error| storage_error(error.kind()))?;
        set_private_root_mode(root.path())?;
        let payload = root.path().join("payload");
        platform::create_private_directory(&payload)
            .map_err(|error| storage_error(error.kind()))?;
        for expected in &self.files {
            ensure_active(&self.control)?;
            copy_exact_file(
                &self.payload.join(&expected.name),
                &payload.join(&expected.name),
                expected.len,
                &self.control,
            )?;
        }
        verify_directory(&self.payload, &payload, &self.files, &self.control)?;
        Ok(DisposableImageDirectory {
            _root: root,
            payload,
        })
    }

    pub(crate) fn publish_new(self, target: &Path) -> Result<ImageExportPublication, String> {
        self.publish_new_with_syncs(target, platform::sync_directory, |parent| parent.sync())
    }

    fn publish_new_with_sync<F>(
        self,
        target: &Path,
        sync: F,
    ) -> Result<ImageExportPublication, String>
    where
        F: FnOnce(&platform::ParentDirectory) -> io::Result<()>,
    {
        self.publish_new_with_syncs(target, platform::sync_directory, sync)
    }

    fn publish_new_with_syncs<P, F>(
        self,
        target: &Path,
        payload_sync: P,
        sync: F,
    ) -> Result<ImageExportPublication, String>
    where
        P: FnOnce(&Path) -> io::Result<()>,
        F: FnOnce(&platform::ParentDirectory) -> io::Result<()>,
    {
        let target_name = target
            .file_name()
            .ok_or_else(|| "图片导出目标无效；未创建公开目标。".to_string())?;
        validate_basename(target_name)?;
        if target_name != self.folder_name {
            return Err("图片导出目标与暂存请求不匹配；未创建公开目标。".into());
        }
        let target_parent = target
            .parent()
            .ok_or_else(|| "图片导出目标无效；未创建公开目标。".to_string())?;
        if !self
            .parent
            .matches(target_parent)
            .map_err(|error| storage_error(error.kind()))?
        {
            return Err("图片导出目标与暂存请求不匹配；未创建公开目标。".into());
        }
        self.verify_copy(&self.payload)?;
        payload_sync(&self.payload).map_err(|error| storage_error(error.kind()))?;
        let summary = self.summary;
        let publish = self.control.publish(|| {
            self.parent
                .rename_exclusive(&self.payload, &self.folder_name)
        });
        match publish {
            Ok(()) => {}
            Err(PublicationGateError::Cancelled) => return Err(cancelled_error()),
            Err(PublicationGateError::Io(error)) => return Err(storage_error(error.kind())),
        }
        let durability_warning = sync(&self.parent).err().map(|error| {
            format!(
                "图片目录已完整发布，但父目录持久化确认失败（{:?}）。",
                error.kind()
            )
        });
        Ok(ImageExportPublication {
            pages: summary.pages,
            total_bytes: summary.total_bytes,
            durability_warning,
        })
    }
}

impl DisposableImageDirectory {
    pub(crate) fn path(&self) -> &Path {
        &self.payload
    }
}

fn write_emitted_image(
    payload: &Path,
    image: ExportedImage,
    control: &ImageExportControl,
) -> Result<ExpectedFile, String> {
    validate_basename(OsStr::new(&image.filename))?;
    ensure_active(control)?;
    let path = payload.join(&image.filename);
    let mut file =
        platform::create_private_file(&path).map_err(|error| storage_error(error.kind()))?;
    for chunk in image.bytes.chunks(COPY_CHUNK) {
        ensure_active(control)?;
        file.write_all(chunk)
            .map_err(|error| storage_error(error.kind()))?;
    }
    file.flush()
        .and_then(|()| file.sync_all())
        .map_err(|error| storage_error(error.kind()))?;
    ensure_active(control)?;
    drop(file);

    let mut readback =
        platform::open_regular_nofollow(&path).map_err(|error| storage_error(error.kind()))?;
    compare_reader_to_bytes(&mut readback, &image.bytes, control)?;
    ensure_active(control)?;
    Ok(ExpectedFile {
        name: OsString::from(image.filename),
        len: image.bytes.len(),
    })
}

fn verify_directory(
    pristine: &Path,
    actual: &Path,
    expected: &[ExpectedFile],
    control: &ImageExportControl,
) -> Result<(), String> {
    ensure_active(control)?;
    if !platform::is_plain_directory(actual).map_err(|error| storage_error(error.kind()))? {
        return Err("图片导出副本不是普通目录。".into());
    }
    let expected_names = expected
        .iter()
        .map(|file| file.name.clone())
        .collect::<BTreeSet<_>>();
    let mut actual_names = BTreeSet::new();
    let entries = fs::read_dir(actual).map_err(|error| storage_error(error.kind()))?;
    for entry in entries {
        ensure_active(control)?;
        if actual_names.len() >= expected.len() {
            return Err("图片导出副本包含额外项目。".into());
        }
        let entry = entry.map_err(|error| storage_error(error.kind()))?;
        let file_type = entry
            .file_type()
            .map_err(|error| storage_error(error.kind()))?;
        if !file_type.is_file() || file_type.is_symlink() {
            return Err("图片导出副本包含非普通文件。".into());
        }
        let name = entry.file_name();
        if !expected_names.contains(&name) || !actual_names.insert(name) {
            return Err("图片导出副本文件集合不匹配。".into());
        }
    }
    if actual_names != expected_names {
        return Err("图片导出副本文件集合不匹配。".into());
    }

    for file in expected {
        ensure_active(control)?;
        let mut source = platform::open_regular_nofollow(&pristine.join(&file.name))
            .map_err(|error| storage_error(error.kind()))?;
        let mut copy = platform::open_regular_nofollow(&actual.join(&file.name))
            .map_err(|error| storage_error(error.kind()))?;
        if source
            .metadata()
            .map_err(|error| storage_error(error.kind()))?
            .len()
            != file.len as u64
            || copy
                .metadata()
                .map_err(|error| storage_error(error.kind()))?
                .len()
                != file.len as u64
        {
            return Err("图片导出副本文件长度不匹配。".into());
        }
        compare_readers_exact(&mut source, &mut copy, file.len, control)?;
    }
    Ok(())
}

fn copy_exact_file(
    source: &Path,
    destination: &Path,
    expected_len: usize,
    control: &ImageExportControl,
) -> Result<(), String> {
    let mut source =
        platform::open_regular_nofollow(source).map_err(|error| storage_error(error.kind()))?;
    if source
        .metadata()
        .map_err(|error| storage_error(error.kind()))?
        .len()
        != expected_len as u64
    {
        return Err("图片导出暂存文件长度不匹配。".into());
    }
    let mut destination =
        platform::create_private_file(destination).map_err(|error| storage_error(error.kind()))?;
    let mut remaining = expected_len;
    let mut buffer = [0_u8; COPY_CHUNK];
    while remaining > 0 {
        ensure_active(control)?;
        let wanted = remaining.min(buffer.len());
        read_fully(&mut source, &mut buffer[..wanted], control)?;
        destination
            .write_all(&buffer[..wanted])
            .map_err(|error| storage_error(error.kind()))?;
        remaining -= wanted;
    }
    let mut extra = [0_u8; 1];
    if source
        .read(&mut extra)
        .map_err(|error| storage_error(error.kind()))?
        != 0
    {
        return Err("图片导出暂存文件长度不匹配。".into());
    }
    destination
        .flush()
        .and_then(|()| destination.sync_all())
        .map_err(|error| storage_error(error.kind()))?;
    ensure_active(control)
}

fn compare_reader_to_bytes(
    reader: &mut fs::File,
    expected: &[u8],
    control: &ImageExportControl,
) -> Result<(), String> {
    let mut offset = 0;
    let mut buffer = [0_u8; COPY_CHUNK];
    while offset < expected.len() {
        ensure_active(control)?;
        let wanted = (expected.len() - offset).min(buffer.len());
        read_fully(reader, &mut buffer[..wanted], control)?;
        if buffer[..wanted] != expected[offset..offset + wanted] {
            return Err("图片导出暂存文件回读不一致。".into());
        }
        offset += wanted;
    }
    let mut extra = [0_u8; 1];
    if reader
        .read(&mut extra)
        .map_err(|error| storage_error(error.kind()))?
        != 0
    {
        return Err("图片导出暂存文件长度不匹配。".into());
    }
    Ok(())
}

fn compare_readers_exact(
    left: &mut fs::File,
    right: &mut fs::File,
    len: usize,
    control: &ImageExportControl,
) -> Result<(), String> {
    let mut left_buffer = [0_u8; COPY_CHUNK];
    let mut right_buffer = [0_u8; COPY_CHUNK];
    let mut remaining = len;
    while remaining > 0 {
        ensure_active(control)?;
        let wanted = remaining.min(COPY_CHUNK);
        read_fully(left, &mut left_buffer[..wanted], control)?;
        read_fully(right, &mut right_buffer[..wanted], control)?;
        if left_buffer[..wanted] != right_buffer[..wanted] {
            return Err("图片导出副本内容不匹配。".into());
        }
        remaining -= wanted;
    }
    let mut left_extra = [0_u8; 1];
    let mut right_extra = [0_u8; 1];
    if left
        .read(&mut left_extra)
        .map_err(|error| storage_error(error.kind()))?
        != 0
        || right
            .read(&mut right_extra)
            .map_err(|error| storage_error(error.kind()))?
            != 0
    {
        return Err("图片导出副本文件长度不匹配。".into());
    }
    Ok(())
}

fn read_fully(
    reader: &mut fs::File,
    mut output: &mut [u8],
    control: &ImageExportControl,
) -> Result<(), String> {
    while !output.is_empty() {
        ensure_active(control)?;
        let read = reader
            .read(output)
            .map_err(|error| storage_error(error.kind()))?;
        if read == 0 {
            return Err("图片导出文件提前结束。".into());
        }
        output = &mut output[read..];
    }
    Ok(())
}

fn validate_basename(name: &OsStr) -> Result<(), String> {
    let mut components = Path::new(name).components();
    if !matches!(components.next(), Some(Component::Normal(component)) if component == name)
        || components.next().is_some()
    {
        return Err("图片导出文件夹名称无效；未创建公开目标。".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let bytes = name.as_bytes();
        if bytes.is_empty()
            || bytes.len() > 255
            || bytes
                .iter()
                .any(|byte| *byte == 0 || *byte < 0x20 || *byte == 0x7f)
        {
            return Err("图片导出文件夹名称无效；未创建公开目标。".into());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let units = name.encode_wide().collect::<Vec<_>>();
        if units.is_empty()
            || units.len() > 255
            || units
                .iter()
                .any(|unit| *unit == 0 || *unit < 0x20 || *unit == 0x7f)
        {
            return Err("图片导出文件夹名称无效；未创建公开目标。".into());
        }
    }
    if name
        .to_str()
        .is_some_and(|text| text.chars().any(char::is_control))
    {
        return Err("图片导出文件夹名称无效；未创建公开目标。".into());
    }
    #[cfg(not(any(unix, windows)))]
    if name.to_string_lossy().chars().any(char::is_control) {
        return Err("图片导出文件夹名称无效；未创建公开目标。".into());
    }
    Ok(())
}

fn set_private_root_mode(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| storage_error(error.kind()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn ensure_active(control: &ImageExportControl) -> Result<(), String> {
    if control.is_cancelled() {
        Err(cancelled_error())
    } else if control.is_published() {
        Err("图片目录已经发布。".into())
    } else {
        Ok(())
    }
}

fn cancelled_error() -> String {
    "图片导出已取消；未创建公开目标。".into()
}

fn storage_error(kind: io::ErrorKind) -> String {
    format!("图片导出文件操作失败（{kind:?}）；未覆盖已有目标。")
}
