//! Browser sessions. A page signs in once (`POST /api/session` with the owner's token or a
//! pass) and receives a cookie the page's script cannot read (`HttpOnly`) and another site
//! never sends (`SameSite=Strict`): the token no longer travels in URLs (`/events?token=`)
//! nor sits in the browser's storage, where an injected script could take it.
//!
//! The cookie is signed, not stored: `v1.<kind>.<hash>.<expiry>.<mac>`, the MAC made with a
//! key kept in the gateway's data directory, so a session survives a restart. It names
//! what it was opened with by a hash only: the owner's token (a new token ends every owner
//! session) or a pass (looked up again on every request, so a revoked pass ends at once).
//!
//! Attempts are throttled per address: after a few wrong secrets, each further one waits
//! longer, up to five minutes.

use crate::canon::sha256_hex;
use crate::wire::{mac, random_bytes};
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const COOKIE: &str = "inari_session";
/// How long a session lasts without signing in again.
pub const TTL: Duration = Duration::from_secs(30 * 24 * 3600);
/// Wrong secrets allowed before waiting starts.
const FREE_FAILS: u32 = 5;
const MAX_WAIT: Duration = Duration::from_secs(300);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Owner,
    Pass,
}

impl Kind {
    fn tag(self) -> &'static str {
        match self {
            Kind::Owner => "o",
            Kind::Pass => "p",
        }
    }
}

/// What an owner session is known by: a hash of the token it was opened with.
pub fn owner_hash(token: &str) -> String {
    sha256_hex(format!("inari-owner|{token}").as_bytes())[..32].to_string()
}

pub struct Sessions {
    key: [u8; 32],
    fails: Mutex<HashMap<IpAddr, (u32, Instant)>>,
}

impl Sessions {
    /// The key lives in `<data_dir>/session.key` (created on first use, readable by its owner
    /// only); without a usable directory it is random, and sessions end with the process.
    pub fn open(data_dir: &Path) -> Self {
        let file = data_dir.join("session.key");
        let key = std::fs::read(&file)
            .ok()
            .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
            .unwrap_or_else(|| {
                let fresh = random_bytes::<32>();
                if crate::capsule::io::atomic_write(&file, &fresh).is_ok() {
                    restrict(&file);
                }
                fresh
            });
        Self {
            key,
            fails: Mutex::default(),
        }
    }

