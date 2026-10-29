//! A thread's folder as a project for the local engine, as OpenFox shows one: its task board
//! (`.openfox/tasks.json`), its dev server (`.openfox/dev.json`, run by the core), the
//! background processes its agent started, its Git branches, its instructions
//! (`.openfox/instructions.md`), and the forges it is cloned from and published to. All in the
//! folder, so it travels with the thread; the processes stay on this core, in the kept folder.

use super::forge::{branch_like, git};
use super::state::ThreadState;
use super::LocalEngine;
use crate::agent::{Runner, Turn};
use crate::capsule::io;
use crate::openfox::proto::{DevConfig, DevServer, Process, Task, TaskSettings};
use crate::openfox::ProjectView;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// The lines of output kept for a process.
const LOG_LINES: usize = 500;

/// A process the core runs in a thread's folder.
#[derive(Clone)]
struct Proc {
    id: String,
    name: String,
    command: String,
    pid: Option<u32>,
    log: Arc<Mutex<VecDeque<String>>>,
    /// Its exit code once it ended.
    end: Arc<Mutex<Option<i64>>>,
    stopped: Arc<std::sync::atomic::AtomicBool>,
}

impl Proc {
    fn running(&self) -> bool {
        self.end.lock().expect("not poisoned").is_none()
    }

    fn view(&self) -> Process {
        let end = *self.end.lock().expect("not poisoned");
        Process {
            id: self.id.clone(),
            name: self.name.clone(),
            command: self.command.clone(),
            status: match end {
                None => "running".into(),
                Some(_) if self.stopped.load(std::sync::atomic::Ordering::Relaxed) => {
                    "killed".into()
                }
                Some(_) => "exited".into(),
            },
            exit_code: end,
        }
    }

    fn lines(&self, n: usize) -> Vec<String> {
        let log = self.log.lock().expect("not poisoned");
        log.iter()
            .skip(log.len().saturating_sub(n))
            .cloned()
            .collect()
    }

