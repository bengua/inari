//! `inari-core-rs`: the Rust core's command line. For now: capsules.

use inari_core::capsule::{self, attach, io, EventKind, Timeline};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn usage() -> ExitCode {
    eprintln!(
        "usage: inari-core-rs <command>\n  verify <capsule-dir|archive>\n  inspect <capsule-dir|archive>\n  \
         create <dir> <thread-id> [holder]\n  append <dir> <author> <kind> <text>\n  \
         attach <dir> <path-in-workspace> <file>\n  pack <dir> <archive>\n  unpack <archive> <dir>"
    );
    ExitCode::from(2)
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// A directory as is; an archive unpacked into a temporary directory.
fn open(path: &Path) -> capsule::Result<(PathBuf, Option<PathBuf>)> {
    if path.is_dir() {
        return Ok((path.to_path_buf(), None));
    }
    let tmp = std::env::temp_dir().join(format!("inari-rs-{}-{}", std::process::id(), now()));
    io::unpack(path, &tmp)?;
    Ok((tmp.clone(), Some(tmp)))
}

fn kind(name: &str) -> Option<EventKind> {
    serde_json::from_value(json!(name)).ok()
}

fn run(args: &[String]) -> capsule::Result<ExitCode> {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    match a.as_slice() {
        ["verify", path] => {
            let (root, tmp) = open(Path::new(path))?;
            let problems = io::verify(&root);
            if let Some(t) = tmp {
                let _ = std::fs::remove_dir_all(t);
            }
            for p in &problems {
                eprintln!("{p}");
            }
            println!(
                "{}",
                if problems.is_empty() {
                    "ok".into()
                } else {
                    format!("{} problem(s)", problems.len())
                }
            );
            Ok(if problems.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
        ["inspect", path] => {
            let (root, tmp) = open(Path::new(path))?;
            let c = io::load(&root)?;
            let holder = c
                .manifest
                .extra
                .get("holder")
                .cloned()
                .unwrap_or(json!(null));
            println!(
                "{}",
                json!({"capsule": c.manifest.capsule_id, "thread": c.manifest.thread_id, "holder": holder,
                       "events": c.timeline.len(), "files": c.manifest.files.len(),
                       "workspace": c.snapshot.tree.len()})
            );
            if let Some(t) = tmp {
                let _ = std::fs::remove_dir_all(t);
            }
            Ok(ExitCode::SUCCESS)
        }
        ["create", dir, thread] | ["create", dir, thread, _] => {
            let holder = a.get(4).copied();
            let mut t = Timeline::new();
            t.append(
                &format!("{}@{thread}", holder.unwrap_or("rust")),
                EventKind::System,
                json!({"text": "created by the Rust core"}),
                now(),
            );
            io::create(
                Path::new(dir),
                thread,
                &format!("rs-{thread}-{}", now() as u64),
                holder,
                &t,
                now(),
            )?;
            Ok(ExitCode::SUCCESS)
        }
        ["append", dir, author, k, text] => {
            let Some(k) = kind(k) else { return Ok(usage()) };
            io::append_event(Path::new(dir), author, k, json!({"text": text}), now())?;
            Ok(ExitCode::SUCCESS)
        }
        ["attach", dir, rel, file] => {
            let data = std::fs::read(file)?;
            let got = attach::attach(Path::new(dir), rel, &data)?;
            println!(
                "{}",
                json!({"path": got.path, "sha256": got.sha256, "size": got.size, "mime": got.mime})
            );
            Ok(ExitCode::SUCCESS)
        }
        ["pack", dir, archive] => {
            io::pack(Path::new(dir), Path::new(archive)).map(|_| ExitCode::SUCCESS)
        }
        ["unpack", archive, dir] => {
            io::unpack(Path::new(archive), Path::new(dir)).map(|_| ExitCode::SUCCESS)
        }
        _ => Ok(usage()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}
