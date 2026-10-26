//! One-gesture approval of sensitive actions from a phone, as the Python reference
//! (`inari_core.approvals`, `approval_bus`). The node asks on the bus; the gateway,
//! which holds the device key, answers with a signed verdict. A verdict counts once,
//! before it expires, and only with the right signature; silence means no.

use crate::bus::Bus;
use crate::canon::canonical;
use crate::wire::{mac, random_hex};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::oneshot;

pub fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The verdict's signature: HMAC-SHA256 of the canonical `{"id","n","ok"}`.
pub fn sign(request: &Value, approved: bool, key: &[u8]) -> String {
    let body = json!({"id": request["request_id"], "n": request["nonce"], "ok": approved});
    hex(&mac(key, &[&canonical(&body)]))
}

/// Constant-time comparison of two signatures.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

type Waiter = (Value, oneshot::Sender<bool>);

/// The node's side: questions out on `approval.request`, verdicts in on `approval.response`.
pub struct NodeApprovals {
    bus: Arc<Bus>,
    key: Vec<u8>,
    pending: Arc<Mutex<HashMap<String, Waiter>>>,
}

impl NodeApprovals {
    pub fn start(bus: Arc<Bus>, key: &[u8]) -> Arc<Self> {
        let me = Arc::new(Self {
            bus: Arc::clone(&bus),
            key: key.to_vec(),
            pending: Arc::default(),
        });
        let mut answers = bus.subscribe("approval.response");
        let weak = Arc::downgrade(&me);
        tokio::spawn(async move {
            while let Some(m) = answers.recv().await {
                let Some(me) = weak.upgrade() else { break };
                me.resolve(&m.payload);
            }
        });
        me
    }

    /// Accept a verdict; false for anything not valid, fresh and unused.
    pub fn resolve(&self, response: &Value) -> bool {
        let id = response
            .get("request_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        let approved = response
            .get("approved")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let given = response
            .get("signature")
            .and_then(Value::as_str)
            .unwrap_or("");
        let mut pending = self.pending.lock().expect("not poisoned");
        let Some((request, _)) = pending.get(id) else {
            return false;
        };
        if now() > request["expires_at"].as_f64().unwrap_or(0.0)
            || !same(&sign(request, approved, &self.key), given)
        {
            return false;
        }
        let (_, waiter) = pending.remove(id).expect("present");
        let _ = waiter.send(approved);
        true
    }

    pub async fn ask(&self, agent: &str, node: &str, command: &str, ttl: f64) -> bool {
        let request = json!({"request_id": random_hex(16), "agent_id": agent, "node_id": node,
                             "command": command, "nonce": random_hex(16), "expires_at": now() + ttl});
        let id = request["request_id"].as_str().unwrap_or("").to_string();
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .expect("not poisoned")
            .insert(id.clone(), (request.clone(), tx));
        self.bus.publish("approval.request", request);
        let verdict = tokio::time::timeout(Duration::from_secs_f64(ttl), rx).await;
        self.pending.lock().expect("not poisoned").remove(&id);
        matches!(verdict, Ok(Ok(true))) // default deny
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_signature_matches_the_reference() {
        // python: hmac.new(b"k", canonical({"id": "r1", "n": "x", "ok": True}), sha256).hexdigest()
        let r = json!({"request_id": "r1", "nonce": "x"});
        assert_eq!(
            sign(&r, true, b"k"),
            "f0ee574da627005a41e83e6c4540fb658439d747c83d94e4ad2136edc15d489e"
        );
        assert_ne!(sign(&r, true, b"k"), sign(&r, false, b"k"));
    }

    #[tokio::test]
    async fn a_signed_verdict_counts_once_and_a_forged_one_never() {
        let bus = Bus::new("n");
        let approvals = NodeApprovals::start(Arc::clone(&bus), b"key");
        let mut questions = bus.subscribe("approval.request");
        let a = Arc::clone(&approvals);
        let asked = tokio::spawn(async move { a.ask("ag", "n", "rm -rf x", 5.0).await });
        let q = questions.recv().await.unwrap().payload;
        let forged = json!({"request_id": q["request_id"], "approved": true, "signature": sign(&q, true, b"other")});
        assert!(!approvals.resolve(&forged));
        let good = json!({"request_id": q["request_id"], "approved": true, "signature": sign(&q, true, b"key")});
        assert!(approvals.resolve(&good));
        assert!(!approvals.resolve(&good), "a verdict counts once");
        assert!(asked.await.unwrap());
    }

    #[tokio::test]
    async fn silence_means_no() {
        let bus = Bus::new("n");
        let approvals = NodeApprovals::start(bus, b"key");
        assert!(!approvals.ask("ag", "n", "x", 0.2).await);
    }
}
