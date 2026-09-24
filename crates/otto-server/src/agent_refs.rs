//! Cross-workspace discovery + friendly-reference resolution for every id an
//! agent tool takes — the generalisation of [`crate::repo_directory`].
//!
//! WHY: the git tools were fixed so an agent names a repo instead of guessing
//! an opaque id in the wrong workspace, but the SAME failure lived in every
//! other tool: `otto_list_workflows` / `_connections` / `_swarms` listed only
//! the session's own workspace, `search_issues` demanded an `account_id` no
//! tool listed, `list_scheduled_task_runs` wanted a task id, `k8s_*` a cluster
//! id, and a name passed where an id was expected produced a bare 404 the agent
//! could not recover from. This module is the one place that answers "which
//! object does this caller mean?" for those kinds:
//!
//! - [`directory`] — every object of a kind across EVERY workspace the caller
//!   can read, each row annotated with `workspace_id` + `workspace_name` (the
//!   caller's current workspace first), narrowed to an MCP token's pin.
//! - [`match_ref`] — pure matching of a reference (id, then each human field —
//!   name / title / Jira key / label — case-insensitively) against that set.
//! - [`resolve`] — the async wrapper: ambiguity → `Conflict` listing the
//!   candidates; nothing matched → `NotFound` listing the near misses or what
//!   IS available; an omitted reference picks the sole candidate for kinds
//!   where that is unambiguous (an issue account).
//!
//! Every lookup is a SELF-CALL of the kind's own list route AS the caller
//! (a short-lived token for the effective user, revoked afterwards), so the
//! route's native RBAC — feature grant, workspace role, per-resource grants,
//! owner checks — decides what is visible. Discovery can therefore never widen
//! access: a reference only ever resolves to an object the caller could
//! already list. Served over HTTP as `GET /refs/directory` + `GET
//! /refs/resolve` (the in-session `ottod mcp-tools` bridge calls these) and
//! used in-process by the governed `otto.*` choke point
//! (`mcp_outward::governed_invoke`).

use std::collections::HashSet;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::Json;
use otto_core::auth::AuthContext;
use otto_core::domain::User;
use otto_core::Error;
use otto_rbac::AuthRepo;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::CurrentAuthContext;
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// Cap on how many candidates an error message lists (the agent reads it).
const MAX_LISTED: usize = 25;
/// Wall-clock cap on one list self-call.
const SELF_CALL_TIMEOUT: Duration = Duration::from_secs(20);

/// Where a kind's rows live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    /// Each workspace has its own rows: list every readable workspace.
    Workspace,
    /// A global library served under a workspace path only for the role check
    /// (vaults, product stories): list through the first readable workspace.
    Library,
    /// A global, non-workspace list route (issue accounts, AWS accounts, K8s
    /// clusters, the design library, the workspaces themselves).
    Global,
}

/// One resolvable kind.
#[derive(Debug)]
pub(crate) struct RefKind {
    /// Stable key (`?kind=` on the HTTP routes).
    pub key: &'static str,
    /// Human noun for messages.
    pub noun: &'static str,
    pub scope: Scope,
    /// The list route under `/api/v1`; `{ws}` is replaced by the workspace id
    /// for `Workspace` / `Library` kinds.
    pub list: &'static str,
    /// Key of the rows array when the route answers an object.
    pub rows: Option<&'static str>,
    /// Key of the actual row object inside each item (`{room, members}`).
    pub row: Option<&'static str>,
    /// Human fields matched after the id, in order (case-insensitive, exact).
    pub names: &'static [&'static str],
    /// Row fields kept in a directory listing (`None` = the whole row). Keeps
    /// heavy payloads (a workflow's graph) out of a cross-workspace list.
    pub keep: Option<&'static [&'static str]>,
    /// The tool that lists this kind — named in error messages.
    pub list_tool: &'static str,
    /// An omitted reference resolves to the caller's ONLY candidate.
    pub sole_default: bool,
}

const fn kind(
    key: &'static str,
    noun: &'static str,
    scope: Scope,
    list: &'static str,
    names: &'static [&'static str],
    list_tool: &'static str,
) -> RefKind {
    RefKind {
        key,
        noun,
        scope,
        list,
        rows: None,
        row: None,
        names,
        keep: None,
        list_tool,
        sole_default: false,
    }
}

