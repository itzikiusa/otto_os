//! Run with Otto — source adapters + repo resolution.
//!
//! Each of the eight entry points is normalized into a single
//! [`ResolvedSource`] (title + body + derived goal). The agent never sees the raw
//! source plumbing — only the assembled Context Packet built from this.
//!
//! Repo resolution ([`resolve_repo`]) picks the **registered git repo** the run
//! works in (a run always works in a repo, never a raw workspace root): explicit
//! `repo_id` → source-implied repo → the workspace's single/first registered repo
//! → a clear error. See design §20.1.

use std::sync::Arc;

use otto_core::domain::Repo;
use otto_core::redact::redact_text;
use otto_core::run::{OttoRun, ResolvedSource, SourceKind};
use otto_core::{Error, Id, Result};
use otto_git::{GitProvider, RemoteRef};
use serde_json::json;

use crate::state::ServerCtx;

const BODY_CAP: usize = 12_000;

/// Resolve the registered git repo the run will work in. Whatever picked it
/// (an explicit `repo_id`, a finding id, …), the repo MUST belong to the run's
/// workspace: launch only requires Editor on THAT workspace, so a repo id from
/// another workspace would hand its checkout to an agent the caller has no
/// role over. Out-of-workspace ⇒ `NotFound` (no existence leak).
pub(crate) async fn resolve_repo(
    ctx: &ServerCtx,
    workspace_id: &Id,
    created_by: &str,
    explicit_repo_id: Option<&str>,
    source_kind: SourceKind,
    source_ref: &str,
) -> Result<Repo> {
    let repo =
        resolve_repo_any(ctx, workspace_id, explicit_repo_id, source_kind, source_ref).await?;
    if repo.workspace_id != *workspace_id {
        // Only an explicit repo can come from another workspace; re-check it
        // with the launcher's role there, as at launch.
        crate::run_service::check_repo_for_run(ctx, workspace_id, created_by, &repo.id).await?;
    }
    Ok(repo)
}

async fn resolve_repo_any(
    ctx: &ServerCtx,
    workspace_id: &Id,
    explicit_repo_id: Option<&str>,
    source_kind: SourceKind,
    source_ref: &str,
) -> Result<Repo> {
    // 1. Explicit wins.
    if let Some(rid) = explicit_repo_id.filter(|s| !s.is_empty()) {
        return ctx.git_store.get_repo(&rid.to_string()).await;
    }

    // 2. Source-implied.
    match source_kind {
        SourceKind::GithubPr | SourceKind::GithubIssue => {
            if let Some((owner_repo, _)) = source_ref.split_once('#') {
                let repos = ctx.git_store.list_repos(workspace_id).await?;
                if let Some(r) = repos.into_iter().find(|r| {
                    r.remote_url
                        .as_deref()
                        .is_some_and(|u| u.contains(owner_repo))
                }) {
                    return Ok(r);
                }
            }
        }
        SourceKind::Finding => {
            if let Ok(f) = ctx.findings_store.get_full(source_ref).await {
                if !f.repo_id.is_empty() {
                    if let Ok(r) = ctx.git_store.get_repo(&f.repo_id).await {
                        return Ok(r);
                    }
                }
            }
        }
        _ => {}
    }

    // 3. The workspace's registered repos (single → use it; many → first/newest).
    let mut repos = ctx.git_store.list_repos(workspace_id).await?;
    if repos.is_empty() {
        return Err(Error::Invalid(
            "no git repo registered in this workspace; register one or pass repo_id".into(),
        ));
    }
    Ok(repos.remove(0))
}

/// Build a `(provider, remote)` pair for the repo without an owner-auth check
/// (the run engine is a trusted, autonomous context — unlike a user-facing route).
pub(crate) async fn provider_for_repo(
    ctx: &ServerCtx,
    repo: &Repo,
) -> Result<(Arc<dyn GitProvider>, RemoteRef)> {
    repo.provider
        .ok_or_else(|| Error::Invalid("repo has no git provider".into()))?;
    let account_id = repo
        .git_account_id
        .as_ref()
        .ok_or_else(|| Error::Invalid("repo has no git account".into()))?;
    let account = ctx.git_store.get_account(account_id).await?;
    let remote_url = repo
        .remote_url
        .as_deref()
        .ok_or_else(|| Error::Invalid("repo has no remote url".into()))?;
    let (_, remote_ref) = otto_git::detect(remote_url)
        .ok_or_else(|| Error::Invalid(format!("unsupported remote: {remote_url}")))?;
    let token = otto_core::secrets::get_async(&ctx.secrets, &account.token_ref)
        .await?
        .ok_or_else(|| Error::Invalid(format!("token missing for git account {}", account.id)))?;
    Ok((otto_git::make_provider(&account, token), remote_ref))
}

