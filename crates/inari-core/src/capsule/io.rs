//! Reading, writing, verifying and archiving a capsule directory.

use super::model::{EventKind, FileDigest, Manifest, Snapshot, FORMAT};
use super::timeline::Timeline;
use super::{Error, Result};
use crate::canon::sha256_hex;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

pub const MANIFEST: &str = "manifest.json";
pub const WORKING_MEMORY: &str = "memory/working_memory.md";
pub const TIMELINE: &str = "memory/timeline_events.jsonl";
pub const VECTORS: &str = "memory/vector_index/index.json";
pub const SNAPSHOT: &str = "artifacts/snapshot.json";
pub const BLOBS: &str = "artifacts/blobs";
pub const ENVIRONMENT: &str = "environment/spec.json";

pub struct Capsule {
    pub root: PathBuf,
    pub manifest: Manifest,
    pub timeline: Timeline,
    pub snapshot: Snapshot,
    pub working_memory: String,
}

/// Write through a temporary file and a rename: a crash leaves the file whole or absent.
pub fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".part");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, data)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            walk(&path, out)?;
        } else if entry.file_type()?.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .expect("walked under root")
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Digest of every file the manifest covers (not itself, not `signature/`, no `.part`).
pub fn scan(root: &Path) -> Result<BTreeMap<String, FileDigest>> {
    let mut files = Vec::new();
    walk(root, &mut files)?;
    let mut out = BTreeMap::new();
    for path in files {
        let name = rel(root, &path);
        if name == MANIFEST || name.starts_with("signature/") || name.ends_with(".part") {
            continue;
        }
        let data = fs::read(&path)?;
        out.insert(
            name,
            FileDigest {
                sha256: sha256_hex(&data),
                size: data.len() as u64,
            },
        );
    }
    Ok(out)
}

pub fn read_manifest(root: &Path) -> Result<Manifest> {
    Ok(serde_json::from_slice(&fs::read(root.join(MANIFEST))?)?)
}

/// Recompute the digests and the journal head, then write `manifest.json`.
pub fn seal(root: &Path, mut manifest: Manifest, timeline: &Timeline) -> Result<Manifest> {
    manifest.files = scan(root)?;
    manifest.timeline_head = timeline.head();
    atomic_write(
        &root.join(MANIFEST),
        serde_json::to_string_pretty(&manifest)?.as_bytes(),
    )?;
    Ok(manifest)
}

pub fn load(root: &Path) -> Result<Capsule> {
    let manifest = read_manifest(root)?;
    if manifest.format != FORMAT {
        return Err(Error::Integrity(format!(
            "unknown format {}",
            manifest.format
        )));
    }
    let timeline = Timeline::loads(&fs::read_to_string(root.join(TIMELINE))?)?;
    let snapshot: Snapshot = serde_json::from_slice(&fs::read(root.join(SNAPSHOT))?)?;
    let working_memory = fs::read_to_string(root.join(WORKING_MEMORY)).unwrap_or_default();
    Ok(Capsule {
        root: root.to_path_buf(),
        manifest,
        timeline,
        snapshot,
        working_memory,
    })
}

/// Every problem found; empty means the capsule is sound. Same wording as the reference.
pub fn verify(root: &Path) -> Vec<String> {
    let manifest = match read_manifest(root) {
        Ok(m) => m,
        Err(e) => return vec![format!("unreadable manifest: {e}")],
    };
    let actual = match scan(root) {
        Ok(a) => a,
        Err(e) => return vec![format!("unreadable: {e}")],
    };
    let mut problems = Vec::new();
    for name in manifest.files.keys().filter(|k| !actual.contains_key(*k)) {
        problems.push(format!("missing file: {name}"));
    }
    for name in actual.keys().filter(|k| !manifest.files.contains_key(*k)) {
        problems.push(format!("undeclared file: {name}"));
    }
    for (name, digest) in &manifest.files {
        if actual.get(name).is_some_and(|a| a != digest) {
            problems.push(format!("digest mismatch: {name}"));
        }
    }
    let capsule = match load(root) {
        Ok(c) => c,
        Err(e) => {
            problems.push(format!("unreadable: {e}"));
            return problems;
        }
    };
    problems.extend(capsule.timeline.verify());
    if capsule.timeline.head() != manifest.timeline_head {
        problems.push("timeline head does not match the manifest".into());
    }
    for (name, sha) in &capsule.snapshot.tree {
        if !root.join(BLOBS).join(sha).is_file() {
            problems.push(format!("snapshot blob absent: {name}"));
        }
    }
    problems
}

