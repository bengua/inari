//! Typed views of the capsule's JSON files. Unknown fields are kept (`extra`)
//! so a newer writer's additions survive a round-trip through this reader.

use crate::canon::digest_of;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

pub const FORMAT: &str = "ssc/1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    System,
    UserMessage,
    AssistantMessage,
    Thought,
    Decision,
    ToolCall,
    ToolResult,
    ProviderSwitch,
    Checkpoint,
}

impl EventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::System => "system",
            Self::UserMessage => "user_message",
            Self::AssistantMessage => "assistant_message",
            Self::Thought => "thought",
            Self::Decision => "decision",
            Self::ToolCall => "tool_call",
            Self::ToolResult => "tool_result",
            Self::ProviderSwitch => "provider_switch",
            Self::Checkpoint => "checkpoint",
        }
    }
}

/// One immutable journal entry. `prev` chains the events of one author; `lamport`
/// orders events across authors, so merging two journals is a set union.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelineEvent {
    pub id: String,
    pub node_id: String,
    pub node_seq: u64,
    pub lamport: u64,
    pub ts: f64,
    pub kind: EventKind,
    #[serde(default = "empty_object")]
    pub data: Value,
    #[serde(default)]
    pub prev: String,
    #[serde(default)]
    pub hash: String,
}

fn empty_object() -> Value {
    Value::Object(Map::new())
}

impl TimelineEvent {
    /// Everything but `hash`, exactly as the Python reference hashes it.
    pub fn body(&self) -> Value {
        json!({
            "id": self.id, "node_id": self.node_id, "node_seq": self.node_seq,
            "lamport": self.lamport, "ts": self.ts, "kind": self.kind.as_str(),
            "data": self.data, "prev": self.prev,
        })
    }

    pub fn compute_hash(&self) -> String {
        digest_of(&self.body())
    }

    pub fn make(
        node_id: &str,
        node_seq: u64,
        lamport: u64,
        ts: f64,
        kind: EventKind,
        data: Value,
        prev: &str,
    ) -> Self {
        let mut event = Self {
            id: format!("{node_id}:{node_seq}"),
            node_id: node_id.to_string(),
            node_seq,
            lamport,
            ts,
            kind,
            data,
            prev: prev.to_string(),
            hash: String::new(),
        };
        event.hash = event.compute_hash();
        event
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDigest {
    pub sha256: String,
    pub size: u64,
}

/// `manifest.json`. Only the fields this crate reasons about are typed; the rest
/// (lineage, access, labels, title…) travel untouched in `extra`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub capsule_id: String,
    pub thread_id: String,
    #[serde(default)]
    pub timeline_head: String,
    #[serde(default)]
    pub files: BTreeMap<String, FileDigest>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Manifest {
    pub fn labels(&self) -> Map<String, Value> {
        self.extra
            .get("labels")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
    }

    pub fn set_label(&mut self, key: &str, value: &str) {
        let mut labels = self.labels();
        labels.insert(key.to_string(), Value::String(value.to_string()));
        self.extra.insert("labels".into(), Value::Object(labels));
    }
}

/// `artifacts/snapshot.json`: the workspace as path -> content hash.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub tree: BTreeMap<String, String>,
}

impl Snapshot {
    pub fn digest(&self) -> String {
        digest_of(&json!({"parent": self.parent, "tree": self.tree}))
    }
}
