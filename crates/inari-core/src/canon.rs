//! Canonical JSON and hashing: one byte sequence per value, on every node and in
//! every language. Keys are sorted, separators are compact, text is raw UTF-8 —
//! the same rule as the Python reference (`json.dumps(sort_keys=True,
//! separators=(",", ":"), ensure_ascii=False)`).

use serde_json::Value;
use sha2::{Digest, Sha256};

/// Serialize `value` so equal values always yield identical bytes.
///
/// `serde_json::Map` is ordered by key unless the `preserve_order` feature is
/// enabled, which this crate never does.
pub fn canonical(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).expect("a JSON value always serializes")
}

pub fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn digest_of(value: &Value) -> String {
    sha256_hex(&canonical(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keys_are_sorted_and_separators_compact() {
        let v = json!({"b": 1, "a": {"d": [1, 2], "c": "é"}});
        assert_eq!(
            String::from_utf8(canonical(&v)).unwrap(),
            r#"{"a":{"c":"é","d":[1,2]},"b":1}"#
        );
    }

    #[test]
    fn floats_keep_their_shortest_form() {
        assert_eq!(canonical(&json!(1.0)), b"1.0");
        assert_eq!(canonical(&json!(1793012345.6789)), b"1793012345.6789");
    }
}
