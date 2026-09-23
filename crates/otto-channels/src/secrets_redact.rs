//! Secret redaction for text Otto relays into a chat.
//!
//! The mirror posts an agent's shell commands (the activity feed) and its final
//! reply to a Slack/Telegram channel that every member can read. Agents
//! routinely run `curl -H "Authorization: Bearer eyJ…"`, `mysql -pS3cret`,
//! `export JIRA_TOKEN=…` or `git clone https://user:pw@host/…`, so those must
//! be scrubbed before they leave the machine.
//!
//! This layers shell-aware shapes (`KEY=value` with a sensitive key, password
//! flags and their values, `Authorization`/`Bearer`/`Basic` header values,
//! mysql `-p<pw>`, `-u user:pass`, URL userinfo, PEM blocks) on top of
//! [`otto_core::redact::redact_text`]'s per-word shapes (JWTs, AWS keys,
//! emails). `keep_emails` leaves e-mail addresses intact — a final reply that
//! names a person's address is communication, not a leak; a command line is not.

const PLACEHOLDER: &str = "[redacted]";

/// Flags whose NEXT word is a secret, whatever it looks like.
const STRONG_FLAGS: &[&str] = &[
    "--password",
    "--passwd",
    "--pass",
    "--token",
    "--api-key",
    "--apikey",
    "--secret",
    "--client-secret",
    "--access-token",
    "--auth-token",
];

/// Words whose next word is a secret when it LOOKS like a token (see
/// [`token_like`]) — header names and auth schemes, which also occur in prose.
const WEAK_TRIGGERS: &[&str] = &[
    "bearer",
    "basic",
    "token",
    "authorization",
    "x-api-key",
    "api-key",
    "apikey",
    "private-token",
    "x-auth-token",
];

/// Substrings that make a `KEY=value` key sensitive (matched lower-cased).
const SENSITIVE_KEY_PARTS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "api_key",
    "apikey",
    "api-key",
    "access_key",
    "accesskey",
    "private_key",
    "credential",
    "authorization",
    "cookie",
];

/// Redact likely secrets from `text`, preserving whitespace and layout.
pub fn redact_secrets(text: &str, keep_emails: bool) -> String {
    let collapsed = collapse_pem(text);
    let mut out = String::with_capacity(collapsed.len());
    let mut word = String::new();
    let mut st = ScanState::default();
    for ch in collapsed.chars() {
        if ch.is_whitespace() {
            flush_word(&mut word, &mut out, &mut st, keep_emails);
            out.push(ch);
        } else {
            word.push(ch);
        }
    }
    flush_word(&mut word, &mut out, &mut st, keep_emails);
    out
}

#[derive(Default)]
struct ScanState {
    /// Previous word was a [`STRONG_FLAGS`] flag.
    strong: bool,
    /// Previous word was a [`WEAK_TRIGGERS`] word.
    weak: bool,
    /// Previous word was `-u` / `--user` (next may be `user:pass`).
    user_flag: bool,
    /// A `mysql`-family command appeared earlier (enables `-p<pw>`).
    saw_mysql: bool,
}

/// Quotes/brackets/punctuation that commonly wrap a token.
fn trim_wrap(w: &str) -> &str {
    w.trim_matches(|c: char| {
        matches!(
            c,
            '"' | '\'' | '`' | '(' | ')' | '[' | ']' | ',' | ';' | ':' | '<' | '>' | '{' | '}'
        )
    })
}

/// A plausible credential: ≥ 8 chars of token alphabet that also carries a
/// digit, base64 padding, or several upper-case letters mixed with lower-case
/// (so prose like "Basic authentication" / "Bearer Authentication" is left
/// alone).
fn token_like(w: &str) -> bool {
    let alphabet = w
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+' | '/' | '=' | '~'));
    let uppers = w.chars().filter(|c| c.is_ascii_uppercase()).count();
    let has_lower = w.chars().any(|c| c.is_ascii_lowercase());
    w.len() >= 8
        && alphabet
        && (w.chars().any(|c| c.is_ascii_digit()) || w.ends_with('=') || (uppers >= 2 && has_lower))
}

