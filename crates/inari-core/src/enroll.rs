//! Attaching a new core to a mesh. A page asks for an invitation (`mesh.invite` on the
//! bus); the hub keeps it for a while; the new machine runs `inari-core-rs join` with the
//! code, proves it holds it (the transport's handshake keyed by the code), and receives
//! the mesh's keys and where its bus is. A code works once, before it expires.
//!
//! The transport authenticates but does not encrypt: the keys travel over the private
//! network the cores already share (Tailscale, WireGuard, an SSH tunnel), as the bus does.

use crate::bus::Bus;
use crate::canon::sha256_hex;
use crate::wire::Wire;
use base64::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// How long an invitation waits for its machine.
pub const INVITE_TTL: Duration = Duration::from_secs(30 * 60);

/// What the joining side sends first, in clear: which invitation it holds (not the code).
fn invite_id(code: &str) -> String {
    sha256_hex(code.as_bytes())[..32].to_string()
}

/// A fresh invitation code: 24 characters that are easy to copy.
pub fn new_code() -> String {
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let bytes = crate::wire::random_bytes::<24>();
    bytes
        .iter()
        .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
        .collect()
}

struct Invite {
    code: String,
    node_id: String,
    until: Instant,
}

/// The hub's side: invitations kept in memory, and the counter the new cores come to.
pub struct Enroll {
    invites: Mutex<HashMap<String, Invite>>,
    mesh_key: Vec<u8>,
    device_key: Vec<u8>,
    bus_port: u16,
}

impl Enroll {
    pub fn start(bus: &Arc<Bus>, mesh_key: &[u8], device_key: &[u8], bus_port: u16) -> Arc<Self> {
        let me = Arc::new(Self {
            invites: Mutex::default(),
            mesh_key: mesh_key.to_vec(),
            device_key: device_key.to_vec(),
            bus_port,
        });
        let mut asks = bus.subscribe("mesh.invite");
        let keep = Arc::clone(&me);
        tokio::spawn(async move {
            while let Some(m) = asks.recv().await {
                let code = m.payload.get("code").and_then(Value::as_str).unwrap_or("");
                let node = m
                    .payload
                    .get("node_id")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if code.len() >= 16 && !node.is_empty() {
                    keep.add(code, node);
                }
            }
        });
        me
    }

    pub fn add(&self, code: &str, node_id: &str) {
        let mut all = self.invites.lock().expect("not poisoned");
        all.retain(|_, i| i.until > Instant::now());
        all.insert(
            invite_id(code),
            Invite {
                code: code.into(),
                node_id: node_id.into(),
                until: Instant::now() + INVITE_TTL,
            },
        );
    }

    pub async fn listen(self: &Arc<Self>, bind: &str, port: u16) -> std::io::Result<u16> {
        let listener = TcpListener::bind((bind, port)).await?;
        let bound = listener.local_addr()?.port();
        let me = Arc::clone(self);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let me = Arc::clone(&me);
                tokio::spawn(async move {
                    let _ = tokio::time::timeout(Duration::from_secs(15), me.serve(stream)).await;
                });
            }
        });
        Ok(bound)
    }

    async fn serve(&self, mut stream: TcpStream) -> Option<()> {
        let mut id = [0u8; 32];
        stream.read_exact(&mut id).await.ok()?;
        let id = String::from_utf8(id.to_vec()).ok()?;
        let (code, node_id) = {
            let all = self.invites.lock().expect("not poisoned");
            let i = all.get(&id).filter(|i| i.until > Instant::now())?;
            (i.code.clone(), i.node_id.clone())
        };
        let mut wire = Wire::new(stream, code.as_bytes());
        wire.handshake(false, json!({"enroll": true}), Duration::from_secs(5))
            .await
            .ok()?;
        // proven: the code is spent whatever happens next
        self.invites.lock().expect("not poisoned").remove(&id);
        let b64 = base64::engine::general_purpose::STANDARD;
        let grant = json!({"node_id": node_id, "bus_port": self.bus_port,
                           "mesh_key": b64.encode(&self.mesh_key), "device_key": b64.encode(&self.device_key)});
        wire.send(&grant).await.ok()?;
        Some(())
    }
}

/// What a new core receives.
#[derive(Debug, Clone)]
pub struct Grant {
    pub node_id: String,
    pub hub: String,
    pub mesh_key: Vec<u8>,
    pub device_key: Vec<u8>,
    /// The address this machine reached the hub from: where the hub can reach it back.
    pub local_ip: String,
}

