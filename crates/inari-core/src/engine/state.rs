//! A thread's own state for the local engine: its structured data (criteria, todos, review
//! findings), how bold it may be, the workflow it runs. Kept in the thread's journal as a
//! `thought` event (never shown in the conversation), so it travels with the thread from one
//! core to another; the last one is the state.

use crate::capsule::{io, EventKind};
use crate::openfox::Item;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub description: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
}

/// Where a workflow is in a thread.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Flow {
    pub workflow: String,
    pub name: String,
    /// running, waiting (a step for the person), completed, cancelled, blocked.
    pub status: String,
    pub step: String,
    #[serde(default)]
    pub step_name: String,
    #[serde(default)]
    pub params: BTreeMap<String, String>,
    #[serde(default)]
    pub output: BTreeMap<String, String>,
    #[serde(default)]
    pub iterations: u32,
    /// The step results so far, by step id (`step_result` transitions read them).
    #[serde(default)]
    pub results: BTreeMap<String, String>,
    #[serde(default)]
    pub color: Option<String>,
    /// Only the steps of this sub-group (and those its tagged transitions lead to).
    #[serde(default)]
    pub sub_group: Option<String>,
    #[serde(default)]
    pub active_steps: Vec<String>,
    #[serde(default)]
    pub active_groups: Vec<String>,
    /// The agent steps already entered once (entered again, they get the nudge).
    #[serde(default)]
    pub entered: Vec<String>,
    /// The branches the step for the person offers.
    #[serde(default)]
    pub choices: Vec<crate::openfox::proto::StepChoice>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ThreadState {
    /// Structured data by key: criteria, todos, review_findings…
    #[serde(default)]
    pub metadata: BTreeMap<String, Vec<Entry>>,
    /// normal (paths outside the folder are asked) or dangerous (they are not).
    #[serde(default)]
    pub danger: String,
    #[serde(default)]
    pub next_id: u64,
    #[serde(default)]
    pub compactions: u64,
    #[serde(default)]
    pub flow: Option<Flow>,
    /// The forge and repository the thread's folder comes from or goes to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<GitLink>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GitLink {
    pub forge: String,
    pub repo: String,
    /// The last pull request opened from it.
    #[serde(default)]
    pub pull_request: Option<String>,
}

/// The statuses each known key takes.
pub fn statuses(key: &str) -> Option<&'static [&'static str]> {
    match key {
        "criteria" => Some(&["pending", "in_progress", "completed", "passed", "failed"]),
        "todos" => Some(&["pending", "in_progress", "completed"]),
        "review_findings" => Some(&["open", "resolved", "dismissed"]),
        _ => None,
    }
}

impl ThreadState {
    /// The last state kept in a thread's journal (a new thread's is empty).
    pub fn load(capsule: &Path) -> Self {
        let Ok(c) = io::load(capsule) else {
            return Self::default();
        };
        c.timeline
            .ordered()
            .iter()
            .rev()
            .filter(|e| e.kind == EventKind::Thought)
            .find_map(|e| e.data.get("engine_state"))
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default()
    }

    /// Keep it in the journal (not shown in the conversation).
    pub fn save(&self, capsule: &Path, author: &str) -> Result<(), String> {
        io::append_event(
            capsule,
            author,
            EventKind::Thought,
            json!({"engine_state": self}),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0),
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    }

