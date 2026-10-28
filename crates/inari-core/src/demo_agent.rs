//! A small ACP agent on stdin/stdout, so nothing outside this binary is needed to try Inari or to
//! test it. Two modes:
//!
//! - **demo** (`inari-core-rs demo-agent`): what `inari-core-rs demo` runs. It understands
//!   `write <file>: <text>` (writes the file in its workspace) and `run: <command>` (runs a
//!   read-only command such as `ls`, `cat`, `echo`, `pwd`, `date` or `sleep` at once; anything
//!   else asks permission first, and `sudo` is never run).
//! - **fake** (`inari-core-rs fake-agent`): the scripted agent of the test suite. Words in the
//!   prompt choose what it does (`make`, `perm`, `permstr`, `crash`, `slow`, `orphan`, `long`,
//!   `whereami`); it records what it was asked in `$FAKE_ACP_LOG`, and its sessions next to it.

use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Demo,
    Fake,
}

/// Commands the demo agent runs without asking (each `;`-separated part starts with one).
const READ_ONLY: [&str; 8] = ["ls", "cat", "echo", "pwd", "date", "sleep", "head", "wc"];

struct Agent {
    mode: Mode,
    log: Option<PathBuf>,
    sessions: Vec<String>,
    tools: u32,
    out: std::io::Stdout,
}

impl Agent {
    fn send(&mut self, v: Value) {
        let mut out = self.out.lock();
        let _ = writeln!(out, "{v}");
        let _ = out.flush();
    }

    fn update(&mut self, sid: &str, body: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": sid, "update": body}}));
    }

