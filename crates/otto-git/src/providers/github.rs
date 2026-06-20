//! GitHub REST v3 client (api.github.com).
//!
//! Auth: `Authorization: Bearer <token>`; JSON via `application/vnd.github+json`,
//! PR diff via `application/vnd.github.diff`. Inline review comments need the
//! PR head commit sha (fetched on demand) and use side=RIGHT + line.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use otto_core::api::{
    CreatePrReq, DiffResp, MergeStrategy, NewPrCommentReq, PrComment, PrCommit, PrDetail,
    PrReviewer, PrState, PrSummary, UpdatePrReq,
};
use otto_core::{Error, Result};
use serde_json::{json, Value};

use crate::types::CiStatus;

use super::client::Http;
use super::{map_state, ts, varr, vbool, vstr, vstr_opt, RemoteRef, RemoteRepoSummary};

const BASE: &str = "https://api.github.com";

/// Percent-encode characters that must be escaped in GitHub search query values.
fn percent_encode_query(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        match c {
            ' ' => out.push_str("%20"),
            '"' => out.push_str("%22"),
            ':' => out.push_str("%3A"),
            '/' => out.push_str("%2F"),
            '?' => out.push_str("%3F"),
            '&' => out.push_str("%26"),
            '#' => out.push_str("%23"),
            other => out.push(other),
        }
    }
    out
}

pub struct Github {
    http: Http,
    token: String,
}

impl Github {
    pub fn new(token: String) -> Self {
        Self {
            http: Http::new("github"),
            token,
        }
    }