/// Every resolvable kind. The list routes are the ones the matching `list_*`
/// tools call, so "what a reference can resolve to" and "what the list tool
/// shows" never drift.
pub(crate) const KINDS: &[RefKind] = &[
    RefKind {
        keep: Some(&["id", "name", "root_path", "archived", "my_role"]),
        ..kind(
            "workspace",
            "workspace",
            Scope::Global,
            "/api/v1/workspaces",
            &["name"],
            "list_workspaces",
        )
    },
    RefKind {
        keep: Some(&[
            "id",
            "name",
            "description",
            "workspace_id",
            "version",
            "updated_at",
        ]),
        ..kind(
            "workflow",
            "workflow",
            Scope::Workspace,
            "/api/v1/workspaces/{ws}/workflows",
            &["name"],
            "list_workflows",
        )
    },
    RefKind {
        keep: Some(&[
            "id",
            "name",
            "kind",
            "environment",
            "read_only",
            "workspace_id",
            "section_id",
        ]),
        ..kind(
            "connection",
            "connection",
            Scope::Workspace,
            "/api/v1/workspaces/{ws}/connections",
            &["name"],
            "list_connections",
        )
    },
    RefKind {
        keep: Some(&[
            "id",
            "name",
            "workspace_id",
            "bootstrap_servers",
            "security_protocol",
            "environment",
            "read_only",
        ]),
        ..kind(
            "broker_cluster",
            "broker cluster",
            Scope::Workspace,
            "/api/v1/workspaces/{ws}/brokers/clusters",
            &["name"],
            "list_broker_clusters",
        )
    },
    RefKind {
        keep: Some(&[
            "id",
            "name",
            "description",
            "status",
            "workspace_id",
            "preset_slug",
        ]),
        ..kind(
            "swarm",
            "swarm",
            Scope::Workspace,
            "/api/v1/workspaces/{ws}/swarm/swarms",
            &["name"],
            "list_swarms",
        )
    },
    kind(
        "scheduled_task",
        "scheduled task",
        Scope::Workspace,
        "/api/v1/workspaces/{ws}/scheduled-tasks",
        &["name"],
        "list_scheduled_tasks",
    ),
    RefKind {
        keep: Some(&[
            "id",
            "name",
            "workspace_id",
            "repo_path",
            "status",
            "created_at",
            "updated_at",
        ]),
        ..kind(
            "goal_loop",
            "goal loop",
            Scope::Workspace,
            "/api/v1/workspaces/{ws}/goal-loops",
            &["name"],
            "list_goal_loops",
        )
    },
    RefKind {
        row: Some("room"),
        ..kind(
            "agent_room",
            "agent room",
            Scope::Workspace,
            "/api/v1/workspaces/{ws}/agent-rooms",
            &["name"],
            "list_agent_rooms",
        )
    },
    kind(
        "canvas_scene",
        "canvas scene",
        Scope::Workspace,
        "/api/v1/workspaces/{ws}/canvas/scenes",
        &["title"],
        "canvas_list_scenes",
    ),
    RefKind {
        keep: Some(&[
            "id",
            "workspace_id",
            "source_kind",
            "source_key",
            "title",
            "url",
            "issue_type",
            "stage",
            "tree_kind",
            "parent_id",
        ]),
        ..kind(
            "product_story",
            "product story",
            Scope::Library,
            "/api/v1/workspaces/{ws}/product/stories",
            &["source_key", "title"],
            "list_product_stories",
        )
    },
    kind(
        "vault",
        "vault",
        Scope::Library,
        "/api/v1/workspaces/{ws}/vault/vaults",
        &["name"],
        "vault_list",
    ),
    RefKind {
        rows: Some("requests"),
        ..kind(
            "api_request",
            "saved API request",
            Scope::Workspace,
            "/api/v1/workspaces/{ws}/api-client/overview?kind=requests",
            &["name"],
            "api_list",
        )
    },
    RefKind {
        rows: Some("automations"),
        ..kind(
            "api_automation",
            "API automation",
            Scope::Workspace,
            "/api/v1/workspaces/{ws}/api-client/overview?kind=automations",
            &["name"],
            "api_list",
        )
    },
    RefKind {
        rows: Some("environments"),
        ..kind(
            "api_environment",
            "API environment",
            Scope::Workspace,
            "/api/v1/workspaces/{ws}/api-client/overview?kind=environments",
            &["name"],
            "api_list",
        )
    },
    RefKind {
        sole_default: true,
        ..kind(
            "issue_account",
            "issue (Jira/Confluence) account",
            Scope::Global,
            "/api/v1/issue/accounts",
            &["label", "email", "base_url"],
            "list_issue_accounts",
        )
    },
    kind(
        "aws_account",
        "AWS account",
        Scope::Global,
        "/api/v1/aws/accounts",
        &["name", "profile"],
        "aws_list_accounts",
    ),
    RefKind {
        keep: Some(&[
            "id",
            "name",
            "source",
            "context_name",
            "default_namespace",
            "environment",
            "known_namespaces",
            "capabilities",
        ]),
        ..kind(
            "k8s_cluster",
            "Kubernetes cluster",
            Scope::Global,
            "/api/v1/k8s/clusters",
            &["name", "context_name"],
            "k8s_list_clusters",
        )
    },
    kind(
        "design_artifact",
        "design artifact",
        Scope::Global,
        "/api/v1/design/artifacts?limit=500",
        &["title"],
        "design_list",
    ),
];

