//! The local engine's tools, as OpenFox defines them for the model (names, parameters and
//! descriptions from OpenFox's src/server/tools, MIT licence, see THIRD-PARTY.md): read, write
//! and edit a file, run a command. They work in the thread's folder; a path outside it is the
//! person's to allow, and every action of a thread under a pass is (its supervision).

use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

/// The longest text a tool gives back to the model.
const OUTPUT_CHARS: usize = 30_000;
const READ_LINES: usize = 2000;

/// The tools the engine has (MCP servers' come on top, named `<server>_<tool>`).
pub const BUILT_IN: [&str; 14] = [
    "read_file",
    "write_file",
    "edit_file",
    "run_command",
    "ask_user",
    "session_metadata",
    "call_sub_agent",
    "load_skill",
    "step_done",
    "return_value",
    "background_process",
    "dev_server",
    "project_tasks",
    "kitsune",
];

/// The tools' definitions, for the agent's allowed ones; `subagents` are the ids
/// call_sub_agent offers.
pub fn definitions(allowed: &[String], subagents: &[String]) -> Vec<Value> {
    let all = [
        json!({"type": "function", "function": {"name": "read_file",
            "description": "Read the contents of a file or list directory contents. For text files: returns file content. For directories: returns a tree-formatted listing of entries with file sizes.",
            "parameters": {"type": "object", "properties": {
                "path": {"type": "string", "description": "Path to the file (relative to workdir or absolute)"},
                "offset": {"type": "number", "description": "Line number to start from (1-indexed). Default: 1. Only applies to text files."},
                "limit": {"type": "number", "description": format!("Maximum number of lines to read. Default: {READ_LINES}. Only applies to text files.")}},
                "required": ["path"]}}}),
        json!({"type": "function", "function": {"name": "write_file",
            "description": "Write content to a file. Creates the file if it does not exist, or overwrites if it does. Creates parent directories as needed.",
            "parameters": {"type": "object", "properties": {
                "path": {"type": "string", "description": "Path to the file (relative to workdir or absolute)"},
                "content": {"type": "string", "description": "Content to write to the file"}},
                "required": ["path", "content"]}}}),
        json!({"type": "function", "function": {"name": "edit_file",
            "description": "Replace specific text in a file. Use this for surgical edits. The old_string must match exactly (including whitespace and indentation).",
            "parameters": {"type": "object", "properties": {
                "path": {"type": "string", "description": "Path to the file (relative to workdir or absolute)"},
                "old_string": {"type": "string", "description": "Exact text to find and replace. Must match exactly including whitespace."},
                "new_string": {"type": "string", "description": "Replacement text"},
                "replace_all": {"type": "boolean", "description": "Replace all occurrences (default: false). If false and multiple matches found, the operation fails."}},
                "required": ["path", "old_string", "new_string"]}}}),
        json!({"type": "function", "function": {"name": "run_command",
            "description": "Execute a shell command. Returns stdout, stderr, exit code, and elapsed duration. Does NOT support trailing \"&\" for backgrounding.\nCommands run from your working directory automatically, so prefer relative paths.",
            "parameters": {"type": "object", "properties": {
                "command": {"type": "string", "description": "The command to execute"},
                "cwd": {"type": "string", "description": "Working directory for the command (default: session workdir)"},
                "timeout": {"type": "number", "description": "Timeout in milliseconds (default: 120000)"}},
                "required": ["command"]}}}),
        json!({"type": "function", "function": {"name": "ask_user",
            "description": "Pause execution and ask the user a question. Use this when you need clarification or user input before proceeding.",
            "parameters": {"type": "object", "properties": {
                "question": {"type": "string", "description": "The question to ask the user"},
                "type": {"type": "string", "enum": ["text", "confirm", "choice"], "description": "Type of question (text, confirm, or choice)"},
                "options": {"type": "array", "description": "Options for choice-type questions. Each entry may be a plain string or an object {value, label, description?}.",
                    "items": {"type": "object", "properties": {"value": {"type": "string"}, "label": {"type": "string"}, "description": {"type": "string"}}, "required": ["label"]}}},
                "required": ["question"]}}}),
        super::state::metadata_definition(),
        json!({"type": "function", "function": {"name": "call_sub_agent",
            "description": format!("Call a sub-agent to perform a specialized task. Available sub-agents: {}. The sub-agent will execute with isolated context and return a text result.", subagents.join(", ")),
            "parameters": {"type": "object", "properties": {
                "subAgentType": {"type": "string", "enum": subagents, "description": "Type of sub-agent to call"},
                "prompt": {"type": "string", "description": "Task description for the sub-agent. Be specific about what you need."}},
                "required": ["subAgentType", "prompt"]}}}),
        json!({"type": "function", "function": {"name": "load_skill",
            "description": "Load a skill's detailed instructions into context. Use this when you need domain-specific knowledge for a task. Call with the skill ID shown in the AVAILABLE SKILLS section.",
            "parameters": {"type": "object", "properties": {
                "skillId": {"type": "string", "description": "ID of the skill to load"}},
                "required": ["skillId"]}}}),
        json!({"type": "function", "function": {"name": "step_done",
            "description": "Signal that you have completed the current workflow step. Call this exactly once when you have finished all work for this step. After calling this, no further tool calls should be made.",
            "parameters": {"type": "object", "properties": {}, "required": []}}}),
        json!({"type": "function", "function": {"name": "return_value",
            "description": "Set the return value for this sub-agent execution. Call this once at the end of your work with a structured summary of your findings. The content will be passed back to the calling agent.",
            "parameters": {"type": "object", "properties": {
                "content": {"type": "string", "description": "The return value content — a structured summary of findings, results, or recommendations"},
                "result": {"type": "string", "description": "Result classification (e.g. \"passed\", \"failed\", \"error\"). Used for workflow routing."}},
                "required": ["content"]}}}),
        json!({"type": "function", "function": {"name": "background_process",
            "description": "Run a long-lived command in the background (a watcher, a server) and manage it: start it, list them, read a process's output, stop it. They run in your working directory.",
            "parameters": {"type": "object", "properties": {
                "action": {"type": "string", "enum": ["start", "list", "logs", "stop"], "description": "The action to perform"},
                "command": {"type": "string", "description": "The command to start (for: start)"},
                "name": {"type": "string", "description": "A short name for it (for: start)"},
                "id": {"type": "string", "description": "The process id (for: logs, stop)"}},
                "required": ["action"]}}}),
        json!({"type": "function", "function": {"name": "dev_server",
            "description": "Manage the project's dev server, configured in .openfox/dev.json ({command, url, hotReload}): see its status, start, stop or restart it, read its logs.",
            "parameters": {"type": "object", "properties": {
                "action": {"type": "string", "enum": ["status", "start", "stop", "restart", "logs"], "description": "The action to perform"}},
                "required": ["action"]}}}),
        json!({"type": "function", "function": {"name": "project_tasks",
            "description": "The project's task board: list tasks, add one, move one to todo, in_progress or done.",
            "parameters": {"type": "object", "properties": {
                "action": {"type": "string", "enum": ["list", "add", "move"], "description": "The action to perform"},
                "prompt": {"type": "string", "description": "What the task is (for: add)"},
                "taskId": {"type": "string", "description": "The task (for: move)"},
                "to": {"type": "string", "enum": ["todo", "in_progress", "done"], "description": "Where it goes (for: move)"},
                "status": {"type": "string", "description": "Only the tasks in this column (for: list)"}},
                "required": ["action"]}}}),
        super::brain::definition(),
    ];
    all.into_iter()
        .filter(|d| {
            d.pointer("/function/name")
                .and_then(Value::as_str)
                .is_some_and(|n| allowed.iter().any(|a| a == n))
        })
        .collect()
}

