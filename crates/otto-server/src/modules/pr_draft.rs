//! PR / commit drafting: the Jira-key helpers, the draft prompt + parser, the
//! `/draft-pr` and `/draft-commit` handlers, and the hosting-remote owner check.

use super::*;

/// Helper: build provider + remote ref from a repo row in ServerCtx.
///
/// S4: the repo's bound git credential may be *used* only by its owner (or root).
/// A workspace can have many members but a repo binds exactly one account, so the
/// Editor role-check on the repo is not sufficient to stop user B from opening /
/// drafting PRs through user A's hosting token — authorize the owner here.
pub(crate) async fn resolve_provider_remote(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    repo: &otto_core::domain::Repo,
) -> Result<(Arc<dyn otto_git::GitProvider>, otto_git::RemoteRef)> {
    otto_git::http::provider_ctx(ctx, &otto_core::auth::AuthUser(user.clone()), repo).await
}

/// First `[A-Z]+-[0-9]+` token in a branch name (e.g. `feature/PROJ-16232-x` →
/// `PROJ-16232`). Seeds the Jira key into commit/PR drafts. Returns `None` when the
/// branch carries no such token — we never fabricate a key.
pub(crate) fn jira_key_from_branch(branch: &str) -> Option<String> {
    let b = branch.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_uppercase() {
            let start = i;
            while i < b.len() && b[i].is_ascii_uppercase() {
                i += 1;
            }
            // …immediately followed by `-` and at least one digit.
            if i < b.len() && b[i] == b'-' {
                let mut j = i + 1;
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                if j > i + 1 {
                    return Some(branch[start..j].to_string());
                }
            }
        } else {
            i += 1;
        }
    }
    None
}

/// Guarantee the Jira `key` appears in `subject` (a PR title or a commit subject
/// line). If it's already present anywhere in the subject, return it unchanged;
/// otherwise prefix `"{key} "`. Deterministic — the drafting agent is *asked* to
/// include the key, but LLMs forget (especially under the Conventional-Commits
/// format), so this makes the reference always present, identically for PRs and
/// commits.
pub(crate) fn ensure_jira_in_subject(subject: &str, key: &str) -> String {
    if subject.contains(key) {
        subject.to_string()
    } else {
        format!("{key} {subject}")
    }
}

/// Apply [`ensure_jira_in_subject`] to the FIRST LINE of a (possibly multi-line)
/// commit message, leaving the body untouched.
pub(crate) fn ensure_jira_in_commit(message: &str, key: &str) -> String {
    match message.split_once('\n') {
        Some((subject, rest)) => format!("{}\n{}", ensure_jira_in_subject(subject, key), rest),
        None => ensure_jira_in_subject(message, key),
    }
}

/// Prepend an installed skill (its body + `references/`, via `resolve_skill_inline`)
/// ahead of a built-in draft prompt — the same shape `run_review_core` uses to
/// inline its review lenses, so the method travels in the prompt on any provider.
/// An empty skill string ⇒ the prompt is returned byte-for-byte unchanged, so an
/// un-installed skill is a no-op (identical to the prior drafting behaviour).
pub(crate) fn compose_draft_prompt(skill_text: &str, base_prompt: &str) -> String {
    if skill_text.is_empty() {
        base_prompt.to_string()
    } else {
        format!("{skill_text}\n\n---\n\n{base_prompt}")
    }
}

/// Tolerantly pull `{title, description}` out of an agent reply (which may wrap
/// the JSON in prose or a markdown fence). Falls back to using the branch name
/// as the title and the whole reply as the description.
pub(crate) fn parse_pr_draft(text: &str, fallback_title: &str) -> (String, String) {
    if let (Some(s), Some(e)) = (text.find('{'), text.rfind('}')) {
        if e > s {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text[s..=e]) {
                let title = v.get("title").and_then(|x| x.as_str()).unwrap_or("").trim();
                let desc = v
                    .get("description")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .trim();
                if !title.is_empty() || !desc.is_empty() {
                    let title = if title.is_empty() {
                        fallback_title
                    } else {
                        title
                    };
                    return (title.to_string(), desc.to_string());
                }
            }
        }
    }
    (fallback_title.to_string(), text.trim().to_string())
}

