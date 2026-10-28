//! Authenticated framing over a stream (the format the retired Python reference spoke too).
//!
//! Handshake: each side sends 16 random bytes; the session key is
//! HMAC(key, "session" | client-nonce | server-nonce). Each frame is a 4-byte
//! big-endian length, a 32-byte HMAC(session, seq | body) and a JSON body, with a
//! sequence number per direction: frames cannot be altered, injected or replayed.
//! There is no encryption: run it over a private network or a tunnel.

use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub const MAX_FRAME: usize = 32 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("{0}")]
    Lost(String),
    #[error("bad frame signature (wrong key or tampering)")]
    BadSignature,
    #[error("frame too large")]
    TooLarge,
    #[error("timed out: {0}")]
    Timeout(&'static str),
}

pub fn mac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC takes any key length");
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            m.update(b"|");
        }
        m.update(part);
    }
    m.finalize().into_bytes().into()
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    getrandom::getrandom(&mut out).expect("the system has a random source");
    out
}

pub fn random_hex(n: usize) -> String {
    let mut buf = vec![0u8; n];
    getrandom::getrandom(&mut buf).expect("the system has a random source");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

pub struct Wire {
    stream: TcpStream,
    key: Vec<u8>,
    session: Vec<u8>,
    seq_out: u64,
    seq_in: u64,
    pub peer: Value,
}

impl Wire {
    pub fn new(stream: TcpStream, key: &[u8]) -> Self {
        Self {
            stream,
            key: key.to_vec(),
            session: Vec::new(),
            seq_out: 0,
            seq_in: 0,
            peer: Value::Null,
        }
    }

    pub async fn connect(
        addr: &str,
        key: &[u8],
        timeout: Duration,
    ) -> Result<Self, TransportError> {
        let stream = tokio::time::timeout(timeout, TcpStream::connect(addr))
            .await
            .map_err(|_| TransportError::Timeout("connect"))?
            .map_err(|e| TransportError::Lost(format!("cannot connect to {addr}: {e}")))?;
        Ok(Self::new(stream, key))
    }

    pub async fn handshake(
        &mut self,
        client: bool,
        hello: Value,
        timeout: Duration,
    ) -> Result<(), TransportError> {
        let mine: [u8; 16] = random_bytes();
        let lost = |e: std::io::Error| TransportError::Lost(format!("handshake failed: {e}"));
        self.stream.write_all(&mine).await.map_err(lost)?;
        let mut theirs = [0u8; 16];
        tokio::time::timeout(timeout, self.stream.read_exact(&mut theirs))
            .await
            .map_err(|_| TransportError::Timeout("handshake"))?
            .map_err(lost)?;
        let (c, s) = if client {
            (mine, theirs)
        } else {
            (theirs, mine)
        };
        self.session = mac(&self.key, &[b"session", &c, &s]).to_vec();
        self.send(&json!({ "hello": hello })).await?;
        let reply = tokio::time::timeout(timeout, self.recv())
            .await
            .map_err(|_| TransportError::Timeout("handshake"))??;
        self.peer = reply.get("hello").cloned().unwrap_or(Value::Null);
        Ok(())
    }

    pub async fn send(&mut self, obj: &Value) -> Result<(), TransportError> {
        let body = serde_json::to_vec(obj).expect("a JSON value serializes");
        if body.len() > MAX_FRAME {
            return Err(TransportError::TooLarge);
        }
        let tag = mac(&self.session, &[&self.seq_out.to_be_bytes(), &body]);
        self.seq_out += 1;
        let mut frame = Vec::with_capacity(4 + 32 + body.len());
        frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
        frame.extend_from_slice(&tag);
        frame.extend_from_slice(&body);
        self.stream
            .write_all(&frame)
            .await
            .map_err(|e| TransportError::Lost(format!("send failed: {e}")))
    }

    pub async fn recv(&mut self) -> Result<Value, TransportError> {
        let lost = |e: std::io::Error| TransportError::Lost(format!("connection lost: {e}"));
        let mut len = [0u8; 4];
        self.stream.read_exact(&mut len).await.map_err(lost)?;
        let size = u32::from_be_bytes(len) as usize;
        if size > MAX_FRAME {
            return Err(TransportError::TooLarge);
        }
        let mut tag = [0u8; 32];
        self.stream.read_exact(&mut tag).await.map_err(lost)?;
        let mut body = vec![0u8; size];
        self.stream.read_exact(&mut body).await.map_err(lost)?;
        let expected = mac(&self.session, &[&self.seq_in.to_be_bytes(), &body]);
        // Constant-time comparison.
        if tag
            .iter()
            .zip(expected.iter())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            != 0
        {
            return Err(TransportError::BadSignature);
        }
        self.seq_in += 1;
        serde_json::from_slice(&body).map_err(|e| TransportError::Lost(format!("bad frame: {e}")))
    }

    /// Separate halves, so a link can read and write at the same time.
    pub fn split(self) -> (WireReader, WireWriter) {
        let (r, w) = self.stream.into_split();
        (
            WireReader {
                half: r,
                session: self.session.clone(),
                seq: self.seq_in,
            },
            WireWriter {
                half: w,
                session: self.session,
                seq: self.seq_out,
            },
        )
    }

    pub async fn close(mut self) {
        let _ = self.stream.shutdown().await;
    }
}

pub struct WireReader {
    half: tokio::net::tcp::OwnedReadHalf,
    session: Vec<u8>,
    seq: u64,
}

pub struct WireWriter {
    half: tokio::net::tcp::OwnedWriteHalf,
    session: Vec<u8>,
    seq: u64,
}

impl WireReader {
    pub async fn recv(&mut self) -> Result<Value, TransportError> {
        let lost = |e: std::io::Error| TransportError::Lost(format!("connection lost: {e}"));
        let mut len = [0u8; 4];
        self.half.read_exact(&mut len).await.map_err(lost)?;
        let size = u32::from_be_bytes(len) as usize;
        if size > MAX_FRAME {
            return Err(TransportError::TooLarge);
        }
        let mut tag = [0u8; 32];
        self.half.read_exact(&mut tag).await.map_err(lost)?;
        let mut body = vec![0u8; size];
        self.half.read_exact(&mut body).await.map_err(lost)?;
        let expected = mac(&self.session, &[&self.seq.to_be_bytes(), &body]);
        if tag
            .iter()
            .zip(expected.iter())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            != 0
        {
            return Err(TransportError::BadSignature);
        }
        self.seq += 1;
        serde_json::from_slice(&body).map_err(|e| TransportError::Lost(format!("bad frame: {e}")))
    }
}

impl WireWriter {
    pub async fn send(&mut self, obj: &Value) -> Result<(), TransportError> {
        let body = serde_json::to_vec(obj).expect("a JSON value serializes");
        if body.len() > MAX_FRAME {
            return Err(TransportError::TooLarge);
        }
        let tag = mac(&self.session, &[&self.seq.to_be_bytes(), &body]);
        self.seq += 1;
        let mut frame = Vec::with_capacity(4 + 32 + body.len());
        frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
        frame.extend_from_slice(&tag);
        frame.extend_from_slice(&body);
        self.half
            .write_all(&frame)
            .await
            .map_err(|e| TransportError::Lost(format!("send failed: {e}")))
    }
}
