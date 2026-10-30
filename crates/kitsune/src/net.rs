//! A small blocking HTTP client for the model servers Kitsune uses on the machine or the private
//! network (transcription, the small language model, embeddings, the geocoder, the voice): plain
//! HTTP, as Inari's local engine speaks it, so no async runtime and no TLS stack are pulled into
//! this crate.
//!
//! Zero cloud is enforced here, not hoped for: every address a request goes to is resolved first,
//! and the request is refused unless all of them are on this machine or a private network
//! (loopback, RFC 1918, link-local, CGNAT/Tailscale, IPv6 ULA). Audio, words and vectors cannot
//! leave the house through this client, whatever URL a setting holds.

use crate::{Error, Result};
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

/// Is this address on the machine or a private network?
pub fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || (o[0] == 100 && (64..128).contains(&o[1]))
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            v6.is_loopback()
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xffc0) == 0xfe80
                || v6
                    .to_ipv4_mapped()
                    .is_some_and(|v4| is_private(IpAddr::V4(v4)))
        }
    }
}

/// The addresses of `host:port`, refused unless every one of them is private.
fn local_addrs(host: &str, url: &str) -> Result<Vec<SocketAddr>> {
    let addrs: Vec<SocketAddr> = host
        .to_socket_addrs()
        .map_err(|e| Error::Invalid(format!("{url}: the host cannot be resolved ({e})")))?
        .collect();
    if addrs.is_empty() || addrs.iter().any(|a| !is_private(a.ip())) {
        return Err(Error::Invalid(format!(
            "{url} is not on this machine or the private network: Kitsune sends nothing to the cloud"
        )));
    }
    Ok(addrs)
}

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

/// Why a request did not get an answer: the server could not be reached at all (worth trying
/// again later), as opposed to a server that answered something wrong.
pub fn unreachable(e: &Error) -> bool {
    matches!(e, Error::Invalid(m) if m.contains("does not answer") || m.contains("cannot be resolved"))
}

fn send(
    method: &str,
    url: &str,
    content_type: &str,
    body: &[u8],
    extra: &[(String, String)],
    wait: Duration,
) -> Result<Response> {
    let t = target(url)?;
    let addrs = local_addrs(&t.host, url)?;
    let mut stream = TcpStream::connect(&addrs[..])
        .map_err(|e| Error::Invalid(format!("{url} does not answer ({e})")))?;
    stream.set_read_timeout(Some(wait)).ok();
    stream.set_write_timeout(Some(wait)).ok();
    let mut head = format!(
        "{method} {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n",
        t.path, t.authority
    );
    if method == "POST" {
        head.push_str(&format!(
            "Content-Type: {content_type}\r\nContent-Length: {}\r\n",
            body.len()
        ));
    }
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

/// GET a URL and read the reply.
pub fn get(url: &str, extra: &[(String, String)], wait: Duration) -> Result<Response> {
    send("GET", url, "", &[], extra, wait)
}

/// A query-string value, percent-encoded (RFC 3986 unreserved characters kept).
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
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
        "POST",
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
        "POST",
        url,
        &format!("multipart/form-data; boundary={boundary}"),
        &body,
        extra,
        wait,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_machine_and_the_private_network_are_reached() {
        for ok in [
            "127.0.0.1",
            "10.0.0.5",
            "192.168.1.20",
            "172.16.3.4",
            "100.101.102.103",
            "169.254.1.1",
            "::1",
            "fd00::1",
            "fe80::1",
        ] {
            assert!(is_private(ok.parse().unwrap()), "{ok}");
        }
        for cloud in [
            "8.8.8.8",
            "1.1.1.1",
            "34.120.0.1",
            "2001:4860:4860::8888",
            "100.128.0.1",
        ] {
            assert!(!is_private(cloud.parse().unwrap()), "{cloud}");
        }
        let e = post_json(
            "http://1.1.1.1/v1/chat/completions",
            &serde_json::json!({}),
            &[],
            Duration::from_secs(1),
        )
        .err()
        .unwrap();
        assert!(e.to_string().contains("nothing to the cloud"), "{e}");
        assert!(!unreachable(&e), "a refused cloud address is not retried");
    }

    #[test]
    fn a_query_value_is_encoded() {
        assert_eq!(
            encode("12 rue de la Paix, Paris"),
            "12%20rue%20de%20la%20Paix%2C%20Paris"
        );
        assert_eq!(encode("é"), "%C3%A9");
    }
}
