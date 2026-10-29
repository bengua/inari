//! An OpenAI-compatible model server, as local ones serve it (vLLM, SGLang, Ollama, llama.cpp,
//! LM Studio): its models, and a chat completion streamed piece by piece, with the model's
//! reasoning and its tool calls. Plain HTTP on the machine or the private network: the
//! frontier models are reached through their own agents (ACP), not from here.

use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// A model server as the settings name it.
#[derive(Clone, Debug)]
pub struct Server {
    /// Its base address, ending in `/v1` (http://127.0.0.1:8000/v1).
    pub url: String,
    pub key: String,
}

/// One tool call the model asked for, its arguments as the JSON text it wrote.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Call {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// A whole streamed reply.
#[derive(Clone, Debug, Default)]
pub struct Reply {
    pub text: String,
    pub reasoning: String,
    pub calls: Vec<Call>,
    pub finish: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

/// A piece of a reply as it arrives.
pub enum Piece<'a> {
    Text(&'a str),
    Reasoning(&'a str),
}

struct Target {
    host: String,
    authority: String,
    path: String,
}

fn target(url: &str) -> Result<Target, String> {
    let rest = url.strip_prefix("http://").ok_or(
        "the local engine speaks plain http to a server on the machine or the private network",
    )?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    if authority.is_empty() {
        return Err(format!("no host in {url:?}"));
    }
    let host = if authority.contains(':') {
        authority.to_string()
    } else {
        format!("{authority}:80")
    };
    Ok(Target {
        host,
        authority: authority.to_string(),
        path: path.trim_end_matches('/').to_string(),
    })
}

/// A response's body, read as it comes: by length, by chunks, or to the end.
struct Body {
    stream: TcpStream,
    buf: Vec<u8>,
    chunked: bool,
    left: Option<usize>,
    chunk_left: usize,
    done: bool,
}

impl Body {
    async fn fill(&mut self) -> Result<bool, String> {
        let mut chunk = [0u8; 16384];
        let n = self
            .stream
            .read(&mut chunk)
            .await
            .map_err(|e| e.to_string())?;
        self.buf.extend_from_slice(&chunk[..n]);
        Ok(n > 0)
    }

    async fn line(&mut self) -> Result<Option<String>, String> {
        loop {
            if let Some(i) = self.buf.windows(2).position(|w| w == b"\r\n") {
                let line = String::from_utf8_lossy(&self.buf[..i]).to_string();
                self.buf.drain(..i + 2);
                return Ok(Some(line));
            }
            if !self.fill().await? {
                return Ok(None);
            }
        }
    }

    /// The next bytes of the body; None at its end.
    async fn next(&mut self) -> Result<Option<Vec<u8>>, String> {
        if self.done {
            return Ok(None);
        }
        if self.chunked {
            if self.chunk_left == 0 {
                let size = loop {
                    match self.line().await? {
                        None => return Ok(None),
                        Some(l) if l.trim().is_empty() => continue,
                        Some(l) => {
                            break usize::from_str_radix(
                                l.split(';').next().unwrap_or("").trim(),
                                16,
                            )
                            .map_err(|_| format!("a bad chunk size: {l:?}"))?
                        }
                    }
                };
                if size == 0 {
                    self.done = true;
                    return Ok(None);
                }
                self.chunk_left = size;
            }
            while self.buf.is_empty() {
                if !self.fill().await? {
                    return Ok(None);
                }
            }
            let n = self.chunk_left.min(self.buf.len());
            let out: Vec<u8> = self.buf.drain(..n).collect();
            self.chunk_left -= n;
            return Ok(Some(out));
        }
        if let Some(0) = self.left {
            self.done = true;
            return Ok(None);
        }
        if self.buf.is_empty() && !self.fill().await? {
            self.done = true;
            return Ok(None);
        }
        let n = self.left.unwrap_or(usize::MAX).min(self.buf.len());
        let out: Vec<u8> = self.buf.drain(..n).collect();
        if let Some(l) = self.left.as_mut() {
            *l -= n;
        }
        Ok(Some(out))
    }

    async fn all(&mut self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        while let Some(b) = self.next().await? {
            out.extend(b);
            if out.len() > 32 * 1024 * 1024 {
                return Err("the answer is too large".into());
            }
        }
        Ok(out)
    }
}

/// Send a request; the status and the body to read.
async fn send(
    server: &Server,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> Result<(u16, Body), String> {
    let t = target(&server.url)?;
    let mut stream = tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(&t.host))
        .await
        .map_err(|_| format!("{} does not answer (10 s)", server.url))?
        .map_err(|e| format!("{} does not answer ({e})", server.url))?;
    let payload = body.map(|b| serde_json::to_vec(b).expect("serializes"));
    let mut head = format!(
        "{method} {}{path} HTTP/1.1\r\nHost: {}\r\nAccept: */*\r\nConnection: close\r\n",
        t.path, t.authority
    );
    if !server.key.is_empty() {
        head.push_str(&format!("Authorization: Bearer {}\r\n", server.key));
    }
    if let Some(p) = &payload {
        head.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            p.len()
        ));
    }
    head.push_str("\r\n");
    stream
        .write_all(head.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    if let Some(p) = payload {
        stream.write_all(&p).await.map_err(|e| e.to_string())?;
    }
    let mut body = Body {
        stream,
        buf: Vec::new(),
        chunked: false,
        left: None,
        chunk_left: 0,
        done: false,
    };
    let status_line = body
        .line()
        .await?
        .ok_or_else(|| format!("{} closed the connection", server.url))?;
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("not an HTTP answer: {status_line:?}"))?;
    while let Some(line) = body.line().await? {
        if line.is_empty() {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            body.left = v.trim().parse().ok();
        } else if lower.starts_with("transfer-encoding:") && lower.contains("chunked") {
            body.chunked = true;
        }
    }
    if body.chunked {
        body.left = None;
    }
    Ok((status, body))
}

