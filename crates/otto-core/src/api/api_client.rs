//! API client ("Postman" section). Collection/request/environment routes are.
//!
//! Split out of `api.rs` by area (re-exported from there, so every path is
//! still `otto_core::api::*`). Mirrored by `ui/src/lib/api/types/api-client.ts`.
use super::*;

// ---------------------------------------------------------------------------
// API client ("Postman" section). Collection/request/environment routes are
// workspace-scoped (`/workspaces/{wid}/api-client/...`), so `workspace_id`
// comes from the path, not these bodies.
// ---------------------------------------------------------------------------

pub(super) fn default_body_mode() -> String {
    "none".to_string()
}

/// `POST/PATCH /workspaces/{wid}/api-client/collections[/{id}]`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertApiCollectionReq {
    pub name: String,
    #[serde(default)]
    pub parent_id: Option<Id>,
}

/// `POST/PATCH /workspaces/{wid}/api-client/requests[/{id}]`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertApiRequestReq {
    #[serde(default)]
    pub collection_id: Option<Id>,
    pub name: String,
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: Value,
    #[serde(default)]
    pub query: Value,
    #[serde(default = "default_body_mode")]
    pub body_mode: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub auth: Value,
    /// Optional `ssh`-kind connection id to tunnel this request through.
    #[serde(default)]
    pub ssh_connection_id: Option<Id>,
    /// Versioned extension object (scripts / docs / settings / GraphQL
    /// variables / transport). Must be a JSON object when present; capped at
    /// 256 KiB. See `ApiRequest::extras`.
    #[serde(default)]
    pub extras: Option<Value>,
}

/// `POST/PATCH /workspaces/{wid}/api-client/environments[/{id}]`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertApiEnvironmentReq {
    pub name: String,
    /// Non-secret variables. Any key also listed in `secret_keys` is stripped
    /// from the row (its value belongs in `secret_values`).
    #[serde(default)]
    pub variables: Value,
    /// Names of variables whose values are Keychain-backed. On update,
    /// OMITTED (absent / `null`) keeps the stored set and its values — only an
    /// explicit list replaces it (a key dropped from the list loses its
    /// Keychain value). On create, omitted = none.
    #[serde(default)]
    pub secret_keys: Option<Vec<String>>,
    /// WRITE-ONLY: new/changed secret values, keyed by variable name. Absent
    /// keys keep their previously stored value; values are never echoed back.
    #[serde(default)]
    pub secret_values: std::collections::BTreeMap<String, String>,
    /// Update only: `{old_name: new_name}` for renamed secret variables — the
    /// stored Keychain value moves to the new name (no retyping, nothing
    /// lost). A value sent for the new name in `secret_values` still wins.
    #[serde(default)]
    pub secret_renames: std::collections::BTreeMap<String, String>,
}

/// `POST /workspaces/{wid}/api-client/execute` — run a request through the
/// daemon. `{{var}}` placeholders are substituted from `environment_id` (or the
/// workspace's active environment when absent). The run is recorded in history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteApiReq {
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: Value,
    #[serde(default)]
    pub query: Value,
    #[serde(default = "default_body_mode")]
    pub body_mode: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub auth: Value,
    #[serde(default)]
    pub environment_id: Option<Id>,
    /// Per-request execution settings (timeout, redirects, TLS verification).
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub follow_redirects: Option<bool>,
    #[serde(default)]
    pub verify_ssl: Option<bool>,
    #[serde(default)]
    pub vars: Option<Value>,
    /// Optional `ssh`-kind connection id: route this request through a SOCKS5
    /// tunnel over that SSH bastion (for IP-whitelisted upstreams). None =
    /// send directly.
    #[serde(default)]
    pub ssh_connection_id: Option<Id>,
    /// Confirms sending a stored secret to a host it isn't bound to (a
    /// `$secret` marker is bound to its owning saved request's host; an
    /// environment secret to the hosts of the workspace's human-authored saved
    /// requests). Without it such a send is refused with `409
    /// needs_confirm=new_host`. Honoured only for a person's credential —
    /// agent callers can never self-confirm.
    #[serde(default)]
    pub confirm_new_host: bool,
}

