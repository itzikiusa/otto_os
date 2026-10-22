//! Confluence pages; Channels / workspace integrations; Provider updates.
//!
//! Split out of `api.rs` by area (re-exported from there, so every path is
//! still `otto_core::api::*`). Mirrored by `ui/src/lib/api/types/integrations.ts`.
use super::*;

// ---------------------------------------------------------------------------
// Confluence pages
//
// Callers speak Markdown, never Confluence storage XHTML: every body crosses
// this boundary as `body_md` and the server converts (`markdown_to_storage` on
// the way in, `storage_to_markdown` on the way out). Agents and skills author
// Markdown; the storage format is an implementation detail of the wire.
// ---------------------------------------------------------------------------

/// A Confluence page as returned by the page endpoints, body rendered to Markdown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfluencePageResp {
    pub id: String,
    pub title: String,
    pub space_key: String,
    /// Human-friendly browser URL.
    pub url: String,
    /// Current version number. Updates are applied against this automatically.
    pub version: i64,
    /// Page body converted from storage XHTML to Markdown.
    pub body_md: String,
}

/// `POST /api/v1/issue/confluence/pages?account_id=`
///
/// Supply EITHER `body_md` (Markdown, converted server-side) or `body_html`
/// (Confluence storage XHTML, passed through untouched). `body_html` is the
/// escape hatch for anything Markdown cannot express — panel/expand/status
/// macros, layouts, nested tables.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateConfluencePageReq {
    pub space_key: String,
    pub title: String,
    /// Page body in Markdown; converted to storage format server-side.
    #[serde(default)]
    pub body_md: Option<String>,
    /// Page body already in Confluence storage format (XHTML). Wins over
    /// `body_md` when both are present.
    #[serde(default)]
    pub body_html: Option<String>,
    /// Create as a child of this page when present.
    #[serde(default)]
    pub parent_id: Option<String>,
}

/// `PUT /api/v1/issue/confluence/pages/{page_id}?account_id=`
///
/// The current version is resolved server-side, so callers never track version
/// numbers — a stale number is the classic source of lost Confluence edits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateConfluencePageReq {
    /// Absent → keep the page's current title.
    #[serde(default)]
    pub title: Option<String>,
    /// Replacement page body in Markdown.
    #[serde(default)]
    pub body_md: Option<String>,
    /// Replacement body already in Confluence storage format (XHTML). Wins over
    /// `body_md` when both are present.
    #[serde(default)]
    pub body_html: Option<String>,
    /// The page `version` this edit was based on (from the page read). When
    /// present and the page has moved on since, the update is refused with 409
    /// instead of silently overwriting the newer (e.g. human) edit. Absent →
    /// last-writer-wins against the current version (legacy behaviour).
    #[serde(default)]
    pub base_version: Option<i64>,
}

/// `POST /api/v1/issue/confluence/pages/{page_id}/comments?account_id=`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddConfluenceCommentReq {
    /// Comment body in Markdown.
    #[serde(default)]
    pub body_md: Option<String>,
    /// Comment body already in Confluence storage format (XHTML).
    #[serde(default)]
    pub body_html: Option<String>,
}

// ---------------------------------------------------------------------------
// Channels / workspace integrations
// ---------------------------------------------------------------------------

/// `PUT /api/v1/workspaces/{id}/integrations/{channel}`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertIntegrationReq {
    pub enabled: bool,
    /// Write-only; when Some and non-empty the token is (re-)stored in keychain.
    pub bot_token: Option<String>,
    /// Write-only; Slack app-level token (slack only).
    pub app_token: Option<String>,
    pub allowed_users: String,
    /// The explicit "open to everyone" opt-in (see `Integration::open_to_all`).
    /// Omitted ⇒ the stored value is kept.
    #[serde(default)]
    pub open_to_all: Option<bool>,
    pub agent_reply: bool,
    pub reply_instructions: String,
    pub channel_id: String,
    /// Preferred agent CLI for this channel. Empty = use the default agent.
    #[serde(default)]
    pub preferred_cli: String,
}

// ---------------------------------------------------------------------------
// Provider updates
// ---------------------------------------------------------------------------

/// `POST /api/v1/workspaces/{id}/providers/update`
///
/// When `provider` is `Some`, only that provider's update command is run.
/// When `None`, all providers with a configured update command are updated.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateProvidersReq {
    /// Optionally restrict to a single named provider.
    pub provider: Option<String>,
}
