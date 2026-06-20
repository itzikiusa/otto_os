//! GitLab v4 client ({base|gitlab.com}/api/v4).
//!
//! Auth: `PRIVATE-TOKEN` header. Project id is the URL-encoded full path
//! (`owner%2Frepo`, nested groups included). "PRs" are merge requests keyed
//! by `iid`. Diff is assembled from `/changes` payloads (per-file hunk text).
//! Inline comments need the MR `diff_refs` shas and use discussions with a
//! text position; replies address the *discussion* id (see `comment`).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use otto_core::api::{
    CreatePrReq, DiffResp, FileDiff, MergeStrategy, NewPrCommentReq, PrComment, PrCommit, PrDetail,
    PrReviewer, PrState, PrSummary, UpdatePrReq,
};
use otto_core::Result;
use serde_json::{json, Value};

use super::client::Http;
use super::{map_state, ts, varr, vstr, vstr_opt, vu64, RemoteRef, RemoteRepoSummary};

/// Percent-encode characters that must be escaped in GitLab query/path values.
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

pub struct Gitlab {
    http: Http,
    token: String,
    base: String,
}

impl Gitlab {
    pub fn new(token: String, api_base_url: Option<String>) -> Self {
        let base = api_base_url
            .unwrap_or_else(|| "https://gitlab.com".to_string())
            .trim_end_matches('/')
            .to_string();
        Self {
            http: Http::new("gitlab"),
            token,
            base,
        }
    }

    fn req(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .client()
            .request(method, format!("{}/api/v4{path}", self.base))
            .header("PRIVATE-TOKEN", &self.token)
    }

    fn project_id(r: &RemoteRef) -> String {
        urlencoding::encode(&format!("{}/{}", r.owner, r.repo)).into_owned()
    }

    fn mr_path(r: &RemoteRef, tail: &str) -> String {
        format!("/projects/{}/merge_requests{tail}", Self::project_id(r))
    }
}

fn summary_from(v: &Value) -> PrSummary {
    PrSummary {
        number: vu64(v, &["iid"]),
        title: vstr(v, &["title"]),
        author: {
            let name = vstr(v, &["author", "name"]);
            if name.is_empty() {
                vstr(v, &["author", "username"])
            } else {
                name
            }
        },
        state: map_state(&vstr(v, &["state"])),
        source_branch: vstr(v, &["source_branch"]),
        target_branch: vstr(v, &["target_branch"]),
        updated_at: ts(&vstr(v, &["updated_at"])),
        draft: Some(vstr(v, &["title"]).starts_with("Draft:") || vstr(v, &["title"]).starts_with("WIP:")),
        ci_status: None,
        labels: v.get("labels").and_then(|l| l.as_array())
            .map(|arr| arr.iter().filter_map(|l| l.as_str().map(str::to_string)).collect())
            .unwrap_or_default(),
        url: vstr(v, &["web_url"]),
    }
}

fn note_to_comment(note: &Value, id_override: Option<String>) -> PrComment {
    let path = vstr_opt(note, &["position", "new_path"])
        .or_else(|| vstr_opt(note, &["position", "old_path"]));
    let line = note
        .get("position")
        .and_then(|p| p.get("new_line"))
        .and_then(Value::as_u64)
        .map(|l| l as u32);
    PrComment {
        id: id_override.unwrap_or_else(|| vu64(note, &["id"]).to_string()),
        author: vstr(note, &["author", "name"]),
        body: vstr(note, &["body"]),
        path,
        line,
        created_at: ts(&vstr(note, &["created_at"])),
        replies: Vec::new(),
    }
}

#[async_trait]
impl super::GitProvider for Gitlab {
    async fn list_prs(&self, r: &RemoteRef, state: PrState) -> Result<Vec<PrSummary>> {
        let mut rb = self
            .req(reqwest::Method::GET, &Self::mr_path(r, ""))
            .query(&[("per_page", "50")]);
        rb = match state {
            PrState::Open => rb.query(&[("state", "opened")]),
            PrState::Merged => rb.query(&[("state", "merged")]),
            PrState::Declined => rb.query(&[("state", "closed")]),
            PrState::All => rb,
        };
        let v = self.http.json(rb).await?;
        Ok(varr(&v, &[]).iter().map(summary_from).collect())
    }

