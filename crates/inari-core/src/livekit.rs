//! A LiveKit access token, so the owner's Kitsune page joins its own voice room: a JWT signed
//! with HMAC-SHA256 under the LiveKit server's API secret, as LiveKit's own SDKs make it. No
//! crate for it: sha2 is already here, and HMAC is a few lines.

use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    inner.update(msg);
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// A token for `identity` to join `room`, valid `ttl` seconds from `now` (seconds since the epoch).
pub fn token(
    api_key: &str,
    api_secret: &str,
    identity: &str,
    room: &str,
    now: u64,
    ttl: u64,
) -> String {
    let header = json!({"alg": "HS256", "typ": "JWT"});
    let claims: Value = json!({
        "iss": api_key,
        "sub": identity,
        "nbf": now.saturating_sub(10),
        "exp": now + ttl,
        "video": {"room": room, "roomJoin": true, "canPublish": true, "canSubscribe": true, "canPublishData": true},
    });
    let body = format!(
        "{}.{}",
        b64(header.to_string().as_bytes()),
        b64(claims.to_string().as_bytes())
    );
    let sig = hmac_sha256(api_secret.as_bytes(), body.as_bytes());
    format!("{body}.{}", b64(&sig))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn hmac_matches_rfc_4231() {
        // test case 2
        assert_eq!(
            hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // test case 6: a key longer than the block
        assert_eq!(
            hex(&hmac_sha256(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn a_token_names_its_room_and_who_joins() {
        let t = token(
            "APIkey",
            "secret",
            "owner",
            "kitsune-home",
            1_800_000_000,
            3600,
        );
        let parts: Vec<&str> = t.split('.').collect();
        assert_eq!(parts.len(), 3);
        let claims: Value = serde_json::from_slice(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(parts[1])
                .unwrap(),
        )
        .unwrap();
        assert_eq!(claims["iss"], "APIkey");
        assert_eq!(claims["video"]["room"], "kitsune-home");
        assert_eq!(claims["exp"], 1_800_003_600u64);
        let sig = hmac_sha256(b"secret", format!("{}.{}", parts[0], parts[1]).as_bytes());
        assert_eq!(parts[2], b64(&sig));
    }
}
