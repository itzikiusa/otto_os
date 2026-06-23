//! Jira Cloud / Server REST API v3 client.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use futures_util::future::join_all;

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

/// One selectable value for an editable Jira field (option, version, component, user, …).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FieldOption {
    /// Identifier to send back in the PUT body — Jira `id`, or `accountId` for users,
    /// or the `value`/`key`/`name` fallback. Labels key on their own string value.
    pub id: String,
    /// Human-readable label for the dropdown.
    pub label: String,
}

/// A field the current user is allowed to edit, as reported by `editmeta`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EditableField {
    /// Raw Jira field key (e.g. `"customfield_10016"`, `"labels"`, `"priority"`).
    pub key: String,
    /// Display name (editmeta `name`).
    pub name: String,
    /// Schema type: `string`, `number`, `option`, `array`, `date`, `datetime`, `user`,
    /// `priority`, `version`, `component`, … (editmeta `schema.type`).
    pub schema_type: String,
    /// For `array` types, the element type (editmeta `schema.items`, e.g. `option`, `string`, `user`).
    pub items: Option<String>,
    /// Allowed values when the field is constrained (option / array-of-option / version / component / priority).
    pub allowed_values: Vec<FieldOption>,
    /// Whether Jira marks the field required.
    pub required: bool,
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
    /// Numeric Jira issue id (top-level `"id"` of the issue resource). Needed by
    /// the dev-status API, which keys on the numeric id rather than the issue key.
    pub id: String,
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

/// A branch surfaced by Jira's dev-status integration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DevBranch {
    pub name: String,
    pub url: String,
    pub repo: String,
    pub last_commit: Option<String>,
}

/// A commit surfaced by Jira's dev-status integration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DevCommit {
    /// Commit hash (full or short `displayId` as returned by the dev tool).
    pub id: String,
    pub message: String,
    pub url: String,
    pub author: String,
    pub timestamp: String,
    pub repo: String,
}

/// A pull request surfaced by Jira's dev-status integration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DevPr {
    pub id: String,
    pub name: String,
    pub url: String,
    pub status: String,
    pub repo: String,
    pub last_update: String,
}

