//! Kitsune's MCP face: the person's memory offered to agentic models (Claude, the local engine,
//! n8n, Home Assistant) over the Model Context Protocol, with exactly four typed tools:
//!
//! - `search_notes(query, mode = "hybrid" | "fts" | "semantic", limit = 5) -> [Note]`
//! - `get_active_reminders(trigger_type?: "time" | "geofence") -> [Note]`
//! - `create_note(raw_transcript) -> Note`
//! - `dismiss_reminder(note_id) -> bool`
//!
//! The protocol lives here; what the tools do is a [`Memory`]'s (the store alone, or the whole
//! brain with its local models). The transport (stdio, or the gateway's HTTP door) is a thin loop
//! around [`handle`]. Which tools are offered and the token are configuration, never hard-coded.

use crate::search::{Mode, SearchOptions};
use crate::store::{NewNote, Note};
use crate::{Result, Store};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The protocol revision Kitsune speaks.
const PROTOCOL: &str = "2025-06-18";

/// The most notes a search returns.
const MAX_LIMIT: usize = 50;

/// What the four tools do.
pub trait Memory {
    fn search_notes(&self, query: &str, mode: Mode, limit: usize) -> Result<Vec<Note>>;
    fn get_active_reminders(&self, trigger_type: Option<&str>) -> Result<Vec<Note>>;
    fn create_note(&self, raw_transcript: &str) -> Result<Note>;
    fn dismiss_reminder(&self, note_id: i64) -> Result<bool>;
}

/// The store alone: words only (no embedding model), a note kept as said (no small model).
impl Memory for Store {
    fn search_notes(&self, query: &str, mode: Mode, limit: usize) -> Result<Vec<Note>> {
        let opts = SearchOptions {
            mode,
            limit,
            ..Default::default()
        };
        Ok(self
            .search(query, None, &opts)?
            .into_iter()
            .map(|h| h.note)
            .collect())
    }
    fn get_active_reminders(&self, trigger_type: Option<&str>) -> Result<Vec<Note>> {
        self.active_reminders(trigger_type)
    }
    fn create_note(&self, raw_transcript: &str) -> Result<Note> {
        let id = self.insert_note(&NewNote {
            raw_transcript: raw_transcript.into(),
            source: "mcp".into(),
            ..Default::default()
        })?;
        Ok(self.get_note(id)?.expect("just kept"))
    }
    fn dismiss_reminder(&self, note_id: i64) -> Result<bool> {
        self.dismiss(note_id)
    }
}

/// Which tools a client may use, and the token it must present. Set from the GUI, never here.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Config {
    /// If set, a call must carry this token (an `_auth` argument, or the gateway's Bearer).
    #[serde(default)]
    pub token: String,
    /// The tools offered; empty means all four.
    #[serde(default)]
    pub tools: Vec<String>,
}

impl Config {
    pub fn offers(&self, name: &str) -> bool {
        self.tools.is_empty() || self.tools.iter().any(|t| t == name)
    }
}

fn note_schema() -> Value {
    json!({"type": "object", "properties": {
        "id": {"type": "integer"}, "raw_transcript": {"type": "string"},
        "clean_summary": {"type": "string"}, "category": {"type": "string"},
        "tags": {"type": "array", "items": {"type": "string"}},
        "trigger_type": {"type": "string", "enum": ["none", "time", "geofence"]},
        "due_date": {"type": ["string", "null"]}, "latitude": {"type": ["number", "null"]},
        "longitude": {"type": ["number", "null"]}, "radius_meters": {"type": "integer"},
        "status": {"type": "string"}, "created_at": {"type": "string"}},
        "required": ["id", "raw_transcript", "clean_summary", "category", "tags", "trigger_type", "status", "created_at"]})
}

/// The four tools, with their input and output schemas.
fn all_tools() -> Vec<Value> {
    let notes_out = json!({"type": "object", "properties": {"notes": {"type": "array", "items": note_schema()}}, "required": ["notes"]});
    vec![
        json!({"name": "search_notes",
            "description": "Search the person's notes by words (fts), by meaning (semantic) or both (hybrid).",
            "inputSchema": {"type": "object", "properties": {
                "query": {"type": "string"},
                "mode": {"type": "string", "enum": ["hybrid", "fts", "semantic"], "default": "hybrid"},
                "limit": {"type": "integer", "minimum": 1, "maximum": MAX_LIMIT, "default": 5}},
                "required": ["query"]},
            "outputSchema": notes_out.clone()}),
        json!({"name": "get_active_reminders",
            "description": "The reminders still waiting: by time, by place (geofence), or both.",
            "inputSchema": {"type": "object", "properties": {
                "trigger_type": {"type": "string", "enum": ["time", "geofence"]}}},
            "outputSchema": notes_out}),
        json!({"name": "create_note",
            "description": "Keep a note from what the person said or wrote; it is read, tagged and given its reminder locally.",
            "inputSchema": {"type": "object", "properties": {"raw_transcript": {"type": "string"}},
                "required": ["raw_transcript"]},
            "outputSchema": {"type": "object", "properties": {"note": note_schema()}, "required": ["note"]}}),
        json!({"name": "dismiss_reminder",
            "description": "A reminder done with: it stops ringing and leaves the active list.",
            "inputSchema": {"type": "object", "properties": {"note_id": {"type": "integer"}},
                "required": ["note_id"]},
            "outputSchema": {"type": "object", "properties": {"dismissed": {"type": "boolean"}}, "required": ["dismissed"]}}),
    ]
}

