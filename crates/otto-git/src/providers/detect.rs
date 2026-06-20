//! Remote-URL → (provider kind, owner/repo) detection for the three known
//! hosts, handling https and ssh (scp-like and ssh://) URL forms.

use otto_core::domain::GitProviderKind;

use super::RemoteRef;

/// Detect the hosting provider and owner/repo from a git remote URL.
/// Returns None for unknown hosts or unparseable URLs.
pub fn detect(remote_url: &str) -> Option<(GitProviderKind, RemoteRef)> {
    let url = remote_url.trim();
    let (host, path) = split_host_path(url)?;
    let kind = match host.to_ascii_lowercase().as_str() {
        "github.com" | "www.github.com" => GitProviderKind::Github,
        "bitbucket.org" | "www.bitbucket.org" => GitProviderKind::Bitbucket,
        "gitlab.com" | "www.gitlab.com" => GitProviderKind::Gitlab,
        h if h.contains("gitlab") => GitProviderKind::Gitlab, // self-hosted gitlab.<corp>.com
        h if h.contains("github") => GitProviderKind::Github, // GitHub Enterprise (github.corp.com)
        _ => return None,
    };

    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() < 2 {
        return None;
    }
    let (owner, repo) = match kind {
        // GitLab supports nested groups: a/b/c → owner "a/b", repo "c".
        GitProviderKind::Gitlab => (
            segments[..segments.len() - 1].join("/"),
            segments[segments.len() - 1].to_string(),
        ),
        // GitHub/Bitbucket are strictly owner/repo.
        _ => (segments[0].to_string(), segments[1].to_string()),
    };
    Some((kind, RemoteRef { owner, repo }))
}

/// Split a remote URL into (host, path) for https://, ssh:// and scp-like
/// (git@host:path) forms.
fn split_host_path(url: &str) -> Option<(String, String)> {
    // https://host/path or http://host/path or ssh://[user@]host[:port]/path
    for scheme in ["https://", "http://", "ssh://", "git://"] {
        if let Some(rest) = url.strip_prefix(scheme) {
            let rest = rest.split_once('@').map_or(rest, |(_, r)| r);
            let (hostport, path) = rest.split_once('/')?;
            let host = hostport.split(':').next()?.to_string();
            return Some((host, path.to_string()));
        }
    }
    // scp-like: [user@]host:path (no scheme)
    if let Some((left, path)) = url.split_once(':') {
        if !left.contains('/') && !path.starts_with("//") {
            let host = left.split_once('@').map_or(left, |(_, h)| h).to_string();
            return Some((host, path.to_string()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::domain::GitProviderKind::*;

    fn rr(owner: &str, repo: &str) -> RemoteRef {
        RemoteRef {
            owner: owner.into(),
            repo: repo.into(),
        }
    }

    #[test]
    fn github_forms() {
        assert_eq!(
            detect("https://github.com/octo/hello.git"),
            Some((Github, rr("octo", "hello")))
        );
        assert_eq!(
            detect("https://github.com/octo/hello"),
            Some((Github, rr("octo", "hello")))
        );
        assert_eq!(
            detect("https://github.com/octo/hello/"),
            Some((Github, rr("octo", "hello")))
        );
        assert_eq!(
            detect("git@github.com:octo/hello.git"),
            Some((Github, rr("octo", "hello")))
        );
        assert_eq!(
            detect("ssh://git@github.com/octo/hello.git"),
            Some((Github, rr("octo", "hello")))
        );
    }

    #[test]
    fn bitbucket_forms() {
        assert_eq!(
            detect("https://bitbucket.org/team/proj"),
            Some((Bitbucket, rr("team", "proj")))
        );
        assert_eq!(
            detect("https://user@bitbucket.org/team/proj.git"),
            Some((Bitbucket, rr("team", "proj")))
        );
        assert_eq!(
            detect("git@bitbucket.org:team/proj.git"),
            Some((Bitbucket, rr("team", "proj")))
        );
    }

    #[test]
    fn gitlab_forms_and_nested_groups() {
        assert_eq!(
            detect("https://gitlab.com/o/r"),
            Some((Gitlab, rr("o", "r")))
        );
        assert_eq!(
            detect("https://gitlab.com/o/g/r.git"),
            Some((Gitlab, rr("o/g", "r")))
        );
        assert_eq!(
            detect("git@gitlab.com:o/g/sub/r.git"),
            Some((Gitlab, rr("o/g/sub", "r")))
        );
        // self-hosted gitlab host heuristic
        assert_eq!(
            detect("https://gitlab.corp.example.com/team/app.git"),
            Some((Gitlab, rr("team", "app")))
        );
    }

    #[test]
    fn unknown_and_garbage() {
        assert_eq!(detect("https://example.com/o/r.git"), None);
        assert_eq!(detect("not a url"), None);
        assert_eq!(detect("https://github.com/only-owner"), None);
        assert_eq!(detect(""), None);
    }

    #[test]
    fn github_enterprise() {
        assert_eq!(
            detect("https://github.corp.example.com/octo/hello.git"),
            Some((Github, rr("octo", "hello")))
        );
        assert_eq!(
            detect("git@github.myco.net:team/app.git"),
            Some((Github, rr("team", "app")))
        );
        assert_eq!(
            detect("https://github.internal/org/repo"),
            Some((Github, rr("org", "repo")))
        );
    }
}
