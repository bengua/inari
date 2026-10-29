//! What a thread on the local engine shows beside its conversation: the same panel as an OpenFox
//! thread's (the Plan tab, the live cards), and what the person may change of it.

use super::state::ThreadState;
use super::LocalEngine;
use crate::agent::Runner;
use crate::openfox::{ContextView, Panel, Question, SubAgent};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How often the panel goes to the pages while text streams in it (the thinking, a sub-agent).
const PANEL_EVERY: Duration = Duration::from_millis(250);

impl LocalEngine {
    pub(crate) fn edit_panel(&self, agent: &str, change: impl FnOnce(&mut Panel)) {
        let mut all = self.panels.lock().expect("not poisoned");
        change(all.entry(agent.to_string()).or_default());
    }

    /// Send the panel to the pages; `now` false waits for PANEL_EVERY since the last time.
    pub(crate) fn show(&self, runner: &Runner, agent: &str, now: bool) {
        {
            let mut shown = self.shown.lock().expect("not poisoned");
            let last = shown.get(agent).copied();
            if !now && last.is_some_and(|at| at.elapsed() < PANEL_EVERY) {
                return;
            }
            shown.insert(agent.to_string(), Instant::now());
        }
        let panel = self
            .panels
            .lock()
            .expect("not poisoned")
            .get(agent)
            .cloned()
            .unwrap_or_default();
        runner.tell(
            &format!("panel.{agent}"),
            json!({"agent": agent, "panel": panel}),
        );
    }

    /// The panel as the thread's state and the engine's settings make it.
    pub(crate) fn fill_panel(
        &self,
        agent: &str,
        mode: &str,
        state: &ThreadState,
        workdir: Option<&Path>,
    ) {
        let modes = self.modes(workdir);
        let workflows = self.workflow_list(workdir);
        let window = self.settings().context_window;
        let queue: Vec<Value> = self
            .queues
            .lock()
            .expect("not poisoned")
            .get(agent)
            .cloned()
            .unwrap_or_default()
            .iter()
            .enumerate()
            .map(|(i, q)| json!({"id": i.to_string(), "content": q}))
            .collect();
        self.edit_panel(agent, |p| {
            p.agent = mode.to_string();
            p.agents = modes;
            p.danger = if state.danger == "dangerous" {
                "dangerous"
            } else {
                "normal"
            }
            .into();
            p.todos = state.items("todos");
            p.criteria = state.items("criteria");
            p.metadata = state
                .metadata
                .keys()
                .filter(|k| !["todos", "criteria"].contains(&k.as_str()))
                .map(|k| (k.clone(), state.items(k)))
                .collect();
            p.paused = "none".into();
            p.queue = queue;
            p.workflows = workflows;
            p.workflow = state.flow.as_ref().map(super::workflow::flow_view);
            let used = p.context.as_ref().map(|c| c.used).unwrap_or(0);
            p.context = Some(ContextView {
                used,
                max: window,
                compactions: state.compactions,
                danger: used * 10 > window * 8,
                can_compact: true,
            });
        });
    }

    /// A message sent while the thread works: the model reads it at its next step.
    pub fn join(&self, agent: &str, text: &str) -> bool {
        if !self.active.lock().expect("not poisoned").contains(agent) {
            return false;
        }
        self.queues
            .lock()
            .expect("not poisoned")
            .entry(agent.to_string())
            .or_default()
            .push(text.to_string());
        true
    }

    pub(crate) fn take_queue(&self, agent: &str) -> Vec<String> {
        self.queues
            .lock()
            .expect("not poisoned")
            .remove(agent)
            .unwrap_or_default()
    }