/// Every change to a live capsule reads it, changes it and reseals it: two changes at
/// once (a model chosen while a message arrives) would each overwrite the other. One
/// lock per capsule directory, held for the whole read-change-seal.
fn lock(root: &Path) -> std::sync::Arc<std::sync::Mutex<()>> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    let mut all = LOCKS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    Arc::clone(all.entry(root.to_path_buf()).or_default())
}

/// Run `f` while no other change to the capsule at `root` runs.
pub fn locked<T>(root: &Path, f: impl FnOnce() -> T) -> T {
    let l = lock(root);
    let _held = l.lock().unwrap_or_else(|e| e.into_inner());
    f()
}

/// Change a live capsule's manifest (labels, title, holder) and reseal it.
pub fn edit_manifest(root: &Path, f: impl FnOnce(&mut Manifest)) -> Result<Manifest> {
    locked(root, || {
        let capsule = load(root)?;
        let mut manifest = capsule.manifest;
        f(&mut manifest);
        seal(root, manifest, &capsule.timeline)
    })
}

/// Add one event to a live capsule and reseal it.
pub fn append_event(
    root: &Path,
    author: &str,
    kind: EventKind,
    data: Value,
    ts: f64,
) -> Result<Capsule> {
    locked(root, || append_event_unlocked(root, author, kind, data, ts))
}

fn append_event_unlocked(
    root: &Path,
    author: &str,
    kind: EventKind,
    data: Value,
    ts: f64,
) -> Result<Capsule> {
    let mut capsule = load(root)?;
    capsule.timeline.append(author, kind, data, ts);
    atomic_write(&root.join(TIMELINE), capsule.timeline.dumps().as_bytes())?;
    seal(root, capsule.manifest, &capsule.timeline)?;
    load(root)
}

/// A deterministic tar: same capsule, same bytes.
pub fn pack(root: &Path, archive: &Path) -> Result<()> {
    let mut files = Vec::new();
    walk(root, &mut files)?;
    let mut builder = tar::Builder::new(fs::File::create(archive)?);
    for path in files {
        let name = rel(root, &path);
        if name.ends_with(".part") {
            continue;
        }
        let data = fs::read(&path)?;
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        header.set_uid(0);
        header.set_gid(0);
        header.set_entry_type(tar::EntryType::Regular);
        builder.append_data(&mut header, &name, data.as_slice())?;
    }
    builder.finish()?;
    Ok(())
}