    fn req(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .client()
            .request(method, format!("{BASE}{path}"))
            .bearer_auth(&self.token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
    }

    fn prs_path(r: &RemoteRef) -> String {
        format!("/repos/{}/{}/pulls", r.owner, r.repo)
    }

    fn issues_path(r: &RemoteRef) -> String {
        format!("/repos/{}/{}/issues", r.owner, r.repo)
    }

    async fn pr_raw(&self, r: &RemoteRef, number: u64) -> Result<Value> {
        self.http
            .json(self.req(
                reqwest::Method::GET,
                &format!("{}/{number}", Self::prs_path(r)),
            ))
            .await
    }

    /// Fetch GitHub check-run status for the HEAD commit of `number` and
    /// aggregate it into a [`CiStatus`]. Returns `CiStatus::none()` on any
    /// provider error so a CI probe never fails the whole PR fetch.
    pub async fn fetch_ci_status(&self, r: &RemoteRef, number: u64) -> CiStatus {
        // First get the head sha from the PR.
        let pr = match self.pr_raw(r, number).await {
            Ok(v) => v,
            Err(_) => return CiStatus::none(),
        };
        let sha = vstr(&pr, &["head", "sha"]);
        if sha.is_empty() {
            return CiStatus::none();
        }
        // Fetch check-runs for the commit.
        let path = format!("/repos/{}/{}/commits/{sha}/check-runs?per_page=100", r.owner, r.repo);
        let v = match self.http.json(self.req(reqwest::Method::GET, &path)).await {
            Ok(v) => v,
            Err(_) => return CiStatus::none(),
        };
        let runs = varr(&v, &["check_runs"]);
        if runs.is_empty() {
            // Fall back to legacy commit statuses.
            return self.fetch_commit_status(r, &sha).await;
        }
        let total = runs.len() as u32;
        let mut passed = 0u32;
        let mut failed = 0u32;
        let mut pending = 0u32;
        let mut run_url: Option<String> = None;
        for run in runs {
            let conclusion = vstr(run, &["conclusion"]);
            let status = vstr(run, &["status"]);
            if run_url.is_none() {
                run_url = vstr_opt(run, &["html_url"]);
            }
            match (status.as_str(), conclusion.as_str()) {
                (_, "success") | (_, "neutral") | (_, "skipped") => passed += 1,
                (_, "failure") | (_, "cancelled") | (_, "timed_out") | (_, "action_required") => {
                    failed += 1
                }
                _ => pending += 1,
            }
        }
        let state = if failed > 0 {
            "failure"
        } else if pending > 0 {
            "pending"
        } else if passed == total && total > 0 {
            "success"
        } else {
            "none"
        };
        CiStatus { state: state.to_string(), total, passed, failed, url: run_url }
    }

    /// Fallback: aggregate legacy GitHub commit statuses (the older Statuses
    /// API, still used by some third-party integrations).
    async fn fetch_commit_status(&self, r: &RemoteRef, sha: &str) -> CiStatus {
        let path = format!("/repos/{}/{}/commits/{sha}/statuses?per_page=100", r.owner, r.repo);
        let v = match self.http.json(self.req(reqwest::Method::GET, &path)).await {
            Ok(v) => v,
            Err(_) => return CiStatus::none(),
        };
        let items = varr(&v, &[]);
        if items.is_empty() {
            return CiStatus::none();
        }
        // Statuses are newest-first; keep only the latest per context name.
        let mut seen: std::collections::HashSet<String> = Default::default();
        let mut total = 0u32;
        let mut passed = 0u32;
        let mut failed = 0u32;
        let mut pending = 0u32;
        let mut url: Option<String> = None;
        for item in items {
            let ctx = vstr(item, &["context"]);
            if !seen.insert(ctx) {
                continue;
            }
            total += 1;
            if url.is_none() {
                url = vstr_opt(item, &["target_url"]);
            }
            match vstr(item, &["state"]).as_str() {
                "success" => passed += 1,
                "failure" | "error" => failed += 1,
                _ => pending += 1,
            }
        }
        let state = if failed > 0 {
            "failure"
        } else if pending > 0 {
            "pending"
        } else if passed == total && total > 0 {
            "success"
        } else {
            "none"
        };
        CiStatus { state: state.to_string(), total, passed, failed, url }
    }
}

fn summary_from(v: &Value) -> PrSummary {
    let state = if vstr_opt(v, &["merged_at"]).is_some() {
        PrState::Merged
    } else {
        map_state(&vstr(v, &["state"]))
    };
    PrSummary {
        number: super::vu64(v, &["number"]),
        title: vstr(v, &["title"]),
        author: vstr(v, &["user", "login"]),
        state,
        source_branch: vstr(v, &["head", "ref"]),
        target_branch: vstr(v, &["base", "ref"]),
        updated_at: ts(&vstr(v, &["updated_at"])),
        url: vstr(v, &["html_url"]),
        draft: Some(v.get("draft").and_then(|d| d.as_bool()).unwrap_or(false)),
        ci_status: None,
        labels: v.get("labels")
            .and_then(|l| l.as_array())
            .map(|arr| arr.iter().filter_map(|l| l.get("name").and_then(|n| n.as_str()).map(str::to_string)).collect())
            .unwrap_or_default(),
    }
}

fn comment_from(v: &Value, path: Option<String>, line: Option<u32>) -> PrComment {
    PrComment {
        id: super::vu64(v, &["id"]).to_string(),
        author: vstr(v, &["user", "login"]),
        body: vstr(v, &["body"]),
        path,
        line,
        created_at: ts(&vstr(v, &["created_at"])),
        replies: Vec::new(),
    }
}

#[async_trait]
impl super::GitProvider for Github {
    async fn list_prs(&self, r: &RemoteRef, state: PrState) -> Result<Vec<PrSummary>> {
        let gh_state = match state {
            PrState::Open => "open",
            PrState::Merged | PrState::Declined => "closed",
            PrState::All => "all",
        };
        let items = self
            .http
            .paginate_json(
                self.req(reqwest::Method::GET, &Self::prs_path(r))
                    .query(&[("state", gh_state), ("per_page", "100")]),
                self.http.client(),
                ("Authorization", format!("Bearer {}", self.token)),
            )
            .await?;
        let mut prs: Vec<PrSummary> = items.iter().map(summary_from).collect();
        if matches!(state, PrState::Merged | PrState::Declined) {
            prs.retain(|p| p.state == state);
        }
        Ok(prs)
    }

