//! Inari's local engine: the OpenFox core inside the core. A thread's turn is a loop of the
//! model (an OpenAI-compatible server on the machine or the private network: vLLM, Ollama,
//! llama.cpp…) and its tools (read, write, edit a file, run a command), in the thread's
//! folder, journaled as any harness's. No other process than inari-core-rs and the model
//! server. Its settings (providers, model, agent, limits) are the interface's, kept in
//! `<data_dir>/engine/<harness>/`; a provider's key is kept apart (`keys.json`, mode 600) and
//! never shown.

pub mod llm;
pub mod prompts;
pub mod tools;

use crate::agent::{acp_update, flush_answer, prepare_model, Probe, RunSummary, Turn};
use crate::capsule::{io, EventKind};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// A model server the engine may use.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Provider {
    pub id: String,
    pub name: String,
    /// Its OpenAI-compatible base address (http://127.0.0.1:8000/v1).
    pub url: String,
}

/// The engine's settings, set from the interface.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub providers: Vec<Provider>,
    /// The model a thread uses when it chose none: "<provider>/<model>".
    #[serde(default)]
    pub model: String,
    /// The agent a thread works as when it chose none (builder, planner).
    #[serde(default = "builder")]
    pub mode: String,
    /// The model's context, in tokens: past 80 % the conversation is summarised.
    #[serde(default = "window")]
    pub context_window: u64,
    /// How many model calls a turn may make.
    #[serde(default = "steps")]
    pub max_steps: u32,
    /// How long a turn may run, in seconds.
    #[serde(default = "limit")]
    pub turn_timeout: u64,
    #[serde(default)]
    pub temperature: Option<f64>,
    /// Instructions added to every thread's system prompt.
    #[serde(default)]
    pub instructions: String,
}

fn builder() -> String {
    "builder".into()
}
fn window() -> u64 {
    32_768
}
fn steps() -> u32 {
    100
}
fn limit() -> u64 {
    3600
}

impl Default for Settings {
    fn default() -> Self {
        serde_json::from_value(json!({})).expect("defaults")
    }
}

fn id_like(x: &str) -> bool {
    !x.is_empty()
        && x.len() <= 64
        && x.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c))
}

impl Settings {
    pub fn problem(&self) -> Option<String> {
        for p in &self.providers {
            if !id_like(&p.id) {
                return Some(format!(
                    "a provider's id is letters, digits, - _ and . only: {:?}",
                    p.id
                ));
            }
            if !p.url.starts_with("http://") {
                return Some(format!(
                    "{}: the local engine speaks plain http (http://host:port/v1)",
                    p.name
                ));
            }
        }
        if !prompts::AGENTS.iter().any(|a| a.id == self.mode) {
            return Some(format!("unknown agent {:?}", self.mode));
        }
        if !(2048..=2_000_000).contains(&self.context_window) {
            return Some("a context window is between 2,048 and 2,000,000 tokens".into());
        }
        if !(1..=1000).contains(&self.max_steps) {
            return Some("a turn makes between 1 and 1,000 model calls".into());
        }
        if !(30..=86_400).contains(&self.turn_timeout) {
            return Some("a turn's limit is between 30 s and 24 h".into());
        }
        if self.temperature.is_some_and(|t| !(0.0..=2.0).contains(&t)) {
            return Some("a temperature is between 0 and 2".into());
        }
        None
    }
}

pub struct LocalEngine {
    dir: PathBuf,
    settings: Mutex<Settings>,
    /// Run before a turn whose thread chose a model (as an ACP harness's `prepare`).
    pub prepare: Vec<String>,
    pub prepare_timeout: Duration,
    pub extra_env: BTreeMap<String, String>,
    /// How long the model may stay silent in the middle of an answer.
    pub quiet: Duration,
}

