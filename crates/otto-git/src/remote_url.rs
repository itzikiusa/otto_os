//! Remote-URL helpers split out of `local.rs` (LOC ratchet); re-exported
//! from `crate::local` so existing paths keep working.

use otto_core::{Error, Result};

/// Accept only URLs `git clone` / `git remote add` should ever receive:
/// `https?://`, `ssh://`, `git://` with a host and a path, or scp-like
/// `user@host:path`. Refuses empty, whitespace, control characters, a leading
/// `-` (git would read it as an option), and local paths / `file://` — cloning
/// a local path is a REGISTRATION, and the UI registers those via
/// `POST /repos {path}` instead.
pub fn validate_remote_url(url: &str) -> Result<()> {
    let bad = |why: &str| Err(Error::Invalid(format!("invalid remote url: {why}")));
    let u = url.trim();
    if u.is_empty() {
        return bad("must not be empty");
    }
    if u.starts_with('-') {
        return bad("must not start with '-'");
    }
    if u.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return bad("contains whitespace or a control character");
    }
    if let Some((scheme, rest)) = u.split_once("://") {
        if !matches!(scheme, "http" | "https" | "ssh" | "git") {
            return bad("scheme must be http, https, ssh or git");
        }
        let (host, path) = match rest.split_once('/') {
            Some(p) => p,
            None => return bad("missing path"),
        };
        // `user[:pass]@host[:port]` — only the host half has to be non-empty.
        let host = host.rsplit_once('@').map(|(_, h)| h).unwrap_or(host);
        if host.is_empty() {
            return bad("missing host");
        }
        if path.is_empty() {
            return bad("missing path");
        }
        return Ok(());
    }
    // scp-like `user@host:path`.
    let ok_ident = |v: &str| {
        !v.is_empty()
            && v.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    };
    let Some((user, rest)) = u.split_once('@') else {
        return bad("expected a URL with a scheme or a user@host:path remote");
    };
    let Some((host, path)) = rest.split_once(':') else {
        return bad("expected a URL with a scheme or a user@host:path remote");
    };
    if !ok_ident(user) || !ok_ident(host) {
        return bad("bad user or host");
    }
    if path.is_empty() || path.starts_with('-') {
        return bad("missing path");
    }
    Ok(())
}

/// Remove any `user:password@` userinfo from a URL so a credentialed remote a
/// user may have pasted isn't echoed into notices/logs. Best-effort string op;
/// returns non-URL strings unchanged. The real (credentialed) URL is still used
/// for the actual git operation — this is only for display.
pub fn strip_url_userinfo(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return url.to_string();
    };
    let after = scheme_end + 3;
    let rest = &url[after..];
    let path_start = rest.find('/').unwrap_or(rest.len());
    let authority = &rest[..path_start];
    match authority.rfind('@') {
        Some(at) => format!(
            "{}{}{}",
            &url[..after],
            &authority[at + 1..],
            &rest[path_start..]
        ),
        None => url.to_string(),
    }
}
