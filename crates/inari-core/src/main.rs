//! `inari-core-rs`: the Rust core's command line.

use inari_core::capsule::{self, attach, io, EventKind, Timeline};
use inari_core::{agent, bus, buslink, host, node, rpc, teleport};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn usage() -> ExitCode {
    eprintln!(
        "usage: inari-core-rs <command>\n  verify <capsule-dir|archive>\n  inspect <capsule-dir|archive>\n  \
         create <dir> <thread-id> [holder]\n  append <dir> <author> <kind> <text>\n  \
         attach <dir> <path-in-workspace> <file>\n  pack <dir> <archive>\n  unpack <archive> <dir>\n  \
         serve-endpoint <root> <node> <key-file> <port>\n  adopt <root> <node> <capsule-dir>\n  \
         status <root> <node> <agent>\n  teleport <root> <node> <agent> <target-node> <host:port> <key-file>\n  \
         fork <src> <dest> <new-thread> [upto-event]\n  merge <dest> <src>\n  \
         bus-echo <name> <key-file> <expected-peer>\n  bus-say <name> <key-file> <host:port> <subject> <payload-json> <wait-subject>\n  \
         turn <root> <node> <agent> <text> [file-to-attach]... -- <acp-agent command...>\n  \
         node <config.json>\n  \
         demo [--port N] [--host H] [--token T] [--mascot DIR] [--workdir DIR]\n  \
         demo-agent | fake-agent   (an ACP agent on stdin/stdout: the demo's, the tests')\n  \
         join <hub-enroll host:port> <code> [--endpoint-port N] [--gateway-port N] [--service]"
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

/// One user turn on an agent this node holds, through an ACP agent; prints the summary.
fn turn(a: &[&str]) -> capsule::Result<ExitCode> {
    let Some(split) = a.iter().position(|x| *x == "--") else {
        return Ok(usage());
    };
    let (head, command) = (&a[..split], &a[split + 1..]);
    let [root, node, agent, text, files @ ..] = head else {
        return Ok(usage());
    };
    if command.is_empty() {
        return Ok(usage());
    }
    let host = std::sync::Arc::new(host::CapsuleHost::new(node, Path::new(root))?);
    let sessions = Path::new(root).join("acp-sessions.json");
    let harness = agent::AcpHarness::new(command.iter().map(|c| c.to_string()).collect(), sessions);
    let runner = agent::Runner::new(host, vec![("acp".into(), harness)], None, None);
    let summary = tokio::runtime::Runtime::new()?.block_on(async {
        let mut attached = Vec::new();
        for f in files {
            let name = Path::new(f)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            match runner.attach(agent, &name, &std::fs::read(f)?).await {
                Ok(got) => attached.push(got),
                Err(e) => return Err(capsule::Error::Integrity(e)),
            }
        }
        Ok(runner.run(agent, text, attached).await)
    })?;
    println!(
        "{}",
        json!({"stopped": summary.stopped, "text": summary.final_text, "error": summary.error})
    );
    Ok(if summary.stopped == "done" {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

/// Attach this machine to a mesh with an invitation from the Inari page.
fn join(a: &[&str]) -> capsule::Result<ExitCode> {
    let [addr, code, rest @ ..] = a else {
        return Ok(usage());
    };
    let opt = |name: &str, default: u16| -> u16 {
        rest.iter()
            .position(|x| *x == name)
            .and_then(|i| rest.get(i + 1))
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let config_dir = home.join(".config/inari-core");
    let data_dir = home.join(".local/share/inari-core/data");
    let grant = match tokio::runtime::Runtime::new()?.block_on(inari_core::enroll::join(addr, code))
    {
        Ok(g) => g,
        Err(e) => {
            eprintln!("{e}");
            return Ok(ExitCode::from(1));
        }
    };
    let file = inari_core::enroll::write_config(
        &grant,
        &config_dir,
        &data_dir,
        opt("--endpoint-port", 7411),
        opt("--gateway-port", 7487),
    )?;
    println!(
        "joined as {} (hub {}), configuration in {}",
        grant.node_id,
        grant.hub,
        file.display()
    );
    if rest.contains(&"--service") {
        let exe = std::env::current_exe()?;
        let unit = format!(
            "[Unit]\nDescription=Inari core ({})\nAfter=network-online.target\n[Service]\n\
             ExecStart={} node {}\nRestart=always\nRestartSec=5\n[Install]\nWantedBy=default.target\n",
            grant.node_id,
            exe.display(),
            file.display()
        );
        let path = home.join(".config/systemd/user/inari-core.service");
        io::atomic_write(&path, unit.as_bytes())?;
        let ok = std::process::Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status()
            .is_ok_and(|s| s.success())
            && std::process::Command::new("systemctl")
                .args(["--user", "enable", "--now", "inari-core.service"])
                .status()
                .is_ok_and(|s| s.success());
        println!(
            "{}",
            if ok {
                "service started: inari-core.service"
            } else {
                "service written; start it with: systemctl --user enable --now inari-core.service"
            }
        );
    } else {
        println!(
            "start it with: {} node {}",
            std::env::current_exe()?.display(),
            file.display()
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn run(args: &[String]) -> capsule::Result<ExitCode> {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    if a.first() == Some(&"join") {
        return join(&a[1..]);
    }
    if a.first() == Some(&"turn") {
        return turn(&a[1..]);
    }
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
        ["fork", src, dest, thread] | ["fork", src, dest, thread, _] => {
            capsule::fork::fork(Path::new(src), Path::new(dest), thread, a.get(4).copied())?;
            Ok(ExitCode::SUCCESS)
        }
        ["merge", dest, src] => {
            println!("{}", capsule::fork::merge(Path::new(dest), Path::new(src))?);
            Ok(ExitCode::SUCCESS)
        }
        ["bus-echo", name, key_file, peer] => {
            let key = std::fs::read(key_file)?.trim_ascii().to_vec();
            tokio::runtime::Runtime::new()?.block_on(async {
                let b = bus::Bus::new(name);
                let mut all = b.subscribe(">");
                let server = buslink::BusServer::new(std::sync::Arc::clone(&b), &key, &[peer]);
                println!("listening {}", server.start("127.0.0.1", 0).await?);
                use std::io::Write;
                std::io::stdout().flush()?;
                while let Some(m) = all.recv().await {
                    if !m.subject.starts_with("echo.") && m.sender != *name {
                        b.publish(&format!("echo.{}", m.subject), m.payload);
                    }
                }
                Ok::<_, capsule::Error>(())
            })?;
            Ok(ExitCode::SUCCESS)
        }
        ["bus-say", name, key_file, addr, subject, payload, wait] => {
            let key = std::fs::read(key_file)?.trim_ascii().to_vec();
            let payload: serde_json::Value = serde_json::from_str(payload)?;
            let got = tokio::runtime::Runtime::new()?.block_on(async {
                let b = bus::Bus::new(name);
                let mut rx = b.subscribe(wait);
                let _client = buslink::BusClient::start(std::sync::Arc::clone(&b), &key, addr);
                b.publish(subject, payload);
                tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
                    .await
                    .ok()
                    .flatten()
            });
            match got {
                Some(m) => {
                    println!("{}", m.to_json());
                    Ok(ExitCode::SUCCESS)
                }
                None => Ok(ExitCode::from(1)),
            }
        }
        ["node", config] => {
            let cfg = match node::NodeConfig::load(Path::new(config)) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("{e}");
                    return Ok(ExitCode::from(2));
                }
            };
            tokio::runtime::Runtime::new()?.block_on(async {
                match node::Node::start(cfg).await {
                    Ok(n) => {
                        let mut hub = if n.is_hub {
                            format!(", bus {}", n.bus_port)
                        } else {
                            String::new()
                        };
                        if n.gateway.is_some() {
                            hub.push_str(&format!(", gateway {}", n.gateway_port));
                        }
                        println!(
                            "node {} up: endpoint {}{hub}",
                            n.cfg.node_id, n.endpoint_port
                        );
                        use std::io::Write;
                        let _ = std::io::stdout().flush();
                        std::future::pending::<()>().await;
                        Ok(ExitCode::SUCCESS)
                    }
                    Err(e) => {
                        eprintln!("cannot start: {e}");
                        Ok(ExitCode::from(1))
                    }
                }
            })
        }
        ["demo", rest @ ..] => demo(rest),
        ["pack", dir, archive] => {
            io::pack(Path::new(dir), Path::new(archive)).map(|_| ExitCode::SUCCESS)
        }
        ["unpack", archive, dir] => {
            io::unpack(Path::new(archive), Path::new(dir)).map(|_| ExitCode::SUCCESS)
        }
        _ => Ok(usage()),
    }
}

/// `inari-core-rs demo`: three nodes in this process and the interface, until Ctrl+C.
fn demo(a: &[&str]) -> capsule::Result<ExitCode> {
    let mut o = inari_core::demo::DemoOptions {
        host: "127.0.0.1".into(),
        port: 8787,
        token: "demo".into(),
        workdir: None,
        mascot_dir: None,
        agent: vec![
            std::env::current_exe()?.display().to_string(),
            "demo-agent".into(),
        ],
    };
    let mut it = a.iter();
    while let Some(flag) = it.next() {
        let Some(value) = it.next() else {
            return Ok(usage());
        };
        match *flag {
            "--port" => match value.parse() {
                Ok(p) => o.port = p,
                Err(_) => return Ok(usage()),
            },
            "--host" => o.host = value.to_string(),
            "--token" => o.token = value.to_string(),
            "--mascot" => o.mascot_dir = Some(PathBuf::from(value)),
            "--workdir" => o.workdir = Some(PathBuf::from(value)),
            _ => return Ok(usage()),
        }
    }
    let (host, token) = (o.host.clone(), o.token.clone());
    tokio::runtime::Runtime::new()?.block_on(async move {
        match inari_core::demo::start(o).await {
            Ok(d) => {
                let port = d.nodes[0].gateway_port;
                println!("inari-core demo: http://{host}:{port}/#token={token}  (Ctrl+C to stop; capsules in {})", d.workdir.display());
                use std::io::Write;
                let _ = std::io::stdout().flush();
                std::future::pending::<()>().await;
                Ok(ExitCode::SUCCESS)
            }
            Err(e) => {
                eprintln!("cannot start the demo: {e}");
                Ok(ExitCode::from(1))
            }
        }
    })
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // the ACP agents speak on stdin/stdout: no runtime, nothing else printed
    match args.first().map(String::as_str) {
        Some("demo-agent") => {
            return ExitCode::from(
                inari_core::demo_agent::serve(inari_core::demo_agent::Mode::Demo) as u8,
            )
        }
        Some("fake-agent") => {
            return ExitCode::from(
                inari_core::demo_agent::serve(inari_core::demo_agent::Mode::Fake) as u8,
            )
        }
        _ => {}
    }
    match run(&args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}