    async fn get_pr(&self, r: &RemoteRef, number: u64) -> Result<PrDetail> {
        let mr = self
            .http
            .json(self.req(
                reqwest::Method::GET,
                &Self::mr_path(r, &format!("/{number}")),
            ))
            .await?;

        // Discussions: first non-system note is the thread head, rest replies.
        // For threads the exposed comment id is the DISCUSSION id so that
        // replies can target it (`in_reply_to`).
        let discussions = self
            .http
            .json(
                self.req(
                    reqwest::Method::GET,
                    &Self::mr_path(r, &format!("/{number}/discussions")),
                )
                .query(&[("per_page", "100")]),
            )
            .await?;
        let mut comments: Vec<PrComment> = Vec::new();
        for d in varr(&discussions, &[]) {
            let disc_id = vstr(d, &["id"]);
            let notes: Vec<&Value> = varr(d, &["notes"])
                .iter()
                .filter(|n| !n.get("system").and_then(Value::as_bool).unwrap_or(false))
                .collect();
            let Some(first) = notes.first() else { continue };
            let mut head = note_to_comment(first, Some(disc_id));
            for reply in &notes[1..] {
                head.replies.push(note_to_comment(reply, None));
            }
            comments.push(head);
        }

        // Approvals (best effort — endpoint exists on CE and SaaS).
        // GitLab exposes no per-approver timestamp, so reviewed_at is None and
        // anyone in approved_by is, by definition, an approver.
        let (approved_by, reviewers): (Vec<String>, Vec<PrReviewer>) = match self
            .http
            .json(self.req(
                reqwest::Method::GET,
                &Self::mr_path(r, &format!("/{number}/approvals")),
            ))
            .await
        {
            Ok(ap) => {
                let approved_by = varr(&ap, &["approved_by"])
                    .iter()
                    .map(|e| vstr(e, &["user", "name"]))
                    .filter(|s| !s.is_empty())
                    .collect();
                let reviewers = varr(&ap, &["approved_by"])
                    .iter()
                    .filter(|e| !vstr(e, &["user", "name"]).is_empty())
                    .map(|e| PrReviewer {
                        name: vstr(e, &["user", "name"]),
                        approved: true,
                        avatar_url: vstr_opt(e, &["user", "avatar_url"]),
                        reviewed_at: None,
                    })
                    .collect();
                (approved_by, reviewers)
            }
            Err(_) => (Vec::new(), Vec::new()),
        };

        let mergeable = match vstr(&mr, &["merge_status"]).as_str() {
            "can_be_merged" => Some(true),
            "cannot_be_merged" => Some(false),
            _ => None,
        };

        Ok(PrDetail {
            summary: summary_from(&mr),
            description_md: vstr(&mr, &["description"]),
            comments,
            approved_by,
            reviewers,
            mergeable,
        })
    }

    async fn get_pr_diff(&self, r: &RemoteRef, number: u64) -> Result<DiffResp> {
        let v = self
            .http
            .json(self.req(
                reqwest::Method::GET,
                &Self::mr_path(r, &format!("/{number}/changes")),
            ))
            .await?;
        let mut files = Vec::new();
        for ch in varr(&v, &["changes"]) {
            let new_path = vstr(ch, &["new_path"]);
            let old_path = vstr(ch, &["old_path"]);
            let renamed = ch
                .get("renamed_file")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                || (old_path != new_path && !old_path.is_empty());
            let diff_text = vstr(ch, &["diff"]);
            let is_binary = diff_text.is_empty()
                || diff_text.starts_with("Binary files")
                || diff_text.contains("GIT binary patch");
            let hunks = if is_binary {
                Vec::new()
            } else {
                crate::parse::parse_hunks(&diff_text)
            };
            files.push(FileDiff {
                path: new_path,
                old_path: if renamed { Some(old_path) } else { None },
                is_binary: is_binary && !diff_text.is_empty(),
                hunks,
                too_large: None,
                added: None,
                deleted: None,
                language: None,
            });
        }
        Ok(DiffResp { files })
    }

