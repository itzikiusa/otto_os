//! Hosted git provider clients: GitHub, Bitbucket Cloud, GitLab.
//!
//! All impls speak plain REST via the shared retrying [`client::Http`] and map
//! provider payloads into the common `otto_core::api` PR DTOs.

pub mod bitbucket;
pub mod client;
pub mod detect;
pub mod github;
pub mod gitlab;

#[cfg(test)]
mod wire_tests;

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use otto_core::api::{
    Collaborator, CreatePrReq, DiffResp, MergeStrategy, NewPrCommentReq, PrComment, PrCommit,
    PrDetail, PrState, PrSummary, UpdatePrReq,
};
use otto_core::domain::{GitAccount, GitProviderKind};
use otto_core::Result;
use serde::Serialize;

pub use detect::detect;

/// owner/repo pair extracted from a remote URL. For GitLab nested groups,
/// `owner` is the full group path ("group/subgroup").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteRef {
    pub owner: String,
    pub repo: String,
}

/// A brief summary of a remote repository returned by `list_repos`.
#[derive(Debug, Clone, Serialize)]
pub struct RemoteRepoSummary {
    /// Provider-native "{owner}/{repo}" slug.
    pub full_name: String,
    /// Short repository name.
    pub name: String,
    /// HTTPS clone URL.
    pub clone_url: String,
    /// SSH clone URL.
    pub ssh_url: String,
    /// Repository description (empty string when absent).
    pub description: String,
    pub private: bool,
    /// ISO-8601 last-updated timestamp (empty string when unknown).
    pub updated_at: String,
}

/// One CI check / job / commit-status row for a pull request, as listed by
/// [`GitProvider::list_checks`]. `state` is the normalized per-row verdict:
/// `success` | `failure` | `pending` | `skipped` | `neutral` — the aggregate
/// lives in [`crate::types::CiStatus`], this is the per-row detail the merge
/// modal lists.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PrCheck {
    pub name: String,
    pub state: String,
    /// Link to the run/job page, where the provider exposes one.
    pub url: Option<String>,
    /// RFC3339 start/finish timestamps; absent for providers that don't report them.
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

/// Result of an authenticated token probe (`GitProvider::verify_token`).
/// `scopes` is best-effort: echoed from response headers where the provider
/// exposes them (`X-OAuth-Scopes`), empty otherwise.
#[derive(Debug, Clone)]
pub struct TokenCheck {
    pub login: String,
    pub scopes: Vec<String>,
}

/// One page of a provider's PR list. `has_more` is the provider's OWN next-page
/// signal (`Link rel="next"`, `x-next-page`, Bitbucket's `next` cursor) — never
/// `items.len() == per_page`, which lies on a perfectly full last page.
#[derive(Debug, Clone, Default)]
pub struct PrPage {
    pub items: Vec<PrSummary>,
    pub has_more: bool,
}

