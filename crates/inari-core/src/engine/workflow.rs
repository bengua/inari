//! Workflows on the local engine, as OpenFox's executor walks them: a state machine of steps
//! (an agent, a sub-agent, a shell command, a step for the person, children run together) with
//! transitions on a step's result or the thread's metadata, until `$done` or `$blocked`. The run
//! is kept in the thread's state, so a step for the person waits across turns and cores.

use super::defs;
use super::run::{author, Run};
use super::state::{Flow, ThreadState};
use super::{prompts, tools, LocalEngine};
use crate::agent::{acp_update, RunSummary, Turn};
use crate::capsule::EventKind;
use crate::openfox::{proto, Launch, WorkflowRun};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::time::Duration;

const DONE: &str = "$done";
const BLOCKED: &str = "$blocked";
const CONTINUE: &str = "continue";
const STEP_DONE_PROMPT: &str = "\n\nOnce you're done, call step_done()";
const STEP_DONE_NUDGE: &str = "You haven't called step_done(). If you haven't finished the task, continue and when you're finished call step_done()";
const STEP_DONE_REMINDER: &str = "If you have finished the task, call step_done()";
const KICKOFF: &str = "Begin working on the task.";

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Definition {
    pub metadata: Meta,
    pub entry_step: String,
    #[serde(default)]
    pub settings: Option<WfSettings>,
    #[serde(default)]
    pub steps: Vec<Step>,
}

