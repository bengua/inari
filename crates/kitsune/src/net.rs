//! A small blocking HTTP client for the model servers Kitsune uses on the machine or the private
//! network (a Whisper transcription server, a text-to-speech server): plain HTTP, as Inari's
//! local engine speaks it, so no async runtime and no TLS stack are pulled into this crate. The
//! frontier services are not reached from here — these are local models the person runs.

use crate::{Error, Result};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// A reply read whole.
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Response {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

struct Target {
    host: String,
    authority: String,
    path: String,
}

fn target(url: &str) -> Result<Target> {
    let rest = url.strip_prefix("http://").ok_or_else(|| {
        Error::Invalid(
            "Kitsune reaches a local model server over plain http (http://host:port/…)".into(),
        )
    })?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    if authority.is_empty() {
        return Err(Error::Invalid(format!("no host in {url:?}")));
    }
    let host = if authority.contains(':') {
        authority.to_string()
    } else {
        format!("{authority}:80")
    };
    Ok(Target {
        host,
        authority: authority.to_string(),
        path: path.to_string(),
    })
}

fn send(
    url: &str,
    content_type: &str,
    body: &[u8],
    extra: &[(String, String)],
    wait: Duration,
) -> Result<Response> {
    let t = target(url)?;
    let mut stream = TcpStream::connect(&t.host)
        .map_err(|e| Error::Invalid(format!("{url} does not answer ({e})")))?;
    stream.set_read_timeout(Some(wait)).ok();
    stream.set_write_timeout(Some(wait)).ok();
    let mut head = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n",
        t.path,
        t.authority,
        body.len()
    );
    for (k, v) in extra {
        if k.contains([':', '\r', '\n']) || v.contains(['\r', '\n']) {
            return Err(Error::Invalid(format!("a bad header: {k}")));
        }
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    stream
        .write_all(head.as_bytes())
        .map_err(|e| Error::Invalid(e.to_string()))?;
    stream
        .write_all(body)
        .map_err(|e| Error::Invalid(e.to_string()))?;
    stream.flush().ok();

    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|e| Error::Invalid(format!("{url}: {e}")))?;
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| Error::Invalid("not an HTTP answer".into()))?;
    let head = String::from_utf8_lossy(&raw[..split]);
    let mut lines = head.lines();
    let status: u16 = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| Error::Invalid("no HTTP status".into()))?;
    let chunked = lines.any(|l| {
        let l = l.to_ascii_lowercase();
        l.starts_with("transfer-encoding:") && l.contains("chunked")
    });
    let rest = raw[split + 4..].to_vec();
    let body = if chunked { dechunk(&rest) } else { rest };
    Ok(Response { status, body })
}

/// Join an HTTP/1.1 chunked body (the servers close the connection, so this is the simple case).
fn dechunk(mut data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(i) = data.windows(2).position(|w| w == b"\r\n") {
        let size =
            usize::from_str_radix(String::from_utf8_lossy(&data[..i]).trim(), 16).unwrap_or(0);
        if size == 0 {
            break;
        }
        let start = i + 2;
        let end = (start + size).min(data.len());
        out.extend_from_slice(&data[start..end]);
        if end + 2 > data.len() {
            break;
        }
        data = &data[end + 2..];
    }
    out
}

/// POST a JSON body and read the reply.
pub fn post_json(
    url: &str,
    body: &serde_json::Value,
    extra: &[(String, String)],
    wait: Duration,
) -> Result<Response> {
    send(
        url,
        "application/json",
        &serde_json::to_vec(body)?,
        extra,
        wait,
    )
}

/// One field of a multipart form: a value, or a file.
pub enum Part<'a> {
    Field(&'a str, &'a str),
    File {
        field: &'a str,
        filename: &'a str,
        mime: &'a str,
        bytes: &'a [u8],
    },
}

/// POST a `multipart/form-data` body (what an OpenAI-compatible audio endpoint takes).
pub fn post_multipart(
    url: &str,
    parts: &[Part],
    extra: &[(String, String)],
    wait: Duration,
) -> Result<Response> {
    let boundary = {
        let mut b = [0u8; 16];
        getrandom::getrandom(&mut b).map_err(|e| Error::Invalid(e.to_string()))?;
        format!(
            "kitsune{}",
            b.iter().map(|x| format!("{x:02x}")).collect::<String>()
        )
    };
    let mut body = Vec::new();
    for part in parts {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        match part {
            Part::Field(name, value) => {
                body.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
                );
                body.extend_from_slice(value.as_bytes());
            }
            Part::File {
                field,
                filename,
                mime,
                bytes,
            } => {
                body.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{field}\"; filename=\"{filename}\"\r\nContent-Type: {mime}\r\n\r\n").as_bytes(),
                );
                body.extend_from_slice(bytes);
            }
        }
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    send(
        url,
        &format!("multipart/form-data; boundary={boundary}"),
        &body,
        extra,
        wait,
    )
}
