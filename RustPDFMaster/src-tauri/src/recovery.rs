//! Private, local-only draft journal. Never included in release artifacts.

use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{self, Read, Write},
    path::PathBuf,
};

const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_ENVELOPE: u64 = 360 * 1024 * 1024;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    documents: Option<Vec<Envelope>>,
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
    #[cfg(test)]
    pub fn save(&self, name: &str, bytes: &[u8], dirty: bool) -> io::Result<()> {
        if bytes.len() > MAX_BYTES || name.len() > 1024 {
            return Err(io::Error::other("draft exceeds size budget"));
        }
        self.reject_link()?;
        let envelope = Envelope {
            version: 1,
            name: name.into(),
            pdf: STANDARD.encode(bytes),
            dirty,
            documents: None,
        };
        self.write_envelope(&envelope)
    }
    pub fn save_many(&self, drafts: &[Draft]) -> io::Result<()> {
        if drafts.len() > 16
            || drafts.iter().map(|d| d.bytes.len() as u64).sum::<u64>() > 256 * 1024 * 1024
        {
            return Err(io::Error::other("workspace draft exceeds budget"));
        }
        let mut documents = Vec::new();
        for draft in drafts {
            if draft.bytes.len() > MAX_BYTES || draft.name.len() > 1024 {
                return Err(io::Error::other("draft exceeds size budget"));
            }
            documents.push(Envelope {
                version: 1,
                name: draft.name.clone(),
                pdf: STANDARD.encode(&draft.bytes),
                dirty: true,
                documents: None,
            });
        }
        self.write_envelope(&Envelope {
            version: 2,
            name: String::new(),
            pdf: String::new(),
            dirty: false,
            documents: Some(documents),
        })
    }
    fn write_envelope(&self, envelope: &Envelope) -> io::Result<()> {
        self.reject_link()?;
        let parent = self
            .path
            .parent()
            .ok_or_else(|| io::Error::other("draft parent missing"))?;
        let mut staged = tempfile::NamedTempFile::new_in(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            staged
                .as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        serde_json::to_writer(&mut staged, envelope)?;
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
    #[cfg(test)]
    pub fn load(&self) -> io::Result<Option<Draft>> {
        let mut drafts = self.load_many()?;
        if drafts.len() > 1 {
            return Err(io::Error::other(
                "multiple drafts require workspace recovery",
            ));
        }
        Ok(drafts.pop())
    }
    pub fn load_many(&self) -> io::Result<Vec<Draft>> {
        self.reject_link()?;
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
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
        let entries = match envelope.version {
            1 if envelope.documents.is_none() => vec![envelope],
            2 if envelope.name.is_empty() && envelope.pdf.is_empty() && !envelope.dirty => envelope
                .documents
                .ok_or_else(|| io::Error::other("workspace drafts missing"))?,
            _ => return Err(io::Error::other("unsupported draft envelope")),
        };
        if entries.len() > 16 {
            return Err(io::Error::other("too many drafts"));
        }
        let mut result = Vec::new();
        let mut total = 0usize;
        for entry in entries {
            if entry.version != 1
                || entry.documents.is_some()
                || entry.name.len() > 1024
                || entry.pdf.len() > 45 * 1024 * 1024
            {
                return Err(io::Error::other("invalid draft entry"));
            }
            let bytes = STANDARD.decode(entry.pdf).map_err(io::Error::other)?;
            total = total
                .checked_add(bytes.len())
                .ok_or_else(|| io::Error::other("draft budget"))?;
            if bytes.len() > MAX_BYTES || total > 256 * 1024 * 1024 {
                return Err(io::Error::other("draft PDF exceeds budget"));
            }
            if entry.dirty {
                result.push(Draft {
                    name: entry.name,
                    bytes,
                });
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_journal_roundtrips_every_draft_and_reads_v1() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("draft.json");
        let journal = Journal::new(path.clone());
        journal.save("legacy.pdf", b"legacy", true).unwrap();
        assert_eq!(journal.load_many().unwrap()[0].bytes, b"legacy");
        journal
            .save_many(&[
                Draft {
                    name: "一.pdf".into(),
                    bytes: b"first".to_vec(),
                },
                Draft {
                    name: "二.pdf".into(),
                    bytes: b"second".to_vec(),
                },
            ])
            .unwrap();
        let drafts = Journal::new(path).load_many().unwrap();
        assert_eq!(drafts.len(), 2);
        assert_eq!(drafts[0].name, "一.pdf");
        assert_eq!(drafts[0].bytes, b"first");
        assert_eq!(drafts[1].name, "二.pdf");
        assert_eq!(drafts[1].bytes, b"second");
        // The legacy single-document accessor must never drop siblings.
        assert!(journal.load().is_err());
    }
    #[test]
    fn malformed_workspace_entry_rejects_the_whole_file_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("draft.json");
        let journal = Journal::new(path.clone());
        journal
            .save_many(&[
                Draft {
                    name: "a.pdf".into(),
                    bytes: b"one".to_vec(),
                },
                Draft {
                    name: "b.pdf".into(),
                    bytes: b"two".to_vec(),
                },
            ])
            .unwrap();
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        value["documents"][1]["pdf"] = serde_json::json!("invalid-base64");
        let invalid = serde_json::to_vec(&value).unwrap();
        std::fs::write(&path, &invalid).unwrap();
        assert!(journal.load_many().is_err());
        assert_eq!(std::fs::read(path).unwrap(), invalid);
    }
    #[test]
    fn excessive_workspace_count_keeps_the_previous_journal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("draft.json");
        let journal = Journal::new(path.clone());
        journal.save("original.pdf", b"one", true).unwrap();
        let before = std::fs::read(&path).unwrap();
        let drafts = (0..17)
            .map(|_| Draft {
                name: "a.pdf".into(),
                bytes: b"two".to_vec(),
            })
            .collect::<Vec<_>>();
        assert!(journal.save_many(&drafts).is_err());
        assert_eq!(std::fs::read(path).unwrap(), before);
    }

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
