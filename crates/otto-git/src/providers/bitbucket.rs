//! Bitbucket Cloud 2.0 client (api.bitbucket.org/2.0).
//!
//! Auth: HTTP Basic with `username:app-password`. PR id is the provider-native
//! numeric id. Inline comments use `inline: {path, to: line}`; replies use
//! `parent: {id}`. Merge strategies: merge_commit | squash | fast_forward
//! (`rebase` is mapped to fast_forward — Bitbucket has no true rebase merge).

use async_trait::async_trait;
use otto_core::api::{
    CreatePrReq, DiffResp, MergeStrategy, NewPrCommentReq, PrComment, PrCommit, PrDetail,
    PrReviewer, PrState, PrSummary, UpdatePrReq,
};
use otto_core::{Error, Result};
use serde_json::{json, Value};

use super::client::Http;
use super::{map_state, ts, varr, vbool, vstr, vstr_opt, vu64, RemoteRef, RemoteRepoSummary};

const BASE: &str = "https://api.bitbucket.org/2.0";

/// Percent-encode characters that must be escaped inside Bitbucket query
/// values (space, `"`, `:`, `/`, `?`, `&`, `#`).
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

pub struct Bitbucket {
    http: Http,
    username: String,
    token: String,
}

impl Bitbucket {
    pub fn new(username: String, token: String) -> Self {
        Self {
            http: Http::new("bitbucket"),
            username,
            token,
        }
    }

    fn pr_path(r: &RemoteRef, tail: &str) -> String {
        format!("/repositories/{}/{}/pullrequests{tail}", r.owner, r.repo)
    }

    /// Build a request with either Basic or Bearer auth.
    fn build(
        &self,
        method: reqwest::Method,
        path: &str,
        bearer: bool,
        body: Option<&serde_json::Value>,
    ) -> reqwest::RequestBuilder {
        let mut rb = self
            .http
            .client()
            .request(method, format!("{BASE}{path}"))
            .header("Accept", "application/json");
        rb = if bearer {
            rb.bearer_auth(&self.token)
        } else {
            rb.basic_auth(&self.username, Some(&self.token))
        };
        if let Some(b) = body {
            rb = rb.json(b);
        }
        rb
    }

    /// Send with Basic auth, retrying once as Bearer on 401/403.
    /// Returns a successful `Response` or an `Upstream` error.
    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<reqwest::Response> {
        let resp = self
            .http
            .send_raw(self.build(method.clone(), path, false, body))
            .await?;
        let status = resp.status().as_u16();
        if status == 401 || status == 403 {
            let resp2 = self
                .http
                .send_raw(self.build(method, path, true, body))
                .await?;
            return self.http.into_result(resp2).await;
        }
        self.http.into_result(resp).await
    }

    /// Send and parse the response body as JSON.
    async fn send_json(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<serde_json::Value> {
        let resp = self.send(method, path, body).await?;
        resp.json::<serde_json::Value>()
            .await
            .map_err(|e| Error::Upstream(format!("bitbucket decode: {e}")))
    }

    /// Send and return the response body as plain text.
    async fn send_text(&self, method: reqwest::Method, path: &str) -> Result<String> {
        let resp = self.send(method, path, None).await?;
        resp.text()
            .await
            .map_err(|e| Error::Upstream(format!("bitbucket read: {e}")))
    }
}

fn summary_from(v: &Value) -> PrSummary {
    PrSummary {
        number: vu64(v, &["id"]),
        title: vstr(v, &["title"]),
        author: {
            let display = vstr(v, &["author", "display_name"]);
            if display.is_empty() {
                vstr(v, &["author", "nickname"])
            } else {
                display
            }
        },
        state: map_state(&vstr(v, &["state"])),
        source_branch: vstr(v, &["source", "branch", "name"]),
        target_branch: vstr(v, &["destination", "branch", "name"]),
        updated_at: ts(&vstr(v, &["updated_on"])),
        url: vstr(v, &["links", "html", "href"]),
        draft: None,
        ci_status: None,
        labels: vec![],
    }
}

fn comment_from(v: &Value) -> PrComment {
    let path = vstr_opt(v, &["inline", "path"]);
    let line = v
        .get("inline")
        .and_then(|i| i.get("to"))
        .and_then(Value::as_u64)
        .map(|l| l as u32);
    PrComment {
        id: vu64(v, &["id"]).to_string(),
        author: vstr(v, &["user", "display_name"]),
        body: vstr(v, &["content", "raw"]),
        path,
        line,
        created_at: ts(&vstr(v, &["created_on"])),
        replies: Vec::new(),
    }
}

#[async_trait]
impl super::GitProvider for Bitbucket {
    async fn list_prs(&self, r: &RemoteRef, state: PrState) -> Result<Vec<PrSummary>> {
        let states: &[&str] = match state {
            PrState::Open => &["OPEN"],
            PrState::Merged => &["MERGED"],
            PrState::Declined => &["DECLINED", "SUPERSEDED"],
            PrState::All => &["OPEN", "MERGED", "DECLINED", "SUPERSEDED"],
        };
        // Build the path with query params manually because we need to add
        // multiple `state` values and we no longer have a raw RequestBuilder
        // at this layer. Encode them directly into the URL.
        let mut path = format!("{}?pagelen=50", Self::pr_path(r, ""));
        for s in states {
            path.push_str(&format!("&state={s}"));
        }
        let v = self.send_json(reqwest::Method::GET, &path, None).await?;
        Ok(varr(&v, &["values"]).iter().map(summary_from).collect())
    }