    fn say(&mut self, sid: &str, text: &str) {
        self.update(sid, json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}}));
    }

    /// One line in the test log, written as the Python fake wrote it (`{"key": value}`).
    fn record(&self, key: &str, value: &Value) {
        if let Some(log) = &self.log {
            let line = format!("{{\"{key}\": {value}}}\n");
            append(log, &line);
        }
    }

    fn sessions_file(&self) -> Option<PathBuf> {
        self.log.as_ref().map(|l| l.with_extension("sessions"))
    }

    fn known(&self) -> Vec<String> {
        match self.sessions_file() {
            Some(f) => std::fs::read_to_string(f)
                .map(|t| t.split_whitespace().map(String::from).collect())
                .unwrap_or_default(),
            None => self.sessions.clone(),
        }
    }

    fn remember(&mut self, sid: &str) {
        let mut all = self.known();
        all.push(sid.to_string());
        match self.sessions_file() {
            Some(f) => {
                let _ = std::fs::write(f, all.join(" "));
            }
            None => self.sessions = all,
        }
    }

    fn handle(&mut self, msg: &Value, input: &mut dyn BufRead) -> bool {
        let Some(method) = msg.get("method").and_then(Value::as_str) else {
            return true; // a reply to a request of ours that nobody waits for
        };
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let p = msg.get("params").cloned().unwrap_or(Value::Null);
        let sid = p
            .get("sessionId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        match method {
            "initialize" => {
                let images = std::env::var("FAKE_ACP_IMAGES").is_ok_and(|v| !v.is_empty());
                self.send(
                    json!({"jsonrpc": "2.0", "id": id, "result": {"protocolVersion": 1,
                    "agentCapabilities": {"promptCapabilities": {"image": images}}}}),
                );
            }
            "session/new" => {
                let sid = format!("s{}", self.known().len() + 1);
                self.remember(&sid);
                let models = match self.mode {
                    Mode::Fake => json!([{"modelId": "fam/alpha"}, {"modelId": "fam/beta"}]),
                    Mode::Demo => json!([{"modelId": "demo/swift"}, {"modelId": "demo/deep"}]),
                };
                let current = models[0]["modelId"].clone();
                self.send(
                    json!({"jsonrpc": "2.0", "id": id, "result": {"sessionId": sid,
                    "models": {"availableModels": models, "currentModelId": current},
                    "modes": {"currentModeId": "default", "availableModes": [
                        {"id": "default", "name": "Default", "description": "Asks first"},
                        {"id": "plan", "name": "Plan", "description": "Read-only"}]}}}),
                );
                self.update(
                    &sid,
                    json!({"sessionUpdate": "available_commands_update", "availableCommands": [
                    {"name": "summarize", "description": "Summarize the thread"},
                    {"name": "review", "description": "Review the changes"}]}),
                );
            }
            "session/set_mode" => {
                let mode = p.get("modeId").cloned().unwrap_or(Value::Null);
                self.record("set_mode", &mode);
                // like Gemini: the change is also announced as message text
                self.say(
                    &sid,
                    &format!("[MODE_UPDATE] {}", mode.as_str().unwrap_or("")),
                );
                self.send(json!({"jsonrpc": "2.0", "id": id, "result": {}}));
            }
            "session/set_model" => {
                self.record(
                    "set_model",
                    &p.get("modelId").cloned().unwrap_or(Value::Null),
                );
                self.send(json!({"jsonrpc": "2.0", "id": id, "result": {}}));
            }
            "session/load" => {
                if self.known().contains(&sid) {
                    self.send(json!({"jsonrpc": "2.0", "id": id, "result": {}}));
                } else {
                    self.send(json!({"jsonrpc": "2.0", "id": id,
                        "error": {"code": -32000, "message": "unknown session"}}));
                }
            }
            "session/prompt" => {
                if !self.prompt(&sid, &p, input) {
                    return false;
                }
                let cost = json!({"amount": 0.02, "currency": "USD"});
                self.update(&sid, json!({"sessionUpdate": "usage_update", "used": 1200, "size": 8000, "cost": cost}));
                self.send(
                    json!({"jsonrpc": "2.0", "id": id, "result": {"stopReason": "end_turn"}}),
                );
            }
            _ => {} // session/cancel and anything else: nothing to answer
        }
        true
    }

    /// One turn. False when the agent must stop (the test word `crash`).
    fn prompt(&mut self, sid: &str, p: &Value, input: &mut dyn BufRead) -> bool {
        let blocks = p
            .get("prompt")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let text = blocks
            .first()
            .and_then(|b| b.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let cwd = std::env::current_dir().unwrap_or_default();
        if let Some(log) = &self.log {
            let kinds: Vec<Value> = blocks.iter().map(|b| b["type"].clone()).collect();
            let line = json!({"cwd": cwd.display().to_string(), "prompt": text, "blocks": kinds});
            append(log, &format!("{line}\n"));
        }
        // the priming quotes earlier words: only the newest message counts
        let last = text
            .rsplit_once("New message from the user:")
            .map_or(text.as_str(), |(_, l)| l)
            .to_string();
        match self.mode {
            Mode::Fake => self.fake_turn(sid, &text, &last, input),
            Mode::Demo => {
                self.demo_turn(sid, last.trim(), &cwd, input);
                true
            }
        }
    }

    fn fake_turn(&mut self, sid: &str, text: &str, last: &str, input: &mut dyn BufRead) -> bool {
        if last.contains("orphan") {
            // a child of its own, as an agent that runs a CLI
            if let (Ok(child), Some(log)) = (Command::new("sleep").arg("300").spawn(), &self.log) {
                let _ = std::fs::write(log.with_extension("child"), child.id().to_string());
            }
        }
        if last.contains("crash") {
            std::process::exit(3);
        }
        if last.contains("slow") {
            std::thread::sleep(Duration::from_secs(10));
        }
        self.update(sid, json!({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "secret thoughts"}}));
        if last.contains("make") {
            let _ = std::fs::write("made.txt", "made by the fake agent");
            self.update(
                sid,
                json!({"sessionUpdate": "tool_call", "toolCallId": "k1", "title": "write made.txt",
                "kind": "edit", "rawInput": {"path": "made.txt"}}),
            );
            self.update(sid, json!({"sessionUpdate": "tool_call_update", "toolCallId": "k1", "status": "completed",
                "content": [{"type": "content", "content": {"type": "text", "text": "ok written"}}]}));
        }
        if last.contains("perm") {
            // "permstr": the request id is a string, as OpenFox's adapter sends it
            let req = if last.contains("permstr") {
                json!("ofx-1")
            } else {
                json!(900)
            };
            let chosen = self.ask(sid, req, "k2", "bash: rm -rf x", input);
            self.say(
                sid,
                &format!("permission result: {}", chosen.as_deref().unwrap_or("None")),
            );
            return true;
        }
        // "long": a reply far longer than any summary, ending on a known word
        let tail = if last.contains("long") {
            vec!["word"; 300].join(" ")
        } else if last.contains("whereami") {
            std::env::var("HOME").unwrap_or_default()
        } else {
            text.lines().last().unwrap_or("").chars().take(60).collect()
        };
        self.say(sid, "fake reply to: ");
        self.say(sid, &tail);
        true
    }

    /// Ask the client's permission for a tool call; the option chosen, if any.
    fn ask(
        &mut self,
        sid: &str,
        req: Value,
        call: &str,
        title: &str,
        input: &mut dyn BufRead,
    ) -> Option<String> {
        self.send(json!({"jsonrpc": "2.0", "id": req, "method": "session/request_permission", "params": {
            "sessionId": sid,
            "toolCall": {"toolCallId": call, "title": title, "kind": "execute", "status": "pending"},
            "options": [{"optionId": "allow", "name": "Allow", "kind": "allow_once"},
                        {"optionId": "deny", "name": "Deny", "kind": "reject_once"}]}}));
        let mut line = String::new();
        input.read_line(&mut line).ok()?;
        let reply: Value = serde_json::from_str(&line).ok()?;
        reply["result"]["outcome"]["optionId"]
            .as_str()
            .map(String::from)
    }

    fn demo_turn(&mut self, sid: &str, last: &str, cwd: &Path, input: &mut dyn BufRead) {
        self.tools += 1;
        let call = format!("t{}", self.tools);
        if let Some((path, body)) = last.strip_prefix("write ").and_then(|r| r.split_once(": ")) {
            let path = path.trim();
            if path.is_empty() || path.starts_with('/') || path.split('/').any(|p| p == "..") {
                self.say(
                    sid,
                    "I only write inside my workspace: give a relative path.",
                );
                return;
            }
            self.update(sid, json!({"sessionUpdate": "tool_call", "toolCallId": call, "title": format!("write_file: {path}"),
                "kind": "edit", "status": "in_progress", "rawInput": {"path": path}}));
            let target = cwd.join(path);
            if let Some(parent) = target.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let result = match std::fs::write(&target, body) {
                Ok(()) => format!("wrote {} bytes", body.len()),
                Err(e) => format!("OSError: {e}"),
            };
            self.update(sid, json!({"sessionUpdate": "tool_call_update", "toolCallId": call, "status": "completed",
                "content": [{"type": "content", "content": {"type": "text", "text": result}}]}));
            self.say(sid, &format!("Done. Output: {result}"));
        } else if let Some(cmd) = last
            .strip_prefix("run:")
            .map(str::trim)
            .filter(|c| !c.is_empty())
        {
            let title = format!("bash: {cmd}");
            if !read_only(cmd) {
                let chosen = self.ask(sid, json!(format!("demo-{call}")), &call, &title, input);
                if chosen.as_deref() != Some("allow") {
                    self.say(sid, "Not run: permission denied.");
                    return;
                }
                if cmd.split_whitespace().next() == Some("sudo") {
                    self.say(
                        sid,
                        "Allowed. The demo never runs sudo, so nothing was changed.",
                    );
                    return;
                }
            }
            self.update(
                sid,
                json!({"sessionUpdate": "tool_call", "toolCallId": call, "title": title,
                "kind": "execute", "status": "in_progress", "rawInput": {"command": cmd}}),
            );
            let output = run(cmd, cwd);
            self.update(sid, json!({"sessionUpdate": "tool_call_update", "toolCallId": call, "status": "completed",
                "content": [{"type": "content", "content": {"type": "text", "text": output}}]}));
            let first = output
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("(nothing)");
            self.say(sid, &format!("Done. Output: {}", first.trim()));
        } else {
            self.say(sid, "I am the demo agent. Ask me `write notes.txt: some text` to create a file, \
                or `run: ls` to run a command in my workspace (anything but a read-only command asks you first).");
        }
    }
}

