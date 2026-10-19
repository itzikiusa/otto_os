//! Credentials carry their approved destinations all the way to Git's prompt.
//! Git itself resolves pushRemote, pushurl and URL rewrites; checking the actual
//! prompt closes the race between a preflight config read and the network call.

use otto_core::domain::{GitAccount, GitProviderKind, Repo};
use otto_core::{Error, Result};

/// An HTTP Git credential; Debug is redacted and serialization is unavailable.
#[derive(Clone)]
pub struct TransportCredential {
    pub(crate) token: String,
    pub(crate) origins: Vec<String>,
}

impl std::fmt::Debug for TransportCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransportCredential")
            .field("origins", &self.origins)
            .finish_non_exhaustive()
    }
}

impl TransportCredential {
    /// Bind to a recorded, authorized account/repository association, never to
    /// the freshly resolved operation URL. A rewrite must not grant itself trust.
    pub fn for_repo(token: String, account: &GitAccount, repo: &Repo) -> Result<Self> {
        Self::for_remote(token, account, repo.remote_url.as_deref())
    }

    pub(crate) fn for_remote(
        token: String,
        account: &GitAccount,
        recorded_remote: Option<&str>,
    ) -> Result<Self> {
        let mut origins = vec![http_origin(account_transport_base(account))?];
        // A bound self-hosted GitLab may deliberately expose Git and its API
        // on different hosts. The recorded binding, not live Git config, is
        // the authorization for that additional transport authority.
        if account.provider == GitProviderKind::Gitlab && account.api_base_url.is_some() {
            if let Some(remote) =
                recorded_remote.filter(|u| u.starts_with("https://") || u.starts_with("http://"))
            {
                let origin = http_origin(remote)?;
                if !origins.contains(&origin) {
                    origins.push(origin);
                }
            }
        }
        Ok(Self { token, origins })
    }
}

pub(crate) fn account_transport_base(account: &GitAccount) -> &str {
    match account.provider {
        GitProviderKind::Github => "https://github.com",
        GitProviderKind::Bitbucket => "https://bitbucket.org",
        GitProviderKind::Gitlab => account
            .api_base_url
            .as_deref()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or("https://gitlab.com"),
    }
}

pub(crate) fn http_origin(value: &str) -> Result<String> {
    let url = reqwest::Url::parse(value.trim())
        .map_err(|_| Error::Invalid("invalid Git credential destination".into()))?;
    if !matches!(url.scheme(), "https" | "http") || url.host_str().is_none() {
        return Err(Error::Invalid(
            "Git credential destination must be HTTP(S)".into(),
        ));
    }
    Ok(url.origin().ascii_serialization())
}

/// Identity used to invalidate a recorded binding when origin changes. SSH
/// and HTTPS may name the same forge; custom ports remain significant.
pub(crate) fn forge_identity(remote: &str) -> Option<String> {
    let url = if remote.contains("://") {
        reqwest::Url::parse(remote).ok()?
    } else {
        let (host, path) = remote.split_once(':')?;
        reqwest::Url::parse(&format!("ssh://{host}/{path}")).ok()?
    };
    let host = url.host_str()?.to_ascii_lowercase();
    let public_ssh_alias = matches!(
        host.as_str(),
        "ssh.github.com" | "altssh.gitlab.com" | "altssh.bitbucket.org"
    );
    let host = match host.as_str() {
        "www.github.com" | "ssh.github.com" => "github.com",
        "www.gitlab.com" | "altssh.gitlab.com" => "gitlab.com",
        "www.bitbucket.org" | "altssh.bitbucket.org" => "bitbucket.org",
        _ => &host,
    };
    let port = url
        .port()
        .filter(|p| !(url.scheme() == "ssh" && (*p == 22 || (*p == 443 && public_ssh_alias))));
    Some(match port {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    })
}

/// Only matching forge identities can be adopted automatically. A deliberately
/// different SSH/API identity is accepted by explicit account binding instead.
pub(crate) fn account_matches_remote(account: &GitAccount, remote: &str) -> bool {
    matches!(
        (forge_identity(account_transport_base(account)), forge_identity(remote)),
        (Some(a), Some(b)) if a == b
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::AskPass;

    fn credential(origin: &str) -> TransportCredential {
        TransportCredential {
            token: "E16-dummy".into(),
            origins: vec![http_origin(origin).unwrap()],
        }
    }

    #[test]
    fn askpass_confines_actual_prompt_authority_and_scheme() {
        let helper = AskPass::new(&credential("https://gitlab.example:8443")).unwrap();
        for (prompt, allowed) in [
            ("Password for 'https://u@gitlab.example:8443': ", true),
            (
                "Username for 'https://gitlab.example:8443/team/app': ",
                true,
            ),
            ("Password for 'https://unrelated.example:8443': ", false),
            ("Password for 'http://gitlab.example:8443': ", false),
            ("Password for 'https://gitlab.example': ", false),
            ("Password for 'https://gitlab.example.evil:8443': ", false),
            (
                "Password for 'https://gitlab.example:8443@evil.example': ",
                false,
            ),
            ("Enter passphrase for key '/tmp/key': ", false),
        ] {
            let envs = helper.envs();
            let script = envs.iter().find(|(k, _)| k == "GIT_ASKPASS").unwrap();
            let out = std::process::Command::new(&script.1)
                .arg(prompt)
                .envs(envs)
                .output()
                .unwrap();
            assert_eq!(out.status.success(), allowed, "{prompt}");
            if !allowed {
                assert!(out.stdout.is_empty(), "denied prompt leaked a credential");
            } else if prompt.starts_with("Password") {
                assert_eq!(out.stdout, b"E16-dummy\n");
            } else {
                assert_eq!(out.stdout, b"x-token-auth\n");
            }
        }
    }

    #[test]
    fn forge_identity_preserves_public_ssh_aliases_and_custom_ports() {
        assert_eq!(
            forge_identity("git@github.com:o/r"),
            forge_identity("https://github.com/o/r")
        );
        assert_eq!(
            forge_identity("ssh://git@ssh.github.com:443/o/r"),
            forge_identity("https://github.com/o/r")
        );
        assert_ne!(
            forge_identity("https://gitlab.example:8443/o/r"),
            forge_identity("https://gitlab.example/o/r")
        );
    }
}