    async fn get_pr(&self, r: &RemoteRef, number: u64) -> Result<PrDetail> {
        let pr = self.pr_raw(r, number).await?;

        // General (issue) comments — flat thread.
        let issue_comments = self
            .http
            .json(
                self.req(
                    reqwest::Method::GET,
                    &format!("{}/{number}/comments", Self::issues_path(r)),
                )
                .query(&[("per_page", "100")]),
            )
            .await?;

        // Inline review comments — threaded via in_reply_to_id.
        let review_comments = self
            .http
            .json(
                self.req(
                    reqwest::Method::GET,
                    &format!("{}/{number}/comments", Self::prs_path(r)),
                )
                .query(&[("per_page", "100")]),
            )
            .await?;

        // Reviews → approvals.
        let reviews = self
            .http
            .json(
                self.req(
                    reqwest::Method::GET,
                    &format!("{}/{number}/reviews", Self::prs_path(r)),
                )
                .query(&[("per_page", "100")]),
            )
            .await?;

        let mut comments: Vec<PrComment> = varr(&issue_comments, &[])
            .iter()
            .map(|c| comment_from(c, None, None))
            .collect();

        // Build review-comment threads.
        let mut top: Vec<PrComment> = Vec::new();
        let mut replies: Vec<(u64, PrComment)> = Vec::new();
        for c in varr(&review_comments, &[]) {
            let path = vstr_opt(c, &["path"]);
            let line = c
                .get("line")
                .and_then(Value::as_u64)
                .or_else(|| c.get("original_line").and_then(Value::as_u64))
                .map(|l| l as u32);
            let pc = comment_from(c, path, line);
            match c.get("in_reply_to_id").and_then(Value::as_u64) {
                Some(parent) => replies.push((parent, pc)),
                None => top.push(pc),
            }
        }
        for (parent, reply) in replies {
            if let Some(t) = top.iter_mut().find(|t| t.id == parent.to_string()) {
                t.replies.push(reply);
            } else {
                top.push(reply); // orphan — surface as top-level
            }
        }
        comments.append(&mut top);

        let approved_by: Vec<String> = varr(&reviews, &[])
            .iter()
            .filter(|rv| vstr(rv, &["state"]) == "APPROVED")
            .map(|rv| vstr(rv, &["user", "login"]))
            .collect();

        // Reviews come back in chronological order; dedupe by reviewer keeping
        // their latest review while preserving first-seen order.
        let mut reviewers: Vec<PrReviewer> = Vec::new();
        for rv in varr(&reviews, &[]) {
            let name = vstr(rv, &["user", "login"]);
            if name.is_empty() {
                continue;
            }
            let entry = PrReviewer {
                approved: vstr(rv, &["state"]) == "APPROVED",
                avatar_url: vstr_opt(rv, &["user", "avatar_url"]),
                reviewed_at: vstr_opt(rv, &["submitted_at"]).map(|s| ts(&s)),
                name: name.clone(),
            };
            match reviewers.iter_mut().find(|r| r.name == name) {
                Some(existing) => *existing = entry,
                None => reviewers.push(entry),
            }
        }

        // Best-effort CI status — never fails the PR fetch.
        let ci = self.fetch_ci_status(r, number).await;
        let mut summary = summary_from(&pr);
        summary.ci_status = Some(ci.state.clone());

        Ok(PrDetail {
            summary,
            description_md: vstr(&pr, &["body"]),
            comments,
            approved_by,
            reviewers,
            mergeable: vbool(&pr, &["mergeable"]),
        })
    }

    async fn get_pr_diff(&self, r: &RemoteRef, number: u64) -> Result<DiffResp> {
        let text = self
            .http
            .text(
                self.req(
                    reqwest::Method::GET,
                    &format!("{}/{number}", Self::prs_path(r)),
                )
                .header("Accept", "application/vnd.github.diff"),
            )
            .await?;
        Ok(crate::parse::parse_diff(&text))
    }

    async fn create_pr(&self, r: &RemoteRef, req: &CreatePrReq) -> Result<PrSummary> {
        let v = self
            .http
            .json(
                self.req(reqwest::Method::POST, &Self::prs_path(r))
                    .json(&json!({
                        "title": req.title,
                        "body": req.description,
                        "head": req.source_branch,
                        "base": req.target_branch,
                    })),
            )
            .await?;
        Ok(summary_from(&v))
    }