/// Common operations on a hosted provider's pull/merge requests.
#[async_trait]
pub trait GitProvider: Send + Sync {
    /// One page of PRs. `page` is 1-based; both bounds are clamped by the
    /// handler before they reach a URL.
    async fn list_prs(
        &self,
        r: &RemoteRef,
        state: PrState,
        page: u32,
        per_page: u32,
    ) -> Result<PrPage>;
    async fn get_pr(&self, r: &RemoteRef, number: u64) -> Result<PrDetail>;
    /// Fetch a hosted **issue** (not a PR). Default: unsupported — only GitHub
    /// overrides it (the Run with Otto github-issue source). GitLab/Bitbucket
    /// issues are out of scope for v1.
    async fn get_issue(&self, _r: &RemoteRef, _number: u64) -> Result<otto_core::api::IssueLite> {
        Err(otto_core::Error::Invalid(
            "fetching issues is only supported for GitHub".into(),
        ))
    }
    /// Provider's unified diff, parsed with `crate::parse::parse_diff`.
    async fn get_pr_diff(&self, r: &RemoteRef, number: u64) -> Result<DiffResp>;
    /// The PR's file list with per-file counts and NO patches, from the
    /// provider's stat API — what the UI's summary-first load asks for, so a
    /// 10k-file PR is a few KB per page instead of the whole unified diff.
    /// `Ok(None)` = no such API; the caller falls back to [`Self::get_pr_diff`].
    async fn get_pr_diff_summary(&self, _r: &RemoteRef, _number: u64) -> Result<Option<DiffResp>> {
        Ok(None)
    }
    /// One file's patch (`path`, plus a rename's `old_path`) without
    /// downloading the whole PR diff. `Ok(None)` = unsupported (fallback as
    /// above). The result may hold other files if the provider ignores the
    /// filter — the caller still selects by path.
    async fn get_pr_file_diff(
        &self,
        _r: &RemoteRef,
        _number: u64,
        _path: &str,
        _old_path: Option<&str>,
    ) -> Result<Option<DiffResp>> {
        Ok(None)
    }
    async fn create_pr(&self, r: &RemoteRef, req: &CreatePrReq) -> Result<PrSummary>;
    async fn update_pr(&self, r: &RemoteRef, number: u64, req: &UpdatePrReq) -> Result<()>;
    async fn comment(&self, r: &RemoteRef, number: u64, c: &NewPrCommentReq) -> Result<PrComment>;
    /// Resolve (`resolved: true`) or reopen (`false`) a review thread.
    /// `thread_id` is the `PrComment.thread_id` surfaced by `get_pr` —
    /// provider-specific (Bitbucket comment id, GitLab discussion id, GitHub
    /// GraphQL thread node id). Default: unsupported.
    async fn resolve_pr_thread(
        &self,
        _r: &RemoteRef,
        _number: u64,
        _thread_id: &str,
        _resolved: bool,
    ) -> Result<()> {
        Err(otto_core::Error::Invalid(
            "resolving review threads is not supported for this provider".into(),
        ))
    }
    async fn approve(&self, r: &RemoteRef, number: u64) -> Result<()>;
    /// Merge PR `number`. `delete_source_branch` asks the provider to drop the
    /// source branch as part of (or right after) the merge — GitHub has no
    /// merge-body flag for it, so its impl issues a follow-up ref delete.
    async fn merge(
        &self,
        r: &RemoteRef,
        number: u64,
        strategy: MergeStrategy,
        delete_source_branch: bool,
    ) -> Result<()>;
    async fn decline(&self, r: &RemoteRef, number: u64) -> Result<()>;
    async fn request_changes(&self, r: &RemoteRef, number: u64, body: Option<&str>) -> Result<()>;
    async fn list_pr_commits(&self, r: &RemoteRef, number: u64) -> Result<Vec<PrCommit>>;
    /// List repositories in `namespace` (org, workspace, group), optionally
    /// filtering by `query`. Returns up to ~50 results.
    async fn list_repos(
        &self,
        namespace: &str,
        query: Option<&str>,
    ) -> Result<Vec<RemoteRepoSummary>>;

    /// People the bound account can request as PR reviewers on this repo,
    /// filtered by the case-insensitive `q` prefix/substring. Backs the
    /// create-PR reviewer typeahead. Default: empty (the UI degrades to a
    /// free-text input).
    async fn list_collaborators(&self, _r: &RemoteRef, _q: &str) -> Result<Vec<Collaborator>> {
        Ok(Vec::new())
    }

    /// Cheapest authenticated call that proves the stored token works, returning
    /// who we authenticated as (+ scopes where the provider exposes them).
    /// Backs the git-account "Test connection" button.
    async fn verify_token(&self) -> Result<TokenCheck> {
        Err(otto_core::Error::Invalid(
            "connection test is not supported for this provider".into(),
        ))
    }

