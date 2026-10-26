//! The append-only journal: a grow-only set of events, merged by union.

use super::model::{EventKind, TimelineEvent};
use super::{Error, Result};
use crate::canon::digest_of;
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct Timeline {
    by_id: BTreeMap<String, TimelineEvent>,
}

impl Timeline {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// Insert an event; `Ok(false)` if it was already known.
    pub fn add(&mut self, event: TimelineEvent) -> Result<bool> {
        match self.by_id.get(&event.id) {
            None => {
                self.by_id.insert(event.id.clone(), event);
                Ok(true)
            }
            Some(known) if known.hash == event.hash => Ok(false),
            Some(_) => Err(Error::Conflict(format!(
                "two different events share the id {}",
                event.id
            ))),
        }
    }

    pub fn ordered(&self) -> Vec<&TimelineEvent> {
        let mut events: Vec<_> = self.by_id.values().collect();
        events.sort_by(|a, b| {
            (a.lamport, &a.node_id, a.node_seq).cmp(&(b.lamport, &b.node_id, b.node_seq))
        });
        events
    }

    pub fn clock(&self) -> u64 {
        self.by_id.values().map(|e| e.lamport).max().unwrap_or(0)
    }

    fn last_of(&self, node_id: &str) -> Option<&TimelineEvent> {
        self.by_id
            .values()
            .filter(|e| e.node_id == node_id)
            .max_by_key(|e| e.node_seq)
    }

    pub fn append(
        &mut self,
        node_id: &str,
        kind: EventKind,
        data: Value,
        ts: f64,
    ) -> TimelineEvent {
        let (seq, prev) = match self.last_of(node_id) {
            Some(last) => (last.node_seq + 1, last.hash.clone()),
            None => (1, String::new()),
        };
        let event = TimelineEvent::make(node_id, seq, self.clock() + 1, ts, kind, data, &prev);
        self.by_id.insert(event.id.clone(), event.clone());
        event
    }

    /// Union with another replica: commutative, associative, idempotent.
    pub fn merge(&mut self, other: &Timeline) -> Result<usize> {
        let mut added = 0;
        for event in other.by_id.values() {
            if self.add(event.clone())? {
                added += 1;
            }
        }
        Ok(added)
    }

    /// Problems found; empty when every per-author chain is intact.
    pub fn verify(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let mut by_node: BTreeMap<&str, Vec<&TimelineEvent>> = BTreeMap::new();
        for event in self.by_id.values() {
            by_node
                .entry(event.node_id.as_str())
                .or_default()
                .push(event);
        }
        for (node, mut events) in by_node {
            events.sort_by_key(|e| e.node_seq);
            let mut prev = String::new();
            for (i, event) in events.iter().enumerate() {
                if event.node_seq != i as u64 + 1 {
                    problems.push(format!("{node}: gap before seq {}", event.node_seq));
                    break;
                }
                if event.hash != event.compute_hash() {
                    problems.push(format!("{}: hash does not match content", event.id));
                }
                if event.prev != prev {
                    problems.push(format!("{}: broken chain", event.id));
                }
                prev = event.hash.clone();
            }
        }
        problems
    }

    /// Order-independent fingerprint: the digest of the sorted (id, hash) pairs.
    pub fn head(&self) -> String {
        let mut pairs: Vec<(&String, &String)> =
            self.by_id.values().map(|e| (&e.id, &e.hash)).collect();
        pairs.sort();
        let list: Vec<Value> = pairs.into_iter().map(|(i, h)| json!([i, h])).collect();
        digest_of(&Value::Array(list))
    }

    pub fn dumps(&self) -> String {
        let mut out = String::new();
        for event in self.ordered() {
            let line = serde_json::to_value(event).expect("an event serializes");
            out.push_str(&serde_json::to_string(&line).expect("a value serializes"));
            out.push('\n');
        }
        out
    }

    pub fn loads(text: &str) -> Result<Self> {
        let mut timeline = Self::new();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            timeline.add(serde_json::from_str(line)?)?;
        }
        Ok(timeline)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merge_is_a_union_whatever_the_order() {
        let mut a = Timeline::new();
        let mut b = Timeline::new();
        a.append("n1", EventKind::UserMessage, json!({"text": "1"}), 1.0);
        b.append("n2", EventKind::UserMessage, json!({"text": "2"}), 1.5);
        let (mut left, mut right) = (a.clone(), b.clone());
        left.merge(&b).unwrap();
        right.merge(&a).unwrap();
        assert_eq!(left.head(), right.head());
        assert_eq!(left.merge(&b).unwrap(), 0);
        assert!(left.verify().is_empty());
    }

    #[test]
    fn two_events_with_one_id_are_refused() {
        let mut a = Timeline::new();
        let mut b = Timeline::new();
        a.append("n", EventKind::UserMessage, json!({"text": "left"}), 1.0);
        b.append("n", EventKind::UserMessage, json!({"text": "right"}), 1.0);
        assert!(matches!(a.merge(&b), Err(Error::Conflict(_))));
    }

    #[test]
    fn a_rewritten_event_is_caught() {
        let mut t = Timeline::new();
        t.append(
            "n",
            EventKind::UserMessage,
            json!({"text": "build it"}),
            1.0,
        );
        let mut text = t.dumps();
        text = text.replace("build it", "delete it");
        let back = Timeline::loads(&text).unwrap();
        assert!(back
            .verify()
            .iter()
            .any(|p| p.contains("hash does not match")));
    }
}