    async fn update_pr(&self, r: &RemoteRef, number: u64, req: &UpdatePrReq) -> Result<()> {
        let mut body = serde_json::Map::new();
        if let Some(t) = &req.title {
            body.insert("title".into(), json!(t));
        }
        if let Some(d) = &req.description {
            body.insert("body".into(), json!(d));
        }
        if body.is_empty() {
            return Ok(());
        }
        self.http
            .ok(self
                .req(
                    reqwest::Method::PATCH,
                    &format!("{}/{number}", Self::prs_path(r)),
                )
                .json(&Value::Object(body)))
            .await
    }

    async fn comment(&self, r: &RemoteRef, number: u64, c: &NewPrCommentReq) -> Result<PrComment> {
        // Reply to an inline review comment thread.
        if let Some(reply_to) = &c.in_reply_to {
            let id: u64 = reply_to
                .parse()
                .map_err(|_| Error::Invalid(format!("bad comment id: {reply_to}")))?;
            let v = self
                .http
                .json(
                    self.req(
                        reqwest::Method::POST,
                        &format!("{}/{number}/comments", Self::prs_path(r)),
                    )
                    .json(&json!({ "body": c.body, "in_reply_to": id })),
                )
                .await?;
            let path = vstr_opt(&v, &["path"]);
            let line = v.get("line").and_then(Value::as_u64).map(|l| l as u32);
            return Ok(comment_from(&v, path, line));
        }
        // Inline comment: needs the head commit sha.
        if let (Some(path), Some(line)) = (&c.path, c.line) {
            let pr = self.pr_raw(r, number).await?;
            let commit_id = vstr(&pr, &["head", "sha"]);
            let v = self
                .http
                .json(
                    self.req(
                        reqwest::Method::POST,
                        &format!("{}/{number}/comments", Self::prs_path(r)),
                    )
                    .json(&json!({
                        "body": c.body,
                        "commit_id": commit_id,
                        "path": path,
                        "line": line,
                        "side": "RIGHT",
                    })),
                )
                .await?;
            return Ok(comment_from(&v, Some(path.clone()), Some(line)));
        }
        // General comment → issue comment.
        let v = self
            .http
            .json(
                self.req(
                    reqwest::Method::POST,
                    &format!("{}/{number}/comments", Self::issues_path(r)),
                )
                .json(&json!({ "body": c.body })),
            )
            .await?;
        Ok(comment_from(&v, None, None))
    }

    async fn approve(&self, r: &RemoteRef, number: u64) -> Result<()> {
        self.http
            .ok(self
                .req(
                    reqwest::Method::POST,
                    &format!("{}/{number}/reviews", Self::prs_path(r)),
                )
                .json(&json!({ "event": "APPROVE" })))
            .await
    }

    async fn merge(&self, r: &RemoteRef, number: u64, strategy: MergeStrategy) -> Result<()> {
        let method = match strategy {
            MergeStrategy::Merge => "merge",
            MergeStrategy::Squash => "squash",
            MergeStrategy::Rebase => "rebase",
        };
        self.http
            .ok(self
                .req(
                    reqwest::Method::PUT,
                    &format!("{}/{number}/merge", Self::prs_path(r)),
                )
                .json(&json!({ "merge_method": method })))
            .await
    }

    async fn decline(&self, r: &RemoteRef, number: u64) -> Result<()> {
        self.http
            .ok(self
                .req(
                    reqwest::Method::PATCH,
                    &format!("{}/{number}", Self::prs_path(r)),
                )
                .json(&json!({ "state": "closed" })))
            .await
    }

    async fn request_changes(&self, r: &RemoteRef, number: u64, body: Option<&str>) -> Result<()> {
        let b = body.unwrap_or("Changes requested.");
        self.http
            .ok(self
                .req(
                    reqwest::Method::POST,
                    &format!("{}/{number}/reviews", Self::prs_path(r)),
                )
                .json(&json!({ "event": "REQUEST_CHANGES", "body": b })))
            .await
    }