/// A path the model gave, resolved against the folder; whether it stays inside it.
pub fn resolve(workdir: &Path, given: &str) -> (PathBuf, bool) {
    let given = given.trim().trim_start_matches('@');
    let raw = if Path::new(given).is_absolute() {
        PathBuf::from(given)
    } else {
        workdir.join(given)
    };
    // `..` resolved by hand: the file may not exist yet
    let mut out = PathBuf::new();
    for c in raw.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    let base = std::fs::canonicalize(workdir).unwrap_or_else(|_| workdir.to_path_buf());
    let real = out
        .parent()
        .and_then(|p| std::fs::canonicalize(p).ok())
        .map(|p| p.join(out.file_name().unwrap_or_default()))
        .unwrap_or_else(|| out.clone());
    let inside = out.starts_with(workdir) && (real.starts_with(&base) || !out.exists());
    (out, inside)
}

fn clip(text: &str) -> String {
    if text.chars().count() <= OUTPUT_CHARS {
        return text.to_string();
    }
    let tail: String = text
        .chars()
        .rev()
        .take(OUTPUT_CHARS)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("[… the first part is cut]\n{tail}")
}

/// Read a file (numbered lines) or list a folder.
pub fn read_file(path: &Path, args: &Value) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.is_dir() {
        let mut lines = Vec::new();
        list(path, 0, &mut lines);
        if lines.is_empty() {
            return Ok("(empty directory)".into());
        }
        return Ok(clip(&lines.join("\n")));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let Ok(text) = String::from_utf8(bytes.clone()) else {
        return Ok(format!("(a binary file, {} bytes)", bytes.len()));
    };
    let offset = args
        .get("offset")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1) as usize;
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map(|l| l as usize)
        .unwrap_or(READ_LINES)
        .min(READ_LINES);
    let total = text.lines().count();
    let mut out: Vec<String> = text
        .lines()
        .enumerate()
        .skip(offset - 1)
        .take(limit)
        .map(|(i, l)| format!("{:>6}\t{l}", i + 1))
        .collect();
    if offset - 1 + out.len() < total {
        out.push(format!(
            "(lines {offset}-{} of {total}: read on with offset {})",
            offset - 1 + out.len(),
            offset + out.len()
        ));
    }
    Ok(clip(&out.join("\n")))
}

