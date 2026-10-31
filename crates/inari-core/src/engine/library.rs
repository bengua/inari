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

/// Where a skill repository's archive is, and which folder of it holds the skill: a GitHub
/// address (`https://github.com/owner/repo`, `…/tree/<branch>/<folder>`), or an archive address
/// (`…tar.gz`) as is. Its own `path` wins over the one in the address.
fn skill_archive(url: &str, path: &str) -> Result<(String, String), String> {
    let url = url.trim().trim_end_matches('/');
    if url.ends_with(".tar.gz") || url.ends_with(".tgz") {
        return Ok((url.to_string(), path.trim_matches('/').to_string()));
    }
    let rest = url
        .strip_prefix("https://github.com/")
        .ok_or("a skill comes from a GitHub repository (https://github.com/owner/repo) or a .tar.gz address")?;
    let parts: Vec<&str> = rest.split('/').collect();
    if parts.len() < 2
        || parts[..2]
            .iter()
            .any(|p| p.is_empty() || !id_like(p.trim_end_matches(".git")))
    {
        return Err(format!("not a repository address: {url}"));
    }
    let (owner, repo) = (parts[0], parts[1].trim_end_matches(".git"));
    let (reference, inner) = if parts.len() > 3 && parts[2] == "tree" {
        (parts[3].to_string(), parts[4..].join("/"))
    } else {
        ("HEAD".to_string(), String::new())
    };
    let folder = if path.trim().is_empty() {
        inner
    } else {
        path.trim_matches('/').to_string()
    };
    Ok((
        format!("https://codeload.github.com/{owner}/{repo}/tar.gz/{reference}"),
        folder,
    ))
}

/// The licence of a repository, from its LICENSE files: MIT or Apache, or None (Inari takes code
/// under these two only).
fn permissive_licence(root: &std::path::Path) -> Option<&'static str> {
    let mut texts = String::new();
    for e in std::fs::read_dir(root).ok()?.flatten() {
        let n = e.file_name().to_string_lossy().to_uppercase();
        if n.starts_with("LICENSE") || n.starts_with("LICENCE") || n.starts_with("COPYING") {
            texts.push_str(&std::fs::read_to_string(e.path()).unwrap_or_default());
        }
    }
    if texts.contains("Permission is hereby granted, free of charge")
        || texts.contains("MIT License")
    {
        Some("MIT")
    } else if texts.contains("Apache License") {
        Some("Apache-2.0")
    } else {
        None
    }
}

/// The folders holding a SKILL.md under `dir` (three levels down at most).
fn skill_dirs(dir: &std::path::Path, depth: u32, out: &mut Vec<PathBuf>) {
    if dir.join("SKILL.md").is_file() {
        out.push(dir.to_path_buf());
        return;
    }
    if depth == 0 {
        return;
    }
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let ft = match e.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if ft.is_dir() && !e.file_name().to_string_lossy().starts_with('.') {
            skill_dirs(&e.path(), depth - 1, out);
        }
    }
}

