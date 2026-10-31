//! The `git_pr` workflow node, moved verbatim out of
//! `execute_node`'s match (see `nodes/mod.rs`).

use super::super::*;
use super::NodeCall;

pub(in crate::workflow_engine) async fn execute(
    call: NodeCall<'_>,
) -> Result<(Value, Vec<String>)> {
    let NodeCall {
        ctx,
        ws,
        user,
        node,
        input,
        env,
        session_tx,
        log_tx,
        activity_tx,
        ..
    } = call;
    let run_cwd: &str = env.run_cwd.as_str();
    let p = &node.params;
    let mut logs: Vec<String> = Vec::new();
    let open = p.get("open").and_then(Value::as_bool).unwrap_or(false);
    // The run-level worktree (where the work happened) + base, used when a
    // target doesn't carry its own.
    let run_worktree = input
        .get("worktree")
        .and_then(Value::as_str)
        .or_else(|| input.get("worktree_path").and_then(Value::as_str))
        .or_else(|| input.get("working_directory").and_then(Value::as_str))
        .filter(|s| !s.trim().is_empty())
        .map(expand_tilde)
        .unwrap_or_else(|| run_cwd.to_string());
    let run_input_base: Option<String> = input
        .get("base")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| env.run_base.clone());

    // Resolve (repo_id, worktree, base) per target, deduped by repo_id.
    // Targets come from the implementer/reviewer reference (input) or
    // explicit `params.repos`; when neither names one, the run-level
    // repos registry (the declared branches/worktrees) supplies them.
    // With `detect_changed: true`, ALSO consider every registered repo
    // — the per-repo draft below keeps only the ones that actually
    // changed, so the run opens a PR for exactly what the implementer
    // touched, across repositories. A `None` base ⇒ draft_pr_core
    // resolves that repo's default branch.
    let mut targets = collect_pr_targets(p, &input);
    if targets.is_empty() {
        let declared = env.files.repos();
        // Same guard as review_run: every declaration errored → fail
        // with the reasons rather than silently PR-ing the run cwd.
        if let Some(msg) = crate::workflow_context::all_declared_errored(&declared) {
            return Err(otto_core::Error::Invalid(format!(
                "git_pr: no declared repos[] entry resolved — {msg}"
            )));
        }
        targets = declared
            .iter()
            .filter(|e| e.error.is_none() && e.repo_id.is_some())
            .map(crate::workflow_context::entry_to_target)
            .collect();
    }
    if p.get("detect_changed")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        if let Ok(repos) = ctx.git_store.list_repos(&ws.id).await {
            for r in repos {
                targets.push(json!({ "repo_id": r.id, "worktree": r.path }));
            }
        }
    }
    let mut resolved: Vec<(String, String, Option<String>)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = Default::default();
    let mut notes: Vec<String> = Vec::new();
    if targets.is_empty() {
        // One implicit target from the run context (the common case).
        let repo_id = resolve_step_repo_id(ctx, ws, p, &input, run_cwd)
            .await?
            .ok_or_else(|| {
                otto_core::Error::Invalid(
                    "git_pr: no repo_id; pass repo_id or a working_directory/worktree_path \
                         under a registered repo"
                        .into(),
                )
            })?;
        let base = p
            .get("base")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| run_input_base.clone());
        seen.insert(repo_id.clone());
        resolved.push((repo_id, run_worktree.clone(), base));
    } else {
        for t in &targets {
            let worktree = t
                .get("worktree")
                .and_then(Value::as_str)
                .or_else(|| t.get("worktree_path").and_then(Value::as_str))
                .filter(|s| !s.trim().is_empty())
                .map(expand_tilde)
                .unwrap_or_else(|| run_worktree.clone());
            let base = t
                .get("base")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or_else(|| run_input_base.clone());
            let rid = match t
                .get("repo_id")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
            {
                Some(r) => Some(r.to_string()),
                None => resolve_repo_id_for_path(ctx, &ws.id, &worktree).await,
            };
            match rid {
                Some(r) if seen.insert(r.clone()) => resolved.push((r, worktree, base)),
                Some(_) => {} // duplicate repo across targets — one PR each
                None => notes.push(format!(
                    "skipped a target (no registered repo for {worktree})"
                )),
            }
        }
        if resolved.is_empty() {
            return Err(otto_core::Error::Invalid(
                "git_pr: no registered repo resolved for any target".into(),
            ));
        }
    }

    // Draft (+ optionally open) a PR per resolved repo. A per-repo failure
    // (no diff / open error) becomes a note + a skipped entry, so one bad
    // repo never sinks the others.
    let mut prs: Vec<Value> = Vec::new();
    let mut opened_n: u64 = 0;
    for (repo_id, worktree, base) in &resolved {
        let repo = match ctx.git_store.get_repo(repo_id).await {
            // S3-302: any registered repo (a target's repo_id may come
            // from run input; ensure_run_location vetted it at start).
            Ok(r) => r,
            Err(e) => {
                notes.push(format!("{repo_id}: repo missing: {e}"));
                continue;
            }
        };
        let wt = if worktree.trim().is_empty() {
            repo.path.clone()
        } else {
            worktree.clone()
        };
        // FIRST check before any PR: agents are told to commit, but a
        // stalled/stuck one leaves its work uncommitted — the branch then
        // has no commits ahead of base and the provider rejects the PR
        // ("no changes to be pulled"). Sweep leftovers into a commit, but
        // ONLY in a dedicated worktree — never a user's main checkout.
        // Otto's own commit / push skips hooks that live inside the work
        // tree (husky: an agent there can edit them). Say so on the run
        // instead of letting a commit-msg / pre-push rule surface in CI.
        let mut hooks_note = otto_git::LocalGit::new(&wt)
            .worktree_hooks_skipped()
            .await
            .then(|| skipped_hooks_note(&repo.name));
        if wt != repo.path {
            match otto_git::LocalGit::new(&wt)
                .commit_all_if_dirty(&format!("chore: commit workflow changes ({})", env.wf_name))
                .await
            {
                Ok(Some(sha)) => {
                    logs.push(format!(
                        "git_pr: committed leftover worktree changes on {} ({})",
                        repo.name,
                        &sha[..sha.len().min(9)]
                    ));
                    logs.extend(hooks_note.take());
                }
                Ok(None) => {}
                Err(e) => notes.push(format!(
                    "{}: leftover-changes commit failed: {e}",
                    repo.name
                )),
            }
        }
        // Draft the PR message with the NODE's chosen provider/model (the
        // "Open PR" node crafts title+description with an agent). Build the
        // same prompt the HTTP draft uses, then run it through the node's
        // provider instead of the claude-only orchestrator.
        let draft = match crate::modules::pr_draft_prompt(ctx, &wt, base.as_deref()).await {
            Ok((draft_prompt, source, base_branch)) => {
                let provider = p
                    .get("provider")
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or(env.default_provider.as_str());
                let model = p.get("model").and_then(Value::as_str);
                match run_node_agent(
                    ctx,
                    ws,
                    user,
                    node,
                    &env.wf_name,
                    &env.run_id,
                    provider,
                    model,
                    &draft_prompt,
                    &wt,
                    None,
                    session_tx,
                    log_tx,
                    activity_tx,
                )
                .await
                {
                    Ok((reply, _sid)) => {
                        let (mut title, description) =
                            crate::modules::parse_pr_draft(&reply, &source);
                        if let Some(key) = crate::modules::jira_key_from_branch(&source) {
                            title = crate::modules::ensure_jira_in_subject(&title, &key);
                        }
                        otto_core::api::DraftPrResp {
                            title,
                            description,
                            source_branch: source,
                            target_branch: base_branch,
                            // The workflow node runs the turn itself (honoring the node's
                            // provider/model), so there is no drafting session to link to.
                            session_id: None,
                        }
                    }
                    Err(e) => {
                        notes.push(format!("{}: PR draft agent failed ({e})", repo.name));
                        continue;
                    }
                }
            }
            Err(e) => {
                notes.push(format!("{}: nothing to PR ({e})", repo.name));
                continue;
            }
        };
        if !open {
            prs.push(json!({
                "repo_id": repo_id, "repo": repo.name, "title": draft.title,
                "description": draft.description, "source_branch": draft.source_branch,
                "target_branch": draft.target_branch, "opened": false,
            }));
            logs.push(format!(
                "git_pr: drafted '{}' on {} ({} → {})",
                draft.title, repo.name, draft.source_branch, draft.target_branch
            ));
            continue;
        }
        // Push the source branch to the remote FIRST — `create_pr_for_repo`
        // only points the PR at an EXISTING remote head, and a workflow
        // branch lives only in the run's worktree until now (that's the
        // "source: branch not found" the provider returns otherwise). Uses
        // the repo's git-account token; mirrors run_service::open_pr. On
        // failure we note it, so the reason is visible instead of a bare
        // "branch not found" from the create call.
        let push_token = match repo.git_account_id.as_ref() {
            Some(aid) => match ctx.git_store.get_account(aid).await {
                Ok(acc) => otto_core::secrets::get_async(&ctx.secrets, &acc.token_ref)
                    .await
                    .ok()
                    .flatten()
                    .and_then(|token| {
                        otto_git::transport::TransportCredential::for_repo(token, &acc, &repo).ok()
                    }),
                Err(_) => None,
            },
            None => None,
        };
        logs.extend(hooks_note.take());
        match otto_git::LocalGit::new(&wt).push(push_token).await {
            Ok(_) => logs.push(format!(
                "git_pr: pushed {} to remote for {}",
                draft.source_branch, repo.name
            )),
            Err(e) => notes.push(format!(
                "{}: push of {} failed: {e}",
                repo.name, draft.source_branch
            )),
        }
        let req = otto_core::api::CreatePrReq {
            title: draft.title.clone(),
            description: draft.description.clone(),
            source_branch: draft.source_branch.clone(),
            target_branch: draft.target_branch.clone(),
            draft: None,
            reviewers: None,
        };
        let auth = otto_core::auth::AuthUser(user.clone());
        match otto_git::http::create_pr_for_repo(ctx, &auth, &repo, &req).await {
            Ok(summary) => {
                opened_n += 1;
                prs.push(json!({
                    "repo_id": repo_id, "repo": repo.name, "title": draft.title,
                    "source_branch": draft.source_branch, "target_branch": draft.target_branch,
                    "opened": true, "number": summary.number, "url": summary.url,
                }));
                logs.push(format!(
                    "git_pr: opened PR #{} '{}' on {} ({} → {})",
                    summary.number,
                    draft.title,
                    repo.name,
                    draft.source_branch,
                    draft.target_branch
                ));
            }
            Err(e) => {
                notes.push(format!("{}: open failed: {e}", repo.name));
                // R15: keep the agent-drafted title/description on the failure
                // entry too — so the user sees what the pull-request skill
                // prepared (and the node output isn't flagged "missing
                // title/description"). Otto still owns opening the PR.
                prs.push(json!({
                    "repo_id": repo_id, "repo": repo.name, "opened": false,
                    "error": e.to_string(),
                    "title": draft.title, "description": draft.description,
                    "source_branch": draft.source_branch, "target_branch": draft.target_branch,
                }));
            }
        }
    }
    if prs.is_empty() {
        return Err(otto_core::Error::Upstream(format!(
            "git_pr: no PR could be drafted/opened ({})",
            notes.join("; ")
        )));
    }
    // R13: the step was asked to OPEN a PR (open:true) but none opened —
    // surface the reason (e.g. "repo has no git account") as a real failure
    // instead of a misleading "success" with opened:false. A dry run
    // (open:false) legitimately opens nothing, so it's exempt.
    require_opened(open, opened_n, &prs)?;
    // Back-compat: mirror the primary (first) PR's fields at the top level,
    // alongside the full `prs[]` and an `opened` flag/count.
    let mut outm = prs[0].as_object().cloned().unwrap_or_default();
    outm.insert("prs".into(), Value::Array(prs.clone()));
    outm.insert("opened".into(), json!(open && opened_n > 0));
    outm.insert("opened_count".into(), json!(opened_n));
    outm.insert(
        "repos".into(),
        json!(resolved
            .iter()
            .map(|(r, _, _)| r.clone())
            .collect::<Vec<_>>()),
    );
    if !notes.is_empty() {
        outm.insert("notes".into(), json!(notes));
    }
    Ok((Value::Object(outm), logs))
}

/// R13: a step asked to OPEN a PR (`open: true`) that opened none, with an
/// open error on any repo, fails with those errors instead of reporting
/// success with `opened: false`. A dry run (`open: false`) is exempt.
pub(in crate::workflow_engine) fn require_opened(
    open: bool,
    opened_n: u64,
    prs: &[Value],
) -> Result<()> {
    if open && opened_n == 0 {
        let errs: Vec<String> = prs
            .iter()
            .filter_map(|pr| pr.get("error").and_then(Value::as_str))
            .map(str::to_string)
            .collect();
        if !errs.is_empty() {
            return Err(otto_core::Error::Upstream(format!(
                "git_pr: asked to open a PR but none opened — {}",
                errs.join("; ")
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "git_pr_tests.rs"]
mod tests;
