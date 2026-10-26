//! Teleportation: a two-phase commit with a fencing epoch. At most one node runs an
//! agent, ever. Same states, same rules and same outcome fields as the reference.

use serde::Serialize;
use serde_json::Value;

pub const BLOB_PREFIX: &str = "artifacts/blobs/";

/// A failed call: `transport` is true when the reply may simply have been lost.
#[derive(Debug, Clone)]
pub struct CallError {
    pub kind: String,
    pub message: String,
    pub transport: bool,
}

impl CallError {
    pub fn new(kind: &str, message: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            message: message.into(),
            transport: false,
        }
    }
    pub fn transport(message: impl Into<String>) -> Self {
        Self {
            kind: "TransportError".into(),
            message: message.into(),
            transport: true,
        }
    }
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

pub type CallResult<T> = Result<T, CallError>;

/// What a node offers to take part in a teleportation (source side and target side).
#[allow(async_fn_in_trait)]
pub trait Endpoint {
    fn node_id(&self) -> &str;
    async fn freeze(&self, agent: &str) -> CallResult<(Value, u64)>;
    async fn read(&self, agent: &str, rel: &str) -> CallResult<Vec<u8>>;
    async fn unfreeze(&self, agent: &str, epoch: u64) -> CallResult<()>;
    async fn tombstone(&self, agent: &str, epoch: u64) -> CallResult<()>;
    async fn offer(&self, sid: &str, manifest: &Value, epoch: u64) -> CallResult<Vec<String>>;
    async fn receive(&self, sid: &str, rel: &str, data: &[u8]) -> CallResult<()>;
    async fn verify(&self, sid: &str) -> CallResult<()>;
    async fn prepare(&self, sid: &str) -> CallResult<()>;
    async fn activate(&self, sid: &str, epoch: u64) -> CallResult<()>;
    async fn abort(&self, sid: &str) -> CallResult<()>;
    async fn holder(&self, agent: &str) -> CallResult<Option<(String, u64)>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Planned,
    Frozen,
    Offered,
    Transferred,
    Verified,
    Ready,
    Committing,
    Committed,
    InDoubt,
    RollingBack,
    RolledBack,
}

impl State {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Frozen => "frozen",
            Self::Offered => "offered",
            Self::Transferred => "transferred",
            Self::Verified => "verified",
            Self::Ready => "ready",
            Self::Committing => "committing",
            Self::Committed => "committed",
            Self::InDoubt => "in_doubt",
            Self::RollingBack => "rolling_back",
            Self::RolledBack => "rolled_back",
        }
    }

    fn pre_commit(&self) -> bool {
        matches!(
            self,
            Self::Planned
                | Self::Frozen
                | Self::Offered
                | Self::Transferred
                | Self::Verified
                | Self::Ready
        )
    }
}

/// The transition table. An input that is not allowed in a state is a bug in the caller.
pub struct Machine {
    pub state: State,
    pub history: Vec<State>,
}

impl Default for Machine {
    fn default() -> Self {
        Self {
            state: State::Planned,
            history: vec![State::Planned],
        }
    }
}

