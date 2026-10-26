//! An embedded event bus with NATS-style subjects (`*` one token, `>` the rest), and
//! links between buses. A message records the buses it crossed, so a ring delivers
//! once; receivers drop duplicates by message id; durable subjects wait in an outbox
//! while a link is down. Same message shape as the reference, so Rust and Python
//! buses can be linked.

use serde_json::{json, Value};
use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

pub const DURABLE: [&str; 3] = ["teleport.>", "approval.>", "agent.>"];

pub fn subject_matches(pattern: &str, subject: &str) -> bool {
    let p: Vec<&str> = pattern.split('.').collect();
    let s: Vec<&str> = subject.split('.').collect();
    for (i, token) in p.iter().enumerate() {
        if *token == ">" {
            return i == p.len() - 1 && s.len() > i;
        }
        if i >= s.len() || (*token != "*" && *token != s[i]) {
            return false;
        }
    }
    p.len() == s.len()
}

pub fn is_durable(subject: &str) -> bool {
    DURABLE.iter().any(|p| subject_matches(p, subject))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub subject: String,
    pub payload: Value,
    pub sender: String,
    pub msg_id: String,
    pub path: Vec<String>,
}

impl Message {
    pub fn to_json(&self) -> Value {
        json!({"subject": self.subject, "payload": self.payload, "sender": self.sender,
               "msg_id": self.msg_id, "path": self.path})
    }

    pub fn from_json(v: &Value) -> Option<Self> {
        Some(Self {
            subject: v.get("subject")?.as_str()?.to_string(),
            payload: v.get("payload").cloned().unwrap_or(Value::Null),
            sender: v
                .get("sender")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            msg_id: v.get("msg_id")?.as_str()?.to_string(),
            path: v
                .get("path")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
        })
    }
}

/// Where a bus forwards messages: the peer's name (for loop prevention) and a queue
/// the link's own task drains — so publishing never waits on a network.
#[derive(Clone)]
pub struct LinkHandle {
    pub peer_name: Arc<Mutex<String>>,
    pub tx: mpsc::UnboundedSender<Message>,
}

struct Seen {
    set: HashSet<String>,
    order: VecDeque<String>,
    limit: usize,
}

pub struct Bus {
    pub name: String,
    subs: Mutex<Vec<(String, mpsc::UnboundedSender<Message>)>>,
    links: Mutex<Vec<LinkHandle>>,
    seen: Mutex<Seen>,
}

impl Bus {
    pub fn new(name: &str) -> Arc<Self> {
        Arc::new(Self {
            name: name.into(),
            subs: Mutex::default(),
            links: Mutex::default(),
            seen: Mutex::new(Seen {
                set: HashSet::new(),
                order: VecDeque::new(),
                limit: 4096,
            }),
        })
    }

    /// Messages whose subject matches `pattern`, in order, on a channel.
    pub fn subscribe(&self, pattern: &str) -> mpsc::UnboundedReceiver<Message> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.subs
            .lock()
            .expect("not poisoned")
            .push((pattern.into(), tx));
        rx
    }

    pub fn add_link(&self, link: LinkHandle) {
        self.links.lock().expect("not poisoned").push(link);
    }

    pub fn publish(&self, subject: &str, payload: Value) -> usize {
        let message = Message {
            subject: subject.into(),
            payload,
            sender: self.name.clone(),
            msg_id: crate::wire::random_hex(16),
            path: vec![self.name.clone()],
        };
        self.dispatch(message)
    }

    /// A message arriving over a link; duplicates are dropped.
    pub fn deliver(&self, message: Message) -> usize {
        if self
            .seen
            .lock()
            .expect("not poisoned")
            .set
            .contains(&message.msg_id)
        {
            return 0;
        }
        self.dispatch(message)
    }

    fn dispatch(&self, message: Message) -> usize {
        {
            let mut seen = self.seen.lock().expect("not poisoned");
            if seen.set.insert(message.msg_id.clone()) {
                seen.order.push_back(message.msg_id.clone());
            }
            while seen.order.len() > seen.limit {
                if let Some(old) = seen.order.pop_front() {
                    seen.set.remove(&old);
                }
            }
        }
        let mut delivered = 0;
        self.subs
            .lock()
            .expect("not poisoned")
            .retain(|(pattern, tx)| {
                if subject_matches(pattern, &message.subject) {
                    if tx.send(message.clone()).is_err() {
                        return false; // the subscriber is gone
                    }
                    delivered += 1;
                }
                true
            });
        let mut forwarded = message.clone();
        forwarded.path.push(self.name.clone());
        for link in self.links.lock().expect("not poisoned").iter() {
            let peer = link.peer_name.lock().expect("not poisoned").clone();
            if !message.path.contains(&peer) {
                let _ = link.tx.send(forwarded.clone());
            }
        }
        delivered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subjects_match_like_nats() {
        for (p, s, ok) in [
            ("a.b", "a.b", true),
            ("a.*", "a.b", true),
            ("a.*", "a.b.c", false),
            ("a.>", "a.b.c", true),
            ("a.>", "a", false),
            ("agent.*.state", "agent.x.state", true),
            ("agent.*.state", "agent.x.load", false),
        ] {
            assert_eq!(subject_matches(p, s), ok, "{p} {s}");
        }
    }

    #[test]
    fn subscribers_get_their_subjects_and_a_message_is_seen_once() {
        let bus = Bus::new("a");
        let mut rx = bus.subscribe("teleport.>");
        assert_eq!(bus.publish("teleport.request", json!({"n": 1})), 1);
        assert_eq!(bus.publish("noise", json!({})), 0);
        let m = rx.try_recv().unwrap();
        assert_eq!(m.payload["n"], 1);
        assert_eq!(bus.deliver(m), 0, "duplicate dropped");
    }
}