    fn mac_of(&self, kind: Kind, hash: &str, exp: u64) -> String {
        let m = mac(
            &self.key,
            &[
                b"inari-session",
                kind.tag().as_bytes(),
                hash.as_bytes(),
                exp.to_string().as_bytes(),
            ],
        );
        m.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// A cookie value for a session of `kind`, known by `hash`, valid for [`TTL`].
    pub fn issue(&self, kind: Kind, hash: &str) -> String {
        let exp = crate::approvals::now() as u64 + TTL.as_secs();
        format!(
            "v1.{}.{hash}.{exp}.{}",
            kind.tag(),
            self.mac_of(kind, hash, exp)
        )
    }

    /// The session a cookie value carries, if it is ours, intact and not expired.
    pub fn read(&self, value: &str) -> Option<(Kind, String)> {
        let mut parts = value.split('.');
        let (v, tag, hash, exp, given) = (
            parts.next()?,
            parts.next()?,
            parts.next()?,
            parts.next()?,
            parts.next()?,
        );
        if v != "v1" || parts.next().is_some() {
            return None;
        }
        let kind = match tag {
            "o" => Kind::Owner,
            "p" => Kind::Pass,
            _ => return None,
        };
        let exp: u64 = exp.parse().ok()?;
        let want = self.mac_of(kind, hash, exp);
        let same = want.len() == given.len()
            && want
                .bytes()
                .zip(given.bytes())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0;
        (same && (crate::approvals::now() as u64) < exp).then(|| (kind, hash.to_string()))
    }

    /// How long `peer` must still wait before its next attempt (zero: it may try now).
    pub fn wait(&self, peer: Option<IpAddr>) -> Duration {
        let Some(ip) = peer else {
            return Duration::ZERO;
        };
        let fails = self.fails.lock().expect("not poisoned");
        match fails.get(&ip) {
            Some(&(n, at)) if n >= FREE_FAILS => {
                let wait = Duration::from_secs(1 << (n - FREE_FAILS).min(9)).min(MAX_WAIT);
                wait.saturating_sub(at.elapsed())
            }
            _ => Duration::ZERO,
        }
    }

    /// Count an attempt: a wrong secret adds to the address's count, a right one clears it.
    pub fn attempt(&self, peer: Option<IpAddr>, ok: bool) {
        let Some(ip) = peer else { return };
        let mut fails = self.fails.lock().expect("not poisoned");
        if ok {
            fails.remove(&ip);
            return;
        }
        if fails.len() > 4096 {
            fails.clear(); // a flood of addresses: forget them rather than grow without end
        }
        let entry = fails.entry(ip).or_insert((0, Instant::now()));
        *entry = (entry.0 + 1, Instant::now());
    }
}

/// The value of our cookie in a request's `Cookie` header.
pub fn from_header(cookies: &str) -> Option<&str> {
    cookies
        .split(';')
        .filter_map(|c| c.trim().split_once('='))
        .find(|(name, _)| *name == COOKIE)
        .map(|(_, value)| value)
        .filter(|v| !v.is_empty())
}

/// The `Set-Cookie` header that opens a session (`value`) or ends it (`None`). `Secure` when
/// the page was reached over HTTPS (the gate in front says so in `X-Forwarded-Proto`).
pub fn set_cookie(value: Option<&str>, secure: bool) -> String {
    let (value, age) = match value {
        Some(v) => (v, TTL.as_secs()),
        None => ("", 0),
    };
    format!(
        "Set-Cookie: {COOKIE}={value}; Path=/; HttpOnly; SameSite=Strict; Max-Age={age}{}\r\n",
        if secure { "; Secure" } else { "" }
    )
}

#[cfg(unix)]
fn restrict(file: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict(_: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cookie_reads_back_and_resists_tampering() {
        let dir = tempfile::tempdir().unwrap();
        let s = Sessions::open(dir.path());
        let c = s.issue(Kind::Pass, "abc");
        assert_eq!(s.read(&c), Some((Kind::Pass, "abc".to_string())));
        assert_eq!(s.read(&c.replace(".p.", ".o.")), None, "the kind is signed");
        assert_eq!(s.read(&c.replace("abc", "abd")), None, "the hash is signed");
        let parts: Vec<&str> = c.split('.').collect();
        let later = format!(
            "v1.p.abc.{}.{}",
            parts[3].parse::<u64>().unwrap() + 1,
            parts[4]
        );
        assert_eq!(s.read(&later), None, "the expiry is signed");
        assert_eq!(s.read("v1.p.abc.1.00"), None);
        // the key is kept: a restarted gateway still knows its sessions; another key does not
        assert_eq!(
            Sessions::open(dir.path()).read(&c),
            Some((Kind::Pass, "abc".to_string()))
        );
        let other = tempfile::tempdir().unwrap();
        assert_eq!(Sessions::open(other.path()).read(&c), None);
    }

    #[test]
    fn an_expired_cookie_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let s = Sessions::open(dir.path());
        let exp = crate::approvals::now() as u64 - 1;
        let c = format!("v1.o.h.{exp}.{}", s.mac_of(Kind::Owner, "h", exp));
        assert_eq!(s.read(&c), None);
    }

    #[test]
    fn wrong_secrets_make_an_address_wait() {
        let dir = tempfile::tempdir().unwrap();
        let s = Sessions::open(dir.path());
        let ip: Option<IpAddr> = Some("10.0.0.7".parse().unwrap());
        for _ in 0..FREE_FAILS {
            assert_eq!(s.wait(ip), Duration::ZERO);
            s.attempt(ip, false);
        }
        assert!(s.wait(ip) > Duration::ZERO, "the sixth try waits");
        assert_eq!(s.wait(Some("10.0.0.8".parse().unwrap())), Duration::ZERO);
        s.attempt(ip, true);
        assert_eq!(
            s.wait(ip),
            Duration::ZERO,
            "a right secret clears the count"
        );
    }

    #[test]
    fn the_cookie_is_found_among_others() {
        assert_eq!(from_header("a=1; inari_session=v1.x; b=2"), Some("v1.x"));
        assert_eq!(from_header("a=1"), None);
        assert_eq!(from_header("inari_session="), None);
        assert!(set_cookie(Some("v"), true).contains("HttpOnly; SameSite=Strict"));
        assert!(set_cookie(None, false).contains("Max-Age=0"));
    }
}