/// Look a kind up by key.
pub(crate) fn kind_of(key: &str) -> Option<&'static RefKind> {
    KINDS.iter().find(|k| k.key == key)
}

/// One candidate object.
#[derive(Debug, Clone)]
pub(crate) struct Candidate {
    pub id: String,
    /// The object's own workspace (`None` for global rows).
    pub workspace_id: Option<String>,
    pub workspace_name: Option<String>,
    /// The first non-empty human field (for messages).
    pub label: String,
    pub row: Value,
}

impl Candidate {
    /// The agent-facing row: the (projected) object + its workspace.
    pub(crate) fn to_json(&self, kind: &RefKind, current: Option<&str>) -> Value {
        let mut out = match (kind.keep, &self.row) {
            (Some(keep), Value::Object(map)) => Value::Object(
                keep.iter()
                    .filter_map(|k| map.get(*k).map(|v| ((*k).to_string(), v.clone())))
                    .collect(),
            ),
            (_, row) => row.clone(),
        };
        if let Some(obj) = out.as_object_mut() {
            if kind.scope == Scope::Workspace {
                obj.insert("workspace_id".into(), json!(self.workspace_id));
                obj.insert("workspace_name".into(), json!(self.workspace_name));
                obj.insert(
                    "current".into(),
                    json!(current.is_some() && current == self.workspace_id.as_deref()),
                );
            }
        }
        out
    }

    /// One line for an error listing.
    fn describe(&self) -> String {
        match &self.workspace_name {
            Some(ws) => format!("{}  {}  (workspace: {ws})", self.id, self.label),
            None => format!("{}  {}", self.id, self.label),
        }
    }
}

/// How a reference matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MatchedBy {
    Id,
    /// The index into [`RefKind::names`] of the field that matched.
    Field(usize),
    /// An omitted reference settled by the sole candidate.
    Sole,
}

impl MatchedBy {
    pub(crate) fn describe(self, kind: &RefKind) -> String {
        match self {
            Self::Id => "id".into(),
            Self::Field(i) => kind.names.get(i).copied().unwrap_or("name").to_string(),
            Self::Sole => "only_candidate".into(),
        }
    }
}

/// Outcome of [`match_ref`].
#[derive(Debug)]
pub(crate) enum RefMatch<'a> {
    One(&'a Candidate, MatchedBy),
    Ambiguous(Vec<&'a Candidate>),
    None,
}

/// A row's id as a string (numeric ids — vaults — included).
fn id_of(row: &Value) -> Option<String> {
    match row.get("id")? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// The rows of one list response for `kind` (tolerates a bare array, a keyed
/// array, and nested row objects).
pub(crate) fn rows_of(kind: &RefKind, v: &Value) -> Vec<Value> {
    let arr = match kind.rows {
        Some(key) => v.get(key).and_then(Value::as_array),
        None => v.as_array(),
    };
    arr.map(|items| {
        items
            .iter()
            .map(|item| match kind.row {
                Some(key) => item.get(key).cloned().unwrap_or_else(|| item.clone()),
                None => item.clone(),
            })
            .collect()
    })
    .unwrap_or_default()
}

/// Build candidates from one list response. `listing_ws` is the workspace the
/// rows were listed through (`None` for a global route); a row's own
/// `workspace_id` wins, so a global connection profile listed through
/// workspace A is still reported as global-to-A (the workspace it was reached
/// through), never as a workspace it does not belong to.
pub(crate) fn candidates_from(
    kind: &RefKind,
    v: &Value,
    listing_ws: Option<(&str, &str)>,
) -> Vec<Candidate> {
    rows_of(kind, v)
        .into_iter()
        .filter_map(|row| {
            let id = id_of(&row)?;
            let label = kind
                .names
                .iter()
                .find_map(|f| {
                    row.get(*f)
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                })
                .unwrap_or("")
                .to_string();
            let (workspace_id, workspace_name) = match (kind.scope, listing_ws) {
                (Scope::Workspace, Some((ws, name))) => {
                    let own = row
                        .get("workspace_id")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty());
                    match own {
                        Some(o) if o != ws => (Some(o.to_string()), None),
                        _ => (Some(ws.to_string()), Some(name.to_string())),
                    }
                }
                _ if kind.key == "workspace" => (Some(id.clone()), Some(label.clone())),
                _ => (None, None),
            };
            Some(Candidate {
                id,
                workspace_id,
                workspace_name,
                label,
                row,
            })
        })
        .collect()
}

/// Settle one tier's hits: one → it; several → the one in `prefer_ws` when
/// exactly one lives there, else ambiguous.
fn settle<'a>(hits: Vec<&'a Candidate>, by: MatchedBy, prefer_ws: Option<&str>) -> RefMatch<'a> {
    match hits.len() {
        0 => RefMatch::None,
        1 => RefMatch::One(hits[0], by),
        _ => {
            let preferred: Vec<&Candidate> = hits
                .iter()
                .copied()
                .filter(|c| prefer_ws.is_some() && c.workspace_id.as_deref() == prefer_ws)
                .collect();
            if preferred.len() == 1 {
                RefMatch::One(preferred[0], by)
            } else {
                RefMatch::Ambiguous(hits)
            }
        }
    }
}

