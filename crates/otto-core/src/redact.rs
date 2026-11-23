//! Redaction — a regex-free, dependency-light pass that strips likely secrets
//! and PII from text and JSON before it leaves a trust boundary. Two callers:
//!
//! * the first-party agent tools / "send-to-agent" packets, which must show the
//!   operator *what* was redacted before a packet is handed to an agent; and
//! * the database / broker "mask PII/prod" presets.
//!
//! It is deliberately conservative and heuristic (no entropy guessing): it
//! redacts values under sensitive JSON keys, and a small set of high-confidence
//! text shapes (JWTs, AWS access keys, PEM blocks, `Bearer` tokens, emails).
//! `Redacted::hits` reports the kinds + counts so a UI can summarize the diff.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// One class of redaction and how many were applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactionHit {
    pub kind: String,
    pub count: usize,
}

/// Result of a redaction pass: the cleaned value plus a per-kind tally.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Redacted<T> {
    pub value: T,
    pub hits: Vec<RedactionHit>,
}

const PLACEHOLDER: &str = "[redacted]";

/// Tally helper: accumulates kind→count and emits a stable, sorted hit list.
#[derive(Default)]
struct Tally(BTreeMap<&'static str, usize>);

impl Tally {
    fn bump(&mut self, kind: &'static str) {
        *self.0.entry(kind).or_insert(0) += 1;
    }
    fn into_hits(self) -> Vec<RedactionHit> {
        self.0
            .into_iter()
            .map(|(kind, count)| RedactionHit {
                kind: kind.to_string(),
                count,
            })
            .collect()
    }
}

/// Map a (lower-cased) JSON key name to a redaction kind, if it looks sensitive.
/// Checked most-specific first.
fn sensitive_key_kind(key_lower: &str) -> Option<&'static str> {
    let k = key_lower;
    if k.contains("password") || k.contains("passwd") {
        Some("password")
    } else if k.contains("api_key")
        || k.contains("apikey")
        || k.contains("access_key")
        || k.contains("accesskey")
    {
        Some("api_key")
    } else if k.contains("private_key") || k.contains("privatekey") {
        Some("private_key")
    } else if k.contains("client_secret") || k.contains("secret") {
        Some("secret")
    } else if k.contains("authorization") {
        Some("authorization")
    } else if k.contains("credential") {
        Some("credential")
    } else if k.contains("cookie") {
        Some("cookie")
    } else if k.contains("refresh_token")
        || k.contains("access_token")
        || k.contains("id_token")
        || k.contains("session_token")
        || k.contains("token")
    {
        Some("token")
    } else {
        None
    }
}

/// Trim a small set of surrounding punctuation that commonly wraps tokens in
/// prose, without stripping characters that are part of tokens (`.-_/+=@`).
fn trim_punct(w: &str) -> &str {
    w.trim_matches(|c: char| {
        matches!(
            c,
            '"' | '\'' | '(' | ')' | '[' | ']' | ',' | ';' | ':' | '<' | '>' | '{' | '}'
        )
    })
}

fn is_b64url_dotted(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+' | '/' | '='))
}

fn looks_like_email(w: &str) -> bool {
    let at = match w.find('@') {
        Some(i) => i,
        None => return false,
    };
    let (local, rest) = w.split_at(at);
    let domain = &rest[1..];
    !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
}

fn is_token_body(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

fn is_basic_b64(w: &str) -> bool {
    if w.len() < 8
        || !w
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
    {
        return false;
    }
    let has_upper = w.bytes().any(|b| b.is_ascii_uppercase());
    let has_lower = w.bytes().any(|b| b.is_ascii_lowercase());
    let has_digit = w.bytes().any(|b| b.is_ascii_digit());
    w.ends_with('=') || (has_upper && has_lower) || has_digit || w.contains('+') || w.contains('/')
}

/// Whether `tail` (starting at `-----BEGIN`) begins with a real PEM header line
/// for a key/certificate/PGP block, even if the closing `-----END` was clipped
/// by a read/log size cap.
fn looks_like_pem_header(tail: &str) -> bool {
    let first_line = tail.lines().next().unwrap_or(tail).trim_end_matches('\r');
    let Some(rest) = first_line.strip_prefix("-----BEGIN ") else {
        return false;
    };
    let Some(label) = rest.strip_suffix("-----") else {
        return false;
    };
    !label.is_empty()
        && label
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || matches!(b, b' ' | b'-'))
        && (label.contains("KEY") || label.contains("CERTIFICATE") || label.contains("PGP"))
}