fn list(dir: &Path, depth: usize, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().to_string();
        if out.len() > 400 {
            out.push("…".into());
            return;
        }
        let indent = "  ".repeat(depth);
        let path = e.path();
        if path.is_dir() {
            out.push(format!("{indent}{name}/"));
            if depth < 2 && ![".git", "node_modules", "target"].contains(&name.as_str()) {
                list(&path, depth + 1, out);
            }
        } else {
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            out.push(format!("{indent}{name} ({size} bytes)"));
        }
    }
}

pub fn write_file(path: &Path, args: &Value) -> Result<String, String> {
    let content = args
        .get("content")
        .and_then(Value::as_str)
        .ok_or("write_file needs content")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, content).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(format!(
        "Wrote {} bytes to {}",
        content.len(),
        path.display()
    ))
}

pub fn edit_file(path: &Path, args: &Value) -> Result<String, String> {
    let old = args
        .get("old_string")
        .and_then(Value::as_str)
        .ok_or("edit_file needs old_string")?;
    let new = args
        .get("new_string")
        .and_then(Value::as_str)
        .ok_or("edit_file needs new_string")?;
    if old.is_empty() {
        return Err("old_string is empty: use write_file to create a file".into());
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let found = text.matches(old).count();
    let all = args
        .get("replace_all")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let edited = match found {
        0 => return Err(format!("old_string was not found in {}: read the file again and copy the text exactly", path.display())),
        1 => text.replacen(old, new, 1),
        n if all => {
            let _ = n;
            text.replace(old, new)
        }
        n => {
            return Err(format!(
                "old_string is found {n} times in {}: add surrounding lines to make it unique, or set replace_all",
                path.display()
            ))
        }
    };
    std::fs::write(path, edited).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(format!(
        "Edited {} ({} replacement{})",
        path.display(),
        if all { found } else { 1 },
        if all && found > 1 { "s" } else { "" }
    ))
}

/// Run a command in `cwd`; the whole process group ends at the deadline. Under a pass the
/// environment is a bare PATH; otherwise the core's own, as for any agent it runs.
pub async fn run_command(
    cwd: &Path,
    command: &str,
    timeout: Duration,
    bare: bool,
) -> (bool, String) {
    let mut cmd = Command::new("/bin/sh");
    cmd.args(["-c", command])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true);
    if bare {
        cmd.env_clear()
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .env("HOME", cwd);
    }
    let started = std::time::Instant::now();
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return (false, format!("cannot start the shell: {e}")),
    };
    let pid = child.id();
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(out)) => {
            let code = out.status.code().unwrap_or(-1);
            let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
            let err = String::from_utf8_lossy(&out.stderr);
            if !err.is_empty() {
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&err);
            }
            let secs = started.elapsed().as_secs_f64();
            (
                code == 0,
                clip(&format!(
                    "exit code {code} ({secs:.1} s)\n{}",
                    if text.is_empty() {
                        "(no output)"
                    } else {
                        &text
                    }
                )),
            )
        }
        Ok(Err(e)) => (false, e.to_string()),
        Err(_) => {
            if let Some(pid) = pid {
                // SAFETY: signalling the process group this function created.
                unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
            }
            (false, format!("timed out after {} s", timeout.as_secs()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_stays_in_the_folder_or_says_it_leaves() {
        let dir = tempfile::tempdir().unwrap();
        let w = dir.path();
        assert!(resolve(w, "src/a.rs").1);
        assert!(resolve(w, "@src/a.rs").1);
        assert!(!resolve(w, "../outside.txt").1);
        assert!(!resolve(w, "/etc/passwd").1);
        assert!(resolve(w, &w.join("x").display().to_string()).1);
    }

    #[test]
    fn an_edit_must_find_its_text_once() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.txt");
        std::fs::write(&f, "one two two").unwrap();
        assert!(edit_file(&f, &json!({"old_string": "three", "new_string": "x"})).is_err());
        assert!(edit_file(&f, &json!({"old_string": "two", "new_string": "x"})).is_err());
        edit_file(&f, &json!({"old_string": "one", "new_string": "1"})).unwrap();
        edit_file(
            &f,
            &json!({"old_string": "two", "new_string": "2", "replace_all": true}),
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "1 2 2");
        let read = read_file(&f, &json!({})).unwrap();
        assert_eq!(read, "     1\t1 2 2");
    }
}