/// The new machine's side: present the code at `addr` (the hub's enroll counter).
pub async fn join(addr: &str, code: &str) -> Result<Grant, String> {
    let mut stream = tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(addr))
        .await
        .map_err(|_| format!("{addr}: no answer"))?
        .map_err(|e| format!("{addr}: {e}"))?;
    let local_ip = stream
        .local_addr()
        .map(|a| a.ip().to_string())
        .unwrap_or_default();
    stream
        .write_all(invite_id(code.trim()).as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    let mut wire = Wire::new(stream, code.trim().as_bytes());
    wire.handshake(true, json!({"enroll": true}), Duration::from_secs(5))
        .await
        .map_err(|_| "the invitation was refused: wrong, used or expired code".to_string())?;
    let grant = tokio::time::timeout(Duration::from_secs(10), wire.recv())
        .await
        .map_err(|_| "the hub did not answer".to_string())?
        .map_err(|e| e.to_string())?;
    let b64 = base64::engine::general_purpose::STANDARD;
    let field = |k: &str| {
        grant
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let host = addr.rsplit_once(':').map_or(addr, |(h, _)| h);
    Ok(Grant {
        node_id: field("node_id"),
        hub: format!(
            "{host}:{}",
            grant.get("bus_port").and_then(Value::as_u64).unwrap_or(0)
        ),
        mesh_key: b64.decode(field("mesh_key")).map_err(|e| e.to_string())?,
        device_key: b64.decode(field("device_key")).map_err(|e| e.to_string())?,
        local_ip,
    })
}

fn detect_harnesses_in(dirs: &[PathBuf]) -> Vec<(String, Vec<String>)> {
    let find = |bin: &str| {
        dirs.iter()
            .map(|d| d.join(bin))
            .find(|p| p.is_file())
            .map(|p| p.display().to_string())
    };
    let mut out = Vec::new();
    if let Some(p) = find("claude-agent-acp") {
        out.push(("claude".to_string(), vec![p]));
    }
    if let Some(p) = find("gemini") {
        out.push(("gemini".to_string(), vec![p, "--acp".into()]));
    }
    if let Some(p) = find("openfox-acp") {
        out.push(("openfox".to_string(), vec![p]));
    }
    if let Some(p) = find("kimi") {
        out.push(("kimi".to_string(), vec![p, "acp".into()]));
    }
    out
}

/// The harnesses found on this machine, by name: what a new core can drive.
pub fn detect_harnesses() -> Vec<(String, Vec<String>)> {
    let path = std::env::var("PATH").unwrap_or_default();
    let home = std::env::var("HOME").unwrap_or_default();
    let dirs: Vec<PathBuf> = std::iter::once(Path::new(&home).join(".local/bin"))
        .chain(std::env::split_paths(&path))
        .collect();
    detect_harnesses_in(&dirs)
}

fn harness_spec(name: &str, command: Vec<String>) -> Value {
    if name == "kimi" {
        json!({"command": command, "timeout": 900, "update": {"npm": "@moonshot-ai/kimi-code"}})
    } else {
        json!({"command": command, "timeout": 900})
    }
}

fn write_secret(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    std::io::Write::write_all(&mut f, data)
}

/// Write the keys and the node's configuration in `config_dir`; data goes to `data_dir`.
pub fn write_config(
    grant: &Grant,
    config_dir: &Path,
    data_dir: &Path,
    endpoint_port: u16,
    gateway_port: u16,
) -> std::io::Result<PathBuf> {
    let detected = detect_harnesses();
    let home = std::env::var("HOME").unwrap_or_default();
    let usage = if Path::new(&home).join(".claude/.credentials.json").is_file() {
        json!([{"kind": "claude"}])
    } else {
        json!([])
    };
    write_config_with_detection(
        grant,
        config_dir,
        data_dir,
        endpoint_port,
        gateway_port,
        detected,
        usage,
    )
}

fn write_config_with_detection(
    grant: &Grant,
    config_dir: &Path,
    data_dir: &Path,
    endpoint_port: u16,
    gateway_port: u16,
    detected: Vec<(String, Vec<String>)>,
    usage: Value,
) -> std::io::Result<PathBuf> {
    write_secret(&config_dir.join("mesh.key"), &grant.mesh_key)?;
    write_secret(&config_dir.join("device.key"), &grant.device_key)?;
    let token = crate::wire::random_hex(24);
    write_secret(&config_dir.join("token"), token.as_bytes())?;
    let harnesses: serde_json::Map<String, Value> = detected
        .into_iter()
        .map(|(n, c)| {
            let spec = harness_spec(&n, c);
            (n, spec)
        })
        .collect();
    let cfg = json!({
        "node_id": grant.node_id, "role": "heavy", "hub": grant.hub,
        "data_dir": data_dir, "key_file": config_dir.join("mesh.key"), "device_key_file": config_dir.join("device.key"),
        "bind_host": grant.local_ip, "advertise_host": grant.local_ip, "endpoint_port": endpoint_port,
        "gateway_listen": format!("127.0.0.1:{gateway_port}"), "gateway_token_file": config_dir.join("token"),
        "gateway_trust_loopback": true, "usage": usage, "harnesses": harnesses, "heartbeat": 2.0,
    });
    let file = config_dir.join("node.json");
    crate::capsule::io::atomic_write(
        &file,
        serde_json::to_string_pretty(&cfg)
            .expect("serializes")
            .as_bytes(),
    )
    .map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_code_works_once_and_a_wrong_one_never() {
        let bus = Bus::new("hub");
        let enroll = Enroll::start(&bus, b"mesh", b"device", 7400);
        let port = enroll.listen("127.0.0.1", 0).await.unwrap();
        let code = new_code();
        bus.publish("mesh.invite", json!({"code": code, "node_id": "vps"}));
        tokio::time::sleep(Duration::from_millis(50)).await;
        let addr = format!("127.0.0.1:{port}");
        assert!(join(&addr, &new_code()).await.is_err(), "an unknown code");
        let grant = join(&addr, &code).await.unwrap();
        assert_eq!(
            (grant.node_id.as_str(), grant.hub.as_str()),
            ("vps", "127.0.0.1:7400")
        );
        assert_eq!(
            (grant.mesh_key.as_slice(), grant.device_key.as_slice()),
            (&b"mesh"[..], &b"device"[..])
        );
        assert_eq!(grant.local_ip, "127.0.0.1");
        assert!(join(&addr, &code).await.is_err(), "a code works once");
    }

    #[test]
    fn codes_are_long_and_readable() {
        let c = new_code();
        assert_eq!(c.len(), 24);
        assert!(c
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit()));
        assert_ne!(c, new_code());
    }

    #[test]
    fn kimi_detection_uses_absolute_path_and_acp() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let kimi = bin.join("kimi");
        std::fs::write(&kimi, b"").unwrap();

        let found = detect_harnesses_in(&[bin]);
        assert_eq!(found.len(), 1);
        let (_, command) = found.iter().find(|(name, _)| name == "kimi").unwrap();
        assert_eq!(command, &vec![kimi.display().to_string(), "acp".into()]);
    }

    #[test]
    fn kimi_spec_has_nested_update_npm() {
        let kimi = harness_spec("kimi", vec!["/tmp/kimi".into(), "acp".into()]);
        assert_eq!(kimi["update"]["npm"], "@moonshot-ai/kimi-code");
        assert!(kimi.get("npm").is_none());

        let other = harness_spec("gemini", vec!["gemini".into(), "--acp".into()]);
        assert!(other.get("update").is_none());
    }

    #[test]
    fn write_config_persists_kimi_update_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let config_dir = dir.path().join("config");
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();
        let grant = Grant {
            node_id: "test".into(),
            hub: "127.0.0.1:7400".into(),
            mesh_key: b"mesh".to_vec(),
            device_key: b"device".to_vec(),
            local_ip: "127.0.0.1".into(),
        };
        let detected = vec![(
            "kimi".to_string(),
            vec!["/detected/kimi".into(), "acp".into()],
        )];

        let path = write_config_with_detection(
            &grant,
            &config_dir,
            &data_dir,
            7401,
            7487,
            detected,
            json!([]),
        )
        .unwrap();
        let cfg: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(
            cfg["harnesses"]["kimi"]["command"],
            json!(["/detected/kimi", "acp"])
        );
        assert_eq!(
            cfg["harnesses"]["kimi"]["update"]["npm"],
            "@moonshot-ai/kimi-code"
        );
        assert!(cfg["harnesses"]["kimi"].get("npm").is_none());
    }
}