    pub fn items(&self, key: &str) -> Vec<Item> {
        self.metadata
            .get(key)
            .map(|l| {
                l.iter()
                    .map(|e| Item {
                        id: e.id.clone(),
                        text: e.description.clone(),
                        status: e.status.clone(),
                        reason: Some(e.reason.clone()).filter(|r| !r.is_empty()),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// OpenFox's `session_metadata` tool: get, list, add, update, remove, schema.
    pub fn metadata_tool(&mut self, args: &Value) -> Result<String, String> {
        let s = |k: &str| {
            args.get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string()
        };
        let action = s("action");
        let key = s("key");
        let id = match args.get("id") {
            Some(Value::Number(n)) => n.to_string(),
            _ => s("id"),
        };
        let need_key = || -> Result<(), String> {
            if key.is_empty() || key.len() > 64 {
                Err(format!(
                    "{action} needs a key (criteria, todos, review_findings…)"
                ))
            } else {
                Ok(())
            }
        };
        let check_status = |status: &str| -> Result<(), String> {
            match statuses(&key) {
                Some(known) if !known.contains(&status) => Err(format!(
                    "{status:?} is not a status of {key}: {}",
                    known.join(", ")
                )),
                _ => Ok(()),
            }
        };
        match action.as_str() {
            "list" => Ok(if self.metadata.is_empty() {
                "No session metadata yet.".into()
            } else {
                self.metadata
                    .iter()
                    .map(|(k, v)| format!("{k}: {} item(s)", v.len()))
                    .collect::<Vec<_>>()
                    .join("\n")
            }),
            "get" => {
                need_key()?;
                let items = self.metadata.get(&key).cloned().unwrap_or_default();
                Ok(serde_json::to_string_pretty(&items).expect("serializes"))
            }
            "schema" => {
                need_key()?;
                Ok(match statuses(&key) {
                    Some(st) => format!(
                        "{key}: items of {{id (auto), description, status}}; status is one of {}",
                        st.join(", ")
                    ),
                    None => format!("{key}: items of {{id (auto), description, status}}"),
                })
            }
            "add" => {
                need_key()?;
                let description = s("description");
                if description.is_empty() {
                    return Err("add needs a description".into());
                }
                let status = Some(s("status"))
                    .filter(|x| !x.is_empty())
                    .unwrap_or_else(|| {
                        if key == "review_findings" {
                            "open"
                        } else {
                            "pending"
                        }
                        .into()
                    });
                check_status(&status)?;
                self.next_id += 1;
                let id = self.next_id.to_string();
                self.metadata.entry(key.clone()).or_default().push(Entry {
                    id: id.clone(),
                    description,
                    status,
                    reason: s("reason"),
                });
                Ok(format!("Added {key} item {id}."))
            }
            "update" => {
                need_key()?;
                let status = s("status");
                if !status.is_empty() {
                    check_status(&status)?;
                }
                let item = self
                    .metadata
                    .get_mut(&key)
                    .and_then(|l| l.iter_mut().find(|e| e.id == id))
                    .ok_or_else(|| {
                        format!("no {key} item {id:?}: use action get to see the ids")
                    })?;
                if !status.is_empty() {
                    item.status = status;
                }
                if args.get("description").is_some() && !s("description").is_empty() {
                    item.description = s("description");
                }
                if args.get("reason").is_some() {
                    item.reason = s("reason");
                }
                Ok(format!("Updated {key} item {id}."))
            }
            "remove" => {
                need_key()?;
                let list = self.metadata.entry(key.clone()).or_default();
                let before = list.len();
                list.retain(|e| e.id != id);
                if list.len() == before {
                    return Err(format!("no {key} item {id:?}"));
                }
                Ok(format!("Removed {key} item {id}."))
            }
            other => Err(format!(
                "unknown action {other:?}: get, list, add, update, remove or schema"
            )),
        }
    }

    /// The criteria as a workflow's prompts show them.
    pub fn criteria_list(&self) -> String {
        let list = self.metadata.get("criteria").cloned().unwrap_or_default();
        if list.is_empty() {
            return "(no criteria)".into();
        }
        list.iter()
            .map(|c| {
                let mark = match c.status.as_str() {
                    "passed" => "[PASSED]",
                    "completed" => "[NEEDS VERIFICATION]",
                    "failed" => "[FAILED]",
                    _ => "[PENDING]",
                };
                let why = if c.reason.is_empty() {
                    String::new()
                } else {
                    format!(" — {}", c.reason)
                };
                format!("- {} {mark} {}{why}", c.id, c.description)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// The tool's definition, as OpenFox gives it to the model.
pub fn metadata_definition() -> Value {
    json!({"type": "function", "function": {"name": "session_metadata",
        "description": "Manage structured session data. Each key holds an array of items with id (auto), description, and status. Known keys: criteria, todos, review_findings. Use action=schema to see the expected shape for a specific key.",
        "parameters": {"type": "object", "properties": {
            "action": {"type": "string", "enum": ["get", "list", "add", "update", "remove", "schema"], "description": "The action to perform"},
            "key": {"type": "string", "description": "Metadata key (required for: get, add, update, remove, schema). Known keys: criteria, todos, review_findings."},
            "id": {"type": "string", "description": "Item ID (required for: update, remove). Accepts numeric ids too — they are normalized to strings."},
            "description": {"type": "string", "description": "Item description (required for: add)"},
            "status": {"type": "string", "description": "Item status (for: add, update). Defaults to \"pending\" on add."},
            "reason": {"type": "string", "description": "Why the status is what it is (a failed criterion's reason, say)"}},
            "required": ["action"]}}})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_metadata_tool_works_as_openfoxs() {
        let mut s = ThreadState::default();
        s.metadata_tool(
            &json!({"action": "add", "key": "criteria", "description": "parses empty input"}),
        )
        .unwrap();
        s.metadata_tool(&json!({"action": "add", "key": "todos", "description": "write the parser", "status": "in_progress"})).unwrap();
        assert!(s
            .metadata_tool(
                &json!({"action": "update", "key": "criteria", "id": 1, "status": "nope"})
            )
            .is_err());
        s.metadata_tool(
            &json!({"action": "update", "key": "criteria", "id": 1, "status": "completed"}),
        )
        .unwrap();
        assert_eq!(s.items("criteria")[0].status, "completed");
        assert!(s
            .criteria_list()
            .contains("[NEEDS VERIFICATION] parses empty input"));
        assert!(s
            .metadata_tool(&json!({"action": "remove", "key": "todos", "id": "9"}))
            .is_err());
        assert_eq!(
            s.metadata_tool(&json!({"action": "list"})).unwrap(),
            "criteria: 1 item(s)\ntodos: 1 item(s)"
        );
    }
}
