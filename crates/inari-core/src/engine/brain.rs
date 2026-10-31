//! The `kitsune` tool: the agents' hands on the core's second brain (its notes, reminders and
//! lists), so an assistant can keep a thought, find one, set a reminder or fill the shopping list.
//! One tool with an `action`, as `project_tasks`, so a small model has one schema to learn.
//! Writes under a pass (someone else's request) are the owner's to allow.

use super::LocalEngine;
use crate::agent::Turn;
use kitsune::{Capture, Kitsune, Mode, NewItem, Reminder, SearchOptions};
use serde_json::{json, Value};

/// The actions that change the brain (the others only read it).
const WRITES: [&str; 7] = [
    "note",
    "tag",
    "remind",
    "dismiss",
    "add_items",
    "check",
    "remove_items",
];

pub fn definition() -> Value {
    json!({"type": "function", "function": {"name": "kitsune",
        "description": "The person's second brain (Kitsune): their notes, reminders and lists (todo, shopping, pantry). Actions: search {query, mode?, limit?}; recent {limit?}; note {text, tags?} keeps a thought (read like any capture: summary, category, reminder); tag {id, tags}; reminders {trigger_type?}; remind {id, due_date (ISO 8601 UTC)}; dismiss {id}; lists; items {list, with_done?}; add_items {list, kind (todo|shopping|pantry|custom), items: [{text, qty?, unit?, aisle?, due_date?, expires_on? (YYYY-MM-DD)}]}; check {ids, done?}; remove_items {ids}; expiring {day (YYYY-MM-DD)}.",
        "parameters": {"type": "object", "properties": {
            "action": {"type": "string", "enum": ["search", "recent", "note", "tag", "reminders", "remind", "dismiss", "lists", "items", "add_items", "check", "remove_items", "expiring"]},
            "query": {"type": "string"},
            "mode": {"type": "string", "enum": ["hybrid", "fts", "semantic"]},
            "limit": {"type": "number"},
            "text": {"type": "string"},
            "tags": {"type": "array", "items": {"type": "string"}},
            "id": {"type": "number"},
            "ids": {"type": "array", "items": {"type": "number"}},
            "trigger_type": {"type": "string", "enum": ["time", "geofence"]},
            "due_date": {"type": "string"},
            "list": {"type": "string"},
            "kind": {"type": "string", "enum": ["todo", "shopping", "pantry", "custom"]},
            "with_done": {"type": "boolean"},
            "done": {"type": "boolean"},
            "day": {"type": "string"},
            "items": {"type": "array", "items": {"type": "object"}}},
            "required": ["action"]}}})
}

/// One call of the tool on a brain (blocking: SQLite and the local model servers).
pub fn run(k: &Kitsune, args: &Value) -> Result<Value, String> {
    let s = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let n = |key: &str| args.get(key).and_then(Value::as_i64);
    let ids: Vec<i64> = args
        .get("ids")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default();
    let tags: Vec<String> = args
        .get("tags")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    let limit = n("limit").unwrap_or(10).clamp(1, 50) as usize;
    let e = |e: kitsune::Error| e.to_string();
    let store = k.store();
    Ok(match s("action").as_str() {
        "search" => {
            let mode: Mode = s("mode").parse().map_err(e)?;
            let hits = k
                .search(
                    &s("query"),
                    &SearchOptions {
                        mode,
                        limit,
                        ..Default::default()
                    },
                )
                .map_err(e)?;
            json!({"notes": hits.into_iter().map(|h| h.note).collect::<Vec<_>>()})
        }
        "recent" => json!({"notes": store.recent_notes(limit).map_err(e)?}),
        "note" => {
            let note = k
                .keep(&Capture {
                    raw: s("text"),
                    source: "mcp".into(),
                    tags,
                    ..Default::default()
                })
                .map_err(e)?;
            json!({"note": note})
        }
        "tag" => {
            let id = n("id").ok_or("tag needs an id")?;
            let note = store
                .get_note(id)
                .map_err(e)?
                .ok_or(format!("no note {id}"))?;
            let mut all = note.tags.clone();
            all.extend(tags);
            store
                .set_reading(id, &note.clean_summary, &note.category, &all)
                .map_err(e)?;
            json!({"ok": true})
        }
        "reminders" => {
            let kind = s("trigger_type");
            let kind = (!kind.is_empty()).then_some(kind.as_str());
            json!({"reminders": store.active_reminders(kind).map_err(e)?})
        }
        "remind" => {
            let id = n("id").ok_or("remind needs an id")?;
            let ok = store
                .set_reminder(
                    id,
                    &Reminder::Time {
                        due_date: s("due_date"),
                    },
                )
                .map_err(e)?;
            json!({"ok": ok})
        }
        "dismiss" => {
            json!({"ok": store.dismiss(n("id").ok_or("dismiss needs an id")?).map_err(e)?})
        }
        "lists" => json!({"lists": store.lists().map_err(e)?}),
        "items" => {
            let with_done = args
                .get("with_done")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            json!({"items": store.items(&s("list"), with_done).map_err(e)?})
        }
        "add_items" => {
            let items: Vec<NewItem> =
                serde_json::from_value(args.get("items").cloned().unwrap_or(json!([])))
                    .map_err(|e| format!("items: {e}"))?;
            let kind = if s("kind").is_empty() {
                "custom".to_string()
            } else {
                s("kind")
            };
            json!({"ids": store.add_items(&s("list"), &kind, &items).map_err(e)?})
        }
        "check" => {
            let done = args.get("done").and_then(Value::as_bool).unwrap_or(true);
            json!({"changed": store.check_items(&ids, done).map_err(e)?})
        }
        "remove_items" => json!({"removed": store.remove_items(&ids).map_err(e)?}),
        "expiring" => json!({"items": store.expiring(&s("day")).map_err(e)?}),
        other => return Err(format!("kitsune has no action {other:?}")),
    })
}

