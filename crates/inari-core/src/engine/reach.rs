//! The agents' reach beyond the thread: the web (searched through the person's own SearXNG, a
//! page read through curl) and other threads (a job handed to a coder harness in a thread of its
//! own, a "room" the person follows in Inari).

use super::LocalEngine;
use crate::agent::Turn;
use serde_json::{json, Value};
use std::time::Duration;

pub fn definitions() -> Vec<Value> {
    vec![
        json!({"type": "function", "function": {"name": "web_search",
            "description": "Search the web through the person's own SearXNG. Returns titles, addresses and snippets; read a page with web_fetch.",
            "parameters": {"type": "object", "properties": {
                "query": {"type": "string"},
                "limit": {"type": "number", "description": "How many results (default 8, at most 20)"}},
                "required": ["query"]}}}),
        json!({"type": "function", "function": {"name": "web_fetch",
            "description": "Read a public web page (http or https) as plain text. Addresses on the machine or the private network are refused.",
            "parameters": {"type": "object", "properties": {
                "url": {"type": "string"}},
                "required": ["url"]}}}),
        json!({"type": "function", "function": {"name": "delegate",
            "description": "Hand a job to a new Inari thread on another harness (a coder: the local engine, Claude Code, Codex…), which the person follows in Inari. Give it everything it needs: the goal, the plan, the acceptance criteria. Returns the new thread's id.",
            "parameters": {"type": "object", "properties": {
                "harness": {"type": "string", "description": "The harness of the new thread (empty: the core's default)"},
                "title": {"type": "string"},
                "prompt": {"type": "string", "description": "The job, whole: the new thread starts from it alone"}},
                "required": ["title", "prompt"]}}}),
    ]
}

/// Readable text out of an HTML page: no scripts, styles or tags, the common entities decoded,
/// the spaces folded. Plain code over a page, never over a model's answer.
pub fn html_text(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len() / 3);
    let mut i = 0;
    let bytes = html.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'<' {
            // skip a whole script or style element, else just the tag
            let skip_to = ["script", "style", "noscript"]
                .iter()
                .find(|t| lower[i + 1..].starts_with(*t))
                .and_then(|t| lower[i..].find(&format!("</{t}")).map(|e| i + e))
                .unwrap_or(i);
            let end = lower[skip_to..]
                .find('>')
                .map(|e| skip_to + e + 1)
                .unwrap_or(bytes.len());
            let tag = &lower[i..end.min(lower.len())];
            if [
                "<p", "<br", "<div", "<li", "<h1", "<h2", "<h3", "<tr", "</p", "</div",
            ]
            .iter()
            .any(|b| tag.starts_with(b))
            {
                out.push('\n');
            }
            i = end;
            continue;
        }
        let next = html[i..].find('<').map(|e| i + e).unwrap_or(html.len());
        out.push_str(&html[i..next]);
        i = next;
    }
    let decoded = out
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'");
    let mut text = String::new();
    for line in decoded.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if !line.is_empty() {
            text.push_str(&line);
            text.push('\n');
        }
    }
    text
}

/// A query value, percent-encoded.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// Is this address on the machine or a private network (where web_fetch must not go)?
fn private_host(url: &str) -> bool {
    use std::net::ToSocketAddrs;
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority
        .rsplit_once('@')
        .map(|(_, h)| h)
        .unwrap_or(authority);
    let with_port = if host.starts_with('[') || host.contains(':') {
        host.to_string()
    } else {
        format!("{host}:80")
    };
    match with_port.to_socket_addrs() {
        Ok(addrs) => addrs.into_iter().any(|a| kitsune::net::is_private(a.ip())),
        Err(_) => true, // unresolvable: not fetched
    }
}