/// Match a reference against `cands`: exact id first, then each of the kind's
/// human fields in order (case-insensitive, exact). Pure — no I/O.
pub(crate) fn match_ref<'a>(
    kind: &RefKind,
    cands: &'a [Candidate],
    reference: &str,
    prefer_ws: Option<&str>,
) -> RefMatch<'a> {
    let reference = reference.trim();
    if reference.is_empty() {
        return RefMatch::None;
    }
    if let Some(c) = cands.iter().find(|c| c.id == reference) {
        return RefMatch::One(c, MatchedBy::Id);
    }
    for (i, field) in kind.names.iter().enumerate() {
        let hits: Vec<&Candidate> = cands
            .iter()
            .filter(|c| {
                c.row
                    .get(*field)
                    .and_then(Value::as_str)
                    .is_some_and(|v| v.trim().eq_ignore_ascii_case(reference))
            })
            .collect();
        if !hits.is_empty() {
            return settle(hits, MatchedBy::Field(i), prefer_ws);
        }
    }
    RefMatch::None
}

/// The "several matched" error.
pub(crate) fn ambiguous_error(
    kind: &RefKind,
    arg: &str,
    reference: &str,
    hits: &[&Candidate],
) -> Error {
    let list = hits
        .iter()
        .take(MAX_LISTED)
        .map(|c| format!("- {}", c.describe()))
        .collect::<Vec<_>>()
        .join("\n");
    Error::Conflict(format!(
        "{} reference '{reference}' matches {} {}s — pass one of these ids as {arg}:\n{list}",
        kind.noun,
        hits.len(),
        kind.noun
    ))
}

/// The "nothing matched" error: near misses first, else what IS available.
pub(crate) fn not_found_error(
    kind: &RefKind,
    arg: &str,
    reference: &str,
    cands: &[Candidate],
) -> Error {
    if cands.is_empty() {
        return Error::NotFound(format!(
            "no {} matches '{reference}': you have no {}s in any workspace you can read \
             (call {} to check).",
            kind.noun, kind.noun, kind.list_tool
        ));
    }
    let needle = reference.trim().to_lowercase();
    let near: Vec<&Candidate> = if needle.len() >= 3 {
        cands
            .iter()
            .filter(|c| {
                let l = c.label.to_lowercase();
                !l.is_empty() && (l.contains(&needle) || needle.contains(&l))
            })
            .collect()
    } else {
        vec![]
    };
    let (heading, pool): (&str, Vec<&Candidate>) = if near.is_empty() {
        ("Available", cands.iter().collect())
    } else {
        ("Closest matches", near)
    };
    let more = pool.len().saturating_sub(MAX_LISTED);
    let mut list = pool
        .iter()
        .take(MAX_LISTED)
        .map(|c| format!("- {}", c.describe()))
        .collect::<Vec<_>>()
        .join("\n");
    if more > 0 {
        list.push_str(&format!(
            "\n… and {more} more — call {} for the full list",
            kind.list_tool
        ));
    }
    let what = if reference.trim().is_empty() {
        format!("{arg} is required")
    } else {
        format!("no {} matches '{reference}'", kind.noun)
    };
    Error::NotFound(format!(
        "{what} (searched {} {}s across every workspace you can read). {heading}:\n{list}\n\
         Pass an id above as {arg} (a name works too).",
        cands.len(),
        kind.noun
    ))
}

/// True when `s` looks like an Otto-minted id (a 26-char Crockford ULID) or a
/// plain integer (vault ids) — a reference that needs no directory lookup when
/// nothing else (a token pin) requires one.
pub fn looks_like_id(s: &str) -> bool {
    let s = s.trim();
    (s.len() == 26
        && s.bytes().all(|b| {
            b.is_ascii_digit()
                || (b.is_ascii_uppercase() && !matches!(b, b'I' | b'L' | b'O' | b'U'))
        }))
        || (!s.is_empty() && s.len() <= 12 && s.bytes().all(|b| b.is_ascii_digit()))
}

// ===========================================================================
// Self-calls as the caller
// ===========================================================================

/// A short-lived token for the effective user, used to call list routes so
/// their native RBAC decides visibility. Always [`SelfCaller::close`] it.
pub(crate) struct SelfCaller {
    client: reqwest::Client,
    base: String,
    token: String,
    pool: sqlx::SqlitePool,
}