/// Replace `inner` — which MUST be a subslice of `word` (e.g. from
/// `trim_matches` / slicing) — with the placeholder, keeping everything around
/// it. Located by pointer offset, not by searching, so a secret that also
/// occurs earlier in the word (`-u abc:abc`) is replaced at the right spot.
fn replace_inner(word: &str, inner: &str) -> String {
    let start = (inner.as_ptr() as usize).wrapping_sub(word.as_ptr() as usize);
    if inner.is_empty() || start > word.len() || word.len() - start < inner.len() {
        return PLACEHOLDER.to_string();
    }
    format!(
        "{}{PLACEHOLDER}{}",
        &word[..start],
        &word[start + inner.len()..]
    )
}

fn flush_word(word: &mut String, out: &mut String, st: &mut ScanState, keep_emails: bool) {
    if word.is_empty() {
        return;
    }
    let w = word.as_str();
    let core = trim_wrap(w);
    let lower = core.to_ascii_lowercase();
    let (was_strong, was_weak, was_user) = (st.strong, st.weak, st.user_flag);
    st.strong = STRONG_FLAGS.contains(&lower.as_str());
    st.weak = WEAK_TRIGGERS.contains(&lower.as_str());
    st.user_flag = lower == "-u" || lower == "--user";
    let base = lower.rsplit('/').next().unwrap_or(lower.as_str());
    if base.starts_with("mysql") || base.starts_with("mariadb") {
        st.saw_mysql = true;
    }

    // A scheme word right after a header name (`Authorization: Bearer …`) is
    // itself a trigger, not the secret.
    let after_weak = was_weak && !st.weak;
    match redact_word(
        w,
        keep_emails,
        was_strong,
        after_weak,
        was_user,
        st.saw_mysql,
    ) {
        Some(r) => out.push_str(&r),
        None => out.push_str(w),
    }
    word.clear();
}

/// The redacted form of one whitespace-delimited word, or `None` to keep it.
fn redact_word(
    w: &str,
    keep_emails: bool,
    after_strong: bool,
    after_weak: bool,
    after_user: bool,
    saw_mysql: bool,
) -> Option<String> {
    let core = trim_wrap(w);
    if core.is_empty() {
        return None;
    }
    if after_strong || (after_weak && token_like(core)) {
        return Some(replace_inner(w, core));
    }
    if after_user && core.contains(':') && !core.contains("://") {
        // `-u user:pass` → keep the user, drop the password.
        let pw = &core[core.find(':')? + 1..];
        if !pw.is_empty() {
            return Some(replace_inner(w, pw));
        }
    }
    if let Some(r) = redact_key_value(w) {
        return Some(r);
    }
    if let Some(r) = redact_url_userinfo(w) {
        return Some(r);
    }
    // mysql/mariadb `-p<password>` (no space) — only once such a command was
    // seen, so `ssh -p2222` / `git log -p` elsewhere stay readable.
    if saw_mysql && core.starts_with("-p") && !core.starts_with("--") && core.len() > 2 {
        return Some(replace_inner(w, &core[2..]));
    }
    let r = otto_core::redact::redact_text(w);
    let only_email = !r.hits.is_empty() && r.hits.iter().all(|h| h.kind == "email");
    if r.hits.is_empty() || (keep_emails && only_email) {
        None
    } else {
        Some(r.value)
    }
}

/// `KEY=value` / `--password=value` with a sensitive key → `KEY=[redacted]`.
fn redact_key_value(w: &str) -> Option<String> {
    let eq = w.find('=')?;
    let key = w[..eq].trim_start_matches(['-', '"', '\'', '`']);
    let value = &w[eq + 1..];
    let value_core = value.trim_matches(|c: char| matches!(c, '"' | '\'' | ';' | ','));
    if key.is_empty() || value_core.is_empty() {
        return None;
    }
    let key_lower = key.to_ascii_lowercase();
    if !SENSITIVE_KEY_PARTS.iter().any(|p| key_lower.contains(p)) {
        return None;
    }
    Some(format!("{}={}", &w[..eq], replace_inner(value, value_core)))
}

/// `scheme://user:pass@host…` → `scheme://user:[redacted]@host…`.
fn redact_url_userinfo(w: &str) -> Option<String> {
    let scheme_end = w.find("://")? + 3;
    let rest = &w[scheme_end..];
    let at = rest.find('@')?;
    let userinfo = &rest[..at];
    if userinfo.contains('/') {
        return None; // the '@' is in the path, not the authority
    }
    let colon = userinfo.find(':')?;
    if colon + 1 >= userinfo.len() {
        return None;
    }
    Some(format!(
        "{}{}:{PLACEHOLDER}{}",
        &w[..scheme_end],
        &userinfo[..colon],
        &rest[at..]
    ))
}