    /// Ask the person; their answer (None: they skipped it, or the turn stopped).
    pub(crate) async fn ask(
        &self,
        t: &crate::agent::Turn,
        call: &str,
        question: Question,
    ) -> Option<String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.questions
            .lock()
            .expect("not poisoned")
            .insert(call.to_string(), tx);
        self.edit_panel(&t.agent, |p| p.question = Some(question));
        self.show(&t.runner, &t.agent, true);
        t.phase("listening");
        let answer = tokio::select! {
            a = rx => a.ok().flatten(),
            _ = super::run::stopped(t) => None,
        };
        self.questions.lock().expect("not poisoned").remove(call);
        self.edit_panel(&t.agent, |p| p.question = None);
        self.show(&t.runner, &t.agent, true);
        answer
    }

    /// Something done on the thread's panel (the Plan tab, a card): the panel says how it went.
    pub async fn thread_action(
        &self,
        runner: &Arc<Runner>,
        agent: &str,
        action: &str,
        p: &Value,
    ) -> Result<(), String> {
        let capsule = runner.host.agents.join(agent);
        let author = format!("{}@{agent}", runner.host.node_id);
        let workdir = runner.host.workdir(agent);
        let text = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let mut state = ThreadState::load(&capsule);
        let result: Result<(), String> = match action {
            "agent" => {
                let mode = text("mode");
                if !self
                    .modes(Some(&workdir))
                    .iter()
                    .any(|m| m["id"] == mode.as_str())
                {
                    Err(format!("no agent {mode:?} for the local engine"))
                } else {
                    runner.set_mode(agent, &mode).map(|_| ())
                }
            }
            "danger" => {
                state.danger = if text("level") == "dangerous" {
                    "dangerous"
                } else {
                    "normal"
                }
                .into();
                state.save(&capsule, &author)
            }
            "criteria" => {
                let old = state.metadata.get("criteria").cloned().unwrap_or_default();
                let mut kept = Vec::new();
                for c in p
                    .get("criteria")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let d = c
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if d.is_empty() {
                        continue;
                    }
                    let id = c
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|i| !i.is_empty());
                    let status = old
                        .iter()
                        .find(|o| Some(o.id.as_str()) == id)
                        .map(|o| o.status.clone())
                        .unwrap_or_else(|| "pending".into());
                    let id = match id {
                        Some(i) => i.to_string(),
                        None => {
                            state.next_id += 1;
                            state.next_id.to_string()
                        }
                    };
                    kept.push(super::state::Entry {
                        id,
                        description: d,
                        status,
                        reason: String::new(),
                    });
                }
                state.metadata.insert("criteria".into(), kept);
                state.save(&capsule, &author)
            }
            "answer" => {
                let call = text("call_id");
                let tx = self.questions.lock().expect("not poisoned").remove(&call);
                match tx {
                    Some(tx) => {
                        let _ = tx.send(p.get("answer").and_then(Value::as_str).map(String::from));
                        Ok(())
                    }
                    None => Err("this question is no longer asked".into()),
                }
            }
            "compact" => {
                self.compact_next
                    .lock()
                    .expect("not poisoned")
                    .insert(agent.to_string());
                Ok(())
            }
            "dequeue" => {
                let at: usize = text("id")
                    .parse()
                    .map_err(|_| "unknown message".to_string())?;
                let mut q = self.queues.lock().expect("not poisoned");
                match q.get_mut(agent).filter(|l| at < l.len()) {
                    Some(l) => {
                        l.remove(at);
                        Ok(())
                    }
                    None => Err("this message was already read".into()),
                }
            }
            "workflow_exit" => match state.flow.as_mut() {
                Some(f) if ["running", "waiting", "blocked"].contains(&f.status.as_str()) => {
                    f.status = "cancelled".into();
                    state.save(&capsule, &author)
                }
                _ => Err("no workflow runs in this thread".into()),
            },
            "refresh" => Ok(()),
            "pause" | "resume" | "retry" | "retry_now" => Err(
                "the local engine has no pause: Stop ends the step, a new message goes on".into(),
            ),
            other => self.project_action(runner, agent, other, p).await,
        };
        // what the thread is now, read back
        let state = ThreadState::load(&capsule);
        let mode = runner.mode_of(agent);
        let mode = if mode.is_empty() {
            self.settings().mode
        } else {
            mode
        };
        self.fill_panel(agent, &mode, &state, Some(&workdir));
        self.fill_project(agent, &workdir).await;
        self.edit_panel(agent, |x| {
            x.error = result.as_ref().err().cloned().unwrap_or_default()
        });
        self.show(runner, agent, true);
        result
    }

    /// A sub-agent's card in the thread.
    pub(crate) fn sub_started(&self, agent: &str, kind: &str) {
        self.edit_panel(agent, |p| {
            p.subagents.retain(|a| !a.done);
            p.subagents.push(SubAgent {
                kind: kind.into(),
                text: String::new(),
                done: false,
            });
        });
    }

    pub(crate) fn sub_said(&self, agent: &str, kind: &str, text: &str) {
        self.edit_panel(agent, |p| {
            if let Some(a) = p
                .subagents
                .iter_mut()
                .rev()
                .find(|a| a.kind == kind && !a.done)
            {
                a.text.push_str(text);
                let n = a.text.chars().count();
                if n > 6000 {
                    a.text = a.text.chars().skip(n - 6000).collect();
                }
            }
        });
    }

    pub(crate) fn sub_done(&self, agent: &str, kind: &str) {
        self.edit_panel(agent, |p| {
            if let Some(a) = p
                .subagents
                .iter_mut()
                .rev()
                .find(|a| a.kind == kind && !a.done)
            {
                a.done = true;
            }
        });
    }
}