/// `POST /repos/{id}/pr/draft` — draft a PR title + description from the current
/// branch's diff against `base`, using the configured default agent CLI.
pub(crate) async fn draft_pr(
    Path(repo_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<otto_core::api::DraftPrReq>,
) -> crate::error::ApiResult<Json<otto_core::api::DraftPrResp>> {
    let repo = ctx
        .git_store
        .get_repo(&repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Editor).await?;

    // Empty base ⇒ detect the repo's default branch instead of erroring.
    let want = Some(body.base.as_str()).filter(|s| !s.trim().is_empty());
    let ws = ctx
        .workspaces
        .get(&repo.workspace_id)
        .await
        .map_err(crate::error::ApiError)?;
    // `head` = the branch the PR is FOR (the modal's Source). Absent ⇒ the
    // checked-out branch, as before. A named head is diffed as
    // merge-base(base, head)..head, so drafting a PR for a branch that isn't
    // checked out describes THAT branch, not whatever HEAD is.
    let head = body
        .head
        .as_deref()
        .map(str::trim)
        .filter(|h| !h.is_empty());
    let resp = draft_pr_core_for(&ctx, &ws, &user, &repo.path, want, head, None)
        .await
        .map_err(crate::error::ApiError)?;
    Ok(Json(resp))
}

/// Core PR-draft logic shared by the `POST /repos/{id}/pr/draft` handler and the
/// Run with Otto engine: diff the checkout at `repo_path` against `base`, draft a
/// title + description with the `pull-request` skill, and return them. The caller
/// owns auth/HTTP concerns. `repo_path` may be a worktree (Run with Otto passes
/// the run's `otto-run/<id>` worktree). `base: None` (or a ref that doesn't
/// exist here) falls back to the repo's detected default branch — never a
/// fabricated `main`, so a master-/develop-based repo can't exit 128.
pub(crate) async fn draft_pr_core(
    ctx: &ServerCtx,
    ws: &otto_core::domain::Workspace,
    user: &otto_core::domain::User,
    repo_path: &str,
    base: Option<&str>,
    run_id: Option<&Id>,
) -> Result<otto_core::api::DraftPrResp> {
    draft_pr_core_for(ctx, ws, user, repo_path, base, None, run_id).await
}

/// [`draft_pr_core`] for an explicit source branch `head` (`None` ⇒ the
/// checkout's current branch). The reply's `source_branch` is `head`.
/// `run_id` (Run with Otto) is stamped into the drafting session's meta so a
/// run cancel can find and stop it — dropping this future does not kill a
/// manager-owned session.
pub(crate) async fn draft_pr_core_for(
    ctx: &ServerCtx,
    ws: &otto_core::domain::Workspace,
    user: &otto_core::domain::User,
    repo_path: &str,
    base: Option<&str>,
    head: Option<&str>,
    run_id: Option<&Id>,
) -> Result<otto_core::api::DraftPrResp> {
    // Runs as a REAL Otto session, not a throwaway orchestrator PTY. Drafting
    // blocks a modal for as long as it takes, and the old headless PTY gave the
    // user nothing to look at — no way to tell "thinking" from "wedged". As a
    // session it appears in Agents the moment it is created (`on_ready` fires
    // before the turn completes), so it can be opened in a pane and watched, or
    // talked to, while the modal is still spinning.
    //
    // `lean_turn` + the configured drafting model are what make it quick: the
    // prompt already carries the diff and the `pull-request` skill, so MCP
    // servers and tool round trips are pure latency (see `lean_turn_args`).
    //
    // The workflow git_pr node uses `pr_draft_prompt` + `run_node_agent` instead
    // so it can honor the node's chosen Provider/Model.
    let (prompt, source, base_branch) = pr_draft_prompt_for(ctx, repo_path, base, head).await?;
    let model = pr_draft_model(ctx).await;
    let mut meta = serde_json::json!({
        "source": "pr-draft",
        "model": model,
        "lean_turn": true,
    });
    if let Some(run_id) = run_id {
        meta["run_id"] = serde_json::json!(run_id);
    }
    let title = format!("PR draft · {source}");
    let (reply, session_id) = crate::agent_session::run_session_turn(
        ctx,
        ws,
        user,
        None,
        &title,
        repo_path,
        "claude",
        meta,
        &prompt,
        DRAFT_STUCK_AFTER,
        |_id| {},
    )
    .await
    .map_err(|e| e.0)?;
    let (mut title, description) = parse_pr_draft(&reply, &source);
    if let Some(key) = jira_key_from_branch(&source) {
        title = ensure_jira_in_subject(&title, &key);
    }
    Ok(otto_core::api::DraftPrResp {
        title,
        description,
        source_branch: source,
        target_branch: base_branch,
        session_id: Some(session_id),
    })
}

/// Idle trip for a drafting turn. Generous next to how long the turn *should*
/// take (seconds), because the session is visible: a user watching it stall is
/// better served by a live session they can inspect than by a fast error.
pub(crate) const DRAFT_STUCK_AFTER: std::time::Duration = std::time::Duration::from_secs(300);

/// The model that drafts PR titles/descriptions and commit messages, from the
/// `pr_draft_model` setting. Falls back to the fast default when unset — never
/// to the user's default agent model, which is what made drafting take minutes.
pub(crate) async fn pr_draft_model(ctx: &ServerCtx) -> String {
    let repo = otto_state::SettingsRepo::new(ctx.pool.clone());
    let value = repo
        .get(otto_state::PR_DRAFT_MODEL_KEY)
        .await
        .ok()
        .flatten();
    otto_state::pr_draft_model_from(value.as_ref())
}

/// Build the PR-draft agent prompt for the checkout at `repo_path` vs `base`
/// (diff → title/description instructions + the installed `pull-request` skill).
/// Returns `(prompt, source_branch, resolved_base_branch)`. Errors when there is
/// no diff. Shared by [`draft_pr_core`] (HTTP, claude) and the workflow `git_pr`
/// node (honors the node's provider) so both craft the message identically.
pub(crate) async fn pr_draft_prompt(
    ctx: &ServerCtx,
    repo_path: &str,
    base: Option<&str>,
) -> Result<(String, String, String)> {
    pr_draft_prompt_for(ctx, repo_path, base, None).await
}

/// [`pr_draft_prompt`] for an explicit source branch `head`: the diff is
/// `merge-base(base, head)..head` (committed work on that branch only), and
/// the prompt/`source` name `head`. `None` ⇒ the checked-out branch + its
/// working tree, exactly as before.
pub(crate) async fn pr_draft_prompt_for(
    ctx: &ServerCtx,
    repo_path: &str,
    base: Option<&str>,
    head: Option<&str>,
) -> Result<(String, String, String)> {
    let git = otto_git::LocalGit::new(repo_path);
    let source = match head {
        Some(h) => h.to_string(),
        None => git.current_branch().await?,
    };
    let resolved = git.resolve_base(base).await?;
    let base = resolved.branch.as_str();
    // Cap the diff fed to the drafting agent — a title/description doesn't need
    // every line, and a huge prompt is slow + can exceed input limits. git is
    // stopped at twice the cap (enough to know it was cut) instead of the whole
    // patch being buffered just to keep 40 KB of it.
    const MAX_DIFF: usize = 40_000;
    let (diff, _) = match head {
        Some(h) => {
            git.range_diff_text_capped(&resolved.diff_ref, h, 2 * MAX_DIFF)
                .await?
        }
        None => {
            git.diff_text_capped(Some(&resolved.diff_ref), 2 * MAX_DIFF)
                .await?
        }
    };
    if diff.trim().is_empty() {
        // An explicit head drafts from COMMITS only. When that head is the
        // checked-out branch and the work is still in the working tree, say so
        // — a bare "no changes" reads as a bug to someone looking at their
        // edits (S15-307).
        let uncommitted = match head {
            Some(h) => {
                git.current_branch().await.is_ok_and(|cur| cur == h)
                    && git.status().await.is_ok_and(|st| !st.changes.is_empty())
            }
            None => false,
        };
        return Err(Error::Invalid(no_pr_changes_message(
            &source,
            base,
            uncommitted,
        )));
    }
    let truncated = diff.len() > MAX_DIFF;
    let diff_slice = if truncated {
        let mut end = MAX_DIFF;
        while end > 0 && !diff.is_char_boundary(end) {
            end -= 1;
        }
        &diff[..end]
    } else {
        diff.as_str()
    };

    // Seed the Jira key from the branch (best-effort). It belongs in the PR
    // TITLE only — a key in the body auto-links in some clients (GitKraken) and
    // can crash them. Enrichment (issue summary) is left to the installed skill.
    let jira = match jira_key_from_branch(&source) {
        Some(key) => format!(
            "This branch is for Jira issue {key}. Use `{key}` as the PR title prefix \
             (e.g. `{key} <summary>`); do NOT put the key, a Jira link, or a Jira \
             hostname anywhere in the description body.\n"
        ),
        None => String::new(),
    };

    let base_prompt = format!(
        "You are preparing a pull request from branch `{source}` into `{base}`. Based ONLY on \
         the diff below, write:\n\
         - a concise, imperative PR title (max ~72 chars, no trailing period)\n\
         - a clear PR description in Markdown: a one-line summary, then a \"What changed\" bullet \
         list, then \"Testing\" notes if any are evident from the diff.\n\
         {jira}\
         Reply with ONLY a JSON object, no prose and no markdown fence: \
         {{\"title\": \"...\", \"description\": \"...\"}}.\n\n\
         {trunc}DIFF:\n{diff}",
        source = source,
        base = base,
        jira = jira,
        trunc = if truncated {
            "(diff truncated for brevity)\n\n"
        } else {
            ""
        },
        diff = diff_slice,
    );
    // Prepend the installed `pull-request` skill (if any). Un-installed ⇒ no-op.
    let skill_text = resolve_skill_inline(&ctx.context_library, "pull-request");
    let prompt = compose_draft_prompt(&skill_text, &base_prompt);

    Ok((prompt, source, base.to_string()))
}

/// The PR-draft "nothing to draft" error. `uncommitted`: the source is the
/// checked-out branch and its working tree has changes that are not in any
/// commit yet — the PR would carry none of them.
pub(crate) fn no_pr_changes_message(source: &str, base: &str, uncommitted: bool) -> String {
    if uncommitted {
        format!(
            "no commits between '{source}' and '{base}' — your changes are not committed yet. \
             Commit your changes first: the PR includes commits only"
        )
    } else {
        format!("no changes between '{source}' and '{base}'")
    }
}

/// Tolerantly extract a commit message from an agent reply. The agent is asked
/// to reply with ONLY the message, but it may still wrap it in a markdown fence
/// or add a stray preamble — strip a leading/trailing ``` fence and trim.
pub(crate) fn parse_commit_draft(text: &str) -> String {
    let trimmed = text.trim();
    // Strip a surrounding ```…``` fence if present (with or without a language
    // tag on the opening line).
    if let Some(rest) = trimmed.strip_prefix("```") {
        if let Some(end) = rest.rfind("```") {
            let inner = &rest[..end];
            // Drop the (possibly empty) language tag on the first line.
            let body = inner.split_once('\n').map(|(_, b)| b).unwrap_or(inner);
            return body.trim().to_string();
        }
    }
    trimmed.to_string()
}

/// `POST /repos/{id}/draft-commit-message` — draft a Conventional Commits–style
/// commit message from the STAGED diff (falls back to the full working diff when
/// nothing is staged), using the configured default agent CLI. Symmetric with
/// `draft_pr`, but scoped to what's about to be committed.
pub(crate) async fn draft_commit_message(
    Path(repo_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> crate::error::ApiResult<Json<otto_core::api::DraftCommitMessageResp>> {
    let repo = ctx
        .git_store
        .get_repo(&repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Editor).await?;

    let git = otto_git::LocalGit::new(&repo.path);
    // Prefer the staged diff (what's actually about to be committed). When the
    // index is empty, fall back to the full working diff so the button is still
    // useful before staging.
    //
    // Cap the diff fed to the drafting agent — a commit message doesn't need
    // every line, and a huge prompt is slow + can exceed input limits. git is
    // stopped at twice the cap instead of buffering the whole patch.
    const MAX_DIFF: usize = 40_000;
    let (staged, _) = git
        .staged_diff_text_capped(2 * MAX_DIFF)
        .await
        .map_err(crate::error::ApiError)?;
    let (diff, from_staged) = if staged.trim().is_empty() {
        let (working, _) = git
            .diff_text_capped(None, 2 * MAX_DIFF)
            .await
            .map_err(crate::error::ApiError)?;
        (working, false)
    } else {
        (staged, true)
    };
    if diff.trim().is_empty() {
        return Err(crate::error::ApiError(Error::Invalid(
            "nothing to commit — no staged or unstaged changes".into(),
        )));
    }
    let truncated = diff.len() > MAX_DIFF;
    let diff_slice = if truncated {
        let mut end = MAX_DIFF;
        while end > 0 && !diff.is_char_boundary(end) {
            end -= 1;
        }
        &diff[..end]
    } else {
        diff.as_str()
    };

    let scope = if from_staged {
        "the STAGED changes (what is about to be committed)"
    } else {
        "the working-tree changes (nothing is staged yet)"
    };
    // Seed the Jira key from the current branch (best-effort) so the subject
    // carries it. Enrichment (issue summary) is left to the installed skill.
    let branch = git.current_branch().await.unwrap_or_default();
    let jira = match jira_key_from_branch(&branch) {
        Some(key) => format!(
            "This change is for Jira issue {key}. Prefix the subject line with `{key}` \
             (e.g. `{key} type(scope): summary`); do NOT repeat the key in the body. "
        ),
        None => String::new(),
    };
    let base_prompt = format!(
        "You are writing a git commit message for {scope}. Based ONLY on the diff below, write:\n\
         - a Conventional Commits subject line: `type(scope): summary` (type ∈ feat, fix, docs, \
         style, refactor, perf, test, build, ci, chore; scope optional; imperative mood; \
         ≤72 chars; no trailing period)\n\
         - an optional body (blank line, then a short bullet list of WHAT changed and WHY) when \
         the change is non-trivial; omit the body for tiny changes.\n\
         Infer and honor any repo convention you can see in the diff (e.g. an emoji prefix or a \
         scope naming style). {jira}Reply with ONLY the raw commit message text — no prose, no \
         explanation, and no markdown code fence.\n\n\
         {trunc}DIFF:\n{diff}",
        scope = scope,
        jira = jira,
        trunc = if truncated {
            "(diff truncated for brevity)\n\n"
        } else {
            ""
        },
        diff = diff_slice,
    );
    // Prepend the installed `commit-message` skill (if any). Un-installed ⇒ no-op.
    let skill_text = resolve_skill_inline(&ctx.context_library, "commit-message");
    let prompt = compose_draft_prompt(&skill_text, &base_prompt);

    // Same path as `draft_pr_core`: a REAL, visible session (watchable from
    // the WIP panel while it runs) on the configured drafting model with
    // `lean_turn` — not the headless orchestrator PTY on the user's default
    // model, which was both invisible and slow.
    let ws = ctx
        .workspaces
        .get(&repo.workspace_id)
        .await
        .map_err(crate::error::ApiError)?;
    let model = pr_draft_model(&ctx).await;
    let meta = serde_json::json!({
        "source": "commit-draft",
        "model": model,
        "lean_turn": true,
    });
    let title = format!(
        "Commit draft · {}",
        if branch.trim().is_empty() {
            "HEAD"
        } else {
            branch.as_str()
        }
    );
    let (reply, session_id) = crate::agent_session::run_session_turn(
        &ctx,
        &ws,
        &user,
        None,
        &title,
        &repo.path,
        "claude",
        meta,
        &prompt,
        DRAFT_STUCK_AFTER,
        |_id| {},
    )
    .await?;
    let message = parse_commit_draft(&reply);
    // Always carry the Jira key in the subject line (the agent is asked to, but
    // forgets under the Conventional-Commits format) — same guarantee as the PR.
    let message = match jira_key_from_branch(&branch) {
        Some(key) => ensure_jira_in_commit(&message, &key),
        None => message,
    };

    Ok(Json(otto_core::api::DraftCommitMessageResp {
        message,
        from_staged,
        session_id: Some(session_id),
    }))
}

#[cfg(test)]
mod commit_pr_draft_tests {
    use super::{compose_draft_prompt, jira_key_from_branch};

    #[test]
    fn jira_key_parsed_from_branch() {
        assert_eq!(
            jira_key_from_branch("feature/PROJ-16232-rate-limit").as_deref(),
            Some("PROJ-16232")
        );
        assert_eq!(
            jira_key_from_branch("PROJ-445").as_deref(),
            Some("PROJ-445")
        );
        assert_eq!(
            jira_key_from_branch("bugfix/PROJ-7").as_deref(),
            Some("PROJ-7")
        );
        assert_eq!(jira_key_from_branch("main"), None);
        assert_eq!(jira_key_from_branch("hotfix/no-key-here"), None);
        // An uppercase run without a numeric suffix is not a key.
        assert_eq!(jira_key_from_branch("release/NOTES-final"), None);
    }

    #[test]
    fn jira_prefixed_when_missing_left_alone_when_present() {
        use super::{ensure_jira_in_commit, ensure_jira_in_subject};
        // Subject: prefix when absent, untouched when already referenced anywhere.
        assert_eq!(
            ensure_jira_in_subject("feat(bonus): add id column", "PROJ-16519"),
            "PROJ-16519 feat(bonus): add id column"
        );
        assert_eq!(
            ensure_jira_in_subject("PROJ-16519 feat: add id", "PROJ-16519"),
            "PROJ-16519 feat: add id"
        );
        assert_eq!(
            ensure_jira_in_subject("feat: add id (PROJ-16519)", "PROJ-16519"),
            "feat: add id (PROJ-16519)"
        );
        // Commit: only the subject line is touched; the body is preserved verbatim.
        assert_eq!(
            ensure_jira_in_commit("fix: thing\n\n- detail one\n- detail two", "PROJ-7"),
            "PROJ-7 fix: thing\n\n- detail one\n- detail two"
        );
        // Single-line commit message.
        assert_eq!(
            ensure_jira_in_commit("chore: bump", "AB-1"),
            "AB-1 chore: bump"
        );
    }

    #[test]
    fn compose_passthrough_when_no_skill() {
        assert_eq!(compose_draft_prompt("", "BASE PROMPT"), "BASE PROMPT");
    }

    #[test]
    fn compose_prepends_skill() {
        assert_eq!(
            compose_draft_prompt("SKILL", "BASE"),
            "SKILL\n\n---\n\nBASE"
        );
    }
}
