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
    /// API root. Always [`BASE`] in production; overridden only by tests
    /// (`with_base`) so every hop can be pointed at a local stub.
    base: String,
}

impl Github {
    pub fn new(token: String) -> Self {
        Self::with_base(token, BASE.to_string())
    }

    /// Same client against a different API root — the wiremock tests' entry
    /// point. Not a user-facing setting: GitHub Enterprise is out of scope.
    pub(crate) fn with_base(token: String, base: String) -> Self {
        Self {
            http: Http::new("github"),
            token,
            base: base.trim_end_matches('/').to_string(),
        }
    }

    fn req(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .client()
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
    }

    /// `Authorization` pair for the raw-`reqwest` pagination helper.
    fn auth_header(&self) -> (&'static str, String) {
        ("Authorization", format!("Bearer {}", self.token))
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

    /// POST /graphql with the bound token. GraphQL failures come back as 200 +
    /// an `errors` array — surface those as `Upstream` like REST errors.
    async fn graphql(&self, query: &str, variables: Value) -> Result<Value> {
        let v = self
            .http
            .json(
                self.http
                    .client()
                    .post(format!("{}/graphql", self.base))
                    .bearer_auth(&self.token)
                    .json(&json!({ "query": query, "variables": variables })),
            )
            .await?;
        if let Some(msg) = varr(&v, &["errors"]).first().map(|e| vstr(e, &["message"])) {
            return Err(Error::Upstream(format!("github graphql: {msg}")));
        }
        Ok(v)
    }

    /// Review threads with resolution state — REST doesn't expose `isResolved`,
    /// so this is the one GraphQL read. Returns the `reviewThreads.nodes` array.
    async fn fetch_review_threads(&self, r: &RemoteRef, number: u64) -> Result<Vec<Value>> {
        const Q: &str = "query($owner:String!,$name:String!,$number:Int!){\
            repository(owner:$owner,name:$name){pullRequest(number:$number){\
            reviewThreads(first:100){nodes{id isResolved \
            comments(first:50){nodes{databaseId}}}}}}}";
        let v = self
            .graphql(
                Q,
                json!({ "owner": r.owner, "name": r.repo, "number": number }),
            )
            .await?;
        Ok(varr(
            &v,
            &[
                "data",
                "repository",
                "pullRequest",
                "reviewThreads",
                "nodes",
            ],
        )
        .to_vec())
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
        let path = format!(
            "/repos/{}/{}/commits/{sha}/check-runs?per_page=100",
            r.owner, r.repo
        );
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
        CiStatus {
            state: state.to_string(),
            total,
            passed,
            failed,
            url: run_url,
        }
    }

    /// Fallback: aggregate legacy GitHub commit statuses (the older Statuses
    /// API, still used by some third-party integrations).
    async fn fetch_commit_status(&self, r: &RemoteRef, sha: &str) -> CiStatus {
        let path = format!(
            "/repos/{}/{}/commits/{sha}/statuses?per_page=100",
            r.owner, r.repo
        );
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
        CiStatus {
            state: state.to_string(),
            total,
            passed,
            failed,
            url,
        }
    }
}

use super::PrCheck;

/// `check_runs[]` → one [`PrCheck`] per run. The state mapping mirrors
/// [`Github::fetch_ci_status`]'s aggregate walk, but keeps each row's own
/// verdict instead of folding it: `success`/`neutral`/`skipped` survive
/// verbatim, the four failing conclusions collapse to `failure`, and anything
/// not yet concluded is `pending`.
pub(crate) fn checks_from_check_runs(v: &Value) -> Vec<PrCheck> {
    varr(v, &["check_runs"])
        .iter()
        .map(|run| {
            let status = vstr(run, &["status"]);
            let conclusion = vstr(run, &["conclusion"]);
            let state = match (status.as_str(), conclusion.as_str()) {
                (_, "success") => "success",
                (_, "neutral") => "neutral",
                (_, "skipped") => "skipped",
                (_, "failure") | (_, "cancelled") | (_, "timed_out") | (_, "action_required") => {
                    "failure"
                }
                _ => "pending",
            };
            PrCheck {
                name: vstr(run, &["name"]),
                state: state.to_string(),
                url: vstr_opt(run, &["html_url"]),
                started_at: vstr_opt(run, &["started_at"]),
                completed_at: vstr_opt(run, &["completed_at"]),
            }
        })
        .collect()
}

/// Legacy Statuses API → one [`PrCheck`] per `context`. Statuses are
/// newest-first and repeat per context, so only the first row of each wins
/// (same de-dup as [`Github::fetch_commit_status`]).
pub(crate) fn checks_from_statuses(v: &Value) -> Vec<PrCheck> {
    let mut seen: std::collections::HashSet<String> = Default::default();
    let mut out = Vec::new();
    for item in varr(v, &[]) {
        let ctx = vstr(item, &["context"]);
        if !seen.insert(ctx.clone()) {
            continue;
        }
        let state = match vstr(item, &["state"]).as_str() {
            "success" => "success",
            "failure" | "error" => "failure",
            _ => "pending",
        };
        out.push(PrCheck {
            name: ctx,
            state: state.to_string(),
            url: vstr_opt(item, &["target_url"]),
            started_at: None,
            completed_at: None,
        });
    }
    out
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
        reviewer_warnings: Vec::new(),
        draft: Some(v.get("draft").and_then(|d| d.as_bool()).unwrap_or(false)),
        ci_status: None,
        labels: v
            .get("labels")
            .and_then(|l| l.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|l| l.get("name").and_then(|n| n.as_str()).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// Create-PR request body. `draft` is included only when explicitly requested
/// (absent = GitHub's default ready-for-review). Reviewers are NOT part of the
/// create payload — GitHub only accepts them via the two-step
/// `requested_reviewers` call after creation (see `create_pr`).
fn create_pr_body(req: &CreatePrReq) -> Value {
    let mut body = json!({
        "title": req.title,
        "body": req.description,
        "head": req.source_branch,
        "base": req.target_branch,
    });
    if let Some(draft) = req.draft {
        body["draft"] = json!(draft);
    }
    body
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
        resolved: false,
        thread_id: None, // stamped from GraphQL reviewThreads in get_pr
    }
}

/// Stamp GraphQL review-thread resolution onto the REST-built threads.
/// `thread_nodes` is `reviewThreads.nodes`: each node maps its comments'
/// `databaseId`s (== REST comment ids) to the thread node `id` + `isResolved`.
fn apply_thread_resolution(top: &mut [PrComment], thread_nodes: &[Value]) {
    let mut by_comment: std::collections::HashMap<String, (String, bool)> =
        std::collections::HashMap::new();
    for t in thread_nodes {
        let tid = vstr(t, &["id"]);
        if tid.is_empty() {
            continue;
        }
        let resolved = vbool(t, &["isResolved"]).unwrap_or(false);
        for c in varr(t, &["comments", "nodes"]) {
            let db_id = super::vu64(c, &["databaseId"]);
            if db_id > 0 {
                by_comment.insert(db_id.to_string(), (tid.clone(), resolved));
            }
        }
    }
    for c in top.iter_mut() {
        if let Some((tid, resolved)) = by_comment.get(&c.id) {
            c.thread_id = Some(tid.clone());
            c.resolved = *resolved;
        }
    }
}

#[async_trait]
impl super::GitProvider for Github {
    async fn list_prs(
        &self,
        r: &RemoteRef,
        state: PrState,
        page: u32,
        per_page: u32,
    ) -> Result<super::PrPage> {
        let gh_state = match state {
            PrState::Open => "open",
            PrState::Merged | PrState::Declined => "closed",
            PrState::All => "all",
        };
        // ONE request per page — the client pages, not the daemon.
        let resp = self
            .http
            .send(self.req(reqwest::Method::GET, &Self::prs_path(r)).query(&[
                ("state", gh_state.to_string()),
                ("per_page", per_page.to_string()),
                ("page", page.to_string()),
            ]))
            .await?;
        let has_more = super::client::parse_next_link(resp.headers()).is_some();
        let v: Value = resp
            .json()
            .await
            .map_err(|e| Error::Upstream(format!("github: bad json: {e}")))?;
        let mut items: Vec<PrSummary> = varr(&v, &[]).iter().map(summary_from).collect();
        // GitHub has no "merged"/"declined" filter — both are `closed`; the
        // split is ours, so this page may be shorter than `per_page`.
        if matches!(state, PrState::Merged | PrState::Declined) {
            items.retain(|p| p.state == state);
        }
        Ok(super::PrPage { items, has_more })
    }

    async fn get_pr(&self, r: &RemoteRef, number: u64) -> Result<PrDetail> {
        let pr = self.pr_raw(r, number).await?;

        // A PR detail must be WHOLE, so every comment list follows `Link
        // rel="next"` (capped at 20 pages inside `paginate_json`) instead of
        // silently stopping at 100.
        //
        // General (issue) comments — flat thread.
        let issue_comments = Value::Array(
            self.http
                .paginate_json(
                    self.req(
                        reqwest::Method::GET,
                        &format!("{}/{number}/comments", Self::issues_path(r)),
                    )
                    .query(&[("per_page", "100")]),
                    self.http.client(),
                    self.auth_header(),
                )
                .await?,
        );

        // Inline review comments — threaded via in_reply_to_id.
        let review_comments = Value::Array(
            self.http
                .paginate_json(
                    self.req(
                        reqwest::Method::GET,
                        &format!("{}/{number}/comments", Self::prs_path(r)),
                    )
                    .query(&[("per_page", "100")]),
                    self.http.client(),
                    self.auth_header(),
                )
                .await?,
        );

        // Reviews → approvals.
        let reviews = Value::Array(
            self.http
                .paginate_json(
                    self.req(
                        reqwest::Method::GET,
                        &format!("{}/{number}/reviews", Self::prs_path(r)),
                    )
                    .query(&[("per_page", "100")]),
                    self.http.client(),
                    self.auth_header(),
                )
                .await?,
        );

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
        // Best-effort resolution state (GraphQL only) — a missing scope or a
        // network hiccup must never fail the PR fetch.
        if !top.is_empty() {
            match self.fetch_review_threads(r, number).await {
                Ok(nodes) => apply_thread_resolution(&mut top, &nodes),
                Err(e) => {
                    tracing::debug!("github review-thread resolution unavailable: {e}")
                }
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

    async fn get_issue(&self, r: &RemoteRef, number: u64) -> Result<otto_core::api::IssueLite> {
        let v: Value = self
            .http
            .json(self.req(
                reqwest::Method::GET,
                &format!("{}/{number}", Self::issues_path(r)),
            ))
            .await?;
        Ok(otto_core::api::IssueLite {
            number,
            title: vstr(&v, &["title"]),
            body: vstr(&v, &["body"]),
            url: vstr(&v, &["html_url"]),
            state: vstr(&v, &["state"]),
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
                    .json(&create_pr_body(req)),
            )
            .await?;
        let mut summary = summary_from(&v);
        // Reviewers: the create endpoint doesn't take them — request them in a
        // best-effort second call. A failure must not fail the (already open)
        // PR; it surfaces as a warning on the response instead.
        if let Some(reviewers) = req.reviewers.as_ref().filter(|l| !l.is_empty()) {
            let path = format!(
                "{}/{}/requested_reviewers",
                Self::prs_path(r),
                summary.number
            );
            if let Err(e) = self
                .http
                .ok(self
                    .req(reqwest::Method::POST, &path)
                    .json(&json!({ "reviewers": reviewers })))
                .await
            {
                summary
                    .reviewer_warnings
                    .push(reviewer_warning(reviewers, &e.to_string()));
            }
        }
        Ok(summary)
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

    /// GraphQL `resolveReviewThread` / `unresolveReviewThread` — `thread_id`
    /// is the reviewThread node id from `get_pr` (not a REST comment id).
    async fn resolve_pr_thread(
        &self,
        _r: &RemoteRef,
        _number: u64,
        thread_id: &str,
        resolved: bool,
    ) -> Result<()> {
        let m = if resolved {
            "mutation($id:ID!){resolveReviewThread(input:{threadId:$id}){thread{id}}}"
        } else {
            "mutation($id:ID!){unresolveReviewThread(input:{threadId:$id}){thread{id}}}"
        };
        self.graphql(m, json!({ "id": thread_id }))
            .await
            .map(|_| ())
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

    async fn merge(
        &self,
        r: &RemoteRef,
        number: u64,
        strategy: MergeStrategy,
        delete_source_branch: bool,
    ) -> Result<()> {
        let method = match strategy {
            MergeStrategy::Merge => "merge",
            MergeStrategy::Squash => "squash",
            MergeStrategy::Rebase => "rebase",
        };
        // Read the head ref BEFORE the merge: GitHub has no merge-body flag for
        // deleting the source branch, and the PR payload is the only place the
        // ref name is available.
        let head_ref = if delete_source_branch {
            vstr(&self.pr_raw(r, number).await?, &["head", "ref"])
        } else {
            String::new()
        };
        self.http
            .ok(self
                .req(
                    reqwest::Method::PUT,
                    &format!("{}/{number}/merge", Self::prs_path(r)),
                )
                .json(&json!({ "merge_method": method })))
            .await?;
        if !head_ref.is_empty() {
            let path = format!("/repos/{}/{}/git/refs/heads/{head_ref}", r.owner, r.repo);
            if let Err(e) = self.http.ok(self.req(reqwest::Method::DELETE, &path)).await {
                // The merge is the operation the caller asked for; the delete
                // is best-effort. The repo's "automatically delete head
                // branches" setting may have got there first (422 "Reference
                // does not exist" = the wanted end state), and a protected
                // branch or a token without delete rights must not turn a merge
                // that ALREADY LANDED into a failure the user retries.
                if !e.to_string().contains("Reference does not exist") {
                    tracing::warn!(pr = number, branch = %head_ref, "merged, but the source branch could not be deleted: {e}");
                }
            }
        }
        Ok(())
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

    /// Repo collaborators (anyone with access — the set GitHub accepts as
    /// requested reviewers), filtered by `q` on login/display name.
    async fn list_collaborators(
        &self,
        r: &RemoteRef,
        q: &str,
    ) -> Result<Vec<otto_core::api::Collaborator>> {
        let path = format!("/repos/{}/{}/collaborators?per_page=100", r.owner, r.repo);
        let v = self
            .http
            .json(self.req(reqwest::Method::GET, &path))
            .await?;
        let needle = q.to_ascii_lowercase();
        Ok(varr(&v, &[])
            .iter()
            .map(collaborator_from)
            .filter(|c| {
                needle.is_empty()
                    || c.name.to_ascii_lowercase().contains(&needle)
                    || c.display_name.to_ascii_lowercase().contains(&needle)
            })
            .collect())
    }

    /// `GET /user` with the bound token: proves authentication and echoes the
    /// classic-PAT scopes from the `x-oauth-scopes` header (absent for
    /// fine-grained PATs — scopes stay empty).
    async fn verify_token(&self) -> Result<super::TokenCheck> {
        let resp = self
            .http
            .send(self.req(reqwest::Method::GET, "/user"))
            .await?;
        let scopes = scopes_header(resp.headers());
        let v: Value = resp
            .json()
            .await
            .map_err(|e| otto_core::Error::Upstream(format!("github: bad json: {e}")))?;
        Ok(super::TokenCheck {
            login: vstr(&v, &["login"]),
            scopes,
        })
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

    async fn ci_status(&self, r: &RemoteRef, number: u64) -> CiStatus {
        self.fetch_ci_status(r, number).await
    }

    /// Check-runs for the PR head sha, falling back to the legacy Statuses API
    /// when the commit has no check-runs — the same two-step walk
    /// [`Github::fetch_ci_status`] does, kept per row.
    async fn list_checks(&self, r: &RemoteRef, number: u64) -> Result<Vec<PrCheck>> {
        let sha = vstr(&self.pr_raw(r, number).await?, &["head", "sha"]);
        if sha.is_empty() {
            return Ok(Vec::new());
        }
        let path = format!(
            "/repos/{}/{}/commits/{sha}/check-runs?per_page=100",
            r.owner, r.repo
        );
        let v = self
            .http
            .json(self.req(reqwest::Method::GET, &path))
            .await?;
        let rows = checks_from_check_runs(&v);
        if !rows.is_empty() {
            return Ok(rows);
        }
        let path = format!(
            "/repos/{}/{}/commits/{sha}/statuses?per_page=100",
            r.owner, r.repo
        );
        let v = self
            .http
            .json(self.req(reqwest::Method::GET, &path))
            .await?;
        Ok(checks_from_statuses(&v))
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

/// Warning attached to a create-PR response when the post-create reviewer
/// request fails: the PR is open, only the review request was lost.
fn reviewer_warning(names: &[String], err: &str) -> String {
    format!("could not request reviewer(s) {}: {err}", names.join(", "))
}

/// One collaborator row → common DTO (`login` is the requestable handle).
fn collaborator_from(v: &Value) -> otto_core::api::Collaborator {
    let login = vstr(v, &["login"]);
    let name = vstr(v, &["name"]);
    otto_core::api::Collaborator {
        display_name: if name.is_empty() { login.clone() } else { name },
        name: login,
    }
}

/// Comma-separated `x-oauth-scopes` response header → scope list (also sent by
/// Bitbucket Cloud; absent for GitHub fine-grained PATs).
pub(crate) fn scopes_header(headers: &reqwest::header::HeaderMap) -> Vec<String> {
    headers
        .get("x-oauth-scopes")
        .and_then(|v| v.to_str().ok())
        .map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
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
    crate::types::CiStatus {
        state: state.to_string(),
        total,
        passed,
        failed,
        url: run_url,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_thread_resolution, collaborator_from, comment_from, create_pr_body,
        parse_check_runs_fixture, parse_github_expiry, reviewer_warning, scopes_header,
        summary_from,
    };
    use chrono::{Datelike, Timelike};
    use otto_core::api::CreatePrReq;

    #[test]
    fn thread_resolution_joins_by_database_id() {
        let mut top = vec![
            comment_from(
                &serde_json::json!({"id": 101, "user": {"login": "a"}, "body": "x",
                                    "created_at": "2026-01-01T00:00:00Z"}),
                Some("f.rs".into()),
                Some(3),
            ),
            comment_from(
                &serde_json::json!({"id": 202, "user": {"login": "b"}, "body": "y",
                                    "created_at": "2026-01-01T00:00:00Z"}),
                Some("g.rs".into()),
                Some(9),
            ),
        ];
        let nodes = vec![serde_json::json!({
            "id": "PRRT_abc", "isResolved": true,
            "comments": {"nodes": [{"databaseId": 101}]}
        })];
        apply_thread_resolution(&mut top, &nodes);
        assert!(top[0].resolved);
        assert_eq!(top[0].thread_id.as_deref(), Some("PRRT_abc"));
        // Comment 202 has no thread node → untouched defaults.
        assert!(!top[1].resolved);
        assert!(top[1].thread_id.is_none());
    }

    fn req(draft: Option<bool>, reviewers: Option<Vec<String>>) -> CreatePrReq {
        CreatePrReq {
            title: "t".into(),
            description: "d".into(),
            source_branch: "feat/x".into(),
            target_branch: "main".into(),
            proof_pack_id: None,
            allow_unproven: None,
            draft,
            reviewers,
        }
    }

    #[test]
    fn create_body_includes_draft_only_when_set() {
        let b = create_pr_body(&req(Some(true), None));
        assert_eq!(b["draft"], serde_json::json!(true));
        assert_eq!(b["title"], serde_json::json!("t"));
        assert_eq!(b["head"], serde_json::json!("feat/x"));
        assert_eq!(b["base"], serde_json::json!("main"));
        // Absent draft → today's payload, no `draft` key at all.
        assert!(create_pr_body(&req(None, None)).get("draft").is_none());
        assert_eq!(
            create_pr_body(&req(Some(false), None))["draft"],
            serde_json::json!(false)
        );
    }

    #[test]
    fn create_body_never_carries_reviewers() {
        // GitHub reviewers go through the two-step requested_reviewers call.
        let b = create_pr_body(&req(None, Some(vec!["alice".into()])));
        assert!(b.get("reviewers").is_none());
    }

    #[test]
    fn reviewer_failure_is_a_warning_not_an_error() {
        let w = reviewer_warning(
            &["alice".into(), "bob".into()],
            "github 422: not a collaborator",
        );
        assert!(w.contains("alice, bob"));
        assert!(w.contains("422"));
        // The create flow attaches this to reviewer_warnings on an Ok summary —
        // verify the summary type defaults to no warnings.
        let s = summary_from(&serde_json::json!({"number": 7, "title": "x", "state": "open"}));
        assert!(s.reviewer_warnings.is_empty());
    }

    #[test]
    fn collaborator_mapping_prefers_display_name() {
        let c = collaborator_from(&serde_json::json!({"login": "octo", "name": "Octo Cat"}));
        assert_eq!(c.name, "octo");
        assert_eq!(c.display_name, "Octo Cat");
        let c2 = collaborator_from(&serde_json::json!({"login": "octo"}));
        assert_eq!(c2.display_name, "octo");
    }

    #[test]
    fn scopes_header_parses_comma_list() {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert("x-oauth-scopes", "repo, read:org".parse().unwrap());
        assert_eq!(
            scopes_header(&h),
            vec!["repo".to_string(), "read:org".to_string()]
        );
        assert!(scopes_header(&reqwest::header::HeaderMap::new()).is_empty());
    }

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

    // --- per-check rows (merge modal) ---------------------------------------

    #[test]
    fn checks_rows_from_check_runs() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"check_runs":[
                {"name":"build","status":"completed","conclusion":"success",
                 "html_url":"https://ci.example.com/1",
                 "started_at":"2026-01-01T00:00:00Z","completed_at":"2026-01-01T00:04:00Z"},
                {"name":"lint","status":"completed","conclusion":"timed_out","html_url":null},
                {"name":"docs","status":"completed","conclusion":"skipped","html_url":null},
                {"name":"e2e","status":"in_progress","conclusion":"","html_url":null}
            ]}"#,
        )
        .unwrap();
        let rows = super::checks_from_check_runs(&v);
        let got: Vec<(&str, &str)> = rows
            .iter()
            .map(|c| (c.name.as_str(), c.state.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("build", "success"),
                ("lint", "failure"),
                ("docs", "skipped"),
                ("e2e", "pending"),
            ]
        );
        assert_eq!(rows[0].url.as_deref(), Some("https://ci.example.com/1"));
        assert_eq!(rows[0].started_at.as_deref(), Some("2026-01-01T00:00:00Z"));
        assert_eq!(
            rows[0].completed_at.as_deref(),
            Some("2026-01-01T00:04:00Z")
        );
        assert!(rows[1].url.is_none());
    }

    #[test]
    fn checks_rows_from_statuses() {
        // Newest-first with a repeated context: only the newest row survives.
        let v: serde_json::Value = serde_json::from_str(
            r#"[
                {"context":"ci/build","state":"success","target_url":"https://ci.example.com/b"},
                {"context":"ci/build","state":"failure","target_url":"https://ci.example.com/old"},
                {"context":"ci/lint","state":"error","target_url":null},
                {"context":"ci/e2e","state":"pending","target_url":null}
            ]"#,
        )
        .unwrap();
        let rows = super::checks_from_statuses(&v);
        let got: Vec<(&str, &str)> = rows
            .iter()
            .map(|c| (c.name.as_str(), c.state.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("ci/build", "success"),
                ("ci/lint", "failure"),
                ("ci/e2e", "pending"),
            ]
        );
        assert_eq!(rows[0].url.as_deref(), Some("https://ci.example.com/b"));
    }
}
