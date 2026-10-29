//! Kitsune's MCP face: the second brain offered to the agentic network (n8n, Home Assistant,
//! Claude, the local engine) over the Model Context Protocol, so any of them can keep a note,
//! search the memory, or set a reminder. This module is the protocol and the tools, over a
//! `Store`; the transport (stdio or http) is a thin loop around `handle`, wired where Kitsune
//! runs. Nothing is assumed: which tools are offered and the auth token are configuration, never
//! hard-coded.

use crate::search::{Filters, SearchOptions};
use crate::store::NewNote;
use crate::{Result, Store};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The protocol revision Kitsune speaks (the same the core's MCP client uses).
const PROTOCOL: &str = "2025-03-26";

/// Which tools a client may use, and the token it must present. Set from the GUI, never here.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Config {
    /// If set, a call must carry this token (an `_auth` argument) — the transport may also check it.
    #[serde(default)]
    pub token: String,
    /// The tools offered; empty means every known tool.
    #[serde(default)]
    pub tools: Vec<String>,
    /// The embedding model used for the meaning side of a search (empty: words only).
    #[serde(default)]
    pub embed_model: String,
}

impl Config {
    pub fn offers(&self, name: &str) -> bool {
        self.tools.is_empty() || self.tools.iter().any(|t| t == name)
    }
}

/// Every tool Kitsune can expose, with its schema. The caller filters by `Config::offers`.
fn all_tools() -> Vec<Value> {
    vec![
        json!({"name": "capture_note", "description": "Keep a note in the second brain.",
            "inputSchema": {"type": "object", "properties": {
                "text": {"type": "string", "description": "The note's text."},
                "tags": {"type": "array", "items": {"type": "string"}},
                "source": {"type": "string", "enum": ["voice", "text", "mcp"], "description": "Defaults to mcp."},
                "lang": {"type": "string"},
                "place": {"type": "string"},
                "lat": {"type": "number"}, "lon": {"type": "number"}},
                "required": ["text"]}}),
        json!({"name": "search_notes", "description": "Find notes by words and, when a query vector is given, by meaning (RRF).",
            "inputSchema": {"type": "object", "properties": {
                "query": {"type": "string"},
                "k": {"type": "integer", "description": "How many to return (default 10)."},
                "since": {"type": "number", "description": "Only notes at or after this epoch time."},
                "until": {"type": "number"},
                "tags": {"type": "array", "items": {"type": "string"}, "description": "The note must carry all of these."},
                "near": {"type": "object", "properties": {"lat": {"type": "number"}, "lon": {"type": "number"}, "radius_m": {"type": "number"}}}},
                "required": ["query"]}}),
        json!({"name": "get_note", "description": "Read one note by its id.",
            "inputSchema": {"type": "object", "properties": {"id": {"type": "integer"}}, "required": ["id"]}}),
        json!({"name": "summarize_range", "description": "The notes created in a time range, oldest first, for the caller to summarise.",
            "inputSchema": {"type": "object", "properties": {"start": {"type": "number"}, "end": {"type": "number"}}, "required": ["start", "end"]}}),
        json!({"name": "list_triggers", "description": "Every reminder set (time, geo or event).",
            "inputSchema": {"type": "object", "properties": {}}}),
        json!({"name": "create_trigger", "description": "Set a reminder: a time, a place entered or left, or a system event.",
            "inputSchema": {"type": "object", "properties": {
                "kind": {"type": "string", "enum": ["time", "geo", "event"]},
                "note_id": {"type": "integer"},
                "spec": {"type": "object", "description": "time: {at, every?}; geo: {lat, lon, radius_m, on: enter|exit}; event: {event}."},
                "next_at": {"type": "number", "description": "For a time trigger: when it first fires."}},
                "required": ["kind", "spec"]}}),
        json!({"name": "delete_trigger", "description": "Remove a reminder by its id.",
            "inputSchema": {"type": "object", "properties": {"id": {"type": "integer"}}, "required": ["id"]}}),
    ]
}

/// Handle one MCP request against a store; the JSON-RPC response (None for a notification).
pub fn handle(store: &Store, cfg: &Config, request: &Value) -> Option<Value> {
    let id = request.get("id").cloned();
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let params = request.get("params").cloned().unwrap_or(Value::Null);

    // a notification (no id) gets no reply
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
        "notifications/initialized" | "notifications/cancelled" => None,
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
            if !cfg.offers(name) {
                return reply(Err((-32601, format!("Kitsune does not offer {name}"))));
            }
            reply(Ok(match call_tool(store, cfg, name, &args) {
                Ok(text) => json!({"content": [{"type": "text", "text": text}]}),
                Err(e) => json!({"content": [{"type": "text", "text": e}], "isError": true}),
            }))
        }
        other => reply(Err((-32601, format!("unknown method {other}")))),
    }
}

