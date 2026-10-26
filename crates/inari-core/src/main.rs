//! `inari-core-rs`: the Rust core's command line. For now: capsules.

use inari_core::capsule::{self, attach, io, EventKind, Timeline};
use inari_core::{host, rpc, teleport};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn usage() -> ExitCode {
    eprintln!(
        "usage: inari-core-rs <command>\n  verify <capsule-dir|archive>\n  inspect <capsule-dir|archive>\n  \
         create <dir> <thread-id> [holder]\n  append <dir> <author> <kind> <text>\n  \
         attach <dir> <path-in-workspace> <file>\n  pack <dir> <archive>\n  unpack <archive> <dir>\n  \
         serve-endpoint <root> <node> <key-file> <port>\n  adopt <root> <node> <capsule-dir>\n  \
         status <root> <node> <agent>\n  teleport <root> <node> <agent> <target-node> <host:port> <key-file>"
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
        ["serve-endpoint", root, node, key_file, port] => {
            let key = std::fs::read(key_file)?.trim_ascii().to_vec();
            let port: u16 = port.parse().unwrap_or(0);
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(async {
                let host = std::sync::Arc::new(host::CapsuleHost::new(node, Path::new(root))?);
                host.recover();
                let server = rpc::EndpointServer::new(host, &key);
                let bound = server.start("127.0.0.1", port).await?;
                println!("listening {bound}");
                use std::io::Write;
                std::io::stdout().flush()?;
                std::future::pending::<()>().await;
                Ok::<_, capsule::Error>(())
            })?;
            Ok(ExitCode::SUCCESS)
        }
        ["adopt", root, node, capsule_dir] => {
            let host = host::CapsuleHost::new(node, Path::new(root))?;
            match host.adopt(Path::new(capsule_dir)) {
                Ok(agent) => {
                    println!("{agent}");
                    Ok(ExitCode::SUCCESS)
                }
                Err(e) => {
                    eprintln!("{e}");
                    Ok(ExitCode::from(1))
                }
            }
        }
        ["status", root, node, agent] => {
            let host = host::CapsuleHost::new(node, Path::new(root))?;
            println!("{}", host.status(agent));
            Ok(ExitCode::SUCCESS)
        }
        ["teleport", root, node, agent, target_node, addr, key_file] => {
            let key = std::fs::read(key_file)?.trim_ascii().to_vec();
            let rt = tokio::runtime::Runtime::new()?;
            let outcome = rt.block_on(async {
                let source = host::CapsuleHost::new(node, Path::new(root))?;
                let target = rpc::RemoteEndpoint::new(
                    target_node,
                    addr,
                    &key,
                    std::time::Duration::from_secs(30),
                );
                Ok::<_, capsule::Error>(teleport::teleport(&source, &target, agent, None).await)
            })?;
            println!("{}", serde_json::to_string(&outcome).expect("serializes"));
            Ok(if outcome.state == "committed" {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
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
