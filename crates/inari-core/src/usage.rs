//! Quotas and usage of the model providers, in one shape, as the Python reference
//! (`inari_core.usage`): sources are adapters, the gateway polls them and puts the
//! result in its snapshot. A local model has no quota: it is listed as unlimited.

use serde_json::{json, Map, Value};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// A plain HTTP/1.1 exchange (no TLS: usage services are reached on loopback or
/// through a tunnel). Returns the status and the body, chunked encoding decoded.
pub async fn http(
    method: &str,
    url: &str,
    body: Option<&[u8]>,
    timeout: Duration,
) -> Result<(u16, Vec<u8>), String> {
    let rest = url.strip_prefix("http://").ok_or("only http:// URLs")?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let addr = if authority.contains(':') {
        authority.to_string()
    } else {
        format!("{authority}:80")
    };
    let exchange = async {
        let mut stream = TcpStream::connect(&addr).await.map_err(|e| e.to_string())?;
        let body = body.unwrap_or_default();
        let head = format!(
            "{method} {path} HTTP/1.1\r\nHost: {authority}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream
            .write_all(head.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        stream.write_all(body).await.map_err(|e| e.to_string())?;
        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .await
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(raw)
    };
    let raw = tokio::time::timeout(timeout, exchange)
        .await
        .map_err(|_| "timed out".to_string())??;
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("no header end")?;
    let head = String::from_utf8_lossy(&raw[..split]).to_lowercase();
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or("no status")?;
    let mut payload = raw[split + 4..].to_vec();
    if head.contains("transfer-encoding: chunked") {
        payload = dechunk(&payload)?;
    }
    Ok((status, payload))
}

fn dechunk(mut data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let line_end = data
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or("bad chunk")?;
        let size_text = String::from_utf8_lossy(&data[..line_end]);
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| "bad chunk size")?;
        data = &data[line_end + 2..];
        if size == 0 {
            return Ok(out);
        }
        if data.len() < size + 2 {
            return Err("truncated chunk".into());
        }
        out.extend_from_slice(&data[..size]);
        data = &data[size + 2..];
    }
}

async fn http_json(url: &str, body: Option<&Value>, timeout: Duration) -> Result<Value, String> {
    let bytes = body.map(|b| serde_json::to_vec(b).expect("serializes"));
    let (status, raw) = http(
        if body.is_some() { "POST" } else { "GET" },
        url,
        bytes.as_deref(),
        timeout,
    )
    .await?;
    if !(200..300).contains(&status) {
        return Err(format!("HTTP {status}"));
    }
    serde_json::from_slice(&raw).map_err(|e| e.to_string())
}

pub enum Source {
    /// Providers described in configuration: a local model, or a quota kept by hand.
    Static(Vec<Value>),
    /// The provider-usage service of a Cloudroom-compatible server.
    Cloudroom {
        base: String,
        providers: Vec<String>,
    },
    /// Claude's own usage endpoint, with the Claude Code login of this machine.
    Claude { credentials: std::path::PathBuf },
}

const CLAUDE_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

/// The usage entry for Claude from the endpoint's reply (`five_hour`, `seven_day`,
/// `extra_usage`); `plan` is the subscription type from the login.
pub fn claude_entry(reply: &Value, plan: &str) -> Value {
    let mut windows = Vec::new();
    for (key, label) in [
        ("five_hour", "Five-hour limit"),
        ("seven_day", "Weekly limit"),
    ] {
        if let Some(used) = reply
            .pointer(&format!("/{key}/utilization"))
            .and_then(Value::as_f64)
        {
            let resets = reply
                .pointer(&format!("/{key}/resets_at"))
                .and_then(Value::as_str)
                .unwrap_or("");
            windows.push(json!({"label": label, "used_percent": used, "resets_at": resets}));
        }
    }
    let mut plan_label = match plan {
        "" => String::new(),
        p => p[..1].to_uppercase() + &p[1..],
    };
    // Extra usage takes over at the limit, paid per token: shown with what it cost so far.
    let extra = reply
        .get("extra_usage")
        .filter(|e| e.get("is_enabled").and_then(Value::as_bool) == Some(true));
    if let Some(e) = extra {
        let places = e
            .get("decimal_places")
            .and_then(Value::as_i64)
            .unwrap_or(2)
            .clamp(0, 6) as i32;
        let used = e
            .get("used_credits")
            .and_then(Value::as_f64)
            .map(|c| c / 10f64.powi(places));
        let currency = e.get("currency").and_then(Value::as_str).unwrap_or("");
        let note = match used {
            Some(u) => format!("extra usage {u:.2} {currency}")
                .trim_end()
                .to_string(),
            None => "extra usage".to_string(),
        };
        plan_label = if plan_label.is_empty() {
            note
        } else {
            format!("{plan_label} · {note}")
        };
        if let (Some(used), Some(limit)) = (
            used,
            e.get("monthly_limit")
                .and_then(Value::as_f64)
                .map(|l| l / 10f64.powi(places))
                .filter(|l| *l > 0.0),
        ) {
            windows.push(json!({"label": "Extra usage this month", "used_percent": (used / limit * 100.0).min(100.0), "resets_at": ""}));
        }
    }
    json!({"provider_id": "claude", "name": "Claude Code", "status": "ok", "plan": plan_label, "tint": "", "logo": "", "windows": windows})
}