/// Copy a skill's regular files (no links), 300 files and 8 MB at most.
fn copy_skill(from: &std::path::Path, to: &std::path::Path) -> Result<(), String> {
    fn walk(
        from: &std::path::Path,
        to: &std::path::Path,
        n: &mut usize,
        bytes: &mut u64,
    ) -> Result<(), String> {
        std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
        for e in std::fs::read_dir(from)
            .map_err(|e| e.to_string())?
            .flatten()
        {
            let ft = e.file_type().map_err(|e| e.to_string())?;
            let name = e.file_name();
            if name.to_string_lossy().starts_with('.') || ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                walk(&e.path(), &to.join(&name), n, bytes)?;
            } else if ft.is_file() {
                *n += 1;
                *bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
                if *n > 300 || *bytes > 8 * 1024 * 1024 {
                    return Err("this skill is too large (300 files, 8 MB at most)".into());
                }
                std::fs::copy(e.path(), to.join(&name)).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
    walk(from, to, &mut 0, &mut 0)
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

    /// Install skills from a repository (`url`, and `path` to one folder; without it, every folder
    /// with a SKILL.md): its archive fetched with curl and opened with tar, as the core does
    /// elsewhere; refused unless its licence is MIT or Apache. Each skill becomes the person's own,
    /// with where it came from kept beside it.
    async fn skill_install_git(&self, p: &Value) -> Result<String, String> {
        let text = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let (archive, folder) = skill_archive(&text("url"), &text("path"))?;
        let work = self
            .dir
            .join("tmp")
            .join(format!("skill-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
        let result = async {
            let tgz = work.join("repo.tar.gz");
            let got = tokio::process::Command::new("curl")
                .args(["-sSLf", "--max-time", "120", "--max-filesize", "52428800", "-o"])
                .arg(&tgz)
                .arg(&archive)
                .kill_on_drop(true)
                .output()
                .await
                .map_err(|e| format!("curl cannot start ({e}): is it installed on this core?"))?;
            if !got.status.success() {
                return Err(format!("{archive}: {}", String::from_utf8_lossy(&got.stderr).trim()));
            }
            let out = work.join("repo");
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            let tar = tokio::process::Command::new("tar")
                .args(["--no-same-owner", "--no-same-permissions", "-xzf"])
                .arg(&tgz)
                .arg("-C")
                .arg(&out)
                .output()
                .await
                .map_err(|e| format!("tar cannot start ({e})"))?;
            if !tar.status.success() {
                return Err(format!("the archive does not open: {}", String::from_utf8_lossy(&tar.stderr).trim()));
            }
            // the archive holds one folder, the repository
            let root = std::fs::read_dir(&out)
                .map_err(|e| e.to_string())?
                .flatten()
                .find(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| e.path())
                .ok_or("the archive is empty")?;
            let licence = match permissive_licence(&root) {
                Some(l) => l,
                None if p.get("force").and_then(Value::as_bool) == Some(true) => "unchecked",
                None => return Err("the repository's licence is neither MIT nor Apache: Inari takes code under these two only".into()),
            };
            let start = if folder.is_empty() { root.clone() } else { root.join(&folder) };
            if folder.split('/').any(|s| s == "..") || !start.starts_with(&root) {
                return Err("a skill's folder is inside the repository".into());
            }
            let mut found = Vec::new();
            skill_dirs(&start, if folder.is_empty() { 3 } else { 0 }, &mut found);
            if found.is_empty() {
                return Err(format!("no SKILL.md in {}", if folder.is_empty() { "the repository" } else { &folder }));
            }
            let mut names = Vec::new();
            for dir in found {
                let id: String = dir
                    .file_name()
                    .map(|n| n.to_string_lossy().to_lowercase())
                    .unwrap_or_default()
                    .chars()
                    .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
                    .collect();
                if !id_like(&id) {
                    continue;
                }
                let dest = self.dir.join("skills").join(&id);
                let _ = std::fs::remove_dir_all(&dest);
                copy_skill(&dir, &dest)?;
                let from = dir.strip_prefix(&root).map(|p| p.display().to_string()).unwrap_or_default();
                std::fs::write(
                    dest.join(".source"),
                    format!("url: {}\npath: {from}\nlicence: {licence}\n", text("url").trim()),
                )
                .map_err(|e| e.to_string())?;
                names.push(id);
            }
            Ok(format!("Installed: {} ({licence})", names.join(", ")))
        }
        .await;
        let _ = std::fs::remove_dir_all(&work);
        result
    }

    /// Change the library; the detail says what came of a test.
    pub async fn change(&self, action: &str, p: &Value) -> Result<String, String> {
        if action.starts_with("mcp_") {
            return self.mcp_change(action, p).await;
        }
        if action == "skill_install_git" {
            return self.skill_install_git(p).await;
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

    #[test]
    fn a_skill_address_is_read() {
        assert_eq!(
            skill_archive("https://github.com/cloudflare/security-audit-skill", "").unwrap(),
            (
                "https://codeload.github.com/cloudflare/security-audit-skill/tar.gz/HEAD".into(),
                String::new()
            )
        );
        assert_eq!(
            skill_archive("https://github.com/Barathraj2387/andrej-karapathy-skills/tree/main/skills/karpathy-guidelines", "").unwrap(),
            ("https://codeload.github.com/Barathraj2387/andrej-karapathy-skills/tar.gz/main".into(), "skills/karpathy-guidelines".into())
        );
        assert!(skill_archive("https://evil.example/x", "").is_err());
        assert!(skill_archive("https://github.com/a", "").is_err());
    }

    /// A repository archive with two skills and a licence, installed from a file:// address
    /// (curl reads it as it reads https); one without a permissive licence is refused.
    #[tokio::test]
    async fn skills_are_installed_from_a_repository_archive() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("src").join("skills-main");
        for (skill, body) in [
            ("karpathy-guidelines", "Think before coding."),
            ("security-audit", "Hunt for bugs."),
        ] {
            let d = repo.join("skills").join(skill);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(
                d.join("SKILL.md"),
                format!("---\nname: {skill}\ndescription: d\n---\n\n{body}\n"),
            )
            .unwrap();
            std::fs::write(d.join("NOTES.md"), "more").unwrap();
        }
        std::fs::write(
            repo.join("LICENSE"),
            "MIT License\n\nPermission is hereby granted, free of charge…",
        )
        .unwrap();
        let tgz = dir.path().join("repo.tar.gz");
        assert!(std::process::Command::new("tar")
            .arg("-czf")
            .arg(&tgz)
            .arg("-C")
            .arg(dir.path().join("src"))
            .arg("skills-main")
            .status()
            .unwrap()
            .success());
        let e = LocalEngine::new(dir.path().join("engine"));
        let said = e
            .change(
                "skill_install_git",
                &json!({"url": format!("file://{}", tgz.display())}),
            )
            .await
            .unwrap();
        assert!(
            said.starts_with("Installed: ") && said.ends_with(" (MIT)"),
            "{said}"
        );
        assert!(
            said.contains("karpathy-guidelines") && said.contains("security-audit"),
            "{said}"
        );
        let ids: Vec<String> = defs::skills(&e.dir, None)
            .into_iter()
            .filter(|s| s.scope == "user")
            .map(|s| s.id)
            .collect();
        assert!(
            ids.contains(&"karpathy-guidelines".to_string())
                && ids.contains(&"security-audit".to_string()),
            "{ids:?}"
        );
        assert!(e.dir.join("skills/security-audit/NOTES.md").is_file());
        assert!(
            std::fs::read_to_string(e.dir.join("skills/security-audit/.source"))
                .unwrap()
                .contains("licence: MIT")
        );
        // one folder only
        std::fs::remove_dir_all(e.dir.join("skills")).unwrap();
        e.change(
            "skill_install_git",
            &json!({"url": format!("file://{}", tgz.display()), "path": "skills/security-audit"}),
        )
        .await
        .unwrap();
        assert_eq!(
            defs::skills(&e.dir, None)
                .into_iter()
                .filter(|s| s.scope == "user")
                .count(),
            1
        );
        // a repository without a permissive licence is refused
        std::fs::write(repo.join("LICENSE"), "All rights reserved.").unwrap();
        assert!(std::process::Command::new("tar")
            .arg("-czf")
            .arg(&tgz)
            .arg("-C")
            .arg(dir.path().join("src"))
            .arg("skills-main")
            .status()
            .unwrap()
            .success());
        let no = e
            .change(
                "skill_install_git",
                &json!({"url": format!("file://{}", tgz.display())}),
            )
            .await
            .unwrap_err();
        assert!(no.contains("neither MIT nor Apache"), "{no}");
    }

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
