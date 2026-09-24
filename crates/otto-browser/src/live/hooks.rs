//! What the live runtime needs from its host (the daemon): audit, event
//! broadcast, and the approval queue for an agent's outward actions. The
//! server implements [`LiveHooks`] on top of `ServerCtx` (audit log,
//! `Event` bus, MCP approvals); tests use [`NoopHooks`] / a recorder.

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use super::install::InstallJob;
use super::types::LiveSessionInfo;

/// An agent action that leaves the page (a form submit / non-GET document
/// request), held until a human decides.
#[derive(Debug, Clone, Serialize)]
pub struct OutwardAction {
    pub workspace_id: String,
    pub tab_id: String,
    pub owner_id: String,
    /// `scheme://host[:port]` of the page the action starts from.
    pub page_origin: String,
    pub page_title: String,
    /// HTTP method of the held request (POST, PUT, …).
    pub method: String,
    /// Host the request goes to (never the path/query — may carry tokens).
    pub target_host: String,
    /// The pre-action screenshot (JPEG), when it could be captured.
    pub screenshot_path: Option<String>,
}

impl OutwardAction {
    pub fn title(&self) -> String {
        format!(
            "Browser: {} to {} from {}",
            describe_method(&self.method),
            self.target_host,
            self.page_origin
        )
    }
}

fn describe_method(m: &str) -> &'static str {
    match m {
        "POST" => "submit a form",
        "PUT" | "PATCH" => "update data",
        "DELETE" => "delete data",
        _ => "send a request",
    }
}

/// One audit row (`action` e.g. `browser.live.navigate`).
#[derive(Debug, Clone)]
pub struct LiveAudit {
    pub action: &'static str,
    pub user_id: Option<String>,
    pub target: String,
    pub detail: Value,
}

#[async_trait::async_trait]
pub trait LiveHooks: Send + Sync {
    /// Best-effort; must never fail the action being audited.
    async fn audit(&self, entry: LiveAudit);
    /// A session opened / became ready / crashed / closed.
    fn session_changed(&self, info: &LiveSessionInfo);
    /// The engine download job ticked.
    fn install_progress(&self, job: &InstallJob);
    /// File an approval; returns its id.
    async fn request_approval(&self, action: &OutwardAction) -> Result<String, String>;
    /// Wait up to `timeout` for a decision: `Some(approved)`, or `None` when
    /// still undecided (treated as a denial by the caller).
    async fn await_approval(&self, approval_id: &str, timeout: Duration) -> Option<bool>;
}

/// No host: audits/events vanish and every outward action is denied.
pub struct NoopHooks;

#[async_trait::async_trait]
impl LiveHooks for NoopHooks {
    async fn audit(&self, _entry: LiveAudit) {}
    fn session_changed(&self, _info: &LiveSessionInfo) {}
    fn install_progress(&self, _job: &InstallJob) {}
    async fn request_approval(&self, _action: &OutwardAction) -> Result<String, String> {
        Err("no approval queue".into())
    }
    async fn await_approval(&self, _approval_id: &str, _timeout: Duration) -> Option<bool> {
        Some(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outward_titles_name_where_and_what() {
        let a = OutwardAction {
            workspace_id: "w".into(),
            tab_id: "t".into(),
            owner_id: "u".into(),
            page_origin: "https://shop.example".into(),
            page_title: "Cart".into(),
            method: "POST".into(),
            target_host: "pay.example".into(),
            screenshot_path: None,
        };
        assert_eq!(
            a.title(),
            "Browser: submit a form to pay.example from https://shop.example"
        );
    }
}
