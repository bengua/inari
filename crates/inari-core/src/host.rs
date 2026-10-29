//! A node's home for agents: capsules on disk, a shared blob store, and the fencing
//! record of who may run what (`holders.json`). Every write is atomic, and `recover`
//! makes the record agree with the disk after a crash. Same layout as the reference.

use crate::capsule::io::{self, atomic_write, safe_relative, BLOBS, MANIFEST};
use crate::capsule::{Capsule, EventKind};
use crate::teleport::{CallError, CallResult, Endpoint, BLOB_PREFIX};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

type Check = Box<dyn Fn(&Capsule) -> Result<(), String> + Send + Sync>;

pub struct CapsuleHost {
    pub node_id: String,
    pub root: PathBuf,
    pub agents: PathBuf,
    pub staging: PathBuf,
    pub store: PathBuf,
    records: PathBuf,
    sessions: Mutex<HashMap<String, (String, u64)>>,
    prepared: Mutex<HashSet<String>>,
    /// Proof that this node can host the agent (its harness, its secrets…), run in
    /// `prepare`, before the point of no return.
    pub on_prepare: Option<Check>,
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn err(kind: &str, e: impl std::fmt::Display) -> CallError {
    CallError::new(kind, e.to_string())
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

impl CapsuleHost {
    pub fn new(node_id: &str, root: &Path) -> std::io::Result<Self> {
        let host = Self {
            node_id: node_id.into(),
            root: root.into(),
            agents: root.join("agents"),
            staging: root.join("staging"),
            store: root.join("blobs"),
            records: root.join("holders.json"),
            sessions: Mutex::default(),
            prepared: Mutex::default(),
            on_prepare: None,
        };
        for dir in [&host.agents, &host.staging, &host.store] {
            fs::create_dir_all(dir)?;
        }
        Ok(host)
    }

    fn load_records(&self) -> Map<String, Value> {
        fs::read(&self.records)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
    }

    fn set(&self, agent: &str, holder: &str, epoch: u64, status: &str) -> CallResult<()> {
        let mut records = self.load_records();
        records.insert(
            agent.into(),
            json!({"holder": holder, "epoch": epoch, "status": status}),
        );
        atomic_write(
            &self.records,
            serde_json::to_string(&records)
                .expect("serializes")
                .as_bytes(),
        )
        .map_err(|e| err("OSError", e))
    }

    fn record(&self, agent: &str) -> Option<(String, u64, String)> {
        let r = self.load_records().get(agent)?.clone();
        Some((
            r.get("holder")?.as_str()?.to_string(),
            r.get("epoch")?.as_u64()?,
            r.get("status")?.as_str()?.to_string(),
        ))
    }

    pub fn status(&self, agent: &str) -> String {
        self.record(agent)
            .map(|r| r.2)
            .unwrap_or_else(|| "absent".into())
    }

    fn set_holder(&self, capsule_dir: &Path, epoch: u64) -> CallResult<()> {
        io::edit_manifest(capsule_dir, |m| {
            m.extra.insert(
                "holder".into(),
                json!({"node_id": self.node_id, "epoch": epoch}),
            );
        })
        .map_err(|e| err("IntegrityError", e))?;
        Ok(())
    }

    fn absorb_blobs(&self, capsule_dir: &Path) -> std::io::Result<()> {
        if let Ok(entries) = fs::read_dir(capsule_dir.join(BLOBS)) {
            for entry in entries {
                let entry = entry?;
                let target = self.store.join(entry.file_name());
                if !target.exists() {
                    fs::copy(entry.path(), target)?;
                }
            }
        }
        Ok(())
    }

    /// Take ownership of an existing capsule as its running holder; returns its id.
    pub fn adopt(&self, capsule_dir: &Path) -> CallResult<String> {
        let capsule = io::load(capsule_dir).map_err(|e| err("IntegrityError", e))?;
        let agent = capsule.manifest.thread_id.clone();
        let epoch = capsule
            .manifest
            .extra
            .get("holder")
            .and_then(|h| h.get("epoch"))
            .and_then(Value::as_u64)
            .unwrap_or(1);
        let dest = self.agents.join(&agent);
        copy_dir(capsule_dir, &dest).map_err(|e| err("OSError", e))?;
        self.set_holder(&dest, epoch)?;
        self.absorb_blobs(&dest).map_err(|e| err("OSError", e))?;
        self.set(&agent, &self.node_id, epoch, "running")?;
        Ok(agent)
    }

    /// Where a thread's files are put out while it works.
    pub fn workdir(&self, agent: &str) -> PathBuf {
        self.root.join("work").join(agent)
    }

    /// The tree a kept folder was last in step with, beside it (never inside it).
    fn kept_tree(&self, agent: &str) -> PathBuf {
        self.root.join("work").join(format!("{agent}.tree.json"))
    }

    /// Put a thread's files out for a turn. A kept folder (`keep`) that is still there is only
    /// brought in step with what the thread's files became since; anything else starts afresh.
    pub fn open_workdir(&self, agent: &str, keep: bool) -> crate::capsule::Result<PathBuf> {
        let workdir = self.workdir(agent);
        let path = self.agents.join(agent);
        let before: Option<std::collections::BTreeMap<String, String>> =
            fs::read(self.kept_tree(agent))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok());
        match before {
            Some(before) if keep && workdir.is_dir() => {
                let tree = io::sync(&path, &workdir, &before)?;
                self.keep_tree(agent, &tree)?;
            }
            _ => {
                self.drop_workdir(agent);
                io::materialize(&path, &workdir)?;
            }
        }
        Ok(workdir)
    }

    /// After a turn: the folder becomes the thread's files; it stays on disk when `keep`.
    pub fn close_workdir(&self, agent: &str, keep: bool) -> crate::capsule::Result<()> {
        let workdir = self.workdir(agent);
        let kept = io::update_snapshot(&self.agents.join(agent), &workdir);
        match (&kept, keep) {
            (Ok(c), true) => self.keep_tree(agent, &c.snapshot.tree)?,
            _ => self.drop_workdir(agent),
        }
        kept.map(|_| ())
    }

    /// The folder kept for a thread put away into its files (before it leaves this node).
    pub fn put_away(&self, agent: &str) -> crate::capsule::Result<()> {
        if self.kept_tree(agent).is_file() && self.workdir(agent).is_dir() {
            let c = io::update_snapshot(&self.agents.join(agent), &self.workdir(agent))?;
            self.keep_tree(agent, &c.snapshot.tree)?;
        }
        Ok(())
    }

    /// No folder left on disk for this thread.
    pub fn drop_workdir(&self, agent: &str) {
        let _ = fs::remove_dir_all(self.workdir(agent));
        let _ = fs::remove_file(self.kept_tree(agent));
    }

    fn keep_tree(
        &self,
        agent: &str,
        tree: &std::collections::BTreeMap<String, String>,
    ) -> crate::capsule::Result<()> {
        atomic_write(
            &self.kept_tree(agent),
            serde_json::to_vec(tree).expect("serializes").as_slice(),
        )
    }

    /// After a crash: trust the capsule on disk, drop half-finished staging.
    /// Put a thread this node runs in the trash: kept on disk (`trash/<agent>-<time>`), no
    /// longer run here, not brought back by `recover`.
    pub fn discard(&self, agent: &str) -> CallResult<()> {
        let (holder, epoch, status) = self
            .record(agent)
            .ok_or_else(|| err("KeyError", format!("unknown agent {agent}")))?;
        if status != "running" {
            return Err(err("StateError", format!("{agent} is {status} here")));
        }
        let trash = self.root.join("trash");
        fs::create_dir_all(&trash).map_err(|e| err("OSError", e))?;
        let dest = trash.join(format!("{agent}-{}", now() as u64));
        fs::rename(self.agents.join(agent), &dest).map_err(|e| err("OSError", e))?;
        self.drop_workdir(agent);
        self.set(agent, &holder, epoch, "deleted")
    }

    pub fn recover(&self) -> Vec<String> {
        if let Ok(entries) = fs::read_dir(&self.staging) {
            for entry in entries.flatten() {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
        let mut fixed = Vec::new();
        let records = self.load_records();
        if let Ok(entries) = fs::read_dir(&self.agents) {
            let mut names: Vec<_> = entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            for name in names {
                let Ok(capsule) = io::load(&self.agents.join(&name)) else {
                    continue;
                };
                let Some(holder) = capsule.manifest.extra.get("holder") else {
                    continue;
                };
                let (node, epoch) = (
                    holder.get("node_id").and_then(Value::as_str),
                    holder.get("epoch").and_then(Value::as_u64),
                );
                if let (Some(node), Some(epoch)) = (node, epoch) {
                    let known = records
                        .get(&name)
                        .and_then(|r| r.get("epoch"))
                        .and_then(Value::as_u64);
                    if node == self.node_id
                        && known.is_none_or(|k| k < epoch)
                        && self.set(&name, node, epoch, "running").is_ok()
                    {
                        fixed.push(name);
                    }
                }
            }
        }
        fixed
    }

    fn safe(&self, base: &Path, rel: &str) -> CallResult<PathBuf> {
        safe_relative(rel)
            .map(|r| base.join(r))
            .ok_or_else(|| err("IntegrityError", format!("path escapes the capsule: {rel}")))
    }
}

impl Endpoint for CapsuleHost {
    fn node_id(&self) -> &str {
        &self.node_id
    }

    async fn freeze(&self, agent: &str) -> CallResult<(Value, u64)> {
        let (holder, epoch, status) = self
            .record(agent)
            .ok_or_else(|| err("InariError", format!("{agent} is not running here")))?;
        if holder != self.node_id || status != "running" {
            return Err(err("InariError", format!("{agent} is not running here")));
        }
        let path = self.agents.join(agent);
        // what changed in a folder kept between turns leaves with the thread
        self.put_away(agent).map_err(|e| err("OSError", e))?;
        io::append_event(
            &path,
            &format!("{}@{agent}", self.node_id),
            EventKind::Checkpoint,
            json!({"reason": "teleport"}),
            now(),
        )
        .map_err(|e| err("IntegrityError", e))?;
        self.set(agent, &self.node_id, epoch, "frozen")?;
        let manifest: Value =
            serde_json::from_slice(&fs::read(path.join(MANIFEST)).map_err(|e| err("OSError", e))?)
                .map_err(|e| err("IntegrityError", e))?;
        Ok((manifest, epoch))
    }

    async fn read(&self, agent: &str, rel: &str) -> CallResult<Vec<u8>> {
        fs::read(self.safe(&self.agents.join(agent), rel)?).map_err(|e| err("OSError", e))
    }

    async fn unfreeze(&self, agent: &str, epoch: u64) -> CallResult<()> {
        if let Some((_, e, status)) = self.record(agent) {
            if status == "frozen" && e == epoch {
                self.set(agent, &self.node_id, epoch, "running")?;
            }
        }
        Ok(())
    }

    async fn tombstone(&self, agent: &str, epoch: u64) -> CallResult<()> {
        self.drop_workdir(agent);
        self.set(agent, "", epoch, "tombstoned")
    }

    async fn offer(&self, sid: &str, manifest: &Value, epoch: u64) -> CallResult<Vec<String>> {
        let agent = manifest
            .get("thread_id")
            .and_then(Value::as_str)
            .ok_or_else(|| err("IntegrityError", "manifest has no thread_id"))?;
        if let Some((_, known, _)) = self.record(agent) {
            if known >= epoch {
                return Err(err(
                    "StaleEpoch",
                    format!("epoch {epoch} is not newer than {known}"),
                ));
            }
        }
        let stage = self.safe(&self.staging, sid)?;
        let _ = fs::remove_dir_all(&stage);
        fs::create_dir_all(&stage).map_err(|e| err("OSError", e))?;
        atomic_write(
            &stage.join(MANIFEST),
            serde_json::to_string_pretty(manifest)
                .expect("serializes")
                .as_bytes(),
        )
        .map_err(|e| err("OSError", e))?;
        self.sessions
            .lock()
            .expect("not poisoned")
            .insert(sid.into(), (agent.into(), epoch));
        let mut have = Vec::new();
        if let Some(files) = manifest.get("files").and_then(Value::as_object) {
            for rel in files.keys() {
                if let Some(sha) = rel.strip_prefix(BLOB_PREFIX) {
                    if self.store.join(sha).is_file() {
                        have.push(sha.to_string());
                    }
                }
            }
        }
        have.sort();
        Ok(have)
    }

    async fn receive(&self, sid: &str, rel: &str, data: &[u8]) -> CallResult<()> {
        let stage = self.safe(&self.staging, sid)?;
        atomic_write(&self.safe(&stage, rel)?, data).map_err(|e| err("OSError", e))
    }

    async fn verify(&self, sid: &str) -> CallResult<()> {
        let stage = self.safe(&self.staging, sid)?;
        let manifest = io::read_manifest(&stage).map_err(|e| err("IntegrityError", e))?;
        for rel in manifest.files.keys() {
            if let Some(sha) = rel.strip_prefix(BLOB_PREFIX) {
                let target = stage.join(rel);
                if !target.exists() && self.store.join(sha).is_file() {
                    let data = fs::read(self.store.join(sha)).map_err(|e| err("OSError", e))?;
                    atomic_write(&target, &data).map_err(|e| err("OSError", e))?;
                }
            }
        }
        let problems = io::verify(&stage);
        if problems.is_empty() {
            Ok(())
        } else {
            Err(err("IntegrityError", problems.join("; ")))
        }
    }

    async fn prepare(&self, sid: &str) -> CallResult<()> {
        let stage = self.safe(&self.staging, sid)?;
        let capsule = io::load(&stage).map_err(|e| err("IntegrityError", e))?;
        if let Some(check) = &self.on_prepare {
            check(&capsule).map_err(|e| err("ValueError", e))?;
        }
        self.prepared
            .lock()
            .expect("not poisoned")
            .insert(sid.into());
        Ok(())
    }

    async fn activate(&self, sid: &str, epoch: u64) -> CallResult<()> {
        if !self.prepared.lock().expect("not poisoned").contains(sid) {
            return Err(err("InariError", "activate before prepare"));
        }
        let (agent, offered) = self
            .sessions
            .lock()
            .expect("not poisoned")
            .get(sid)
            .cloned()
            .ok_or_else(|| err("InariError", "unknown session"))?;
        if epoch != offered {
            return Err(err(
                "StaleEpoch",
                format!("session was offered epoch {offered}, not {epoch}"),
            ));
        }
        let stage = self.safe(&self.staging, sid)?;
        self.set_holder(&stage, epoch)?;
        let dest = self.agents.join(&agent);
        let _ = fs::remove_dir_all(&dest);
        fs::rename(&stage, &dest).map_err(|e| err("OSError", e))?;
        self.absorb_blobs(&dest).map_err(|e| err("OSError", e))?;
        // a folder left from an earlier stay here is older than what arrives
        self.drop_workdir(&agent);
        self.set(&agent, &self.node_id, epoch, "running")?;
        self.prepared.lock().expect("not poisoned").remove(sid);
        Ok(())
    }

    async fn abort(&self, sid: &str) -> CallResult<()> {
        if let Ok(stage) = self.safe(&self.staging, sid) {
            let _ = fs::remove_dir_all(stage);
        }
        self.prepared.lock().expect("not poisoned").remove(sid);
        self.sessions.lock().expect("not poisoned").remove(sid);
        Ok(())
    }

    async fn holder(&self, agent: &str) -> CallResult<Option<(String, u64)>> {
        Ok(self
            .record(agent)
            .filter(|r| r.2 != "tombstoned")
            .map(|r| (r.0, r.1)))
    }
}