async fn claude_usage(credentials: &std::path::Path) -> Result<Value, String> {
    let unavailable = |status: &str| json!({"provider_id": "claude", "name": "Claude Code", "status": status, "plan": "", "tint": "", "logo": "", "windows": []});
    let Ok(raw) = std::fs::read(credentials) else {
        return Ok(unavailable("unauthenticated"));
    };
    let login: Value = serde_json::from_slice(&raw).map_err(|e| e.to_string())?;
    let oauth = login.get("claudeAiOauth").cloned().unwrap_or(Value::Null);
    let Some(token) = oauth.get("accessToken").and_then(Value::as_str) else {
        return Ok(unavailable("unauthenticated"));
    };
    let expires_ms = oauth
        .get("expiresAt")
        .and_then(Value::as_f64)
        .unwrap_or(f64::MAX);
    if crate::approvals::now() * 1000.0 >= expires_ms {
        return Ok(unavailable("expired"));
    }
    // curl reads the headers from its input: the token never appears in a command line.
    let mut child = tokio::process::Command::new("curl")
        .args([
            "-sS",
            "-m",
            "15",
            "-H",
            "@-",
            "-w",
            "\n%{http_code}",
            CLAUDE_USAGE_URL,
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("curl: {e}"))?;
    let headers = format!(
        "Authorization: Bearer {token}\nAccept: application/json\nanthropic-beta: oauth-2025-04-20\nUser-Agent: claude-code/2.1.0\n"
    );
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(headers.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
    }
    let out = tokio::time::timeout(Duration::from_secs(20), child.wait_with_output())
        .await
        .map_err(|_| "timed out".to_string())?
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (body, code) = text.rsplit_once('\n').unwrap_or(("", &text));
    let plan = oauth
        .get("subscriptionType")
        .and_then(Value::as_str)
        .unwrap_or("");
    match code.trim() {
        "200" => Ok(claude_entry(
            &serde_json::from_str(body).map_err(|e| e.to_string())?,
            plan,
        )),
        "401" => Ok(unavailable("expired")),
        other => Err(format!("Claude usage: HTTP {other}")),
    }
}

fn family(provider: &str) -> &str {
    match provider {
        "claude-code" => "claude",
        other => other,
    }
}

impl Source {
    pub fn from_config(spec: &Value) -> Option<Self> {
        match spec.get("kind").and_then(Value::as_str)? {
            "static" => Some(Self::Static(
                spec.get("items")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            )),
            "claude" => Some(Self::Claude {
                credentials: spec
                    .get("credentials")
                    .and_then(Value::as_str)
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| {
                        std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
                            .join(".claude/.credentials.json")
                    }),
            }),
            "cloudroom" => Some(Self::Cloudroom {
                base: spec.get("url")?.as_str()?.trim_end_matches('/').to_string(),
                providers: spec
                    .get("providers")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|p| p.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
            }),
            _ => None,
        }
    }

    /// The usage entries, and the logo URLs found (family -> absolute URL).
    pub async fn fetch(&self) -> Result<(Vec<Value>, Map<String, Value>), String> {
        match self {
            Self::Claude { credentials } => Ok((vec![claude_usage(credentials).await?], Map::new())),
            Self::Static(items) => Ok((
                items
                    .iter()
                    .map(|i| {
                        let id = i.get("provider_id").and_then(Value::as_str).unwrap_or("");
                        json!({"provider_id": id, "name": i.get("name").and_then(Value::as_str).unwrap_or(id),
                               "status": i.get("status").and_then(Value::as_str).unwrap_or("unlimited"),
                               "plan": i.get("plan").and_then(Value::as_str).unwrap_or(""),
                               "tint": i.get("tint").and_then(Value::as_str).unwrap_or(""), "logo": "",
                               "windows": i.get("windows").cloned().unwrap_or(json!([]))})
                    })
                    .collect(),
                Map::new(),
            )),
            Self::Cloudroom { base, providers } => {
                let wait = Duration::from_secs(40);
                let hosts = http_json(&format!("{base}/api/v1/hosts"), None, wait).await?;
                let ids: Vec<Value> = hosts
                    .as_array()
                    .map(|a| a.iter().filter(|h| h.get("status").and_then(Value::as_str) == Some("connected")).filter_map(|h| h.get("id").cloned()).collect())
                    .unwrap_or_default();
                let mut found: Vec<Value> = Vec::new();
                let mut logos = Map::new();
                for provider in providers {
                    let body = json!({"force": false, "machineIds": ids, "providerId": provider, "maxAgeMs": 600000});
                    let data = http_json(&format!("{base}/api/v1/plugins/provider-usage/rpc/getUsage"), Some(&body), wait).await?;
                    let machines = data.pointer("/result/machines").and_then(Value::as_array).cloned().unwrap_or_default();
                    for entry in machines.iter().flat_map(|m| m.get("providers").and_then(Value::as_array).cloned().unwrap_or_default()) {
                        if entry.get("providerId").and_then(Value::as_str) != Some(provider) || found.iter().any(|f| f["provider_id"] == family(provider)) {
                            continue;
                        }
                        let fam = family(provider);
                        if let Some(logo) = entry.get("logoUrl").and_then(Value::as_str) {
                            logos.insert(fam.into(), json!(format!("{base}{logo}")));
                        }
                        let usage = entry.get("usage").cloned().unwrap_or(json!({}));
                        let windows: Vec<Value> = usage
                            .get("windows")
                            .and_then(Value::as_array)
                            .map(|a| {
                                a.iter()
                                    .filter_map(|w| {
                                        Some(json!({"label": w.get("label")?, "used_percent": w.get("usedPercent")?.as_f64()?,
                                                    "resets_at": w.get("resetsAt").and_then(Value::as_str).unwrap_or("")}))
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        found.push(json!({
                            "provider_id": fam,
                            "name": entry.get("displayName").and_then(Value::as_str).unwrap_or(provider),
                            "status": usage.get("status").and_then(Value::as_str).unwrap_or("error"),
                            "plan": usage.get("planLabel").and_then(Value::as_str).unwrap_or(""),
                            "tint": entry.pointer("/strings/iconTint/dark").and_then(Value::as_str).unwrap_or(""),
                            "logo": if logos.contains_key(fam) { format!("/api/logo/{fam}") } else { String::new() },
                            "windows": windows,
                        }));
                    }
                }
                Ok((found, logos))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_usage_becomes_windows() {
        let reply = json!({"five_hour": {"utilization": 6.0, "resets_at": "2026-10-27T15:10:00Z"},
                           "seven_day": {"utilization": 100.0, "resets_at": "2026-10-30T21:00:00Z"},
                           "extra_usage": {"is_enabled": true, "monthly_limit": 5000.0, "used_credits": 500.0, "currency": "EUR", "decimal_places": 2}});
        let e = claude_entry(&reply, "pro");
        assert_eq!(e["plan"], "Pro · extra usage 5.00 EUR");
        let used: Vec<f64> = e["windows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["used_percent"].as_f64().unwrap())
            .collect();
        assert_eq!(used, [6.0, 100.0, 10.0]);
        assert_eq!(claude_entry(&json!({}), "")["windows"], json!([]));
    }

    #[tokio::test]
    async fn no_login_means_unauthenticated() {
        let s = Source::from_config(
            &json!({"kind": "claude", "credentials": "/nonexistent/creds.json"}),
        )
        .unwrap();
        let (items, _) = s.fetch().await.unwrap();
        assert_eq!(items[0]["status"], "unauthenticated");
    }

    #[test]
    fn chunks_are_joined() {
        assert_eq!(
            dechunk(b"4\r\nWiki\r\n5;x=y\r\npedia\r\n0\r\n\r\n").unwrap(),
            b"Wikipedia"
        );
        assert!(dechunk(b"9\r\nshort\r\n").is_err());
    }

    #[tokio::test]
    async fn a_static_source_lists_its_items() {
        let s = Source::from_config(
            &json!({"kind": "static", "items": [{"provider_id": "local", "name": "Local"}]}),
        )
        .unwrap();
        let (items, _) = s.fetch().await.unwrap();
        assert_eq!(items[0]["status"], "unlimited");
        assert_eq!(items[0]["name"], "Local");
    }
}
