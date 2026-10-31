//! What the local engine's agents are made of, in OpenFox's own file formats so a definition
//! written for OpenFox works as is: agents (`<id>.agent.md`), skills (`<id>/SKILL.md`), slash
//! commands (`<id>.command.md`) and workflows (`<id>.workflow.json`). The built-in ones come
//! from OpenFox (MIT, see THIRD-PARTY.md); the person's are kept in the engine's folder
//! (`agents/`, `skills/`, `commands/`, `workflows/`), and a project's in the thread's folder
//! (`.openfox/…`).

use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

/// One agent: how it works, the tools it may use, whether only another agent calls it.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct AgentDef {
    pub id: String,
    pub name: String,
    pub description: String,
    pub subagent: bool,
    pub tools: Vec<String>,
    pub prompt: String,
    /// builtin, user or project.
    pub scope: String,
    /// The model it works on: a tier the settings map (fast, standard, expert…) or
    /// "<provider>/<model>"; empty, the thread's (or its caller's) model.
    pub model: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct SkillDef {
    pub id: String,
    pub name: String,
    pub description: String,
    pub prompt: String,
    pub scope: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct CommandDef {
    pub id: String,
    pub name: String,
    /// The agent the command switches to for its turn, if any.
    pub agent_mode: String,
    pub prompt: String,
    pub scope: String,
}

const BUILTIN_AGENTS: [&str; 15] = [
    include_str!("defaults/builder.agent.md"),
    include_str!("defaults/planner.agent.md"),
    include_str!("defaults/explorer.agent.md"),
    include_str!("defaults/verifier.agent.md"),
    include_str!("defaults/code-reviewer.agent.md"),
    // Kitsune's assistant and its specialists, one per function (see docs/tdah-recherche-fr.md)
    include_str!("defaults/assistant.agent.md"),
    include_str!("defaults/triage.agent.md"),
    include_str!("defaults/organizer.agent.md"),
    include_str!("defaults/shopper.agent.md"),
    include_str!("defaults/tracker.agent.md"),
    include_str!("defaults/librarian.agent.md"),
    include_str!("defaults/summarizer.agent.md"),
    include_str!("defaults/home.agent.md"),
    include_str!("defaults/researcher.agent.md"),
    include_str!("defaults/developer.agent.md"),
];

const BUILTIN_COMMANDS: [&str; 3] = [
    include_str!("defaults/add-criteria.command.md"),
    include_str!("defaults/init.command.md"),
    include_str!("defaults/commit-push.command.md"),
];

pub const BUILTIN_WORKFLOWS: [&str; 8] = [
    include_str!("defaults/default.workflow.json"),
    // the assistant's everyday workflows
    include_str!("defaults/kitsune-inbox.workflow.json"),
    include_str!("defaults/daily-plan.workflow.json"),
    include_str!("defaults/weekly-review.workflow.json"),
    include_str!("defaults/groceries.workflow.json"),
    include_str!("defaults/doc-sort.workflow.json"),
    include_str!("defaults/research-brief.workflow.json"),
    include_str!("defaults/dev-handoff.workflow.json"),
];

/// A Markdown file's front matter (simple `key: value` lines and `- item` lists) and its body.
pub fn front_matter(text: &str) -> (BTreeMap<String, Value>, String) {
    let mut out = BTreeMap::new();
    let text = text.trim_start_matches('\u{feff}');
    let Some(rest) = text.strip_prefix("---") else {
        return (out, text.trim().to_string());
    };
    let Some(end) = rest.find("\n---") else {
        return (out, text.trim().to_string());
    };
    let (head, body) = (&rest[..end], &rest[end + 4..]);
    let mut list_key: Option<String> = None;
    for line in head.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let (Some(key), Some(item)) = (&list_key, line.trim_start().strip_prefix("- ")) {
            if line.starts_with(' ') || line.starts_with('-') {
                if let Some(Value::Array(a)) = out.get_mut(key) {
                    a.push(Value::String(unquote(item)));
                    continue;
                }
            }
        }
        if line.starts_with(' ') {
            continue; // nested keys (a skill's metadata) are not needed
        }
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let (k, v) = (k.trim().to_string(), v.trim());
        if v.is_empty() {
            out.insert(k.clone(), Value::Array(Vec::new()));
            list_key = Some(k);
        } else {
            list_key = None;
            let value = match v {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                _ => Value::String(unquote(v)),
            };
            out.insert(k, value);
        }
    }
    (
        out,
        body.trim_start_matches(['\r', '\n']).trim_end().to_string(),
    )
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2
        && ((v.starts_with('\'') && v.ends_with('\'')) || (v.starts_with('"') && v.ends_with('"')))
    {
        v[1..v.len() - 1].to_string()
    } else {
        v.to_string()
    }
}