/// Handle one MCP request; the JSON-RPC response (None for a notification).
pub fn handle(memory: &dyn Memory, cfg: &Config, request: &Value) -> Option<Value> {
    let id = request.get("id").cloned();
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let params = request.get("params").cloned().unwrap_or(Value::Null);
    let reply = |result: std::result::Result<Value, (i64, String)>| -> Option<Value> {
        let id = id.clone()?;
        Some(match result {
            Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
            Err((code, message)) => {
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
            }
        })
    };
    match method {
        "initialize" => reply(Ok(json!({
            "protocolVersion": PROTOCOL,
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "kitsune", "version": env!("CARGO_PKG_VERSION")}
        }))),
        m if m.starts_with("notifications/") => None,
        "ping" => reply(Ok(json!({}))),
        "tools/list" => {
            let tools: Vec<Value> = all_tools()
                .into_iter()
                .filter(|t| cfg.offers(t["name"].as_str().unwrap_or("")))
                .collect();
            reply(Ok(json!({"tools": tools})))
        }
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            if !cfg.token.is_empty()
                && args.get("_auth").and_then(Value::as_str) != Some(cfg.token.as_str())
            {
                return reply(Err((
                    -32001,
                    "this tool needs the right _auth token".into(),
                )));
            }
            if !cfg.offers(name) || !all_tools().iter().any(|t| t["name"] == name) {
                return reply(Err((-32601, format!("Kitsune does not offer {name}"))));
            }
            reply(Ok(match call_tool(memory, name, &args) {
                Ok(structured) => json!({
                    "content": [{"type": "text", "text": serde_json::to_string_pretty(&structured).expect("serializes")}],
                    "structuredContent": structured
                }),
                Err(e) => json!({"content": [{"type": "text", "text": e}], "isError": true}),
            }))
        }
        other => reply(Err((-32601, format!("unknown method {other}")))),
    }
}

fn call_tool(memory: &dyn Memory, name: &str, a: &Value) -> std::result::Result<Value, String> {
    let err = |e: crate::Error| e.to_string();
    match name {
        "search_notes" => {
            let query = a
                .get("query")
                .and_then(Value::as_str)
                .ok_or("search_notes needs a query")?;
            let mode: Mode = a
                .get("mode")
                .and_then(Value::as_str)
                .unwrap_or("hybrid")
                .parse()
                .map_err(err)?;
            let limit = a
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(5)
                .clamp(1, MAX_LIMIT as u64) as usize;
            let notes = memory.search_notes(query, mode, limit).map_err(err)?;
            Ok(json!({"notes": notes}))
        }
        "get_active_reminders" => {
            let kind = a.get("trigger_type").and_then(Value::as_str);
            Ok(json!({"notes": memory.get_active_reminders(kind).map_err(err)?}))
        }
        "create_note" => {
            let raw = a
                .get("raw_transcript")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .ok_or("create_note needs a raw_transcript")?;
            Ok(json!({"note": memory.create_note(raw).map_err(err)?}))
        }
        "dismiss_reminder" => {
            let id = a
                .get("note_id")
                .and_then(Value::as_i64)
                .ok_or("dismiss_reminder needs a note_id")?;
            Ok(json!({"dismissed": memory.dismiss_reminder(id).map_err(err)?}))
        }
        other => Err(format!("unknown tool {other}")),
    }
}

