//! An `Endpoint` served and called over TCP, one connection per call, with the
//! reference's method names, arguments and replies — so Rust and Python nodes can
//! teleport agents to each other.

use crate::host::CapsuleHost;
use crate::teleport::{CallError, CallResult, Endpoint};
use crate::wire::{TransportError, Wire};
use base64::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

pub const CHUNK: usize = 512 * 1024;
const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

pub struct EndpointServer {
    host: Arc<CapsuleHost>,
    key: Vec<u8>,
    partial: Mutex<HashMap<(String, String), Vec<u8>>>,
}

fn arg_str<'a>(a: &'a Value, k: &str) -> CallResult<&'a str> {
    a.get(k)
        .and_then(Value::as_str)
        .ok_or_else(|| CallError::new("KeyError", k))
}

fn arg_u64(a: &Value, k: &str) -> CallResult<u64> {
    a.get(k)
        .and_then(Value::as_u64)
        .ok_or_else(|| CallError::new("KeyError", k))
}

impl EndpointServer {
    pub fn new(host: Arc<CapsuleHost>, key: &[u8]) -> Arc<Self> {
        Arc::new(Self {
            host,
            key: key.to_vec(),
            partial: Mutex::default(),
        })
    }

    /// Bind and serve forever in the background; returns the bound port.
    pub async fn start(self: &Arc<Self>, bind: &str, port: u16) -> std::io::Result<u16> {
        let listener = TcpListener::bind((bind, port)).await?;
        let port = listener.local_addr()?.port();
        let me = Arc::clone(self);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let me = Arc::clone(&me);
                tokio::spawn(async move { me.serve(Wire::new(stream, &me.key)).await });
            }
        });
        Ok(port)
    }

    async fn serve(&self, mut wire: Wire) {
        if wire
            .handshake(
                false,
                json!({"node": self.host.node_id}),
                Duration::from_secs(5),
            )
            .await
            .is_err()
        {
            return; // a bad peer is dropped; nothing to clean up
        }
        let Ok(call) = wire.recv().await else { return };
        let method = call
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let args = call.get("args").cloned().unwrap_or(json!({}));
        let reply = match self.dispatch(&method, &args).await {
            Ok(result) => json!({"ok": true, "result": result}),
            Err(e) => json!({"ok": false, "kind": e.kind, "message": e.message}),
        };
        let _ = wire.send(&reply).await;
        wire.close().await;
    }

    async fn dispatch(&self, method: &str, a: &Value) -> CallResult<Value> {
        let h = &*self.host;
        Ok(match method {
            "freeze" => {
                let (manifest, epoch) = h.freeze(arg_str(a, "agent")?).await?;
                json!({"manifest": manifest, "epoch": epoch})
            }
            "read_chunk" => {
                let data = h.read(arg_str(a, "agent")?, arg_str(a, "rel")?).await?;
                let offset = (arg_u64(a, "offset")? as usize).min(data.len());
                let end = (offset + CHUNK).min(data.len());
                json!({"data": B64.encode(&data[offset..end]), "total": data.len()})
            }
            "unfreeze" => h
                .unfreeze(arg_str(a, "agent")?, arg_u64(a, "epoch")?)
                .await
                .map(|_| Value::Null)?,
            "tombstone" => h
                .tombstone(arg_str(a, "agent")?, arg_u64(a, "epoch")?)
                .await
                .map(|_| Value::Null)?,
            "offer" => {
                let manifest = a
                    .get("manifest")
                    .ok_or_else(|| CallError::new("KeyError", "manifest"))?;
                json!(
                    h.offer(arg_str(a, "sid")?, manifest, arg_u64(a, "epoch")?)
                        .await?
                )
            }
            "receive_chunk" => {
                let key = (
                    arg_str(a, "sid")?.to_string(),
                    arg_str(a, "rel")?.to_string(),
                );
                let piece = B64
                    .decode(arg_str(a, "data")?)
                    .map_err(|e| CallError::new("ValueError", e.to_string()))?;
                let mut partial = self.partial.lock().await;
                let buf = partial.entry(key.clone()).or_default();
                if arg_u64(a, "offset")? as usize != buf.len() {
                    return Err(CallError::new("InariError", "out-of-order chunk"));
                }
                buf.extend_from_slice(&piece);
                if a.get("last").and_then(Value::as_bool).unwrap_or(false) {
                    let data = partial.remove(&key).unwrap_or_default();
                    drop(partial);
                    h.receive(&key.0, &key.1, &data).await?;
                }
                Value::Null
            }
            "verify" => h.verify(arg_str(a, "sid")?).await.map(|_| Value::Null)?,
            "prepare" => h.prepare(arg_str(a, "sid")?).await.map(|_| Value::Null)?,
            "abort" => h.abort(arg_str(a, "sid")?).await.map(|_| Value::Null)?,
            "activate" => h
                .activate(arg_str(a, "sid")?, arg_u64(a, "epoch")?)
                .await
                .map(|_| Value::Null)?,
            "holder" => match h.holder(arg_str(a, "agent")?).await? {
                Some((node, epoch)) => json!([node, epoch]),
                None => Value::Null,
            },
            other => {
                return Err(CallError::new(
                    "InariError",
                    format!("unknown method {other:?}"),
                ))
            }
        })
    }
}