    async fn stop(&self) {
        if !self.running() {
            return;
        }
        self.stopped
            .store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(pid) = self.pid {
            // SAFETY: signalling the process group this module started.
            unsafe { libc::kill(-(pid as i32), libc::SIGTERM) };
            for _ in 0..30 {
                if !self.running() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            // SAFETY: as above.
            unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
        }
    }
}

fn start(dir: &Path, name: &str, command: &str, env: &[(String, String)]) -> Result<Proc, String> {
    let mut cmd = Command::new("/bin/sh");
    cmd.args(["-c", command])
        .current_dir(dir)
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = cmd.spawn().map_err(|e| format!("cannot start it: {e}"))?;
    let p = Proc {
        id: crate::wire::random_hex(4),
        name: name.into(),
        command: command.into(),
        pid: child.id(),
        log: Arc::default(),
        end: Arc::default(),
        stopped: Arc::default(),
    };
    for out in [
        child
            .stdout
            .take()
            .map(|o| Box::new(o) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
        child
            .stderr
            .take()
            .map(|o| Box::new(o) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let log = Arc::clone(&p.log);
        tokio::spawn(async move {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                let mut log = log.lock().expect("not poisoned");
                log.push_back(l.chars().take(2000).collect());
                while log.len() > LOG_LINES {
                    log.pop_front();
                }
            }
        });
    }
    let end = Arc::clone(&p.end);
    tokio::spawn(async move {
        let code = child.wait().await.ok().and_then(|s| s.code()).unwrap_or(-1);
        *end.lock().expect("not poisoned") = Some(code as i64);
    });
    Ok(p)
}

/// What runs for each thread on this core.
#[derive(Default)]
pub(crate) struct Works {
    dev: Mutex<HashMap<String, Proc>>,
    procs: Mutex<HashMap<String, Vec<Proc>>>,
    /// The dev server's last lines, shown once asked for.
    shown_logs: Mutex<HashMap<String, Vec<String>>>,
}

fn openfox_dir(workdir: &Path) -> PathBuf {
    workdir.join(".openfox")
}

fn read_tasks(workdir: &Path) -> Vec<Task> {
    std::fs::read(openfox_dir(workdir).join("tasks.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_json(path: &Path, v: &impl serde::Serialize) -> Result<(), String> {
    std::fs::create_dir_all(path.parent().expect("in a folder")).map_err(|e| e.to_string())?;
    io::atomic_write(path, &serde_json::to_vec_pretty(v).expect("serializes"))
        .map_err(|e| e.to_string())
}

fn dev_config(workdir: &Path) -> Option<DevConfig> {
    std::fs::read(openfox_dir(workdir).join("dev.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
}

/// The port of an address (http://localhost:5173 → 5173), for $PORT.
fn port_of(url: &str) -> Option<String> {
    let rest = url.split_once("://").map(|x| x.1).unwrap_or(url);
    let host = rest.split('/').next()?;
    host.rsplit_once(':')
        .map(|x| x.1.to_string())
        .filter(|p| p.parse::<u16>().is_ok())
}

/// A task's turn, in the background; once it ends well, the task is done (unless moved meanwhile).
fn run_task(r: Arc<Runner>, a: String, prompt: String, id: String) {
    let job: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> =
        Box::pin(async move {
            let out = r.run(&a, &prompt, Vec::new()).await;
            let Some(h) = r.local_of(&a) else { return };
            let e = h.local().expect("the local engine");
            let w = r.host.workdir(&a);
            if out.stopped == "done"
                && read_tasks(&w)
                    .iter()
                    .any(|t| t.id == id && t.status == "in_progress")
            {
                let _ = e.task_move(&w, &id, "done", None);
                r.put_away(&a);
            }
            let _ = e.thread_action(&r, &a, "refresh", &json!({})).await;
        });
    tokio::spawn(job);
}

impl LocalEngine {
    fn task_change(
        &self,
        workdir: &Path,
        change: impl FnOnce(&mut Vec<Task>) -> Result<String, String>,
    ) -> Result<String, String> {
        let mut tasks = read_tasks(workdir);
        let out = change(&mut tasks)?;
        write_json(&openfox_dir(workdir).join("tasks.json"), &tasks)?;
        Ok(out)
    }

    fn task_add(&self, workdir: &Path, prompt: &str) -> Result<String, String> {
        let prompt = prompt.trim();
        if prompt.is_empty() || prompt.len() > 20_000 {
            return Err("a task is what to do: not empty, 20,000 characters at most".into());
        }
        self.task_change(workdir, |tasks| {
            let id = (tasks
                .iter()
                .filter_map(|t| t.id.parse::<u64>().ok())
                .max()
                .unwrap_or(0)
                + 1)
            .to_string();
            tasks.push(Task {
                id: id.clone(),
                prompt: prompt.into(),
                status: "todo".into(),
                run_state: None,
                queue_position: None,
                version: 1,
                active_session_id: None,
                agent_id: None,
            });
            Ok(format!("Added task {id}."))
        })
    }

    fn task_move(
        &self,
        workdir: &Path,
        id: &str,
        to: &str,
        run_state: Option<&str>,
    ) -> Result<Task, String> {
        if !["todo", "in_progress", "done"].contains(&to) {
            return Err(format!(
                "a task goes to todo, in_progress or done, not {to:?}"
            ));
        }
        let mut moved = None;
        self.task_change(workdir, |tasks| {
            let t = tasks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or_else(|| format!("no task {id:?}"))?;
            t.status = to.into();
            t.run_state = if to == "in_progress" {
                run_state.map(String::from)
            } else {
                None
            };
            t.version += 1;
            moved = Some(t.clone());
            Ok(String::new())
        })?;
        Ok(moved.expect("moved"))
    }

    async fn dev_start(&self, agent: &str, workdir: &Path) -> Result<(), String> {
        let cfg = dev_config(workdir).ok_or("no dev server set up: Dev server → Set it up")?;
        let old = self.works.dev.lock().expect("not poisoned").remove(agent);
        if let Some(old) = old {
            old.stop().await;
        }
        let env: Vec<(String, String)> = port_of(&cfg.url)
            .map(|p| vec![("PORT".into(), p)])
            .unwrap_or_default();
        let p = start(workdir, "dev server", &cfg.command, &env)?;
        self.works
            .dev
            .lock()
            .expect("not poisoned")
            .insert(agent.into(), p);
        Ok(())
    }

    async fn dev_stop(&self, agent: &str) {
        let p = self.works.dev.lock().expect("not poisoned").remove(agent);
        if let Some(p) = p {
            p.stop().await;
        }
    }

    fn dev_view(&self, agent: &str, workdir: &Path) -> DevServer {
        let cfg = dev_config(workdir);
        let p = self
            .works
            .dev
            .lock()
            .expect("not poisoned")
            .get(agent)
            .cloned();
        let (state, error) = match &p {
            Some(p) if p.running() => ("running", None),
            Some(p) if !p.stopped.load(std::sync::atomic::Ordering::Relaxed) => (
                "error",
                Some(format!(
                    "it ended ({}): {}",
                    p.end.lock().expect("not poisoned").unwrap_or(-1),
                    p.lines(3).join(" ")
                )),
            ),
            _ => ("off", None),
        };
        DevServer {
            state: state.into(),
            url: cfg
                .as_ref()
                .map(|c| c.url.clone())
                .filter(|u| !u.is_empty()),
            hot_reload: cfg.as_ref().is_some_and(|c| c.hot_reload),
            config: cfg,
            error_message: error,
        }
    }

    /// Stop what runs for a thread (its folder is gone, or it leaves this core).
    pub(crate) async fn stop_all(&self, agent: &str) {
        self.dev_stop(agent).await;
        let list = self
            .works
            .procs
            .lock()
            .expect("not poisoned")
            .remove(agent)
            .unwrap_or_default();
        for p in list {
            p.stop().await;
        }
    }

    /// After a turn: a dev server that does not reload by itself starts again on the new files.
    pub(crate) async fn after_turn(&self, agent: &str, workdir: &Path) {
        if !workdir.is_dir() {
            self.stop_all(agent).await;
            return;
        }
        let running = self
            .works
            .dev
            .lock()
            .expect("not poisoned")
            .get(agent)
            .is_some_and(Proc::running);
        if running && dev_config(workdir).is_some_and(|c| !c.hot_reload) {
            let _ = self.dev_start(agent, workdir).await;
        }
    }

    /// Something done on the thread's project from the panel.
    pub(crate) async fn project_action(
        &self,
        runner: &Arc<Runner>,
        agent: &str,
        action: &str,
        p: &Value,
    ) -> Result<(), String> {
        let workdir = runner.host.workdir(agent);
        let text = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let on_disk = || -> Result<(), String> {
            if workdir.is_dir() {
                Ok(())
            } else {
                Err("the thread's folder is not on disk: send it a message first (and keep it on disk in the engine's settings)".into())
            }
        };
        let changed = |r: &Arc<Runner>| r.put_away(agent); // the thread's files follow the folder
        match action {
            "task_add" => {
                on_disk()?;
                self.task_add(&workdir, &text("prompt"))?;
                changed(runner);
                Ok(())
            }
            "task_remove" => {
                on_disk()?;
                let id = text("id");
                self.task_change(&workdir, |tasks| {
                    tasks.retain(|t| t.id != id);
                    Ok(String::new())
                })?;
                changed(runner);
                Ok(())
            }
            "task_move" => {
                on_disk()?;
                let to = text("to");
                let busy = self.active.lock().expect("not poisoned").contains(agent);
                let task = self.task_move(
                    &workdir,
                    &text("id"),
                    &to,
                    Some(if busy { "queued" } else { "running" }),
                )?;
                changed(runner);
                if to == "in_progress" {
                    // the thread takes it as its next message, and the task is done when it is
                    let prompt = format!(
                        "Task {} from the project's task board:\n\n{}\n\nWhen it is finished, move it to done with project_tasks.",
                        task.id, task.prompt
                    );
                    run_task(Arc::clone(runner), agent.to_string(), prompt, task.id);
                }
                Ok(())
            }
            "dev_config" => {
                on_disk()?;
                let command = text("command");
                let url = text("url");
                if command.trim().is_empty() || command.len() > 2000 {
                    return Err("the dev server needs its command".into());
                }
                if !url.is_empty() && !(url.starts_with("http://") || url.starts_with("https://")) {
                    return Err("its address starts with http:// or https://".into());
                }
                write_json(
                    &openfox_dir(&workdir).join("dev.json"),
                    &DevConfig {
                        command: command.trim().into(),
                        url: url.trim().into(),
                        hot_reload: p
                            .get("hot_reload")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    },
                )?;
                changed(runner);
                Ok(())
            }
            "dev_start" | "dev_restart" => {
                on_disk()?;
                self.dev_start(agent, &workdir).await
            }
            "dev_stop" => {
                self.dev_stop(agent).await;
                Ok(())
            }
            "dev_logs" => {
                let lines = self
                    .works
                    .dev
                    .lock()
                    .expect("not poisoned")
                    .get(agent)
                    .map(|p| p.lines(200))
                    .unwrap_or_else(|| {
                        vec!["(the dev server has not run since the core started)".into()]
                    });
                self.works
                    .shown_logs
                    .lock()
                    .expect("not poisoned")
                    .insert(agent.into(), lines);
                Ok(())
            }
            "process_stop" => {
                let id = text("id");
                let p = self
                    .works
                    .procs
                    .lock()
                    .expect("not poisoned")
                    .get(agent)
                    .and_then(|l| l.iter().find(|x| x.id == id).cloned())
                    .ok_or("no such process")?;
                p.stop().await;
                Ok(())
            }
            "checkout" | "branch_new" => {
                on_disk()?;
                let name = text(if action == "checkout" {
                    "branch"
                } else {
                    "name"
                });
                if !branch_like(&name) {
                    return Err(format!("not a branch name: {name:?}"));
                }
                let args: &[&str] = if action == "checkout" {
                    &["checkout", &name]
                } else {
                    &["checkout", "-b", &name]
                };
                git(&workdir, args, &[]).await?;
                changed(runner);
                Ok(())
            }
            "instructions" => {
                on_disk()?;
                let t = text("text");
                if t.len() > 100_000 {
                    return Err("instructions are 100,000 characters at most".into());
                }
                let path = openfox_dir(&workdir).join("instructions.md");
                if t.trim().is_empty() {
                    let _ = std::fs::remove_file(&path);
                } else {
                    std::fs::create_dir_all(openfox_dir(&workdir)).map_err(|e| e.to_string())?;
                    io::atomic_write(&path, t.trim().as_bytes()).map_err(|e| e.to_string())?;
                }
                changed(runner);
                Ok(())
            }
            "git_clone" => {
                on_disk()?;
                self.clone_into(&workdir, &text("forge"), &text("repo"), &text("branch"))
                    .await?;
                self.link(runner, agent, &text("forge"), &text("repo"), None)?;
                changed(runner);
                Ok(())
            }
            "git_publish" => {
                on_disk()?;
                let base = Some(text("base"))
                    .filter(|b| !b.is_empty())
                    .unwrap_or_else(|| "main".into());
                let url = self
                    .publish(
                        &workdir,
                        &text("forge"),
                        &text("repo"),
                        &base,
                        &text("title"),
                        &text("body"),
                    )
                    .await?;
                self.link(
                    runner,
                    agent,
                    &text("forge"),
                    &text("repo"),
                    Some(url.clone()),
                )?;
                let author = format!("{}@{agent}", runner.host.node_id);
                let _ = io::append_event(
                    &runner.host.agents.join(agent),
                    &author,
                    crate::capsule::EventKind::Checkpoint,
                    json!({"reason": "published", "detail": url}),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs_f64())
                        .unwrap_or(0.0),
                );
                Ok(())
            }
            other => Err(format!("unknown action {other:?}")),
        }
    }

    /// Remember which forge and repository a thread's folder belongs to.
    fn link(
        &self,
        runner: &Runner,
        agent: &str,
        forge: &str,
        repo: &str,
        pr: Option<String>,
    ) -> Result<(), String> {
        let capsule = runner.host.agents.join(agent);
        let mut state = ThreadState::load(&capsule);
        let mut g = state.git.take().unwrap_or_default();
        g.forge = forge.into();
        g.repo = repo.into();
        if pr.is_some() {
            g.pull_request = pr;
        }
        state.git = Some(g);
        state.save(&capsule, &format!("{}@{agent}", runner.host.node_id))
    }

    /// What the panel shows of the thread's project, read now.
    pub(crate) async fn fill_project(&self, agent: &str, workdir: &Path, capsule: &Path) {
        let kept = self.settings().keep_folder;
        let on_disk = workdir.is_dir();
        if !on_disk {
            self.stop_all(agent).await;
        }
        let mut view = ProjectView {
            kept,
            on_disk,
            ..Default::default()
        };
        if on_disk {
            view.instructions =
                std::fs::read_to_string(openfox_dir(workdir).join("instructions.md"))
                    .unwrap_or_default();
            view.tasks = read_tasks(workdir);
            view.task_settings = Some(TaskSettings {
                slot_limit: 1,
                queue_paused: false,
            });
            view.dev = Some(self.dev_view(agent, workdir));
            view.dev_logs = self
                .works
                .shown_logs
                .lock()
                .expect("not poisoned")
                .remove(agent)
                .unwrap_or_default();
            view.processes = self
                .works
                .procs
                .lock()
                .expect("not poisoned")
                .get(agent)
                .map(|l| l.iter().map(Proc::view).collect())
                .unwrap_or_default();
            if workdir.join(".git").exists() {
                view.is_git = true;
                if let Ok(list) = git(workdir, &["branch", "--format=%(refname:short)"], &[]).await
                {
                    view.branches = list
                        .lines()
                        .map(String::from)
                        .filter(|b| !b.is_empty())
                        .collect();
                }
                view.branch = git(workdir, &["rev-parse", "--abbrev-ref", "HEAD"], &[])
                    .await
                    .unwrap_or_default();
                if view.branches.is_empty() && !view.branch.is_empty() {
                    view.branches.push(view.branch.clone());
                }
            }
        }
        view.forges = self
            .forges()
            .into_iter()
            .map(|f| json!({"id": f.id, "name": f.name, "kind": f.kind}))
            .collect();
        view.git = ThreadState::load(capsule).git.map(|g| json!(g));
        self.edit_panel(agent, |p| p.project = Some(view));
    }

    /// The agent's project tools: background processes, the dev server, the task board.
    pub(crate) async fn project_tool(
        &self,
        t: &Turn,
        name: &str,
        args: &Value,
    ) -> Result<String, String> {
        let text = |k: &str| {
            args.get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string()
        };
        let action = text("action");
        let out = match (name, action.as_str()) {
            ("background_process", "start") => {
                let command = text("command");
                if command.is_empty() {
                    return Err("start needs a command".into());
                }
                if !t.pass.is_empty() && !t.runner.approve(&t.agent, &command).await {
                    return Err("the person did not allow this command".into());
                }
                let label = Some(text("name"))
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| command.chars().take(40).collect());
                let p = start(&t.workdir, &label, &command, &[])?;
                let id = p.id.clone();
                self.works
                    .procs
                    .lock()
                    .expect("not poisoned")
                    .entry(t.agent.clone())
                    .or_default()
                    .push(p);
                // a moment, so a process that fails at once says so
                tokio::time::sleep(Duration::from_millis(500)).await;
                let p = self.proc_of(&t.agent, &id)?;
                format!(
                    "Started {label} (id {id}): {}\n{}",
                    p.view().status,
                    p.lines(20).join("\n")
                )
            }
            ("background_process", "list") => {
                let list = self
                    .works
                    .procs
                    .lock()
                    .expect("not poisoned")
                    .get(&t.agent)
                    .cloned()
                    .unwrap_or_default();
                if list.is_empty() {
                    "No background process.".into()
                } else {
                    list.iter()
                        .map(|p| {
                            let v = p.view();
                            format!("{} · {} · {} · {}", v.id, v.name, v.status, v.command)
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            }
            ("background_process", "logs") => {
                let p = self.proc_of(&t.agent, &text("id"))?;
                let lines = p.lines(200);
                if lines.is_empty() {
                    "(no output yet)".into()
                } else {
                    lines.join("\n")
                }
            }
            ("background_process", "stop") => {
                let p = self.proc_of(&t.agent, &text("id"))?;
                p.stop().await;
                format!("Stopped {}.", p.name)
            }
            ("dev_server", "status") => {
                serde_json::to_string_pretty(&self.dev_view(&t.agent, &t.workdir))
                    .expect("serializes")
            }
            ("dev_server", "start") | ("dev_server", "restart") => {
                self.dev_start(&t.agent, &t.workdir).await?;
                tokio::time::sleep(Duration::from_millis(500)).await;
                serde_json::to_string_pretty(&self.dev_view(&t.agent, &t.workdir))
                    .expect("serializes")
            }
            ("dev_server", "stop") => {
                self.dev_stop(&t.agent).await;
                "The dev server is stopped.".into()
            }
            ("dev_server", "logs") => self
                .works
                .dev
                .lock()
                .expect("not poisoned")
                .get(&t.agent)
                .map(|p| p.lines(200).join("\n"))
                .unwrap_or_else(|| "The dev server has not run.".into()),
            ("project_tasks", "list") => {
                let only = text("status");
                let tasks: Vec<String> = read_tasks(&t.workdir)
                    .iter()
                    .filter(|x| only.is_empty() || x.status == only)
                    .map(|x| format!("{} [{}] {}", x.id, x.status, x.prompt))
                    .collect();
                if tasks.is_empty() {
                    "No task.".into()
                } else {
                    tasks.join("\n")
                }
            }
            ("project_tasks", "add") => self.task_add(&t.workdir, &text("prompt"))?,
            ("project_tasks", "move") => {
                let task =
                    self.task_move(&t.workdir, &text("taskId"), &text("to"), Some("running"))?;
                format!("Task {} is {} now.", task.id, task.status)
            }
            (_, other) => return Err(format!("{name} has no action {other:?}")),
        };
        self.fill_project(&t.agent, &t.workdir, &t.path()).await;
        self.show(&t.runner, &t.agent, true);
        Ok(out)
    }

    fn proc_of(&self, agent: &str, id: &str) -> Result<Proc, String> {
        self.works
            .procs
            .lock()
            .expect("not poisoned")
            .get(agent)
            .and_then(|l| l.iter().find(|p| p.id == id).cloned())
            .ok_or_else(|| format!("no process {id:?}: use action list"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_port_is_read_from_the_address() {
        assert_eq!(port_of("http://localhost:5173").as_deref(), Some("5173"));
        assert_eq!(
            port_of("http://localhost:8080/app").as_deref(),
            Some("8080")
        );
        assert_eq!(port_of("http://localhost"), None);
    }

    #[tokio::test]
    async fn a_process_runs_logs_and_stops() {
        let dir = tempfile::tempdir().unwrap();
        let p = start(dir.path(), "echo", "echo one; echo two >&2; sleep 30", &[]).unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(p.running());
        let mut lines = p.lines(10);
        lines.sort();
        assert_eq!(lines, ["one", "two"]);
        p.stop().await;
        assert_eq!(p.view().status, "killed");
    }
}
