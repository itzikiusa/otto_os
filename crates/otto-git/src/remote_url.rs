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
        let (authority, path) = match rest.split_once('/') {
            Some(p) => p,
            None => return bad("missing path"),
        };
        // `user[:pass]@host[:port]` — reject leading `-` in host (for all
        // schemes) and in userinfo (for `ssh://`) so OpenSSH cannot parse
        // user/host as CLI flags like `-oProxyCommand=...`.
        let (userinfo, host) = match authority.rsplit_once('@') {
            Some((u, h)) => (Some(u), h),
            None => (None, authority),
        };
        if scheme == "ssh" {
            if let Some(ui) = userinfo {
                let user = ui.split_once(':').map(|(u, _)| u).unwrap_or(ui);
                if user.starts_with('-') {
                    return bad("ssh user must not start with '-'");
                }
            }
        }
        if host.is_empty() {
            return bad("missing host");
        }
        if host.starts_with('-') {
            return bad("host must not start with '-'");
        }
        if path.is_empty() {
            return bad("missing path");
        }
        return Ok(());
    }
    // scp-like `user@host:path`.
    let ok_ident = |v: &str| {
        !v.is_empty()
            && !v.starts_with('-')
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

#[cfg(test)]
mod tests {
    use super::validate_remote_url;

    #[test]
    fn rejects_leading_dash_in_ssh_and_scp_user_or_host() {
        for bad in [
            "ssh://-oProxyCommand=id@example.com/org/repo.git",
            "ssh://-user:pass@example.com/org/repo.git",
            "ssh://git@-oProxyCommand=id/org/repo.git",
            "ssh://-host.example/org/repo.git",
            "https://-host.example/org/repo.git",
            "git@-oProxyCommand=id:org/repo.git",
            "-oProxyCommand=id@example.com:org/repo.git",
        ] {
            assert!(
                validate_remote_url(bad).is_err(),
                "must reject leading-dash remote url: {bad}"
            );
        }

        for good in [
            "ssh://git@my-host.example:2222/org/repo.git",
            "ssh://deploy-user@my-host.example/org/repo.git",
            "git@my-host.example:org/repo.git",
            "deploy-user@my-host.example:org/repo.git",
            "https://my-host.example/org/repo.git",
        ] {
            assert!(
                validate_remote_url(good).is_ok(),
                "must accept valid remote url: {good}"
            );
        }
    }
}
