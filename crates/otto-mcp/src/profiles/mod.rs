//! **MCP profiles** — named variants of the Otto MCP connection, each exposing
//! only a subset of the tools ("Database", "Issues", "Git & PRs", …).
//!
//! A profile is a set of tool [`GROUPS`] plus per-tool includes / excludes,
//! or `all_tools` (the locked built-in **Full**, the initial default). An
//! agent session runs under the UNION of the profiles stored on it
//! (`meta.mcp_profiles`; none ⇒ the default); an external connection names
//! one (`ottod mcp-server --profile <id>`, `POST /mcp/http?profile=<id>`).
//!
//! Profiles only **narrow** what a caller sees: [`ToolFilter`] is a thin hook
//! at the two points every surface has — where `tools/list` is served and
//! where `tools/call` is dispatched. It never grants anything; the per-token
//! scope, the control-plane enable list, RBAC, policy and approvals all run
//! exactly as before on whatever the filter lets through. No filter (the
//! Full profile, a legacy connection) ⇒ behaviour is unchanged.
//!
//! Group membership is keyed by tool NAME (the [`canonical`] form, so the
//! outward `otto.create_pr`, the session stdio `otto_create_pr` and the bare
//! `create_pr` are one tool), and every tool of both catalogs must belong to
//! exactly one group — `tests::every_catalog_tool_has_exactly_one_group`
//! (outward) and ottod's native-catalog twin fail on a tool added without one.

pub mod routes;

use async_trait::async_trait;
use otto_core::auth::AuthContext;
use otto_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::outward::OutwardTools;

/// One tool group: a stable id, a label, and its members — exact canonical
/// names and/or name prefixes (a family such as `aws_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolGroup {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub prefixes: &'static [&'static str],
    pub exact: &'static [&'static str],
}

impl ToolGroup {
    /// Whether a canonical tool name belongs to this group.
    pub fn contains(&self, key: &str) -> bool {
        self.exact.contains(&key) || self.prefixes.iter().any(|p| key.starts_with(p))
    }
}