/// Classify a single (already punctuation-trimmed) word. `auth_scheme` is set
/// when the previous word was a literal `Bearer`/`Basic` auth scheme keyword.
fn classify_word(w: &str, auth_scheme: Option<&'static str>) -> Option<&'static str> {
    if w.starts_with("eyJ") && w.len() > 20 && is_b64url_dotted(w) {
        return Some("jwt");
    }
    if w.len() == 20
        && (w.starts_with("AKIA") || w.starts_with("ASIA"))
        && w[4..]
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        return Some("aws_key");
    }
    if (w.starts_with("ghp_")
        || w.starts_with("gho_")
        || w.starts_with("ghu_")
        || w.starts_with("ghs_")
        || w.starts_with("ghr_"))
        && w.len() >= 20
        && is_token_body(&w[4..])
    {
        return Some("github_token");
    }
    if let Some(rest) = w.strip_prefix("github_pat_") {
        if rest.len() >= 20 && is_token_body(rest) {
            return Some("github_token");
        }
    }
    if let Some(rest) = w
        .strip_prefix("sk-ant-")
        .or_else(|| w.strip_prefix("sk-proj-"))
    {
        if rest.len() >= 16 && is_token_body(rest) {
            return Some("api_key");
        }
    }
    if let Some(rest) = w.strip_prefix("sk-") {
        if rest.len() >= 24 && is_token_body(rest) && rest.bytes().any(|b| b.is_ascii_digit()) {
            return Some("api_key");
        }
    }
    if (w.starts_with("xoxb-")
        || w.starts_with("xoxp-")
        || w.starts_with("xoxo-")
        || w.starts_with("xoxa-")
        || w.starts_with("xoxr-"))
        && w.len() >= 20
        && is_token_body(&w[5..])
    {
        return Some("slack_token");
    }
    if w.len() == 39 && w.starts_with("AIza") && is_token_body(&w[4..]) {
        return Some("api_key");
    }
    if looks_like_email(w) {
        return Some("email");
    }
    match auth_scheme {
        Some("bearer") if w.len() >= 8 && is_b64url_dotted(w) => return Some("bearer"),
        Some("basic") if is_basic_b64(w) => return Some("basic"),
        _ => {}
    }
    None
}

/// Redact a chunk of free text. Whitespace is preserved; matched words are
/// swapped for `[redacted]`. PEM key blocks (possibly multi-line) are collapsed.
pub fn redact_text(input: &str) -> Redacted<String> {
    let mut tally = Tally::default();
    let cleaned = redact_text_into(input, &mut tally);
    Redacted {
        value: cleaned,
        hits: tally.into_hits(),
    }
}

