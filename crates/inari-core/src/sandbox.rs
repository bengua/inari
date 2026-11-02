//! Per-session sandbox: an agent session runs inside a bubblewrap bubble — the
//! thread's workdir writable, everything else read-only — and, when systemd
//! offers user scopes, inside a workload cgroup of its own.
//!
//! The bubble is the security boundary, the cgroup is accounting: a node
//! without bwrap refuses to start the thread and says so, while a node without
//! systemd user scopes just runs the session uncaged (and logs it).

use std::path::{Path, PathBuf};

/// Why a session could not be sandboxed. Starting the thread anyway would
/// defeat the bubble, so this is fatal for the thread — it surfaces to the
/// owner as the reason the harness did not start.
#[derive(Debug)]
pub struct Unavailable(pub String);

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Unavailable {}

fn on_path(exe: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p)
            .map(|d| d.join(exe))
            .find(|c| c.is_file())
    })
}

/// bubblewrap is installed on this node.
pub fn available() -> bool {
    on_path("bwrap").is_some()
}

fn scope_prefix() -> Option<Vec<String>> {
    let systemd_run = on_path("systemd-run")?;
    // a user bus to make the scope on: none in containers and most CI runners
    let bus = std::env::var_os("DBUS_SESSION_BUS_ADDRESS");
    let runtime_bus = std::env::var_os("XDG_RUNTIME_DIR")
        .map(|d| Path::new(&d).join("bus"))
        .filter(|p| p.exists());
    if bus.is_none() && runtime_bus.is_none() {
        return None;
    }
    let unit = format!(
        "inari-session-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    );
    Some(vec![
        systemd_run.to_string_lossy().into_owned(),
        "--user".into(),
        "--scope".into(),
        "--quiet".into(),
        "--collect".into(),
        format!("--unit={unit}"),
        format!("--description=Inari agent session ({unit})"),
        "--".into(),
    ])
}

/// The argv that runs `command` inside the bubble: the whole filesystem
/// read-only except `workdir` and `extra_rw` (a pass's own HOME, say), the
/// network kept (the harness speaks to its backends over HTTP), the mount,
/// IPC and UTS namespaces private — the PID namespace stays shared so the
/// node's process-group kill and /proc checks still reach whatever the agent
/// started — the bubble dying with the node.
pub fn wrap(
    command: &[String],
    workdir: &Path,
    extra_rw: &[PathBuf],
) -> Result<Vec<String>, Unavailable> {
    wrap_with(command, workdir, extra_rw, available())
}

/// `wrap` with the bwrap presence decided by the caller (the tests). When
/// `bwrap` is true the binary is resolved on PATH to its absolute location:
/// the harness spawns with an empty environment, a bare name would not resolve.
pub fn wrap_with(
    command: &[String],
    workdir: &Path,
    extra_rw: &[PathBuf],
    bwrap: bool,
) -> Result<Vec<String>, Unavailable> {
    if command.is_empty() {
        return Err(Unavailable("sandbox: empty command".into()));
    }
    if !bwrap {
        return Err(Unavailable(
            "sandbox: bwrap not found — this node refuses to start the thread until bubblewrap is installed".into(),
        ));
    }
    let bwrap = on_path("bwrap").ok_or_else(|| {
        Unavailable(
            "sandbox: bwrap not found — this node refuses to start the thread until bubblewrap is installed".into(),
        )
    })?;
    let wd = workdir.to_string_lossy().into_owned();
    let mut argv: Vec<String> = scope_prefix().unwrap_or_default();
    argv.push(bwrap.to_string_lossy().into_owned());
    // No `--new-session`, no PID namespace: the node ends a turn by
    // signalling the spawned process's whole group and checks its children
    // through /proc — a private session or PID namespace would put the
    // agent (and whatever it started) out of that reach, outliving the turn.
    for arg in [
        "--die-with-parent",
        "--unshare-user",
        "--unshare-ipc",
        "--unshare-uts",
        "--unshare-cgroup-try",
        "--share-net",
        "--ro-bind",
        "/",
        "/",
        "--bind",
    ] {
        argv.push(arg.into());
    }
    argv.push(wd.clone());
    argv.push(wd.clone());
    for p in extra_rw {
        let p = p.to_string_lossy().into_owned();
        argv.push("--bind".into());
        argv.push(p.clone());
        argv.push(p);
    }
    argv.push("--chdir".into());
    argv.push(wd);
    argv.push("--".into());
    argv.extend(command.iter().cloned());
    Ok(argv)
}