impl LocalEngine {
    pub(crate) async fn web_search(&self, args: &Value) -> Result<String, String> {
        let base = self.settings().search_url;
        if base.trim().is_empty() {
            return Err("no search engine set up: Settings → Local engine → Web search (your SearXNG, e.g. http://127.0.0.1:8888)".into());
        }
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if query.is_empty() {
            return Err("web_search needs a query".into());
        }
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(8)
            .clamp(1, 20) as usize;
        let url = format!(
            "{}/search?q={}&format=json",
            base.trim_end_matches('/'),
            encode(&query)
        );
        let (status, raw) = crate::usage::http("GET", &url, None, Duration::from_secs(20)).await?;
        if status != 200 {
            return Err(format!(
                "the search engine answered HTTP {status} (is the json format enabled in SearXNG?)"
            ));
        }
        let v: Value =
            serde_json::from_slice(&raw).map_err(|e| format!("the search engine's answer: {e}"))?;
        let results: Vec<Value> = v["results"]
            .as_array()
            .into_iter()
            .flatten()
            .take(limit)
            .map(|r| json!({"title": r["title"], "url": r["url"], "snippet": r["content"]}))
            .collect();
        Ok(json!({"results": results}).to_string())
    }

    pub(crate) async fn web_fetch(&self, args: &Value) -> Result<String, String> {
        let url = args
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err("web_fetch reads an http or https address".into());
        }
        let probe = url.clone();
        if tokio::task::spawn_blocking(move || private_host(&probe))
            .await
            .unwrap_or(true)
        {
            return Err(format!("{url} is on the machine or the private network: web_fetch reads only the public web"));
        }
        let out = tokio::time::timeout(
            Duration::from_secs(30),
            tokio::process::Command::new("curl")
                .args([
                    "-sSL",
                    "--max-time",
                    "20",
                    "--max-redirs",
                    "3",
                    "--max-filesize",
                    "3000000",
                    "--proto",
                    "=http,https",
                    "-A",
                    "Inari",
                    &url,
                ])
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| format!("{url} did not answer within 30 s"))?
        .map_err(|e| format!("curl cannot start ({e}): is it installed on this core?"))?;
        if !out.status.success() {
            return Err(format!(
                "{url}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        let page = String::from_utf8_lossy(&out.stdout);
        let mut text = if page.contains('<') {
            html_text(&page)
        } else {
            page.to_string()
        };
        if text.len() > 20_000 {
            let cut = (0..=20_000)
                .rev()
                .find(|i| text.is_char_boundary(*i))
                .unwrap_or(0);
            text.truncate(cut);
            text.push_str("\n[…the page goes on]");
        }
        Ok(text)
    }

    pub(crate) async fn delegate_tool(&self, t: &Turn, args: &Value) -> Result<String, String> {
        let s = |k: &str| {
            args.get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string()
        };
        let (harness, title, prompt) = (s("harness"), s("title"), s("prompt"));
        if title.is_empty() || prompt.is_empty() {
            return Err("delegate needs a title and the job (prompt)".into());
        }
        let known: Vec<String> = t
            .runner
            .harness_list()
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        if !harness.is_empty() && !known.contains(&harness) {
            return Err(format!(
                "no harness {harness:?} on this core; there are: {}",
                known.join(", ")
            ));
        }
        let shown = if harness.is_empty() {
            t.runner.default_harness.clone()
        } else {
            harness.clone()
        };
        if self.settings().delegate_asks
            && !t
                .runner
                .approve(&t.agent, &format!("open a thread on {shown}: {title}"))
                .await
        {
            return Err("the person did not allow opening this thread".into());
        }
        let id = t.runner.delegate(&t.agent, &harness, &title, &prompt)?;
        Ok(json!({"thread": id, "harness": shown, "note": "the thread works on its own; the person follows it in Inari"}).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_reads_as_text() {
        let html = "<html><head><style>p{x:y}</style><script>var a='<p>';</script></head><body><h1>Titre</h1><p>Un &amp; deux&nbsp;trois</p><ul><li>un</li><li>deux</li></ul></body></html>";
        assert_eq!(html_text(html), "Titre\nUn & deux trois\nun\ndeux\n");
    }

    #[test]
    fn the_private_network_is_not_fetched() {
        assert!(private_host("http://127.0.0.1:8080/admin"));
        assert!(private_host("http://192.168.1.1/"));
        assert!(private_host("https://10.0.0.2"));
        assert!(!private_host("https://1.1.1.1/"));
        assert_eq!(encode("café crème"), "caf%C3%A9%20cr%C3%A8me");
    }
}