impl SelfCaller {
    pub(crate) async fn open(ctx: &ServerCtx, user: &User) -> Result<Self, Error> {
        let (token, _) = AuthRepo::new(ctx.pool.clone())
            .issue_api_token(&user.id, Some("mcp-otto-refs"))
            .await?;
        let client = reqwest::Client::builder()
            .timeout(SELF_CALL_TIMEOUT)
            .build()
            .map_err(|e| Error::Internal(format!("http client: {e}")))?;
        Ok(Self {
            client,
            base: ctx.base_url.trim_end_matches('/').to_string(),
            token,
            pool: ctx.pool.clone(),
        })
    }

    /// GET an `/api/v1/...` path; a non-2xx maps to the matching error kind.
    pub(crate) async fn get(&self, path: &str) -> Result<Value, Error> {
        let resp = self
            .client
            .get(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .header("X-Otto-Agent", "mcp-outward")
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("self-call: {e}")))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(status_error(status, &text));
        }
        Ok(crate::mcp_outward::parse_self_ok(&text))
    }

    /// Revoke the token.
    pub(crate) async fn close(self) {
        let _ = AuthRepo::new(self.pool).revoke(&self.token).await;
    }
}

/// Map a non-2xx self-call to an [`Error`] of the same class, with the
/// daemon's own message (its `"<class>: "` prefix stripped so it is not
/// doubled when re-displayed).
fn status_error(status: reqwest::StatusCode, text: &str) -> Error {
    let full = crate::mcp_outward::self_call_error(status, text);
    let msg = full
        .split_once(": ")
        .map(|(_, m)| m.to_string())
        .unwrap_or_else(|| full.clone());
    let strip = |m: &str, p: &str| m.strip_prefix(p).unwrap_or(m).to_string();
    match status.as_u16() {
        400 => Error::Invalid(strip(&msg, "invalid: ")),
        403 => Error::Forbidden(strip(&msg, "forbidden: ")),
        404 => Error::NotFound(strip(&msg, "not found: ")),
        409 => Error::Conflict(strip(&msg, "conflict: ")),
        _ => Error::Upstream(full),
    }
}

/// The workspaces the caller can read, as `(id, name)`, narrowed to `pin`.
/// Archived workspaces are skipped.
pub(crate) async fn readable_workspaces(
    caller: &SelfCaller,
    pin: Option<&str>,
) -> Result<Vec<(String, String)>, Error> {
    let v = caller.get("/api/v1/workspaces").await?;
    Ok(v.as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter(|w| !w.get("archived").and_then(Value::as_bool).unwrap_or(false))
        .filter_map(|w| {
            let id = w.get("id").and_then(Value::as_str)?.to_string();
            let name = w
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(&id)
                .to_string();
            Some((id, name))
        })
        .filter(|(id, _)| pin.is_none_or(|p| p == id.as_str()))
        .collect())
}

/// The caller's candidates of `kind`, current workspace first, deduplicated
/// by id (a global profile appears once). `ws_filter` narrows `Workspace`
/// kinds to one workspace. A workspace whose list call fails (no feature
/// grant, no role there) is skipped; if EVERY call failed the first error is
/// returned so "forbidden" is never reported as "none exist".
pub(crate) async fn load_candidates(
    caller: &SelfCaller,
    kind: &RefKind,
    pin: Option<&str>,
    ws_filter: Option<&str>,
    prefer: Option<&str>,
) -> Result<Vec<Candidate>, Error> {
    let mut out: Vec<Candidate> = Vec::new();
    match kind.scope {
        Scope::Global => {
            let v = caller.get(kind.list).await?;
            out = candidates_from(kind, &v, None);
            if kind.key == "workspace" {
                out.retain(|c| {
                    !c.row
                        .get("archived")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                        && pin.is_none_or(|p| p == c.id)
                });
            }
        }
        Scope::Workspace | Scope::Library => {
            let mut wss = readable_workspaces(caller, pin).await?;
            if let Some(f) = ws_filter {
                wss.retain(|(id, _)| id == f);
                if wss.is_empty() {
                    return Err(Error::Forbidden(format!(
                        "workspace '{f}' is not one you can read{}",
                        pin.map(|p| format!(" (this token is scoped to workspace '{p}')"))
                            .unwrap_or_default()
                    )));
                }
            }
            // Current workspace first, then by name.
            wss.sort_by(|a, b| {
                let ac = prefer == Some(a.0.as_str());
                let bc = prefer == Some(b.0.as_str());
                bc.cmp(&ac)
                    .then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))
            });
            let mut first_err: Option<Error> = None;
            let mut any_ok = false;
            for (ws, name) in &wss {
                let path = kind.list.replace("{ws}", &crate::mcp_outward::seg(ws));
                match caller.get(&path).await {
                    Ok(v) => {
                        any_ok = true;
                        let listing = (kind.scope == Scope::Workspace)
                            .then_some((ws.as_str(), name.as_str()));
                        out.extend(candidates_from(kind, &v, listing));
                        if kind.scope == Scope::Library {
                            break;
                        }
                    }
                    Err(e) => {
                        if first_err.is_none() {
                            first_err = Some(e);
                        }
                    }
                }
            }
            if !any_ok {
                if let Some(e) = first_err {
                    return Err(e);
                }
            }
        }
    }
    let mut seen: HashSet<String> = HashSet::new();
    out.retain(|c| seen.insert(c.id.clone()));
    Ok(out)
}

