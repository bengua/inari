//! A turn of the local engine, and the loop every run of the model goes through: a thread's
//! turn, a sub-agent, a workflow's step. The model streams, asks for tools, gets their results,
//! until it answers without a tool (or a step says it is done, or a sub-agent returns).

use super::defs::{self, AgentDef};
use super::state::ThreadState;
use super::{llm, prompts, tools, LocalEngine, Settings};
use crate::agent::{acp_update, flush_answer, prepare_model, RunSummary, Turn};
use crate::capsule::{io, EventKind};
use crate::openfox::{proto::Choice, Question};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

/// One run of the model with its tools.
pub(crate) struct Run {
    pub server: llm::Server,
    pub model: String,
    pub agent: AgentDef,
    /// The tools it may use (built-in and MCP).
    pub tools: Vec<String>,
    pub messages: Vec<Value>,
    /// A sub-agent's kind: its words go to its card, its tools are not journaled.
    pub sub: Option<String>,
    pub max_steps: u32,
}

/// How a run ended.
#[derive(Default)]
pub(crate) struct RunEnd {
    /// The last answer the model gave.
    pub text: String,
    /// cancelled or frozen when the turn was stopped.
    pub stopped: Option<String>,
    /// A workflow step said it is done.
    pub step_done: bool,
    /// A sub-agent's return value and its result.
    pub returned: Option<(String, Option<String>)>,
    /// The model calls ran out.
    pub out_of_steps: bool,
}

