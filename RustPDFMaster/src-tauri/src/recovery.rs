//! Private, local-only draft journal. Never included in release artifacts.

use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{self, Read, Write},
    path::PathBuf,
};

const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_ENVELOPE: u64 = 45 * 1024 * 1024;

pub struct Journal {
    path: PathBuf,
}

pub fn lock_directory(directory: &std::path::Path) -> io::Result<File> {
    let path = directory.join("draft.lock");
    if path
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        return Err(io::Error::other("draft lock cannot be a symlink"));
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    file.try_lock().map_err(io::Error::other)?;
    Ok(file)
}
pub struct Draft {
    pub name: String,
    pub bytes: Vec<u8>,
}
#[derive(Serialize, Deserialize)]
struct Envelope {
    version: u8,
    name: String,
    pdf: String,
    dirty: bool,
}

impl Journal {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
    fn reject_link(&self) -> io::Result<()> {
        match self.path.symlink_metadata() {
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_file() => {
                Err(io::Error::other("draft path is not a regular file"))
            }
            Ok(_) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
    pub fn save(&self, name: &str, bytes: &[u8], dirty: bool) -> io::Result<()> {
        if bytes.len() > MAX_BYTES || name.len() > 1024 {
            return Err(io::Error::other("draft exceeds size budget"));
        }
        self.reject_link()?;
        let parent = self
            .path
            .parent()
            .ok_or_else(|| io::Error::other("draft parent missing"))?;
        let envelope = Envelope {
            version: 1,
            name: name.into(),
            pdf: STANDARD.encode(bytes),
            dirty,
        };
        let mut staged = tempfile::NamedTempFile::new_in(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            staged
                .as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        serde_json::to_writer(&mut staged, &envelope)?;
        staged.flush()?;
        staged.as_file().sync_all()?;
        #[cfg(windows)]
        {
            let staged_path = staged.into_temp_path();
            crate::storage::move_file_write_through(&staged_path, &self.path, true)?;
        }
        #[cfg(not(windows))]
        {
            staged.persist(&self.path).map_err(|e| e.error)?;
            #[cfg(unix)]
            File::open(parent)?.sync_all()?;
        }
        Ok(())
    }
    pub fn load(&self) -> io::Result<Option<Draft>> {
        self.reject_link()?;
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        if file.metadata()?.len() > MAX_ENVELOPE {
            return Err(io::Error::other("draft envelope exceeds budget"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_ENVELOPE + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_ENVELOPE {
            return Err(io::Error::other("draft envelope exceeds budget"));
        }
        let envelope: Envelope = serde_json::from_slice(&bytes)?;
        if envelope.version != 1 || envelope.name.len() > 1024 {
            return Err(io::Error::other("unsupported draft envelope"));
        }
        let bytes = STANDARD.decode(envelope.pdf).map_err(io::Error::other)?;
        if bytes.len() > MAX_BYTES {
            return Err(io::Error::other("draft PDF exceeds budget"));
        }
        Ok(envelope.dirty.then_some(Draft {
            name: envelope.name,
            bytes,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simultaneous_candidates_cannot_overwrite_the_same_journal() {
        let dir = tempfile::tempdir().unwrap();
        let lock = lock_directory(dir.path()).unwrap();
        assert!(lock_directory(dir.path()).is_err());
        drop(lock);
        assert!(lock_directory(dir.path()).is_ok());
    }

    #[test]
    fn dirty_draft_survives_a_new_process_view() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("draft.json");
        Journal::new(path.clone())
            .save("测试.pdf", b"synthetic bytes", true)
            .unwrap();
        let draft = Journal::new(path).load().unwrap().unwrap();
        assert_eq!(draft.name, "测试.pdf");
        assert_eq!(draft.bytes, b"synthetic bytes");
    }

    #[test]
    fn saved_draft_is_retained_but_not_reopened_as_dirty() {
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::new(dir.path().join("draft.json"));
        journal.save("test.pdf", b"one", true).unwrap();
        journal.save("test.pdf", b"two", false).unwrap();
        assert!(journal.load().unwrap().is_none());
        assert!(journal.path.exists());
    }

    #[test]
    fn malformed_journal_is_not_overwritten_by_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("draft.json");
        std::fs::write(&path, b"bad draft").unwrap();
        assert!(Journal::new(path.clone()).load().is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"bad draft");
    }

    #[cfg(unix)]
    #[test]
    fn journal_is_private_and_symlinks_are_rejected() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("draft.json");
        Journal::new(path.clone())
            .save("test.pdf", b"private", true)
            .unwrap();
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let link = dir.path().join("link.json");
        symlink(dir.path().join("outside"), &link).unwrap();
        assert!(
            Journal::new(link.clone())
                .save("test.pdf", b"bytes", true)
                .is_err()
        );
        assert!(!dir.path().join("outside").exists());
    }
}