    async fn list_pr_commits(&self, r: &RemoteRef, number: u64) -> Result<Vec<PrCommit>> {
        let v = self
            .http
            .json(
                self.req(
                    reqwest::Method::GET,
                    &format!("{}/{number}/commits", Self::prs_path(r)),
                )
                .query(&[("per_page", "100")]),
            )
            .await?;
        let commits = super::varr(&v, &[])
            .iter()
            .map(|c| {
                let sha = super::vstr(c, &["sha"]);
                let short_sha = sha.chars().take(7).collect();
                let message = super::vstr(c, &["commit", "message"]);
                let subject = message.lines().next().unwrap_or("").to_string();
                let author = super::vstr(c, &["commit", "author", "name"]);
                let date = super::ts(&super::vstr(c, &["commit", "author", "date"]));
                PrCommit {
                    sha,
                    short_sha,
                    author,
                    date,
                    subject,
                }
            })
            .collect();
        Ok(commits)
    }

    async fn list_repos(
        &self,
        namespace: &str,
        query: Option<&str>,
    ) -> Result<Vec<RemoteRepoSummary>> {
        fn repo_from(v: &serde_json::Value) -> RemoteRepoSummary {
            RemoteRepoSummary {
                full_name: vstr(v, &["full_name"]),
                name: vstr(v, &["name"]),
                clone_url: vstr(v, &["clone_url"]),
                ssh_url: vstr(v, &["ssh_url"]),
                description: vstr(v, &["description"]),
                private: vbool(v, &["private"]).unwrap_or(false),
                updated_at: vstr(v, &["updated_at"]),
            }
        }

        if let Some(q) = query {
            if !q.is_empty() {
                // Search endpoint: q = "<query> org:<namespace>"
                let encoded = percent_encode_query(&format!("{q} org:{namespace}"));
                let url = format!("/search/repositories?per_page=50&q={encoded}");
                let v = self.http.json(self.req(reqwest::Method::GET, &url)).await?;
                return Ok(varr(&v, &["items"]).iter().map(repo_from).collect());
            }
        }

        // No query: try org first, fall back to user.
        let org_url = format!("/orgs/{namespace}/repos?per_page=50&sort=updated");
        match self
            .http
            .json(self.req(reqwest::Method::GET, &org_url))
            .await
        {
            Ok(v) => Ok(varr(&v, &[]).iter().map(repo_from).collect()),
            Err(_) => {
                // 404 → try as a user namespace.
                let user_url = format!("/users/{namespace}/repos?per_page=50&sort=updated");
                let v = self
                    .http
                    .json(self.req(reqwest::Method::GET, &user_url))
                    .await?;
                Ok(varr(&v, &[]).iter().map(repo_from).collect())
            }
        }
    }

    /// GitHub returns `github-authentication-token-expiration` on any
    /// authenticated call for fine-grained PATs / GitHub-App tokens. We make a
    /// cheap `GET /user` and read the header; absent header ⇒ token does not
    /// expire (classic PAT without expiry) ⇒ `Ok(None)`.
    async fn token_expiry(&self) -> Result<Option<DateTime<Utc>>> {
        let resp = self
            .http
            .send(self.req(reqwest::Method::GET, "/user"))
            .await?;
        let raw = resp
            .headers()
            .get("github-authentication-token-expiration")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        Ok(raw.as_deref().and_then(parse_github_expiry))
    }
}

/// Parse GitHub's token-expiration header. Observed forms:
/// `2024-12-31 23:59:59 UTC`, `2024-12-31 23:59:59 +0000`, and plain RFC3339.
fn parse_github_expiry(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // RFC3339 (e.g. when GitHub returns ISO form).
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    // "UTC" suffix → treat the naive datetime as UTC.
    if let Some(naive) = s.strip_suffix(" UTC") {
        if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(naive.trim(), "%Y-%m-%d %H:%M:%S") {
            return Some(DateTime::<Utc>::from_naive_utc_and_offset(ndt, Utc));
        }
    }
    // Numeric offset (e.g. "+0000").
    if let Ok(dt) = DateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S %z") {
        return Some(dt.with_timezone(&Utc));
    }
    None
}

