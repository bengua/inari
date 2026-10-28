//! The page speaks English and French: every text `app.js` passes to `t()`, and every text of the
//! static `index.html`, has its French in `i18n.js`, and no translation drops a `{name}`.

use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::path::PathBuf;

fn web(file: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../src/inari_core/web")
        .join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The French table, between its `FR-START` and `FR-END` marks.
fn french() -> Map<String, Value> {
    let js = web("i18n.js");
    let body = js
        .split_once("/* FR-START */")
        .and_then(|(_, rest)| rest.split_once("/* FR-END */"))
        .expect("the French table is marked")
        .0;
    serde_json::from_str(body).expect("the French table is JSON")
}

/// The source of the first argument of a call, from just after its `(`.
fn first_argument(src: &[char], start: usize) -> String {
    let (mut depth, mut quote, mut i) = (0i32, None::<char>, start);
    while i < src.len() {
        let c = src[i];
        if let Some(q) = quote {
            if c == '\\' {
                i += 2;
                continue;
            }
            if c == q {
                quote = None;
            }
        } else if matches!(c, '"' | '\'' | '`') {
            quote = Some(c);
        } else if matches!(c, '(' | '[' | '{') {
            depth += 1;
        } else if matches!(c, ')' | ']' | '}') {
            if depth == 0 {
                break;
            }
            depth -= 1;
        } else if c == ',' && depth == 0 {
            break;
        }
        i += 1;
    }
    src[start..i].iter().collect()
}

/// The double-quoted literals in a piece of source (escapes kept as written).
fn literals(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = src.chars();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut lit = String::new();
        while let Some(d) = chars.next() {
            match d {
                '\\' => {
                    lit.push(d);
                    if let Some(e) = chars.next() {
                        lit.push(e);
                    }
                }
                '"' => break,
                _ => lit.push(d),
            }
        }
        out.push(lit);
    }
    out
}

/// The `{name}` placeholders of a text.
fn placeholders(text: &str) -> BTreeSet<String> {
    text.split('{')
        .skip(1)
        .filter_map(|rest| rest.split_once('}').map(|(name, _)| name))
        .filter(|name| !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_'))
        .map(String::from)
        .collect()
}

#[test]
fn every_text_given_to_t_has_its_french() {
    let src: Vec<char> = web("app.js").chars().collect();
    let fr = french();
    let mut keys = BTreeSet::new();
    for i in 0..src.len().saturating_sub(2) {
        let starts_word =
            i == 0 || !(src[i - 1].is_alphanumeric() || src[i - 1] == '_' || src[i - 1] == '.');
        if starts_word && src[i] == 't' && src[i + 1] == '(' {
            keys.extend(literals(&first_argument(&src, i + 2)));
        }
    }
    keys.remove(" pass"); // t(role + " pass"): the full keys follow
    keys.extend(["supervision pass", "collaboration pass"].map(String::from));
    let missing: Vec<_> = keys.iter().filter(|k| !fr.contains_key(*k)).collect();
    assert!(missing.is_empty(), "no French for: {missing:?}");
    assert!(
        keys.len() > 200,
        "the scan found the page's texts ({})",
        keys.len()
    );
}

#[test]
fn the_static_page_is_translated_too() {
    let html = web("index.html");
    let fr = french();
    let mut texts = BTreeSet::new();
    for attr in ["title=\"", "aria-label=\"", "placeholder=\""] {
        for part in html.split(attr).skip(1) {
            texts.insert(part.split('"').next().unwrap_or("").to_string());
        }
    }
    for part in html.split('>').skip(1) {
        let text = part.split('<').next().unwrap_or("").trim();
        if text.chars().any(|c| c.is_ascii_alphabetic()) {
            texts.insert(text.to_string());
        }
    }
    texts.remove("Inari");
    let missing: Vec<_> = texts.iter().filter(|t| !fr.contains_key(*t)).collect();
    assert!(missing.is_empty(), "no French for: {missing:?}");
}

#[test]
fn french_keeps_every_placeholder() {
    for (key, value) in french() {
        assert_eq!(
            placeholders(&key),
            placeholders(value.as_str().unwrap()),
            "{key}"
        );
    }
}