fn redact_text_into(input: &str, tally: &mut Tally) -> String {
    // 1. Collapse PEM blocks first (they span multiple whitespace-delimited words).
    let mut s = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("-----BEGIN") {
        s.push_str(&rest[..start]);
        let tail = &rest[start..];
        if let Some(end_rel) = tail.find("-----END") {
            // Include the closing "-----END ...-----" line up to the next newline.
            let after_end = &tail[end_rel..];
            let close = after_end
                .find('\n')
                .map(|n| end_rel + n)
                .unwrap_or(tail.len());
            s.push_str(PLACEHOLDER);
            tally.bump("private_key");
            rest = &tail[close..];
        } else if looks_like_pem_header(tail) {
            // Truncated PEM block (e.g. clipped at a read/log cap before
            // `-----END`): fail closed and redact the remainder of the block.
            s.push_str(PLACEHOLDER);
            tally.bump("private_key");
            rest = "";
            break;
        } else {
            s.push_str("-----BEGIN");
            rest = &tail["-----BEGIN".len()..];
        }
    }
    s.push_str(rest);

    // 2. Word scan for the remaining single-token shapes.
    let mut out = String::with_capacity(s.len());
    let mut word = String::new();
    let mut auth_scheme: Option<&'static str> = None;
    let flush = |word: &mut String,
                 out: &mut String,
                 auth_scheme: &mut Option<&'static str>,
                 tally: &mut Tally| {
        if word.is_empty() {
            return;
        }
        let trimmed = trim_punct(word);
        let next_scheme = match trimmed {
            "Bearer" | "bearer" => Some("bearer"),
            "Basic" | "basic" => Some("basic"),
            _ => None,
        };
        match classify_word(trimmed, *auth_scheme) {
            Some(kind) => {
                // Preserve any leading/trailing punctuation we trimmed.
                let lead = &word[..word.find(trimmed).unwrap_or(0)];
                let tail_start = word
                    .find(trimmed)
                    .map(|i| i + trimmed.len())
                    .unwrap_or(word.len());
                out.push_str(lead);
                out.push_str(PLACEHOLDER);
                out.push_str(&word[tail_start..]);
                tally.bump(kind);
            }
            None => {
                // Also inspect the value half of `KEY=VALUE` pairs (`.env`,
                // query strings, CLI flags like `--token=ghp_...`).
                if let Some((lhs, rhs)) = trimmed.split_once('=') {
                    let val = trim_punct(rhs);
                    if let Some(kind) = classify_word(val, None) {
                        let trimmed_start = word.find(trimmed).unwrap_or(0);
                        let rhs_offset = trimmed_start + lhs.len() + 1;
                        let val_offset = rhs_offset + rhs.find(val).unwrap_or(0);
                        out.push_str(&word[..val_offset]);
                        out.push_str(PLACEHOLDER);
                        out.push_str(&word[val_offset + val.len()..]);
                        tally.bump(kind);
                    } else {
                        out.push_str(word);
                    }
                } else {
                    out.push_str(word);
                }
            }
        }
        *auth_scheme = next_scheme;
        word.clear();
    };
    for ch in s.chars() {
        if ch.is_whitespace() {
            flush(&mut word, &mut out, &mut auth_scheme, tally);
            out.push(ch);
        } else {
            word.push(ch);
        }
    }
    flush(&mut word, &mut out, &mut auth_scheme, tally);
    out
}

/// Redact a JSON value: values under sensitive keys are replaced wholesale;
/// every other string is passed through [`redact_text`].
pub fn redact_json(input: &Value) -> Redacted<Value> {
    let mut tally = Tally::default();
    let value = redact_value(input, &mut tally);
    Redacted {
        value,
        hits: tally.into_hits(),
    }
}