#[derive(Deserialize, Clone, Debug)]
pub(crate) struct Meta {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub parameters: Vec<proto::WorkflowParameter>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WfSettings {
    #[serde(default = "fifty")]
    pub max_iterations: u32,
}

fn fifty() -> u32 {
    50
}

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Step {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub transitions: Vec<Transition>,
    #[serde(default)]
    pub sub_group: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub nudge_prompt: Option<String>,
    #[serde(default)]
    pub sub_agent_type: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub timeout: Option<u64>,
    #[serde(default)]
    pub success_exit_codes: Option<Vec<i64>>,
    #[serde(default)]
    pub children: Vec<Step>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Transition {
    pub when: Value,
    pub goto: String,
    #[serde(default)]
    pub sub_group: Option<String>,
}

impl Step {
    fn shown(&self) -> String {
        if self.name.is_empty() {
            self.id.clone()
        } else {
            self.name.clone()
        }
    }
}

/// Every workflow the engine knows, by id (the project's over the person's over the builtin).
pub(crate) fn definitions(engine_dir: &Path, workdir: Option<&Path>) -> Vec<(Definition, String)> {
    defs::workflows(engine_dir, workdir)
        .into_iter()
        .filter_map(|(v, scope)| serde_json::from_value(v).ok().map(|d| (d, scope)))
        .collect()
}

/// A run as the page shows it.
pub(crate) fn flow_view(f: &Flow) -> WorkflowRun {
    WorkflowRun {
        id: f.workflow.clone(),
        name: if f.name.is_empty() {
            f.workflow.clone()
        } else {
            f.name.clone()
        },
        color: f.color.clone(),
        status: f.status.clone(),
        step: f.step_name.clone(),
        choices: if f.status == "waiting" {
            f.choices.clone()
        } else {
            Vec::new()
        },
    }
}

/// Whether a transition's condition holds (OpenFox's `evaluateCondition`).
fn holds(when: &Value, result: Option<&str>, state: &ThreadState) -> bool {
    let s = |k: &str| when.get(k).and_then(Value::as_str).unwrap_or("");
    let entries = |key: &str| state.metadata.get(key).cloned().unwrap_or_default();
    let field = |e: &super::state::Entry, f: &str| -> String {
        match f {
            "id" => e.id.clone(),
            "description" => e.description.clone(),
            "reason" => e.reason.clone(),
            _ => e.status.clone(),
        }
    };
    match s("type") {
        "always" => true,
        "step_result" => result == Some(s("result")),
        "metadata_all_match" => entries(s("key"))
            .iter()
            .all(|e| field(e, s("field")) == s("value")),
        "metadata_all_in" => {
            let values: Vec<&str> = when
                .get("values")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            entries(s("key"))
                .iter()
                .all(|e| values.contains(&field(e, s("field")).as_str()))
        }
        _ => false,
    }
}

/// The branches a step for the person offers: one per `step_result`, "Continue" for `always`.
fn user_choices(step: &Step, steps: &[Step]) -> Vec<proto::StepChoice> {
    let mut out: Vec<proto::StepChoice> = Vec::new();
    for t in &step.transitions {
        let kind = t.when.get("type").and_then(Value::as_str).unwrap_or("");
        let (id, label) = match kind {
            "step_result" => {
                let r = t.when.get("result").and_then(Value::as_str).unwrap_or("");
                (r.to_string(), r.to_string())
            }
            "always" => (CONTINUE.to_string(), "Continue".to_string()),
            _ => continue,
        };
        if id.is_empty() || out.iter().any(|c| c.id == id) {
            continue;
        }
        out.push(proto::StepChoice {
            id,
            label,
            next_step_name: steps.iter().find(|s| s.id == t.goto).map(Step::shown),
        });
    }
    out
}

/// What a prompt's `{{…}}` stand for.
struct Context {
    workdir: String,
    reason: String,
    criteria_count: usize,
    pending_count: usize,
    criteria_list: String,
    modified_files: String,
    step_output: BTreeMap<String, String>,
    params: BTreeMap<String, String>,
}

fn resolve(template: &str, c: &Context) -> String {
    let mut out = template
        .replace("{{workdir}}", &c.workdir)
        .replace("{{reason}}", &c.reason)
        .replace("{{criteriaCount}}", &c.criteria_count.to_string())
        .replace("{{pendingCount}}", &c.pending_count.to_string())
        .replace("{{criteriaList}}", &c.criteria_list)
        .replace("{{modifiedFiles}}", &c.modified_files)
        .replace(
            "{{verifierFindings}}",
            c.step_output
                .get("content")
                .map(String::as_str)
                .unwrap_or(""),
        )
        .replace(
            "{{previousStepOutput}}",
            c.step_output
                .get("stdout")
                .map(String::as_str)
                .unwrap_or(""),
        );
    // {{stepOutput.<key>}}, a parallel child's keys too ({{stepOutput.lint.stdout}})
    while let Some(at) = out.find("{{stepOutput.") {
        let rest = &out[at + 13..];
        let Some(end) = rest.find("}}") else { break };
        let key = &rest[..end];
        let value = c.step_output.get(key).cloned().unwrap_or_default();
        out.replace_range(at..at + 13 + end + 2, &value);
    }
    for (k, v) in &c.params {
        out = out.replace(&format!("{{{{{k}}}}}"), v);
    }
    out
}

/// The files changed in the folder, as Git sees them.
async fn modified_files(workdir: &Path) -> String {
    if !workdir.join(".git").exists() {
        return "(the folder is not a Git repository)".into();
    }
    let (ok, out) = tools::run_command(
        workdir,
        "git status --porcelain",
        Duration::from_secs(30),
        false,
    )
    .await;
    let files: Vec<String> = out
        .lines()
        .skip(1)
        .filter(|l| l.len() > 3 && ok)
        .map(|l| format!("- {}", l[3..].trim()))
        .collect();
    if files.is_empty() {
        "(no modified files)".into()
    } else {
        files.join("\n")
    }
}

/// The exit code `run_command` gives on its first line.
fn exit_code(out: &str) -> i64 {
    out.strip_prefix("exit code ")
        .and_then(|r| r.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(1)
}

impl LocalEngine {
    /// The workflows a message may launch, as the panel lists them.
    pub(crate) fn workflow_list(&self, workdir: Option<&Path>) -> Vec<proto::Workflow> {
        definitions(&self.dir, workdir)
            .into_iter()
            .map(|(d, scope)| {
                let mut groups: Vec<String> = Vec::new();
                for g in d.steps.iter().filter_map(|s| s.sub_group.clone()) {
                    if !groups.contains(&g) {
                        groups.push(g);
                    }
                }
                proto::Workflow {
                    id: d.metadata.id,
                    name: d.metadata.name,
                    description: d.metadata.description,
                    color: d.metadata.color,
                    parameters: d.metadata.parameters,
                    sub_groups: Some(groups).filter(|g| !g.is_empty()),
                    scope: Some(scope),
                }
            })
            .collect()
    }

    fn keep_flow(&self, t: &Turn, flow: &Flow) {
        let mut state = ThreadState::load(&t.path());
        state.flow = Some(flow.clone());
        let _ = state.save(&t.path(), &author(t));
        self.fill_panel(&t.agent, &self.mode_for(t), &state, Some(&t.workdir));
        self.show(&t.runner, &t.agent, true);
    }

    async fn context(&self, t: &Turn, flow: &Flow) -> Context {
        let state = ThreadState::load(&t.path());
        let criteria = state.metadata.get("criteria").cloned().unwrap_or_default();
        let pending = criteria
            .iter()
            .filter(|c| !["completed", "passed"].contains(&c.status.as_str()))
            .count();
        Context {
            workdir: t.workdir.display().to_string(),
            reason: format!("{pending} criteria remaining"),
            criteria_count: criteria.len(),
            pending_count: pending,
            criteria_list: state.criteria_list(),
            modified_files: modified_files(&t.workdir).await,
            step_output: flow.output.clone(),
            params: flow.params.clone(),
        }
    }

    /// A workflow's turn: launched now, or taken up where it waits.
    pub(crate) async fn flow(&self, t: &Turn, l: &Launch) -> Result<RunSummary, String> {
        let s = self.settings();
        let (server, model, shown_model) = self.model_for(t)?;
        self.mcp_ready().await;
        let all = definitions(&self.dir, Some(&t.workdir));
        let state = ThreadState::load(&t.path());
        let (mut flow, def) = if l.resume {
            let f = state
                .flow
                .clone()
                .filter(|f| f.status != "completed")
                .ok_or("no workflow waits in this thread")?;
            let d = all
                .iter()
                .find(|(d, _)| d.metadata.id == f.workflow)
                .map(|(d, _)| d.clone())
                .ok_or_else(|| format!("the workflow {:?} is gone", f.workflow))?;
            (f, d)
        } else {
            let d = all
                .iter()
                .find(|(d, _)| d.metadata.id == l.id)
                .map(|(d, _)| d.clone())
                .ok_or_else(|| format!("no workflow {:?}", l.id))?;
            if let Some(p) = d
                .metadata
                .parameters
                .iter()
                .find(|p| p.required && l.params.get(&p.id).is_none_or(|v| v.trim().is_empty()))
            {
                return Err(format!("the workflow needs {}", p.label));
            }
            let mut f = Flow {
                workflow: d.metadata.id.clone(),
                name: d.metadata.name.clone(),
                color: d.metadata.color.clone(),
                status: "running".into(),
                step: d.entry_step.clone(),
                params: l.params.clone(),
                sub_group: l.sub_group.clone(),
                ..Default::default()
            };
            if let Some(g) = &l.sub_group {
                f.active_steps = d
                    .steps
                    .iter()
                    .filter(|x| x.sub_group.as_deref() == Some(g))
                    .map(|x| x.id.clone())
                    .collect();
                f.active_groups = vec![g.clone()];
                f.step = f
                    .active_steps
                    .first()
                    .cloned()
                    .ok_or_else(|| format!("the workflow has no sub-group {g:?}"))?;
            }
            (f, d)
        };
        let resume_step = l.resume.then(|| flow.step.clone());
        let mut choice = l
            .resume
            .then(|| l.choice.clone().unwrap_or_else(|| CONTINUE.into()));
        let mut resume_used = false;
        flow.status = "running".into();
        flow.choices.clear();
        let max = def
            .settings
            .as_ref()
            .map(|x| x.max_iterations)
            .unwrap_or(50);

        // the conversation the agent steps share
        let system = self.system_for(t, &shown_model);
        let past = self.past(t, &server, &model, &system, &t.text, &s).await?;
        let mut messages = vec![json!({"role": "system", "content": system})];
        messages.extend(past);
        if !t.text.trim().is_empty() {
            messages.push(json!({"role": "user", "content": t.text}));
        }
        let agents = defs::agents(&self.dir, Some(&t.workdir));
        self.edit_panel(&t.agent, |p| {
            p.running = true;
            p.error.clear();
        });
        let mut last_text = String::new();
        let mut open: HashMap<String, bool> = HashMap::new();
        let summary_of = |stopped: &str, text: &str| RunSummary {
            stopped: stopped.into(),
            final_text: text.to_string(),
            ..Default::default()
        };

        loop {
            if flow.iterations >= max {
                flow.status = "blocked".into();
                self.keep_flow(t, &flow);
                t.note(
                    EventKind::Checkpoint,
                    json!({"reason": "workflow_blocked", "detail": format!("{} steps run, the workflow's limit", max)}),
                );
                return Ok(summary_of("done", &last_text));
            }
            flow.iterations += 1;
            let Some(step) = def.steps.iter().find(|x| x.id == flow.step).cloned() else {
                flow.status = "blocked".into();
                self.keep_flow(t, &flow);
                return Err(format!("the workflow has no step {:?}", flow.step));
            };
            if flow.step_name != step.shown() || flow.iterations == 1 {
                t.note(
                    EventKind::Checkpoint,
                    json!({"reason": "workflow_step", "detail": format!("{} — {}", flow.name, step.shown())}),
                );
            }
            flow.step_name = step.shown();
            self.keep_flow(t, &flow);
            self.edit_panel(&t.agent, |p| p.phase = step.shown());
            let ctx = self.context(t, &flow).await;
            let resuming_here = resume_step.as_deref() == Some(step.id.as_str()) && !resume_used;

            let (result, output): (String, BTreeMap<String, String>) = match step.kind.as_str() {
                "agent" => {
                    let id = step.agent_id.clone().unwrap_or_else(|| "builder".into());
                    let agent = agents
                        .iter()
                        .find(|a| a.id == id && !a.subagent)
                        .cloned()
                        .ok_or_else(|| format!("the workflow's agent {id:?} is unknown"))?;
                    let tools = self.tools_for(&agent, &["step_done"]);
                    let entered = flow.entered.contains(&step.id);
                    let said = if resuming_here {
                        None
                    } else if !entered {
                        Some(match &step.prompt {
                            Some(p) => resolve(p, &ctx) + STEP_DONE_PROMPT,
                            None => KICKOFF.to_string() + STEP_DONE_PROMPT,
                        })
                    } else {
                        // the step's condition met already: only the reminder
                        let state = ThreadState::load(&t.path());
                        let fired = step
                            .transitions
                            .iter()
                            .find(|x| holds(&x.when, None, &state));
                        let satisfied = fired.is_some_and(|x| x.goto != step.id);
                        let mut parts = Vec::new();
                        if let (Some(n), false) = (&step.nudge_prompt, satisfied) {
                            parts.push(resolve(n, &ctx));
                        }
                        parts.push(
                            if satisfied {
                                STEP_DONE_REMINDER
                            } else {
                                STEP_DONE_NUDGE
                            }
                            .to_string(),
                        );
                        Some(parts.join("\n\n"))
                    };
                    if let Some(said) = said {
                        messages.push(json!({"role": "user", "content":
                            format!("{}\n\n{said}", prompts::reminder(&agent, &tools))}));
                    }
                    self.edit_panel(&t.agent, |p| p.agent = agent.id.clone());
                    let mut run = Run {
                        server: server.clone(),
                        model: model.clone(),
                        agent,
                        tools,
                        messages: std::mem::take(&mut messages),
                        sub: None,
                        max_steps: s.max_steps,
                    };
                    let end = self.agent_loop(t, &mut run).await;
                    messages = run.messages;
                    let end = match end {
                        Ok(e) => e,
                        Err(e) => {
                            flow.status = "blocked".into();
                            self.keep_flow(t, &flow);
                            return Err(format!("{}: {e}", step.shown()));
                        }
                    };
                    if !end.text.is_empty() {
                        last_text = end.text.clone();
                    }
                    if let Some(why) = end.stopped {
                        flow.status = "blocked".into();
                        self.keep_flow(t, &flow);
                        return Ok(summary_of(&why, &last_text));
                    }
                    if !flow.entered.contains(&step.id) {
                        flow.entered.push(step.id.clone());
                    }
                    resume_used = true;
                    let mut output = BTreeMap::new();
                    if let Some((content, result)) = &end.returned {
                        output.insert("content".into(), content.clone());
                        if let Some(r) = result {
                            output.insert("result".into(), r.clone());
                        }
                    }
                    output.insert("stepDoneCalled".into(), end.step_done.to_string());
                    if !end.step_done {
                        // not done: the same step again, with the nudge
                        flow.output = output;
                        continue;
                    }
                    let result = end
                        .returned
                        .and_then(|(_, r)| r)
                        .unwrap_or_else(|| "completed".into());
                    (result, output)
                }
                "sub_agent" | "shell" => {
                    let (r, o) = self.child(t, &step, &ctx, &server, &model, &mut open).await;
                    if t.cancelled() || !t.running() {
                        flow.status = "blocked".into();
                        self.keep_flow(t, &flow);
                        return Ok(summary_of(
                            if t.cancelled() { "cancelled" } else { "frozen" },
                            &last_text,
                        ));
                    }
                    if step.kind == "shell" {
                        // the agent steps read the command's outcome
                        messages.push(json!({"role": "user", "content": format!("Exit code: {}\n```\n{}\n```",
                            o.get("exitCode").cloned().unwrap_or_default(),
                            o.get("stdout").cloned().unwrap_or_default().chars().take(10_000).collect::<String>())}));
                    }
                    (r, o)
                }
                "parallel" => {
                    // run one after the other: the same outcome, a simpler machine
                    let mut output = BTreeMap::new();
                    let mut ok = true;
                    let mut lines = Vec::new();
                    for c in &step.children {
                        let (r, o) = self.child(t, c, &ctx, &server, &model, &mut open).await;
                        if t.cancelled() || !t.running() {
                            break;
                        }
                        ok &= r == "success";
                        lines.push(format!("{}: {r}", c.id));
                        for (k, v) in o {
                            output.insert(format!("{}.{k}", c.id), v);
                        }
                    }
                    if t.cancelled() || !t.running() {
                        flow.status = "blocked".into();
                        self.keep_flow(t, &flow);
                        return Ok(summary_of(
                            if t.cancelled() { "cancelled" } else { "frozen" },
                            &last_text,
                        ));
                    }
                    output.insert("summary".into(), lines.join("\n"));
                    (if ok { "success" } else { "failure" }.into(), output)
                }
                "user" => {
                    if resuming_here {
                        resume_used = true;
                        (
                            choice.take().unwrap_or_else(|| CONTINUE.into()),
                            flow.output.clone(),
                        )
                    } else {
                        flow.status = "waiting".into();
                        flow.choices = user_choices(&step, &def.steps);
                        self.keep_flow(t, &flow);
                        t.note(
                            EventKind::Checkpoint,
                            json!({"reason": "workflow_waiting", "detail": step.shown()}),
                        );
                        return Ok(summary_of("done", &last_text));
                    }
                }
                other => {
                    flow.status = "blocked".into();
                    self.keep_flow(t, &flow);
                    return Err(format!("a workflow step of an unknown type: {other:?}"));
                }
            };
            flow.results.insert(step.id.clone(), result.clone());
            flow.output = output;

            // the next step: the first transition that holds (in a slice, its own and the
            // ones tagged with a sub-group it entered)
            let state = ThreadState::load(&t.path());
            let fired = step
                .transitions
                .iter()
                .filter(|x| {
                    flow.sub_group.is_none()
                        || x.sub_group
                            .as_ref()
                            .is_none_or(|g| flow.active_groups.contains(g))
                })
                .find(|x| holds(&x.when, Some(&result), &state))
                .cloned();
            let mut next = fired
                .as_ref()
                .map(|x| x.goto.clone())
                .unwrap_or_else(|| BLOCKED.into());
            if flow.sub_group.is_some()
                && next != DONE
                && next != BLOCKED
                && !flow.active_steps.contains(&next)
            {
                match fired
                    .and_then(|x| x.sub_group)
                    .filter(|g| flow.active_groups.contains(g))
                {
                    Some(_) => {
                        flow.active_steps.push(next.clone());
                        if let Some(g) = def
                            .steps
                            .iter()
                            .find(|x| x.id == next)
                            .and_then(|x| x.sub_group.clone())
                        {
                            if !flow.active_groups.contains(&g) {
                                flow.active_groups.push(g);
                            }
                        }
                    }
                    None => next = DONE.into(),
                }
            }
            match next.as_str() {
                DONE => {
                    flow.status = "completed".into();
                    self.keep_flow(t, &flow);
                    t.note(
                        EventKind::Checkpoint,
                        json!({"reason": "workflow_done", "detail": flow.name}),
                    );
                    return Ok(summary_of("done", &last_text));
                }
                BLOCKED => {
                    flow.status = "blocked".into();
                    self.keep_flow(t, &flow);
                    t.note(
                        EventKind::Checkpoint,
                        json!({"reason": "workflow_blocked", "detail": step.shown()}),
                    );
                    return Ok(summary_of("done", &last_text));
                }
                _ => flow.step = next,
            }
        }
    }

    /// A sub-agent's step or a shell command's: its result and output.
    async fn child(
        &self,
        t: &Turn,
        step: &Step,
        ctx: &Context,
        server: &super::llm::Server,
        model: &str,
        open: &mut HashMap<String, bool>,
    ) -> (String, BTreeMap<String, String>) {
        let error = |e: String| {
            (
                "error".to_string(),
                BTreeMap::from([("error".to_string(), e)]),
            )
        };
        match step.kind.as_str() {
            "sub_agent" => {
                let kind = step.sub_agent_type.clone().unwrap_or_default();
                let prompt = step
                    .prompt
                    .as_deref()
                    .map(|p| resolve(p, ctx))
                    .unwrap_or_else(|| KICKOFF.into());
                let parent = Run {
                    server: server.clone(),
                    model: model.to_string(),
                    agent: Default::default(),
                    tools: Vec::new(),
                    messages: Vec::new(),
                    sub: None,
                    max_steps: 0,
                };
                match self.sub_agent(t, &parent, &kind, &prompt).await {
                    Ok((content, result)) => {
                        let mut o = BTreeMap::from([("content".to_string(), content)]);
                        if let Some(r) = &result {
                            o.insert("result".into(), r.clone());
                        }
                        (result.unwrap_or_else(|| "success".into()), o)
                    }
                    Err(e) => error(e),
                }
            }
            "shell" => {
                let Some(command) = step.command.as_deref().map(|c| resolve(c, ctx)) else {
                    return error("Missing command".into());
                };
                if !t.pass.is_empty() && !t.runner.approve(&t.agent, &command).await {
                    return error("the person did not allow this command".into());
                }
                let id = format!("wf-{}", crate::wire::random_hex(4));
                let (mut nothing, mut summary) = (String::new(), RunSummary::default());
                acp_update(
                    t,
                    &json!({"sessionUpdate": "tool_call", "toolCallId": id, "title": format!("Run: {command}"),
                            "kind": "execute", "rawInput": {"command": command}}),
                    &mut nothing,
                    open,
                    &mut summary,
                );
                let timeout =
                    Duration::from_millis(step.timeout.unwrap_or(60_000).clamp(1000, 3_600_000));
                let (_, out) =
                    tools::run_command(&t.workdir, &command, timeout, !t.pass.is_empty()).await;
                let code = exit_code(&out);
                let ok = step
                    .success_exit_codes
                    .clone()
                    .unwrap_or_else(|| vec![0])
                    .contains(&code);
                acp_update(
                    t,
                    &json!({"sessionUpdate": "tool_call_update", "toolCallId": id,
                            "status": if ok { "completed" } else { "failed" }, "content": [], "rawOutput": out}),
                    &mut nothing,
                    open,
                    &mut summary,
                );
                let body = out.split_once('\n').map(|x| x.1).unwrap_or("").to_string();
                (
                    if ok { "success" } else { "failure" }.into(),
                    BTreeMap::from([
                        ("stdout".to_string(), body),
                        ("stderr".to_string(), String::new()),
                        ("exitCode".to_string(), code.to_string()),
                    ]),
                )
            }
            other => error(format!("a child step of an unknown type: {other:?}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::state::Entry;
    use super::*;

    #[test]
    fn the_builtin_workflow_reads() {
        let d: Vec<_> = defs::BUILTIN_WORKFLOWS
            .iter()
            .map(|raw| serde_json::from_str::<Definition>(raw).unwrap())
            .collect();
        assert_eq!(d[0].entry_step, "build");
        assert!(d[0].steps.iter().any(|s| s.kind == "sub_agent"));
    }

    #[test]
    fn transitions_and_templates_work_as_openfoxs() {
        let mut state = ThreadState::default();
        let all_in = json!({"type": "metadata_all_in", "key": "criteria", "field": "status", "values": ["completed", "passed"]});
        assert!(holds(&all_in, None, &state), "no criteria: it holds");
        state.metadata.insert(
            "criteria".into(),
            vec![Entry {
                id: "1".into(),
                description: "x".into(),
                status: "pending".into(),
                reason: String::new(),
            }],
        );
        assert!(!holds(&all_in, None, &state));
        assert!(holds(
            &json!({"type": "step_result", "result": "happy"}),
            Some("happy"),
            &state
        ));
        let c = Context {
            workdir: "/w".into(),
            reason: "1 criteria remaining".into(),
            criteria_count: 1,
            pending_count: 1,
            criteria_list: "- 1".into(),
            modified_files: "- a".into(),
            step_output: BTreeMap::from([
                ("content".into(), "found".into()),
                ("lint.stdout".into(), "ok".into()),
            ]),
            params: BTreeMap::from([("topic".into(), "parsers".into())]),
        };
        assert_eq!(
            resolve("{{criteriaCount}} in {{workdir}}: {{stepOutput.content}} {{stepOutput.lint.stdout}} {{verifierFindings}} {{topic}} {{stepOutput.none}}.", &c),
            "1 in /w: found ok found parsers ."
        );
        let step = Step {
            id: "pick".into(),
            kind: "user".into(),
            transitions: vec![
                Transition {
                    when: json!({"type": "step_result", "result": "happy"}),
                    goto: "a".into(),
                    sub_group: None,
                },
                Transition {
                    when: json!({"type": "always"}),
                    goto: "a".into(),
                    sub_group: None,
                },
            ],
            ..Default::default()
        };
        let choices = user_choices(&step, &[]);
        assert_eq!(
            choices.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            ["happy", "continue"]
        );
        assert_eq!(exit_code("exit code 3 (0.1 s)\nboom"), 3);
    }
}
