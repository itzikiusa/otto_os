//! Confluence Cloud REST API v1 client (storage-format XHTML ↔ Markdown helpers included).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use otto_core::{Error, Result};

use crate::jira::CommentRef;

/// How long to wait for the TCP/TLS connection to a Confluence endpoint.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Overall per-request deadline so a hung endpoint can't block the caller.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Process-wide cache: composite key (site_base + auth_header) → shared client.
static CLIENT_CACHE: OnceLock<Mutex<HashMap<String, reqwest::Client>>> = OnceLock::new();

/// Return a cached `reqwest::Client`, building one on first use for this key.
fn get_or_build_client(cache_key: &str) -> reqwest::Client {
    let cache = CLIENT_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut map) = cache.lock() {
        if let Some(c) = map.get(cache_key) {
            return c.clone();
        }
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .unwrap_or_default();
        map.insert(cache_key.to_string(), client.clone());
        return client;
    }
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .unwrap_or_default()
}

// ──────────────────────────────────────────────────────────────────────────────
// Domain types
// ──────────────────────────────────────────────────────────────────────────────

/// A Confluence space (key + display name).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConfluenceSpace {
    pub key: String,
    pub name: String,
}

/// A lightweight page summary returned from a CQL search.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConfluencePageSummary {
    pub id: String,
    pub title: String,
    pub space_key: String,
    pub url: String,
}

/// A Confluence page fetched or created via the REST API.
#[derive(Debug, Clone)]
pub struct ConfluencePage {
    pub id: String,
    pub title: String,
    /// Raw Confluence storage-format XHTML.
    pub body_storage: String,
    /// Current version number (needed for updates).
    pub version: i64,
    pub space_key: String,
    /// Human-friendly browser URL.
    pub url: String,
}

/// A single comment on a Confluence page, body converted to Markdown.
#[derive(Debug, Clone)]
pub struct PageComment {
    pub id: String,
    pub author: String,
    pub body_md: String,
    pub created: String,
}

// ──────────────────────────────────────────────────────────────────────────────
// Client
// ──────────────────────────────────────────────────────────────────────────────

/// A thin Confluence REST client scoped to one Atlassian site.
pub struct ConfluenceClient {
    /// Site root, e.g. `https://site.atlassian.net` (no trailing `/wiki`).
    site_base: String,
    auth_header: String,
    http: reqwest::Client,
}

impl ConfluenceClient {
    /// Construct a new client.
    ///
    /// `site_base` may end with `/wiki` or `/` — both are stripped so that
    /// the REST root becomes `{site_base}/wiki/rest/api`.
    pub fn new(site_base: &str, email: &str, token: &str) -> Self {
        // Strip trailing "/" first, then "/wiki" if present, then "/" again.
        let site_base = site_base.trim_end_matches('/');
        let site_base = site_base.strip_suffix("/wiki").unwrap_or(site_base);
        let site_base = site_base.trim_end_matches('/').to_string();

        let credentials = format!("{email}:{token}");
        let auth_header = format!("Basic {}", B64.encode(credentials.as_bytes()));
        // Reuse a cached client keyed by site + credentials to avoid rebuilding the
        // connection pool on every request.
        let http = get_or_build_client(&format!("{site_base}\x00{auth_header}"));
        Self {
            site_base,
            auth_header,
            http,
        }
    }

    /// Convenience: build the Confluence REST API root.
    fn api(&self, path: &str) -> String {
        format!("{}/wiki/rest/api{}", self.site_base, path)
    }

