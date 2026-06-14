//! Jira Cloud / Server REST API v3 client.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use otto_core::domain::{IssueDetail, IssueProject, IssueSummary};
use otto_core::{Error, Result};

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
        Self {
            base_url,
            auth_header,
            http: reqwest::Client::new(),
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
    pub async fn search(&self, q: &str, project: Option<&str>) -> Result<Vec<IssueSummary>> {
        let jql = build_jql(q, project);
        let fields = "summary,status,issuetype";

        // Try the newer endpoint first.
        let new_url = format!("{}/rest/api/3/search/jql", self.base_url);
        let resp = self
            .http
            .get(&new_url)
            .header("Authorization", &self.auth_header)
            .header("Accept", "application/json")
            .query(&[
                ("jql", &jql),
                ("maxResults", &"25".to_string()),
                ("fields", &fields.to_string()),
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
                    ("jql", &jql),
                    ("maxResults", &"25".to_string()),
                    ("fields", &fields.to_string()),
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
        // Jira ADF description is a JSON object — emit empty string rather than
        // trying to render the document format.
        let description = fields
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
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
}

/// Build a JQL query from a search string and optional project key.
///
/// When `project` is `Some(key)`:
///   - all-digits `q` → `project = "KEY" AND (key = "KEY-q" OR summary ~ "q*" OR text ~ "q")`
///   - full issue-key `q` → `project = "KEY" AND key = "q"`
///   - text `q` → `project = "KEY" AND (summary ~ "q*" OR text ~ "q")`
///
/// When `project` is `None`, falls back to the original unrestricted behaviour.
fn build_jql(q: &str, project: Option<&str>) -> String {
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
}