fn text(m: &BTreeMap<String, Value>, k: &str) -> String {
    m.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

pub fn parse_agent(raw: &str, scope: &str) -> Option<AgentDef> {
    let (m, body) = front_matter(raw);
    let id = text(&m, "id");
    if id.is_empty() {
        return None;
    }
    Some(AgentDef {
        name: Some(text(&m, "name"))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| id.clone()),
        id,
        description: text(&m, "description"),
        subagent: m.get("subagent").and_then(Value::as_bool).unwrap_or(false),
        tools: m
            .get("allowedTools")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default(),
        prompt: body,
        scope: scope.into(),
        model: text(&m, "model"),
    })
}

pub fn parse_command(raw: &str, scope: &str) -> Option<CommandDef> {
    let (m, body) = front_matter(raw);
    let id = text(&m, "id");
    if id.is_empty() {
        return None;
    }
    Some(CommandDef {
        name: Some(text(&m, "name"))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| id.clone()),
        id,
        agent_mode: text(&m, "agentMode"),
        prompt: body,
        scope: scope.into(),
    })
}

pub fn parse_skill(id: &str, raw: &str, scope: &str) -> SkillDef {
    let (m, body) = front_matter(raw);
    SkillDef {
        id: id.into(),
        name: Some(text(&m, "name"))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| id.into()),
        description: text(&m, "description"),
        prompt: body,
        scope: scope.into(),
    }
}

/// The files of a folder with an ending, sorted.
fn files(dir: &Path, ending: &str) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(ending))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .collect();
    out.sort();
    out
}

/// Later lists override earlier ones by id: OpenFox's, then the person's, then the project's.
fn merged<T: Clone>(lists: Vec<Vec<T>>, id: impl Fn(&T) -> String) -> Vec<T> {
    let mut out: Vec<T> = Vec::new();
    for list in lists {
        for item in list {
            match out.iter().position(|x| id(x) == id(&item)) {
                Some(i) => out[i] = item,
                None => out.push(item),
            }
        }
    }
    out
}

/// Every agent: OpenFox's, the person's (`<engine>/agents`), the project's (`.openfox/agents`).
pub fn agents(engine_dir: &Path, workdir: Option<&Path>) -> Vec<AgentDef> {
    let builtin = BUILTIN_AGENTS
        .iter()
        .filter_map(|raw| parse_agent(raw, "builtin"))
        .collect();
    let user = files(&engine_dir.join("agents"), ".agent.md")
        .iter()
        .filter_map(|raw| parse_agent(raw, "user"))
        .collect();
    let project = workdir
        .map(|w| files(&w.join(".openfox").join("agents"), ".agent.md"))
        .unwrap_or_default()
        .iter()
        .filter_map(|raw| parse_agent(raw, "project"))
        .collect();
    merged(vec![builtin, user, project], |a: &AgentDef| a.id.clone())
}

pub fn commands(engine_dir: &Path, workdir: Option<&Path>) -> Vec<CommandDef> {
    let builtin = BUILTIN_COMMANDS
        .iter()
        .filter_map(|raw| parse_command(raw, "builtin"))
        .collect();
    let user = files(&engine_dir.join("commands"), ".command.md")
        .iter()
        .filter_map(|raw| parse_command(raw, "user"))
        .collect();
    let project = workdir
        .map(|w| files(&w.join(".openfox").join("commands"), ".command.md"))
        .unwrap_or_default()
        .iter()
        .filter_map(|raw| parse_command(raw, "project"))
        .collect();
    merged(vec![builtin, user, project], |c: &CommandDef| c.id.clone())
}

