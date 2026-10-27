//! Core passes: a person who is not the core's owner gets threads on it. In a
//! *collaboration* pass both sides see and act on the shared threads; in a *supervision*
//! pass (a minor, say) the owner follows every thread and answers the agents'
//! permission requests, and the guest always sees that it is so. A pass names the
//! harnesses its threads may use and may end on a date.
//!
//! The hub keeps the passes (`passes.json` in its data directory) and tells every
//! gateway (`mesh.passes`); a gateway knows a pass by the hash of its token only.

use crate::bus::Bus;
use crate::canon::sha256_hex;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const ROLES: [&str; 2] = ["collaboration", "supervision"];

/// The hash a token is known by.
pub fn token_hash(token: &str) -> String {
    sha256_hex(token.trim().as_bytes())
}

pub struct PassBook {
    file: PathBuf,
    passes: Mutex<Vec<Value>>,
    bus: Arc<Bus>,
}

impl PassBook {
    /// The hub's book: read from disk, changed by `pass.create` / `pass.revoke`, told to
    /// every gateway on change and every few seconds (a gateway that just started learns it).
    pub fn start(bus: &Arc<Bus>, data_dir: &Path) -> Arc<Self> {
        let file = data_dir.join("passes.json");
        let passes = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let me = Arc::new(Self {
            file,
            passes: Mutex::new(passes),
            bus: Arc::clone(bus),
        });
        let mut asks = bus.subscribe("pass.*");
        let book = Arc::clone(&me);
        tokio::spawn(async move {
            while let Some(m) = asks.recv().await {
                match m.subject.as_str() {
                    "pass.create" => book.create(m.payload),
                    "pass.revoke" => {
                        book.revoke(m.payload.get("id").and_then(Value::as_str).unwrap_or(""))
                    }
                    _ => {}
                }
            }
        });
        let tell = Arc::clone(&me);
        tokio::spawn(async move {
            loop {
                tell.announce();
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
        me
    }

    fn save(&self, all: &[Value]) {
        let _ = crate::capsule::io::atomic_write(
            &self.file,
            serde_json::to_string_pretty(all)
                .expect("serializes")
                .as_bytes(),
        );
    }

    pub fn announce(&self) {
        let all = self.passes.lock().expect("not poisoned").clone();
        self.bus.publish("mesh.passes", json!({"passes": all}));
    }

    fn create(&self, p: Value) {
        let field = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let (id, name, role, hash) = (
            field("id"),
            field("name"),
            field("role"),
            field("token_sha"),
        );
        if id.is_empty() || name.is_empty() || hash.len() != 64 || !ROLES.contains(&role.as_str()) {
            return;
        }
        let pass = json!({"id": id, "name": name, "role": role, "token_sha": hash,
                          "harnesses": p.get("harnesses").cloned().unwrap_or(json!([])),
                          "until": p.get("until").and_then(Value::as_f64).unwrap_or(0.0),
                          "created": crate::approvals::now()});
        {
            let mut all = self.passes.lock().expect("not poisoned");
            all.retain(|x| x["id"] != pass["id"]);
            all.push(pass);
            self.save(&all);
        }
        self.announce();
    }

    fn revoke(&self, id: &str) {
        {
            let mut all = self.passes.lock().expect("not poisoned");
            all.retain(|x| x["id"] != id);
            self.save(&all);
        }
        self.announce();
    }
}

/// A pass is valid when it exists and its end date (if any) has not come.
pub fn valid(pass: &Value) -> bool {
    let until = pass.get("until").and_then(Value::as_f64).unwrap_or(0.0);
    until <= 0.0 || crate::approvals::now() < until
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn passes_are_kept_told_and_revoked() {
        let dir = tempfile::tempdir().unwrap();
        let bus = Bus::new("hub");
        let mut told = bus.subscribe("mesh.passes");
        let _book = PassBook::start(&bus, dir.path());
        told.recv().await.unwrap(); // the first announcement
        bus.publish("pass.create", json!({"id": "p1", "name": "Léo", "role": "supervision",
                                           "token_sha": token_hash("secret"), "harnesses": ["openfox"]}));
        let m = told.recv().await.unwrap();
        assert_eq!(m.payload["passes"][0]["name"], "Léo");
        let on_disk: Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("passes.json")).unwrap())
                .unwrap();
        assert_eq!(on_disk[0]["token_sha"], token_hash("secret"));
        bus.publish(
            "pass.create",
            json!({"id": "p2", "name": "x", "role": "admin", "token_sha": token_hash("y")}),
        );
        bus.publish("pass.revoke", json!({"id": "p1"}));
        let m = told.recv().await.unwrap();
        assert_eq!(
            m.payload["passes"],
            json!([]),
            "an unknown role is refused, a revoked pass is gone"
        );
    }

    #[test]
    fn a_pass_ends_on_its_date() {
        assert!(valid(&json!({"until": 0})));
        assert!(!valid(&json!({"until": 1.0})));
        assert!(valid(&json!({"until": crate::approvals::now() + 60.0})));
    }
}
