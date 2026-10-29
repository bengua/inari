//! Capsules the Python reference wrote before it was retired (`fixtures/python/`, made by
//! `fixtures/make_python_fixtures.py`): the Rust core reads them hash for hash, finds the same
//! damage in the same words, verifies the reference's signatures, and keeps writing capsules
//! that stay valid. Older capsules on disk never need converting.

use inari_core::capsule::{self, EventKind};
use inari_core::signature;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python")
}

fn expected() -> Value {
    serde_json::from_slice(&std::fs::read(fixtures().join("expected.json")).unwrap()).unwrap()
}

/// A copy the test may change.
fn copy(src: &Path, dest: &Path) {
    std::fs::create_dir_all(dest).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let to = dest.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &to);
        } else {
            std::fs::copy(entry.path(), to).unwrap();
        }
    }
}

#[test]
fn a_python_capsule_reads_hash_for_hash() {
    let root = fixtures().join("capsule");
    assert_eq!(capsule::verify(&root), Vec::<String>::new());
    let c = capsule::load(&root).unwrap();
    let want = expected();
    assert_eq!(c.manifest.thread_id, want["thread"].as_str().unwrap());
    assert_eq!(c.manifest.capsule_id, want["capsule"].as_str().unwrap());
    let hashes: Vec<&str> = c
        .timeline
        .ordered()
        .iter()
        .map(|e| e.hash.as_str())
        .collect();
    let want_hashes: Vec<&str> = want["hashes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h.as_str().unwrap())
        .collect();
    assert_eq!(hashes, want_hashes, "same events, same order, same hashes");
    for e in c.timeline.ordered() {
        assert_eq!(
            e.hash,
            e.compute_hash(),
            "{} recomputes to the same hash",
            e.id
        );
    }
    let text = c
        .timeline
        .ordered()
        .iter()
        .find_map(|e| e.data["text"].as_str().filter(|t| t.contains('🦊')));
    assert_eq!(text, Some("fait ✓ — λ 🦊"));
    let tree: Vec<&String> = c.snapshot.tree.keys().collect();
    let want_tree: Vec<&str> = want["workspace"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap())
        .collect();
    assert_eq!(tree, want_tree);
}

#[test]
fn a_python_archive_unpacks_and_verifies() {
    let dir = tempfile::tempdir().unwrap();
    capsule::unpack(&fixtures().join("capsule.ssc"), &dir.path().join("c")).unwrap();
    assert_eq!(capsule::verify(&dir.path().join("c")), Vec::<String>::new());
    assert_eq!(
        capsule::load(&dir.path().join("c")).unwrap().timeline.len(),
        expected()["events"].as_u64().unwrap() as usize
    );
}

#[test]
fn damage_is_reported_in_the_reference_words() {
    let dir = tempfile::tempdir().unwrap();
    copy(&fixtures().join("capsule"), dir.path());
    std::fs::write(dir.path().join("memory/working_memory.md"), "tampered").unwrap();
    assert!(capsule::verify(dir.path())
        .contains(&"digest mismatch: memory/working_memory.md".to_string()));
}

#[test]
fn a_python_capsule_lives_on_in_rust() {
    let dir = tempfile::tempdir().unwrap();
    copy(&fixtures().join("capsule"), dir.path());
    capsule::append_event(
        dir.path(),
        "rust@thread-1",
        EventKind::Decision,
        json!({"text": "decided in Rust ✓"}),
        1714000002.5,
    )
    .unwrap();
    assert_eq!(capsule::verify(dir.path()), Vec::<String>::new());
    assert_eq!(
        capsule::load(dir.path()).unwrap().timeline.len(),
        expected()["events"].as_u64().unwrap() as usize + 1
    );
}

#[test]
fn the_reference_signature_verifies_and_rust_signs_alike() {
    let signed = fixtures().join("signed");
    let public = std::fs::read_to_string(fixtures().join("test-signer.pub"))
        .unwrap()
        .trim()
        .to_string();
    let trusted: BTreeSet<String> = [public.clone()].into();
    assert_eq!(capsule::verify(&signed), Vec::<String>::new());
    assert_eq!(
        signature::verify_signature(&signed, &trusted),
        Vec::<String>::new()
    );
    assert_eq!(
        signature::key_id(&public),
        expected()["key_id"].as_str().unwrap()
    );
    // the same key in Rust writes the same signature (Ed25519 is deterministic)
    let dir = tempfile::tempdir().unwrap();
    copy(&fixtures().join("capsule"), dir.path());
    let private = std::fs::read_to_string(fixtures().join("test-signer.key")).unwrap();
    signature::sign_capsule(dir.path(), private.trim()).unwrap();
    let ours: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join(signature::SIG)).unwrap()).unwrap();
    let theirs: Value =
        serde_json::from_slice(&std::fs::read(signed.join(signature::SIG)).unwrap()).unwrap();
    assert_eq!(ours, theirs);
    // a changed capsule no longer matches
    std::fs::write(dir.path().join("manifest.json"), b"{}").unwrap();
    assert_eq!(
        signature::verify_signature(dir.path(), &trusted),
        vec!["signature does not match: the capsule changed after signing"]
    );
}