/// Fetch + normalize the run's source into a [`ResolvedSource`]. `repo` is the
/// already-resolved working repo (used by the GitHub adapters).
pub(crate) async fn resolve_source(
    ctx: &ServerCtx,
    run: &OttoRun,
    repo: &Repo,
) -> Result<ResolvedSource> {
    match run.source_kind {
        SourceKind::Jira => resolve_jira(ctx, run).await,
        SourceKind::Confluence => resolve_confluence(ctx, run).await,
        SourceKind::GithubPr => resolve_github_pr(ctx, run, repo).await,
        SourceKind::GithubIssue => resolve_github_issue(ctx, run, repo).await,
        SourceKind::Channel => Ok(resolve_channel(run)),
        SourceKind::ProductStory => resolve_product_story(ctx, run).await,
        SourceKind::Finding => resolve_finding(ctx, run).await,
        SourceKind::Test => resolve_test(ctx, run).await,
        SourceKind::ScheduledReport => resolve_scheduled_report(ctx, run).await,
    }
}

fn clean(s: &str, cap: usize) -> String {
    let red = redact_text(s).value;
    if red.len() <= cap {
        red
    } else {
        let mut end = cap;
        while end > 0 && !red.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}\n…(truncated)", &red[..end])
    }
}

/// Crude XHTML/HTML tag strip for Confluence storage format (keeps the text).
fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// The issue accounts a run may read its source with, in the order to try
/// them: the run creator's own (oldest first), else the daemon's (a
/// single-user / admin-configured setup).
async fn issue_accounts(
    ctx: &ServerCtx,
    created_by: &Id,
) -> Result<Vec<otto_core::domain::IssueAccount>> {
    if let Ok(mine) = ctx.issues_store.list_accounts(created_by).await {
        if !mine.is_empty() {
            return Ok(mine);
        }
    }
    let all = ctx.issues_store.list_all_accounts().await?;
    if all.is_empty() {
        return Err(Error::Invalid(
            "no Jira/Confluence account configured".into(),
        ));
    }
    Ok(all)
}

/// `fetch` with each of the run's issue accounts in turn; the first success
/// wins. A missing / unreadable token skips that account rather than ending
/// the search, and the final error names what every account said.
async fn first_readable<T, F, Fut>(
    ctx: &ServerCtx,
    run: &OttoRun,
    what: &str,
    fetch: F,
) -> Result<T>
where
    F: Fn(otto_core::domain::IssueAccount, String) -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut tried = Vec::new();
    for account in issue_accounts(ctx, &run.created_by).await? {
        let label = account.label.clone();
        match otto_core::secrets::get_async(&ctx.secrets, &account.token_ref).await {
            Ok(Some(token)) => match fetch(account, token).await {
                Ok(v) => return Ok(v),
                Err(e) => tried.push(format!("{label}: {e}")),
            },
            Ok(None) => tried.push(format!("{label}: token missing")),
            Err(e) => tried.push(format!("{label}: {e}")),
        }
    }
    if tried.is_empty() {
        return Err(Error::Invalid(
            "no Jira/Confluence account connected".into(),
        ));
    }
    Err(Error::Invalid(format!(
        "no account could read {what} {} — {}",
        run.source_ref,
        tried.join("; ")
    )))
}

async fn resolve_jira(ctx: &ServerCtx, run: &OttoRun) -> Result<ResolvedSource> {
    // With several Jira sites connected the key belongs to one of them — try
    // each (the first one used to win, and every other site's keys failed).
    let issue = first_readable(ctx, run, "Jira issue", |a, token| async move {
        otto_issues::Tracker::for_account(&a, &token)
            .get_issue_full(&run.source_ref)
            .await
    })
    .await?;
    let comments = issue
        .comments
        .iter()
        .take(5)
        .map(|c| format!("- {}", c.body_md))
        .collect::<Vec<_>>()
        .join("\n");
    let body = clean(
        &format!(
            "Status: {}\nType: {}\nLabels: {}\n\n{}\n\n## Recent comments\n{}",
            issue.status,
            issue.issue_type,
            issue.labels.join(", "),
            issue.description_md,
            comments
        ),
        BODY_CAP,
    );
    Ok(ResolvedSource {
        title: format!("{}: {}", issue.key, issue.summary),
        body_md: body,
        goal: format!(
            "Implement and resolve Jira issue {}: {}",
            issue.key, issue.summary
        ),
        source_url: Some(issue.url),
        repo_hint: None,
        metadata: json!({"key": issue.key, "type": issue.issue_type}),
    })
}