impl LocalEngine {
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
        self.active
            .lock()
            .expect("not poisoned")
            .insert(t.agent.clone());
        let limit = Duration::from_secs(self.settings().turn_timeout);
        let out = tokio::time::timeout(limit, async {
            match &t.workflow {
                Some(l) => self.flow(t, l).await,
                None => self.conversation(t).await,
            }
        })
        .await;
        self.active.lock().expect("not poisoned").remove(&t.agent);
        // what was sent too late for the model goes on as the next turn's message
        let left = self.take_queue(&t.agent);
        self.edit_panel(&t.agent, |p| {
            p.running = false;
            p.thinking.clear();
            p.progress.clear();
            p.question = None;
            p.queue.clear();
            for a in p.subagents.iter_mut() {
                a.done = true;
            }
        });
        let state = ThreadState::load(&t.path());
        self.fill_panel(&t.agent, &self.mode_for(t), &state, Some(&t.workdir));
        self.after_turn(&t.agent, &t.workdir).await;
        self.fill_project(&t.agent, &t.workdir, &t.path()).await;
        self.show(&t.runner, &t.agent, true);
        if !left.is_empty() {
            t.runner.run_later(&t.agent, left.join("\n\n"));
        }
        match out {
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

    /// The agent the thread works as: its own choice, else the engine's.
    pub(crate) fn mode_for(&self, t: &Turn) -> String {
        if t.mode.is_empty() {
            self.settings().mode
        } else {
            t.mode.clone()
        }
    }

    /// The model and its server for this turn: the thread's choice, else the engine's.
    pub(crate) fn model_for(&self, t: &Turn) -> Result<(llm::Server, String, String), String> {
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

    /// The system prompt (the same for every top-level agent, so the server's cache keeps it).
    pub(crate) fn system_for(&self, t: &Turn, shown_model: &str) -> String {
        let s = self.settings();
        let skills: Vec<_> = defs::skills(&self.dir, Some(&t.workdir))
            .into_iter()
            .filter(|k| !s.disabled_skills.contains(&k.id))
            .collect();
        let subs: Vec<AgentDef> = defs::agents(&self.dir, Some(&t.workdir))
            .into_iter()
            .filter(|a| a.subagent)
            .collect();
        let mut instructions = s.instructions.clone();
        if let Ok(project) =
            std::fs::read_to_string(t.workdir.join(".openfox").join("instructions.md"))
        {
            if !project.trim().is_empty() {
                instructions = format!("{}\n\n{}", instructions.trim(), project.trim());
            }
        }
        prompts::system(
            &t.workdir.display().to_string(),
            &instructions,
            shown_model,
            &today(),
            &skills,
            &subs,
        )
    }

    /// The tools an agent has: its own that the engine knows, the MCP servers', and the one a
    /// run of its kind needs (step_done for a workflow step, return_value for a sub-agent).
    pub(crate) fn tools_for(&self, a: &AgentDef, extra: &[&str]) -> Vec<String> {
        let mut out: Vec<String> = a
            .tools
            .iter()
            .filter(|t| tools::BUILT_IN.contains(&t.as_str()))
            .filter(|t| !(a.subagent && t.as_str() == "call_sub_agent"))
            .cloned()
            .collect();
        for e in extra {
            if !out.iter().any(|t| t == e) {
                out.push(e.to_string());
            }
        }
        if !a.subagent || a.id == "explorer" {
            out.extend(self.mcp_tool_names());
        }
        out
    }

    async fn conversation(&self, t: &Turn) -> Result<RunSummary, String> {
        let s = self.settings();
        self.mcp_ready().await;
        let (server, model, shown_model) = self.model_for(t)?;
        let mut mode = self.mode_for(t);
        let mut said = t.text.clone();
        // a slash command: its prompt, with what follows as its arguments
        if let Some(rest) = t.text.trim_start().strip_prefix('/') {
            let (id, args) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            if let Some(c) = defs::commands(&self.dir, Some(&t.workdir))
                .into_iter()
                .find(|c| c.id == id)
            {
                said = if args.trim().is_empty() {
                    c.prompt.clone()
                } else {
                    format!("{}\n\nArguments: {}", c.prompt, args.trim())
                };
                if !c.agent_mode.is_empty() {
                    mode = c.agent_mode.clone();
                }
            }
        }
        let all = defs::agents(&self.dir, Some(&t.workdir));
        let agent = all
            .iter()
            .find(|a| a.id == mode && !a.subagent)
            .or_else(|| all.iter().find(|a| a.id == "builder"))
            .cloned()
            .ok_or("the engine has no builder agent")?;
        let tools = self.tools_for(&agent, &[]);
        let system = self.system_for(t, &shown_model);
        let mut current = format!("{}\n\n{said}", prompts::reminder(&agent, &tools));
        if !t.attachments.is_empty() {
            current.push_str("\n\nAttached files (in your working directory):");
            for a in &t.attachments {
                current.push_str(&format!("\n- {} ({}, {} bytes)", a.path, a.mime, a.size));
            }
        }
        let past = self.past(t, &server, &model, &system, &current, &s).await?;
        let mut messages = vec![json!({"role": "system", "content": system})];
        messages.extend(past);
        messages.push(json!({"role": "user", "content": current}));
        let state = ThreadState::load(&t.path());
        self.fill_panel(&t.agent, &agent.id, &state, Some(&t.workdir));
        self.edit_panel(&t.agent, |p| {
            p.running = true;
            p.error.clear();
            p.phase = agent.name.clone();
            p.subagents.retain(|a| !a.done);
        });
        self.show(&t.runner, &t.agent, true);
        let mut run = Run {
            server,
            model,
            agent,
            tools,
            messages,
            sub: None,
            max_steps: s.max_steps,
        };
        let started = Instant::now();
        let end = self.agent_loop(t, &mut run).await?;
        let mut summary = RunSummary {
            stopped: end.stopped.clone().unwrap_or_else(|| "done".into()),
            final_text: end.text.clone(),
            ..Default::default()
        };
        if end.out_of_steps {
            t.note(
                EventKind::Checkpoint,
                json!({"reason": "harness_error", "detail": format!("the turn reached its {} model calls: send a message to go on", s.max_steps)}),
            );
        }
        let secs = started.elapsed().as_secs_f64();
        self.edit_panel(&t.agent, |p| {
            p.stats = Some(json!({"model": shown_model_name(&run.model), "seconds": secs}));
        });
        if summary.stopped.is_empty() {
            summary.stopped = "done".into();
        }
        Ok(summary)
    }

    /// The conversation so far for the model; summarised first past 80 % of its context, or
    /// when the person asked (Compact).
    pub(crate) async fn past(
        &self,
        t: &Turn,
        server: &llm::Server,
        model: &str,
        system: &str,
        current: &str,
        s: &Settings,
    ) -> Result<Vec<Value>, String> {
        let past = history(&t.path())?;
        let asked = self
            .compact_next
            .lock()
            .expect("not poisoned")
            .remove(&t.agent);
        let budget = s.context_window * 8 / 10;
        if (asked || estimate(system, &past, current) > budget) && past.len() > 1 {
            t.phase("loading");
            self.edit_panel(&t.agent, |p| {
                p.progress = "Summarising the conversation…".into()
            });
            self.show(&t.runner, &t.agent, true);
            let summary = self.compact(server, model, &past, s).await?;
            t.note(
                EventKind::Checkpoint,
                json!({"reason": "compacted", "detail": "the conversation so far was summarised", "summary": summary}),
            );
            let mut state = ThreadState::load(&t.path());
            state.compactions += 1;
            let _ = state.save(&t.path(), &author(t));
            self.edit_panel(&t.agent, |p| p.progress.clear());
            return Ok(vec![
                json!({"role": "user", "content": format!("Summary of the conversation so far:\n{summary}")}),
                json!({"role": "assistant", "content": "Understood."}),
            ]);
        }
        Ok(past)
    }

    /// The model and its tools until it answers without a tool (or the run ends otherwise).
    pub(crate) async fn agent_loop(&self, t: &Turn, run: &mut Run) -> Result<RunEnd, String> {
        let subs: Vec<String> = defs::agents(&self.dir, Some(&t.workdir))
            .into_iter()
            .filter(|a| a.subagent)
            .map(|a| a.id)
            .collect();
        let mut defs_json = tools::definitions(&run.tools, &subs);
        defs_json.extend(self.mcp_definitions(&run.tools));
        let temperature = self.settings().temperature;
        let mut end = RunEnd::default();
        let mut text = String::new();
        let mut open: HashMap<String, bool> = HashMap::new();
        let mut summary = RunSummary::default();
        for step in 0..run.max_steps {
            if t.cancelled() || !t.running() {
                if run.sub.is_none() {
                    flush_answer(t, &mut text, &mut summary);
                }
                end.stopped = Some(if t.cancelled() { "cancelled" } else { "frozen" }.into());
                return Ok(end);
            }
            // what the person sent meanwhile, read now (the thread's own run only)
            if run.sub.is_none() {
                for q in self.take_queue(&t.agent) {
                    if let Some(e) = t.append(EventKind::UserMessage, json!({"text": q.trim()})) {
                        t.emit(&e);
                    }
                    run.messages.push(json!({"role": "user", "content": q}));
                }
                self.edit_panel(&t.agent, |p| p.queue.clear());
            }
            t.phase("inference");
            let mut request = json!({"model": run.model, "messages": run.messages});
            if !defs_json.is_empty() {
                request["tools"] = json!(defs_json);
            }
            if let Some(temp) = temperature {
                request["temperature"] = json!(temp);
            }
            let sub = run.sub.clone();
            let started = Instant::now();
            let streamed = llm::chat(&run.server, &request, self.quiet, |piece| {
                match (&sub, piece) {
                    (None, llm::Piece::Text(chunk)) => acp_update(
                        t,
                        &json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": chunk}}),
                        &mut text,
                        &mut open,
                        &mut summary,
                    ),
                    (None, llm::Piece::Reasoning(r)) => {
                        self.edit_panel(&t.agent, |p| {
                            p.thinking.push_str(r);
                            let n = p.thinking.chars().count();
                            if n > 8000 {
                                p.thinking = p.thinking.chars().skip(n - 8000).collect();
                            }
                        });
                        self.show(&t.runner, &t.agent, false);
                    }
                    (Some(kind), llm::Piece::Text(chunk)) => {
                        self.sub_said(&t.agent, kind, chunk);
                        self.show(&t.runner, &t.agent, false);
                    }
                    (Some(_), llm::Piece::Reasoning(_)) => {}
                }
            });
            // a stop asked meanwhile ends the answer where it is
            let reply = tokio::select! {
                r = streamed => r?,
                _ = stopped(t) => {
                    if run.sub.is_none() {
                        flush_answer(t, &mut text, &mut summary);
                    }
                    end.stopped = Some(if t.cancelled() { "cancelled" } else { "frozen" }.into());
                    return Ok(end);
                }
            };
            if run.sub.is_none() {
                let speed =
                    reply.completion_tokens as f64 / started.elapsed().as_secs_f64().max(0.001);
                let window = self.settings().context_window;
                if reply.prompt_tokens > 0 {
                    t.runner.set_usage(
                        &t.agent,
                        reply.prompt_tokens + reply.completion_tokens,
                        window,
                        None,
                    );
                }
                self.edit_panel(&t.agent, |p| {
                    p.thinking.clear();
                    if let Some(c) = p.context.as_mut() {
                        if reply.prompt_tokens > 0 {
                            c.used = reply.prompt_tokens + reply.completion_tokens;
                            c.danger = c.used * 10 > c.max * 8;
                        }
                    }
                    p.stats = Some(json!({"model": shown_model_name(&run.model),
                        "speed": if reply.completion_tokens > 0 { speed } else { 0.0 }}));
                });
                self.show(&t.runner, &t.agent, true);
                flush_answer(t, &mut text, &mut summary);
            } else if !reply.text.is_empty() {
                self.sub_said(&t.agent, run.sub.as_deref().unwrap_or(""), "\n");
            }
            if !reply.text.trim().is_empty() {
                end.text = reply.text.trim().to_string();
            }
            if reply.calls.is_empty() {
                return Ok(end);
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
            run.messages.push(json!({"role": "assistant",
                "content": if reply.text.is_empty() { Value::Null } else { json!(reply.text) },
                "tool_calls": calls.iter().map(|c| json!({"id": c.id, "type": "function",
                    "function": {"name": c.name, "arguments": c.arguments}})).collect::<Vec<_>>()}));
            for call in &calls {
                let output = self.use_tool(t, run, call, &mut open, &mut end).await;
                run.messages
                    .push(json!({"role": "tool", "tool_call_id": call.id, "content": output}));
            }
            if end.step_done || end.returned.is_some() {
                return Ok(end);
            }
        }
        end.out_of_steps = true;
        Ok(end)
    }

    /// One tool call: shown (journaled for the thread's own run, on its card for a sub-agent),
    /// done or refused; the text for the model.
    async fn use_tool(
        &self,
        t: &Turn,
        run: &Run,
        call: &llm::Call,
        open: &mut HashMap<String, bool>,
        end: &mut RunEnd,
    ) -> String {
        let args: Value = serde_json::from_str(if call.arguments.trim().is_empty() {
            "{}"
        } else {
            &call.arguments
        })
        .unwrap_or(Value::Null);
        let title = crate::openfox::tool_title(&call.name, &args);
        let mut nothing = String::new();
        let mut summary = RunSummary::default();
        match &run.sub {
            None => acp_update(
                t,
                &json!({"sessionUpdate": "tool_call", "toolCallId": call.id, "title": title,
                        "kind": crate::openfox::tool_kind(&call.name), "rawInput": args}),
                &mut nothing,
                open,
                &mut summary,
            ),
            Some(kind) => {
                self.sub_said(&t.agent, kind, &format!("\n[{title}]\n"));
                self.show(&t.runner, &t.agent, false);
            }
        }
        let result = if args.is_null() {
            Err(format!(
                "the arguments of {} are not valid JSON: call it again with a JSON object",
                call.name
            ))
        } else if !run.tools.contains(&call.name) {
            Err(format!(
                "{} is not available to {} (available: {})",
                call.name,
                run.agent.name,
                run.tools.join(", ")
            ))
        } else {
            self.run_tool(t, run, &call.name, &args, end).await
        };
        let (ok, mut output) = match result {
            Ok(o) => (true, o),
            Err(e) => (false, e),
        };
        if run.sub.is_none() {
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
                &mut summary,
            );
        }
        if output.is_empty() {
            output = "(done)".into();
        }
        output
    }

    async fn run_tool(
        &self,
        t: &Turn,
        run: &Run,
        name: &str,
        args: &Value,
        end: &mut RunEnd,
    ) -> Result<String, String> {
        let under_pass = !t.pass.is_empty();
        let text = |k: &str| {
            args.get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        match name {
            "read_file" | "write_file" | "edit_file" => {
                let path_arg = text("path");
                if path_arg.is_empty() {
                    return Err(format!("{name} needs a path"));
                }
                let (path, inside) = tools::resolve(&t.workdir, &path_arg);
                let writes = name != "read_file";
                let bold = ThreadState::load(&t.path()).danger == "dangerous" && !under_pass;
                // outside the folder (unless the thread is bold), or a pass's write: the person says
                if ((!inside && !bold) || (under_pass && writes))
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
                let command = text("command");
                if command.trim().is_empty() {
                    return Err("run_command needs a command".into());
                }
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
                if under_pass && !t.runner.approve(&t.agent, &command).await {
                    return Err("the person did not allow this command".into());
                }
                let timeout = args
                    .get("timeout")
                    .and_then(Value::as_u64)
                    .unwrap_or(120_000)
                    .clamp(1000, 600_000);
                let (ok, out) =
                    tools::run_command(&cwd, &command, Duration::from_millis(timeout), under_pass)
                        .await;
                if ok {
                    Ok(out)
                } else {
                    Err(out)
                }
            }
            "ask_user" => {
                if run.sub.is_some() {
                    return Err(
                        "a sub-agent does not ask the person: return what you need in return_value"
                            .into(),
                    );
                }
                let question = text("question");
                if question.trim().is_empty() {
                    return Err("ask_user needs a question".into());
                }
                let mut kind = text("type");
                let mut options: Vec<Choice> = args
                    .get("options")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|o| match o {
                        Value::String(s) => Some(Choice {
                            value: s.clone(),
                            label: Some(s.clone()),
                            description: None,
                        }),
                        Value::Object(m) => {
                            let label = m.get("label").and_then(Value::as_str)?.to_string();
                            Some(Choice {
                                value: m
                                    .get("value")
                                    .and_then(Value::as_str)
                                    .unwrap_or(&label)
                                    .to_string(),
                                label: Some(label),
                                description: m
                                    .get("description")
                                    .and_then(Value::as_str)
                                    .map(String::from),
                            })
                        }
                        _ => None,
                    })
                    .collect();
                if kind == "confirm" && options.is_empty() {
                    options = ["Yes", "No"]
                        .iter()
                        .map(|v| Choice {
                            value: v.to_string(),
                            label: Some(v.to_string()),
                            description: None,
                        })
                        .collect();
                }
                if !options.is_empty() {
                    kind = "choice".into();
                } else {
                    kind = "text".into();
                }
                let call = format!("ask-{}", crate::wire::random_hex(4));
                let answer = self
                    .ask(
                        t,
                        &call,
                        Question {
                            call_id: call.clone(),
                            question,
                            kind,
                            options,
                        },
                    )
                    .await;
                Ok(match answer {
                    Some(a) => format!("The user answered: {a}"),
                    None => "The user skipped the question.".into(),
                })
            }
            "session_metadata" => {
                let mut state = ThreadState::load(&t.path());
                let out = state.metadata_tool(args)?;
                if !["get", "list", "schema"].contains(&text("action").as_str()) {
                    state.save(&t.path(), &author(t))?;
                    self.fill_panel(&t.agent, &self.mode_for(t), &state, Some(&t.workdir));
                    self.show(&t.runner, &t.agent, true);
                }
                Ok(out)
            }
            "load_skill" => {
                let id = text("skillId");
                let disabled = self.settings().disabled_skills;
                defs::skills(&self.dir, Some(&t.workdir))
                    .into_iter()
                    .find(|s| s.id == id && !disabled.contains(&s.id))
                    .map(|s| format!("# Skill: {}\n\n{}", s.name, s.prompt))
                    .ok_or_else(|| format!("no skill {id:?}"))
            }
            "call_sub_agent" => {
                let kind = text("subAgentType");
                let prompt = text("prompt");
                let out = self.sub_agent(t, run, &kind, &prompt).await?;
                Ok(out.0)
            }
            "step_done" => {
                end.step_done = true;
                Ok("Step marked as done.".into())
            }
            "return_value" => {
                let content = text("content");
                let result = Some(text("result")).filter(|r| !r.is_empty());
                end.returned = Some((content, result));
                Ok("Return value set.".into())
            }
            "background_process" | "dev_server" | "project_tasks" => {
                self.project_tool(t, name, args).await
            }
            mcp => self.mcp_call(mcp, args).await,
        }
    }

    /// A sub-agent's run, with a fresh context: its return value (or last answer) and result.
    pub(crate) async fn sub_agent(
        &self,
        t: &Turn,
        parent: &Run,
        kind: &str,
        prompt: &str,
    ) -> Result<(String, Option<String>), String> {
        let def = defs::agents(&self.dir, Some(&t.workdir))
            .into_iter()
            .find(|a| a.id == kind && a.subagent)
            .ok_or_else(|| format!("no sub-agent {kind:?}"))?;
        let tools = self.tools_for(&def, &["return_value"]);
        let shown_model = parent.model.to_string();
        let system = prompts::subagent_system(&self.system_for(t, &shown_model), &def);
        let state = ThreadState::load(&t.path());
        let mut context = prompt.to_string();
        if state.metadata.contains_key("criteria") {
            context.push_str(&format!("\n\n## Criteria\n{}", state.criteria_list()));
        }
        let mut run = Run {
            server: parent.server.clone(),
            model: parent.model.clone(),
            agent: def,
            tools,
            messages: vec![
                json!({"role": "system", "content": system}),
                json!({"role": "user", "content": context}),
            ],
            sub: Some(kind.to_string()),
            max_steps: 60,
        };
        self.sub_started(&t.agent, kind);
        self.show(&t.runner, &t.agent, true);
        let end = Box::pin(self.agent_loop(t, &mut run)).await;
        self.sub_done(&t.agent, kind);
        self.show(&t.runner, &t.agent, true);
        let end = end?;
        if let Some(why) = end.stopped {
            return Err(format!("the sub-agent was stopped ({why})"));
        }
        Ok(match end.returned {
            Some((content, result)) => (content, result),
            None => (
                if end.text.is_empty() {
                    "(the sub-agent said nothing)".into()
                } else {
                    end.text
                },
                None,
            ),
        })
    }

    /// Summarise the conversation so far with the turn's model.
    pub(crate) async fn compact(
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

pub(crate) fn author(t: &Turn) -> String {
    format!("{}@{}", t.runner.host.node_id, t.agent)
}

fn shown_model_name(model: &str) -> String {
    model.rsplit('/').next().unwrap_or(model).to_string()
}

/// Resolves once the turn is asked to stop (or the thread left this core).
pub(crate) async fn stopped(t: &Turn) {
    loop {
        if t.cancelled() || !t.running() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

pub(crate) fn today() -> String {
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
pub(crate) fn history(capsule: &Path) -> Result<Vec<Value>, String> {
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
    fn today_is_a_date() {
        let d = today();
        assert_eq!(d.len(), 10);
        assert!(d.starts_with("20"));
    }
}
