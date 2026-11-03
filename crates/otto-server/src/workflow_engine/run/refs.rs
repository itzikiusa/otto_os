//! Run-start ref resolution that needs the daemon (credentials, providers):
//! sync the branches a run names with `origin` and resolve PRs named only by
//! number. The pure parts live in [`otto_workflows::run_refs`].

use super::super::*;
use otto_workflows::run_refs::RunPr;

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
    prs: &[RunPr],
) -> HashMap<String, otto_git::branch_sync::SyncReport> {
    let wanted = otto_workflows::run_refs::branches_by_repo(entries, pr_branch, prs);
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

/// Look up each PR the run names (by number) through its repo's provider,
/// acting as `user` (whose ownership of the repo's git account is enforced
/// by `resolve_provider_remote`). A failed lookup is logged and skipped — the
/// run proceeds without that PR's branches, as before.
pub(in crate::workflow_engine) async fn lookup_run_prs(
    ctx: &ServerCtx,
    user: &User,
    entries: &[crate::workflow_context::RepoEntry],
    pr_input: Option<&Value>,
) -> Vec<RunPr> {
    let spec = otto_workflows::run_refs::parse_pr_spec(pr_input);
    let mut out = Vec::new();
    for (i, number) in otto_workflows::run_refs::match_pr_targets(&spec, entries) {
        let Some(rid) = entries[i].repo_id.clone() else {
            continue;
        };
        let Ok(repo) = ctx.git_store.get_repo(&rid).await else {
            continue;
        };
        let detail = match crate::modules::resolve_provider_remote(ctx, user, &repo).await {
            Ok((provider, remote)) => provider.get_pr(&remote, number).await,
            Err(e) => Err(e),
        };
        match detail {
            Ok(d) => {
                tracing::info!(
                    repo = %repo.name,
                    "wf PR #{number}: {} → {}",
                    d.summary.source_branch,
                    d.summary.target_branch
                );
                out.push(RunPr {
                    repo_id: rid,
                    repo_path: repo.path.clone(),
                    number,
                    head: d.summary.source_branch,
                    base: d.summary.target_branch,
                    head_sha: d.summary.head_sha,
                    head_ref: None,
                })
            }
            Err(e) => tracing::warn!(repo = %repo.name, "wf PR #{number}: lookup failed: {e}"),
        }
    }
    out
}

/// Resolve each PR's head commit to a local ref (after the run's branch
/// sync): `origin/<head>` when it is the PR head, else the host's PR head ref
/// fetched into `refs/otto/pr-review/*` (fork PRs). Best-effort per PR.
pub(in crate::workflow_engine) async fn pin_pr_heads(ctx: &ServerCtx, prs: &mut [RunPr]) {
    for pr in prs.iter_mut() {
        let Ok(repo) = ctx.git_store.get_repo(&pr.repo_id).await else {
            continue;
        };
        let token = crate::modules::review::service::repo_transport_credential(ctx, &repo).await;
        match otto_git::LocalGit::new(&pr.repo_path)
            .pr_head_ref(pr.number, Some(&pr.head), pr.head_sha.as_deref(), token)
            .await
        {
            Ok(r) => pr.head_ref = Some(r),
            Err(e) => {
                tracing::warn!(repo = %repo.name, "wf PR #{}: head unavailable: {e}", pr.number)
            }
        }
    }
}
