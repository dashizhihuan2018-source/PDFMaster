use std::{
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
};

pub(super) struct ParentDirectory {
    canonical: PathBuf,
    #[cfg(any(unix, windows))]
    handle: fs::File,
}

impl ParentDirectory {
    pub(super) fn capture(path: &Path) -> io::Result<Self> {
        let canonical = path.canonicalize()?;
        let metadata = fs::metadata(&canonical)?;
        if !metadata.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "parent is not a directory",
            ));
        }
        #[cfg(unix)]
        let handle = fs::File::open(&canonical)?;
        #[cfg(windows)]
        let handle = open_windows_directory(&canonical)?;
        Ok(Self {
            canonical,
            #[cfg(any(unix, windows))]
            handle,
        })
    }

    pub(super) fn canonical(&self) -> &Path {
        &self.canonical
    }

    pub(super) fn matches(&self, candidate: &Path) -> io::Result<bool> {
        let canonical = candidate.canonicalize()?;
        if canonical != self.canonical {
            return Ok(false);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let retained = self.handle.metadata()?;
            let current = fs::File::open(canonical)?.metadata()?;
            Ok(retained.dev() == current.dev() && retained.ino() == current.ino())
        }
        #[cfg(windows)]
        {
            let current = open_windows_directory(&canonical)?;
            Ok(windows_file_identity(&self.handle)? == windows_file_identity(&current)?)
        }
        #[cfg(not(any(unix, windows)))]
        {
            Ok(true)
        }
    }

    pub(super) fn sync(&self) -> io::Result<()> {
        #[cfg(unix)]
        {
            self.handle.sync_all()
        }
        #[cfg(not(unix))]
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "directory sync is unsupported",
            ))
        }
    }

    pub(super) fn rename_exclusive(&self, source: &Path, target_name: &OsStr) -> io::Result<()> {
        #[cfg(unix)]
        {
            self.rename_exclusive_unix(source, target_name)
        }
        #[cfg(windows)]
        {
            if !self.matches(&self.canonical)? {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "captured parent identity changed",
                ));
            }
            crate::storage::move_file_write_through(
                source,
                &self.canonical.join(target_name),
                false,
            )
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (source, target_name);
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "exclusive directory publication is unsupported",
            ))
        }
    }

    #[cfg(unix)]
    fn rename_exclusive_unix(&self, source: &Path, target_name: &OsStr) -> io::Result<()> {
        use std::os::fd::AsRawFd;
        use std::os::unix::ffi::OsStrExt;

        let relative_source = source.strip_prefix(&self.canonical).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "staging directory escaped its parent",
            )
        })?;
        let old = std::ffi::CString::new(relative_source.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid staging name"))?;
        let new = std::ffi::CString::new(target_name.as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid target name"))?;

        #[cfg(any(target_os = "macos", target_os = "ios"))]
        let result = unsafe {
            libc::renameatx_np(
                self.handle.as_raw_fd(),
                old.as_ptr(),
                self.handle.as_raw_fd(),
                new.as_ptr(),
                libc::RENAME_EXCL,
            )
        };
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                self.handle.as_raw_fd(),
                old.as_ptr(),
                self.handle.as_raw_fd(),
                new.as_ptr(),
                1_u32,
            ) as libc::c_int
        };
        #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "linux")))]
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "exclusive directory publication is unsupported",
        ));

        #[cfg(any(target_os = "macos", target_os = "ios", target_os = "linux"))]
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

pub(super) fn create_private_directory(path: &Path) -> io::Result<()> {
    fs::create_dir(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub(super) fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut options = fs::OpenOptions::new();
        options.read(true).custom_flags(libc::O_NOFOLLOW);
        let directory = options.open(path)?;
        if !directory.metadata()?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "payload is not a directory",
            ));
        }
        directory.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "directory sync is unsupported",
        ))
    }
}

pub(super) fn create_private_file(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

pub(super) fn open_regular_nofollow(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !is_plain_regular_metadata(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "entry is not a regular file",
        ));
    }
    Ok(file)
}

pub(super) fn is_plain_directory(path: &Path) -> io::Result<bool> {
    let metadata = fs::symlink_metadata(path)?;
    Ok(metadata.is_dir() && !is_reparse(&metadata))
}

pub(super) fn is_plain_regular_metadata(metadata: &fs::Metadata) -> bool {
    metadata.is_file() && !is_reparse(metadata)
}

#[cfg(windows)]
fn is_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
}

#[cfg(not(windows))]
fn is_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(windows)]
fn open_windows_directory(path: &Path) -> io::Result<fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_LIST_DIRECTORY,
        FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    let mut options = fs::OpenOptions::new();
    options
        .access_mode(FILE_LIST_DIRECTORY)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
    options.open(path)
}

#[cfg(windows)]
fn windows_file_identity(file: &fs::File) -> io::Result<(u32, u32, u32)> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };

    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    let succeeded = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) };
    if succeeded == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok((
            information.dwVolumeSerialNumber,
            information.nFileIndexHigh,
            information.nFileIndexLow,
        ))
    }
}