/// The token's workspace pin, if any.
pub(crate) fn pin_of(auth: &AuthContext) -> Option<&str> {
    auth.mcp_scope
        .as_ref()
        .and_then(|s| s.workspace_id.as_deref())
        .filter(|s| !s.is_empty())
}

/// Resolve `reference` (or, when omitted, the sole candidate for kinds that
/// allow it) with an already-open caller. `arg` names the argument for
/// messages. See the module doc for the error contract.
pub(crate) async fn resolve_with(
    caller: &SelfCaller,
    auth: &AuthContext,
    kind: &RefKind,
    arg: &str,
    reference: Option<&str>,
    ws_filter: Option<&str>,
    prefer: Option<&str>,
) -> Result<(Candidate, MatchedBy), Error> {
    let pin = pin_of(auth);
    if let (Some(p), Some(f)) = (pin, ws_filter) {
        if p != f {
            return Err(Error::Forbidden(format!(
                "this token is scoped to workspace '{p}'"
            )));
        }
    }
    let cands = load_candidates(caller, kind, pin, ws_filter, prefer).await?;
    let reference = reference.map(str::trim).filter(|s| !s.is_empty());
    match reference {
        Some(r) => match match_ref(kind, &cands, r, prefer.or(ws_filter)) {
            RefMatch::One(c, by) => Ok((c.clone(), by)),
            RefMatch::Ambiguous(hits) => Err(ambiguous_error(kind, arg, r, &hits)),
            RefMatch::None => Err(not_found_error(kind, arg, r, &cands)),
        },
        None if kind.sole_default && cands.len() == 1 => Ok((cands[0].clone(), MatchedBy::Sole)),
        None => Err(not_found_error(kind, arg, "", &cands)),
    }
}

/// [`resolve_with`] opening (and closing) its own caller.
pub(crate) async fn resolve(
    ctx: &ServerCtx,
    auth: &AuthContext,
    kind: &RefKind,
    arg: &str,
    reference: Option<&str>,
    ws_filter: Option<&str>,
    prefer: Option<&str>,
) -> Result<(Candidate, MatchedBy), Error> {
    let caller = SelfCaller::open(ctx, &auth.effective_user).await?;
    let r = resolve_with(&caller, auth, kind, arg, reference, ws_filter, prefer).await;
    caller.close().await;
    r
}

/// The caller's session workspace (Otto-minted session credentials only).
pub(crate) async fn caller_session_ws(ctx: &ServerCtx, auth: &AuthContext) -> Option<String> {
    let sid = auth
        .managed_session_id
        .as_ref()
        .or(auth.mcp_session_id.as_ref())?;
    ctx.manager.get(sid).await.ok().map(|s| s.workspace_id)
}

/// The agent-facing directory of `kind` for `auth` (the token's pin applied):
/// `{kind, items, current_workspace_id, workspace_count}`. Shared by
/// `GET /refs/directory` and the governed `otto.list_*` tools, so both shapes
/// are identical.
pub(crate) async fn directory_json(
    ctx: &ServerCtx,
    auth: &AuthContext,
    kind: &RefKind,
    ws_filter: Option<&str>,
    current: Option<&str>,
) -> Result<Value, Error> {
    let caller = SelfCaller::open(ctx, &auth.effective_user).await?;
    let cands = load_candidates(&caller, kind, pin_of(auth), ws_filter, current).await;
    caller.close().await;
    let cands = cands?;
    let workspaces: HashSet<&str> = cands
        .iter()
        .filter_map(|c| c.workspace_id.as_deref())
        .collect();
    Ok(json!({
        "kind": kind.key,
        "items": cands.iter().map(|c| c.to_json(kind, current)).collect::<Vec<_>>(),
        "current_workspace_id": current,
        "workspace_count": workspaces.len(),
    }))
}

/// Refuse the discovery routes to share-link and restricted MCP tokens: they
/// mint a self-call token for the effective user, which a guest link (pinned
/// to one session) or an `mcp_only` token must never be able to widen into.
fn reject_restricted(auth: &AuthContext) -> Result<(), Error> {
    if auth.is_scoped() || auth.mcp_only {
        return Err(Error::Forbidden(
            "discovery is not available to share-link or MCP-restricted tokens".into(),
        ));
    }
    Ok(())
}