/// Drive a stream of MCP requests (one JSON object per line, as stdio transports send them) and
/// write each reply as a line. Returns when the input ends.
pub fn serve_lines<R: std::io::BufRead, W: std::io::Write>(
    memory: &dyn Memory,
    cfg: &Config,
    input: R,
    mut output: W,
) -> Result<()> {
    for line in input.lines() {
        let line = line.map_err(|e| crate::Error::Invalid(e.to_string()))?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(request) => handle(memory, cfg, &request),
            Err(_) => Some(
                json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "not JSON"}}),
            ),
        };
        if let Some(reply) = reply {
            writeln!(output, "{reply}").map_err(|e| crate::Error::Invalid(e.to_string()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Reminder;

    fn call(m: &dyn Memory, cfg: &Config, name: &str, args: Value) -> Value {
        let req = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}});
        handle(m, cfg, &req).unwrap()
    }

    #[test]
    fn exactly_the_four_typed_tools_are_offered() {
        let s = Store::open_in_memory().unwrap();
        let cfg = Config::default();
        let init = handle(
            &s,
            &cfg,
            &json!({"jsonrpc": "2.0", "id": 0, "method": "initialize"}),
        )
        .unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "kitsune");
        let list = handle(
            &s,
            &cfg,
            &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        )
        .unwrap();
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "search_notes",
                "get_active_reminders",
                "create_note",
                "dismiss_reminder"
            ]
        );
        assert!(handle(
            &s,
            &cfg,
            &json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
        )
        .is_none());
    }

    #[test]
    fn a_note_is_created_found_reminded_and_dismissed() {
        let s = Store::open_in_memory().unwrap();
        let cfg = Config::default();
        let made = call(
            &s,
            &cfg,
            "create_note",
            json!({"raw_transcript": "Penser à acheter du café"}),
        );
        let note = &made["result"]["structuredContent"]["note"];
        assert_eq!(note["raw_transcript"], "Penser à acheter du café");
        let id = note["id"].as_i64().unwrap();

        let found = call(
            &s,
            &cfg,
            "search_notes",
            json!({"query": "cafe", "mode": "fts", "limit": 3}),
        );
        assert_eq!(
            found["result"]["structuredContent"]["notes"][0]["id"], id,
            "accents folded"
        );
        let bad = call(
            &s,
            &cfg,
            "search_notes",
            json!({"query": "x", "mode": "fuzzy"}),
        );
        assert_eq!(bad["result"]["isError"], true);
        let sem = call(
            &s,
            &cfg,
            "search_notes",
            json!({"query": "x", "mode": "semantic"}),
        );
        assert_eq!(
            sem["result"]["isError"], true,
            "no embedding model on the bare store"
        );

        s.set_reminder(
            id,
            &Reminder::Time {
                due_date: "2026-11-01T09:00:00Z".into(),
            },
        )
        .unwrap();
        let active = call(
            &s,
            &cfg,
            "get_active_reminders",
            json!({"trigger_type": "time"}),
        );
        assert_eq!(active["result"]["structuredContent"]["notes"][0]["id"], id);
        let geo = call(
            &s,
            &cfg,
            "get_active_reminders",
            json!({"trigger_type": "geofence"}),
        );
        assert!(geo["result"]["structuredContent"]["notes"]
            .as_array()
            .unwrap()
            .is_empty());

        let done = call(&s, &cfg, "dismiss_reminder", json!({"note_id": id}));
        assert_eq!(done["result"]["structuredContent"]["dismissed"], true);
        let none = call(&s, &cfg, "dismiss_reminder", json!({"note_id": 999}));
        assert_eq!(none["result"]["structuredContent"]["dismissed"], false);
        assert!(
            call(&s, &cfg, "create_note", json!({"raw_transcript": "  "}))["result"]["isError"]
                == true
        );
    }

    #[test]
    fn scoping_and_auth_are_honoured() {
        let s = Store::open_in_memory().unwrap();
        let cfg = Config {
            token: "s3cret".into(),
            tools: vec!["search_notes".into()],
        };
        let list = handle(
            &s,
            &cfg,
            &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        )
        .unwrap();
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 1);
        let refused = call(
            &s,
            &cfg,
            "create_note",
            json!({"raw_transcript": "x", "_auth": "s3cret"}),
        );
        assert!(refused["error"]["message"]
            .as_str()
            .unwrap()
            .contains("does not offer"));
        let wrong = call(
            &s,
            &cfg,
            "search_notes",
            json!({"query": "x", "_auth": "nope"}),
        );
        assert_eq!(wrong["error"]["code"], -32001);
        let old_tool = call(&s, &Config::default(), "capture_note", json!({"text": "x"}));
        assert!(
            old_tool["error"]["message"]
                .as_str()
                .unwrap()
                .contains("does not offer"),
            "only the four"
        );
    }

    #[test]
    fn a_line_stream_is_served() {
        let s = Store::open_in_memory().unwrap();
        let input = format!(
            "{}\n{}\nnot json\n\n",
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "create_note", "arguments": {"raw_transcript": "note du flux"}}}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "search_notes", "arguments": {"query": "flux"}}}),
        );
        let mut out = Vec::new();
        serve_lines(
            &s,
            &Config::default(),
            std::io::Cursor::new(input),
            &mut out,
        )
        .unwrap();
        let lines: Vec<&str> = std::str::from_utf8(&out).unwrap().lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[1].contains("note du flux"));
        assert!(lines[2].contains("-32700"));
    }
}