fn skills_in(dir: &Path, scope: &str) -> Vec<SkillDef> {
    let mut out: Vec<SkillDef> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().to_string();
            let raw = std::fs::read_to_string(e.path().join("SKILL.md")).ok()?;
            Some(parse_skill(&id, &raw, scope))
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

pub fn skills(engine_dir: &Path, workdir: Option<&Path>) -> Vec<SkillDef> {
    let user = skills_in(&engine_dir.join("skills"), "user");
    let project = workdir
        .map(|w| skills_in(&w.join(".openfox").join("skills"), "project"))
        .unwrap_or_default();
    merged(vec![user, project], |s: &SkillDef| s.id.clone())
}

/// Every workflow as its JSON: OpenFox's, the person's, the project's.
pub fn workflows(engine_dir: &Path, workdir: Option<&Path>) -> Vec<(Value, String)> {
    let parse = |raw: &str, scope: &str| -> Option<(Value, String)> {
        let v: Value = serde_json::from_str(raw).ok()?;
        v.pointer("/metadata/id")?.as_str()?;
        Some((v, scope.to_string()))
    };
    let builtin = BUILTIN_WORKFLOWS
        .iter()
        .filter_map(|raw| parse(raw, "builtin"))
        .collect();
    let user = files(&engine_dir.join("workflows"), ".workflow.json")
        .iter()
        .filter_map(|raw| parse(raw, "user"))
        .collect();
    let project = workdir
        .map(|w| files(&w.join(".openfox").join("workflows"), ".workflow.json"))
        .unwrap_or_default()
        .iter()
        .filter_map(|raw| parse(raw, "project"))
        .collect();
    merged(vec![builtin, user, project], |(v, _): &(Value, String)| {
        v.pointer("/metadata/id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    })
}

/// A definition written back in OpenFox's format.
pub fn agent_file(a: &AgentDef) -> String {
    let tools: String = a.tools.iter().map(|t| format!("\n  - {t}")).collect();
    let model = if a.model.is_empty() {
        String::new()
    } else {
        format!("model: {}\n", one_line(&a.model))
    };
    format!(
        "---\nid: {}\nname: {}\ndescription: {}\nsubagent: {}\n{model}allowedTools:{tools}\n---\n\n{}\n",
        a.id,
        one_line(&a.name),
        one_line(&a.description),
        a.subagent,
        a.prompt.trim()
    )
}

pub fn command_file(c: &CommandDef) -> String {
    let mode = if c.agent_mode.is_empty() {
        String::new()
    } else {
        format!("agentMode: {}\n", c.agent_mode)
    };
    format!(
        "---\nid: {}\nname: {}\n{mode}---\n\n{}\n",
        c.id,
        one_line(&c.name),
        c.prompt.trim()
    )
}

pub fn skill_file(s: &SkillDef) -> String {
    format!(
        "---\nname: {}\ndescription: {}\n---\n\n{}\n",
        one_line(&s.name),
        one_line(&s.description),
        s.prompt.trim()
    )
}

fn one_line(s: &str) -> String {
    let s = s.replace(['\n', '\r'], " ");
    if s.contains(':') || s.starts_with(['\'', '"', '-', '#']) {
        format!("'{}'", s.replace('\'', "''"))
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A branch a step for the person offers is sent back as an id (Launch::problem): a space in
    /// it made the choice fail silently once (dev-handoff's "Hand off").
    #[test]
    fn every_built_in_workflow_offers_choices_the_node_takes() {
        for raw in BUILTIN_WORKFLOWS {
            let w: Value = serde_json::from_str(raw).expect("a built-in workflow is JSON");
            for step in w["steps"].as_array().expect("steps") {
                for x in step["transitions"].as_array().into_iter().flatten() {
                    if let Some(r) = x.pointer("/when/result").and_then(Value::as_str) {
                        assert!(
                            r.chars()
                                .all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c)),
                            "{}: the choice {r:?} is not an id",
                            w["metadata"]["id"]
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn openfoxs_agents_read_as_they_are() {
        let all = agents(Path::new("/nonexistent"), None);
        let ids: Vec<&str> = all.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "builder",
                "planner",
                "explorer",
                "verifier",
                "code_reviewer",
                "assistant",
                "triage",
                "organizer",
                "shopper",
                "tracker",
                "librarian",
                "summarizer",
                "home",
                "researcher",
                "developer"
            ]
        );
        // the assistant's specialists work on the tier their job needs
        let tier = |id: &str| all.iter().find(|a| a.id == id).unwrap().model.clone();
        assert_eq!(
            (tier("triage"), tier("organizer")),
            ("fast".into(), "standard".into())
        );
        let builder = &all[0];
        assert!(!builder.subagent && builder.tools.contains(&"edit_file".to_string()));
        assert!(builder.prompt.starts_with("# Build Mode"));
        assert!(all[2].subagent);
    }

    #[test]
    fn a_written_definition_reads_back() {
        let a = AgentDef {
            id: "reviewer".into(),
            name: "Reviewer: strict".into(),
            description: "reviews".into(),
            subagent: true,
            tools: vec!["read_file".into()],
            prompt: "Be strict.".into(),
            scope: "user".into(),
            model: "fast".into(),
        };
        let back = parse_agent(&agent_file(&a), "user").unwrap();
        assert_eq!(back, a);
        let c = commands(Path::new("/nonexistent"), None);
        assert!(c
            .iter()
            .any(|c| c.id == "init" && c.agent_mode == "builder"));
        assert_eq!(workflows(Path::new("/nonexistent"), None).len(), 8);
    }
}
