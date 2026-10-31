//! The local engine's library, set from Inari's interface with the same sheets as OpenFox's:
//! its agents, skills, commands and workflows (OpenFox's own are read only, duplicated to be
//! changed), its MCP servers, its tools. The person's are files in the engine's folder, in
//! OpenFox's formats.

use super::{defs, tools, LocalEngine};
use crate::capsule::io;
use crate::openfox_library::Entry;
use serde_json::{json, Value};
use std::path::PathBuf;

/// The tools a sub-agent is never given.
const TOP_LEVEL: [&str; 3] = ["call_sub_agent", "ask_user", "step_done"];

fn id_like(x: &str) -> bool {
    !x.is_empty()
        && x.len() <= 128
        && x.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c))
        && !x.starts_with('.')
}

fn entry(id: &str, name: &str, description: &str, scope: &str, subagent: bool) -> Entry {
    Entry {
        id: id.into(),
        name: if name.is_empty() {
            id.into()
        } else {
            name.into()
        },
        description: description.into(),
        scope: scope.into(),
        enabled: None,
        read_only: scope == "builtin",
        subagent,
    }
}

impl LocalEngine {
    /// Where the person's item is kept.
    fn item_path(&self, kind: &str, id: &str) -> PathBuf {
        match kind {
            "agents" => self.dir.join("agents").join(format!("{id}.agent.md")),
            "commands" => self.dir.join("commands").join(format!("{id}.command.md")),
            "skills" => self.dir.join("skills").join(id).join("SKILL.md"),
            _ => self
                .dir
                .join("workflows")
                .join(format!("{id}.workflow.json")),
        }
    }

    fn workflow_json(&self, id: &str) -> Option<(Value, String)> {
        defs::workflows(&self.dir, None)
            .into_iter()
            .find(|(v, _)| v.pointer("/metadata/id").and_then(Value::as_str) == Some(id))
    }

    /// Read part of the library, as OpenFox's `library` gives it.
    pub async fn library(&self, kind: &str, id: &str) -> Result<Value, String> {
        let disabled = self.settings().disabled_skills;
        match (kind, id.is_empty()) {
            ("agents", true) => Ok(json!({"items": defs::agents(&self.dir, None).iter()
                .map(|a| entry(&a.id, &a.name, &a.description, &a.scope, a.subagent)).collect::<Vec<_>>()})),
            ("commands", true) => Ok(json!({"items": defs::commands(&self.dir, None).iter()
                .map(|c| entry(&c.id, &c.name, "", &c.scope, false)).collect::<Vec<_>>()})),
            ("skills", true) => Ok(json!({"items": defs::skills(&self.dir, None).iter()
                .map(|s| Entry { enabled: Some(!disabled.contains(&s.id)), ..entry(&s.id, &s.name, &s.description, &s.scope, false) })
                .collect::<Vec<_>>()})),
            ("workflows", true) => Ok(json!({"items": self.workflow_list(None).iter()
                .map(|w| entry(&w.id, &w.name, &w.description, w.scope.as_deref().unwrap_or("user"), false)).collect::<Vec<_>>()})),
            ("workflows", false) => {
                let (w, _) = self.workflow_json(id).ok_or_else(|| format!("no workflow {id:?}"))?;
                Ok(json!({"id": id, "json": serde_json::to_string_pretty(&w).expect("serializes")}))
            }
            ("agents", false) => defs::agents(&self.dir, None)
                .into_iter()
                .find(|a| a.id == id)
                .map(|a| json!({"id": id, "name": a.name, "description": a.description, "prompt": a.prompt}))
                .ok_or_else(|| format!("no agent {id:?}")),
            ("commands", false) => defs::commands(&self.dir, None)
                .into_iter()
                .find(|c| c.id == id)
                .map(|c| json!({"id": id, "name": c.name, "description": "", "prompt": c.prompt}))
                .ok_or_else(|| format!("no command {id:?}")),
            ("skills", false) => defs::skills(&self.dir, None)
                .into_iter()
                .find(|s| s.id == id)
                .map(|s| json!({"id": id, "name": s.name, "description": s.description, "prompt": s.prompt}))
                .ok_or_else(|| format!("no skill {id:?}")),
            ("mcp", _) => Ok(self.mcp_view().await),
            ("tools", _) => {
                self.mcp_ready().await;
                let mut out: Vec<Value> = tools::BUILT_IN
                    .iter()
                    .map(|t| json!({"name": t, "actions": [], "alwaysAllowed": false,
                        "topLevelOnly": TOP_LEVEL.contains(t),
                        "isMcp": false}))
                    .collect();
                for name in self.mcp_tool_names() {
                    let server = self
                        .mcp_servers()
                        .into_iter()
                        .find(|s| name.starts_with(&format!("{}_", s.name.replace('.', "_"))))
                        .map(|s| s.name);
                    out.push(json!({"name": name, "actions": [], "alwaysAllowed": false,
                        "topLevelOnly": false, "isMcp": true, "mcpServer": server}));
                }
                Ok(json!({"tools": out}))
            }
            (other, _) => Err(format!("the local engine keeps no {other:?}")),
        }
    }