    /// Best-effort probe for the bound token's expiry, where the provider
    /// exposes it (GitHub response header, GitLab PAT introspection). Returns
    /// `Ok(None)` when the provider exposes no expiry or the token does not
    /// expire. Errors are surfaced so the caller can log-and-skip; they must
    /// never be treated as "no expiry". Default: `Ok(None)` (e.g. Bitbucket,
    /// which has no such endpoint — the user-entered value is used instead).
    async fn token_expiry(&self) -> Result<Option<DateTime<Utc>>> {
        Ok(None)
    }

    /// Aggregated CI / check-run status for a PR (Proof Packs use this to capture
    /// a `ci` evidence artifact). The default derives the state from `get_pr`
    /// (state only, no counts); each provider overrides to add counts + a
    /// dashboard URL via its concrete `fetch_ci_status`. Best-effort: a failed
    /// fetch yields a `none` status rather than erroring.
    async fn ci_status(&self, r: &RemoteRef, number: u64) -> crate::types::CiStatus {
        match self.get_pr(r, number).await {
            Ok(d) => crate::types::CiStatus {
                state: d.summary.ci_status.unwrap_or_else(|| "none".into()),
                ..Default::default()
            },
            Err(_) => crate::types::CiStatus::default(),
        }
    }

    /// Per-check rows behind the aggregate [`Self::ci_status`] — the merge
    /// modal lists them individually (name + state + run link). The default
    /// collapses the aggregate into a single row so a provider without a
    /// per-check API still answers something truthful; each provider overrides
    /// it with the real list.
    async fn list_checks(&self, r: &RemoteRef, number: u64) -> Result<Vec<PrCheck>> {
        let ci = self.ci_status(r, number).await;
        Ok(if ci.state == "none" {
            Vec::new()
        } else {
            vec![PrCheck {
                name: "ci".into(),
                state: ci.state.clone(),
                url: ci.url.clone(),
                started_at: None,
                completed_at: None,
            }]
        })
    }
}

/// Build a provider client for an account + token.
pub fn make_provider(account: &GitAccount, token: String) -> Arc<dyn GitProvider> {
    match account.provider {
        GitProviderKind::Github => Arc::new(github::Github::new(token)),
        GitProviderKind::Bitbucket => {
            Arc::new(bitbucket::Bitbucket::new(account.username.clone(), token))
        }
        GitProviderKind::Gitlab => {
            Arc::new(gitlab::Gitlab::new(token, account.api_base_url.clone()))
        }
    }
}

/// Refuse to call the forge for a remote the client can't actually reach.
/// Any host containing "github" is DETECTED as GitHub (so the repo shows as
/// GitHub), but the GitHub client only ever talks to api.github.com —
/// GitHub Enterprise is out of scope. A GHE remote therefore sent the
/// account's token to api.github.com and could come back with a public
/// namesake repo's PRs as this repo's. Such a remote is now an explicit
/// "not supported" instead.
///
/// GitLab is the same bug class: any host containing "gitlab" is DETECTED as
/// GitLab, but an account without an API base URL talks to gitlab.com — so a
/// self-hosted remote (`gitlab.corp.example`) sent the token there and acted
/// on a public namesake project. Without `gitlab_api_base` a non-gitlab.com
/// remote is refused with the fix (set the account's API base URL); with one,
/// the user said where the API lives (ssh and API hosts may differ).
pub fn check_remote_reachable(
    kind: GitProviderKind,
    remote_url: &str,
    gitlab_api_base: Option<&str>,
) -> Result<()> {
    if kind == GitProviderKind::Gitlab {
        if gitlab_api_base.is_some_and(|b| !b.trim().is_empty()) {
            return Ok(());
        }
        return match detect::remote_host(remote_url) {
            Some(host) if !matches!(host.as_str(), "gitlab.com" | "www.gitlab.com") => {
                Err(otto_core::Error::Invalid(format!(
                    "{host} looks like a self-hosted GitLab: set this git account's API base \
                     URL (Settings → Git Accounts) — without it Otto would call gitlab.com"
                )))
            }
            _ => Ok(()),
        };
    }
    if kind != GitProviderKind::Github {
        return Ok(());
    }
    match detect::remote_host(remote_url) {
        Some(host) if !detect::is_github_dot_com(&host) => Err(otto_core::Error::Invalid(format!(
            "{host} looks like GitHub Enterprise, which Otto's PR integration does not \
             support (it only talks to api.github.com)"
        ))),
        _ => Ok(()),
    }
}

