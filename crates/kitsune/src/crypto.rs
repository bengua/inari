//! Encryption at rest for Kitsune's audio — the raw voice captures and the synthesised speech,
//! the bulk of what is sensitive and what lives as files beside the store. Pure Rust
//! (AES-256-GCM, RustCrypto), so the static binary stays static and no C crypto is pulled in.
//! The key is kept in a file the core's user alone can read (mode 600), as Inari keeps its other
//! secrets (`keys.json`, `session.key`); its path is configuration, never assumed. The store
//! itself (notes, embeddings) is protected at rest by volume encryption (LUKS), the same posture
//! as Inari's capsules.

use crate::{Error, Result};
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use std::path::{Path, PathBuf};

const NONCE_LEN: usize = 12;

/// The 32-byte key that seals a store's audio.
#[derive(Clone)]
pub struct Key([u8; 32]);

impl Key {
    /// Read the key from `path`, or make one there on first use (mode 600, the core's user only).
    pub fn load_or_create(path: &Path) -> Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) if bytes.len() == 32 => {
                let mut k = [0u8; 32];
                k.copy_from_slice(&bytes);
                Ok(Key(k))
            }
            Ok(_) => Err(Error::Invalid(format!(
                "the key file {} is not 32 bytes: move it aside to make a new one",
                path.display()
            ))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut k = [0u8; 32];
                getrandom::getrandom(&mut k).map_err(|e| Error::Invalid(e.to_string()))?;
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| Error::Invalid(e.to_string()))?;
                }
                write_private(path, &k)?;
                Ok(Key(k))
            }
            Err(e) => Err(Error::Invalid(e.to_string())),
        }
    }

    /// Seal bytes: a fresh random nonce, then the AES-256-GCM ciphertext (the tag is in it).
    pub fn seal(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let cipher = Aes256Gcm::new(self.0.as_ref().into());
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::getrandom(&mut nonce).map_err(|e| Error::Invalid(e.to_string()))?;
        let ct = cipher
            .encrypt(Nonce::from_slice(&nonce), plaintext)
            .map_err(|_| Error::Invalid("sealing failed".into()))?;
        let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        Ok(out)
    }

    /// Open what `seal` made; a wrong key or a changed byte fails (the GCM tag).
    pub fn open(&self, blob: &[u8]) -> Result<Vec<u8>> {
        if blob.len() < NONCE_LEN + 16 {
            return Err(Error::Invalid("the sealed data is too short".into()));
        }
        let (nonce, ct) = blob.split_at(NONCE_LEN);
        let cipher = Aes256Gcm::new(self.0.as_ref().into());
        cipher.decrypt(Nonce::from_slice(nonce), ct).map_err(|_| {
            Error::Invalid("the audio could not be opened: wrong key or changed data".into())
        })
    }
}

/// Write a file only the core's user may read (mode 600 on Unix), atomically.
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|e| Error::Invalid(e.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path).map_err(|e| Error::Invalid(e.to_string()))?;
    Ok(())
}

/// Where a store's audio lives, sealed. A file is written under `dir/audio/` with a random name
/// and a `.enc` suffix; the store keeps only the relative path (as `AudioRef::path`).
pub struct AudioVault {
    dir: PathBuf,
    key: Key,
}

impl AudioVault {
    pub fn new(store_dir: &Path, key: Key) -> Self {
        Self {
            dir: store_dir.to_path_buf(),
            key,
        }
    }

    fn resolve(&self, rel: &str) -> Result<PathBuf> {
        if rel.is_empty()
            || rel.starts_with('/')
            || rel.contains("..")
            || !rel.starts_with("audio/")
        {
            return Err(Error::Invalid(format!(
                "not a path inside the store's audio: {rel:?}"
            )));
        }
        Ok(self.dir.join(rel))
    }

    /// Seal `bytes` into a new file; the relative path to keep in the store.
    pub fn put(&self, bytes: &[u8], ext: &str) -> Result<String> {
        let ext = ext.trim_matches('.');
        let ext = if ext.is_empty() { "bin" } else { ext };
        if !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(Error::Invalid(format!("not a file extension: {ext:?}")));
        }
        let mut raw = [0u8; 8];
        getrandom::getrandom(&mut raw).map_err(|e| Error::Invalid(e.to_string()))?;
        let name = raw.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let rel = format!("audio/{name}.{ext}.enc");
        let path = self.resolve(&rel)?;
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| Error::Invalid(e.to_string()))?;
        }
        let sealed = self.key.seal(bytes)?;
        write_private(&path, &sealed)?;
        Ok(rel)
    }

    /// Open the file the store keeps at `rel`.
    pub fn get(&self, rel: &str) -> Result<Vec<u8>> {
        let path = self.resolve(rel)?;
        let blob = std::fs::read(&path).map_err(|e| Error::Invalid(e.to_string()))?;
        self.key.open(&blob)
    }

    /// Forget an audio file (a note removed); a missing one is not an error.
    pub fn remove(&self, rel: &str) -> Result<()> {
        let path = self.resolve(rel)?;
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::Invalid(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_made_once_then_reused_and_kept_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets").join("audio.key");
        let k1 = Key::load_or_create(&path).unwrap();
        assert!(path.is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        // the second load is the same key: what k1 sealed, k2 opens
        let k2 = Key::load_or_create(&path).unwrap();
        let sealed = k1.seal(b"hello").unwrap();
        assert_eq!(k2.open(&sealed).unwrap(), b"hello");
    }

    #[test]
    fn sealing_hides_the_text_and_a_wrong_key_or_change_fails() {
        let a = Key([1u8; 32]);
        let b = Key([2u8; 32]);
        let sealed = a.seal(b"des oeufs et du pain").unwrap();
        assert!(
            !sealed.windows(4).any(|w| w == b"oeuf"),
            "the plaintext is not in the blob"
        );
        assert_eq!(a.open(&sealed).unwrap(), b"des oeufs et du pain");
        assert!(b.open(&sealed).is_err(), "a wrong key fails");
        let mut tampered = sealed.clone();
        *tampered.last_mut().unwrap() ^= 0x01;
        assert!(a.open(&tampered).is_err(), "a changed byte fails");
        // two sealings of the same text differ (fresh nonce each time)
        assert_ne!(a.seal(b"x").unwrap(), a.seal(b"x").unwrap());
    }

    #[test]
    fn the_vault_writes_sealed_audio_under_the_store_and_reads_it_back() {
        let dir = tempfile::tempdir().unwrap();
        let key = Key::load_or_create(&dir.path().join("audio.key")).unwrap();
        let vault = AudioVault::new(dir.path(), key);
        let wav = b"RIFF....fake wav bytes....";
        let rel = vault.put(wav, "wav").unwrap();
        assert!(rel.starts_with("audio/") && rel.ends_with(".wav.enc"));
        // on disk it is sealed, not the raw bytes
        let on_disk = std::fs::read(dir.path().join(&rel)).unwrap();
        assert_ne!(on_disk, wav);
        assert_eq!(vault.get(&rel).unwrap(), wav);
        // path traversal is refused, both ways
        assert!(vault.get("../escape").is_err());
        assert!(vault.put(wav, "wav/../x").is_err());
        // remove, then it is gone (and removing again is fine)
        vault.remove(&rel).unwrap();
        assert!(vault.get(&rel).is_err());
        vault.remove(&rel).unwrap();
    }
}