fn call_tool(
    store: &Store,
    cfg: &Config,
    name: &str,
    a: &Value,
) -> std::result::Result<String, String> {
    let err = |e: crate::Error| e.to_string();
    let s = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let n = |k: &str| a.get(k).and_then(Value::as_f64);
    match name {
        "capture_note" => {
            let note = NewNote {
                text: s("text"),
                source: Some(s("source"))
                    .filter(|x| !x.is_empty())
                    .unwrap_or_else(|| "mcp".into()),
                lang: s("lang"),
                place: s("place"),
                tags: a
                    .get("tags")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(String::from)
                    .collect(),
                lat: n("lat"),
                lon: n("lon"),
                ..Default::default()
            };
            let id = store.capture_note(&note).map_err(err)?;
            Ok(format!("Kept note {id}."))
        }
        "search_notes" => {
            let filters = Filters {
                since: n("since"),
                until: n("until"),
                tags: a
                    .get("tags")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(String::from)
                    .collect(),
                near: a.get("near").and_then(|g| {
                    Some(crate::search::Geo {
                        lat: g.get("lat").and_then(Value::as_f64)?,
                        lon: g.get("lon").and_then(Value::as_f64)?,
                        radius_m: g.get("radius_m").and_then(Value::as_f64).unwrap_or(150.0),
                    })
                }),
            };
            let opts = SearchOptions {
                k: a.get("k")
                    .and_then(Value::as_u64)
                    .unwrap_or(10)
                    .clamp(1, 100) as usize,
                filters,
                ..Default::default()
            };
            // words always; meaning when the caller wired an embedding model and passes a vector
            let vector: Option<Vec<f32>> =
                a.get("query_vector").and_then(Value::as_array).map(|v| {
                    v.iter()
                        .filter_map(|x| x.as_f64().map(|x| x as f32))
                        .collect()
                });
            let hits = store
                .search(&s("query"), vector.as_deref(), &cfg.embed_model, &opts)
                .map_err(err)?;
            if hits.is_empty() {
                return Ok("No note found.".into());
            }
            let out: Vec<Value> = hits.iter().map(|h| json!({"id": h.note.id, "text": h.note.text, "created_at": h.note.created_at, "tags": h.note.tags, "place": h.note.place})).collect();
            Ok(serde_json::to_string_pretty(&out).expect("serializes"))
        }
        "get_note" => {
            let id = a
                .get("id")
                .and_then(Value::as_i64)
                .ok_or("get_note needs an id")?;
            match store.get_note(id).map_err(err)? {
                Some(note) => Ok(serde_json::to_string_pretty(&note).expect("serializes")),
                None => Err(format!("no note {id}")),
            }
        }
        "summarize_range" => {
            let (start, end) = (
                n("start").ok_or("needs start")?,
                n("end").ok_or("needs end")?,
            );
            let notes = store.notes_between(start, end).map_err(err)?;
            if notes.is_empty() {
                return Ok("No note in this range.".into());
            }
            Ok(notes
                .iter()
                .map(|n| format!("[{}] {}", n.id, n.text))
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "list_triggers" => Ok(
            serde_json::to_string_pretty(&store.list_triggers().map_err(err)?).expect("serializes"),
        ),
        "create_trigger" => {
            let kind = s("kind");
            if !["time", "geo", "event"].contains(&kind.as_str()) {
                return Err("kind is time, geo or event".into());
            }
            let spec = a
                .get("spec")
                .cloned()
                .filter(|v| v.is_object())
                .ok_or("spec is an object")?;
            let id = store
                .create_trigger(
                    &kind,
                    a.get("note_id").and_then(Value::as_i64),
                    &spec,
                    n("next_at"),
                )
                .map_err(err)?;
            Ok(format!("Set trigger {id}."))
        }
        "delete_trigger" => {
            let id = a
                .get("id")
                .and_then(Value::as_i64)
                .ok_or("delete_trigger needs an id")?;
            if store.delete_trigger(id).map_err(err)? {
                Ok(format!("Removed trigger {id}."))
            } else {
                Err(format!("no trigger {id}"))
            }
        }
        other => Err(format!("unknown tool {other}")),
    }
}

/// Drive a stream of MCP requests (one JSON object per line, as stdio transports send them) and
/// write each reply as a line. Returns when the input ends.
pub fn serve_lines<R: std::io::BufRead, W: std::io::Write>(
    store: &Store,
    cfg: &Config,
    input: R,
    mut output: W,
) -> Result<()> {
    for line in input.lines() {
        let line = line.map_err(|e| crate::Error::Invalid(e.to_string()))?;
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                writeln!(output, "{}", json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "not JSON"}})).ok();
                continue;
            }
        };
        if let Some(reply) = handle(store, cfg, &request) {
            writeln!(output, "{reply}").map_err(|e| crate::Error::Invalid(e.to_string()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(store: &Store, cfg: &Config, name: &str, args: Value) -> Value {
        let req = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}});
        let reply = handle(store, cfg, &req).unwrap();
        reply["result"].clone()
    }

    fn text(result: &Value) -> String {
        result["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string()
    }

    #[test]
    fn the_network_keeps_and_finds_a_note_and_sets_a_reminder() {
        let store = Store::open_in_memory().unwrap();
        let cfg = Config::default();

        // initialize and list the tools
        let init = handle(
            &store,
            &cfg,
            &json!({"jsonrpc": "2.0", "id": 0, "method": "initialize"}),
        )
        .unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "kitsune");
        let list = handle(
            &store,
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
        assert!(names.contains(&"capture_note") && names.contains(&"create_trigger"));

        // a notification gets no reply
        assert!(handle(
            &store,
            &cfg,
            &json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
        )
        .is_none());

        // capture, then find it by word
        let kept = text(&call(
            &store,
            &cfg,
            "capture_note",
            json!({"text": "Rappeler le dentiste", "tags": ["santé"]}),
        ));
        assert!(kept.starts_with("Kept note "));
        let found = text(&call(
            &store,
            &cfg,
            "search_notes",
            json!({"query": "dentiste"}),
        ));
        assert!(found.contains("Rappeler le dentiste") && found.contains("santé"));

        // set a reminder and see it listed
        let set = text(&call(
            &store,
            &cfg,
            "create_trigger",
            json!({"kind": "time", "note_id": 1, "spec": {"at": 1000.0}, "next_at": 1000.0}),
        ));
        assert!(set.starts_with("Set trigger "));
        let triggers = text(&call(&store, &cfg, "list_triggers", json!({})));
        assert!(triggers.contains("\"kind\": \"time\""));

        // a bad tool call is an error, not a panic
        let bad = call(&store, &cfg, "get_note", json!({"id": 999}));
        assert_eq!(bad["isError"], true);
    }

    #[test]
    fn scoping_and_auth_are_honoured() {
        let store = Store::open_in_memory().unwrap();
        let cfg = Config {
            token: "s3cret".into(),
            tools: vec!["search_notes".into()],
            embed_model: String::new(),
        };
        // only search_notes is offered
        let list = handle(
            &store,
            &cfg,
            &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        )
        .unwrap();
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 1);
        // a tool not in scope is refused even with the token
        let refused = handle(&store, &cfg, &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "capture_note", "arguments": {"text": "x", "_auth": "s3cret"}}})).unwrap();
        assert!(refused["error"]["message"]
            .as_str()
            .unwrap()
            .contains("does not offer"));
        // the wrong token is refused
        let wrong = call(
            &store,
            &cfg,
            "search_notes",
            json!({"query": "x", "_auth": "nope"}),
        );
        // when token is wrong, handle returns an error object, not a result
        let req = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "search_notes", "arguments": {"query": "x", "_auth": "nope"}}});
        let reply = handle(&store, &cfg, &req).unwrap();
        assert_eq!(reply["error"]["code"], -32001);
        let _ = wrong;
    }

    #[test]
    fn a_line_stream_is_served() {
        let store = Store::open_in_memory().unwrap();
        let cfg = Config::default();
        let input = format!(
            "{}\n{}\n\n",
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "capture_note", "arguments": {"text": "note du flux"}}}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "search_notes", "arguments": {"query": "flux"}}}),
        );
        let mut out = Vec::new();
        serve_lines(&store, &cfg, std::io::Cursor::new(input), &mut out).unwrap();
        let lines: Vec<&str> = std::str::from_utf8(&out).unwrap().lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains("note du flux"));
    }
}
