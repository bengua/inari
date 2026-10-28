//! Detached Ed25519 signatures over an exported capsule.
//!
//! The signature covers the bytes of `manifest.json`, which in turn digests every other file
//! and the journal: change anything and the signature stops matching. It lives in
//! `signature/`, which the manifest does not digest (it could not, without chasing its own
//! tail). A signature attests an exported state; a capsule that keeps running (holder changes,
//! new events) must be signed again.
//!
//! Keys are the raw 32 bytes in base64, as the Python reference wrote them: a key pair and a
//! signature made by either verify with the other.

use crate::canon::{canonical, sha256_hex};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::Path;

pub const SIG: &str = "signature/manifest.sig.json";

/// A new key pair: (private, public), each the base64 of its raw 32 bytes.
pub fn generate_keypair() -> (String, String) {
    let key = SigningKey::from_bytes(&crate::wire::random_bytes::<32>());
    (
        B64.encode(key.to_bytes()),
        B64.encode(key.verifying_key().to_bytes()),
    )
}

/// What a public key is known by: the first 16 hex digits of its SHA-256.
pub fn key_id(public_b64: &str) -> String {
    let raw = B64.decode(public_b64.trim()).unwrap_or_default();
    sha256_hex(&raw)[..16].to_string()
}

fn payload(root: &Path) -> std::io::Result<Vec<u8>> {
    let manifest = std::fs::read(root.join("manifest.json"))?;
    Ok(canonical(
        &json!({"manifest_sha256": sha256_hex(&manifest)}),
    ))
}

fn key32(b64: &str) -> Result<[u8; 32], String> {
    let raw = B64
        .decode(b64.trim())
        .map_err(|e| format!("not base64: {e}"))?;
    <[u8; 32]>::try_from(raw.as_slice()).map_err(|_| "a key is 32 bytes".to_string())
}

/// Sign the capsule at `root` with a private key (base64); the signer's key id.
pub fn sign_capsule(root: &Path, private_b64: &str) -> Result<String, String> {
    let key = SigningKey::from_bytes(&key32(private_b64)?);
    let public = B64.encode(key.verifying_key().to_bytes());
    let signature = key.sign(&payload(root).map_err(|e| e.to_string())?);
    let doc = json!({
        "format": "ssc-sig/1",
        "key_id": key_id(&public),
        "public_key": public,
        "signature": B64.encode(signature.to_bytes()),
    });
    let out = root.join(SIG);
    crate::capsule::io::atomic_write(
        &out,
        serde_json::to_string_pretty(&doc)
            .expect("serializes")
            .as_bytes(),
    )
    .map_err(|e| e.to_string())?;
    Ok(key_id(&public))
}

/// Problems, empty when the capsule is signed by one of the `trusted` public keys (base64) and
/// unchanged since.
pub fn verify_signature(root: &Path, trusted: &BTreeSet<String>) -> Vec<String> {
    let path = root.join(SIG);
    if !path.exists() {
        return vec!["not signed".into()];
    }
    let check = || -> Result<Vec<String>, String> {
        let doc: Value = serde_json::from_slice(&std::fs::read(&path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let public = doc["public_key"].as_str().ok_or("'public_key'")?;
        if !trusted.contains(public) {
            let id = doc["key_id"].as_str().unwrap_or("?");
            return Ok(vec![format!("signed by an untrusted key ({id})")]);
        }
        let key = VerifyingKey::from_bytes(&key32(public)?).map_err(|e| e.to_string())?;
        let sig = B64
            .decode(doc["signature"].as_str().ok_or("'signature'")?)
            .map_err(|e| e.to_string())?;
        let sig = Signature::from_slice(&sig).map_err(|e| e.to_string())?;
        let data = payload(root).map_err(|e| e.to_string())?;
        Ok(match key.verify(&data, &sig) {
            Ok(()) => vec![],
            Err(_) => vec!["signature does not match: the capsule changed after signing".into()],
        })
    };
    check().unwrap_or_else(|e| vec![format!("unreadable signature: {e}")])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capsule() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("manifest.json"), r#"{"a": 1}"#).unwrap();
        dir
    }

    #[test]
    fn a_signed_capsule_verifies_until_it_changes() {
        let dir = capsule();
        let (private, public) = generate_keypair();
        let id = sign_capsule(dir.path(), &private).unwrap();
        assert_eq!(id, key_id(&public));
        let trusted: BTreeSet<String> = [public.clone()].into();
        assert!(verify_signature(dir.path(), &trusted).is_empty());
        let (_, other) = generate_keypair();
        assert_eq!(
            verify_signature(dir.path(), &[other].into()),
            vec![format!("signed by an untrusted key ({id})")]
        );
        std::fs::write(dir.path().join("manifest.json"), r#"{"a": 2}"#).unwrap();
        assert_eq!(
            verify_signature(dir.path(), &trusted),
            vec!["signature does not match: the capsule changed after signing"]
        );
    }

    #[test]
    fn an_unsigned_capsule_says_so() {
        assert_eq!(
            verify_signature(capsule().path(), &BTreeSet::new()),
            vec!["not signed"]
        );
    }
}