impl LocalEngine {
    pub fn new(dir: PathBuf) -> Self {
        let settings = std::fs::read(dir.join("settings.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self {
            dir,
            settings: Mutex::new(settings),
            prepare: Vec::new(),
            prepare_timeout: Duration::from_secs(600),
            extra_env: BTreeMap::new(),
            quiet: Duration::from_secs(300),
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().expect("not poisoned").clone()
    }

    pub fn set_settings(&self, s: Settings) -> Result<(), String> {
        if let Some(why) = s.problem() {
            return Err(why);
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        io::atomic_write(
            &self.dir.join("settings.json"),
            &serde_json::to_vec_pretty(&s).expect("serializes"),
        )
        .map_err(|e| e.to_string())?;
        *self.settings.lock().expect("not poisoned") = s;
        Ok(())
    }

    fn keys(&self) -> HashMap<String, String> {
        std::fs::read(self.dir.join("keys.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Keep a provider's key (empty: none), readable by the core's user only.
    pub fn set_key(&self, provider: &str, key: &str) -> Result<(), String> {
        let mut keys = self.keys();
        if key.is_empty() {
            keys.remove(provider);
        } else {
            keys.insert(provider.into(), key.into());
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let path = self.dir.join("keys.json");
        io::atomic_write(&path, &serde_json::to_vec(&keys).expect("serializes"))
            .map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    fn server(&self, provider: &Provider) -> llm::Server {
        llm::Server {
            url: provider.url.trim_end_matches('/').to_string(),
            key: self.keys().get(&provider.id).cloned().unwrap_or_default(),
        }
    }

    /// The models of every provider, as "<provider>/<model>" (a provider that does not answer
    /// gives none).
    pub async fn models(&self) -> Vec<(String, Result<Vec<String>, String>)> {
        let mut out = Vec::new();
        for p in self.settings().providers {
            let got = llm::models(&self.server(&p)).await;
            out.push((p.id.clone(), got));
        }
        out
    }

    /// Try a server before it is kept: its models, or why it does not answer.
    pub async fn test(url: &str, key: &str) -> Result<Vec<String>, String> {
        llm::models(&llm::Server {
            url: url.trim().trim_end_matches('/').to_string(),
            key: key.into(),
        })
        .await
    }

    /// How the engine is, for its sheet: its settings, its providers (never their keys) with
    /// their models.
    pub async fn status(&self) -> Value {
        let s = self.settings();
        let keys = self.keys();
        let mut providers = Vec::new();
        for (id, got) in self.models().await {
            let p = s.providers.iter().find(|p| p.id == id).cloned();
            let Some(p) = p else { continue };
            providers.push(match got {
                Ok(models) => json!({"id": p.id, "name": p.name, "url": p.url, "key": keys.contains_key(&p.id),
                                     "reachable": true, "models": models}),
                Err(e) => json!({"id": p.id, "name": p.name, "url": p.url, "key": keys.contains_key(&p.id),
                                 "reachable": false, "problem": e, "models": []}),
            });
        }
        json!({"providers": providers, "model": s.model, "mode": s.mode, "context_window": s.context_window,
               "max_steps": s.max_steps, "turn_timeout": s.turn_timeout, "temperature": s.temperature,
               "instructions": s.instructions,
               "modes": prompts::AGENTS.iter().map(|a| json!({"id": a.id, "name": a.name, "description": a.description})).collect::<Vec<_>>()})
    }

    pub async fn probe(&self) -> Probe {
        let mut models = Vec::new();
        for (id, got) in self.models().await {
            for m in got.unwrap_or_default() {
                models.push(format!("{id}/{m}"));
            }
        }
        Probe {
            models,
            modes: prompts::AGENTS
                .iter()
                .map(|a| json!({"id": a.id, "name": a.name, "description": a.description}))
                .collect(),
            commands: Vec::new(),
        }
    }

    pub async fn turn(&self, t: &Turn) -> RunSummary {
        if t.resume {
            // the engine never runs a turn again of its own: nothing to follow
            return RunSummary {
                stopped: "done".into(),
                ..Default::default()
            };
        }
        if let Err(e) = prepare_model(&self.prepare, self.prepare_timeout, &self.extra_env, t).await
        {
            t.note(
                EventKind::Checkpoint,
                json!({"reason": "model_unavailable", "detail": e}),
            );
            return RunSummary {
                stopped: "error".into(),
                error: e,
                ..Default::default()
            };
        }
        let limit = Duration::from_secs(self.settings().turn_timeout);
        match tokio::time::timeout(limit, self.conversation(t)).await {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                t.note(
                    EventKind::Checkpoint,
                    json!({"reason": "harness_error", "detail": e}),
                );
                RunSummary {
                    stopped: "error".into(),
                    error: e,
                    ..Default::default()
                }
            }
            Err(_) => {
                t.note(EventKind::Checkpoint, json!({"reason": "harness_timeout"}));
                RunSummary {
                    stopped: "error".into(),
                    error: format!("the turn did not finish within {} s", limit.as_secs()),
                    ..Default::default()
                }
            }
        }
    }

    /// The model and its server for this turn: the thread's choice, else the engine's.
    fn model_for(&self, t: &Turn) -> Result<(llm::Server, String, String), String> {
        let s = self.settings();
        let chosen = if t.model.is_empty() {
            &s.model
        } else {
            &t.model
        };
        let (pid, model) = chosen.split_once('/').ok_or(
            "no model chosen for the local engine: Settings → Agents and subscriptions → Local engine",
        )?;
        let provider = s
            .providers
            .iter()
            .find(|p| p.id == pid)
            .ok_or_else(|| format!("the local engine has no provider {pid:?}"))?;
        Ok((self.server(provider), model.to_string(), chosen.clone()))
    }

    async fn conversation(&self, t: &Turn) -> Result<RunSummary, String> {
        let s = self.settings();
        let (server, model, shown_model) = self.model_for(t)?;
        let agent = prompts::agent(if t.mode.is_empty() { &s.mode } else { &t.mode });
        let today = today();
        let system = prompts::system(
            &t.workdir.display().to_string(),
            &s.instructions,
            &shown_model,
            &today,
        );
        let mut current = format!("{}\n\n{}", prompts::reminder(agent), t.text);
        if !t.attachments.is_empty() {
            current.push_str("\n\nAttached files (in your working directory):");
            for a in &t.attachments {
                current.push_str(&format!("\n- {} ({}, {} bytes)", a.path, a.mime, a.size));
            }
        }
        let mut past = history(&t.path())?;
        // past 80 % of the window, the conversation so far is summarised and the summary kept
        let budget = s.context_window * 8 / 10;
        if estimate(&system, &past, &current) > budget && past.len() > 1 {
            t.phase("loading");
            let summary = self.compact(&server, &model, &past, &s).await?;
            t.note(
                EventKind::Checkpoint,
                json!({"reason": "compacted", "detail": "the conversation so far was summarised", "summary": summary}),
            );
            past = vec![
                json!({"role": "user", "content": format!("Summary of the conversation so far:\n{summary}")}),
                json!({"role": "assistant", "content": "Understood."}),
            ];
        }
        let mut messages = vec![json!({"role": "system", "content": system})];
        messages.extend(past);
        messages.push(json!({"role": "user", "content": current}));
        let defs = tools::definitions(agent.tools);
        let mut summary = RunSummary {
            stopped: "done".into(),
            ..Default::default()
        };
        let mut text = String::new();
        let mut open: HashMap<String, bool> = HashMap::new();
        for step in 0..s.max_steps {
            if t.cancelled() || !t.running() {
                flush_answer(t, &mut text, &mut summary);
                summary.stopped = if t.cancelled() { "cancelled" } else { "frozen" }.into();
                return Ok(summary);
            }
            t.phase("inference");
            let mut request = json!({"model": model, "messages": messages});
            if !defs.is_empty() {
                request["tools"] = json!(defs);
            }
            if let Some(temp) = s.temperature {
                request["temperature"] = json!(temp);
            }
            let streamed = llm::chat(&server, &request, self.quiet, |piece| {
                if let llm::Piece::Text(chunk) = piece {
                    acp_update(
                        t,
                        &json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": chunk}}),
                        &mut text,
                        &mut open,
                        &mut summary,
                    );
                }
            });
            // a stop asked meanwhile ends the answer where it is
            let reply = tokio::select! {
                r = streamed => r?,
                _ = stopped(t) => {
                    flush_answer(t, &mut text, &mut summary);
                    summary.stopped = if t.cancelled() { "cancelled" } else { "frozen" }.into();
                    return Ok(summary);
                }
            };
            if reply.prompt_tokens > 0 {
                t.runner.set_usage(
                    &t.agent,
                    reply.prompt_tokens + reply.completion_tokens,
                    s.context_window,
                    None,
                );
            }
            flush_answer(t, &mut text, &mut summary);
            if reply.calls.is_empty() {
                return Ok(summary);
            }
            let calls: Vec<llm::Call> = reply
                .calls
                .into_iter()
                .enumerate()
                .map(|(i, mut c)| {
                    if c.id.is_empty() {
                        c.id = format!("call_{step}_{i}");
                    }
                    c
                })
                .collect();
            messages.push(json!({"role": "assistant",
                "content": if reply.text.is_empty() { Value::Null } else { json!(reply.text) },
                "tool_calls": calls.iter().map(|c| json!({"id": c.id, "type": "function",
                    "function": {"name": c.name, "arguments": c.arguments}})).collect::<Vec<_>>()}));
            for call in &calls {
                let (ok, output) = self.use_tool(t, agent, call, &mut open, &mut summary).await;
                let _ = ok;
                messages.push(json!({"role": "tool", "tool_call_id": call.id, "content": output}));
            }
        }
        t.note(
            EventKind::Checkpoint,
            json!({"reason": "harness_error", "detail": format!("the turn reached its {} model calls: send a message to go on", s.max_steps)}),
        );
        Ok(summary)
    }

    /// One tool call: journaled, done (or refused), its result journaled; the text for the model.
    async fn use_tool(
        &self,
        t: &Turn,
        agent: &prompts::Agent,
        call: &llm::Call,
        open: &mut HashMap<String, bool>,
        summary: &mut RunSummary,
    ) -> (bool, String) {
        let args: Value = serde_json::from_str(if call.arguments.trim().is_empty() {
            "{}"
        } else {
            &call.arguments
        })
        .unwrap_or(Value::Null);
        let mut nothing = String::new();
        acp_update(
            t,
            &json!({"sessionUpdate": "tool_call", "toolCallId": call.id,
                    "title": crate::openfox::tool_title(&call.name, &args),
                    "kind": crate::openfox::tool_kind(&call.name), "rawInput": args}),
            &mut nothing,
            open,
            summary,
        );
        let result = if args.is_null() {
            Err(format!(
                "the arguments of {} are not valid JSON: call it again with a JSON object",
                call.name
            ))
        } else if !agent.tools.contains(&call.name.as_str()) {
            Err(format!(
                "{} is not available in {} mode (available: {})",
                call.name,
                agent.name,
                agent.tools.join(", ")
            ))
        } else {
            self.run_tool(t, &call.name, &args).await
        };
        let (ok, mut output) = match result {
            Ok(o) => (true, o),
            Err(e) => (false, e),
        };
        let shown = match (ok, crate::openfox::edit_diff(&call.name, &args)) {
            (true, Some(diff)) => format!("{output}\n{diff}"),
            _ => output.clone(),
        };
        acp_update(
            t,
            &json!({"sessionUpdate": "tool_call_update", "toolCallId": call.id,
                    "status": if ok { "completed" } else { "failed" }, "content": [], "rawOutput": shown}),
            &mut nothing,
            open,
            summary,
        );
        if output.is_empty() {
            output = "(done)".into();
        }
        (ok, output)
    }

    async fn run_tool(&self, t: &Turn, name: &str, args: &Value) -> Result<String, String> {
        let under_pass = !t.pass.is_empty();
        let path_arg = args.get("path").and_then(Value::as_str).unwrap_or("");
        match name {
            "read_file" | "write_file" | "edit_file" => {
                if path_arg.is_empty() {
                    return Err(format!("{name} needs a path"));
                }
                let (path, inside) = tools::resolve(&t.workdir, path_arg);
                let writes = name != "read_file";
                // outside the folder, or a pass's write: the person says
                if (!inside || (under_pass && writes))
                    && !t
                        .runner
                        .approve(&t.agent, &format!("{name} {}", path.display()))
                        .await
                {
                    return Err(format!(
                        "the person did not allow {name} on {}",
                        path.display()
                    ));
                }
                match name {
                    "read_file" => tools::read_file(&path, args),
                    "write_file" => tools::write_file(&path, args),
                    _ => tools::edit_file(&path, args),
                }
            }
            "run_command" => {
                let command = args
                    .get("command")
                    .and_then(Value::as_str)
                    .filter(|c| !c.trim().is_empty())
                    .ok_or("run_command needs a command")?;
                let cwd = match args
                    .get("cwd")
                    .and_then(Value::as_str)
                    .filter(|c| !c.is_empty())
                {
                    Some(c) => {
                        let (p, inside) = tools::resolve(&t.workdir, c);
                        if !inside {
                            return Err(format!(
                                "{} is outside the working directory",
                                p.display()
                            ));
                        }
                        p
                    }
                    None => t.workdir.clone(),
                };
                if under_pass && !t.runner.approve(&t.agent, command).await {
                    return Err("the person did not allow this command".into());
                }
                let timeout = args
                    .get("timeout")
                    .and_then(Value::as_u64)
                    .unwrap_or(120_000)
                    .clamp(1000, 600_000);
                t.phase("inference");
                let (ok, out) =
                    tools::run_command(&cwd, command, Duration::from_millis(timeout), under_pass)
                        .await;
                if ok {
                    Ok(out)
                } else {
                    Err(out)
                }
            }
            other => Err(format!("unknown tool {other}")),
        }
    }

    /// Summarise the conversation so far with the turn's model.
    async fn compact(
        &self,
        server: &llm::Server,
        model: &str,
        past: &[Value],
        s: &Settings,
    ) -> Result<String, String> {
        let transcript: String = past
            .iter()
            .map(|m| {
                format!(
                    "{}: {}",
                    m["role"].as_str().unwrap_or(""),
                    m["content"].as_str().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        // keep the end, the part a summary needs most, when even the transcript is too long
        let room = (s.context_window as usize * 3).saturating_sub(8000);
        let transcript: String = if transcript.len() > room {
            transcript
                .chars()
                .rev()
                .take(room)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect()
        } else {
            transcript
        };
        let request = json!({"model": model, "messages": [
            {"role": "system", "content": prompts::COMPACTION},
            {"role": "user", "content": transcript}]});
        let reply = llm::chat(server, &request, self.quiet, |_| {}).await?;
        let summary = reply.text.trim().to_string();
        if summary.is_empty() {
            return Err("the model gave an empty summary".into());
        }
        Ok(summary)
    }
}

/// Resolves once the turn is asked to stop (or the thread left this core).
async fn stopped(t: &Turn) {
    loop {
        if t.cancelled() || !t.running() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

fn today() -> String {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0) as i64;
    // days since 1970-01-01 to a civil date (Howard Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y}/{m:02}/{d:02}")
}

/// A rough count of tokens (about four characters each).
fn estimate(system: &str, past: &[Value], current: &str) -> u64 {
    let chars =
        system.len() + current.len() + past.iter().map(|m| m.to_string().len()).sum::<usize>();
    (chars / 4) as u64
}

const PAST_TOOL_CHARS: usize = 2000;

/// The thread's earlier turns, from its journal, as plain messages: what the person said, what
/// the agent answered, its tool calls and their results told in its words (whichever harness
/// worked then), from the last summary on. The turn's own message (the last one) is left out.
fn history(capsule: &Path) -> Result<Vec<Value>, String> {
    let c = io::load(capsule).map_err(|e| e.to_string())?;
    let events: Vec<_> = c.timeline.ordered().into_iter().cloned().collect();
    let last_user = events
        .iter()
        .rposition(|e| e.kind == EventKind::UserMessage);
    let mut out: Vec<Value> = Vec::new();
    let mut said = String::new(); // the agent's side, gathered until the person speaks again
    let close = |out: &mut Vec<Value>, said: &mut String| {
        if !said.trim().is_empty() {
            out.push(json!({"role": "assistant", "content": said.trim()}));
        }
        said.clear();
    };
    for (i, e) in events.iter().enumerate() {
        if Some(i) == last_user {
            break;
        }
        let text = |k: &str| {
            e.data
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        match e.kind {
            EventKind::UserMessage => {
                close(&mut out, &mut said);
                out.push(json!({"role": "user", "content": text("text")}));
            }
            EventKind::AssistantMessage => {
                said.push_str(&text("text"));
                said.push_str("\n\n");
            }
            EventKind::ToolCall => {
                let input = e
                    .data
                    .pointer("/arguments/input")
                    .cloned()
                    .unwrap_or(Value::Null);
                let input = input.to_string();
                said.push_str(&format!(
                    "[I called {}: {}]\n",
                    text("name"),
                    input.chars().take(400).collect::<String>()
                ));
            }
            EventKind::ToolResult => {
                let output = text("output");
                let short: String = output.chars().take(PAST_TOOL_CHARS).collect();
                let more = if output.chars().count() > PAST_TOOL_CHARS {
                    " …"
                } else {
                    ""
                };
                said.push_str(&format!("[result: {short}{more}]\n\n"));
            }
            EventKind::Checkpoint
                if e.data.get("reason").and_then(Value::as_str) == Some("compacted") =>
            {
                // what came before is in the summary
                out.clear();
                said.clear();
                out.push(json!({"role": "user", "content": format!("Summary of the conversation so far:\n{}", text("summary"))}));
                out.push(json!({"role": "assistant", "content": "Understood."}));
            }
            _ => {}
        }
    }
    close(&mut out, &mut said);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_are_checked() {
        let ok = Settings::default();
        assert_eq!(ok.problem(), None);
        assert_eq!((ok.mode.as_str(), ok.context_window), ("builder", 32_768));
        let bad = Settings {
            providers: vec![Provider {
                id: "v".into(),
                name: "vLLM".into(),
                url: "https://x/v1".into(),
            }],
            ..ok.clone()
        };
        assert!(bad.problem().is_some());
        assert!(Settings {
            mode: "chaos".into(),
            ..ok
        }
        .problem()
        .is_some());
    }

    #[test]
    fn today_is_a_date() {
        let d = today();
        assert_eq!(d.len(), 10);
        assert!(d.starts_with("20"));
    }
}