fn append(path: &Path, line: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = f.write_all(line.as_bytes());
    }
}

/// A command made only of read-only parts (no pipes, redirections or substitutions).
fn read_only(cmd: &str) -> bool {
    cmd.chars()
        .all(|c| c.is_ascii_alphanumeric() || " ._-/;".contains(c))
        && cmd.split(';').all(|part| {
            part.split_whitespace()
                .next()
                .is_some_and(|w| READ_ONLY.contains(&w))
        })
}

/// Run `cmd` with `sh -c` in `cwd`, at most 20 seconds; its output, both streams, at most 4000 bytes.
fn run(cmd: &str, cwd: &Path) -> String {
    let child = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return format!("OSError: {e}"),
    };
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < end => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return "TimeoutError: stopped after 20 s".into();
            }
        }
    }
    let out = child
        .wait_with_output()
        .map(|o| [o.stdout, o.stderr].concat())
        .unwrap_or_default();
    let mut text = String::from_utf8_lossy(&out).into_owned();
    if text.len() > 4000 {
        let mut cut = 4000;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push('…');
    }
    text
}

/// Serve ACP on this process's stdin and stdout until the client closes them.
pub fn serve(mode: Mode) -> i32 {
    let mut agent = Agent {
        mode,
        log: std::env::var_os("FAKE_ACP_LOG")
            .map(PathBuf::from)
            .filter(|_| mode == Mode::Fake),
        sessions: Vec::new(),
        tools: 0,
        out: std::io::stdout(),
    };
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut line = String::new();
    loop {
        line.clear();
        match input.read_line(&mut line) {
            Ok(0) | Err(_) => return 0,
            Ok(_) => {}
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if !agent.handle(&msg, &mut input) {
            return 3;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_read_only_commands_skip_the_question() {
        assert!(read_only("ls"));
        assert!(read_only("sleep 1; ls"));
        assert!(read_only("cat notes.txt"));
        assert!(!read_only("rm -rf x"));
        assert!(!read_only("ls; rm x"));
        assert!(!read_only("ls | sh"));
        assert!(!read_only("echo $(id)"));
        assert!(!read_only("cat x > y"));
        assert!(!read_only("sudo ls"));
    }

    #[test]
    fn a_command_runs_in_its_folder_and_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        assert_eq!(run("ls", dir.path()).trim(), "a.txt");
        assert!(run("yes | head -c 10000", dir.path()).chars().count() <= 4001);
    }
}