    /// Fetch a page by its numeric ID.
    ///
    /// Uses `?expand=body.storage,version,space`.
    pub async fn get_page(&self, id: &str) -> Result<ConfluencePage> {
        let url = self.api(&format!("/content/{id}"));
        let resp = self
            .http
            .get(&url)
            .header("Authorization", &self.auth_header)
            .header("Accept", "application/json")
            .query(&[("expand", "body.storage,version,space")])
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("confluence get_page request: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream(format!(
                "confluence get_page {id} failed ({status}): {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Upstream(format!("confluence get_page parse: {e}")))?;

        parse_page(&self.site_base, &body)
    }

    /// Create a new page in `space_key` with the given Confluence storage-format body.
    ///
    /// If `parent_id` is supplied the page is created as a child of that page.
    pub async fn create_page(
        &self,
        space_key: &str,
        title: &str,
        body_storage: &str,
        parent_id: Option<&str>,
    ) -> Result<ConfluencePage> {
        let url = self.api("/content");

        let mut payload = serde_json::json!({
            "type": "page",
            "title": title,
            "space": { "key": space_key },
            "body": {
                "storage": {
                    "value": body_storage,
                    "representation": "storage"
                }
            }
        });

        if let Some(pid) = parent_id {
            payload["ancestors"] = serde_json::json!([{ "id": pid }]);
        }

        let resp = self
            .http
            .post(&url)
            .header("Authorization", &self.auth_header)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("confluence create_page request: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream(format!(
                "confluence create_page failed ({status}): {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Upstream(format!("confluence create_page parse: {e}")))?;

        parse_page(&self.site_base, &body)
    }

    /// Update an existing page. `version` must be the *current* version number;
    /// the API receives `version + 1`.
    pub async fn update_page(
        &self,
        id: &str,
        title: &str,
        body_storage: &str,
        version: i64,
    ) -> Result<ConfluencePage> {
        let url = self.api(&format!("/content/{id}"));
        let payload = serde_json::json!({
            "version": { "number": version + 1 },
            "type": "page",
            "title": title,
            "body": {
                "storage": {
                    "value": body_storage,
                    "representation": "storage"
                }
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
            .map_err(|e| Error::Upstream(format!("confluence update_page request: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream(format!(
                "confluence update_page {id} failed ({status}): {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Upstream(format!("confluence update_page parse: {e}")))?;

        parse_page(&self.site_base, &body)
    }

    /// Add a footer comment to a page.  Returns a [`CommentRef`] (reused from Jira).
    pub async fn add_comment(&self, page_id: &str, body_storage: &str) -> Result<CommentRef> {
        let url = self.api("/content");
        let payload = serde_json::json!({
            "type": "comment",
            "container": { "id": page_id, "type": "page" },
            "body": {
                "storage": {
                    "value": body_storage,
                    "representation": "storage"
                }
            }
        });

        let resp = self
            .http
            .post(&url)
            .header("Authorization", &self.auth_header)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("confluence add_comment request: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream(format!(
                "confluence add_comment page {page_id} failed ({status}): {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Upstream(format!("confluence add_comment parse: {e}")))?;

        let id = body
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let comment_url = body
            .get("_links")
            .and_then(|l| l.get("self"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Ok(CommentRef {
            id,
            url: comment_url,
        })
    }

    /// List all current Confluence spaces (up to 200).
    pub async fn list_spaces(&self) -> Result<Vec<ConfluenceSpace>> {
        let url = self.api("/space");

        let resp = self
            .http
            .get(&url)
            .header("Authorization", &self.auth_header)
            .header("Accept", "application/json")
            .query(&[("limit", "200"), ("status", "current")])
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("confluence list_spaces request: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream(format!(
                "confluence list_spaces failed ({status}): {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Upstream(format!("confluence list_spaces parse: {e}")))?;

        let results_arr = body
            .get("results")
            .and_then(|v| v.as_array())
            .map(|a| a.as_slice())
            .unwrap_or(&[]);

        let mut spaces = Vec::with_capacity(results_arr.len());
        for s in results_arr {
            let key = s
                .get("key")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let name = s
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            // Skip entries with missing key or name.
            if key.is_empty() && name.is_empty() {
                continue;
            }
            spaces.push(ConfluenceSpace { key, name });
        }
        Ok(spaces)
    }

    /// Search Confluence pages using CQL.
    ///
    /// `space_key` narrows the search to one space; `query` is either a numeric
    /// page id or a title substring.  See [`build_page_cql`] for CQL construction.
    pub async fn search_pages(
        &self,
        space_key: Option<&str>,
        query: &str,
    ) -> Result<Vec<ConfluencePageSummary>> {
        let url = self.api("/content/search");
        let cql = build_page_cql(space_key, query);

        let resp = self
            .http
            .get(&url)
            .header("Authorization", &self.auth_header)
            .header("Accept", "application/json")
            .query(&[
                ("cql", cql.as_str()),
                ("limit", "25"),
                ("expand", "space"),
            ])
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("confluence search_pages request: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream(format!(
                "confluence search_pages failed ({status}): {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Upstream(format!("confluence search_pages parse: {e}")))?;

        let results_arr = body
            .get("results")
            .and_then(|v| v.as_array())
            .map(|a| a.as_slice())
            .unwrap_or(&[]);

        let mut pages = Vec::with_capacity(results_arr.len());
        for p in results_arr {
            let id = p
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let title = p
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            // space.key from expanded space object; fall back to caller's space_key.
            let sk = p
                .get("space")
                .and_then(|s| s.get("key"))
                .and_then(|v| v.as_str())
                .unwrap_or(space_key.unwrap_or(""))
                .to_string();

            // URL: mirror parse_page approach — _links.base + _links.webui, then
            // site_base + /wiki + webui, then a fallback.
            let url = {
                let links_base = p
                    .get("_links")
                    .and_then(|l| l.get("base"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let webui = p
                    .get("_links")
                    .and_then(|l| l.get("webui"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                if !links_base.is_empty() && !webui.is_empty() {
                    format!("{}{}", links_base.trim_end_matches('/'), webui)
                } else if !webui.is_empty() {
                    format!("{}/wiki{}", self.site_base, webui)
                } else {
                    format!("{}/wiki/spaces/{}/pages/{}", self.site_base, sk, id)
                }
            };

            pages.push(ConfluencePageSummary {
                id,
                title,
                space_key: sk,
                url,
            });
        }
        Ok(pages)
    }

    /// List footer comments on a page.  Each comment's body is converted from
    /// storage XHTML to Markdown via [`storage_to_markdown`].
    pub async fn list_comments(&self, page_id: &str) -> Result<Vec<PageComment>> {
        let url = self.api(&format!("/content/{page_id}/child/comment"));

        let resp = self
            .http
            .get(&url)
            .header("Authorization", &self.auth_header)
            .header("Accept", "application/json")
            .query(&[("expand", "body.storage,version,history")])
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("confluence list_comments request: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream(format!(
                "confluence list_comments page {page_id} failed ({status}): {body}"
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::Upstream(format!("confluence list_comments parse: {e}")))?;

        let results_arr = body
            .get("results")
            .and_then(|v| v.as_array())
            .map(|a| a.as_slice())
            .unwrap_or(&[]);

        let mut comments = Vec::with_capacity(results_arr.len());
        for c in results_arr {
            let id = c
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let author = c
                .get("history")
                .and_then(|h| h.get("createdBy"))
                .and_then(|u| u.get("displayName"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let created = c
                .get("history")
                .and_then(|h| h.get("createdDate"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let storage_xhtml = c
                .get("body")
                .and_then(|b| b.get("storage"))
                .and_then(|s| s.get("value"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let body_md = storage_to_markdown(storage_xhtml);

            comments.push(PageComment {
                id,
                author,
                body_md,
                created,
            });
        }
        Ok(comments)
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// CQL helpers
// ──────────────────────────────────────────────────────────────────────────────

/// Build a CQL query string for page searches.
///
/// Rules:
/// - Trim the query first.
/// - If the trimmed query is **all digits** → direct page-id lookup: `type=page and id={digits}`.
/// - Otherwise → title contains-match, optionally filtered by space:
///   `type=page [and space="{key}"] and title ~ "{escaped_query}"`.
///   Inside the CQL string literal `"` becomes `\"` and `\` becomes `\\`.
pub fn build_page_cql(space_key: Option<&str>, query: &str) -> String {
    let q = query.trim();
    if q.chars().all(|c| c.is_ascii_digit()) && !q.is_empty() {
        return format!("type=page and id={q}");
    }
    // Escape CQL string literal: backslash first, then double-quote.
    let escaped: String = q
        .replace('\\', "\\\\")
        .replace('"', "\\\"");

    let space_clause = match space_key {
        Some(k) if !k.trim().is_empty() => format!(" and space=\"{}\"", k.trim()),
        _ => String::new(),
    };
    format!("type=page{space_clause} and title ~ \"{escaped}\"")
}

// ──────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ──────────────────────────────────────────────────────────────────────────────

/// Parse a Confluence API content response into a [`ConfluencePage`].
fn parse_page(site_base: &str, body: &serde_json::Value) -> Result<ConfluencePage> {
    let id = body
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let title = body
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let body_storage = body
        .get("body")
        .and_then(|b| b.get("storage"))
        .and_then(|s| s.get("value"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let version = body
        .get("version")
        .and_then(|v| v.get("number"))
        .and_then(|v| v.as_i64())
        .unwrap_or(1);

    let space_key = body
        .get("space")
        .and_then(|s| s.get("key"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Try _links.base + _links.webui; fall back to constructing a reasonable URL.
    let url = {
        let links_base = body
            .get("_links")
            .and_then(|l| l.get("base"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let webui = body
            .get("_links")
            .and_then(|l| l.get("webui"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if !links_base.is_empty() && !webui.is_empty() {
            format!("{}{}", links_base.trim_end_matches('/'), webui)
        } else if !webui.is_empty() {
            format!("{}/wiki{}", site_base, webui)
        } else {
            format!("{}/wiki/spaces/{}/pages/{}", site_base, space_key, id)
        }
    };

    Ok(ConfluencePage {
        id,
        title,
        body_storage,
        version,
        space_key,
        url,
    })
}

// ──────────────────────────────────────────────────────────────────────────────
// Storage XHTML ↔ Markdown converters (lightweight, no external HTML crate)
// ──────────────────────────────────────────────────────────────────────────────

/// Convert Confluence storage-format XHTML to plain Markdown.
///
/// Handles:
/// - Block structure: `<h1>`–`<h6>`, `<p>`, `<br>`, `<pre>`, `<blockquote>`
/// - Inline: `<strong>`/`<b>`, `<em>`/`<i>`, `<code>`, `<a href>`
/// - Lists: `<ul>`/`<ol>`/`<li>` (nested)
/// - **Tables**: `<table>`/`<tr>`/`<th>`/`<td>` → GFM pipe-table
/// - **Confluence macros** (`<ac:structured-macro>`):
///   - `code` → fenced code block with optional language hint
///   - `info` / `note` / `warning` / `tip` → blockquote with prefix label
///   - `status` → inline `[STATUS]` badge
/// - **Images**: `<ac:image>` / `<ri:attachment>` → `![filename](filename)`
///
/// Unknown/unsupported tags are stripped (their text content is kept).
/// HTML entities `&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;`, `&#39;`,
/// `&nbsp;` are decoded.  Unknown `ac:*` / `ri:*` tags degrade silently.
pub fn storage_to_markdown(storage_xhtml: &str) -> String {
    let mut out = String::new();
    let mut pos = 0;
    let input = storage_xhtml;

    // ── List state ─────────────────────────────────────────────────────────
    #[derive(Clone, Copy, PartialEq)]
    enum ListKind {
        Ul,
        Ol,
    }
    let mut list_stack: Vec<ListKind> = Vec::new();
    let mut ol_counters: Vec<usize> = Vec::new();

    // ── Inline-code state ──────────────────────────────────────────────────
    let mut in_pre = false;
    let mut in_code = false;
    let mut pending_newline = false;

    // ── Table state ────────────────────────────────────────────────────────
    // We parse each table completely with a look-ahead before emitting GFM.
    // `in_table` prevents the character-by-character path from seeing table
    // content; the table look-ahead handles it and advances `pos` past `</table>`.
    // We do NOT track cell-by-cell state in the main loop.

    // ── Helper functions ───────────────────────────────────────────────────

    fn decode_entities(s: &str) -> String {
        s.replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&#39;", "'")
            .replace("&nbsp;", " ")
    }

    macro_rules! push_block_sep {
        () => {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            if !out.ends_with("\n\n") {
                out.push('\n');
            }
        };
    }

    // ── Table look-ahead converter ─────────────────────────────────────────
    // Called when we encounter `<table` at position `start_pos`; it finds the
    // matching `</table>` and returns (gfm_string, bytes_consumed).
    fn convert_table(input: &str, start_pos: usize) -> (String, usize) {
        let src = &input[start_pos..];
        // Locate </table>
        let end_offset = src.to_ascii_lowercase().find("</table>").unwrap_or(src.len());
        let table_src = &src[..end_offset + "</table>".len()];
        let consumed = end_offset + "</table>".len();

        // Extract rows: split on <tr / </tr> boundaries.
        // Very lightweight: find each <tr...>...</tr> segment.
        let src_lc = table_src.to_ascii_lowercase();
        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut search_from = 0usize;
        while let Some(tr_start) = src_lc[search_from..].find("<tr") {
            let tr_start = search_from + tr_start;
            // Skip to end of opening <tr...> tag.
            let tr_open_end = src_lc[tr_start..].find('>').map(|i| tr_start + i + 1).unwrap_or(tr_start + 3);
            let after_tr = tr_open_end;
            // Find </tr>.
            let tr_end = src_lc[after_tr..].find("</tr>").map(|i| after_tr + i).unwrap_or(src_lc.len());
            let row_src = &table_src[after_tr..tr_end];
            let row_src_lc = row_src.to_ascii_lowercase();

            // Extract cells: <th> or <td>.
            let mut cells: Vec<String> = Vec::new();
            let mut cell_pos = 0usize;
            while cell_pos < row_src.len() {
                if let Some(cell_start_rel) = row_src_lc[cell_pos..].find("<td").or_else(|| {
                    row_src_lc[cell_pos..]
                        .find("<th")
                        .map(|i| {
                            // Use whichever is earlier if both present.
                            let td = row_src_lc[cell_pos..].find("<td").unwrap_or(usize::MAX);
                            if i <= td { i } else { td }
                        })
                }) {
                    let cell_start = cell_pos + cell_start_rel;
                    let tag_end = row_src_lc[cell_start..].find('>').map(|i| cell_start + i + 1).unwrap_or(cell_start + 3);
                    // Determine closing tag.
                    let is_th = row_src_lc[cell_start..].starts_with("<th");
                    let close_tag = if is_th { "</th>" } else { "</td>" };
                    let close_start = row_src_lc[tag_end..].find(close_tag).map(|i| tag_end + i).unwrap_or(row_src.len());
                    let cell_content = &row_src[tag_end..close_start];
                    // Recursively convert cell content (may have inline markup).
                    let cell_md = strip_tags(cell_content);
                    let cell_md = decode_entities(&cell_md).replace('|', "\\|").replace('\n', " ").trim().to_string();
                    cells.push(cell_md);
                    cell_pos = close_start + close_tag.len();
                } else {
                    break;
                }
            }

            if !cells.is_empty() {
                rows.push(cells);
            }
            search_from = tr_end + "</tr>".len();
        }

        if rows.is_empty() {
            return (String::new(), consumed);
        }

        let col_count = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        let mut gfm = String::new();
        let header = &rows[0];
        // Header row.
        gfm.push('|');
        for i in 0..col_count {
            let cell = header.get(i).map(String::as_str).unwrap_or("");
            gfm.push_str(&format!(" {} |", cell));
        }
        gfm.push('\n');
        // Separator.
        gfm.push('|');
        for _ in 0..col_count {
            gfm.push_str(" --- |");
        }
        gfm.push('\n');
        // Data rows.
        for row in rows.iter().skip(1) {
            gfm.push('|');
            for i in 0..col_count {
                let cell = row.get(i).map(String::as_str).unwrap_or("");
                gfm.push_str(&format!(" {} |", cell));
            }
            gfm.push('\n');
        }

        (gfm, consumed)
    }

    // ── ac:structured-macro look-ahead handler ─────────────────────────────
    // Called when we see `<ac:structured-macro` at `start_pos`.  Finds the
    // matching `</ac:structured-macro>`, extracts the `ac:name` attribute and
    // any `<ac:plain-text-body>` / `<ac:rich-text-body>` / `<ac:parameter>`
    // children, and returns (md_string, bytes_consumed).
    fn convert_ac_macro(input: &str, start_pos: usize) -> (String, usize) {
        let src = &input[start_pos..];
        let src_lc = src.to_ascii_lowercase();

        // Find the closing tag.
        let close = "</ac:structured-macro>";
        let close_lc = close.to_ascii_lowercase();
        let end_offset = src_lc.find(&close_lc).unwrap_or(src.len());
        let consumed = end_offset + close.len();
        let macro_src = &src[..end_offset];

        // Extract ac:name from opening tag.
        let name = extract_attr(macro_src, "ac:name").unwrap_or_default();
        let name_lc = name.to_ascii_lowercase();

        // Extract <ac:plain-text-body> content (the typical body for code macros).
        fn extract_body_tag<'a>(src: &'a str, tag: &str) -> Option<&'a str> {
            let src_lc = src.to_ascii_lowercase();
            let open = format!("<{}", tag);
            let close = format!("</{}>", tag);
            let start = src_lc.find(&open)?;
            let open_end = src_lc[start..].find('>')? + start + 1;
            let end = src_lc[open_end..].find(&close)? + open_end;
            Some(&src[open_end..end])
        }

        // Extract a parameter by name: <ac:parameter ac:name="X">value</ac:parameter>
        fn extract_param(macro_src: &str, param_name: &str) -> Option<String> {
            // Find <ac:parameter ac:name="X"> by scanning manually.
            let src_lc = macro_src.to_ascii_lowercase();
            let needle = "ac:parameter";
            let mut search = 0;
            while let Some(rel) = src_lc[search..].find(needle) {
                let tag_start = search + rel;
                // Back up to the `<`.
                let lt = src_lc[..tag_start].rfind('<').unwrap_or(0);
                let tag_end = src_lc[tag_start..].find('>').map(|i| tag_start + i + 1).unwrap_or(tag_start + 1);
                let tag_inner = &macro_src[lt + 1..tag_end - 1];
                let actual_name = extract_attr(tag_inner, "ac:name").unwrap_or_default();
                if actual_name.to_ascii_lowercase() == param_name {
                    // Find the closing </ac:parameter>.
                    let close = "</ac:parameter>";
                    let close_lc = close.to_ascii_lowercase();
                    if let Some(close_rel) = src_lc[tag_end..].find(&close_lc) {
                        return Some(macro_src[tag_end..tag_end + close_rel].to_string());
                    }
                }
                search = tag_end;
            }
            None
        }

        let md = match name_lc.as_str() {
            // ── Code macro → fenced code block ────────────────────────────
            "code" => {
                let lang = extract_param(macro_src, "language").unwrap_or_default();
                let body = extract_body_tag(macro_src, "ac:plain-text-body")
                    .or_else(|| extract_body_tag(macro_src, "ac:rich-text-body"))
                    .unwrap_or("");
                format!("\n```{}\n{}\n```\n", lang, body.trim())
            }
            // ── Panel macros (info / note / warning / tip) → blockquote ──
            "info" | "note" | "warning" | "tip" => {
                let prefix = match name_lc.as_str() {
                    "info" => "ℹ INFO",
                    "note" => "NOTE",
                    "warning" => "WARNING",
                    "tip" => "TIP",
                    _ => "",
                };
                let body = extract_body_tag(macro_src, "ac:rich-text-body")
                    .or_else(|| extract_body_tag(macro_src, "ac:plain-text-body"))
                    .unwrap_or("");
                let body_md = storage_to_markdown(body);
                // Prefix every line with `> `.
                let quoted: String = body_md
                    .lines()
                    .map(|l| format!("> {}", l))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("\n> **{}**\n{}\n", prefix, quoted)
            }
            // ── Status macro → inline badge ────────────────────────────────
            "status" => {
                let label = extract_param(macro_src, "title").unwrap_or_else(|| name.clone());
                format!("[{}]", label.to_uppercase())
            }
            // ── Unknown macro → strip silently ────────────────────────────
            _ => String::new(),
        };

        (md, consumed)
    }

    // ── ac:image look-ahead handler ────────────────────────────────────────
    // Handles both self-closing `<ac:image ... />` and
    // `<ac:image ...><ri:attachment .../></ac:image>` patterns.
    fn convert_ac_image(input: &str, start_pos: usize) -> (String, usize) {
        let src = &input[start_pos..];
        let src_lc = src.to_ascii_lowercase();

        // Find the end: either `/>` (self-closing) or `</ac:image>`.
        let self_close = src_lc.find("/>").unwrap_or(usize::MAX);
        let close_tag = src_lc.find("</ac:image>").unwrap_or(usize::MAX);
        let (end_offset, consumed) = if self_close <= close_tag {
            (self_close + 2, self_close + 2)
        } else {
            let ct_len = "</ac:image>".len();
            (close_tag + ct_len, close_tag + ct_len)
        };

        let macro_src = &src[..end_offset];

        // Try to extract filename from <ri:attachment ri:filename="..."/>.
        let filename = {
            let m_lc = macro_src.to_ascii_lowercase();
            if let Some(att_start) = m_lc.find("<ri:attachment") {
                let tag_end = m_lc[att_start..].find('>').map(|i| att_start + i + 1).unwrap_or(att_start + 1);
                let tag_inner = &macro_src[att_start + 1..tag_end.saturating_sub(1)];
                extract_attr(tag_inner, "ri:filename")
            } else {
                None
            }
        };

        let alt = filename.as_deref().unwrap_or("image");
        let src_attr = filename.as_deref().unwrap_or("image");
        let md = format!("![{}]({})", alt, src_attr);
        (md, consumed)
    }

    // ── Main parsing loop ──────────────────────────────────────────────────
    while pos < input.len() {
        if input[pos..].starts_with('<') {
            // Peek at the tag to decide if we need special look-ahead handling
            // before doing the standard single-tag parse.

            // ── Table look-ahead ──────────────────────────────────────────
            if input[pos..].len() >= 6
                && input[pos..pos + 6].to_ascii_lowercase().starts_with("<table")
            {
                push_block_sep!();
                let (gfm, consumed) = convert_table(input, pos);
                out.push_str(&gfm);
                pos += consumed;
                pending_newline = true;
                continue;
            }

            // ── ac:structured-macro look-ahead ────────────────────────────
            if input[pos..]
                .to_ascii_lowercase()
                .starts_with("<ac:structured-macro")
            {
                push_block_sep!();
                let (md, consumed) = convert_ac_macro(input, pos);
                out.push_str(&md);
                pos += consumed;
                pending_newline = true;
                continue;
            }

            // ── ac:image look-ahead ────────────────────────────────────────
            if input[pos..].to_ascii_lowercase().starts_with("<ac:image") {
                let (md, consumed) = convert_ac_image(input, pos);
                out.push_str(&md);
                pos += consumed;
                continue;
            }

            // ── Standard single-tag path ───────────────────────────────────
            // Find the end of the tag.
            let tag_end = match input[pos..].find('>') {
                Some(i) => pos + i + 1,
                None => {
                    // Malformed; emit the rest as text.
                    out.push_str(&decode_entities(&input[pos..]));
                    break;
                }
            };
            let tag_str = &input[pos..tag_end]; // e.g. `<h2>` or `</ul>` or `<a href="...">`
            pos = tag_end;

            // Detect closing tag.
            let inner = &tag_str[1..tag_str.len() - 1]; // strip < >
            let is_close = inner.starts_with('/');
            let inner = if is_close { &inner[1..] } else { inner };

            // Tag name is up to the first whitespace or end.
            let tag_name_end = inner
                .find(|c: char| c.is_ascii_whitespace())
                .unwrap_or(inner.len());
            let tag_name = &inner[..tag_name_end];
            let tag_name_lower = tag_name.to_ascii_lowercase();

            // Handle self-closing (e.g. `<br />`, `<br/>`).
            let is_self_close = inner.ends_with('/') || tag_name_lower == "br";

            match tag_name_lower.as_str() {
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    if !is_close {
                        push_block_sep!();
                        let level: usize = tag_name_lower[1..].parse().unwrap_or(1);
                        let hashes = "#".repeat(level);
                        out.push_str(&hashes);
                        out.push(' ');
                        pending_newline = false;
                    } else {
                        out.push('\n');
                        pending_newline = true;
                    }
                }
                "p" => {
                    if !is_close {
                        if !out.is_empty() {
                            push_block_sep!();
                        }
                        pending_newline = false;
                    } else {
                        // End of paragraph: ensure blank line follows.
                        if !out.ends_with('\n') {
                            out.push('\n');
                        }
                        pending_newline = true;
                    }
                }
                "br" => {
                    out.push('\n');
                }
                "pre" => {
                    if !is_close {
                        push_block_sep!();
                        out.push_str("```\n");
                        in_pre = true;
                    } else {
                        if !out.ends_with('\n') {
                            out.push('\n');
                        }
                        out.push_str("```\n");
                        in_pre = false;
                        pending_newline = true;
                    }
                }
                "code" => {
                    if in_pre {
                        // Inside <pre><code>: ignore the inner <code> tag.
                    } else if !is_close {
                        out.push('`');
                        in_code = true;
                    } else {
                        out.push('`');
                        in_code = false;
                    }
                }
                "strong" | "b" => {
                    out.push_str("**");
                }
                "em" | "i" => {
                    out.push('*');
                }
                "blockquote" => {
                    // Simple blockquote: insert `> ` prefix on open.
                    if !is_close {
                        push_block_sep!();
                        out.push_str("> ");
                    } else if !out.ends_with('\n') {
                        out.push('\n');
                    }
                }
                "ul" => {
                    if !is_close {
                        list_stack.push(ListKind::Ul);
                    } else {
                        list_stack.pop();
                        if list_stack.is_empty() && !out.ends_with('\n') {
                            out.push('\n');
                        }
                    }
                }
                "ol" => {
                    if !is_close {
                        list_stack.push(ListKind::Ol);
                        ol_counters.push(0);
                    } else {
                        list_stack.pop();
                        ol_counters.pop();
                        if list_stack.is_empty() && !out.ends_with('\n') {
                            out.push('\n');
                        }
                    }
                }
                "li" => {
                    if !is_close {
                        // Indent by (depth-1)*2 spaces.
                        let depth = list_stack.len();
                        if depth > 1 {
                            let indent = "  ".repeat(depth - 1);
                            out.push_str(&indent);
                        }
                        match list_stack.last() {
                            Some(ListKind::Ul) => out.push_str("- "),
                            Some(ListKind::Ol) => {
                                if let Some(ctr) = ol_counters.last_mut() {
                                    *ctr += 1;
                                    out.push_str(&format!("{}. ", ctr));
                                }
                            }
                            None => out.push_str("- "),
                        }
                    } else {
                        out.push('\n');
                    }
                }
                "a" if !is_close => {
                    // Extract href attribute.
                    let href = extract_attr(inner, "href").unwrap_or_default();
                    // Look-ahead: find closing </a>, grab text, emit [text](href).
                    let rest = &input[pos..];
                    if let Some(close_a) = rest.find("</a>") {
                        let link_text_raw = &rest[..close_a];
                        let link_text = strip_tags(link_text_raw);
                        let link_text = decode_entities(&link_text);
                        if href.is_empty() {
                            out.push_str(&link_text);
                        } else {
                            out.push_str(&format!("[{}]({})", link_text, href));
                        }
                        pos += close_a + "</a>".len();
                    }
                    // No closing </a>: skip silently.
                }
                // ac:* and ri:* tags not caught by the look-ahead paths above:
                // strip silently (unknown Confluence markup).
                // <tbody> / <thead> / <tfoot> / <tr> / <td> / <th>: these are
                // consumed by the table look-ahead and should never appear here.
                // Strip them as a no-op if they somehow arrive.
                _ => {}
            }
            let _ = (is_self_close, in_code, pending_newline); // suppress unused warnings
        } else {
            // Text node.
            let next_tag = input[pos..].find('<').map(|i| pos + i).unwrap_or(input.len());
            let text_raw = &input[pos..next_tag];
            pos = next_tag;

            if text_raw.is_empty() {
                continue;
            }

            let text = decode_entities(text_raw);
            out.push_str(&text);
        }
    }

    // Trim trailing whitespace but preserve internal structure.
    let result = out.trim_end_matches('\n').to_string();
    // Collapse more-than-2 consecutive newlines to exactly 2.
    collapse_excess_newlines(&result)
}

/// Strip all `<tag>` / `</tag>` sequences from a string (keep text content).
fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pos = 0;
    while pos < s.len() {
        if s[pos..].starts_with('<') {
            if let Some(end) = s[pos..].find('>') {
                pos += end + 1;
            } else {
                break;
            }
        } else {
            let next = s[pos..].find('<').map(|i| pos + i).unwrap_or(s.len());
            out.push_str(&s[pos..next]);
            pos = next;
        }
    }
    out
}

/// Extract the value of a named attribute from tag inner content.
/// Handles both `name="value"` and `name='value'`.
fn extract_attr(tag_inner: &str, attr: &str) -> Option<String> {
    // Look for `attr="`  or `attr='`
    let needle_dq = format!("{}=\"", attr);
    let needle_sq = format!("{}='", attr);

    if let Some(start) = tag_inner.find(&needle_dq) {
        let after = &tag_inner[start + needle_dq.len()..];
        let end = after.find('"').unwrap_or(after.len());
        return Some(after[..end].to_string());
    }
    if let Some(start) = tag_inner.find(&needle_sq) {
        let after = &tag_inner[start + needle_sq.len()..];
        let end = after.find('\'').unwrap_or(after.len());
        return Some(after[..end].to_string());
    }
    None
}

/// Collapse runs of 3+ newlines into exactly 2.
fn collapse_excess_newlines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut nl_run = 0usize;
    for ch in s.chars() {
        if ch == '\n' {
            nl_run += 1;
            if nl_run <= 2 {
                out.push('\n');
            }
        } else {
            nl_run = 0;
            out.push(ch);
        }
    }
    out
}

/// Convert Markdown to Confluence storage-format XHTML.
///
/// Handles: ATX headings (`#`–`###`), paragraphs, unordered lists (`- ` / `* `),
/// ordered lists (`1. `), fenced code blocks (` ``` `), inline code (`` ` ``),
/// `**bold**`, `*italic*`, `[text](url)`.  Escapes `&`, `<`, `>` in plain text.
pub fn markdown_to_storage(md: &str) -> String {
    let mut out = String::new();
    let lines: Vec<&str> = md.lines().collect();
    let mut i = 0;

    // State for lists.
    let mut in_ul = false;
    let mut in_ol = false;
    let mut in_fence = false;

    fn escape_xml(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    /// Convert inline Markdown spans (bold, italic, inline code, links) to storage XHTML.
    fn inline_to_storage(text: &str) -> String {
        let mut out = String::new();
        let chars: Vec<char> = text.chars().collect();
        let n = chars.len();
        let mut j = 0;

        fn escape_xml_char_str(s: &str) -> String {
            s.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        }

        while j < n {
            // Inline code: `...`
            if chars[j] == '`' {
                if let Some(end) = text[j + 1..].find('`') {
                    let code_text = &text[j + 1..j + 1 + end];
                    out.push_str(&format!("<code>{}</code>", escape_xml_char_str(code_text)));
                    j += end + 2;
                    continue;
                }
            }
            // Bold: **...**
            if j + 1 < n && chars[j] == '*' && chars[j + 1] == '*' {
                if let Some(end) = text[j + 2..].find("**") {
                    let bold_text = &text[j + 2..j + 2 + end];
                    out.push_str(&format!(
                        "<strong>{}</strong>",
                        escape_xml_char_str(bold_text)
                    ));
                    j += end + 4;
                    continue;
                }
            }
            // Italic: *...*  (single asterisk)
            if chars[j] == '*' && (j + 1 >= n || chars[j + 1] != '*') {
                if let Some(end) = text[j + 1..].find('*') {
                    let italic_text = &text[j + 1..j + 1 + end];
                    out.push_str(&format!("<em>{}</em>", escape_xml_char_str(italic_text)));
                    j += end + 2;
                    continue;
                }
            }
            // Link: [text](url)
            if chars[j] == '[' {
                if let Some(close_bracket) = text[j..].find("](") {
                    let text_part = &text[j + 1..j + close_bracket];
                    let after = &text[j + close_bracket + 2..];
                    if let Some(close_paren) = after.find(')') {
                        let url_part = &after[..close_paren];
                        out.push_str(&format!(
                            "<a href=\"{}\">{}</a>",
                            escape_xml_char_str(url_part),
                            escape_xml_char_str(text_part)
                        ));
                        j += close_bracket + 2 + close_paren + 1;
                        continue;
                    }
                }
            }
            // Plain character — escape XML-significant chars.
            let ch = chars[j];
            match ch {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                _ => out.push(ch),
            }
            j += 1;
        }
        out
    }

    while i < lines.len() {
        let line = lines[i];

        // Fenced code block.
        if line.trim_start().starts_with("```") {
            // Close any open lists first.
            if in_ul {
                out.push_str("</ul>");
                in_ul = false;
            }
            if in_ol {
                out.push_str("</ol>");
                in_ol = false;
            }
            if in_fence {
                out.push_str("\n</pre>");
                in_fence = false;
            } else {
                out.push_str("<pre>");
                in_fence = true;
            }
            i += 1;
            continue;
        }

        if in_fence {
            out.push('\n');
            out.push_str(&escape_xml(line));
            i += 1;
            continue;
        }

        // ATX headings.
        if let Some(rest) = line.strip_prefix("### ") {
            close_lists(&mut out, &mut in_ul, &mut in_ol);
            out.push_str(&format!("<h3>{}</h3>", inline_to_storage(rest)));
            i += 1;
            continue;
        }
        if let Some(rest) = line.strip_prefix("## ") {
            close_lists(&mut out, &mut in_ul, &mut in_ol);
            out.push_str(&format!("<h2>{}</h2>", inline_to_storage(rest)));
            i += 1;
            continue;
        }
        if let Some(rest) = line.strip_prefix("# ") {
            close_lists(&mut out, &mut in_ul, &mut in_ol);
            out.push_str(&format!("<h1>{}</h1>", inline_to_storage(rest)));
            i += 1;
            continue;
        }

        // Unordered list items: `- ` or `* `.
        if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            if in_ol {
                out.push_str("</ol>");
                in_ol = false;
            }
            if !in_ul {
                out.push_str("<ul>");
                in_ul = true;
            }
            out.push_str(&format!("<li>{}</li>", inline_to_storage(rest)));
            i += 1;
            continue;
        }

        // Ordered list items: `N. `.
        if let Some(rest) = strip_ordered_item(line) {
            if in_ul {
                out.push_str("</ul>");
                in_ul = false;
            }
            if !in_ol {
                out.push_str("<ol>");
                in_ol = true;
            }
            out.push_str(&format!("<li>{}</li>", inline_to_storage(rest)));
            i += 1;
            continue;
        }

        // Blank line — close any open list.
        if line.trim().is_empty() {
            if in_ul {
                out.push_str("</ul>");
                in_ul = false;
            }
            if in_ol {
                out.push_str("</ol>");
                in_ol = false;
            }
            i += 1;
            continue;
        }

        // Regular paragraph text — wrap with <p>.
        close_lists(&mut out, &mut in_ul, &mut in_ol);
        out.push_str(&format!("<p>{}</p>", inline_to_storage(line)));
        i += 1;
    }

    // Close any open blocks.
    if in_ul {
        out.push_str("</ul>");
    }
    if in_ol {
        out.push_str("</ol>");
    }
    if in_fence {
        out.push_str("\n</pre>");
    }

    out
}

/// Close open list tags, updating the flags in place.
fn close_lists(out: &mut String, in_ul: &mut bool, in_ol: &mut bool) {
    if *in_ul {
        out.push_str("</ul>");
        *in_ul = false;
    }
    if *in_ol {
        out.push_str("</ol>");
        *in_ol = false;
    }
}

/// If `line` starts with an ordered-list marker like `1. ` or `12. `, return the rest.
fn strip_ordered_item(line: &str) -> Option<&str> {
    let dot_pos = line.find(". ")?;
    let prefix = &line[..dot_pos];
    if !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit()) {
        Some(&line[dot_pos + 2..])
    } else {
        None
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── storage_to_markdown ──────────────────────────────────────────────────

    #[test]
    fn test_storage_to_markdown_basic() {
        let storage = "<h2>Title</h2><p>Hello <a href=\"x\">link</a></p><ul><li>a</li><li>b</li></ul>";
        let md = storage_to_markdown(storage);

        // Should contain a level-2 heading.
        assert!(md.contains("## Title"), "missing heading; got: {md:?}");
        // Should contain the link in markdown syntax.
        assert!(md.contains("[link](x)"), "missing link; got: {md:?}");
        // Should contain list items.
        assert!(md.contains("- a"), "missing list item a; got: {md:?}");
        assert!(md.contains("- b"), "missing list item b; got: {md:?}");
        // "Hello" text should also be present.
        assert!(md.contains("Hello"), "missing Hello text; got: {md:?}");
    }

    #[test]
    fn test_storage_to_markdown_headings() {
        let storage = "<h1>One</h1><h2>Two</h2><h3>Three</h3>";
        let md = storage_to_markdown(storage);
        assert!(md.contains("# One"), "h1; got: {md:?}");
        assert!(md.contains("## Two"), "h2; got: {md:?}");
        assert!(md.contains("### Three"), "h3; got: {md:?}");
    }

    #[test]
    fn test_storage_to_markdown_entities() {
        let storage = "<p>&amp; &lt;b&gt; &quot;hello&quot;</p>";
        let md = storage_to_markdown(storage);
        assert!(md.contains("& <b>"), "entities; got: {md:?}");
        assert!(md.contains("\"hello\""), "quot entity; got: {md:?}");
    }

    #[test]
    fn test_storage_to_markdown_pre_code() {
        let storage = "<pre><code>fn main() {}</code></pre>";
        let md = storage_to_markdown(storage);
        assert!(md.contains("```"), "fenced code; got: {md:?}");
        assert!(md.contains("fn main()"), "code content; got: {md:?}");
    }

    // ── markdown_to_storage ──────────────────────────────────────────────────

    #[test]
    fn test_markdown_to_storage_basic() {
        let md = "## H\npara\n- one";
        let storage = markdown_to_storage(md);

        assert!(storage.contains("<h2>"), "h2 tag; got: {storage:?}");
        assert!(storage.contains("</h2>"), "h2 close; got: {storage:?}");
        assert!(storage.contains("<p>"), "p tag; got: {storage:?}");
        assert!(storage.contains("<ul>"), "ul tag; got: {storage:?}");
        assert!(storage.contains("<li>"), "li tag; got: {storage:?}");
    }

    #[test]
    fn test_markdown_to_storage_headings() {
        let storage = markdown_to_storage("# A\n## B\n### C");
        assert!(storage.contains("<h1>A</h1>"), "h1; got: {storage:?}");
        assert!(storage.contains("<h2>B</h2>"), "h2; got: {storage:?}");
        assert!(storage.contains("<h3>C</h3>"), "h3; got: {storage:?}");
    }

    #[test]
    fn test_markdown_to_storage_list() {
        let storage = markdown_to_storage("- one\n- two\n- three");
        assert!(storage.contains("<ul>"), "ul open; got: {storage:?}");
        assert!(storage.contains("</ul>"), "ul close; got: {storage:?}");
        assert!(storage.contains("<li>one</li>"), "item one; got: {storage:?}");
        assert!(storage.contains("<li>two</li>"), "item two; got: {storage:?}");
    }

    #[test]
    fn test_markdown_to_storage_ordered_list() {
        let storage = markdown_to_storage("1. first\n2. second");
        assert!(storage.contains("<ol>"), "ol open; got: {storage:?}");
        assert!(storage.contains("</ol>"), "ol close; got: {storage:?}");
        assert!(storage.contains("<li>first</li>"), "item first; got: {storage:?}");
    }

    #[test]
    fn test_markdown_to_storage_fenced_code() {
        let storage = markdown_to_storage("```\nlet x = 1;\n```");
        assert!(storage.contains("<pre>"), "pre tag; got: {storage:?}");
        assert!(storage.contains("let x = 1;"), "code content; got: {storage:?}");
    }

    #[test]
    fn test_markdown_to_storage_link() {
        let storage = markdown_to_storage("[click here](https://example.com)");
        assert!(
            storage.contains("<a href=\"https://example.com\">click here</a>"),
            "link; got: {storage:?}"
        );
    }

    #[test]
    fn test_markdown_to_storage_escapes_xml() {
        let storage = markdown_to_storage("A & B < C > D");
        assert!(storage.contains("&amp;"), "amp escape; got: {storage:?}");
        assert!(storage.contains("&lt;"), "lt escape; got: {storage:?}");
        assert!(storage.contains("&gt;"), "gt escape; got: {storage:?}");
    }

    // ── ConfluenceClient::new strips trailing /wiki ──────────────────────────

    #[test]
    fn test_new_strips_wiki_suffix() {
        let client = ConfluenceClient::new("https://site.atlassian.net/wiki", "e", "t");
        assert_eq!(client.site_base, "https://site.atlassian.net");
    }

    #[test]
    fn test_new_strips_trailing_slash() {
        let client = ConfluenceClient::new("https://site.atlassian.net/", "e", "t");
        assert_eq!(client.site_base, "https://site.atlassian.net");
    }

    #[test]
    fn test_new_no_change_needed() {
        let client = ConfluenceClient::new("https://site.atlassian.net", "e", "t");
        assert_eq!(client.site_base, "https://site.atlassian.net");
    }

    #[test]
    fn test_api_url_construction() {
        let client = ConfluenceClient::new("https://site.atlassian.net", "e", "t");
        assert_eq!(
            client.api("/content/123"),
            "https://site.atlassian.net/wiki/rest/api/content/123"
        );
    }

    // ── build_page_cql ───────────────────────────────────────────────────────

    #[test]
    fn test_cql_all_digits_is_id_lookup() {
        let cql = build_page_cql(None, "12345");
        assert!(cql.contains("id=12345"), "should contain id=12345; got: {cql:?}");
        assert!(!cql.contains("title ~"), "should NOT contain title ~; got: {cql:?}");
    }

    #[test]
    fn test_cql_digits_with_space_key_still_id_lookup() {
        let cql = build_page_cql(Some("DEV"), "99");
        assert!(cql.contains("id=99"), "should contain id=99; got: {cql:?}");
        assert!(!cql.contains("title ~"), "should NOT contain title ~; got: {cql:?}");
    }

    #[test]
    fn test_cql_name_with_space_key() {
        let cql = build_page_cql(Some("DEV"), "onboarding guide");
        assert!(cql.contains("space=\"DEV\""), "should contain space clause; got: {cql:?}");
        assert!(cql.contains("title ~ \"onboarding guide\""), "should contain title ~; got: {cql:?}");
        assert!(cql.contains("type=page"), "should contain type=page; got: {cql:?}");
    }

    #[test]
    fn test_cql_name_without_space_key() {
        let cql = build_page_cql(None, "release notes");
        assert!(!cql.contains("space="), "should NOT have space clause; got: {cql:?}");
        assert!(cql.contains("type=page"), "should contain type=page; got: {cql:?}");
        assert!(cql.contains("title ~"), "should contain title ~; got: {cql:?}");
    }

    #[test]
    fn test_cql_name_empty_space_key_treated_as_none() {
        let cql = build_page_cql(Some(""), "api design");
        assert!(!cql.contains("space="), "empty space key → no space clause; got: {cql:?}");
        assert!(cql.contains("title ~"), "should contain title ~; got: {cql:?}");
    }

    #[test]
    fn test_cql_escapes_double_quote_in_query() {
        let cql = build_page_cql(None, "say \"hello\"");
        // The CQL literal should have the quote escaped as \"
        assert!(cql.contains("\\\"hello\\\""), "quote should be escaped; got: {cql:?}");
    }

    #[test]
    fn test_cql_escapes_backslash_in_query() {
        let cql = build_page_cql(None, r"path\to");
        assert!(cql.contains("\\\\"), "backslash should be escaped; got: {cql:?}");
    }

    #[test]
    fn test_cql_trims_whitespace_from_query() {
        let cql = build_page_cql(None, "  onboard  ");
        assert!(cql.contains("title ~ \"onboard\""), "should trim query; got: {cql:?}");
    }

    // ── storage_to_markdown fidelity: table → GFM pipe-table ────────────────

    #[test]
    fn test_storage_to_markdown_table_basic() {
        let storage = r#"
<table>
  <tr><th>Name</th><th>Age</th></tr>
  <tr><td>Alice</td><td>30</td></tr>
  <tr><td>Bob</td><td>25</td></tr>
</table>"#;
        let md = storage_to_markdown(storage);

        // Header row.
        assert!(md.contains("| Name |"), "header Name; got: {md:?}");
        assert!(md.contains("| Age |"), "header Age; got: {md:?}");
        // Separator.
        assert!(md.contains("| --- |"), "separator; got: {md:?}");
        // Data rows.
        assert!(md.contains("| Alice |"), "Alice row; got: {md:?}");
        assert!(md.contains("| Bob |"), "Bob row; got: {md:?}");
        assert!(md.contains("| 30 |"), "30 cell; got: {md:?}");
        assert!(md.contains("| 25 |"), "25 cell; got: {md:?}");
    }

    #[test]
    fn test_storage_to_markdown_table_pipe_escaped() {
        // Pipe characters inside a cell must be escaped so the GFM is valid.
        let storage = "<table><tr><th>A</th></tr><tr><td>a | b</td></tr></table>";
        let md = storage_to_markdown(storage);
        // The pipe inside the cell should be escaped.
        assert!(md.contains("a \\| b"), "escaped pipe; got: {md:?}");
    }

    // ── storage_to_markdown fidelity: ac:structured-macro code block ─────────

    #[test]
    fn test_storage_to_markdown_code_macro() {
        let storage = r#"<ac:structured-macro ac:name="code">
  <ac:parameter ac:name="language">rust</ac:parameter>
  <ac:plain-text-body>fn hello() {}</ac:plain-text-body>
</ac:structured-macro>"#;
        let md = storage_to_markdown(storage);

        // Should be a fenced code block.
        assert!(md.contains("```rust"), "rust fence; got: {md:?}");
        assert!(md.contains("fn hello()"), "code body; got: {md:?}");
        assert!(md.contains("```"), "closing fence; got: {md:?}");
    }

    #[test]
    fn test_storage_to_markdown_code_macro_no_language() {
        // When no language parameter is given the fence should still be valid.
        let storage = r#"<ac:structured-macro ac:name="code">
  <ac:plain-text-body>SELECT 1;</ac:plain-text-body>
</ac:structured-macro>"#;
        let md = storage_to_markdown(storage);
        assert!(md.contains("```\n"), "fence without lang; got: {md:?}");
        assert!(md.contains("SELECT 1;"), "SQL body; got: {md:?}");
    }

    #[test]
    fn test_storage_to_markdown_info_panel() {
        let storage = r#"<ac:structured-macro ac:name="info">
  <ac:rich-text-body><p>This is informational text.</p></ac:rich-text-body>
</ac:structured-macro>"#;
        let md = storage_to_markdown(storage);
        // Should be rendered as a blockquote with a label.
        assert!(md.contains("> **"), "blockquote with bold label; got: {md:?}");
        assert!(md.contains("INFO"), "INFO label; got: {md:?}");
        assert!(md.contains("informational text"), "body content; got: {md:?}");
    }

    #[test]
    fn test_storage_to_markdown_warning_panel() {
        let storage = r#"<ac:structured-macro ac:name="warning">
  <ac:rich-text-body><p>Do not proceed.</p></ac:rich-text-body>
</ac:structured-macro>"#;
        let md = storage_to_markdown(storage);
        assert!(md.contains("WARNING"), "WARNING label; got: {md:?}");
        assert!(md.contains("Do not proceed"), "body; got: {md:?}");
    }

    #[test]
    fn test_storage_to_markdown_unknown_macro_is_silent() {
        // An unrecognised macro should not produce error output, just be dropped.
        let storage = r#"<p>Before</p><ac:structured-macro ac:name="fancy-widget">
  <ac:rich-text-body>hidden</ac:rich-text-body>
</ac:structured-macro><p>After</p>"#;
        let md = storage_to_markdown(storage);
        assert!(md.contains("Before"), "before text; got: {md:?}");
        assert!(md.contains("After"), "after text; got: {md:?}");
        // The body of an unknown macro is silently dropped.
        assert!(!md.contains("hidden"), "unknown macro body must be dropped; got: {md:?}");
    }

    // ── storage_to_markdown fidelity: ac:image → Markdown image link ──────────

    #[test]
    fn test_storage_to_markdown_image_with_attachment() {
        let storage = r#"<ac:image><ri:attachment ri:filename="diagram.png"/></ac:image>"#;
        let md = storage_to_markdown(storage);
        assert!(md.contains("![diagram.png](diagram.png)"), "image link; got: {md:?}");
    }

    #[test]
    fn test_storage_to_markdown_image_self_closing() {
        // Some macros render as self-closing without a nested ri:attachment.
        let storage = r#"<ac:image ri:filename="chart.svg"/>"#;
        let md = storage_to_markdown(storage);
        // Even without the nested tag the output should be a Markdown image.
        assert!(md.contains("!["), "image syntax; got: {md:?}");
    }
}
