//! Wire request/response DTOs for the control-plane HTTP surface. Mirrored in
//! `ui/src/lib/api/types.ts`. Secret *values* (`secret_env`/`secret_headers`) are
//! accepted on write and routed to the keychain; they are never returned.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

fn default_transport() -> String {
    "stdio".into()
}
fn default_access() -> String {
    "allow".into()
}
fn default_injection() -> String {
    "medium".into()
}

#[derive(Debug, Deserialize)]
pub struct CreateServerReq {
    pub name: String,
    #[serde(default = "default_transport")]
    pub transport: String, // 'stdio' | 'http'
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// Secret env values → keychain (never stored in the row, never returned).
    #[serde(default)]
    pub secret_env: BTreeMap<String, String>,
    #[serde(default)]
    pub secret_headers: BTreeMap<String, String>,
    #[serde(default = "default_injection")]
    pub injection_risk: String,
    #[serde(default = "default_access")]
    pub default_tool_access: String,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Default, Deserialize)]
pub struct UpdateServerReq {
    pub name: Option<String>,
    pub description: Option<String>,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub env: Option<BTreeMap<String, String>>,
    pub url: Option<String>,
    pub headers: Option<BTreeMap<String, String>>,
    pub secret_env: Option<BTreeMap<String, String>>,
    pub secret_headers: Option<BTreeMap<String, String>>,
    pub injection_risk: Option<String>,
    pub default_tool_access: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub struct PatchToolReq {
    pub enabled: Option<bool>,
    pub require_approval: Option<bool>,
    pub risk_label: Option<String>,
    pub injection_risk: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AllowlistEntryReq {
    pub server_id: String,
    #[serde(default)]
    pub tool_name: Option<String>,
    pub mode: String, // 'allow' | 'deny'
}

#[derive(Debug, Deserialize)]
pub struct SetAllowlistReq {
    pub entries: Vec<AllowlistEntryReq>,
}

#[derive(Debug, Deserialize)]
pub struct CreatePolicyReq {
    #[serde(default)]
    pub workspace_id: Option<String>,
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default = "hundred")]
    pub priority: i64,
    #[serde(default, rename = "match")]
    pub match_json: Value,
    pub effect: String, // allow|deny|require_approval|require_dry_run
    #[serde(default)]
    pub reason: Option<String>,
}
fn yes() -> bool {
    true
}
fn hundred() -> i64 {
    100
}

#[derive(Debug, Default, Deserialize)]
pub struct UpdatePolicyReq {
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub priority: Option<i64>,
    #[serde(rename = "match")]
    pub match_json: Option<Value>,
    pub effect: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ImportPoliciesReq {
    pub policies: Vec<CreatePolicyReq>,
    /// Replace all existing rules (true) or merge/append (false).
    #[serde(default)]
    pub replace: bool,
}

#[derive(Debug, Deserialize)]
pub struct EvaluateReq {
    pub server_id: String,
    pub tool: String,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub arguments: Value,
}

#[derive(Debug, Deserialize)]
pub struct InvokeReq {
    #[serde(default)]
    pub arguments: Value,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub workspace_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DecideReq {
    pub approved: bool,
    #[serde(default)]
    pub note: Option<String>,
}

/// The result of a governed invoke, returned to the UI tester / gateway / otto-tools.
#[derive(Debug, Serialize)]
pub struct InvokeResp {
    /// allowed | approved | denied | dry_run | pending_approval | error
    pub decision: String,
    pub executed: bool,
    pub dry_run: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_id: Option<String>,
    /// The (redacted, capped) tool result content when executed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    /// The dry-run preview when `dry_run`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<Value>,
}