    async fn create_pr(&self, r: &RemoteRef, req: &CreatePrReq) -> Result<PrSummary> {
        let v = self
            .http
            .json(
                self.req(reqwest::Method::POST, &Self::mr_path(r, ""))
                    .json(&json!({
                        "title": req.title,
                        "description": req.description,
                        "source_branch": req.source_branch,
                        "target_branch": req.target_branch,
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
            body.insert("description".into(), json!(d));
        }
        if body.is_empty() {
            return Ok(());
        }
        self.http
            .ok(self
                .req(
                    reqwest::Method::PUT,
                    &Self::mr_path(r, &format!("/{number}")),
                )
                .json(&Value::Object(body)))
            .await
    }

    async fn comment(&self, r: &RemoteRef, number: u64, c: &NewPrCommentReq) -> Result<PrComment> {
        // Reply to an existing discussion (id = discussion id, as exposed by get_pr).
        if let Some(disc_id) = &c.in_reply_to {
            let v = self
                .http
                .json(
                    self.req(
                        reqwest::Method::POST,
                        &Self::mr_path(r, &format!("/{number}/discussions/{disc_id}/notes")),
                    )
                    .json(&json!({ "body": c.body })),
                )
                .await?;
            return Ok(note_to_comment(&v, None));
        }
        // Inline comment → discussion with a text position (needs diff_refs).
        if let (Some(path), Some(line)) = (&c.path, c.line) {
            let mr = self
                .http
                .json(self.req(
                    reqwest::Method::GET,
                    &Self::mr_path(r, &format!("/{number}")),
                ))
                .await?;
            let v = self
                .http
                .json(
                    self.req(
                        reqwest::Method::POST,
                        &Self::mr_path(r, &format!("/{number}/discussions")),
                    )
                    .json(&json!({
                        "body": c.body,
                        "position": {
                            "position_type": "text",
                            "base_sha": vstr(&mr, &["diff_refs", "base_sha"]),
                            "start_sha": vstr(&mr, &["diff_refs", "start_sha"]),
                            "head_sha": vstr(&mr, &["diff_refs", "head_sha"]),
                            "new_path": path,
                            "new_line": line,
                        },
                    })),
                )
                .await?;
            let disc_id = vstr_opt(&v, &["id"]);
            let note = varr(&v, &["notes"]).first().cloned().unwrap_or(v.clone());
            return Ok(note_to_comment(&note, disc_id));
        }
        // General note.
        let v = self
            .http
            .json(
                self.req(
                    reqwest::Method::POST,
                    &Self::mr_path(r, &format!("/{number}/notes")),
                )
                .json(&json!({ "body": c.body })),
            )
            .await?;
        Ok(note_to_comment(&v, None))
    }

    async fn approve(&self, r: &RemoteRef, number: u64) -> Result<()> {
        self.http
            .ok(self.req(
                reqwest::Method::POST,
                &Self::mr_path(r, &format!("/{number}/approve")),
            ))
            .await
    }

    async fn merge(&self, r: &RemoteRef, number: u64, strategy: MergeStrategy) -> Result<()> {
        if strategy == MergeStrategy::Rebase {
            // Rebase the source branch first (async on GitLab's side), give it
            // a moment, then merge fast-forward style.
            self.http
                .ok(self.req(
                    reqwest::Method::PUT,
                    &Self::mr_path(r, &format!("/{number}/rebase")),
                ))
                .await?;
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
        let squash = strategy == MergeStrategy::Squash;
        self.http
            .ok(self
                .req(
                    reqwest::Method::PUT,
                    &Self::mr_path(r, &format!("/{number}/merge")),
                )
                .json(&json!({ "squash": squash })))
            .await
    }

    async fn decline(&self, r: &RemoteRef, number: u64) -> Result<()> {
        self.http
            .ok(self
                .req(
                    reqwest::Method::PUT,
                    &Self::mr_path(r, &format!("/{number}")),
                )
                .json(&json!({ "state_event": "close" })))
            .await
    }

    async fn request_changes(&self, r: &RemoteRef, number: u64, body: Option<&str>) -> Result<()> {
        let b = body.unwrap_or("Changes requested.");
        self.http
            .ok(self
                .req(
                    reqwest::Method::POST,
                    &Self::mr_path(r, &format!("/{number}/notes")),
                )
                .json(&json!({ "body": b })))
            .await
    }

    async fn list_pr_commits(&self, r: &RemoteRef, number: u64) -> Result<Vec<PrCommit>> {
        let v = self
            .http
            .json(
                self.req(
                    reqwest::Method::GET,
                    &Self::mr_path(r, &format!("/{number}/commits")),
                )
                .query(&[("per_page", "100")]),
            )
            .await?;
        let commits = varr(&v, &[])
            .iter()
            .map(|c| {
                let sha = vstr(c, &["id"]);
                let short_sha = vstr(c, &["short_id"]);
                let subject = vstr(c, &["title"]);
                let author = vstr(c, &["author_name"]);
                let date = super::ts(&vstr(c, &["created_at"]));
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
            let visibility = vstr(v, &["visibility"]);
            RemoteRepoSummary {
                full_name: vstr(v, &["path_with_namespace"]),
                name: {
                    let n = vstr(v, &["name"]);
                    if n.is_empty() {
                        vstr(v, &["path"])
                    } else {
                        n
                    }
                },
                clone_url: vstr(v, &["http_url_to_repo"]),
                ssh_url: vstr(v, &["ssh_url_to_repo"]),
                description: vstr(v, &["description"]),
                private: visibility != "public",
                updated_at: vstr(v, &["last_activity_at"]),
            }
        }

        let encoded_ns = urlencoding::encode(namespace).into_owned();
        let mut base_path =
            format!("/groups/{encoded_ns}/projects?per_page=50&order_by=last_activity_at");
        if let Some(q) = query {
            if !q.is_empty() {
                base_path.push_str(&format!("&search={}", percent_encode_query(q)));
            }
        }
        match self
            .http
            .json(self.req(reqwest::Method::GET, &base_path))
            .await
        {
            Ok(v) => Ok(varr(&v, &[]).iter().map(repo_from).collect()),
            Err(_) => {
                // 404 → try as a user namespace.
                let mut user_path = format!("/users/{encoded_ns}/projects?per_page=50");
                if let Some(q) = query {
                    if !q.is_empty() {
                        user_path.push_str(&format!("&search={}", percent_encode_query(q)));
                    }
                }
                let v = self
                    .http
                    .json(self.req(reqwest::Method::GET, &user_path))
                    .await?;
                Ok(varr(&v, &[]).iter().map(repo_from).collect())
            }
        }
    }

    /// GitLab exposes the current PAT via `GET /personal_access_tokens/self`,
    /// whose `expires_at` is a `YYYY-MM-DD` date (or null = never expires).
    /// We treat the date as end-of-day UTC. Tokens that don't expire ⇒ `None`.
    async fn token_expiry(&self) -> Result<Option<DateTime<Utc>>> {
        let v = self
            .http
            .json(self.req(reqwest::Method::GET, "/personal_access_tokens/self"))
            .await?;
        Ok(vstr_opt(&v, &["expires_at"]).and_then(|s| parse_gitlab_expiry(&s)))
    }
}

/// Parse GitLab's `expires_at` (`YYYY-MM-DD` date, occasionally full RFC3339).
/// A bare date is interpreted as 23:59:59 UTC on that day so we don't warn a
/// day early.
fn parse_gitlab_expiry(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let ndt = date.and_hms_opt(23, 59, 59)?;
        return Some(DateTime::<Utc>::from_naive_utc_and_offset(ndt, Utc));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::parse_gitlab_expiry;
    use chrono::{Datelike, Timelike};

    #[test]
    fn bare_date_is_end_of_day_utc() {
        let dt = parse_gitlab_expiry("2024-12-31").expect("parsed");
        assert_eq!(dt.year(), 2024);
        assert_eq!(dt.month(), 12);
        assert_eq!(dt.day(), 31);
        assert_eq!(dt.hour(), 23);
        assert_eq!(dt.minute(), 59);
    }

    #[test]
    fn rfc3339_passes_through() {
        let dt = parse_gitlab_expiry("2025-03-10T10:00:00Z").expect("parsed");
        assert_eq!(dt.day(), 10);
        assert_eq!(dt.hour(), 10);
    }

    #[test]
    fn empty_or_garbage_is_none() {
        assert!(parse_gitlab_expiry("").is_none());
        assert!(parse_gitlab_expiry("nope").is_none());
    }
}