fn redact_value(v: &Value, tally: &mut Tally) -> Value {
    match v {
        Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, val) in map {
                if let Some(kind) = sensitive_key_kind(&k.to_lowercase()) {
                    out.insert(k.clone(), Value::String(PLACEHOLDER.to_string()));
                    tally.bump(kind);
                } else {
                    out.insert(k.clone(), redact_value(val, tally));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(|i| redact_value(i, tally)).collect()),
        Value::String(s) => Value::String(redact_text_into(s, tally)),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plain_text_passes_through() {
        let r = redact_text("the quick brown fox jumps over 3 lazy dogs");
        assert_eq!(r.value, "the quick brown fox jumps over 3 lazy dogs");
        assert!(r.hits.is_empty());
    }

    #[test]
    fn redacts_jwt_and_email_and_bearer() {
        let input = "auth Bearer abcdef123456 token eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.payload.sig mail a.b@example.com";
        let r = redact_text(input);
        assert!(r.value.contains(PLACEHOLDER));
        assert!(!r.value.contains("eyJhbGci"));
        assert!(!r.value.contains("a.b@example.com"));
        let kinds: Vec<&str> = r.hits.iter().map(|h| h.kind.as_str()).collect();
        assert!(kinds.contains(&"jwt"));
        assert!(kinds.contains(&"email"));
        assert!(kinds.contains(&"bearer"));
    }

    #[test]
    fn redacts_aws_key_and_pem() {
        let input = "key AKIAIOSFODNN7EXAMPLE then\n-----BEGIN RSA PRIVATE KEY-----\nMIIBjunk\n-----END RSA PRIVATE KEY-----\ntail";
        let r = redact_text(input);
        assert!(!r.value.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(!r.value.contains("MIIBjunk"));
        assert!(r.value.contains("tail"));
        let kinds: Vec<&str> = r.hits.iter().map(|h| h.kind.as_str()).collect();
        assert!(kinds.contains(&"aws_key"));
        assert!(kinds.contains(&"private_key"));
    }

    #[test]
    fn json_redacts_sensitive_keys_and_nested_strings() {
        let v = json!({
            "username": "alice",
            "password": "hunter2",
            "config": { "api_key": "xyz", "note": "contact a.b@example.com" },
            "items": [ { "Authorization": "Bearer t" } ]
        });
        let r = redact_json(&v);
        assert_eq!(r.value["username"], json!("alice"));
        assert_eq!(r.value["password"], json!(PLACEHOLDER));
        assert_eq!(r.value["config"]["api_key"], json!(PLACEHOLDER));
        assert!(r.value["config"]["note"]
            .as_str()
            .unwrap()
            .contains(PLACEHOLDER));
        assert_eq!(r.value["items"][0]["Authorization"], json!(PLACEHOLDER));
        let total: usize = r.hits.iter().map(|h| h.count).sum();
        assert!(total >= 4);
    }

    #[test]
    fn redacts_truncated_pem_private_key() {
        for header in [
            "-----BEGIN RSA PRIVATE KEY-----",
            "-----BEGIN OPENSSH PRIVATE KEY-----",
            "-----BEGIN EC PRIVATE KEY-----",
            "-----BEGIN PRIVATE KEY-----",
            "-----BEGIN PGP PRIVATE KEY BLOCK-----",
        ] {
            let input = format!("prefix\n{header}\nMIIEpAIBAAKCAQEAt0truncatedsecretbytes");
            let r = redact_text(&input);
            assert!(
                !r.value.contains("MIIEpAIBAAKCAQEAt0truncatedsecretbytes"),
                "key body leaked for {header}: {}",
                r.value
            );
            assert!(r.value.starts_with("prefix\n"));
            assert!(r.value.contains(PLACEHOLDER));
            assert!(r.hits.iter().any(|h| h.kind == "private_key"));
        }
        // Prose mentioning `-----BEGIN` without a real PEM key header passes through.
        let prose = redact_text("note: -----BEGIN is a marker in PEM files");
        assert_eq!(prose.value, "note: -----BEGIN is a marker in PEM files");
        assert!(prose.hits.is_empty());
    }

    #[test]
    fn redacts_modern_token_prefixes_and_basic_auth() {
        let gh = "ghp_1234567890abcdef1234567890abcdef1234";
        let gh_pat = "github_pat_11AAAAAA0123456789_abcdef0123456789ABCDEF";
        let anth = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz123456";
        let oai = "sk-proj-abcdefghijklmnopqrstuvwxyz1234567890";
        let slack = "xoxb-fake-token-for-redaction-test";
        let sts = "ASIAIOSFODNN7EXAMPLE";
        let goog = "AIzaSyDaGmWKa4JsXZ-HjGw7ISLn_3namBGewQe";
        let basic = "dXNlcjpwYXNz";
        let input = format!(
            "gh={gh} pat={gh_pat} anth={anth} oai={oai} slack={slack} sts={sts} goog={goog} Authorization: Basic {basic}"
        );
        let r = redact_text(&input);
        for secret in [gh, gh_pat, anth, oai, slack, sts, goog, basic] {
            assert!(
                !r.value.contains(secret),
                "secret leaked ({secret}): {}",
                r.value
            );
        }
        // Normal English after "Basic" must not false-positive.
        let ok = redact_text("Basic configuration and sk-short are fine");
        assert_eq!(ok.value, "Basic configuration and sk-short are fine");
        assert!(ok.hits.is_empty());
    }
}