/// Response of `POST .../api-client/execute`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse {
    pub status: u16,
    pub status_text: String,
    /// `[{ "key", "value" }]`
    pub headers: Value,
    /// UTF-8 (lossy) body for display, truncated to a display cap when large.
    pub body: String,
    /// Full response bytes, base64-encoded — only for `image/*` bodies up to
    /// 5 MiB (the inline preview). Empty otherwise: an exact UTF-8 `body` is
    /// already the whole payload, and anything else is downloaded through
    /// `body_id`. Empty when the response is `too_large`.
    #[serde(default)]
    pub body_base64: String,
    /// Handle for `GET /workspaces/{wid}/api-client/responses/{body_id}/raw`
    /// (the full bytes, for "Save to disk") when neither `body` nor
    /// `body_base64` carries them — a truncated or non-UTF-8 body. Held in a
    /// bounded daemon-side cache (10 min, per user); absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_id: Option<String>,
    /// `body` was cut to the display cap (full bytes via `body_id`).
    #[serde(default)]
    pub truncated: bool,
    /// Body exceeded the inline cap: neither `body` nor `body_base64` is set.
    #[serde(default)]
    pub too_large: bool,
    pub duration_ms: i64,
    pub size_bytes: i64,
    pub content_type: Option<String>,
    /// Per-phase trace of the request lifecycle (resolved request, TTFB,
    /// download, redirects, completion) for the response "Trace" tab.
    #[serde(default)]
    pub trace: Vec<TraceStep>,
}

/// One step in a request's execution trace (see [`ApiResponse::trace`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceStep {
    pub label: String,
    pub detail: String,
    /// Duration of this phase in milliseconds, when measured.
    #[serde(default)]
    pub ms: Option<i64>,
    /// One of: info | timing | redirect | success | error (UI styling hint).
    #[serde(default)]
    pub level: String,
}

/// `POST /api-client/import-curl` — parse a curl command into request fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportCurlReq {
    pub curl: String,
}

/// Parsed request fields from a curl command (drop into a request form).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedCurl {
    pub method: String,
    pub url: String,
    pub headers: Value,
    pub query: Value,
    pub body_mode: String,
    pub body: String,
    pub auth: Value,
}

/// `POST/PATCH /workspaces/{wid}/api-client/automations[/{id}]`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertApiAutomationReq {
    pub name: String,
    #[serde(default)]
    pub steps: Value,
}

/// One step's result in an automation run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiRunStepResult {
    pub request_id: Id,
    pub name: String,
    pub status: Option<u16>,
    pub duration_ms: i64,
    /// True when the request succeeded AND every assertion passed.
    pub ok: bool,
    /// `[{ "desc", "passed" }]`
    pub assertions: Value,
    pub error: Option<String>,
}

/// `POST /workspaces/{wid}/api-client/automations/{id}/run` → run report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiRunResult {
    pub automation_id: Id,
    pub steps: Vec<ApiRunStepResult>,
    /// True when every step was `ok`.
    pub passed: bool,
}

/// Run options. Dataset rows overlay the chosen environment independently;
/// values remain in memory and are never included in the persisted snapshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StartApiAutomationRunReq {
    #[serde(default)]
    pub environment_id: Option<Id>,
    #[serde(default)]
    pub stop_on_failure: bool,
    #[serde(default)]
    pub dataset: Vec<serde_json::Map<String, Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiAutomationRun {
    pub id: Id,
    pub workspace_id: Id,
    pub automation_id: Id,
    pub environment_id: Option<Id>,
    pub created_by: Id,
    /// running | passed | failed | cancelled | interrupted
    pub status: String,
    pub created_at: String,
    pub finished_at: Option<String>,
    pub stop_on_failure: bool,
    pub dataset_rows: usize,
    /// Redacted, ordered request/step metadata as it was when this run started.
    pub snapshot: Value,
    pub report: ApiRunResult,
    /// Dataset row index (zero based) for each result in report.steps.
    pub result_rows: Vec<usize>,
    /// Stable identifiers for completed step executions, parallel to report.steps.
    #[serde(default)]
    pub result_ids: Vec<Id>,
    pub error: Option<String>,
    /// Completed step count — set on run-LIST rows, which omit `report.steps`
    /// (perf F2); absent on a single-run read, where `report.steps` is whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps_total: Option<usize>,
    /// How many of those steps passed (list rows only, like `steps_total`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps_passed: Option<usize>,
}