fn error_of(status: u16, raw: &[u8]) -> String {
    let v: Value = serde_json::from_slice(raw).unwrap_or(Value::Null);
    let said = v
        .pointer("/error/message")
        .or_else(|| v.get("error"))
        .or_else(|| v.get("detail"))
        .or_else(|| v.get("message"))
        .map(|e| {
            e.as_str()
                .map(String::from)
                .unwrap_or_else(|| e.to_string())
        })
        .unwrap_or_else(|| String::from_utf8_lossy(&raw[..raw.len().min(300)]).to_string());
    format!("HTTP {status}: {said}")
}

/// The models a server offers (`GET /models`).
pub async fn models(server: &Server) -> Result<Vec<String>, String> {
    #[derive(Deserialize)]
    struct Listed {
        #[serde(default)]
        data: Vec<Model>,
    }
    #[derive(Deserialize)]
    struct Model {
        id: String,
    }
    let (status, mut body) = tokio::time::timeout(
        Duration::from_secs(20),
        send(server, "GET", "/models", None),
    )
    .await
    .map_err(|_| format!("{} did not list its models (20 s)", server.url))??;
    let raw = tokio::time::timeout(Duration::from_secs(20), body.all())
        .await
        .map_err(|_| "the models list did not arrive (20 s)".to_string())??;
    if status >= 400 {
        return Err(error_of(status, &raw));
    }
    let listed: Listed =
        serde_json::from_slice(&raw).map_err(|e| format!("the models list: {e}"))?;
    Ok(listed.data.into_iter().map(|m| m.id).collect())
}