/// Aggregated development info (branches / commits / PRs) linked to an issue
/// across all detected dev tools. Empty vecs are normal (no integration connected).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct DevStatus {
    pub branches: Vec<DevBranch>,
    pub commits: Vec<DevCommit>,
    pub pull_requests: Vec<DevPr>,
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

    /// Resolve an issue key to its numeric Jira id with a lightweight fetch.
    ///
    /// The dev-status API keys on the numeric id, not the issue key. Rather than
    /// re-fetching the heavy `*all` expansion via [`Self::get_issue_full`], we ask
    /// only for the top-level `id` (always returned regardless of `fields`).
    ///
    /// Uses `GET /rest/api/3/issue/{key}?fields=id`.
    pub async fn get_issue_id(&self, key: &str) -> Result<String> {
        let url = format!("{}/rest/api/3/issue/{}", self.base_url, key);
        let resp = self
            .http
            .get(&url)
            .header("Authorization", &self.auth_header)
            .header("Accept", "application/json")
            .query(&[("fields", "id")])
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("jira get_issue_id request: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream(format!(
                "jira issue id {key} failed ({status}): {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Upstream(format!("jira get_issue_id parse: {e}")))?;

        Ok(body
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string())
    }

    /// Aggregate branches / commits / PRs linked to an issue across every dev tool.
    ///
    /// Jira's dev-status panel is fed by `GET /rest/dev-status/latest/issue/detail`,
    /// which must be queried per `(applicationType, dataType)` combination. Many
    /// combos return empty or 404 — that is normal; per-combo errors are swallowed
    /// and we return whatever aggregates (an empty [`DevStatus`] if nothing is
    /// connected). This method never hard-errors on a missing dev tool.
    pub async fn dev_status(&self, issue_id: &str) -> Result<DevStatus> {
        // The application/data-type matrix Jira exposes for dev-status detail.
        const APP_TYPES: [&str; 4] = ["github", "bitbucket", "gitlab", "stash"];
        const DATA_TYPES: [&str; 3] = ["pullrequest", "branch", "repository"];

        let url = format!("{}/rest/dev-status/latest/issue/detail", self.base_url);

        // Fire all 12 (app_type × data_type) requests concurrently rather than
        // sequentially. Per-combo errors (transport or non-2xx) are swallowed so a
        // missing/disconnected dev tool never hard-errors the whole call.
        let combos: Vec<(&str, &str)> = APP_TYPES
            .iter()
            .flat_map(|&a| DATA_TYPES.iter().map(move |&d| (a, d)))
            .collect();

        let futures = combos.iter().map(|(app_type, data_type)| {
            let url = url.clone();
            let auth = self.auth_header.clone();
            let http = self.http.clone();
            let issue_id = issue_id.to_string();
            let app_type = *app_type;
            let data_type = *data_type;
            async move {
                let resp = match http
                    .get(&url)
                    .header("Authorization", &auth)
                    .header("Accept", "application/json")
                    .query(&[
                        ("issueId", issue_id.as_str()),
                        ("applicationType", app_type),
                        ("dataType", data_type),
                    ])
                    .send()
                    .await
                {
                    Ok(r) => r,
                    Err(_) => return None,
                };
                if !resp.status().is_success() {
                    return None;
                }
                resp.json::<serde_json::Value>().await.ok()
            }
        });

        let results = join_all(futures).await;

        let mut out = DevStatus::default();
        for body in results.into_iter().flatten() {
            merge_dev_detail(&mut out, &body);
        }

        // The same item can surface under multiple combos — dedupe before returning.
        dedupe_dev_status(&mut out);
        Ok(out)
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

    /// Fetch the set of fields the caller may edit on an issue.
    ///
    /// Uses `GET /rest/api/3/issue/{key}/editmeta` and flattens the
    /// `fields` map into a `Vec<EditableField>` sorted by display name.
    pub async fn editmeta(&self, key: &str) -> Result<Vec<EditableField>> {
        let url = format!("{}/rest/api/3/issue/{}/editmeta", self.base_url, key);
        let resp = self
            .http
            .get(&url)
            .header("Authorization", &self.auth_header)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("jira editmeta request: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream(format!(
                "jira editmeta {key} failed ({status}): {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Upstream(format!("jira editmeta parse: {e}")))?;

        Ok(parse_editmeta(&body))
    }

    /// Update arbitrary issue fields. `fields` is the caller-built object placed
    /// under the top-level `"fields"` key, e.g. `{ "customfield_10016": 5 }`.
    ///
    /// Uses `PUT /rest/api/3/issue/{key}` with `{"fields": fields}`.
    pub async fn update_fields(&self, key: &str, fields: serde_json::Value) -> Result<()> {
        let url = format!("{}/rest/api/3/issue/{}", self.base_url, key);
        let payload = serde_json::json!({ "fields": fields });

        let resp = self
            .http
            .put(&url)
            .header("Authorization", &self.auth_header)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("jira update_fields request: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream(format!(
                "jira update_fields {key} failed ({status}): {body}"
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

/// Parse an `editmeta` response body into a list of [`EditableField`].
///
/// Iterates the `fields` map `{fieldId → meta}` and flattens each entry. Fields
/// the UI already handles through dedicated controls (status/assignee via the
/// transition/assignee cards, summary/description via their own editors, plus the
/// container fields comment/attachment/issuelinks) are skipped to avoid double
/// editors. Mirrors the `skip_keys` set in [`parse_issue_full`].
pub(crate) fn parse_editmeta(body: &serde_json::Value) -> Vec<EditableField> {
    // Fields with their own dedicated UI affordances — keep them out of the
    // generic editor so we never render two controls for the same field.
    let skip_keys: std::collections::HashSet<&str> = [
        "summary",
        "description",
        "status",
        "issuetype",
        "assignee",  // assignee has its own card + assignable search flow
        "reporter",  // reporter has its own read-only row; generic editor is wrong for it
        "issuelinks",
        "comment",
        "attachment",
    ]
    .into_iter()
    .collect();

    let fields_obj = match body.get("fields").and_then(|v| v.as_object()) {
        Some(o) => o,
        None => return Vec::new(),
    };

    let mut out: Vec<EditableField> = Vec::new();
    for (fkey, meta) in fields_obj {
        if skip_keys.contains(fkey.as_str()) {
            continue;
        }
        let name = meta
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(fkey.as_str())
            .to_string();
        let schema = meta.get("schema");
        let schema_type = schema
            .and_then(|s| s.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("string")
            .to_string();
        let items = schema
            .and_then(|s| s.get("items"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let required = meta
            .get("required")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Allowed values: option / version / component / priority / user-typed
        // fields constrain the choices. Labels and free-text arrays have none.
        let allowed_values: Vec<FieldOption> = meta
            .get("allowedValues")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|opt| {
                        // id: prefer the stable identifier Jira expects back in the PUT.
                        let id = opt
                            .get("id")
                            .and_then(|v| v.as_str())
                            .or_else(|| opt.get("accountId").and_then(|v| v.as_str()))
                            .or_else(|| opt.get("key").and_then(|v| v.as_str()))
                            .or_else(|| opt.get("value").and_then(|v| v.as_str()))
                            .or_else(|| opt.get("name").and_then(|v| v.as_str()))
                            .unwrap_or("")
                            .to_string();
                        let label = stringify_value(opt);
                        FieldOption { id, label }
                    })
                    .collect()
            })
            .unwrap_or_default();

        out.push(EditableField {
            key: fkey.clone(),
            name,
            schema_type,
            items,
            allowed_values,
            required,
        });
    }
    // Sort for deterministic output (mirror extra_fields in parse_issue_full).
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
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
    // Top-level numeric id (distinct from the human-readable key) — the dev-status
    // API keys on this rather than on the issue key.
    let id = body
        .get("id")
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
        id,
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

/// Read a string field, falling back through alternate keys, ending in `""`.
fn dev_str(v: &serde_json::Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string()
}

/// Merge one dev-status `detail` response into the running aggregate.
///
/// Shape (`GET /rest/dev-status/latest/issue/detail`):
/// ```text
/// { "detail": [ { "repositories": [ { "name", "commits": [...], "branches": [...],
///                                     "pullRequests": [...] } ],
///                 "pullRequests": [...] } ] }
/// ```
/// PRs appear both at `detail[].pullRequests` and (for some tools) nested under
/// `repositories[].pullRequests`; both paths are scanned. Extraction is lenient —
/// missing fields default to `""` rather than dropping the whole item.
pub(crate) fn merge_dev_detail(out: &mut DevStatus, body: &serde_json::Value) {
    let details = match body.get("detail").and_then(|d| d.as_array()) {
        Some(d) => d,
        None => return,
    };
    for d in details {
        // Repositories: commits + branches (+ possibly nested pull requests).
        if let Some(repos) = d.get("repositories").and_then(|r| r.as_array()) {
            for repo in repos {
                let repo_name = dev_str(repo, "name");
                if let Some(commits) = repo.get("commits").and_then(|c| c.as_array()) {
                    for c in commits {
                        // `id` falls back to the shorter `displayId`.
                        let id = {
                            let full = dev_str(c, "id");
                            if full.is_empty() {
                                dev_str(c, "displayId")
                            } else {
                                full
                            }
                        };
                        // Author is an object (`{name}`) in the standard shape; tolerate a
                        // bare string too.
                        let author = c
                            .get("author")
                            .and_then(|a| a.get("name").and_then(|n| n.as_str()))
                            .or_else(|| c.get("author").and_then(|a| a.as_str()))
                            .unwrap_or("")
                            .to_string();
                        let timestamp = {
                            let t = dev_str(c, "authorTimestamp");
                            if !t.is_empty() {
                                t
                            } else {
                                let t = dev_str(c, "timestamp");
                                if !t.is_empty() {
                                    t
                                } else {
                                    dev_str(c, "date")
                                }
                            }
                        };
                        out.commits.push(DevCommit {
                            id,
                            message: dev_str(c, "message"),
                            url: dev_str(c, "url"),
                            author,
                            timestamp,
                            repo: repo_name.clone(),
                        });
                    }
                }
                if let Some(branches) = repo.get("branches").and_then(|b| b.as_array()) {
                    for b in branches {
                        let last_commit = b.get("lastCommit").and_then(|lc| {
                            lc.get("displayId")
                                .and_then(|x| x.as_str())
                                .or_else(|| lc.get("id").and_then(|x| x.as_str()))
                                .map(|s| s.to_string())
                        });
                        out.branches.push(DevBranch {
                            name: dev_str(b, "name"),
                            url: dev_str(b, "url"),
                            repo: repo_name.clone(),
                            last_commit,
                        });
                    }
                }
                if let Some(prs) = repo.get("pullRequests").and_then(|p| p.as_array()) {
                    for pr in prs {
                        out.pull_requests.push(extract_dev_pr(pr, &repo_name));
                    }
                }
            }
        }
        // Pull requests at the detail level (the common GitHub/Bitbucket shape).
        if let Some(prs) = d.get("pullRequests").and_then(|p| p.as_array()) {
            for pr in prs {
                out.pull_requests.push(extract_dev_pr(pr, ""));
            }
        }
        // Branches at the detail level (dataType=branch responses from some tools
        // return branches here with a nested `repository` object rather than under
        // repositories[].branches).
        if let Some(branches) = d.get("branches").and_then(|b| b.as_array()) {
            for b in branches {
                let repo_name = b
                    .get("repository")
                    .and_then(|r| r.get("name"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let last_commit = b.get("lastCommit").and_then(|lc| {
                    lc.get("displayId")
                        .and_then(|x| x.as_str())
                        .or_else(|| lc.get("id").and_then(|x| x.as_str()))
                        .map(|s| s.to_string())
                });
                out.branches.push(DevBranch {
                    name: dev_str(b, "name"),
                    url: dev_str(b, "url"),
                    repo: repo_name,
                    last_commit,
                });
            }
        }
        // Commits at the detail level (some tools return them outside repositories).
        if let Some(commits) = d.get("commits").and_then(|c| c.as_array()) {
            for c in commits {
                let repo_name = c
                    .get("repository")
                    .and_then(|r| r.get("name"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let id = {
                    let full = dev_str(c, "id");
                    if full.is_empty() {
                        dev_str(c, "displayId")
                    } else {
                        full
                    }
                };
                let author = c
                    .get("author")
                    .and_then(|a| a.get("name").and_then(|n| n.as_str()))
                    .or_else(|| c.get("author").and_then(|a| a.as_str()))
                    .unwrap_or("")
                    .to_string();
                let timestamp = {
                    let t = dev_str(c, "authorTimestamp");
                    if !t.is_empty() {
                        t
                    } else {
                        let t = dev_str(c, "timestamp");
                        if !t.is_empty() { t } else { dev_str(c, "date") }
                    }
                };
                out.commits.push(DevCommit {
                    id,
                    message: dev_str(c, "message"),
                    url: dev_str(c, "url"),
                    author,
                    timestamp,
                    repo: repo_name,
                });
            }
        }
    }
}

/// Extract a [`DevPr`] from a dev-status pull-request object.
///
/// The repository name is read from `source.repository.name` (the standard
/// dev-status shape), falling back to `destination.repository.name`, then to the
/// `default_repo` hint (used when PRs are nested under a repository), then `""`.
fn extract_dev_pr(pr: &serde_json::Value, default_repo: &str) -> DevPr {
    let repo = pr
        .get("source")
        .and_then(|s| s.get("repository"))
        .and_then(|r| r.get("name"))
        .and_then(|n| n.as_str())
        .or_else(|| {
            pr.get("destination")
                .and_then(|s| s.get("repository"))
                .and_then(|r| r.get("name"))
                .and_then(|n| n.as_str())
        })
        .map(|s| s.to_string())
        .unwrap_or_else(|| default_repo.to_string());
    let last_update = {
        let t = dev_str(pr, "lastUpdate");
        if !t.is_empty() {
            t
        } else {
            let t = dev_str(pr, "updated");
            if !t.is_empty() {
                t
            } else {
                dev_str(pr, "date")
            }
        }
    };
    DevPr {
        id: dev_str(pr, "id"),
        name: dev_str(pr, "name"),
        url: dev_str(pr, "url"),
        status: dev_str(pr, "status"),
        repo,
        last_update,
    }
}

/// Dedupe each Dev* vec after aggregation — the same item can appear under
/// multiple `applicationType`/`dataType` combos. First-seen order is preserved
/// (the combo loop order is fixed, so the result is deterministic).
pub(crate) fn dedupe_dev_status(status: &mut DevStatus) {
    use std::collections::HashSet;
    let mut seen: HashSet<(String, String)> = HashSet::new();
    status
        .branches
        .retain(|b| seen.insert((b.repo.clone(), b.name.clone())));
    seen.clear();
    status
        .commits
        .retain(|c| seen.insert((c.repo.clone(), c.id.clone())));
    seen.clear();
    status
        .pull_requests
        .retain(|p| seen.insert((p.repo.clone(), p.id.clone())));
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

    // ── parse_editmeta tests ─────────────────────────────────────────────────

    #[test]
    fn test_parse_editmeta_flattens_and_filters() {
        let body = serde_json::json!({
            "fields": {
                // Story Points — numeric custom field (the estimation case).
                "customfield_10016": {
                    "name": "Story Points",
                    "required": false,
                    "schema": {"type": "number", "custom": "…float"}
                },
                // Single-option custom field with allowed values.
                "customfield_10020": {
                    "name": "Severity",
                    "required": true,
                    "schema": {"type": "option"},
                    "allowedValues": [
                        {"id": "1", "value": "Critical"},
                        {"id": "2", "value": "Minor"}
                    ]
                },
                // Labels — array of strings, no allowed values (free text).
                "labels": {
                    "name": "Labels",
                    "required": false,
                    "schema": {"type": "array", "items": "string"}
                },
                // Priority — constrained by name.
                "priority": {
                    "name": "Priority",
                    "required": false,
                    "schema": {"type": "priority"},
                    "allowedValues": [
                        {"id": "3", "name": "High"},
                        {"id": "4", "name": "Low"}
                    ]
                },
                // Dedicated-control fields — must be filtered out.
                "summary": {"name": "Summary", "schema": {"type": "string"}},
                "assignee": {"name": "Assignee", "schema": {"type": "user"}},
                "issuelinks": {"name": "Linked Issues", "schema": {"type": "array", "items": "issuelinks"}}
            }
        });

        let fields = parse_editmeta(&body);
        let keys: Vec<&str> = fields.iter().map(|f| f.key.as_str()).collect();
        // Filtered fields are absent.
        assert!(!keys.contains(&"summary"));
        assert!(!keys.contains(&"assignee"));
        assert!(!keys.contains(&"issuelinks"));
        // Editable fields are present.
        assert!(keys.contains(&"customfield_10016"));
        assert!(keys.contains(&"customfield_10020"));
        assert!(keys.contains(&"labels"));
        assert!(keys.contains(&"priority"));
        // Sorted by display name: Labels, Priority, Severity, Story Points.
        let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["Labels", "Priority", "Severity", "Story Points"]);

        let sp = fields.iter().find(|f| f.key == "customfield_10016").unwrap();
        assert_eq!(sp.schema_type, "number");
        assert!(sp.allowed_values.is_empty());

        let severity = fields.iter().find(|f| f.key == "customfield_10020").unwrap();
        assert_eq!(severity.schema_type, "option");
        assert!(severity.required);
        assert_eq!(severity.allowed_values.len(), 2);
        assert_eq!(severity.allowed_values[0].id, "1");
        assert_eq!(severity.allowed_values[0].label, "Critical");

        let labels = fields.iter().find(|f| f.key == "labels").unwrap();
        assert_eq!(labels.schema_type, "array");
        assert_eq!(labels.items.as_deref(), Some("string"));
        assert!(labels.allowed_values.is_empty());

        let priority = fields.iter().find(|f| f.key == "priority").unwrap();
        assert_eq!(priority.allowed_values[0].id, "3");
        assert_eq!(priority.allowed_values[0].label, "High");
    }

    #[test]
    fn test_parse_editmeta_empty() {
        assert!(parse_editmeta(&serde_json::json!({})).is_empty());
        assert!(parse_editmeta(&serde_json::json!({"fields": {}})).is_empty());
    }

    // ── parse_issue_full tests ───────────────────────────────────────────────

    /// Build a realistic canned Jira issue JSON fixture (what you get from
    /// `GET /rest/api/3/issue/PROJ-123?expand=changelog,names,renderedFields&fields=*all`)
    fn canned_issue_json() -> (serde_json::Value, serde_json::Value) {
        let body = serde_json::json!({
            "key": "PROJ-123",
            "id": "10001",
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
        assert_eq!(issue.id, "10001");
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

    // ── dev-status parsing tests ─────────────────────────────────────────────

    /// A realistic `GET /rest/dev-status/latest/issue/detail` response with one
    /// repository (1 commit + 1 branch) and one detail-level pull request.
    fn canned_dev_detail() -> serde_json::Value {
        serde_json::json!({
            "detail": [{
                "repositories": [{
                    "name": "acme/web",
                    "commits": [{
                        "id": "abc1234def5678",
                        "displayId": "abc1234",
                        "message": "Fix login crash",
                        "url": "https://github.com/acme/web/commit/abc1234def5678",
                        "author": {"name": "Alice Smith"},
                        "authorTimestamp": "2024-01-15T10:00:00.000Z"
                    }],
                    "branches": [{
                        "name": "feature/PROJ-123-fix",
                        "url": "https://github.com/acme/web/tree/feature/PROJ-123-fix",
                        "lastCommit": {"displayId": "abc1234"}
                    }]
                }],
                "pullRequests": [{
                    "id": "42",
                    "name": "Fix login crash on mobile",
                    "url": "https://github.com/acme/web/pull/42",
                    "status": "OPEN",
                    "lastUpdate": "2024-01-16T09:00:00.000Z",
                    "source": {"repository": {"name": "acme/web"}}
                }]
            }]
        })
    }

    #[test]
    fn test_merge_dev_detail_commit() {
        let mut out = DevStatus::default();
        merge_dev_detail(&mut out, &canned_dev_detail());

        assert_eq!(out.commits.len(), 1);
        let c = &out.commits[0];
        assert_eq!(c.id, "abc1234def5678");
        assert_eq!(c.message, "Fix login crash");
        assert_eq!(c.author, "Alice Smith");
        assert_eq!(c.timestamp, "2024-01-15T10:00:00.000Z");
        assert_eq!(c.repo, "acme/web");
    }

    #[test]
    fn test_merge_dev_detail_branch() {
        let mut out = DevStatus::default();
        merge_dev_detail(&mut out, &canned_dev_detail());

        assert_eq!(out.branches.len(), 1);
        let b = &out.branches[0];
        assert_eq!(b.name, "feature/PROJ-123-fix");
        assert_eq!(b.repo, "acme/web");
        assert_eq!(b.last_commit.as_deref(), Some("abc1234"));
    }

    #[test]
    fn test_merge_dev_detail_pull_request() {
        let mut out = DevStatus::default();
        merge_dev_detail(&mut out, &canned_dev_detail());

        assert_eq!(out.pull_requests.len(), 1);
        let pr = &out.pull_requests[0];
        assert_eq!(pr.id, "42");
        assert_eq!(pr.status, "OPEN");
        assert_eq!(pr.url, "https://github.com/acme/web/pull/42");
        assert_eq!(pr.repo, "acme/web");
    }

    #[test]
    fn test_dedupe_dev_status_collapses_duplicate_combos() {
        let mut out = DevStatus::default();
        // The same detail can be returned under two combos (e.g. github+commit
        // and github+repository) — merge twice, then dedupe.
        merge_dev_detail(&mut out, &canned_dev_detail());
        merge_dev_detail(&mut out, &canned_dev_detail());
        assert_eq!(out.commits.len(), 2);
        assert_eq!(out.pull_requests.len(), 2);

        dedupe_dev_status(&mut out);
        assert_eq!(out.commits.len(), 1, "duplicate commit should collapse");
        assert_eq!(out.branches.len(), 1, "duplicate branch should collapse");
        assert_eq!(out.pull_requests.len(), 1, "duplicate PR should collapse");
    }

    #[test]
    fn test_merge_dev_detail_empty_is_noop() {
        let mut out = DevStatus::default();
        merge_dev_detail(&mut out, &serde_json::json!({}));
        assert!(out.commits.is_empty());
        assert!(out.branches.is_empty());
        assert!(out.pull_requests.is_empty());
    }
}
