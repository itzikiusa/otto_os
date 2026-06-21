//! Jira Cloud / Server REST API v3 client.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use otto_core::domain::{IssueDetail, IssueProject, IssueSummary};
use otto_core::{Error, Result};

use crate::adf::{adf_to_markdown, text_to_adf};

/// How long to wait for the TCP/TLS connection to a Jira endpoint to establish.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Overall per-request deadline. A hung Jira endpoint must not block the caller
/// (and stack up in the watcher) indefinitely — bound every call.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Process-wide cache: auth_header (already encodes base_url+email+token) → shared client.
/// `reqwest::Client` is `Clone` and internally reference-counted — cheap to clone out.
static CLIENT_CACHE: OnceLock<Mutex<HashMap<String, reqwest::Client>>> = OnceLock::new();

/// Return a cached `reqwest::Client` for the given auth_header, building one on first use.
/// Falls back to a freshly-built client if the cache lock is poisoned (keeps callers infallible).
fn get_or_build_client(auth_header: &str) -> reqwest::Client {
    let cache = CLIENT_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut map) = cache.lock() {
        if let Some(c) = map.get(auth_header) {
            return c.clone();
        }
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .unwrap_or_default();
        map.insert(auth_header.to_string(), client.clone());
        return client;
    }
    // Poisoned lock — fall back to a fresh client rather than panicking.
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .unwrap_or_default()
}

/// A lightweight reference to a comment that was just created.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CommentRef {
    pub id: String,
    pub url: Option<String>,
}

/// A Jira issue comment with its body rendered as Markdown.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IssueComment {
    pub id: String,
    pub author: String,
    pub body_md: String,
    pub created: String,
}

/// A Jira user (assignee, reporter, commenter, etc.)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JiraUser {
    pub account_id: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
}

/// A generic issue field, with a display name and a stringified value.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JiraField {
    /// The raw Jira field key (e.g. `"customfield_10016"`).
    pub key: String,
    /// Human-readable display name sourced from `?expand=names`.
    pub name: String,
    /// Stringified field value (never empty — caller skips nulls/empties).
    pub value: String,
}

/// An attachment on a Jira issue.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JiraAttachment {
    pub id: String,
    pub filename: String,
    pub mime: String,
    pub size: i64,
    pub created: String,
    pub author: String,
}

/// A link to another issue (or epic / parent / subtask relationship).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JiraLink {
    /// Relationship label e.g. "blocks", "is blocked by", "Epic", "Parent", "Subtask".
    pub rel: String,
    /// Linked issue key (e.g. "PROJ-42").
    pub key: String,
    pub summary: String,
    pub status: String,
    pub issue_type: String,
}

/// A status transition available for an issue.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JiraTransition {
    pub id: String,
    pub name: String,
    pub to_status: String,
}

/// A single entry in the issue changelog.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JiraChangelogEntry {
    pub author: String,
    pub created: String,
    pub items: Vec<JiraChangeItem>,
}

/// One field change within a changelog entry.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JiraChangeItem {
    pub field: String,
    pub from: String,
    pub to: String,
}

/// Full issue detail — everything the Product section needs.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IssueFull {
    pub key: String,
    pub summary: String,
    pub status: String,
    pub issue_type: String,
    pub url: String,
    pub description_md: String,
    pub assignee: Option<JiraUser>,
    pub reporter: Option<JiraUser>,
    pub priority: Option<String>,
    pub labels: Vec<String>,
    /// All non-empty fields including custom ones (display label + stringified value).
    pub fields: Vec<JiraField>,
    pub comments: Vec<IssueComment>,
    pub history: Vec<JiraChangelogEntry>,
    pub attachments: Vec<JiraAttachment>,
    pub links: Vec<JiraLink>,
    /// Story points or time estimate, e.g. "5 pts" or "1d 2h".
    pub estimate: Option<String>,
}

/// The result of a successful issue creation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CreatedIssue {
    pub key: String,
    pub url: String,
}

/// Build the JSON body for `POST /rest/api/3/issue`.
///
/// Factored out as a pure function so it can be unit-tested without network.
pub fn build_create_issue_body(
    project_key: &str,
    issue_type: &str,
    summary: &str,
    adf: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "fields": {
            "project": { "key": project_key },
            "issuetype": { "name": issue_type },
            "summary": summary,
            "description": adf
        }
    })
}

/// A thin Jira REST client scoped to one account.
pub struct JiraClient {
    base_url: String,
    auth_header: String,
    http: reqwest::Client,
}

impl JiraClient {
    pub fn new(base_url: &str, email: &str, token: &str) -> Self {
        let base_url = base_url.trim_end_matches('/').to_string();
        let credentials = format!("{email}:{token}");
        let auth_header = format!("Basic {}", B64.encode(credentials.as_bytes()));
        // Reuse a cached client keyed by the auth header (encodes base_url + credentials).
        let http = get_or_build_client(&format!("{base_url}\x00{auth_header}"));
        Self {
            base_url,
            auth_header,
            http,
        }
    }

