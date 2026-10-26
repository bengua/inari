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
