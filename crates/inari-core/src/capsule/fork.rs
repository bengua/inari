//! Fork a capsule at an event, merge one capsule's journal into another, and the
//! uuid both need. Same results as the reference.

use super::io::{self, atomic_write, Capsule, TIMELINE, VECTORS};
use super::timeline::Timeline;
use super::{Error, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

/// A random (version 4) UUID in its usual text form.
pub fn uuid4() -> String {
    let mut b: [u8; 16] = crate::wire::random_bytes();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == "signature" {
            continue; // a signature attests the source, not the copy
        }
        let target = to.join(&name);
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Copy `src` to `dest` as a new thread whose journal stops at `upto` (default:
/// everything). The fork records where it came from; nobody holds it yet.
pub fn fork(src: &Path, dest: &Path, new_thread: &str, upto: Option<&str>) -> Result<Capsule> {
    let source = io::load(src)?;
    let mut events: Vec<_> = source.timeline.ordered().into_iter().cloned().collect();
    if let Some(id) = upto {
        let at = events
            .iter()
            .position(|e| e.id == id)
            .ok_or_else(|| Error::Integrity(format!("no event {id} in the source journal")))?;
        events.truncate(at + 1);
    }
    copy_tree(src, dest)?;
    let mut timeline = Timeline::new();
    for e in &events {
        timeline.add(e.clone())?;
    }
    let mut manifest = source.manifest.clone();
    let mut lineage = manifest
        .extra
        .get("lineage")
        .cloned()
        .unwrap_or_else(|| json!({}));
    lineage["parent_capsule"] = json!(source.manifest.capsule_id);
    lineage["fork_event_id"] = events.last().map(|e| json!(e.id)).unwrap_or(Value::Null);
    if lineage.get("merged_from").is_none() {
        lineage["merged_from"] = json!([]);
    }
    manifest.extra.insert("lineage".into(), lineage);
    manifest.extra.insert("holder".into(), Value::Null);
    manifest.capsule_id = uuid4();
    manifest.thread_id = new_thread.into();
    atomic_write(&dest.join(TIMELINE), timeline.dumps().as_bytes())?;
    io::seal(dest, manifest, &timeline)?;
    io::load(dest)
}

/// Bring `src`'s journal and notes into `dest`; returns the events added. Working
/// memory and workspace stay `dest`'s: the holder alone writes them.
pub fn merge(dest: &Path, src: &Path) -> Result<usize> {
    let target = io::load(dest)?;
    let other = io::load(src)?;
    let mut timeline = target.timeline.clone();
    let added = timeline.merge(&other.timeline)?; // refuses two histories claiming one event
    let read_index = |p: &Path| -> Value {
        fs::read(p.join(VECTORS))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_else(|| json!({"dim": 64, "embedder": "hashing-v1", "items": {}}))
    };
    let mut index = read_index(dest);
    if let (Some(mine), Some(theirs)) = (
        index.get("items").cloned(),
        read_index(src).get("items").cloned(),
    ) {
        let mut items = mine.as_object().cloned().unwrap_or_default();
        for (id, item) in theirs.as_object().cloned().unwrap_or_default() {
            items.entry(id).or_insert(item);
        }
        index["items"] = Value::Object(items);
    }
    atomic_write(&dest.join(TIMELINE), timeline.dumps().as_bytes())?;
    atomic_write(&dest.join(VECTORS), &crate::canon::canonical(&index))?;
    let mut manifest = target.manifest.clone();
    let mut lineage = manifest
        .extra
        .get("lineage")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let mut merged = lineage
        .get("merged_from")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    merged.push(json!(other.manifest.capsule_id));
    lineage["merged_from"] = Value::Array(merged);
    manifest.extra.insert("lineage".into(), lineage);
    io::seal(dest, manifest, &timeline)?;
    Ok(added)
}