/// Collapse `-----BEGIN …----- … -----END …-----` blocks (keys, certs).
fn collapse_pem(input: &str) -> String {
    let mut s = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("-----BEGIN") {
        s.push_str(&rest[..start]);
        let tail = &rest[start..];
        match tail.find("-----END") {
            Some(end_rel) => {
                let after_end = &tail[end_rel..];
                let close = after_end
                    .find('\n')
                    .map(|n| end_rel + n)
                    .unwrap_or(tail.len());
                s.push_str(PLACEHOLDER);
                rest = &tail[close..];
            }
            None => {
                s.push_str(tail);
                rest = "";
            }
        }
    }
    s.push_str(rest);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(s: &str) -> String {
        redact_secrets(s, false)
    }

    #[test]
    fn prose_and_plain_commands_are_untouched() {
        for s in [
            "cargo test -p otto-channels --lib",
            "git log -p --stat",
            "ssh -p 2222 host",
            "Basic authentication is disabled; the token expired.",
            "ls -la ~/code && echo done",
        ] {
            assert_eq!(cmd(s), s);
        }
    }

    #[test]
    fn authorization_headers_are_redacted() {
        let out = cmd(r#"curl -H "Authorization: Bearer abc123def456ghi" https://api.x.io"#);
        assert!(!out.contains("abc123def456ghi"), "{out}");
        assert!(out.contains("Authorization: Bearer [redacted]\""), "{out}");
        let out = cmd("curl -H 'Authorization: Basic dXNlcjpwYXNzMTIz' x");
        assert!(!out.contains("dXNlcjpwYXNzMTIz"), "{out}");
        let out = cmd("curl -H 'PRIVATE-TOKEN: glpat-1234abcd5678' x");
        assert!(!out.contains("glpat-1234abcd5678"), "{out}");
    }

    #[test]
    fn env_assignments_and_password_flags_are_redacted() {
        assert_eq!(
            cmd("export JIRA_TOKEN=abc123 && PGPASSWORD='s3cret' psql"),
            "export JIRA_TOKEN=[redacted] && PGPASSWORD='[redacted]' psql"
        );
        assert_eq!(
            cmd("tool --password hunter2 --token=xyz789 --verbose"),
            "tool --password [redacted] --token=[redacted] --verbose"
        );
        assert_eq!(
            cmd("mysql -uroot -pS3cret db"),
            "mysql -uroot -p[redacted] db"
        );
        assert_eq!(
            cmd("curl -u admin:hunter2 https://h"),
            "curl -u admin:[redacted] https://h"
        );
        // The secret is replaced where it IS, even if the same text occurs
        // earlier in the word.
        assert_eq!(cmd("curl -u abc:abc x"), "curl -u abc:[redacted] x");
        assert_eq!(cmd("mysql -pp"), "mysql -p[redacted]");
    }

    #[test]
    fn url_userinfo_is_redacted() {
        assert_eq!(
            cmd("git clone https://bob:ghp_abc123@github.com/o/r.git"),
            "git clone https://bob:[redacted]@github.com/o/r.git"
        );
        // No password part → untouched.
        assert_eq!(
            cmd("git clone ssh://git@github.com/o/r.git"),
            "git clone ssh://git@github.com/o/r.git"
        );
    }

    #[test]
    fn core_shapes_and_pem_are_redacted() {
        let out = cmd("echo eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.sig AKIAIOSFODNN7EXAMPLE");
        assert!(!out.contains("eyJhbGci"), "{out}");
        assert!(!out.contains("AKIAIOSFODNN7EXAMPLE"), "{out}");
        let out = cmd("cat <<EOF\n-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNz\n-----END OPENSSH PRIVATE KEY-----\nEOF");
        assert!(!out.contains("b3BlbnNz"), "{out}");
        assert!(out.ends_with("\nEOF"), "{out}");
    }

    #[test]
    fn emails_kept_only_when_asked() {
        let s = "ask jane.doe@example.com about it";
        assert_eq!(redact_secrets(s, true), s);
        assert!(!redact_secrets(s, false).contains("jane.doe@example.com"));
    }
}