/// Every tool group, in display order. Derived from the outward catalog's
/// categories and the session server's native families; a tool belongs to
/// exactly one (enforced by the tests here and in ottod).
pub const GROUPS: &[ToolGroup] = &[
    ToolGroup {
        id: "database",
        label: "Database",
        description: "Database connections: schema, objects and read-only queries.",
        prefixes: &["db_"],
        exact: &["list_connections", "query_db_readonly"],
    },
    ToolGroup {
        id: "issues",
        label: "Issues (Jira)",
        description: "Search, read, comment on and transition Jira issues.",
        prefixes: &[],
        exact: &[
            "search_issues",
            "get_issue",
            "comment_issue",
            "transition_issue",
            "list_issue_transitions",
            "list_issue_accounts",
        ],
    },
    ToolGroup {
        id: "confluence",
        label: "Confluence",
        description: "Search, read, create, update and comment on Confluence pages.",
        prefixes: &[],
        exact: &[
            "search_confluence",
            "get_confluence_page",
            "create_confluence_page",
            "update_confluence_page",
            "comment_confluence_page",
            "list_confluence_page_comments",
        ],
    },
    ToolGroup {
        id: "git",
        label: "Git & PRs",
        description: "Repositories, status, pull requests, PR drafts and merges.",
        prefixes: &[],
        exact: &[
            "list_repos",
            "git_status",
            "list_prs",
            "get_pr",
            "get_pr_diff",
            "get_pr_checks",
            "comment_pr",
            "create_pr",
            "merge_pr",
            "open_pr_draft",
        ],
    },
    ToolGroup {
        id: "code_review",
        label: "Code review",
        description: "Multi-agent PR reviews and their findings.",
        prefixes: &[],
        exact: &[
            "start_pr_review",
            "list_pr_reviews",
            "git_pr_review",
            "list_findings",
            "get_finding",
        ],
    },
    ToolGroup {
        id: "sessions",
        label: "Sessions & agents",
        description: "Open, message and wait on agent sessions; agent rooms.",
        prefixes: &[],
        exact: &[
            "list_sessions",
            "get_session",
            "open_session",
            "send_message",
            "broadcast_message",
            "wait_session",
            "list_agent_rooms",
            "room_post",
            "room_read",
        ],
    },
    ToolGroup {
        id: "context",
        label: "Workspace & context",
        description: "Workspaces, code search, context packets, memory, usage, skills and \
            asking a person for approval.",
        prefixes: &[],
        exact: &[
            "list_workspaces",
            "search_codebase",
            "get_context_packet",
            "get_proof_pack",
            "search_memory",
            "usage_summary",
            "get_usage_summary",
            "list_bundled_skills",
            "ask_human_approval",
        ],
    },
    ToolGroup {
        id: "aws",
        label: "AWS",
        description: "AWS console: S3, SQS, EC2, EKS, Athena and CloudWatch Logs.",
        prefixes: &["aws_"],
        exact: &[],
    },
    ToolGroup {
        id: "kubernetes",
        label: "Kubernetes",
        description: "Kubernetes console: resources, logs, health, pod HTTP and actions.",
        prefixes: &["k8s_"],
        exact: &[],
    },
    ToolGroup {
        id: "browser",
        label: "Browser",
        description: "The in-app browser: navigate, read, query and summarize pages.",
        prefixes: &["browser_"],
        exact: &[],
    },
    ToolGroup {
        id: "vault",
        label: "Vault & docs",
        description: "Vault documentation (read, search, write) and the Workbench scratch file.",
        prefixes: &["vault_", "workbench_"],
        exact: &[],
    },
    ToolGroup {
        id: "automation",
        label: "Workflows & automation",
        description: "Workflows, scheduled tasks, goal loops and self-improvement runs.",
        prefixes: &[],
        exact: &[
            "list_workflows",
            "get_workflow",
            "run_workflow",
            "list_workflow_runs",
            "get_workflow_run",
            "cancel_workflow_run",
            "list_scheduled_tasks",
            "get_scheduled_task",
            "create_scheduled_task",
            "update_scheduled_task",
            "delete_scheduled_task",
            "run_scheduled_task",
            "set_scheduled_task_enabled",
            "list_scheduled_task_runs",
            "list_goal_loops",
            "run_goal_loop",
            "run_self_improvement",
            "get_self_improvement_config",
            "list_improvement_runs",
            "get_improvement_run",
            "list_improvement_edits",
            "approve_improvement_edit",
            "reject_improvement_edit",
            "rollback_improvement_edit",
        ],
    },
    ToolGroup {
        id: "swarm",
        label: "Swarm & work items",
        description: "Agent swarms, their boards, runs, projects and work items.",
        prefixes: &["swarm_"],
        exact: &[
            "list_swarms",
            "get_swarm",
            "get_swarm_board",
            "post_swarm_board",
            "list_swarm_runs",
            "list_swarm_projects",
            "list_swarm_tasks",
            "create_work_item",
        ],
    },
    ToolGroup {
        id: "design",
        label: "Design & canvas",
        description: "Design Hall artifacts and links, and canvas scenes.",
        prefixes: &["design_", "canvas_"],
        exact: &["list_design_projects"],
    },
    ToolGroup {
        id: "brokers",
        label: "Message brokers",
        description: "Kafka clusters, topics, consumer groups, consume and produce.",
        prefixes: &[],
        exact: &[
            "list_broker_clusters",
            "list_broker_topics",
            "get_broker_topic",
            "list_consumer_groups",
            "consume_broker_messages",
            "produce_broker_message",
        ],
    },
    ToolGroup {
        id: "api",
        label: "API client",
        description: "Saved API requests, history and automations.",
        prefixes: &["api_"],
        exact: &[],
    },
    ToolGroup {
        id: "product",
        label: "Product stories",
        description: "Product story analysis.",
        prefixes: &[],
        exact: &["list_product_stories", "get_product_story", "product_story"],
    },
    ToolGroup {
        id: "assistant",
        label: "Assistant",
        description: "The personal assistant's memory, tasks and reminders.",
        prefixes: &["assistant_"],
        exact: &[],
    },
    ToolGroup {
        id: "channels",
        label: "Channels",
        description: "Slack / Telegram integrations.",
        prefixes: &[],
        exact: &["list_integrations", "test_integration"],
    },
    ToolGroup {
        id: "ui",
        label: "App UI control",
        description: "Drive the Otto app's UI (needs the per-session Allow UI control grant).",
        prefixes: &["ui_"],
        exact: &[],
    },
    ToolGroup {
        id: "external",
        label: "External MCP servers",
        description: "Tools of the governed external MCP servers (mcp__server__tool).",
        prefixes: &["mcp__"],
        exact: &[],
    },
];