    /// Change the library; the detail says what came of a test.
    pub async fn change(&self, action: &str, p: &Value) -> Result<String, String> {
        if action.starts_with("mcp_") {
            return self.mcp_change(action, p).await;
        }
        let text = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let kind = text("of");
        let id = text("id");
        if action != "setting"
            && !["agents", "skills", "commands", "workflows"].contains(&kind.as_str())
            && action != "skill_toggle"
        {
            return Err(format!("the local engine keeps no {kind:?}"));
        }
        if !id_like(&id) && action != "setting" {
            return Err(format!("unknown item {id:?}"));
        }
        let write = |path: PathBuf, body: String| -> Result<String, String> {
            std::fs::create_dir_all(path.parent().expect("in a folder"))
                .map_err(|e| e.to_string())?;
            io::atomic_write(&path, body.as_bytes()).map_err(|e| e.to_string())?;
            Ok(String::new())
        };
        match action {
            "item_save" => {
                if kind == "workflows" {
                    let mut w: Value = serde_json::from_str(&text("json"))
                        .map_err(|e| format!("the workflow is not JSON: {e}"))?;
                    if !w.get("steps").is_some_and(Value::is_array)
                        || w.get("entryStep").and_then(Value::as_str).is_none()
                    {
                        return Err("a workflow has its entryStep and its steps".into());
                    }
                    w["metadata"]["id"] = json!(id);
                    if w.pointer("/metadata/name")
                        .and_then(Value::as_str)
                        .is_none()
                    {
                        w["metadata"]["name"] = json!(id);
                    }
                    return write(
                        self.item_path(&kind, &id),
                        serde_json::to_string_pretty(&w).expect("serializes"),
                    );
                }
                let prompt = text("prompt");
                if prompt.trim().is_empty() || prompt.len() > 200_000 {
                    return Err(
                        "its prompt is what it does: not empty, 200,000 characters at most".into(),
                    );
                }
                let (name, description) = (
                    text("name").trim().to_string(),
                    text("description").trim().to_string(),
                );
                let body = match kind.as_str() {
                    "agents" => {
                        // the tools and the kind of agent stay what they were
                        let was = defs::agents(&self.dir, None)
                            .into_iter()
                            .find(|a| a.id == id)
                            .unwrap_or_default();
                        defs::agent_file(&defs::AgentDef {
                            id: id.clone(),
                            name,
                            description,
                            prompt,
                            scope: "user".into(),
                            ..was
                        })
                    }
                    "commands" => {
                        let was = defs::commands(&self.dir, None)
                            .into_iter()
                            .find(|c| c.id == id);
                        defs::command_file(&defs::CommandDef {
                            id: id.clone(),
                            name,
                            agent_mode: was.map(|c| c.agent_mode).unwrap_or_default(),
                            prompt,
                            scope: "user".into(),
                        })
                    }
                    _ => defs::skill_file(&defs::SkillDef {
                        id: id.clone(),
                        name,
                        description,
                        prompt,
                        scope: "user".into(),
                    }),
                };
                write(self.item_path(&kind, &id), body)
            }
            "item_duplicate" => {
                // the copy is the person's, under a new id
                let mut n = 2;
                let new = loop {
                    let candidate = format!("{id}-{n}");
                    if !self.item_path(&kind, &candidate).exists() {
                        break candidate;
                    }
                    n += 1;
                };
                match kind.as_str() {
                    "agents" => {
                        let a = defs::agents(&self.dir, None)
                            .into_iter()
                            .find(|a| a.id == id)
                            .ok_or("no such agent")?;
                        write(
                            self.item_path(&kind, &new),
                            defs::agent_file(&defs::AgentDef {
                                id: new.clone(),
                                name: format!("{} (copy)", a.name),
                                ..a
                            }),
                        )
                    }
                    "commands" => {
                        let c = defs::commands(&self.dir, None)
                            .into_iter()
                            .find(|c| c.id == id)
                            .ok_or("no such command")?;
                        write(
                            self.item_path(&kind, &new),
                            defs::command_file(&defs::CommandDef {
                                id: new.clone(),
                                ..c
                            }),
                        )
                    }
                    "skills" => {
                        let s = defs::skills(&self.dir, None)
                            .into_iter()
                            .find(|s| s.id == id)
                            .ok_or("no such skill")?;
                        write(
                            self.item_path(&kind, &new),
                            defs::skill_file(&defs::SkillDef {
                                id: new.clone(),
                                ..s
                            }),
                        )
                    }
                    _ => {
                        let (mut w, _) = self.workflow_json(&id).ok_or("no such workflow")?;
                        w["metadata"]["id"] = json!(new);
                        let name = w
                            .pointer("/metadata/name")
                            .and_then(Value::as_str)
                            .unwrap_or(&id)
                            .to_string();
                        w["metadata"]["name"] = json!(format!("{name} (copy)"));
                        write(
                            self.item_path(&kind, &new),
                            serde_json::to_string_pretty(&w).expect("serializes"),
                        )
                    }
                }
            }
            "item_remove" => {
                let path = self.item_path(&kind, &id);
                if !path.exists() {
                    return Err("only your own are removed".into());
                }
                if kind == "skills" {
                    std::fs::remove_dir_all(path.parent().expect("in a folder"))
                        .map_err(|e| e.to_string())?;
                } else {
                    std::fs::remove_file(&path).map_err(|e| e.to_string())?;
                }
                Ok(String::new())
            }
            "skill_toggle" => {
                let mut s = self.settings();
                if s.disabled_skills.contains(&id) {
                    s.disabled_skills.retain(|x| *x != id);
                } else {
                    s.disabled_skills.push(id);
                }
                self.set_settings(s).map(|_| String::new())
            }
            "setting" if text("key") == "global_instructions" => {
                let mut s = self.settings();
                s.instructions = p
                    .get("value")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                self.set_settings(s).map(|_| String::new())
            }
            other => Err(format!("unknown action {other:?}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_library_is_changed_in_the_engines_folder() {
        let dir = tempfile::tempdir().unwrap();
        let e = LocalEngine::new(dir.path().to_path_buf());
        let agents = e.library("agents", "").await.unwrap();
        assert!(agents["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["id"] == "builder" && a["read_only"] == true));
        e.change("item_duplicate", &json!({"of": "agents", "id": "builder"}))
            .await
            .unwrap();
        let mine = e.library("agents", "builder-2").await.unwrap();
        assert!(mine["prompt"].as_str().unwrap().starts_with("# Build Mode"));
        e.change("item_save", &json!({"of": "agents", "id": "builder-2", "name": "Mine", "description": "d", "prompt": "Do it."})).await.unwrap();
        let back = defs::agents(dir.path(), None)
            .into_iter()
            .find(|a| a.id == "builder-2")
            .unwrap();
        assert_eq!(
            (back.name.as_str(), back.prompt.as_str(), back.subagent),
            ("Mine", "Do it.", false)
        );
        assert!(
            back.tools.contains(&"edit_file".to_string()),
            "its tools stay"
        );
        assert!(e
            .change("item_remove", &json!({"of": "agents", "id": "builder"}))
            .await
            .is_err());
        e.change("item_remove", &json!({"of": "agents", "id": "builder-2"}))
            .await
            .unwrap();
        e.change("item_save", &json!({"of": "skills", "id": "tests", "name": "Tests", "description": "how", "prompt": "Run them."})).await.unwrap();
        e.change("skill_toggle", &json!({"id": "tests"}))
            .await
            .unwrap();
        assert_eq!(e.settings().disabled_skills, ["tests"]);
        e.change(
            "item_duplicate",
            &json!({"of": "workflows", "id": "default"}),
        )
        .await
        .unwrap();
        assert_eq!(e.workflow_list(None).len(), 9);
        assert!(e
            .change(
                "item_save",
                &json!({"of": "workflows", "id": "x", "json": "{}"})
            )
            .await
            .is_err());
    }
}
