//! The mascot's personas, and the one each person chose.
//!
//! FoxBot's folder (`mascot_dir`) holds the original fox under `sprites/<action>/` and each
//! persona under `sprites_persona/<persona>/<action>/`. A persona is the person's own choice,
//! not the model's: the owner and each pass holder keep theirs, here, in
//! `<data_dir>/personas.json`, so it follows them from one device to the other. An empty choice
//! is the original FoxBot.

use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A persona's name: what may name a folder and travel in a URL.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 40
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// The actions under `dir` that have frames (`<action>/f_00.webp`).
fn actions(dir: &Path) -> BTreeSet<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return BTreeSet::new();
    };
    entries
        .filter_map(Result::ok)
        .filter(|e| e.path().join("f_00.webp").is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| valid_name(n))
        .collect()
}

/// The personas in FoxBot's folder: for each, how many of the original fox's actions it has
/// (`shared` of `of`), and its own (`signatures`). Sorted by name.
pub fn list(mascot_dir: &Path) -> Vec<Value> {
    let original = actions(&mascot_dir.join("sprites"));
    let Ok(entries) = std::fs::read_dir(mascot_dir.join("sprites_persona")) else {
        return Vec::new();
    };
    let mut out: Vec<Value> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| valid_name(n))
        .map(|name| {
            let own = actions(&mascot_dir.join("sprites_persona").join(&name));
            let shared = own.intersection(&original).count();
            let signatures: Vec<&String> = own.difference(&original).collect();
            json!({"id": name, "shared": shared, "of": original.len(), "signatures": signatures})
        })
        .filter(|p| p["shared"].as_u64().unwrap_or(0) > 0)
        .collect();
    out.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    out
}

/// Each person's choice, by who they are: `owner`, or `pass:<id>`.
pub struct Choices {
    file: PathBuf,
    map: Mutex<Map<String, Value>>,
}

impl Choices {
    pub fn open(data_dir: &Path) -> Self {
        let file = data_dir.join("personas.json");
        let map = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self {
            file,
            map: Mutex::new(map),
        }
    }

    /// The persona `who` chose; empty for the original FoxBot (or no choice yet).
    pub fn get(&self, who: &str) -> String {
        let map = self.map.lock().expect("not poisoned");
        map.get(who)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    }

    /// Keep `who`'s choice (empty: the original FoxBot), written at once.
    pub fn set(&self, who: &str, persona: &str) -> std::io::Result<()> {
        let mut map = self.map.lock().expect("not poisoned");
        map.insert(who.to_string(), json!(persona));
        crate::capsule::io::atomic_write(
            &self.file,
            serde_json::to_vec_pretty(&*map)
                .expect("serializes")
                .as_slice(),
        )
        .map_err(|e| std::io::Error::other(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("f_00.webp"), b"x").unwrap();
    }

    #[test]
    fn personas_are_listed_with_what_they_share_with_the_fox() {
        let d = tempfile::tempdir().unwrap();
        for a in ["respire", "cligne", "tourne"] {
            frames(&d.path().join("sprites").join(a));
        }
        for a in ["respire", "cligne", "fume_clope"] {
            frames(&d.path().join("sprites_persona/rebel").join(a));
        }
        frames(&d.path().join("sprites_persona/zen/respire"));
        std::fs::create_dir_all(d.path().join("sprites_persona/empty/respire")).unwrap(); // no frames
        frames(&d.path().join("sprites_persona/Bad Name/respire"));
        let all = list(d.path());
        let ids: Vec<&str> = all.iter().map(|p| p["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["rebel", "zen"]);
        assert_eq!(all[0]["shared"], 2);
        assert_eq!(all[0]["of"], 3);
        assert_eq!(all[0]["signatures"], json!(["fume_clope"]));
        assert!(list(&d.path().join("nowhere")).is_empty());
    }

    #[test]
    fn a_choice_is_kept_per_person_across_restarts() {
        let d = tempfile::tempdir().unwrap();
        let c = Choices::open(d.path());
        assert_eq!(c.get("owner"), "");
        c.set("owner", "zen").unwrap();
        c.set("pass:ab12", "rebel").unwrap();
        let again = Choices::open(d.path());
        assert_eq!(again.get("owner"), "zen");
        assert_eq!(again.get("pass:ab12"), "rebel");
        again.set("owner", "").unwrap();
        assert_eq!(Choices::open(d.path()).get("owner"), "");
    }
}