/// A built-in preset, seeded by migration 0191 under these fixed ids.
pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub all_tools: bool,
    pub groups: &'static [&'static str],
}

/// The built-in presets (kept in step with migration 0191 by a test).
pub const PRESETS: &[Preset] = &[
    Preset {
        id: "full",
        name: "Full",
        all_tools: true,
        groups: &[],
    },
    Preset {
        id: "database",
        name: "Database",
        all_tools: false,
        groups: &["database"],
    },
    Preset {
        id: "issues",
        name: "Issues",
        all_tools: false,
        groups: &["issues", "confluence"],
    },
    Preset {
        id: "git",
        name: "Git & PRs",
        all_tools: false,
        groups: &["git", "code_review"],
    },
    Preset {
        id: "cloud",
        name: "Cloud",
        all_tools: false,
        groups: &["aws", "kubernetes"],
    },
];

/// The one name every surface agrees on: `otto.create_pr` (outward),
/// `otto_create_pr` (session stdio) and `create_pr` (bare) → `create_pr`;
/// native names without the prefix (`k8s_logs`) and gateway names
/// (`mcp__srv__tool`) are already canonical.
pub fn canonical(name: &str) -> &str {
    let n = name.strip_prefix("otto.").unwrap_or(name);
    n.strip_prefix("otto_").unwrap_or(n)
}

/// The group a tool belongs to (the first match; the tests prove there is
/// exactly one for every catalog tool).
pub fn group_of(name: &str) -> Option<&'static ToolGroup> {
    let key = canonical(name);
    GROUPS.iter().find(|g| g.contains(key))
}

pub fn group(id: &str) -> Option<&'static ToolGroup> {
    GROUPS.iter().find(|g| g.id == id)
}

/// One profile's tool set — what the filter needs (persisted as
/// `otto_state::McpProfile`, served to the stdio bridges by
/// `GET /mcp/otto-server/enabled?profiles=`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileDef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub all_tools: bool,
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(default)]
    pub include_tools: Vec<String>,
    #[serde(default)]
    pub exclude_tools: Vec<String>,
}

impl ProfileDef {
    pub fn allows(&self, tool: &str) -> bool {
        let key = canonical(tool);
        let has = |list: &[String]| list.iter().any(|t| canonical(t) == key);
        if has(&self.exclude_tools) {
            return false;
        }
        if self.all_tools || has(&self.include_tools) {
            return true;
        }
        group_of(key).is_some_and(|g| self.groups.iter().any(|id| id == g.id))
    }

    /// Every tool, nothing excluded.
    pub fn is_full(&self) -> bool {
        self.all_tools && self.exclude_tools.is_empty()
    }
}

impl From<&otto_state::McpProfile> for ProfileDef {
    fn from(p: &otto_state::McpProfile) -> Self {
        Self {
            id: p.id.clone(),
            name: p.name.clone(),
            all_tools: p.all_tools,
            groups: p.groups.clone(),
            include_tools: p.include_tools.clone(),
            exclude_tools: p.exclude_tools.clone(),
        }
    }
}

/// The resolved tool set of a session or connection: the UNION of its
/// profiles. Built daemon-side ([`resolve`]) and applied wherever tools are
/// listed or called.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolFilter {
    /// The resolved profile ids (unknown ids dropped; none ⇒ the default).
    pub ids: Vec<String>,
    /// Display label, e.g. `Database + Issues`.
    pub label: String,
    pub profiles: Vec<ProfileDef>,
}

impl ToolFilter {
    pub fn new(profiles: Vec<ProfileDef>) -> Self {
        Self {
            ids: profiles.iter().map(|p| p.id.clone()).collect(),
            label: profiles
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>()
                .join(" + "),
            profiles,
        }
    }

    /// Lets every tool through (a Full profile is in the set). No profiles at
    /// all also counts: an empty filter must never hide anything.
    pub fn is_unrestricted(&self) -> bool {
        self.profiles.is_empty() || self.profiles.iter().any(ProfileDef::is_full)
    }