/// Map provider state strings to the common [`PrState`].
pub(crate) fn map_state(s: &str) -> PrState {
    match s.to_ascii_lowercase().as_str() {
        "open" | "opened" => PrState::Open,
        "merged" => PrState::Merged,
        _ => PrState::Declined, // closed / declined / superseded / locked
    }
}

/// Lenient RFC3339 parse with epoch fallback (providers occasionally omit
/// timestamps on draft objects).
pub(crate) fn ts(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| DateTime::<Utc>::from_timestamp(0, 0).unwrap_or_default())
}

// -- serde_json::Value navigation helpers ----------------------------------

use serde_json::Value;

pub(crate) fn vstr(v: &Value, path: &[&str]) -> String {
    walk(v, path)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

pub(crate) fn vstr_opt(v: &Value, path: &[&str]) -> Option<String> {
    walk(v, path)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub(crate) fn vu64(v: &Value, path: &[&str]) -> u64 {
    walk(v, path).and_then(Value::as_u64).unwrap_or(0)
}

pub(crate) fn vbool(v: &Value, path: &[&str]) -> Option<bool> {
    walk(v, path).and_then(Value::as_bool)
}

pub(crate) fn varr<'a>(v: &'a Value, path: &[&str]) -> &'a [Value] {
    walk(v, path)
        .and_then(Value::as_array)
        .map_or(&[], |a| a.as_slice())
}

fn walk<'a>(v: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut cur = v;
    for p in path {
        cur = cur.get(p)?;
    }
    Some(cur)
}

#[cfg(test)]
mod reach_tests {
    use super::check_remote_reachable;
    use otto_core::domain::GitProviderKind;

    /// A GitHub Enterprise remote is refused BEFORE any token is loaded — it
    /// used to be called at api.github.com with the account's token.
    #[test]
    fn github_enterprise_remote_is_refused() {
        let gh = GitProviderKind::Github;
        assert!(check_remote_reachable(gh, "git@github.com:o/r.git", None).is_ok());
        assert!(check_remote_reachable(gh, "https://github.com/o/r", None).is_ok());
        let err = check_remote_reachable(gh, "https://u:tok@github.corp.example.com/o/r.git", None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("github.corp.example.com"), "{err}");
        assert!(!err.contains("u:tok"), "no credentials in the text: {err}");
    }

    /// A self-hosted GitLab remote with a gitlab.com-defaulting account is
    /// refused before any token is loaded; an explicit API base unlocks it.
    #[test]
    fn self_hosted_gitlab_needs_an_api_base() {
        let gl = GitProviderKind::Gitlab;
        assert!(check_remote_reachable(gl, "git@gitlab.com:o/r.git", None).is_ok());
        assert!(check_remote_reachable(gl, "https://gitlab.com/g/sub/r.git", None).is_ok());
        let err = check_remote_reachable(gl, "https://u:tok@gitlab.corp/o/r", None)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("gitlab.corp") && err.contains("API base"),
            "{err}"
        );
        assert!(!err.contains("u:tok"), "{err}");
        assert!(
            check_remote_reachable(gl, "https://gitlab.corp/o/r", Some("https://gitlab.corp"))
                .is_ok()
        );
        assert!(check_remote_reachable(gl, "https://gitlab.corp/o/r", Some("  ")).is_err());
        assert!(check_remote_reachable(
            GitProviderKind::Bitbucket,
            "https://bitbucket.org/o/r",
            None
        )
        .is_ok());
    }
}