/// `GET /workspaces/{wid}/api-client/overview` — the agent-facing discovery view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiOverview {
    /// Saved collection tree, without request contents.
    pub collections: Vec<ApiOverviewCollection>,
    /// Saved requests in a compact, secret-free shape.
    pub requests: Vec<ApiOverviewRequest>,
    /// Environments with secret values omitted or masked.
    pub environments: Vec<ApiOverviewEnvironment>,
    /// Saved automations in a compact shape.
    pub automations: Vec<ApiOverviewAutomation>,
}

/// One collection in the agent-facing API-client overview.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiOverviewCollection {
    /// Collection id.
    pub id: Id,
    /// Human-authored collection name.
    pub name: String,
    /// Parent collection for folder nesting; None = top-level.
    pub parent_id: Option<Id>,
}

/// One saved request in the agent-facing API-client overview.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiOverviewRequest {
    /// Saved request id.
    pub id: Id,
    /// Human-authored request name.
    pub name: String,
    /// HTTP method as saved.
    pub method: String,
    /// URL template as saved — `{{var}}` placeholders are not substituted.
    pub url: String,
    /// Parent collection, when assigned.
    pub collection_id: Option<Id>,
    /// `auth.type` (`none|bearer|basic|api_key|oauth2`); never a value.
    pub auth_type: String,
    /// Whether execution uses an SSH connection.
    pub has_ssh: bool,
    /// True when `extras.agent` is set (created/edited through an agent session).
    pub agent_authored: bool,
    /// Last saved timestamp.
    pub updated_at: DateTime<Utc>,
    /// Sort position within its collection (the UI tree orders by it).
    #[serde(default)]
    pub position: i64,
}

/// One environment in the agent-facing API-client overview.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiOverviewEnvironment {
    /// Environment id.
    pub id: Id,
    /// Human-authored environment name.
    pub name: String,
    /// Whether this is the workspace's active environment.
    pub is_active: bool,
    /// Non-secret variables; values whose key is secret-shaped are masked `***`.
    pub variables: Value,
    /// Names of Keychain-backed variables; never their values.
    pub secret_keys: Vec<String>,
}

/// One automation in the agent-facing API-client overview.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiOverviewAutomation {
    /// Automation id.
    pub id: Id,
    /// Human-authored automation name.
    pub name: String,
    /// Number of saved request steps.
    pub steps: usize,
}

/// `POST /workspaces/{wid}/api-client/requests/{id}/execute`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSavedRequestReq {
    /// Explicit environment; None uses the active environment.
    #[serde(default)]
    pub environment_id: Option<Id>,
    /// Runtime variable overrides; string values must not contain `{{`.
    #[serde(default)]
    pub vars: Option<Value>,
    /// Execution timeout, clamped to 1..=60000 milliseconds.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Required for methods other than GET, HEAD, and OPTIONS.
    #[serde(default)]
    pub confirm: bool,
    /// Required for an agent-authored request targeting an unknown host.
    #[serde(default)]
    pub confirm_new_host: bool,
    /// Response shape: `full` (default) or `agent`.
    #[serde(default)]
    pub shape: Option<String>,
    /// Decode safe JWT timing/issuer/audience claims; defaults to true.
    #[serde(default)]
    pub decode_jwt: Option<bool>,
}

/// Secret-free description of the saved request that was resolved and sent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResolvedRequest {
    /// Final HTTP method after the pre-request script.
    pub method: String,
    /// Substituted URL with every resolved secret string replaced by `***`.
    pub url: String,
    /// Selected environment id, when any.
    pub environment_id: Option<Id>,
    /// Selected environment name, when any.
    pub environment: Option<String>,
    /// Names of resolved secrets (environment keys and auth members), never values.
    pub secrets_used: Vec<String>,
}

/// Result of executing one saved API-client request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSavedRequestResp {
    /// Persisted API history entry id.
    pub history_id: Id,
    /// Saved request id.
    pub request_id: Id,
    /// Saved request name.
    pub name: String,
    /// Scrubbed upstream response.
    pub response: ApiResponse,
    /// Secret-free resolved request metadata.
    pub resolved: ApiResolvedRequest,
    /// Safe JWT claims keyed by JSON pointer or `text#n`; never tokens/signatures.
    #[serde(default)]
    pub jwt_claims: Option<Value>,
    /// Non-fatal post-response script errors.
    #[serde(default)]
    pub warnings: Vec<String>,
    /// Post-response script test results as `[{desc,passed}]`.
    #[serde(default)]
    pub script_tests: Value,
}