/// An `Endpoint` whose work happens on another node.
pub struct RemoteEndpoint {
    pub node_id: String,
    pub addr: String,
    key: Vec<u8>,
    timeout: Duration,
}

impl RemoteEndpoint {
    pub fn new(node_id: &str, addr: &str, key: &[u8], timeout: Duration) -> Self {
        Self {
            node_id: node_id.into(),
            addr: addr.into(),
            key: key.to_vec(),
            timeout,
        }
    }

    async fn call(&self, method: &str, args: Value) -> CallResult<Value> {
        let tr = |e: TransportError| CallError::transport(format!("{method}: {e}"));
        let mut wire = Wire::connect(&self.addr, &self.key, self.timeout)
            .await
            .map_err(tr)?;
        wire.handshake(true, json!({"node": "caller"}), self.timeout)
            .await
            .map_err(tr)?;
        wire.send(&json!({"method": method, "args": args}))
            .await
            .map_err(tr)?;
        let reply = tokio::time::timeout(self.timeout, wire.recv())
            .await
            .map_err(|_| CallError::transport(format!("{method}: no reply")))?
            .map_err(tr)?;
        wire.close().await;
        if reply.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(reply.get("result").cloned().unwrap_or(Value::Null))
        } else {
            Err(CallError::new(
                reply
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("RemoteError"),
                reply.get("message").and_then(Value::as_str).unwrap_or(""),
            ))
        }
    }
}

impl Endpoint for RemoteEndpoint {
    fn node_id(&self) -> &str {
        &self.node_id
    }
    async fn freeze(&self, agent: &str) -> CallResult<(Value, u64)> {
        let r = self.call("freeze", json!({"agent": agent})).await?;
        Ok((
            r.get("manifest").cloned().unwrap_or(Value::Null),
            r.get("epoch").and_then(Value::as_u64).unwrap_or(0),
        ))
    }
    async fn read(&self, agent: &str, rel: &str) -> CallResult<Vec<u8>> {
        let mut out = Vec::new();
        loop {
            let r = self
                .call(
                    "read_chunk",
                    json!({"agent": agent, "rel": rel, "offset": out.len()}),
                )
                .await?;
            let piece = B64
                .decode(r.get("data").and_then(Value::as_str).unwrap_or(""))
                .map_err(|e| CallError::new("ValueError", e.to_string()))?;
            out.extend_from_slice(&piece);
            if out.len() as u64 >= r.get("total").and_then(Value::as_u64).unwrap_or(0)
                || piece.is_empty()
            {
                return Ok(out);
            }
        }
    }
    async fn unfreeze(&self, agent: &str, epoch: u64) -> CallResult<()> {
        self.call("unfreeze", json!({"agent": agent, "epoch": epoch}))
            .await
            .map(|_| ())
    }
    async fn tombstone(&self, agent: &str, epoch: u64) -> CallResult<()> {
        self.call("tombstone", json!({"agent": agent, "epoch": epoch}))
            .await
            .map(|_| ())
    }
    async fn offer(&self, sid: &str, manifest: &Value, epoch: u64) -> CallResult<Vec<String>> {
        let r = self
            .call(
                "offer",
                json!({"sid": sid, "manifest": manifest, "epoch": epoch}),
            )
            .await?;
        Ok(r.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default())
    }
    async fn receive(&self, sid: &str, rel: &str, data: &[u8]) -> CallResult<()> {
        let pieces: Vec<&[u8]> = if data.is_empty() {
            vec![&[][..]]
        } else {
            data.chunks(CHUNK).collect()
        };
        let mut offset = 0;
        for (i, piece) in pieces.iter().enumerate() {
            self.call(
                "receive_chunk",
                json!({"sid": sid, "rel": rel, "offset": offset, "data": B64.encode(piece), "last": i == pieces.len() - 1}),
            )
            .await?;
            offset += piece.len();
        }
        Ok(())
    }
    async fn verify(&self, sid: &str) -> CallResult<()> {
        self.call("verify", json!({"sid": sid})).await.map(|_| ())
    }
    async fn prepare(&self, sid: &str) -> CallResult<()> {
        self.call("prepare", json!({"sid": sid})).await.map(|_| ())
    }
    async fn activate(&self, sid: &str, epoch: u64) -> CallResult<()> {
        self.call("activate", json!({"sid": sid, "epoch": epoch}))
            .await
            .map(|_| ())
    }
    async fn abort(&self, sid: &str) -> CallResult<()> {
        self.call("abort", json!({"sid": sid})).await.map(|_| ())
    }
    async fn holder(&self, agent: &str) -> CallResult<Option<(String, u64)>> {
        let r = self.call("holder", json!({"agent": agent})).await?;
        Ok(r.as_array()
            .and_then(|a| Some((a.first()?.as_str()?.to_string(), a.get(1)?.as_u64()?))))
    }
}