impl LocalEngine {
    pub(crate) async fn kitsune_tool(&self, t: &Turn, args: &Value) -> Result<String, String> {
        let dir = self
            .kitsune_dir
            .clone()
            .ok_or("this core holds no Kitsune brain")?;
        let action = args.get("action").and_then(Value::as_str).unwrap_or("");
        if !t.pass.is_empty()
            && WRITES.contains(&action)
            && !t
                .runner
                .approve(&t.agent, &format!("kitsune {action}"))
                .await
        {
            return Err(format!("the person did not allow kitsune {action}"));
        }
        let args = args.clone();
        let out = tokio::task::spawn_blocking(move || {
            let k = Kitsune::shared(&dir).map_err(|e| e.to_string())?;
            let k = k.lock().map_err(|_| "the brain is poisoned".to_string())?;
            run(&k, &args)
        })
        .await
        .map_err(|e| e.to_string())??;
        Ok(out.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_keeps_fills_and_reads_the_brain() {
        let dir = tempfile::tempdir().unwrap();
        let k = Kitsune::open(dir.path()).unwrap();
        let kept = run(
            &k,
            &json!({"action": "note", "text": "Appeler le comptable", "tags": ["pro"]}),
        )
        .unwrap();
        let id = kept["note"]["id"].as_i64().unwrap();
        assert_eq!(
            run(&k, &json!({"action": "search", "query": "comptable"})).unwrap()["notes"][0]["id"],
            id
        );
        run(
            &k,
            &json!({"action": "remind", "id": id, "due_date": "2030-01-01T09:00:00Z"}),
        )
        .unwrap();
        assert_eq!(
            run(&k, &json!({"action": "reminders"})).unwrap()["reminders"][0]["id"],
            id
        );
        run(&k, &json!({"action": "tag", "id": id, "tags": ["urgent"]})).unwrap();
        assert_eq!(
            run(&k, &json!({"action": "recent"})).unwrap()["notes"][0]["tags"],
            json!(["pro", "urgent"])
        );

        let added = run(&k, &json!({"action": "add_items", "list": "Courses", "kind": "shopping",
            "items": [{"text": "café", "aisle": "3-épicerie"}, {"text": "lait", "qty": 2, "unit": "L"}]})).unwrap();
        let ids = added["ids"].clone();
        assert_eq!(
            run(&k, &json!({"action": "lists"})).unwrap()["lists"][0]["open"],
            2
        );
        run(&k, &json!({"action": "check", "ids": [ids[0]]})).unwrap();
        assert_eq!(
            run(&k, &json!({"action": "items", "list": "Courses"})).unwrap()["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(run(&k, &json!({"action": "fly"})).is_err());
        assert!(run(
            &k,
            &json!({"action": "add_items", "list": "x", "kind": "fridge", "items": [{"text": "a"}]})
        )
        .is_err());
    }
}