/// Parse a small inline check-runs JSON fixture into a CiStatus aggregate.
/// Used by tests; not exposed to the public API.
#[cfg(test)]
fn parse_check_runs_fixture(json_str: &str) -> crate::types::CiStatus {
    let v: serde_json::Value = serde_json::from_str(json_str).unwrap_or_default();
    let runs = varr(&v, &["check_runs"]);
    let total = runs.len() as u32;
    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut pending = 0u32;
    let mut run_url: Option<String> = None;
    for run in runs {
        let conclusion = vstr(run, &["conclusion"]);
        let status = vstr(run, &["status"]);
        if run_url.is_none() {
            run_url = vstr_opt(run, &["html_url"]);
        }
        match (status.as_str(), conclusion.as_str()) {
            (_, "success") | (_, "neutral") | (_, "skipped") => passed += 1,
            (_, "failure") | (_, "cancelled") | (_, "timed_out") | (_, "action_required") => {
                failed += 1
            }
            _ => pending += 1,
        }
    }
    let state = if failed > 0 {
        "failure"
    } else if pending > 0 {
        "pending"
    } else if passed == total && total > 0 {
        "success"
    } else {
        "none"
    };
    crate::types::CiStatus { state: state.to_string(), total, passed, failed, url: run_url }
}

#[cfg(test)]
mod tests {
    use super::{parse_check_runs_fixture, parse_github_expiry};
    use chrono::{Datelike, Timelike};

    #[test]
    fn parses_utc_suffix_form() {
        let dt = parse_github_expiry("2024-12-31 23:59:59 UTC").expect("parsed");
        assert_eq!(dt.year(), 2024);
        assert_eq!(dt.month(), 12);
        assert_eq!(dt.day(), 31);
        assert_eq!(dt.hour(), 23);
    }

    #[test]
    fn parses_numeric_offset_form() {
        let dt = parse_github_expiry("2024-06-01 12:00:00 +0000").expect("parsed");
        assert_eq!(dt.year(), 2024);
        assert_eq!(dt.hour(), 12);
    }

    #[test]
    fn parses_rfc3339() {
        let dt = parse_github_expiry("2025-01-15T08:30:00Z").expect("parsed");
        assert_eq!(dt.day(), 15);
    }

    #[test]
    fn empty_or_garbage_is_none() {
        assert!(parse_github_expiry("").is_none());
        assert!(parse_github_expiry("   ").is_none());
        assert!(parse_github_expiry("never").is_none());
    }

    // --- CI status aggregation unit tests (parse inline JSON fixtures) --------

    #[test]
    fn ci_status_all_success() {
        let fixture = r#"{"check_runs":[
            {"status":"completed","conclusion":"success","html_url":"https://ci.example.com/1"},
            {"status":"completed","conclusion":"success","html_url":"https://ci.example.com/2"}
        ]}"#;
        let ci = parse_check_runs_fixture(fixture);
        assert_eq!(ci.state, "success");
        assert_eq!(ci.total, 2);
        assert_eq!(ci.passed, 2);
        assert_eq!(ci.failed, 0);
        assert!(ci.url.is_some());
    }

    #[test]
    fn ci_status_one_failure() {
        let fixture = r#"{"check_runs":[
            {"status":"completed","conclusion":"success","html_url":null},
            {"status":"completed","conclusion":"failure","html_url":"https://ci.example.com/fail"}
        ]}"#;
        let ci = parse_check_runs_fixture(fixture);
        assert_eq!(ci.state, "failure");
        assert_eq!(ci.failed, 1);
    }

    #[test]
    fn ci_status_pending_run() {
        let fixture = r#"{"check_runs":[
            {"status":"in_progress","conclusion":"","html_url":null}
        ]}"#;
        let ci = parse_check_runs_fixture(fixture);
        assert_eq!(ci.state, "pending");
        assert_eq!(ci.total, 1);
    }

    #[test]
    fn ci_status_empty_is_none() {
        let fixture = r#"{"check_runs":[]}"#;
        let ci = parse_check_runs_fixture(fixture);
        assert_eq!(ci.state, "none");
        assert_eq!(ci.total, 0);
    }
}
