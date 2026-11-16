//! Real (symlink-resolved) confinement for SSH-target writes.
//!
//! `resolve_remote_in_roots` is lexical: a remote repo holding
//! `docs -> ~/.ssh` would let a save of `docs/authorized_keys` land outside
//! the root. Before an SSH write the parent directory and the root are
//! resolved ON THE REMOTE HOST (`cd -P … && pwd -P`, POSIX, no `realpath`
//! dependency), the real parent must sit inside the real root, and a final
//! component that is itself a symlink is refused.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use otto_core::{Error, Result};

use super::{remote_parent, resolve_remote_in_roots, ResolvedTarget, TargetManager};

/// Prints the real root, the real parent, then `LINK` or `PLAIN` for the
/// file itself. `~` / `~/…` arguments expand to `$HOME` (the args arrive
/// quoted, so the shell would not expand them).
const REALPATH_SCRIPT: &str = r#"x() { case $1 in "~") printf %s "$HOME";; "~/"*) printf %s "$HOME/${1#"~/"}";; *) printf %s "$1";; esac; }
R=$(x "$1"); P=$(x "$2"); F=$(x "$3")
(cd -P -- "$R" && pwd -P) || exit 3
(cd -P -- "$P" && pwd -P) || exit 4
if [ -L "$F" ]; then echo LINK; else echo PLAIN; fi"#;

impl TargetManager {
    /// Refuse an SSH write to `remote_path` (already lexically confined to
    /// `matched_root`) whose real parent escapes the real root, or whose
    /// final component is a symlink. Returns the real path written to.
    pub(crate) async fn confine_remote_write(
        &self,
        target: &ResolvedTarget,
        matched_root: &str,
        remote_path: &str,
    ) -> Result<String> {
        let (real_root, real_parent, is_link) = self
            .probe_remote_real_path(target, matched_root, remote_path)
            .await?;
        check_real_remote_write(&real_root, &real_parent, is_link, leaf_of(remote_path))
            .map_err(Error::Forbidden)
    }

    /// E2 on SSH targets: a non-root caller's write, create, rename (either
    /// end), duplicate or delete of `raw_path` is refused when its REAL
    /// remote path (parent resolved on the host; the entry itself not
    /// followed) is a git control path or project agent config. Local
    /// targets are checked inside path resolution instead.
    pub async fn refuse_remote_control_path(
        &self,
        target: &ResolvedTarget,
        raw_path: &str,
        caller_is_root: bool,
    ) -> Result<()> {
        let ResolvedTarget::Ssh {
            root,
            additional_roots,
            ..
        } = target
        else {
            return Ok(());
        };
        if caller_is_root {
            return Ok(());
        }
        let refuse =
            |p: &str| match super::control_paths::control_path_denial_at(Path::new(p), false) {
                Some(why) => Err(Error::Forbidden(why)),
                None => Ok(()),
            };
        let (lexical, matched) = resolve_remote_in_roots(root, additional_roots, raw_path)?;
        refuse(&lexical)?;
        let (_, real_parent, _) = self
            .probe_remote_real_path(target, &matched, &lexical)
            .await?;
        refuse(&format!(
            "{}/{}",
            real_parent.trim_end_matches('/'),
            leaf_of(&lexical)
        ))
    }

    /// `(real root, real parent, final component is a symlink)` as the remote
    /// host sees them. A host that can't run the probe (SFTP-only, no `sh`,
    /// exec refused) gets a distinct 502 that says so — never an unverified
    /// write, and never a misleading "forbidden".
    async fn probe_remote_real_path(
        &self,
        target: &ResolvedTarget,
        matched_root: &str,
        remote_path: &str,
    ) -> Result<(String, String, bool)> {
        let parent = remote_parent(remote_path)
            .ok_or_else(|| Error::Invalid("remote path has no parent directory".into()))?;
        let unverifiable = |why: &str| {
            Error::Upstream(format!(
                "could not verify the real path of '{remote_path}' on the remote host before \
                 saving ({why}); nothing was written. Saving over SSH needs the host to allow \
                 running `sh` (an SFTP-only account can't be verified)"
            ))
        };
        let out = self
            .exec_command(
                target,
                "sh",
                &[
                    "-c".into(),
                    REALPATH_SCRIPT.into(),
                    "sh".into(),
                    matched_root.into(),
                    parent.clone(),
                    remote_path.into(),
                ],
                None,
                &BTreeMap::new(),
                Duration::from_secs(10),
            )
            .await
            .map_err(|e| unverifiable(&e.to_string()))?;
        match out.exit_code {
            0 => {}
            3 => {
                return Err(unverifiable(&format!(
                    "the workspace root '{matched_root}' is not accessible"
                )))
            }
            4 => {
                return Err(Error::NotFound(format!(
                    "remote folder '{parent}' does not exist or is not accessible"
                )))
            }
            code => {
                let stderr = out.stderr.trim();
                return Err(unverifiable(&format!("probe exited {code}: {stderr}")));
            }
        }
        let mut lines = out.stdout.lines().map(str::trim);
        let (Some(real_root), Some(real_parent), Some(kind)) =
            (lines.next(), lines.next(), lines.next())
        else {
            return Err(unverifiable("unexpected probe output"));
        };
        Ok((
            real_root.to_string(),
            real_parent.to_string(),
            kind == "LINK",
        ))
    }
}

fn leaf_of(remote_path: &str) -> &str {
    remote_path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("")
}

/// The pure decision behind [`TargetManager::confine_remote_write`].
fn check_real_remote_write(
    real_root: &str,
    real_parent: &str,
    leaf_is_link: bool,
    leaf: &str,
) -> std::result::Result<String, String> {
    let root = real_root.trim_end_matches('/');
    let inside =
        root.is_empty() || real_parent == root || real_parent.starts_with(&format!("{root}/"));
    if !real_parent.starts_with('/') || !inside {
        return Err(format!(
            "'{real_parent}' resolves outside the workspace root '{real_root}' on the remote host"
        ));
    }
    if leaf_is_link {
        return Err(format!(
            "'{leaf}' is a symlink on the remote host; saving through it is refused"
        ));
    }
    Ok(format!("{}/{leaf}", real_parent.trim_end_matches('/')))
}

#[cfg(test)]
mod tests {
    use super::check_real_remote_write;

    #[test]
    fn a_remote_symlink_cannot_carry_a_write_out_of_the_root() {
        assert_eq!(
            check_real_remote_write("/home/u/repo", "/home/u/repo/src", false, "a.go").unwrap(),
            "/home/u/repo/src/a.go"
        );
        // `docs -> ~/.ssh`: the real parent is outside the real root.
        assert!(check_real_remote_write("/home/u/repo", "/home/u/.ssh", false, "x").is_err());
        assert!(check_real_remote_write("/home/u/repo", "/home/u/repo2", false, "x").is_err());
        // A final-component symlink is refused even inside the root.
        assert!(check_real_remote_write("/home/u/repo", "/home/u/repo", true, "link").is_err());
        assert!(check_real_remote_write("/", "/etc", false, "x").is_ok());
    }
}