async fn resolve_confluence(ctx: &ServerCtx, run: &OttoRun) -> Result<ResolvedSource> {
    // Same rule as Jira: the first connected site that has the page.
    let page = first_readable(ctx, run, "Confluence page", |a, token| async move {
        otto_issues::ConfluenceClient::for_account(&a, &token)?
            .get_page(&run.source_ref)
            .await
    })
    .await?;
    Ok(ResolvedSource {
        title: page.title.clone(),
        body_md: clean(&strip_tags(&page.body_storage), BODY_CAP),
        goal: format!("Act on the Confluence page: {}", page.title),
        source_url: Some(page.url),
        repo_hint: None,
        metadata: json!({"space": page.space_key, "page_id": page.id}),
    })
}

fn parse_owner_repo_num(source_ref: &str) -> Result<u64> {
    let (_, num) = source_ref
        .split_once('#')
        .ok_or_else(|| Error::Invalid(format!("bad github ref: {source_ref}")))?;
    num.parse::<u64>()
        .map_err(|_| Error::Invalid(format!("bad github number: {source_ref}")))
}

async fn resolve_github_pr(ctx: &ServerCtx, run: &OttoRun, repo: &Repo) -> Result<ResolvedSource> {
    let (provider, remote) = provider_for_repo(ctx, repo).await?;
    let num = parse_owner_repo_num(&run.source_ref)?;
    let pr = provider.get_pr(&remote, num).await?;
    let comments = pr
        .comments
        .iter()
        .take(8)
        .map(|c| format!("- {}: {}", c.author, c.body))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(ResolvedSource {
        title: format!("PR #{}: {}", pr.summary.number, pr.summary.title),
        body_md: clean(
            &format!("{}\n\n## Discussion\n{}", pr.description_md, comments),
            BODY_CAP,
        ),
        goal: format!(
            "Address pull request #{}: {}",
            pr.summary.number, pr.summary.title
        ),
        source_url: Some(pr.summary.url),
        repo_hint: Some(repo.name.clone()),
        metadata: json!({"pr": pr.summary.number}),
    })
}

async fn resolve_github_issue(
    ctx: &ServerCtx,
    run: &OttoRun,
    repo: &Repo,
) -> Result<ResolvedSource> {
    let (provider, remote) = provider_for_repo(ctx, repo).await?;
    let num = parse_owner_repo_num(&run.source_ref)?;
    let issue = provider.get_issue(&remote, num).await?;
    Ok(ResolvedSource {
        title: format!("Issue #{}: {}", issue.number, issue.title),
        body_md: clean(&issue.body, BODY_CAP),
        goal: format!("Resolve issue #{}: {}", issue.number, issue.title),
        source_url: Some(issue.url),
        repo_hint: Some(repo.name.clone()),
        metadata: json!({"issue": issue.number}),
    })
}

fn resolve_channel(run: &OttoRun) -> ResolvedSource {
    // The trigger captured the seed message into goal/context_summary at launch.
    let seed = run
        .context_summary
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| run.goal.clone());
    ResolvedSource {
        title: run.title.clone(),
        body_md: clean(&seed, BODY_CAP),
        goal: run.goal.clone(),
        source_url: None,
        repo_hint: None,
        metadata: json!({"origin": run.origin_kind.as_str()}),
    }
}

async fn resolve_product_story(ctx: &ServerCtx, run: &OttoRun) -> Result<ResolvedSource> {
    let story = ctx.product_repo.get_story(&run.source_ref).await?;
    same_workspace(&run.workspace_id, &story.workspace_id, "product story")?;
    Ok(ResolvedSource {
        title: story.title.clone(),
        body_md: clean(
            &format!(
                "Source: {} ({})\nStage: {}\nTags: {}",
                story.source_key, story.url, story.stage, story.tags
            ),
            BODY_CAP,
        ),
        goal: format!("Implement the product story: {}", story.title),
        source_url: Some(story.url),
        repo_hint: story.cwd.clone(),
        metadata: json!({"story_id": story.id, "source_key": story.source_key}),
    })
}

