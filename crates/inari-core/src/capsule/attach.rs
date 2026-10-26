//! Files a person brings to a thread — a photo from the phone, a PDF, a document —
//! land in the capsule's workspace snapshot, so they travel with the agent.
//!
//! Large files arrive in parts (`UploadStore`): each part must start where the
//! previous one ended, a repeated part is ignored, and nothing reaches the capsule
//! until the last byte is in.

use super::io::{atomic_write, load, safe_relative, seal, BLOBS, SNAPSHOT};
use super::model::Snapshot;
use super::{Error, Result};
use crate::canon::sha256_hex;
use std::fs;
use std::path::{Path, PathBuf};

/// What a file is, from its first bytes (never from its name).
pub fn sniff(data: &[u8]) -> &'static str {
    match data {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [0xFF, 0xD8, 0xFF, ..] => "image/jpeg",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
        [b'G', b'I', b'F', b'8', ..] => "image/gif",
        [b'%', b'P', b'D', b'F', ..] => "application/pdf",
        [_, _, _, _, b'f', b't', b'y', b'p', b'h', b'e', b'i', b'c', ..]
        | [_, _, _, _, b'f', b't', b'y', b'p', b'm', b'i', b'f', b'1', ..] => "image/heic",
        _ if std::str::from_utf8(data).is_ok() => "text/plain",
        _ => "application/octet-stream",
    }
}

pub fn is_image(mime: &str) -> bool {
    mime.starts_with("image/")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attached {
    pub path: String,
    pub sha256: String,
    pub size: u64,
    pub mime: &'static str,
}

/// Put `data` at `rel` in the capsule's workspace and reseal.
pub fn attach(root: &Path, rel: &str, data: &[u8]) -> Result<Attached> {
    let clean = safe_relative(rel)
        .ok_or_else(|| Error::Integrity(format!("path leaves the workspace: {rel}")))?;
    let name = clean
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    let capsule = load(root)?;
    let sha = sha256_hex(data);
    let blob = root.join(BLOBS).join(&sha);
    if !blob.is_file() {
        atomic_write(&blob, data)?;
    }
    let mut tree = capsule.snapshot.tree.clone();
    tree.insert(name.clone(), sha.clone());
    let snapshot = Snapshot {
        parent: Some(capsule.snapshot.digest()),
        tree,
    };
    atomic_write(
        &root.join(SNAPSHOT),
        serde_json::to_string_pretty(&snapshot)?.as_bytes(),
    )?;
    seal(root, capsule.manifest, &capsule.timeline)?;
    Ok(Attached {
        path: name,
        sha256: sha,
        size: data.len() as u64,
        mime: sniff(data),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Partial { received: u64 },
    Complete(PathBuf),
}

/// Parts of uploads in progress, one file per upload id.
pub struct UploadStore {
    dir: PathBuf,
    limit: u64,
}

impl UploadStore {
    pub fn new(dir: impl Into<PathBuf>, limit: u64) -> Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        Ok(Self { dir, limit })
    }

    fn part(&self, id: &str) -> Result<PathBuf> {
        if id.is_empty()
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(Error::Integrity(format!("bad upload id: {id}")));
        }
        Ok(self.dir.join(format!("{id}.upload")))
    }

    /// Add one part. `offset` must be the bytes received so far; a part that was
    /// already received (a retry) is ignored.
    pub fn put(&self, id: &str, offset: u64, total: u64, bytes: &[u8]) -> Result<Progress> {
        if total > self.limit {
            return Err(Error::Integrity(format!(
                "file is {total} bytes, over the {} limit",
                self.limit
            )));
        }
        let path = self.part(id)?;
        let have = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let end = offset + bytes.len() as u64;
        if end > total {
            return Err(Error::Integrity("part runs past the declared size".into()));
        }
        if offset < have {
            if end > have {
                return Err(Error::Integrity("part overlaps what was received".into()));
            }
        } else if offset == have {
            use std::io::Write;
            let mut file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)?;
            file.write_all(bytes)?;
            file.sync_data()?;
        } else {
            return Err(Error::Integrity(format!(
                "part starts at {offset}, expected {have}"
            )));
        }
        let now = fs::metadata(&path)?.len();
        Ok(if now == total {
            Progress::Complete(path)
        } else {
            Progress::Partial { received: now }
        })
    }

    /// Hand the finished file over and forget it.
    pub fn take(&self, id: &str) -> Result<Vec<u8>> {
        let path = self.part(id)?;
        let data = fs::read(&path)?;
        fs::remove_file(path)?;
        Ok(data)
    }
}