    async fn get_pr(&self, r: &RemoteRef, number: u64) -> Result<PrDetail> {
        let pr = self
            .send_json(
                reqwest::Method::GET,
                &Self::pr_path(r, &format!("/{number}")),
                None,
            )
            .await?;
        let comments_v = self
            .send_json(
                reqwest::Method::GET,
                &format!(
                    "{}?pagelen=100",
                    Self::pr_path(r, &format!("/{number}/comments"))
                ),
                None,
            )
            .await?;

        // Thread by parent.id.
        let mut top: Vec<PrComment> = Vec::new();
        let mut replies: Vec<(String, PrComment)> = Vec::new();
        for c in varr(&comments_v, &["values"]) {
            if c.get("deleted").and_then(Value::as_bool).unwrap_or(false) {
                continue;
            }
            let pc = comment_from(c);
            match c
                .get("parent")
                .and_then(|p| p.get("id"))
                .and_then(Value::as_u64)
            {
                Some(parent) => replies.push((parent.to_string(), pc)),
                None => top.push(pc),
            }
        }
        for (parent, reply) in replies {
            if let Some(t) = top.iter_mut().find(|t| t.id == parent) {
                t.replies.push(reply);
            } else {
                top.push(reply);
            }
        }

        let approved_by: Vec<String> = varr(&pr, &["participants"])
            .iter()
            .filter(|p| p.get("approved").and_then(Value::as_bool).unwrap_or(false))
            .map(|p| vstr(p, &["user", "display_name"]))
            .collect();

        let reviewers: Vec<PrReviewer> = varr(&pr, &["participants"])
            .iter()
            .map(|p| PrReviewer {
                name: vstr(p, &["user", "display_name"]),
                approved: vbool(p, &["approved"]).unwrap_or(false),
                avatar_url: vstr_opt(p, &["user", "links", "avatar", "href"]),
                reviewed_at: vstr_opt(p, &["participated_on"]).map(|s| ts(&s)),
            })
            .collect();

        let description = {
            let d = vstr(&pr, &["description"]);
            if d.is_empty() {
                vstr(&pr, &["summary", "raw"])
            } else {
                d
            }
        };

        Ok(PrDetail {
            summary: summary_from(&pr),
            description_md: description,
            comments: top,
            approved_by,
            reviewers,
            // Bitbucket doesn't expose a mergeable flag on the PR object.
            mergeable: None,
        })
    }

    async fn get_pr_diff(&self, r: &RemoteRef, number: u64) -> Result<DiffResp> {
        // Returns a redirect to the raw unified diff; reqwest follows it.
        let text = self
            .send_text(
                reqwest::Method::GET,
                &Self::pr_path(r, &format!("/{number}/diff")),
            )
            .await?;
        Ok(crate::parse::parse_diff(&text))
    }

    async fn create_pr(&self, r: &RemoteRef, req: &CreatePrReq) -> Result<PrSummary> {
        let body = json!({
            "title": req.title,
            "description": req.description,
            "source": { "branch": { "name": req.source_branch } },
            "destination": { "branch": { "name": req.target_branch } },
        });
        let v = self
            .send_json(reqwest::Method::POST, &Self::pr_path(r, ""), Some(&body))
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
        let body_val = Value::Object(body);
        self.send(
            reqwest::Method::PUT,
            &Self::pr_path(r, &format!("/{number}")),
            Some(&body_val),
        )
        .await
        .map(|_| ())
    }