async fn resolve_finding(ctx: &ServerCtx, run: &OttoRun) -> Result<ResolvedSource> {
    let f = ctx.findings_store.get_full(&run.source_ref).await?;
    same_workspace(&run.workspace_id, &f.workspace_id, "finding")?;
    let mut body = format!(
        "Severity: {}\n\n{}\n\n## Evidence\n{}",
        f.severity.as_str(),
        f.body,
        f.evidence
    );
    if let Some(fix) = &f.suggested_fix {
        body.push_str(&format!("\n\n## Suggested fix\n{fix}"));
    }
    if let Some(p) = &f.path {
        body.push_str(&format!("\n\nLocation: {p}"));
    }
    Ok(ResolvedSource {
        title: f.title.clone(),
        body_md: clean(&body, BODY_CAP),
        goal: format!("Fix the review finding: {}", f.title),
        source_url: None,
        repo_hint: None,
        metadata: json!({"finding_id": f.id, "severity": f.severity.as_str()}),
    })
}

async fn resolve_test(ctx: &ServerCtx, run: &OttoRun) -> Result<ResolvedSource> {
    let tc_run = ctx.product_repo.get_testcase_run(&run.source_ref).await?;
    let story = ctx.product_repo.get_story(&tc_run.story_id).await?;
    same_workspace(&run.workspace_id, &story.workspace_id, "test case run")?;
    let cases = ctx.product_repo.list_testcases(&run.source_ref).await?;
    let failing: Vec<_> = cases
        .iter()
        .filter(|c| {
            let s = c.status.to_ascii_lowercase();
            s == "failed" || s == "failing" || s == "rejected"
        })
        .collect();
    let listed: Vec<&_> = if failing.is_empty() {
        cases.iter().collect()
    } else {
        failing.clone()
    };
    let body = listed
        .iter()
        .take(20)
        .map(|c| format!("- [{}] {} ({})", c.status, c.title, c.category))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(ResolvedSource {
        title: format!("Failing test cases ({})", failing.len()),
        body_md: clean(&body, BODY_CAP),
        goal: "Make the failing test case(s) pass.".to_string(),
        source_url: tc_run.confluence_url.clone(),
        repo_hint: None,
        metadata: json!({"run_id": tc_run.id, "failing": failing.len()}),
    })
}

async fn resolve_scheduled_report(ctx: &ServerCtx, run: &OttoRun) -> Result<ResolvedSource> {
    let r = ctx.scheduled_tasks.get_run(&run.source_ref).await?;
    same_workspace(&run.workspace_id, &r.workspace_id, "scheduled-task run")?;
    let body = match &r.report_path {
        Some(p) => tokio::fs::read_to_string(p)
            .await
            .unwrap_or_else(|_| r.summary.clone()),
        None => r.summary.clone(),
    };
    Ok(ResolvedSource {
        title: format!("Scheduled report ({})", r.status),
        body_md: clean(&body, BODY_CAP),
        goal: "Act on the findings in this scheduled-task report.".to_string(),
        source_url: None,
        repo_hint: None,
        metadata: json!({"run_id": r.id, "task_id": r.task_id}),
    })
}

/// A source named by id must belong to the run's own workspace: launch only
/// checks the caller's role THERE, and the source body lands in the run's
/// context, so another workspace's report / finding / story was readable
/// through a run that merely named its id. Reported like a missing one.
fn same_workspace(run_ws: &str, source_ws: &str, what: &str) -> Result<()> {
    if source_ws == run_ws {
        Ok(())
    } else {
        Err(Error::NotFound(what.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_tags_keeps_text() {
        assert_eq!(
            strip_tags("<p>Hello <b>world</b></p>"),
            "Hello world".to_string()
        );
    }

    #[test]
    fn clean_truncates_and_redacts() {
        let long = "a".repeat(BODY_CAP + 100);
        let out = clean(&long, BODY_CAP);
        assert!(out.len() <= BODY_CAP + 20);
        assert!(out.ends_with("(truncated)"));
    }

    #[test]
    fn a_source_of_another_workspace_reads_as_missing() {
        assert!(same_workspace("ws-a", "ws-a", "finding").is_ok());
        assert!(matches!(
            same_workspace("ws-a", "ws-b", "finding"),
            Err(Error::NotFound(_))
        ));
    }

    #[test]
    fn parse_owner_repo_num_works() {
        assert_eq!(parse_owner_repo_num("acme/widgets#42").unwrap(), 42);
        assert!(parse_owner_repo_num("nope").is_err());
    }
}
