//! Run-start ref resolution: sync the branches a run names with `origin`
//! (fetch + safe fast-forward) and resolve a declared base to a commit.

use super::super::*;

/// Resolve a run's declared base to a commit: the ref as given (a local
/// branch, tag or sha), else the remote-tracking `origin/<base>`. Read-only.
pub(in crate::workflow_engine) async fn resolve_wf_base(
    git: &otto_git::LocalGit,
    base: &str,
) -> Option<String> {
    if let Ok(c) = git.rev_parse(base).await {
        return Some(c);
    }
    let remote = format!("origin/{base}");
    if let Ok(c) = git.rev_parse(&remote).await {
        return Some(c);
    }
    // Neither ref exists yet — typically a branch created on the remote after
    // the clone last fetched (a PR targeting a fresh `hotfix/*`) and the
    // run-start sync's fetch failed or didn't cover it. Fetch THAT ONE branch
    // into its remote-tracking ref and retry. Safe: it writes only
    // `refs/remotes/origin/<base>` — never the user's branches, HEAD or tree —
    // and runs non-interactively (BatchMode SSH), so it can't prompt.
    if !fetchable_branch_name(base) {
        return None;
    }
    match git.fetch_origin_branch(base, None).await {
        Ok(()) => tracing::info!("wf worktree: fetched missing base '{base}' from origin"),
        Err(err) => {
            tracing::warn!("wf worktree: fetching base '{base}' from origin failed: {err}");
            return None;
        }
    }
    git.rev_parse(&remote).await.ok()
}

/// A base name that is safe to splice into a fetch refspec: no option-like
/// prefix, no refspec/glob metacharacters, no whitespace or control chars.
/// `git` itself still validates the full ref name.
pub(in crate::workflow_engine) fn fetchable_branch_name(base: &str) -> bool {
    otto_git::branch_sync::is_syncable_branch_name(base)
}

/// Run-start ref sync: for every repo a run declares, fetch `origin` and make
/// each branch the run names — the entry's base and the PR's source branch
/// (`pr_branch`) — resolvable, fast-forwarding a stale local copy where safe
/// (see [`otto_git::branch_sync`]: never forced, never moves a checked-out
/// branch, never drops a local commit). A Slack-triggered PR run `x → y` on a
/// clone where `y` existed only on the remote used to fail before any step
/// ran; a stale local base silently cut the worktree from an old commit.
/// Best-effort: a failed fetch is logged and the run proceeds on cached refs.
/// Keyed by repo id; linked worktrees share their repo's refs, so syncing
/// the registered repo path covers every checkout of it.
pub(in crate::workflow_engine) async fn sync_run_branches(
    ctx: &ServerCtx,
    entries: &[crate::workflow_context::RepoEntry],
    pr_branch: Option<&str>,
) -> HashMap<String, otto_git::branch_sync::SyncReport> {
    let mut wanted: Vec<(String, Vec<String>)> = Vec::new();
    for e in entries.iter().filter(|e| e.error.is_none()) {
        let Some(rid) = e.repo_id.clone() else {
            continue;
        };
        let names = [e.base.as_deref(), e.source.as_deref(), pr_branch]
            .into_iter()
            .flatten()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        match wanted.iter_mut().find(|(r, _)| *r == rid) {
            Some((_, v)) => v.extend(names),
            None => wanted.push((rid, names.collect())),
        }
    }
    let mut out = HashMap::new();
    for (rid, branches) in wanted {
        let Ok(repo) = ctx.git_store.get_repo(&rid).await else {
            continue;
        };
        let token = crate::modules::review::service::repo_transport_credential(ctx, &repo).await;
        let names: Vec<&str> = branches.iter().map(String::as_str).collect();
        let report = otto_git::LocalGit::new(&repo.path)
            .fetch_and_sync_branches(&names, token)
            .await;
        for b in &report.branches {
            tracing::info!(
                repo = %repo.name,
                "wf branch sync: {} → {:?} ({})",
                b.branch,
                b.outcome,
                b.resolved.as_deref().unwrap_or("unresolved")
            );
        }
        out.insert(rid, report);
    }
    out
}