fn unknown_kind(k: &str) -> Error {
    Error::Invalid(format!(
        "unknown kind '{k}' — one of: {}",
        KINDS.iter().map(|k| k.key).collect::<Vec<_>>().join(", ")
    ))
}

// ===========================================================================
// HTTP: GET /refs/directory, GET /refs/resolve
// ===========================================================================

#[derive(Deserialize)]
pub struct DirectoryQuery {
    pub kind: String,
    /// Optional: only this workspace.
    pub workspace_id: Option<String>,
    /// Optional: which workspace is "current" (listed first). Defaults to the
    /// calling session's workspace.
    pub prefer_workspace_id: Option<String>,
}

/// `GET /refs/directory?kind=` — every object of `kind` across the workspaces
/// the caller can read (`Workspace` kinds annotated with `workspace_id`,
/// `workspace_name`, `current`), current workspace first.
pub async fn refs_directory(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Query(q): Query<DirectoryQuery>,
) -> ApiResult<Json<Value>> {
    reject_restricted(&auth).map_err(ApiError)?;
    let kind = kind_of(&q.kind).ok_or_else(|| ApiError(unknown_kind(&q.kind)))?;
    let current = match q.prefer_workspace_id.filter(|s| !s.is_empty()) {
        Some(ws) => Some(ws),
        None => caller_session_ws(&ctx, &auth).await,
    };
    let filter = q.workspace_id.as_deref().filter(|s| !s.is_empty());
    directory_json(&ctx, &auth, kind, filter, current.as_deref())
        .await
        .map(Json)
        .map_err(ApiError)
}

#[derive(Deserialize)]
pub struct ResolveQuery {
    pub kind: String,
    /// The reference: id or a human field (name / title / key / label).
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    pub workspace_id: Option<String>,
    pub prefer_workspace_id: Option<String>,
    /// Argument name used in messages (default `<kind>_id`).
    pub arg: Option<String>,
}

