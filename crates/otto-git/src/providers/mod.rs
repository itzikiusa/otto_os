//! Hosted git provider clients: GitHub, Bitbucket Cloud, GitLab.
//!
//! All impls speak plain REST via the shared retrying [`client::Http`] and map
//! provider payloads into the common `otto_core::api` PR DTOs.

pub mod bitbucket;
pub mod client;
pub mod detect;
pub mod github;
pub mod gitlab;

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use otto_core::api::{
    CreatePrReq, DiffResp, MergeStrategy, NewPrCommentReq, PrComment, PrCommit, PrDetail, PrState,
    PrSummary, UpdatePrReq,
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

/// Common operations on a hosted provider's pull/merge requests.
#[async_trait]
pub trait GitProvider: Send + Sync {
    async fn list_prs(&self, r: &RemoteRef, state: PrState) -> Result<Vec<PrSummary>>;
    async fn get_pr(&self, r: &RemoteRef, number: u64) -> Result<PrDetail>;
    /// Fetch a hosted **issue** (not a PR). Default: unsupported — only GitHub
    /// overrides it (the Run with Otto github-issue source). GitLab/Bitbucket
    /// issues are out of scope for v1.
    async fn get_issue(
        &self,
        _r: &RemoteRef,
        _number: u64,
    ) -> Result<otto_core::api::IssueLite> {
        Err(otto_core::Error::Invalid(
            "fetching issues is only supported for GitHub".into(),
        ))
    }
    /// Provider's unified diff, parsed with `crate::parse::parse_diff`.
    async fn get_pr_diff(&self, r: &RemoteRef, number: u64) -> Result<DiffResp>;
    async fn create_pr(&self, r: &RemoteRef, req: &CreatePrReq) -> Result<PrSummary>;
    async fn update_pr(&self, r: &RemoteRef, number: u64, req: &UpdatePrReq) -> Result<()>;
    async fn comment(&self, r: &RemoteRef, number: u64, c: &NewPrCommentReq) -> Result<PrComment>;
    async fn approve(&self, r: &RemoteRef, number: u64) -> Result<()>;
    async fn merge(&self, r: &RemoteRef, number: u64, strategy: MergeStrategy) -> Result<()>;
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
