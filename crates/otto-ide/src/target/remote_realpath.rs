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

    /// Walk `remote_path` up to its nearest existing ancestor inside
    /// `matched_root` and verify it via [`Self::confine_remote_write`].
    /// Must run before `mkdir -p -- <parent>` so missing intermediate
    /// directories cannot be created through a remote symlink escaping
    /// `matched_root`.
    pub(crate) async fn confine_remote_nearest_ancestor(
        &self,
        target: &ResolvedTarget,
        matched_root: &str,
        remote_path: &str,
    ) -> Result<String> {
        confine_remote_nearest_ancestor_with(matched_root, remote_path, |candidate| async move {
            self.confine_remote_write(target, matched_root, &candidate)
                .await
        })
        .await
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

/// Arguments for remote `mkdir -p -- <parent>` so a directory starting with `-`
/// is never interpreted as a flag.
pub(crate) fn remote_mkdir_args(parent: String) -> [String; 3] {
    ["-p".into(), "--".into(), parent]
}

/// Candidate paths to probe from `remote_path` upward until the nearest
/// existing ancestor whose parent is inside (or equal to) `matched_root`.
pub(crate) fn remote_ancestor_candidates(matched_root: &str, remote_path: &str) -> Vec<String> {
    let root = matched_root.trim_end_matches('/');
    let clean_root = if root.is_empty() && matched_root.starts_with('/') {
        "/"
    } else {
        root
    };
    let mut out = Vec::new();
    let mut cur = remote_path.trim_end_matches('/').to_string();
    if cur.is_empty() && remote_path.starts_with('/') {
        cur = "/".into();
    }
    while !cur.is_empty() && cur != clean_root {
        let Some(parent) = remote_parent(&cur) else {
            break;
        };
        out.push(cur);
        if parent == clean_root {
            break;
        }
        let parent_inside = clean_root == "/"
            || parent == clean_root
            || parent.starts_with(&format!("{clean_root}/"));
        if !parent_inside {
            break;
        }
        cur = parent;
    }
    out
}

async fn confine_remote_nearest_ancestor_with<F, Fut>(
    matched_root: &str,
    remote_path: &str,
    mut probe: F,
) -> Result<String>
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = Result<String>>,
{
    let candidates = remote_ancestor_candidates(matched_root, remote_path);
    if candidates.is_empty() {
        return probe(remote_path.to_string()).await;
    }
    let mut last_not_found = None;
    for candidate in candidates {
        match probe(candidate).await {
            Ok(real) => return Ok(real),
            Err(Error::NotFound(msg)) => last_not_found = Some(Error::NotFound(msg)),
            Err(err) => return Err(err),
        }
    }
    Err(last_not_found.unwrap_or_else(|| {
        Error::NotFound(format!(
            "no existing ancestor of '{remote_path}' under '{matched_root}'"
        ))
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::{
        check_real_remote_write, confine_remote_nearest_ancestor_with, leaf_of,
        remote_ancestor_candidates, remote_mkdir_args, REALPATH_SCRIPT,
    };
    use crate::target::remote_parent;
    use otto_core::Error;

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

    #[test]
    fn remote_mkdir_args_includes_end_of_options_marker() {
        assert_eq!(
            remote_mkdir_args("-dashdir/sub".into()),
            ["-p", "--", "-dashdir/sub"]
        );
    }

    #[test]
    fn remote_ancestor_candidates_walks_up_to_root_boundary_only() {
        assert_eq!(
            remote_ancestor_candidates("/srv/repo", "/srv/repo/a/b/file.txt"),
            vec!["/srv/repo/a/b/file.txt", "/srv/repo/a/b", "/srv/repo/a",]
        );
        assert_eq!(
            remote_ancestor_candidates("/srv/repo", "/srv/repo/file.txt"),
            vec!["/srv/repo/file.txt"]
        );
        assert_eq!(
            remote_ancestor_candidates("~/repo", "~/repo/a/b.txt"),
            vec!["~/repo/a/b.txt", "~/repo/a"]
        );
        assert!(remote_ancestor_candidates("/srv/repo", "/srv/repo").is_empty());
    }

    #[tokio::test]
    async fn nearest_ancestor_confinement_blocks_symlink_escape_before_mkdir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("workspace");
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();

        let root_str = root.to_string_lossy().into_owned();
        let target_file = format!("{root_str}/escape/new_dir/deep/file.txt");
        let probed = Arc::new(Mutex::new(Vec::new()));
        let probed_clone = probed.clone();
        let root_for_probe = root_str.clone();

        // Execute the real POSIX REALPATH_SCRIPT locally to verify ancestor confinement.
        let err = confine_remote_nearest_ancestor_with(&root_str, &target_file, move |cand| {
            let root_s = root_for_probe.clone();
            let probed = probed_clone.clone();
            async move {
                probed.lock().unwrap().push(cand.clone());
                let parent = remote_parent(&cand).unwrap();
                let out = tokio::process::Command::new("sh")
                    .args(["-c", REALPATH_SCRIPT, "sh", &root_s, &parent, &cand])
                    .output()
                    .await
                    .unwrap();
                if out.status.code() == Some(4) {
                    return Err(Error::NotFound(parent));
                }
                assert!(out.status.success(), "{out:?}");
                let stdout = String::from_utf8_lossy(&out.stdout);
                let mut lines = stdout.lines().map(str::trim);
                let real_root = lines.next().unwrap();
                let real_parent = lines.next().unwrap();
                let is_link = lines.next().unwrap() == "LINK";
                check_real_remote_write(real_root, real_parent, is_link, leaf_of(&cand))
                    .map_err(Error::Forbidden)
            }
        })
        .await
        .unwrap_err();

        assert!(
            matches!(err, Error::Forbidden(_)),
            "expected Forbidden, got {err:?}"
        );
        assert!(
            !outside.join("new_dir").exists(),
            "outside directory must not be created"
        );
        // Probed deep child -> missing intermediate -> symlink parent (caught!).
        let seen = probed.lock().unwrap().clone();
        assert_eq!(seen.len(), 3, "{seen:?}");
    }

    #[tokio::test]
    async fn nearest_ancestor_confinement_allows_missing_nested_dir_inside_real_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("workspace");
        let existing_sub = root.join("src");
        std::fs::create_dir_all(&existing_sub).unwrap();

        let root_str = root.to_string_lossy().into_owned();
        let target_file = format!("{root_str}/src/a/b/new.rs");
        let root_for_probe = root_str.clone();
        let real = confine_remote_nearest_ancestor_with(&root_str, &target_file, move |cand| {
            let root_s = root_for_probe.clone();
            async move {
                let parent = remote_parent(&cand).unwrap();
                let out = tokio::process::Command::new("sh")
                    .args(["-c", REALPATH_SCRIPT, "sh", &root_s, &parent, &cand])
                    .output()
                    .await
                    .unwrap();
                if out.status.code() == Some(4) {
                    return Err(Error::NotFound(parent));
                }
                assert!(out.status.success(), "{out:?}");
                let stdout = String::from_utf8_lossy(&out.stdout);
                let mut lines = stdout.lines().map(str::trim);
                let real_root = lines.next().unwrap();
                let real_parent = lines.next().unwrap();
                let is_link = lines.next().unwrap() == "LINK";
                check_real_remote_write(real_root, real_parent, is_link, leaf_of(&cand))
                    .map_err(Error::Forbidden)
            }
        })
        .await
        .unwrap();

        let canon_sub = existing_sub.canonicalize().unwrap();
        assert_eq!(real, format!("{}/a", canon_sub.display()));
    }
}