/// A chat completion, streamed: each piece of text or reasoning goes to `each` as it comes;
/// the whole reply is returned. `quiet` is how long the server may stay silent.
pub async fn chat(
    server: &Server,
    request: &Value,
    quiet: Duration,
    mut each: impl FnMut(Piece<'_>),
) -> Result<Reply, String> {
    let mut request = request.clone();
    request["stream"] = json!(true);
    request["stream_options"] = json!({"include_usage": true});
    let (status, mut body) = tokio::time::timeout(
        Duration::from_secs(60),
        send(server, "POST", "/chat/completions", Some(&request)),
    )
    .await
    .map_err(|_| format!("{} did not start answering (60 s)", server.url))??;
    if status >= 400 {
        let raw = tokio::time::timeout(Duration::from_secs(20), body.all())
            .await
            .unwrap_or(Ok(Vec::new()))?;
        return Err(error_of(status, &raw));
    }
    let mut reply = Reply::default();
    let mut pending = String::new();
    let mut raw_text = Vec::new();
    loop {
        let got = tokio::time::timeout(quiet, body.next())
            .await
            .map_err(|_| format!("the model was silent for {} s", quiet.as_secs()))??;
        let Some(bytes) = got else { break };
        raw_text.extend(bytes);
        // decode only whole UTF-8 sequences: a character may be split between reads
        let upto = match std::str::from_utf8(&raw_text) {
            Ok(_) => raw_text.len(),
            Err(e) => e.valid_up_to(),
        };
        pending.push_str(&String::from_utf8_lossy(&raw_text[..upto]));
        raw_text.drain(..upto);
        while let Some(i) = pending.find('\n') {
            let line = pending[..i].trim_end_matches('\r').to_string();
            pending.drain(..=i);
            let Some(data) = line.strip_prefix("data:") else {
                continue;
            };
            let data = data.trim();
            if data == "[DONE]" {
                return Ok(reply);
            }
            let Ok(v) = serde_json::from_str::<Value>(data) else {
                continue;
            };
            if let Some(e) = v.get("error") {
                return Err(e
                    .get("message")
                    .and_then(Value::as_str)
                    .map(String::from)
                    .unwrap_or_else(|| e.to_string()));
            }
            take(&v, &mut reply, &mut each);
        }
    }
    Ok(reply)
}

/// One streamed event into the reply.
fn take(v: &Value, reply: &mut Reply, each: &mut impl FnMut(Piece<'_>)) {
    if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
        reply.prompt_tokens = u.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0);
        reply.completion_tokens = u
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
    }
    let Some(choice) = v.pointer("/choices/0") else {
        return;
    };
    if let Some(f) = choice.get("finish_reason").and_then(Value::as_str) {
        reply.finish = f.to_string();
    }
    let Some(delta) = choice.get("delta") else {
        return;
    };
    for key in ["reasoning_content", "reasoning", "thinking"] {
        if let Some(r) = delta
            .get(key)
            .and_then(Value::as_str)
            .filter(|r| !r.is_empty())
        {
            reply.reasoning.push_str(r);
            each(Piece::Reasoning(r));
            break;
        }
    }
    if let Some(c) = delta
        .get("content")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty())
    {
        reply.text.push_str(c);
        each(Piece::Text(c));
    }
    for tc in delta
        .get("tool_calls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let index = tc
            .get("index")
            .and_then(Value::as_u64)
            .map(|i| i as usize)
            .unwrap_or(reply.calls.len().saturating_sub(1));
        while reply.calls.len() <= index {
            reply.calls.push(Call::default());
        }
        let call = &mut reply.calls[index];
        if let Some(id) = tc
            .get("id")
            .and_then(Value::as_str)
            .filter(|i| !i.is_empty())
        {
            call.id = id.to_string();
        }
        if let Some(f) = tc.get("function") {
            if let Some(n) = f.get("name").and_then(Value::as_str) {
                call.name.push_str(n);
            }
            if let Some(a) = f.get("arguments").and_then(Value::as_str) {
                call.arguments.push_str(a);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_streamed_reply_gathers_text_reasoning_and_tool_calls() {
        let mut reply = Reply::default();
        let mut seen = Vec::new();
        let events = [
            json!({"choices": [{"delta": {"reasoning_content": "Look "}}]}),
            json!({"choices": [{"delta": {"content": "Writing "}}]}),
            json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "c1", "function": {"name": "write_file", "arguments": "{\"pa"}}]}}]}),
            json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": "th\": \"a\"}"}}]}}]}),
            json!({"choices": [{"delta": {}, "finish_reason": "tool_calls"}]}),
            json!({"choices": [], "usage": {"prompt_tokens": 12, "completion_tokens": 5}}),
        ];
        for e in &events {
            take(e, &mut reply, &mut |p| {
                seen.push(match p {
                    Piece::Text(t) => format!("t:{t}"),
                    Piece::Reasoning(r) => format!("r:{r}"),
                })
            });
        }
        assert_eq!(seen, ["r:Look ", "t:Writing "]);
        assert_eq!(
            reply.calls,
            [Call {
                id: "c1".into(),
                name: "write_file".into(),
                arguments: "{\"path\": \"a\"}".into()
            }]
        );
        assert_eq!(
            (reply.finish.as_str(), reply.prompt_tokens),
            ("tool_calls", 12)
        );
    }

    #[test]
    fn only_plain_http_is_spoken() {
        assert!(target("https://api.example.com/v1").is_err());
        let t = target("http://127.0.0.1:8000/v1").unwrap();
        assert_eq!(
            (t.host.as_str(), t.path.as_str()),
            ("127.0.0.1:8000", "/v1")
        );
        assert_eq!(target("http://gpu-box/v1/").unwrap().host, "gpu-box:80");
    }
}