    /// List projects available in the Jira instance, ordered by name.
    /// Uses `/rest/api/3/project/search` (Cloud); falls back to `/rest/api/3/project`
    /// (Server / older Cloud) if the paginated endpoint 404s.
    pub async fn list_projects(&self) -> Result<Vec<IssueProject>> {
        let search_url = format!("{}/rest/api/3/project/search", self.base_url);
        let resp = self
            .http
            .get(&search_url)
            .header("Authorization", &self.auth_header)
            .header("Accept", "application/json")
            .query(&[("maxResults", "100"), ("orderBy", "name")])
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("jira project/search request: {e}")))?;

        let (is_paginated, resp) = if resp.status() == reqwest::StatusCode::NOT_FOUND {
            // Fall back to bare array endpoint.
            let fallback_url = format!("{}/rest/api/3/project", self.base_url);
            let r = self
                .http
                .get(&fallback_url)
                .header("Authorization", &self.auth_header)
                .header("Accept", "application/json")
                .query(&[("maxResults", "100"), ("orderBy", "name")])
                .send()
                .await
                .map_err(|e| Error::Upstream(format!("jira project list fallback request: {e}")))?;
            (false, r)
        } else {
            (true, resp)
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream(format!(
                "jira project list failed ({status}): {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Upstream(format!("jira project list parse: {e}")))?;

        // Paginated response: { values: [...] }; bare response: [...]
        let projects_arr: &[serde_json::Value] = if is_paginated {
            body.get("values")
                .and_then(|v| v.as_array())
                .map(|a| a.as_slice())
                .unwrap_or(&[])
        } else {
            body.as_array().map(|a| a.as_slice()).unwrap_or(&[])
        };

        let mut results = Vec::with_capacity(projects_arr.len());
        for p in projects_arr {
            let key = p
                .get("key")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let name = p
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !key.is_empty() {
                results.push(IssueProject { key, name });
            }
        }
        Ok(results)
    }

    /// Search for issues. Uses `/rest/api/3/search/jql` first; falls back to
    /// `/rest/api/3/search` if the new endpoint is not available.
    ///
    /// When `project` is `Some(key)`, the JQL is scoped to that project and
    /// the query `q` is interpreted as:
    ///   - all-digits → matches by number (`KEY-<q>`) OR text
    ///   - full key pattern → matches by key exactly
    ///   - otherwise → text/summary search within the project
    ///
    /// When `project` is `None`, the original unrestricted behaviour is kept.
    /// `start_at` enables cursor-style pagination ("load more"). Pass `0` for the
    /// first page. An empty `q` uses the recency-default JQL so the picker is useful
    /// without the user having typed anything.
    pub async fn search(
        &self,
        q: &str,
        project: Option<&str>,
        start_at: u32,
    ) -> Result<Vec<IssueSummary>> {
        let jql = build_jql(q, project);
        self.search_jql(&jql, start_at).await
    }

    /// Run a raw, caller-built JQL query (e.g. `project = X AND issuetype = Story
    /// AND assignee = "..."`) and return brief issue summaries. Used by analytics
    /// plugins that need precise JQL control beyond `search`'s text/key builder.
    pub async fn search_jql(&self, jql: &str, start_at: u32) -> Result<Vec<IssueSummary>> {
        let fields = "summary,status,issuetype";
        let max_results = "25";
        let start_at_s = start_at.to_string();

        // Try the newer endpoint first.
        let new_url = format!("{}/rest/api/3/search/jql", self.base_url);
        let resp = self
            .http
            .get(&new_url)
            .header("Authorization", &self.auth_header)
            .header("Accept", "application/json")
            .query(&[
                ("jql", jql),
                ("maxResults", max_results),
                ("startAt", start_at_s.as_str()),
                ("fields", fields),
            ])
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("jira search request: {e}")))?;

        let resp = if resp.status().is_success() {
            resp
        } else {
            // Fall back to the classic endpoint.
            let fallback_url = format!("{}/rest/api/3/search", self.base_url);
            self.http
                .get(&fallback_url)
                .header("Authorization", &self.auth_header)
                .header("Accept", "application/json")
                .query(&[
                    ("jql", jql),
                    ("maxResults", max_results),
                    ("startAt", start_at_s.as_str()),
                    ("fields", fields),
                ])
                .send()
                .await
                .map_err(|e| Error::Upstream(format!("jira search fallback request: {e}")))?
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream(format!(
                "jira search failed ({status}): {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Upstream(format!("jira search parse: {e}")))?;

        let issues = body
            .get("issues")
            .and_then(|v| v.as_array())
            .map(|a| a.as_slice())
            .unwrap_or(&[]);

        let mut results = Vec::with_capacity(issues.len());
        for issue in issues {
            let key = issue
                .get("key")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let fields = issue.get("fields").cloned().unwrap_or_default();
            let summary = fields
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let status = fields
                .get("status")
                .and_then(|s| s.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let issue_type = fields
                .get("issuetype")
                .and_then(|s| s.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let url = format!("{}/browse/{}", self.base_url, key);
            results.push(IssueSummary {
                key,
                summary,
                status,
                issue_type,
                url,
            });
        }
        Ok(results)
    }

    /// Fetch a single issue by key (e.g. "PROJ-123").
    pub async fn get_issue(&self, key: &str) -> Result<IssueDetail> {
        let url = format!("{}/rest/api/3/issue/{}", self.base_url, key);
        let resp = self
            .http
            .get(&url)
            .header("Authorization", &self.auth_header)
            .header("Accept", "application/json")
            .query(&[("fields", "summary,status,issuetype,assignee,description")])
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("jira issue request: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream(format!(
                "jira issue {key} failed ({status}): {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Upstream(format!("jira issue parse: {e}")))?;

        let fields = body.get("fields").cloned().unwrap_or_default();

        let summary = fields
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let status = fields
            .get("status")
            .and_then(|s| s.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let issue_type = fields
            .get("issuetype")
            .and_then(|s| s.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let assignee = fields
            .get("assignee")
            .and_then(|a| a.get("displayName"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        // Jira ADF description is a JSON object — convert it to Markdown.
        let description = match fields.get("description") {
            Some(desc) if !desc.is_null() => adf_to_markdown(desc),
            _ => String::new(),
        };
        let url = format!("{}/browse/{}", self.base_url, key);

        Ok(IssueDetail {
            key: key.to_string(),
            summary,
            status,
            issue_type,
            url,
            description,
            assignee,
        })
    }

    /// Post a comment on an issue. The `body_text` is converted to ADF before sending.
    ///
    /// Returns a [`CommentRef`] with the new comment's `id` and optional `self` URL.
    pub async fn add_comment(&self, key: &str, body_text: &str) -> Result<CommentRef> {
        let url = format!("{}/rest/api/3/issue/{}/comment", self.base_url, key);
        let adf_body = text_to_adf(body_text);
        let payload = serde_json::json!({ "body": adf_body });

        let resp = self
            .http
            .post(&url)
            .header("Authorization", &self.auth_header)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("jira add_comment request: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream(format!(
                "jira add_comment {key} failed ({status}): {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Upstream(format!("jira add_comment parse: {e}")))?;

        let id = body
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let comment_url = body
            .get("self")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Ok(CommentRef {
            id,
            url: comment_url,
        })
    }

    /// List comments on an issue, ordered by creation time (oldest first).
    ///
    /// Each comment's ADF body is converted to Markdown via [`adf_to_markdown`].
    pub async fn list_comments(&self, key: &str) -> Result<Vec<IssueComment>> {
        let url = format!("{}/rest/api/3/issue/{}/comment", self.base_url, key);

        let resp = self
            .http
            .get(&url)
            .header("Authorization", &self.auth_header)
            .header("Accept", "application/json")
            .query(&[("orderBy", "created")])
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("jira list_comments request: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream(format!(
                "jira list_comments {key} failed ({status}): {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Upstream(format!("jira list_comments parse: {e}")))?;

        let comments = body
            .get("comments")
            .and_then(|v| v.as_array())
            .map(|a| a.as_slice())
            .unwrap_or(&[]);

        let mut results = Vec::with_capacity(comments.len());
        for c in comments {
            let id = c
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let author = c
                .get("author")
                .and_then(|a| a.get("displayName"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let created = c
                .get("created")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            // The body is ADF; convert to markdown. Fall back to empty string if absent.
            let body_md = match c.get("body") {
                Some(adf) if !adf.is_null() => adf_to_markdown(adf),
                _ => String::new(),
            };
            results.push(IssueComment {
                id,
                author,
                body_md,
                created,
            });
        }
        Ok(results)
    }

    /// Fetch the full issue detail: description, comments, changelog, attachments, links,
    /// all non-empty fields with display names.
    ///
    /// Uses `GET /rest/api/3/issue/{key}?expand=changelog,names,renderedFields&fields=*all`
    pub async fn get_issue_full(&self, key: &str) -> Result<IssueFull> {
        let url = format!("{}/rest/api/3/issue/{}", self.base_url, key);
        let resp = self
            .http
            .get(&url)
            .header("Authorization", &self.auth_header)
            .header("Accept", "application/json")
            .query(&[
                ("expand", "changelog,names,renderedFields"),
                ("fields", "*all"),
            ])
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("jira get_issue_full request: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream(format!(
                "jira issue full {key} failed ({status}): {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Upstream(format!("jira get_issue_full parse: {e}")))?;

        let names = body.get("names").cloned().unwrap_or_default();
        Ok(parse_issue_full(&body, &names, &self.base_url))
    }

    /// List the available status transitions for an issue.
    ///
    /// Uses `GET /rest/api/3/issue/{key}/transitions`
    pub async fn list_transitions(&self, key: &str) -> Result<Vec<JiraTransition>> {
        let url = format!("{}/rest/api/3/issue/{}/transitions", self.base_url, key);
        let resp = self
            .http
            .get(&url)
            .header("Authorization", &self.auth_header)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("jira list_transitions request: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream(format!(
                "jira list_transitions {key} failed ({status}): {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Upstream(format!("jira list_transitions parse: {e}")))?;

        Ok(parse_transitions(&body))
    }

    /// Transition an issue to a new status.
    ///
    /// Uses `POST /rest/api/3/issue/{key}/transitions` with `{"transition":{"id":"..."}}`
    pub async fn transition_issue(&self, key: &str, transition_id: &str) -> Result<()> {
        let url = format!("{}/rest/api/3/issue/{}/transitions", self.base_url, key);
        let payload = serde_json::json!({ "transition": { "id": transition_id } });

        let resp = self
            .http
            .post(&url)
            .header("Authorization", &self.auth_header)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("jira transition_issue request: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream(format!(
                "jira transition_issue {key} failed ({status}): {body}"
            )));
        }

        Ok(())
    }

    /// List users assignable to an issue.
    ///
    /// Uses `GET /rest/api/3/user/assignable/search?issueKey={key}`
    pub async fn list_assignable(&self, key: &str) -> Result<Vec<JiraUser>> {
        let url = format!("{}/rest/api/3/user/assignable/search", self.base_url);
        let resp = self
            .http
            .get(&url)
            .header("Authorization", &self.auth_header)
            .header("Accept", "application/json")
            .query(&[("issueKey", key), ("maxResults", "50")])
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("jira list_assignable request: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream(format!(
                "jira list_assignable {key} failed ({status}): {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Upstream(format!("jira list_assignable parse: {e}")))?;

        let users = body.as_array().map(|a| a.as_slice()).unwrap_or(&[]);
        Ok(users.iter().map(parse_user).collect())
    }

    /// Assign an issue to a user (or unassign by passing `"-1"`).
    ///
    /// Uses `PUT /rest/api/3/issue/{key}/assignee` with `{"accountId":"..."}`
    pub async fn assign_issue(&self, key: &str, account_id: &str) -> Result<()> {
        let url = format!("{}/rest/api/3/issue/{}/assignee", self.base_url, key);
        let payload = serde_json::json!({ "accountId": account_id });

        let resp = self
            .http
            .put(&url)
            .header("Authorization", &self.auth_header)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("jira assign_issue request: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream(format!(
                "jira assign_issue {key} failed ({status}): {body}"
            )));
        }

        Ok(())
    }

    /// Fetch the raw bytes of an attachment, returning `(content_type, bytes)`.
    ///
    /// Uses `GET /rest/api/3/attachment/content/{attachment_id}`.
    /// The Jira Cloud REST v3 endpoint redirects to the CDN; reqwest follows redirects
    /// automatically but the Authorization header must be sent on the initial request.
    /// We include it on all hops via `connection()`'s default redirect policy.
    pub async fn attachment_bytes(&self, attachment_id: &str) -> Result<(String, Vec<u8>)> {
        let url = format!(
            "{}/rest/api/3/attachment/content/{}",
            self.base_url, attachment_id
        );
        let resp = self
            .http
            .get(&url)
            .header("Authorization", &self.auth_header)
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("jira attachment request: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream(format!(
                "jira attachment {attachment_id} failed ({status}): {body}"
            )));
        }

        let mime = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| Error::Upstream(format!("jira attachment read: {e}")))?;

        Ok((mime, bytes.to_vec()))
    }

    /// Update the description of an issue. The `body_md` text is converted to ADF.
    pub async fn update_description(&self, key: &str, body_md: &str) -> Result<()> {
        let url = format!("{}/rest/api/3/issue/{}", self.base_url, key);
        let adf_body = text_to_adf(body_md);
        let payload = serde_json::json!({
            "fields": {
                "description": adf_body
            }
        });

        let resp = self
            .http
            .put(&url)
            .header("Authorization", &self.auth_header)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("jira update_description request: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream(format!(
                "jira update_description {key} failed ({status}): {body}"
            )));
        }

        Ok(())
    }

    /// Create a new issue.
    ///
    /// Uses `POST /rest/api/3/issue` with the body built by
    /// [`build_create_issue_body`].  Returns a [`CreatedIssue`] containing the
    /// new issue key and its browse URL.
    pub async fn create_issue(
        &self,
        project_key: &str,
        issue_type: &str,
        summary: &str,
        description_md: &str,
    ) -> Result<CreatedIssue> {
        let url = format!("{}/rest/api/3/issue", self.base_url);
        let adf = text_to_adf(description_md);
        let payload = build_create_issue_body(project_key, issue_type, summary, adf);

        let resp = self
            .http
            .post(&url)
            .header("Authorization", &self.auth_header)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("jira create_issue request: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream(format!(
                "jira create_issue failed ({status}): {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Upstream(format!("jira create_issue parse: {e}")))?;

        let key = body
            .get("key")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let issue_url = format!("{}/browse/{}", self.base_url, key);

        Ok(CreatedIssue {
            key,
            url: issue_url,
        })
    }

    /// List the issue types available in a project, filtering out sub-task types.
    ///
    /// Uses `GET /rest/api/3/project/{project_key}` and reads
    /// `issueTypes[].name` from the response.  Sub-task types (where
    /// `subtask == true`) are excluded so callers only see top-level types
    /// such as "Story", "Task", "Bug", and "Epic".
    pub async fn list_issue_types(&self, project_key: &str) -> Result<Vec<String>> {
        let url = format!("{}/rest/api/3/project/{}", self.base_url, project_key);
        let resp = self
            .http
            .get(&url)
            .header("Authorization", &self.auth_header)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("jira list_issue_types request: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream(format!(
                "jira list_issue_types {project_key} failed ({status}): {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Upstream(format!("jira list_issue_types parse: {e}")))?;

        let types = body
            .get("issueTypes")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter(|t| {
                        // Filter out subtask types
                        !t.get("subtask")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                    })
                    .filter_map(|t| {
                        t.get("name")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(types)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Pure parsing helpers (testable without network)
// ─────────────────────────────────────────────────────────────────────────────

/// Parse a Jira user node `{accountId, displayName, avatarUrls}` into [`JiraUser`].
fn parse_user(v: &serde_json::Value) -> JiraUser {
    let account_id = v
        .get("accountId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let display_name = v
        .get("displayName")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    // Prefer 48×48 avatar; fall back to any size.
    let avatar_url = v
        .get("avatarUrls")
        .and_then(|a| {
            a.get("48x48")
                .or_else(|| a.get("32x32"))
                .or_else(|| a.get("24x24"))
                .or_else(|| a.get("16x16"))
        })
        .and_then(|v| v.as_str())
        .map(str::to_string);
    JiraUser {
        account_id,
        display_name,
        avatar_url,
    }
}

/// Stringify a Jira field value to a human-readable string.
///
/// Priority: `displayName` → `name` → `value` → `key` → compact JSON.
pub(crate) fn stringify_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => String::new(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => {
            // For whole-number floats (e.g. story points = 8.0), strip the ".0"
            // so the value renders as "8" rather than "8.0".
            if let Some(f) = n.as_f64() {
                if f.fract() == 0.0 && f.abs() < 1e15 {
                    return format!("{}", f as i64);
                }
            }
            n.to_string()
        }
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => {
            let parts: Vec<String> = arr
                .iter()
                .filter(|e| !e.is_null())
                .map(stringify_value)
                .filter(|s| !s.is_empty())
                .collect();
            parts.join(", ")
        }
        serde_json::Value::Object(_) => {
            // Try common display keys in priority order.
            for key in &["displayName", "name", "value", "key"] {
                if let Some(s) = v.get(key).and_then(|vv| vv.as_str()) {
                    if !s.is_empty() {
                        return s.to_string();
                    }
                }
            }
            // Compact JSON fallback.
            v.to_string()
        }
    }
}

/// Parse a `transitions` response body into a list of [`JiraTransition`].
pub(crate) fn parse_transitions(body: &serde_json::Value) -> Vec<JiraTransition> {
    let arr = body
        .get("transitions")
        .and_then(|v| v.as_array())
        .map(|a| a.as_slice())
        .unwrap_or(&[]);

    arr.iter()
        .filter_map(|t| {
            let id = t.get("id")?.as_str()?.to_string();
            let name = t
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let to_status = t
                .get("to")
                .and_then(|s| s.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(JiraTransition {
                id,
                name,
                to_status,
            })
        })
        .collect()
}

/// Parse the full issue JSON (from `?expand=changelog,names,renderedFields&fields=*all`)
/// into an [`IssueFull`].
///
/// `names` is the sibling `names` map `{fieldKey → displayLabel}` returned alongside
/// the issue when `expand=names` is used.
pub fn parse_issue_full(
    body: &serde_json::Value,
    names: &serde_json::Value,
    base_url: &str,
) -> IssueFull {
    let key = body
        .get("key")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let url = format!("{}/browse/{}", base_url, key);

    let fields = body.get("fields").cloned().unwrap_or_default();

    let summary = fields
        .get("summary")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let status = fields
        .get("status")
        .and_then(|s| s.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let issue_type = fields
        .get("issuetype")
        .and_then(|s| s.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let description_md = match fields.get("description") {
        Some(desc) if !desc.is_null() => adf_to_markdown(desc),
        _ => String::new(),
    };

    let assignee = fields
        .get("assignee")
        .filter(|v| !v.is_null())
        .map(parse_user);

    let reporter = fields
        .get("reporter")
        .filter(|v| !v.is_null())
        .map(parse_user);

    let priority = fields
        .get("priority")
        .and_then(|p| p.get("name"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let labels: Vec<String> = fields
        .get("labels")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    // ── Attachments ──────────────────────────────────────────────────────────
    let attachments: Vec<JiraAttachment> = fields
        .get("attachment")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|a| {
                    let id = a.get("id")?.as_str()?.to_string();
                    let filename = a
                        .get("filename")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let mime = a
                        .get("mimeType")
                        .and_then(|v| v.as_str())
                        .unwrap_or("application/octet-stream")
                        .to_string();
                    let size = a
                        .get("size")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let created = a
                        .get("created")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let author = a
                        .get("author")
                        .and_then(|au| au.get("displayName"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    Some(JiraAttachment {
                        id,
                        filename,
                        mime,
                        size,
                        created,
                        author,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // ── Links (issuelinks + parent + subtasks + epic) ────────────────────────
    let mut links: Vec<JiraLink> = Vec::new();

    // issuelinks
    if let Some(arr) = fields.get("issuelinks").and_then(|v| v.as_array()) {
        for link in arr {
            let link_type = link.get("type");
            let inward_name = link_type
                .and_then(|t| t.get("inward"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let outward_name = link_type
                .and_then(|t| t.get("outward"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // A link has either inwardIssue or outwardIssue (not both)
            if let Some(issue) = link.get("inwardIssue") {
                if let Some(lk) = extract_linked_issue(issue, &inward_name) {
                    links.push(lk);
                }
            }
            if let Some(issue) = link.get("outwardIssue") {
                if let Some(lk) = extract_linked_issue(issue, &outward_name) {
                    links.push(lk);
                }
            }
        }
    }

    // parent
    if let Some(parent) = fields.get("parent").filter(|v| !v.is_null()) {
        if let Some(lk) = extract_linked_issue(parent, "Parent") {
            links.push(lk);
        }
    }

    // epic link (classic `customfield_10014` string key)
    if let Some(epic_key) = fields
        .get("customfield_10014")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        links.push(JiraLink {
            rel: "Epic".to_string(),
            key: epic_key.to_string(),
            summary: String::new(),
            status: String::new(),
            issue_type: "Epic".to_string(),
        });
    }

    // subtasks
    if let Some(arr) = fields.get("subtasks").and_then(|v| v.as_array()) {
        for sub in arr {
            if let Some(lk) = extract_linked_issue(sub, "Subtask") {
                links.push(lk);
            }
        }
    }

    // ── Comments ─────────────────────────────────────────────────────────────
    let comments: Vec<IssueComment> = fields
        .get("comment")
        .and_then(|c| c.get("comments"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|c| {
                    let id = c
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let author = c
                        .get("author")
                        .and_then(|a| a.get("displayName"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let created = c
                        .get("created")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let body_md = match c.get("body") {
                        Some(adf) if !adf.is_null() => adf_to_markdown(adf),
                        _ => String::new(),
                    };
                    IssueComment {
                        id,
                        author,
                        body_md,
                        created,
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    // ── Changelog ────────────────────────────────────────────────────────────
    let history: Vec<JiraChangelogEntry> = body
        .get("changelog")
        .and_then(|cl| cl.get("histories"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|entry| {
                    let author = entry
                        .get("author")
                        .and_then(|a| a.get("displayName"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let created = entry
                        .get("created")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let items: Vec<JiraChangeItem> = entry
                        .get("items")
                        .and_then(|v| v.as_array())
                        .map(|items| {
                            items
                                .iter()
                                .map(|item| {
                                    let field = item
                                        .get("field")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let from = item
                                        .get("fromString")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let to = item
                                        .get("toString")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    JiraChangeItem { field, from, to }
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    JiraChangelogEntry {
                        author,
                        created,
                        items,
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    // ── Estimate (story points first, then time tracking) ────────────────────
    let estimate: Option<String> = {
        // 1. Story points: scan all fields whose display name (from `names`) contains
        //    "story point" (case-insensitive), and take the first numeric value.
        let mut sp: Option<String> = None;
        if let Some(fields_obj) = fields.as_object() {
            for (fkey, fval) in fields_obj {
                let display_name = names
                    .get(fkey)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_lowercase();
                if display_name.contains("story point") {
                    if let Some(n) = fval.as_f64() {
                        // Drop trailing ".0" for whole numbers.
                        let formatted = if n.fract() == 0.0 {
                            format!("{} pts", n as i64)
                        } else {
                            format!("{} pts", n)
                        };
                        sp = Some(formatted);
                        break;
                    }
                }
            }
        }

        if sp.is_some() {
            sp
        } else {
            // 2. timetracking.originalEstimate (human-readable string, e.g. "2d 4h")
            let tt_orig = fields
                .get("timetracking")
                .and_then(|tt| tt.get("originalEstimate"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string);

            if tt_orig.is_some() {
                tt_orig
            } else {
                // 3. timeoriginalestimate (seconds integer)
                let secs_opt = fields
                    .get("timeoriginalestimate")
                    .and_then(|v| v.as_i64())
                    .filter(|&s| s > 0)
                    .or_else(|| {
                        fields
                            .get("aggregatetimeoriginalestimate")
                            .and_then(|v| v.as_i64())
                            .filter(|&s| s > 0)
                    });

                secs_opt.map(|secs| {
                    // 8h work day = 28800 seconds
                    let days = secs / 28800;
                    let rem = secs % 28800;
                    let hours = rem / 3600;
                    let mins = (rem % 3600) / 60;
                    let mut parts = Vec::new();
                    if days > 0 {
                        parts.push(format!("{}d", days));
                    }
                    if hours > 0 {
                        parts.push(format!("{}h", hours));
                    }
                    if mins > 0 {
                        parts.push(format!("{}m", mins));
                    }
                    if parts.is_empty() {
                        format!("{}s", secs)
                    } else {
                        parts.join(" ")
                    }
                })
            }
        }
    };

    // ── Generic fields (all non-empty, with display name from `names`) ───────
    let skip_keys: std::collections::HashSet<&str> = [
        "summary",
        "description",
        "status",
        "issuetype",
        "assignee",
        "reporter",
        "priority",
        "labels",
        "attachment",
        "issuelinks",
        "parent",
        "subtasks",
        "comment",
        "customfield_10014", // epic link (handled above)
        "worklog",
        "watches",
        "votes",
        "timetracking",
        "aggregateprogress",
        "progress",
        "creator",
    ]
    .into_iter()
    .collect();

    let mut extra_fields: Vec<JiraField> = Vec::new();
    if let Some(fields_obj) = fields.as_object() {
        for (fkey, fval) in fields_obj {
            if skip_keys.contains(fkey.as_str()) || fval.is_null() {
                continue;
            }
            let stringified = stringify_value(fval);
            if stringified.is_empty() {
                continue;
            }
            // Look up the display name from the `names` map (expand=names).
            let display_name = names
                .get(fkey)
                .and_then(|v| v.as_str())
                .unwrap_or(fkey.as_str())
                .to_string();
            extra_fields.push(JiraField {
                key: fkey.clone(),
                name: display_name,
                value: stringified,
            });
        }
    }
    // Sort for deterministic output.
    extra_fields.sort_by(|a, b| a.name.cmp(&b.name));

    IssueFull {
        key,
        summary,
        status,
        issue_type,
        url,
        description_md,
        assignee,
        reporter,
        priority,
        labels,
        fields: extra_fields,
        comments,
        history,
        attachments,
        links,
        estimate,
    }
}

/// Extract a [`JiraLink`] from a linked-issue sub-object.
fn extract_linked_issue(issue: &serde_json::Value, rel: &str) -> Option<JiraLink> {
    let key = issue.get("key")?.as_str()?.to_string();
    let f = issue.get("fields").cloned().unwrap_or_default();
    let summary = f
        .get("summary")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let status = f
        .get("status")
        .and_then(|s| s.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let issue_type = f
        .get("issuetype")
        .and_then(|s| s.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Some(JiraLink {
        rel: rel.to_string(),
        key,
        summary,
        status,
        issue_type,
    })
}

/// Build a JQL query from a search string and optional project key.
///
/// Empty `q` (no typed search) → recency default: issues assigned to the current
/// user ordered by last-updated, so the picker is useful before any text is typed.
///
/// When `project` is `Some(key)`:
///   - all-digits `q` → `project = "KEY" AND (key = "KEY-q" OR summary ~ "q*" OR text ~ "q")`
///   - full issue-key `q` → `project = "KEY" AND key = "q"`
///   - text `q` → `project = "KEY" AND (summary ~ "q*" OR text ~ "q")`
///
/// When `project` is `None`, falls back to the original unrestricted behaviour.
fn build_jql(q: &str, project: Option<&str>) -> String {
    // Empty query: return recently-updated issues for the current user so the
    // picker has content before any typing.
    if q.trim().is_empty() {
        let base = "assignee = currentUser() ORDER BY updated DESC";
        return match project {
            Some(proj) => {
                let ep = escape_jql(proj);
                format!("project = \"{ep}\" AND {base}")
            }
            None => base.to_string(),
        };
    }

    let eq = escape_jql(q);
    match project {
        Some(proj) => {
            let ep = escape_jql(proj);
            let inner = if q.chars().all(|c| c.is_ascii_digit()) && !q.is_empty() {
                // Numeric-only: likely a ticket number.
                format!("(key = \"{ep}-{eq}\" OR summary ~ \"{eq}*\" OR text ~ \"{eq}\")")
            } else if is_issue_key(q) {
                // Full key typed explicitly.
                format!("key = \"{eq}\"")
            } else {
                // Name / text search within the project.
                format!("(summary ~ \"{eq}*\" OR text ~ \"{eq}\")")
            };
            format!("project = \"{ep}\" AND {inner} ORDER BY updated DESC")
        }
        None => {
            // Original behaviour: key match or full-text search.
            if is_issue_key(q) {
                format!("key = \"{eq}\" ORDER BY updated DESC")
            } else {
                format!("text ~ \"{eq}\" OR summary ~ \"{eq}*\" ORDER BY updated DESC")
            }
        }
    }
}

fn is_issue_key(s: &str) -> bool {
    // ^[A-Z][A-Z0-9]+-\d+$
    let mut chars = s.chars();
    // Must start with uppercase letter
    match chars.next() {
        Some(c) if c.is_ascii_uppercase() => {}
        _ => return false,
    }
    let mut found_dash = false;
    let mut after_dash_digits = 0usize;
    for c in chars {
        if !found_dash {
            if c == '-' {
                found_dash = true;
            } else if !c.is_ascii_alphanumeric() || !c.is_ascii_uppercase() && !c.is_ascii_digit() {
                return false;
            }
        } else {
            if c.is_ascii_digit() {
                after_dash_digits += 1;
            } else {
                return false;
            }
        }
    }
    found_dash && after_dash_digits > 0
}

/// Escape double quotes in a JQL string value.
fn escape_jql(s: &str) -> String {
    s.replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_issue_key() {
        assert!(is_issue_key("PROJ-123"));
        assert!(is_issue_key("AB-1"));
        assert!(!is_issue_key("proj-123"));
        assert!(!is_issue_key("hello world"));
        assert!(!is_issue_key("PROJ"));
        assert!(!is_issue_key("PROJ-"));
    }

    #[test]
    fn test_build_jql_no_project() {
        // Key pattern → key = "..." query
        let jql = build_jql("PROJ-123", None);
        assert!(jql.contains("key = \"PROJ-123\""), "{jql}");
        // Text search
        let jql = build_jql("login bug", None);
        assert!(jql.contains("text ~"), "{jql}");
        assert!(jql.contains("summary ~"), "{jql}");
    }

    #[test]
    fn test_build_jql_with_project_digits() {
        // All-digits → number lookup within project
        let jql = build_jql("5218", Some("GRV"));
        assert!(jql.starts_with("project = \"GRV\""), "{jql}");
        assert!(jql.contains("key = \"GRV-5218\""), "{jql}");
    }

    #[test]
    fn test_build_jql_with_project_key() {
        // Full key → key match only
        let jql = build_jql("GRV-5218", Some("GRV"));
        assert!(jql.contains("key = \"GRV-5218\""), "{jql}");
    }

    #[test]
    fn test_build_jql_with_project_text() {
        // Text search within project
        let jql = build_jql("login issue", Some("GRV"));
        assert!(jql.starts_with("project = \"GRV\""), "{jql}");
        assert!(jql.contains("summary ~"), "{jql}");
    }

    // ── parse_issue_full tests ───────────────────────────────────────────────

    /// Build a realistic canned Jira issue JSON fixture (what you get from
    /// `GET /rest/api/3/issue/PROJ-123?expand=changelog,names,renderedFields&fields=*all`)
    fn canned_issue_json() -> (serde_json::Value, serde_json::Value) {
        let body = serde_json::json!({
            "key": "PROJ-123",
            "fields": {
                "summary": "Login page crashes on mobile",
                "status": {"name": "In Progress"},
                "issuetype": {"name": "Bug"},
                "description": {
                    "type": "doc",
                    "version": 1,
                    "content": [{
                        "type": "paragraph",
                        "content": [{"type": "text", "text": "Steps to reproduce the crash."}]
                    }]
                },
                "assignee": {
                    "accountId": "acc-1",
                    "displayName": "Alice Smith",
                    "avatarUrls": {"48x48": "https://cdn.example.com/alice.png"}
                },
                "reporter": {
                    "accountId": "acc-2",
                    "displayName": "Bob Jones",
                    "avatarUrls": {"48x48": "https://cdn.example.com/bob.png"}
                },
                "priority": {"name": "High"},
                "labels": ["mobile", "crash"],
                // Custom field — should appear in `fields`
                "customfield_10016": 8.0,
                // Attachment
                "attachment": [{
                    "id": "att-99",
                    "filename": "screenshot.png",
                    "mimeType": "image/png",
                    "size": 45678_i64,
                    "created": "2024-01-15T10:00:00.000+0000",
                    "author": {"displayName": "Alice Smith"}
                }],
                // Issue link
                "issuelinks": [{
                    "type": {"inward": "is blocked by", "outward": "blocks"},
                    "outwardIssue": {
                        "key": "PROJ-100",
                        "fields": {
                            "summary": "Auth service down",
                            "status": {"name": "Open"},
                            "issuetype": {"name": "Bug"}
                        }
                    }
                }],
                // Comment
                "comment": {
                    "comments": [{
                        "id": "cmt-1",
                        "author": {"displayName": "Charlie Dev"},
                        "created": "2024-01-16T09:00:00.000+0000",
                        "body": {
                            "type": "doc",
                            "version": 1,
                            "content": [{
                                "type": "paragraph",
                                "content": [{"type": "text", "text": "Confirmed on iOS 17."}]
                            }]
                        }
                    }]
                }
            },
            // Changelog
            "changelog": {
                "histories": [{
                    "author": {"displayName": "Alice Smith"},
                    "created": "2024-01-14T08:00:00.000+0000",
                    "items": [{
                        "field": "status",
                        "fromString": "To Do",
                        "toString": "In Progress"
                    }]
                }]
            },
            // Names map for custom fields
            "names": {
                "customfield_10016": "Story Points"
            }
        });

        let names = serde_json::json!({
            "customfield_10016": "Story Points"
        });

        (body, names)
    }

    #[test]
    fn test_parse_issue_full_basic_fields() {
        let (body, names) = canned_issue_json();
        let issue = parse_issue_full(&body, &names, "https://example.atlassian.net");

        assert_eq!(issue.key, "PROJ-123");
        assert_eq!(issue.summary, "Login page crashes on mobile");
        assert_eq!(issue.status, "In Progress");
        assert_eq!(issue.issue_type, "Bug");
        assert_eq!(
            issue.url,
            "https://example.atlassian.net/browse/PROJ-123"
        );
        assert_eq!(issue.priority, Some("High".to_string()));
        assert_eq!(issue.labels, vec!["mobile", "crash"]);
    }

    #[test]
    fn test_parse_issue_full_description_md() {
        let (body, names) = canned_issue_json();
        let issue = parse_issue_full(&body, &names, "https://example.atlassian.net");

        assert!(
            issue.description_md.contains("Steps to reproduce"),
            "description_md: {:?}",
            issue.description_md
        );
    }

    #[test]
    fn test_parse_issue_full_assignee_reporter() {
        let (body, names) = canned_issue_json();
        let issue = parse_issue_full(&body, &names, "https://example.atlassian.net");

        let assignee = issue.assignee.expect("assignee should be present");
        assert_eq!(assignee.account_id, "acc-1");
        assert_eq!(assignee.display_name, "Alice Smith");
        assert_eq!(
            assignee.avatar_url.as_deref(),
            Some("https://cdn.example.com/alice.png")
        );

        let reporter = issue.reporter.expect("reporter should be present");
        assert_eq!(reporter.display_name, "Bob Jones");
    }

    #[test]
    fn test_parse_issue_full_custom_field_via_names() {
        let (body, names) = canned_issue_json();
        let issue = parse_issue_full(&body, &names, "https://example.atlassian.net");

        let sp = issue
            .fields
            .iter()
            .find(|f| f.key == "customfield_10016")
            .expect("story points field should be present");
        assert_eq!(sp.name, "Story Points");
        assert_eq!(sp.value, "8");
    }

    #[test]
    fn test_parse_issue_full_attachment() {
        let (body, names) = canned_issue_json();
        let issue = parse_issue_full(&body, &names, "https://example.atlassian.net");

        assert_eq!(issue.attachments.len(), 1);
        let att = &issue.attachments[0];
        assert_eq!(att.id, "att-99");
        assert_eq!(att.filename, "screenshot.png");
        assert_eq!(att.mime, "image/png");
        assert_eq!(att.size, 45678);
        assert_eq!(att.author, "Alice Smith");
    }

    #[test]
    fn test_parse_issue_full_issue_link() {
        let (body, names) = canned_issue_json();
        let issue = parse_issue_full(&body, &names, "https://example.atlassian.net");

        assert!(!issue.links.is_empty(), "should have at least one link");
        let link = issue
            .links
            .iter()
            .find(|l| l.key == "PROJ-100")
            .expect("PROJ-100 link should be present");
        assert_eq!(link.rel, "blocks");
        assert_eq!(link.summary, "Auth service down");
        assert_eq!(link.status, "Open");
        assert_eq!(link.issue_type, "Bug");
    }

    #[test]
    fn test_parse_issue_full_comment() {
        let (body, names) = canned_issue_json();
        let issue = parse_issue_full(&body, &names, "https://example.atlassian.net");

        assert_eq!(issue.comments.len(), 1);
        let comment = &issue.comments[0];
        assert_eq!(comment.id, "cmt-1");
        assert_eq!(comment.author, "Charlie Dev");
        assert!(
            comment.body_md.contains("Confirmed on iOS 17"),
            "comment body_md: {:?}",
            comment.body_md
        );
    }

    #[test]
    fn test_parse_issue_full_changelog() {
        let (body, names) = canned_issue_json();
        let issue = parse_issue_full(&body, &names, "https://example.atlassian.net");

        assert_eq!(issue.history.len(), 1);
        let entry = &issue.history[0];
        assert_eq!(entry.author, "Alice Smith");
        assert_eq!(entry.items.len(), 1);
        assert_eq!(entry.items[0].field, "status");
        assert_eq!(entry.items[0].from, "To Do");
        assert_eq!(entry.items[0].to, "In Progress");
    }

    // ── estimate extraction tests ────────────────────────────────────────────

    #[test]
    fn test_parse_issue_full_estimate_story_points() {
        // Fixture: issue with a "Story Points" custom field (value 5.0).
        let body = serde_json::json!({
            "key": "PROJ-10",
            "fields": {
                "summary": "Estimate via story points",
                "status": {"name": "To Do"},
                "issuetype": {"name": "Story"},
                "customfield_10016": 5.0
            },
            "changelog": { "histories": [] }
        });
        let names = serde_json::json!({
            "customfield_10016": "Story Points"
        });
        let issue = parse_issue_full(&body, &names, "https://jira.example.com");
        assert_eq!(issue.estimate, Some("5 pts".to_string()));
    }

    #[test]
    fn test_parse_issue_full_estimate_timetracking() {
        // Fixture: issue with timetracking.originalEstimate set.
        let body = serde_json::json!({
            "key": "PROJ-11",
            "fields": {
                "summary": "Estimate via timetracking",
                "status": {"name": "To Do"},
                "issuetype": {"name": "Task"},
                "timetracking": {
                    "originalEstimate": "1d 2h"
                }
            },
            "changelog": { "histories": [] }
        });
        let names = serde_json::json!({});
        let issue = parse_issue_full(&body, &names, "https://jira.example.com");
        assert_eq!(issue.estimate, Some("1d 2h".to_string()));
    }

    // ── build_create_issue_body tests ────────────────────────────────────────

    #[test]
    fn test_build_create_issue_body_nested_shape() {
        let adf = serde_json::json!({
            "type": "doc",
            "version": 1,
            "content": []
        });
        let body = build_create_issue_body("PROJ", "Story", "My summary", adf.clone());

        // Top-level key is "fields"
        let fields = body.get("fields").expect("fields key missing");

        // project.key
        assert_eq!(
            fields
                .get("project")
                .and_then(|p| p.get("key"))
                .and_then(|v| v.as_str()),
            Some("PROJ")
        );
        // issuetype.name
        assert_eq!(
            fields
                .get("issuetype")
                .and_then(|t| t.get("name"))
                .and_then(|v| v.as_str()),
            Some("Story")
        );
        // summary
        assert_eq!(
            fields.get("summary").and_then(|v| v.as_str()),
            Some("My summary")
        );
        // description is the ADF value we passed
        assert_eq!(fields.get("description"), Some(&adf));
    }

    #[test]
    fn test_build_create_issue_body_different_issue_type() {
        let adf = serde_json::json!(null);
        let body = build_create_issue_body("ALPHA", "Bug", "A crash bug", adf);
        let fields = body.get("fields").unwrap();
        assert_eq!(
            fields
                .get("issuetype")
                .and_then(|t| t.get("name"))
                .and_then(|v| v.as_str()),
            Some("Bug")
        );
        assert_eq!(
            fields
                .get("project")
                .and_then(|p| p.get("key"))
                .and_then(|v| v.as_str()),
            Some("ALPHA")
        );
    }

    // ── parse_transitions tests ──────────────────────────────────────────────

    #[test]
    fn test_parse_transitions() {
        let body = serde_json::json!({
            "transitions": [
                {"id": "11", "name": "To Do", "to": {"name": "To Do", "id": "1"}},
                {"id": "21", "name": "In Progress", "to": {"name": "In Progress", "id": "3"}},
                {"id": "31", "name": "Done", "to": {"name": "Done", "id": "5"}}
            ]
        });
        let transitions = parse_transitions(&body);
        assert_eq!(transitions.len(), 3);
        assert_eq!(transitions[0].id, "11");
        assert_eq!(transitions[0].name, "To Do");
        assert_eq!(transitions[0].to_status, "To Do");
        assert_eq!(transitions[1].name, "In Progress");
        assert_eq!(transitions[2].to_status, "Done");
    }

    // ── stringify_value tests ────────────────────────────────────────────────

    #[test]
    fn test_stringify_value_primitives() {
        assert_eq!(stringify_value(&serde_json::json!("hello")), "hello");
        assert_eq!(stringify_value(&serde_json::json!(42)), "42");
        assert_eq!(stringify_value(&serde_json::json!(true)), "true");
        assert_eq!(stringify_value(&serde_json::json!(null)), "");
    }

    #[test]
    fn test_stringify_value_object_display_name() {
        let v = serde_json::json!({"displayName": "High", "id": "2"});
        assert_eq!(stringify_value(&v), "High");
    }

    #[test]
    fn test_stringify_value_object_name_fallback() {
        let v = serde_json::json!({"name": "Critical", "id": "1"});
        assert_eq!(stringify_value(&v), "Critical");
    }

    #[test]
    fn test_stringify_value_array() {
        let v = serde_json::json!([{"name": "Alpha"}, {"name": "Beta"}]);
        let s = stringify_value(&v);
        assert!(s.contains("Alpha"), "{s}");
        assert!(s.contains("Beta"), "{s}");
    }
}