/// A relative path that stays inside its base: no root, no `..`.
pub fn safe_relative(name: &str) -> Option<PathBuf> {
    let path = Path::new(name);
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

pub fn unpack(archive: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    let mut tar = tar::Archive::new(fs::File::open(archive)?);
    for entry in tar.entries()? {
        let mut entry = entry?;
        let name = entry.path()?.to_string_lossy().into_owned();
        let target = match safe_relative(&name) {
            Some(rel) if entry.header().entry_type().is_file() => dest.join(rel),
            _ => return Err(Error::Integrity(format!("unsafe archive member: {name}"))),
        };
        let mut data = Vec::new();
        entry.read_to_end(&mut data)?;
        atomic_write(&target, &data)?;
    }
    Ok(())
}

/// A new capsule with the reference's defaults, held by `holder` at epoch 1.
pub fn create(
    root: &Path,
    thread_id: &str,
    capsule_id: &str,
    holder: Option<&str>,
    timeline: &Timeline,
    now: f64,
) -> Result<Capsule> {
    use serde_json::json;
    atomic_write(&root.join(WORKING_MEMORY), b"")?;
    atomic_write(&root.join(TIMELINE), timeline.dumps().as_bytes())?;
    atomic_write(
        &root.join(VECTORS),
        br#"{"dim":64,"embedder":"hashing-v1","items":{}}"#,
    )?;
    let snapshot = Snapshot::default();
    atomic_write(
        &root.join(SNAPSHOT),
        serde_json::to_string_pretty(&snapshot)?.as_bytes(),
    )?;
    let environment = json!({"image": "", "workdir": "/work", "mounts": [], "env": {},
                             "secret_refs": [], "resources": {}});
    atomic_write(
        &root.join(ENVIRONMENT),
        serde_json::to_string_pretty(&environment)?.as_bytes(),
    )?;
    fs::create_dir_all(root.join(BLOBS))?;
    let extra = json!({
        "created_at": now, "title": "", "labels": {},
        "lineage": {"parent_capsule": null, "fork_event_id": null, "merged_from": []},
        "access": {"owners": [], "readers": [], "approval_required": []},
        "holder": holder.map(|h| json!({"node_id": h, "epoch": 1})),
    });
    let manifest = Manifest {
        format: FORMAT.into(),
        capsule_id: capsule_id.into(),
        thread_id: thread_id.into(),
        timeline_head: String::new(),
        files: BTreeMap::new(),
        extra: extra.as_object().cloned().unwrap_or_default(),
    };
    seal(root, manifest, timeline)?;
    load(root)
}

/// Recreate the capsule's workspace in `dest`, refusing paths that would leave it.
pub fn materialize(root: &Path, dest: &Path) -> Result<()> {
    let capsule = load(root)?;
    fs::create_dir_all(dest)?;
    for (name, sha) in &capsule.snapshot.tree {
        let rel = safe_relative(name)
            .ok_or_else(|| Error::Integrity(format!("path escapes the workspace: {name}")))?;
        let data = fs::read(root.join(BLOBS).join(sha))?;
        if sha256_hex(&data) != *sha {
            return Err(Error::Integrity(format!(
                "blob {} is corrupt",
                &sha[..12.min(sha.len())]
            )));
        }
        let target = dest.join(rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(target, data)?;
    }
    Ok(())
}

/// Bring a folder kept between turns in step with the capsule: only what the snapshot changed
/// since `before` (the tree the folder was last in step with) is written or removed, so what
/// changed on disk meanwhile (a branch, a dev server's files) stays. Returns the new tree.
pub fn sync(
    root: &Path,
    dest: &Path,
    before: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>> {
    let capsule = load(root)?;
    let tree = capsule.snapshot.tree;
    for name in before.keys().filter(|n| !tree.contains_key(*n)) {
        if let Some(rel) = safe_relative(name) {
            let _ = fs::remove_file(dest.join(rel));
        }
    }
    for (name, sha) in &tree {
        let rel = safe_relative(name)
            .ok_or_else(|| Error::Integrity(format!("path escapes the workspace: {name}")))?;
        if before.get(name) == Some(sha) && dest.join(&rel).is_file() {
            continue;
        }
        let data = fs::read(root.join(BLOBS).join(sha))?;
        if sha256_hex(&data) != *sha {
            return Err(Error::Integrity(format!(
                "blob {} is corrupt",
                &sha[..12.min(sha.len())]
            )));
        }
        let target = dest.join(rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(target, data)?;
    }
    Ok(tree)
}

/// Record `workdir` as the capsule's new snapshot (only new content adds a blob) and reseal.
pub fn update_snapshot(root: &Path, workdir: &Path) -> Result<Capsule> {
    locked(root, || update_snapshot_unlocked(root, workdir))
}

fn update_snapshot_unlocked(root: &Path, workdir: &Path) -> Result<Capsule> {
    let capsule = load(root)?;
    let mut files = Vec::new();
    if workdir.is_dir() {
        walk(workdir, &mut files)?;
    }
    let mut tree = BTreeMap::new();
    for path in files {
        let data = fs::read(&path)?;
        let sha = sha256_hex(&data);
        let blob = root.join(BLOBS).join(&sha);
        if !blob.is_file() {
            atomic_write(&blob, &data)?;
        }
        tree.insert(rel(workdir, &path), sha);
    }
    let snapshot = Snapshot {
        parent: Some(capsule.snapshot.digest()),
        tree,
    };
    atomic_write(
        &root.join(SNAPSHOT),
        serde_json::to_string_pretty(&snapshot)?.as_bytes(),
    )?;
    seal(root, capsule.manifest, &capsule.timeline)?;
    load(root)
}
