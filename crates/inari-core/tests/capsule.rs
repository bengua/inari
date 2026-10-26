use inari_core::capsule::attach::{attach, sniff, Progress, UploadStore};
use inari_core::capsule::{io, EventKind, Timeline};
use serde_json::json;
use std::fs;
use std::path::Path;

const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3];

fn fresh(dir: &Path) -> std::path::PathBuf {
    let root = dir.join("caps");
    let mut t = Timeline::new();
    t.append(
        "node-a@t1",
        EventKind::UserMessage,
        json!({"text": "hello λ"}),
        1.5,
    );
    io::create(&root, "t1", "c1", Some("node-a"), &t, 1.0).unwrap();
    root
}

#[test]
fn a_new_capsule_verifies_and_reads_back() {
    let dir = tempfile::tempdir().unwrap();
    let root = fresh(dir.path());
    assert!(io::verify(&root).is_empty(), "{:?}", io::verify(&root));
    let c = io::load(&root).unwrap();
    assert_eq!(c.timeline.len(), 1);
    assert_eq!(c.manifest.thread_id, "t1");
}

#[test]
fn tampering_missing_and_stray_files_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    let root = fresh(dir.path());
    fs::write(root.join(io::WORKING_MEMORY), "rewritten").unwrap();
    fs::write(root.join("stray.txt"), "x").unwrap();
    let problems = io::verify(&root);
    assert!(problems.contains(&"digest mismatch: memory/working_memory.md".to_string()));
    assert!(problems.contains(&"undeclared file: stray.txt".to_string()));
}

#[test]
fn appending_keeps_the_capsule_sound() {
    let dir = tempfile::tempdir().unwrap();
    let root = fresh(dir.path());
    let c = io::append_event(
        &root,
        "node-a@t1",
        EventKind::Decision,
        json!({"text": "go"}),
        2.0,
    )
    .unwrap();
    assert_eq!(c.timeline.len(), 2);
    assert!(io::verify(&root).is_empty());
}

#[test]
fn an_attached_photo_lands_in_the_workspace_and_cannot_escape_it() {
    let dir = tempfile::tempdir().unwrap();
    let root = fresh(dir.path());
    let got = attach(&root, "photos/cat.png", PNG).unwrap();
    assert_eq!(got.mime, "image/png");
    let c = io::load(&root).unwrap();
    assert_eq!(c.snapshot.tree.get("photos/cat.png"), Some(&got.sha256));
    assert!(c.snapshot.parent.is_some());
    assert!(io::verify(&root).is_empty());
    assert!(attach(&root, "../evil.txt", b"x").is_err());
    assert!(attach(&root, "/etc/passwd", b"x").is_err());
}

#[test]
fn files_are_recognised_by_their_bytes() {
    assert_eq!(sniff(PNG), "image/png");
    assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), "image/jpeg");
    assert_eq!(sniff(b"%PDF-1.7 ..."), "application/pdf");
    assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), "image/webp");
    assert_eq!(sniff(b"\0\0\0\x18ftypheic...."), "image/heic");
    assert_eq!(sniff("plain text é".as_bytes()), "text/plain");
    assert_eq!(sniff(&[0, 159, 146, 150]), "application/octet-stream");
}

#[test]
fn an_upload_in_parts_tolerates_retries_and_refuses_gaps() {
    let dir = tempfile::tempdir().unwrap();
    let store = UploadStore::new(dir.path().join("up"), 100).unwrap();
    assert_eq!(
        store.put("u1", 0, 6, b"abc").unwrap(),
        Progress::Partial { received: 3 }
    );
    assert_eq!(
        store.put("u1", 0, 6, b"abc").unwrap(),
        Progress::Partial { received: 3 }
    ); // retry
    assert!(store.put("u1", 5, 6, b"f").is_err()); // gap
    assert!(matches!(
        store.put("u1", 3, 6, b"def").unwrap(),
        Progress::Complete(_)
    ));
    assert_eq!(store.take("u1").unwrap(), b"abcdef");
    assert!(store.put("big", 0, 1000, b"x").is_err()); // over the limit
    assert!(store.put("../x", 0, 1, b"x").is_err()); // bad id
}

#[test]
fn archives_round_trip_and_refuse_traversal() {
    let dir = tempfile::tempdir().unwrap();
    let root = fresh(dir.path());
    let (a, b) = (dir.path().join("a.ssc"), dir.path().join("b.ssc"));
    io::pack(&root, &a).unwrap();
    io::pack(&root, &b).unwrap();
    assert_eq!(fs::read(&a).unwrap(), fs::read(&b).unwrap());
    io::unpack(&a, &dir.path().join("out")).unwrap();
    assert!(io::verify(&dir.path().join("out")).is_empty());

    let evil = dir.path().join("evil.tar");
    let mut builder = tar::Builder::new(fs::File::create(&evil).unwrap());
    let mut header = tar::Header::new_gnu();
    header.set_size(1);
    header.set_mode(0o644);
    header.set_entry_type(tar::EntryType::Regular);
    // `append_data` refuses `..`; write the raw name the way a hostile archive would.
    header.as_gnu_mut().unwrap().name[..12].copy_from_slice(b"../escape.tx");
    header.set_cksum();
    builder.append(&header, &b"x"[..]).unwrap();
    builder.finish().unwrap();
    assert!(io::unpack(&evil, &dir.path().join("dest")).is_err());
}
