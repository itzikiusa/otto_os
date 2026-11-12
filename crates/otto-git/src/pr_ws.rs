//! Whitespace-ignoring PR diffs for forges without a server-side switch
//! (GitHub, GitLab): when the repo's own clone already holds both commits
//! the PR's diff is computed between, the diff is computed there with `-w`.
//! Nothing is fetched — when either commit is missing the caller serves the
//! forge's normal diff and says so (`whitespace_ignored: false`).

use axum::body::Bytes;

use crate::local::{DiffOpts, DiffTarget, LocalGit};
use crate::providers::{GitProvider, RemoteRef};

/// The `-w` diff of PR `number` from `git`, with `whitespace_ignored: true`,
/// plus whether the immutable-diff memo answered (`hit`/`miss`). `None` when
/// the forge does not say which commits the PR spans, a commit is not in
/// the clone, or git fails — every one of them falls back to the forge diff.
/// Memoised under the immutable-diff key, whose mode carries the `-w` flag.
pub(crate) async fn local_ws_diff(
    git: &LocalGit,
    provider: &dyn GitProvider,
    remote: &RemoteRef,
    number: u64,
    opts: &DiffOpts,
) -> Option<(Bytes, &'static str)> {
    let target = match provider.pr_diff_target(remote, number).await {
        Ok(Some(t)) => t,
        Ok(None) => return None,
        Err(e) => {
            tracing::debug!(pr = number, "pr diff range unavailable: {e}");
            return None;
        }
    };
    let (DiffTarget::Range(a, b) | DiffTarget::MergeBase(a, b)) = &target else {
        return None;
    };
    if !git.has_commits(&[a.as_str(), b.as_str()]).await {
        return None;
    }
    let opts = DiffOpts {
        ignore_whitespace: true,
        ..opts.clone()
    };
    let (resolved, key) = crate::http::immutable_diff_key(git, &target, &opts).await?;
    if let Some(body) = crate::diff_cache::get_body(&key) {
        return Some((body, "hit"));
    }
    let mut resp = match git.diff_with(&resolved, &opts).await {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(
                pr = number,
                "local -w diff failed, serving the forge diff: {e}"
            );
            return None;
        }
    };
    resp.whitespace_ignored = Some(true);
    let body = crate::http::diff_json(resp).await.ok()?;
    crate::diff_cache::put_body(key, body.clone());
    Some((body, "miss"))
}