    pub fn allows(&self, tool: &str) -> bool {
        self.is_unrestricted() || self.profiles.iter().any(|p| p.allows(tool))
    }

    /// The error a call outside the set gets — names the profile and how to
    /// widen it.
    pub fn denial(&self, tool: &str) -> String {
        format!(
            "tool '{tool}' is not in the MCP profile \"{}\" this connection runs under — \
             profiles narrow the Otto tool list. Pick a profile that includes it (Otto → \
             session ⋯ → MCP tools, then restart the agent; or attach a connection with \
             another profile).",
            self.label
        )
    }

    /// Keep only the allowed entries of a `tools/list` array (by `name`).
    pub fn retain_listed(&self, tools: &mut Vec<Value>) {
        if self.is_unrestricted() {
            return;
        }
        tools.retain(|t| t["name"].as_str().is_some_and(|n| self.allows(n)));
    }
}

/// Parse a comma-separated profile list (`database,issues`) as sent by the
/// bridges / URLs; blanks dropped, capped.
pub fn parse_ids(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .take(MAX_PROFILES_PER_SESSION)
        .map(str::to_string)
        .collect()
}

/// How many profiles one session / connection may combine.
pub const MAX_PROFILES_PER_SESSION: usize = 16;

/// Resolve profile ids (`"default"`, unknown ids skipped, none ⇒ the
/// default) into a [`ToolFilter`].
pub async fn resolve(pool: &otto_state::DbPool, ids: &[String]) -> Result<ToolFilter> {
    let rows = otto_state::McpProfileRepo::new(pool.clone())
        .resolve(ids)
        .await?;
    Ok(ToolFilter::new(rows.iter().map(ProfileDef::from).collect()))
}

/// [`resolve`] for an optional raw `?profile(s)=` value: `None` (no profile
/// asked for — a legacy connection) ⇒ no filter at all.
pub async fn resolve_param(
    pool: &otto_state::DbPool,
    raw: Option<&str>,
) -> Result<Option<ToolFilter>> {
    match raw {
        Some(raw) => Ok(Some(resolve(pool, &parse_ids(raw)).await?)),
        None => Ok(None),
    }
}

/// An [`OutwardTools`] narrowed by a profile: `tools/list` keeps the allowed
/// entries, `tools/call` of anything else is refused naming the profile —
/// before the inner (governed) call runs. `None` delegates untouched.
pub struct ProfiledTools<'a, T: ?Sized> {
    pub inner: &'a T,
    pub filter: Option<ToolFilter>,
}

#[async_trait]
impl<T: OutwardTools + ?Sized> OutwardTools for ProfiledTools<'_, T> {
    async fn list(&self, auth: &AuthContext) -> Vec<Value> {
        let mut tools = self.inner.list(auth).await;
        if let Some(f) = &self.filter {
            f.retain_listed(&mut tools);
        }
        tools
    }

    async fn call(&self, auth: &AuthContext, tool: &str, args: &Value) -> Result<Value> {
        if let Some(f) = self.filter.as_ref().filter(|f| !f.allows(tool)) {
            return Err(Error::Forbidden(f.denial(tool)));
        }
        self.inner.call(auth, tool, args).await
    }
}

/// Validate + normalize a profile's editable parts: known group ids, tool
/// names that belong to a group (stored canonical), deduplicated.
pub fn normalize_fields(
    groups: &[String],
    include: &[String],
    exclude: &[String],
) -> Result<(Vec<String>, Vec<String>, Vec<String>)> {
    let mut gs: Vec<String> = Vec::new();
    for g in groups {
        let g = g.trim();
        if group(g).is_none() {
            return Err(Error::Invalid(format!("unknown tool group '{g}'")));
        }
        if !gs.iter().any(|x| x == g) {
            gs.push(g.to_string());
        }
    }
    let tools = |list: &[String]| -> Result<Vec<String>> {
        let mut out: Vec<String> = Vec::new();
        for t in list {
            let key = canonical(t.trim());
            if key.is_empty() || group_of(key).is_none() {
                return Err(Error::Invalid(format!("unknown Otto tool '{}'", t.trim())));
            }
            if !out.iter().any(|x| x == key) {
                out.push(key.to_string());
            }
        }
        Ok(out)
    };
    Ok((gs, tools(include)?, tools(exclude)?))
}

#[cfg(test)]
mod tests;