    async fn comment(&self, r: &RemoteRef, number: u64, c: &NewPrCommentReq) -> Result<PrComment> {
        let mut body = serde_json::Map::new();
        body.insert("content".into(), json!({ "raw": c.body }));
        if let Some(path) = &c.path {
            let mut inline = serde_json::Map::new();
            inline.insert("path".into(), json!(path));
            if let Some(line) = c.line {
                inline.insert("to".into(), json!(line));
            }
            body.insert("inline".into(), Value::Object(inline));
        }
        if let Some(reply_to) = &c.in_reply_to {
            let id: u64 = reply_to
                .parse()
                .map_err(|_| Error::Invalid(format!("bad comment id: {reply_to}")))?;
            body.insert("parent".into(), json!({ "id": id }));
        }
        let body_val = Value::Object(body);
        let v = self
            .send_json(
                reqwest::Method::POST,
                &Self::pr_path(r, &format!("/{number}/comments")),
                Some(&body_val),
            )
            .await?;
        Ok(comment_from(&v))
    }

    async fn approve(&self, r: &RemoteRef, number: u64) -> Result<()> {
        self.send(
            reqwest::Method::POST,
            &Self::pr_path(r, &format!("/{number}/approve")),
            None,
        )
        .await
        .map(|_| ())
    }

    async fn merge(&self, r: &RemoteRef, number: u64, strategy: MergeStrategy) -> Result<()> {
        let strat = match strategy {
            MergeStrategy::Merge => "merge_commit",
            MergeStrategy::Squash => "squash",
            // Bitbucket has no rebase-merge; fast_forward is the closest.
            MergeStrategy::Rebase => "fast_forward",
        };
        let body = json!({ "merge_strategy": strat });
        self.send(
            reqwest::Method::POST,
            &Self::pr_path(r, &format!("/{number}/merge")),
            Some(&body),
        )
        .await
        .map(|_| ())
    }

    async fn decline(&self, r: &RemoteRef, number: u64) -> Result<()> {
        self.send(
            reqwest::Method::POST,
            &Self::pr_path(r, &format!("/{number}/decline")),
            None,
        )
        .await
        .map(|_| ())
    }

    async fn request_changes(&self, r: &RemoteRef, number: u64, _body: Option<&str>) -> Result<()> {
        let path = Self::pr_path(r, &format!("/{number}/request-changes"));
        self.send(reqwest::Method::POST, &path, None)
            .await
            .map(|_| ())
    }

    async fn list_pr_commits(&self, r: &RemoteRef, number: u64) -> Result<Vec<PrCommit>> {
        let path = format!(
            "{}?pagelen=100",
            Self::pr_path(r, &format!("/{number}/commits"))
        );
        let v = self.send_json(reqwest::Method::GET, &path, None).await?;
        let commits = varr(&v, &["values"])
            .iter()
            .map(|c| {
                let sha = vstr(c, &["hash"]);
                let short_sha = sha.chars().take(8).collect();
                let message = vstr(c, &["message"]);
                let subject = message.lines().next().unwrap_or("").to_string();
                let author = {
                    let display = vstr(c, &["author", "user", "display_name"]);
                    if display.is_empty() {
                        vstr(c, &["author", "raw"])
                    } else {
                        display
                    }
                };
                let date = super::ts(&vstr(c, &["date"]));
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
        let mut url = format!("/repositories/{namespace}?pagelen=50&sort=-updated_on");
        if let Some(q) = query {
            if !q.is_empty() {
                url.push_str(&format!("&q=name~%22{}%22", percent_encode_query(q)));
            }
        }
        let v = self.send_json(reqwest::Method::GET, &url, None).await?;
        let repos = varr(&v, &["values"])
            .iter()
            .map(|r| {
                let full_name = vstr(r, &["full_name"]);
                let name = vstr(r, &["name"]);
                let description = vstr(r, &["description"]);
                let private = vbool(r, &["is_private"]).unwrap_or(false);
                let updated_at = vstr(r, &["updated_on"]);
                // clone links
                let clones = varr(r, &["links", "clone"]);
                let clone_url = clones
                    .iter()
                    .find(|c| vstr(c, &["name"]) == "https")
                    .map(|c| vstr(c, &["href"]))
                    .unwrap_or_default();
                let ssh_url = clones
                    .iter()
                    .find(|c| vstr(c, &["name"]) == "ssh")
                    .map(|c| vstr(c, &["href"]))
                    .unwrap_or_default();
                RemoteRepoSummary {
                    full_name,
                    name,
                    clone_url,
                    ssh_url,
                    description,
                    private,
                    updated_at,
                }
            })
            .collect();
        Ok(repos)
    }
}
