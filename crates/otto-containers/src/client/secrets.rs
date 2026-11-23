//! Container environment secret classification and masking.
//!
//! Single authoritative secret classifier shared across backend responses and
//! the Containers UI (the UI relies on `EnvEntry.secret` / `EnvEntry.masked`).

use crate::types::EnvEntry;

const MASKED_PLACEHOLDER: &str = "••••••••";

/// Detect environment variables whose key or value indicates a secret.
pub fn is_secret_env(key: &str, value: &str) -> bool {
    let k = key.to_ascii_uppercase();
    const SUBSTRINGS: &[&str] = &[
        "SECRET",
        "TOKEN",
        "PASSWORD",
        "PASSWD",
        "PASSPHRASE",
        "API_KEY",
        "APIKEY",
        "PRIVATE_KEY",
        "ACCESS_KEY",
        "AUTH",
        "CREDENTIAL",
        "CREDS",
        "DATABASE_URL",
        "REDIS_URL",
        "MONGO_URI",
        "DSN",
        "CONN_STR",
        "CONNECTION_STRING",
        "SIGNING_KEY",
        "ENCRYPTION_KEY",
        "MASTER_KEY",
        "CLIENT_SECRET",
        "WEBHOOK",
        "COOKIE",
        "SESSION_KEY",
        "JWT",
        "BEARER",
        "CERT_KEY",
        "PRIVATE",
    ];
    if SUBSTRINGS.iter().any(|pat| k.contains(pat)) {
        return true;
    }
    if k.ends_with("_KEY")
        || k.ends_with("_PWD")
        || k.ends_with("_PASS")
        || k == "KEY"
        || k == "PWD"
    {
        return true;
    }
    let v = value.trim();
    if v.starts_with("ghp_")
        || v.starts_with("gho_")
        || v.starts_with("github_pat_")
        || v.starts_with("sk-")
        || v.starts_with("xoxb-")
        || v.starts_with("xoxp-")
        || v.starts_with("AKIA")
        || v.starts_with("Bearer ")
        || v.contains("BEGIN PRIVATE KEY")
        || v.contains("BEGIN RSA PRIVATE KEY")
        || value_has_inline_url_credentials(v)
    {
        return true;
    }
    false
}

/// Detect URLs embedding inline `user:pass@host` credentials (e.g.
/// `postgres://u:p@host/db`, `redis://:secret@host:6379`, `https://u:tok@host`).
fn value_has_inline_url_credentials(value: &str) -> bool {
    let Some(scheme_end) = value.find("://") else {
        return false;
    };
    let scheme = &value[..scheme_end];
    if scheme.is_empty()
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return false;
    }
    let after_scheme = &value[scheme_end + 3..];
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_end];
    if let Some((userinfo, host)) = authority.rsplit_once('@') {
        !host.is_empty() && !userinfo.is_empty() && userinfo.contains(':')
    } else {
        false
    }
}

/// Parse Docker `KEY=VALUE` entries and mask secret values when `!reveal_secrets`.
pub fn parse_env_entries(raw: &[String], reveal_secrets: bool) -> Vec<EnvEntry> {
    let mut entries = Vec::with_capacity(raw.len());
    for line in raw {
        let (key, val) = match line.split_once('=') {
            Some((k, v)) => (k.to_string(), v.to_string()),
            None => (line.clone(), String::new()),
        };
        let secret = is_secret_env(&key, &val);
        let masked = secret && !reveal_secrets;
        let display = if masked {
            MASKED_PLACEHOLDER.to_string()
        } else {
            val
        };
        entries.push(EnvEntry {
            key,
            value: display,
            secret,
            masked,
        });
    }
    entries.sort_by(|a, b| a.key.cmp(&b.key));
    entries
}