/// `GET /refs/resolve?kind=&ref=` — resolve one reference. 404 lists near
/// misses / what is available, 409 lists the ambiguous candidates.
pub async fn refs_resolve(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Query(q): Query<ResolveQuery>,
) -> ApiResult<Json<Value>> {
    reject_restricted(&auth).map_err(ApiError)?;
    let kind = kind_of(&q.kind).ok_or_else(|| ApiError(unknown_kind(&q.kind)))?;
    let current = match q.prefer_workspace_id.filter(|s| !s.is_empty()) {
        Some(ws) => Some(ws),
        None => caller_session_ws(&ctx, &auth).await,
    };
    let arg = q.arg.unwrap_or_else(|| format!("{}_id", kind.key));
    let (c, by) = resolve(
        &ctx,
        &auth,
        kind,
        &arg,
        q.reference.as_deref(),
        q.workspace_id.as_deref().filter(|s| !s.is_empty()),
        current.as_deref(),
    )
    .await
    .map_err(ApiError)?;
    Ok(Json(json!({
        "kind": kind.key,
        "id": c.id,
        "label": c.label,
        "workspace_id": c.workspace_id,
        "workspace_name": c.workspace_name,
        "matched_by": by.describe(kind),
        "item": c.to_json(kind, current.as_deref()),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wf() -> &'static RefKind {
        kind_of("workflow").unwrap()
    }

    fn cands() -> Vec<Candidate> {
        let a = json!([
            {"id":"W1","workspace_id":"ws-a","name":"PR Reviewer — Go","graph":{"big":true}},
            {"id":"W2","workspace_id":"ws-a","name":"Nightly"}
        ]);
        let b = json!([
            {"id":"W3","workspace_id":"ws-b","name":"nightly"},
            {"id":"W4","workspace_id":"ws-b","name":"Release notes"}
        ]);
        let mut out = candidates_from(wf(), &a, Some(("ws-a", "Alpha")));
        out.extend(candidates_from(wf(), &b, Some(("ws-b", "Beta"))));
        out
    }

    #[test]
    fn every_kind_is_well_formed() {
        let mut keys = HashSet::new();
        for k in KINDS {
            assert!(keys.insert(k.key), "duplicate kind {}", k.key);
            assert!(!k.names.is_empty(), "{} has no human field", k.key);
            assert!(k.list.starts_with("/api/v1/"), "{}", k.key);
            let templated = k.list.contains("{ws}");
            assert_eq!(
                templated,
                k.scope != Scope::Global,
                "{}: scope vs {{ws}}",
                k.key
            );
            if let Some(keep) = k.keep {
                assert!(keep.contains(&"id"), "{} keep must include id", k.key);
            }
        }
        assert!(kind_of("issue_account").unwrap().sole_default);
        assert!(kind_of("nope").is_none());
    }

    #[test]
    fn matches_id_then_name_case_insensitively_across_workspaces() {
        let cs = cands();
        match match_ref(wf(), &cs, "W4", None) {
            RefMatch::One(c, MatchedBy::Id) => assert_eq!(c.id, "W4"),
            other => panic!("{other:?}"),
        }
        match match_ref(wf(), &cs, "release NOTES", Some("ws-a")) {
            RefMatch::One(c, MatchedBy::Field(0)) => {
                assert_eq!(c.id, "W4");
                assert_eq!(c.workspace_name.as_deref(), Some("Beta"));
            }
            other => panic!("{other:?}"),
        }
        // Same name in two workspaces: ambiguous unless the caller's settles it.
        assert!(matches!(
            match_ref(wf(), &cs, "nightly", None),
            RefMatch::Ambiguous(_)
        ));
        match match_ref(wf(), &cs, "nightly", Some("ws-b")) {
            RefMatch::One(c, _) => assert_eq!(c.id, "W3"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(match_ref(wf(), &cs, "nope", None), RefMatch::None));
    }

    #[test]
    fn errors_list_candidates_so_the_agent_can_retry_at_once() {
        let cs = cands();
        let hits: Vec<&Candidate> = cs
            .iter()
            .filter(|c| c.label.eq_ignore_ascii_case("nightly"))
            .collect();
        let e = ambiguous_error(wf(), "workflow_id", "nightly", &hits).to_string();
        assert!(
            e.contains("W2") && e.contains("W3") && e.contains("Beta"),
            "{e}"
        );
        let e = not_found_error(wf(), "workflow_id", "release", &cs).to_string();
        assert!(
            e.contains("Closest matches") && e.contains("W4") && !e.contains("W1"),
            "{e}"
        );
        let e = not_found_error(wf(), "workflow_id", "zz", &cs).to_string();
        assert!(
            e.contains("Available") && e.contains("W1") && e.contains("W4"),
            "{e}"
        );
        let e = not_found_error(wf(), "workflow_id", "", &cs).to_string();
        assert!(e.contains("workflow_id is required"), "{e}");
        let e = not_found_error(wf(), "workflow_id", "x", &[]).to_string();
        assert!(e.contains("list_workflows"), "{e}");
    }

    #[test]
    fn directory_rows_are_projected_and_annotated() {
        let cs = cands();
        let row = cs[0].to_json(wf(), Some("ws-a"));
        assert!(row.get("graph").is_none(), "heavy graph dropped: {row}");
        assert_eq!(row["workspace_name"], "Alpha");
        assert_eq!(row["current"], true);
        assert_eq!(cs[3].to_json(wf(), Some("ws-a"))["current"], false);
    }

    #[test]
    fn nested_and_keyed_rows_are_read() {
        let rooms = kind_of("agent_room").unwrap();
        let v = json!([{"room":{"id":"R1","workspace_id":"ws","name":"Standup"},"members":[]}]);
        let cs = candidates_from(rooms, &v, Some(("ws", "W")));
        assert_eq!((cs[0].id.as_str(), cs[0].label.as_str()), ("R1", "Standup"));
        let reqs = kind_of("api_request").unwrap();
        let v = json!({"requests":[{"id":"Q1","name":"Login"}],"collections":[]});
        assert_eq!(candidates_from(reqs, &v, Some(("ws", "W")))[0].id, "Q1");
        // Numeric ids (vaults) and a Jira key as the human field (stories).
        let vaults = kind_of("vault").unwrap();
        let v = json!([{"id":1,"name":"Platform Docs"}]);
        assert_eq!(candidates_from(vaults, &v, Some(("ws", "W")))[0].id, "1");
        let stories = kind_of("product_story").unwrap();
        let v = json!([{"id":"S1","source_key":"GS-123","title":"Promo"}]);
        let cs = candidates_from(stories, &v, None);
        assert!(matches!(
            match_ref(stories, &cs, "gs-123", None),
            RefMatch::One(_, MatchedBy::Field(0))
        ));
    }

    #[test]
    fn looks_like_id_recognises_ulids_and_integers_only() {
        assert!(looks_like_id("01KZTKNK3Z8N6VD9Q0MTDQSJ3V"));
        assert!(looks_like_id("42"));
        assert!(!looks_like_id("bo_tools"));
        assert!(!looks_like_id("PR Reviewer"));
        assert!(
            !looks_like_id("01kztknk3z8n6vd9q0mtdqsj3v"),
            "lowercase is a name"
        );
        assert!(!looks_like_id(""));
    }

    #[test]
    fn status_errors_keep_their_class_without_doubling_the_prefix() {
        let e = status_error(
            reqwest::StatusCode::NOT_FOUND,
            r#"{"code":"not_found","message":"not found: workflow"}"#,
        );
        assert_eq!(e.to_string(), "not found: workflow");
        let e = status_error(
            reqwest::StatusCode::FORBIDDEN,
            r#"{"message":"forbidden: requires x"}"#,
        );
        assert!(
            matches!(e, Error::Forbidden(ref m) if m == "requires x"),
            "{e}"
        );
    }
}