impl Machine {
    pub fn fire(&mut self, event: &str) -> Result<State, String> {
        use State::*;
        let next = match (self.state, event) {
            (Planned, "freeze_ok") => Frozen,
            (Frozen, "offer_ok") => Offered,
            (Offered, "transfer_ok") => Transferred,
            (Transferred, "verify_ok") => Verified,
            (Verified, "prepare_ok") => Ready,
            (Ready, "commit_sent") => Committing,
            (Committing, "activate_ok") => Committed,
            (Committing, "activate_refused") => RollingBack,
            (Committing, "ack_lost") => InDoubt,
            (InDoubt, "resolved_committed") => Committed,
            (InDoubt, "resolved_aborted") => RollingBack,
            (RollingBack, "rollback_ok") => RolledBack,
            (s, "fail") if s.pre_commit() => RollingBack,
            (s, e) => return Err(format!("{e:?} is not allowed in {}", s.as_str())),
        };
        self.state = next;
        self.history.push(next);
        Ok(next)
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Outcome {
    pub session_id: String,
    pub state: String,
    pub epoch: u64,
    pub bytes_sent: u64,
    pub files_sent: u64,
    pub blobs_skipped: u64,
    pub error: String,
    pub needs_attention: bool,
    pub history: Vec<String>,
}

fn finish(mut out: Outcome, machine: &Machine) -> Outcome {
    out.state = machine.state.as_str().into();
    out.history = machine
        .history
        .iter()
        .map(|s| s.as_str().to_string())
        .collect();
    out
}

/// Move `agent` from `source` to `target`.
pub async fn teleport<S: Endpoint, T: Endpoint>(
    source: &S,
    target: &T,
    agent: &str,
    sid: Option<String>,
) -> Outcome {
    let sid = sid.unwrap_or_else(|| crate::wire::random_hex(16));
    let mut machine = Machine::default();
    let mut out = Outcome {
        session_id: sid.clone(),
        state: "planned".into(),
        ..Default::default()
    };
    let mut frozen_epoch: Option<u64> = None;

    // Everything before the point of no return; any error rolls back.
    let pre: CallResult<u64> = async {
        let (manifest, epoch) = source.freeze(agent).await?;
        frozen_epoch = Some(epoch);
        machine
            .fire("freeze_ok")
            .expect("planned accepts freeze_ok");
        let new_epoch = epoch + 1;
        out.epoch = new_epoch;
        let have = target.offer(&sid, &manifest, new_epoch).await?;
        machine.fire("offer_ok").expect("frozen accepts offer_ok");
        let mut names: Vec<String> = manifest
            .get("files")
            .and_then(Value::as_object)
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        names.sort();
        for rel in names {
            if let Some(sha) = rel.strip_prefix(BLOB_PREFIX) {
                if have.iter().any(|h| h == sha) {
                    out.blobs_skipped += 1;
                    continue;
                }
            }
            let data = source.read(agent, &rel).await?;
            target.receive(&sid, &rel, &data).await?;
            out.files_sent += 1;
            out.bytes_sent += data.len() as u64;
        }
        machine
            .fire("transfer_ok")
            .expect("offered accepts transfer_ok");
        target.verify(&sid).await?;
        machine
            .fire("verify_ok")
            .expect("transferred accepts verify_ok");
        target.prepare(&sid).await?;
        machine
            .fire("prepare_ok")
            .expect("verified accepts prepare_ok");
        Ok(new_epoch)
    }
    .await;

    let new_epoch = match pre {
        Ok(e) => e,
        Err(e) => {
            if machine.state.pre_commit() {
                machine.fire("fail").expect("pre-commit states accept fail");
            }
            return rollback(
                source,
                target,
                agent,
                &sid,
                frozen_epoch,
                e.to_string(),
                out,
                machine,
            )
            .await;
        }
    };

    machine
        .fire("commit_sent")
        .expect("ready accepts commit_sent");
    match target.activate(&sid, new_epoch).await {
        Ok(()) => {
            machine
                .fire("activate_ok")
                .expect("committing accepts activate_ok");
            if let Err(e) = source.tombstone(agent, new_epoch).await {
                out.error = format!("tombstone pending: {e}");
            }
            finish(out, &machine)
        }
        Err(e) if e.transport => {
            machine
                .fire("ack_lost")
                .expect("committing accepts ack_lost");
            let settled = resolve_in_doubt(
                source,
                target,
                agent,
                &sid,
                frozen_epoch.unwrap_or(0),
                new_epoch,
            )
            .await;
            let mut out = finish(out, &machine);
            out.error = format!("reply lost: {e}");
            out.needs_attention = settled.state == "in_doubt";
            out.state = settled.state;
            out
        }
        Err(e) => {
            machine
                .fire("activate_refused")
                .expect("committing accepts activate_refused");
            rollback(
                source,
                target,
                agent,
                &sid,
                frozen_epoch,
                e.to_string(),
                out,
                machine,
            )
            .await
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn rollback<S: Endpoint, T: Endpoint>(
    source: &S,
    target: &T,
    agent: &str,
    sid: &str,
    frozen_epoch: Option<u64>,
    reason: String,
    mut out: Outcome,
    mut machine: Machine,
) -> Outcome {
    out.error = reason;
    let _ = target.abort(sid).await; // best effort: the target may be gone
    let resumed = match frozen_epoch {
        Some(epoch) => source.unfreeze(agent, epoch).await,
        None => Ok(()),
    };
    match resumed {
        Ok(()) => {
            machine
                .fire("rollback_ok")
                .expect("rolling back accepts rollback_ok");
        }
        Err(e) => {
            out.error
                .push_str(&format!("; source could not resume: {e}"));
            out.needs_attention = true;
        }
    }
    finish(out, &machine)
}

/// Settle a move whose last reply was lost by asking who holds the epoch.
/// Stateless: it can run after a crash, from either side.
pub async fn resolve_in_doubt<S: Endpoint, T: Endpoint>(
    source: &S,
    target: &T,
    agent: &str,
    sid: &str,
    old_epoch: u64,
    new_epoch: u64,
) -> Outcome {
    let mut out = Outcome {
        session_id: sid.into(),
        state: "in_doubt".into(),
        epoch: new_epoch,
        ..Default::default()
    };
    let holder = match target.holder(agent).await {
        Ok(h) => h,
        Err(e) => {
            out.error = format!("target unreachable: {e}");
            return out;
        }
    };
    if holder == Some((target.node_id().to_string(), new_epoch)) {
        let _ = source.tombstone(agent, new_epoch).await;
        out.state = "committed".into();
        return out;
    }
    let undone = async {
        target.abort(sid).await?;
        source.unfreeze(agent, old_epoch).await
    }
    .await;
    match undone {
        Ok(()) => out.state = "rolled_back".into(),
        Err(e) => {
            out.error = format!("rollback incomplete: {e}");
            out.state = "rolling_back".into();
            out.needs_attention = true;
        }
    }
    out
}

/// Copy an agent and send the copy elsewhere, with another harness if asked: the
/// fall-back when a teleport cannot happen (the target lacks the harness, the link is
/// unreliable) and the way to try a thread with another model without risking it.
/// The original keeps running where it is, whatever happens to the copy.
pub async fn clone_to<T: Endpoint>(
    source: &crate::host::CapsuleHost,
    target: &T,
    agent: &str,
    new_thread: &str,
    harness: Option<&str>,
) -> Outcome {
    let staging = source.root.join("new").join(new_thread);
    let _ = std::fs::remove_dir_all(&staging);
    let made = (|| -> Result<String, String> {
        crate::capsule::fork::fork(&source.agents.join(agent), &staging, new_thread, None)
            .map_err(|e| e.to_string())?;
        if let Some(h) = harness {
            let capsule = crate::capsule::io::load(&staging).map_err(|e| e.to_string())?;
            let mut manifest = capsule.manifest;
            manifest.set_label("harness", h);
            manifest.set_label("model", "");
            crate::capsule::io::seal(&staging, manifest, &capsule.timeline)
                .map_err(|e| e.to_string())?;
        }
        let id = source.adopt(&staging).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_dir_all(&staging);
        Ok(id)
    })();
    match made {
        Ok(id) => teleport(source, target, &id, None).await,
        Err(e) => Outcome {
            state: "rolled_back".into(),
            error: format!("clone failed: {e}"),
            ..Default::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_refuses_impossible_moves() {
        let mut m = Machine::default();
        assert!(m.fire("activate_ok").is_err());
        for e in [
            "freeze_ok",
            "offer_ok",
            "transfer_ok",
            "verify_ok",
            "prepare_ok",
        ] {
            m.fire(e).unwrap();
        }
        assert_eq!(m.state, State::Ready);
        m.fire("commit_sent").unwrap();
        assert!(m.fire("fail").is_err(), "past the point of no return");
    }
}
