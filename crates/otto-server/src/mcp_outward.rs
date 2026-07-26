//! "Otto as an MCP server" — the OUTWARD surface. External agents (Claude Code,
//! Copilot, …) connect to `ottod mcp-server` over stdio with a **restricted**
//! `kind='mcp'` token and call the `otto.*` tools. Every call funnels
//! through `POST /mcp/otto-tools/invoke` (the only route that token may reach —
//! see feature_guard, design §14 F1), which governs (enabled? allowlisted?
//! dangerous→approval?), audits (`mcp_call_log`, direction='inbound'), then
//! executes the capability **as the token's user** by self-calling the real
//! endpoint with a short-lived ephemeral token — so each tool reuses its
//! endpoint's native RBAC (no privilege escalation). It also hosts the live-agent
//! **gateway** (`/mcp/gateway/*`).

use std::time::Duration;

use axum::extract::{Query, State};
use axum::Json;
use otto_core::api::CreateMcpTokenReq;
use otto_core::auth::{AuthContext, McpScope};
use otto_core::domain::WorkspaceRole;
use otto_core::{Error, Id};
use otto_mcp::{canonical_hash, InvokeCtx};
use otto_rbac::AuthRepo;
use otto_state::{NewApproval, NewCallLog, SettingsRepo};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{CurrentAuthContext, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

const DEFAULT_ENABLED: &[&str] = &[
    "get_context_packet",
    "get_proof_pack",
    "ask_human_approval",
    // Scheduled-tasks reads are safe to expose by default so an agent can inspect
    // existing jobs; the write tools below stay off until an admin enables them.
    "list_scheduled_tasks",
    "list_scheduled_task_runs",
    // ---- Feature reads (metadata/list/get) — safe to expose by default once the
    // outward server itself is turned on. Content-heavy reads (consume/search)
    // stay off by default; see the two opt-in reads excluded from this list.
    // Workflows
    "list_workflows",
    "get_workflow",
    "list_workflow_runs",
    "get_workflow_run",
    // Message brokers
    "list_broker_clusters",
    "list_broker_topics",
    "get_broker_topic",
    "list_consumer_groups",
    // Connections / git
    "list_connections",
    "list_repos",
    "git_status",
    "list_prs",
    "get_pr",
    // Issues (Jira / Confluence)
    "search_issues",
    "get_issue",
    "search_confluence",
    // Swarm
    "list_swarms",
    "get_swarm",
    "list_swarm_runs",
    "get_swarm_board",
    // Vault v3 — the docs home (file-backed markdown vaults, OKF)
    "vault_list",
    "vault_dir",
    "vault_read",
    "vault_search",
    "vault_backlinks",
    "vault_tags",
    "vault_graph",
    "vault_okf_validate",
    // Sessions
    "list_sessions",
    "get_session",
    // Code review / product / channels / usage / skills
    "list_findings",
    "get_finding",
    "list_product_stories",
    "get_product_story",
    "list_integrations",
    "get_usage_summary",
    "list_bundled_skills",
    // Self-improvement (reads)
    "get_self_improvement_config",
    "list_improvement_runs",
    "get_improvement_run",
    "list_improvement_edits",
    // (Vault v2 structural reads removed — Vault feature disabled.)
];
const DANGEROUS: &[&str] = &[
    "run_goal_loop",
    "create_work_item",
    // Creating/altering/running a recurring autonomous job that triggers agents and
    // posts to an external destination is approval-gated (off by default).
    "create_scheduled_task",
    "update_scheduled_task",
    "delete_scheduled_task",
    "run_scheduled_task",
    "set_scheduled_task_enabled",
    // ---- Feature writes — mutating / outward-facing / agent-spawning. Off by
    // default, approval-gated by the control plane.
    "run_workflow",
    "cancel_workflow_run",
    "produce_broker_message",
    "create_pr",
    "comment_pr",
    "start_pr_review",
    "comment_issue",
    "transition_issue",
    "post_swarm_board",
    "test_integration",
    "broadcast_message",
    // Self-improvement (writes — apply/reject/rollback code & skill edits, run a pass)
    "run_self_improvement",
    "approve_improvement_edit",
    "reject_improvement_edit",
    "rollback_improvement_edit",
    // Vault v3 doc writes — file mutations (write/rename) and the soft trash
    // move. Approval-gated like every other write.
    "vault_write",
    "vault_rename",
    "vault_delete",
];

/// Non-mutating tools that are defined and enableable but stay **off by default**
/// — either because they stream potentially large/sensitive payload *content*
/// (message bodies, recalled knowledge, code, rows) or pre-date the default-on
/// read policy. Every read tool is therefore in exactly one of `DEFAULT_ENABLED`
/// or `OPT_IN_READS`; the classification invariant test asserts that.
#[cfg(test)]
const OPT_IN_READS: &[&str] = &[
    "search_codebase",
    "query_db_readonly",
    "open_pr_draft",
    "consume_broker_messages",
    // (search_memory + Vault v2 content reads removed — Vault feature disabled.)
];
const MAX_WAIT_SECS: u64 = 30;

/// Static catalog of the outward `otto.*` tools. Each entry carries a `category`
/// so the control-plane UI can group the (now large) checklist. Adding a tool here
/// surfaces it in the control plane automatically (`GET /mcp/otto-server`).
pub fn otto_tool_specs() -> Vec<Value> {
    vec![
        json!({"name":"otto.search_codebase","mutating":false,"category":"Code & Context",
            "description":"Search a workspace's code for a literal query; returns file:line matches. Read-only, confined to the workspace root.",
            "inputSchema":{"type":"object","required":["workspace_id","query"],"properties":{
                "workspace_id":{"type":"string"},"query":{"type":"string"},
                "path":{"type":"string","description":"optional sub-path within the workspace"},
                "max_results":{"type":"integer"}}}}),
        json!({"name":"otto.get_context_packet","mutating":false,"category":"Code & Context",
            "description":"Assemble a code-grounded context packet for a workspace: metadata + the most relevant code excerpts for a query.",
            "inputSchema":{"type":"object","required":["workspace_id"],"properties":{
                "workspace_id":{"type":"string"},"query":{"type":"string"},"story_id":{"type":"string"}}}}),
        json!({"name":"otto.run_goal_loop","mutating":true,"category":"Agents",
            "description":"Create and start a bounded goal loop (Plan→Execute→Evaluate→Digest). Pass a goal-loop spec. DANGEROUS: spawns autonomous agents — approval-gated.",
            "inputSchema":{"type":"object","required":["workspace_id","name","repo_path","definition","limits","config"],"properties":{
                "workspace_id":{"type":"string"},"name":{"type":"string"},"repo_path":{"type":"string"},
                "definition":{"type":"object"},"limits":{"type":"object"},"config":{"type":"object"}}}}),
        json!({"name":"otto.create_work_item","mutating":true,"category":"Swarm",
            "description":"Create a work item (a Swarm task) under a project. DANGEROUS: mutates project state — approval-gated.",
            "inputSchema":{"type":"object","required":["project_id","title"],"properties":{
                "project_id":{"type":"string"},"title":{"type":"string"},
                "description":{"type":"string"},"priority":{"type":"string"}}}}),
        json!({"name":"otto.query_db_readonly","mutating":false,"category":"Database",
            "description":"Run a READ-ONLY SQL query against an Otto DB connection. Writes/DDL and multi-statement input are rejected server-side regardless of the connection's guard.",
            "inputSchema":{"type":"object","required":["connection_id","statement"],"properties":{
                "connection_id":{"type":"string"},"statement":{"type":"string"},"max_rows":{"type":"integer"}}}}),
        json!({"name":"otto.open_pr_draft","mutating":false,"category":"Git",
            "description":"Draft a PR title + description from a repo's diff vs a base branch. Drafts text only — does NOT open/publish a PR.",
            "inputSchema":{"type":"object","required":["repo_id","base"],"properties":{
                "repo_id":{"type":"string"},"base":{"type":"string"}}}}),
        json!({"name":"otto.get_proof_pack","mutating":false,"category":"Code & Context",
            "description":"Assemble an evidence bundle for a target: git status/recent-commits/diffstat for a repo and a goal loop's machine-checked acceptance criteria.",
            "inputSchema":{"type":"object","required":["workspace_id"],"properties":{
                "workspace_id":{"type":"string"},"repo_id":{"type":"string"},
                "branch":{"type":"string"},"goal_loop_id":{"type":"string"}}}}),
        json!({"name":"otto.ask_human_approval","mutating":false,"category":"Approvals",
            "description":"Request a human's approval for an action and (optionally) wait for the decision. Creates a pending item in the MCP approval queue.",
            "inputSchema":{"type":"object","required":["title"],"properties":{
                "workspace_id":{"type":"string"},"title":{"type":"string"},
                "detail":{"type":"string"},"wait_seconds":{"type":"integer"}}}}),

        // ================= Workflows =================
        json!({"name":"otto.list_workflows","mutating":false,"category":"Workflows",
            "description":"List a workspace's workflows (visual node-graph automations). Read-only.",
            "inputSchema":{"type":"object","required":["workspace_id"],"properties":{"workspace_id":{"type":"string"}}}}),
        json!({"name":"otto.get_workflow","mutating":false,"category":"Workflows",
            "description":"Get one workflow's full definition (graph nodes + edges + metadata) by id. Read-only.",
            "inputSchema":{"type":"object","required":["workflow_id"],"properties":{"workflow_id":{"type":"string"}}}}),
        json!({"name":"otto.list_workflow_runs","mutating":false,"category":"Workflows",
            "description":"List the recent runs of a workflow (status + timing). Read-only.",
            "inputSchema":{"type":"object","required":["workflow_id"],"properties":{"workflow_id":{"type":"string"}}}}),
        json!({"name":"otto.get_workflow_run","mutating":false,"category":"Workflows",
            "description":"Get one workflow run's status, per-node step states and outputs by run id. Read-only.",
            "inputSchema":{"type":"object","required":["run_id"],"properties":{"run_id":{"type":"string"}}}}),
        json!({"name":"otto.run_workflow","mutating":true,"category":"Workflows",
            "description":"Execute a workflow now; returns the new run. Optionally pass `input` (seed JSON) and `start_node` (run that node + downstream). DANGEROUS: spawns agents / external effects — approval-gated.",
            "inputSchema":{"type":"object","required":["workflow_id"],"properties":{
                "workflow_id":{"type":"string"},"input":{"type":"object"},"start_node":{"type":"string"}}}}),
        json!({"name":"otto.cancel_workflow_run","mutating":true,"category":"Workflows",
            "description":"Cancel a running workflow run by id. DANGEROUS — approval-gated.",
            "inputSchema":{"type":"object","required":["run_id"],"properties":{"run_id":{"type":"string"}}}}),

        // ================= Message Brokers =================
        json!({"name":"otto.list_broker_clusters","mutating":false,"category":"Message Brokers",
            "description":"List a workspace's broker clusters (Kafka). Read-only.",
            "inputSchema":{"type":"object","required":["workspace_id"],"properties":{"workspace_id":{"type":"string"}}}}),
        json!({"name":"otto.list_broker_topics","mutating":false,"category":"Message Brokers",
            "description":"List the topics of a broker cluster (name + partition/replication summary). Read-only.",
            "inputSchema":{"type":"object","required":["cluster_id"],"properties":{"cluster_id":{"type":"string"}}}}),
        json!({"name":"otto.get_broker_topic","mutating":false,"category":"Message Brokers",
            "description":"Get one topic's detail (partitions, offsets, config) on a cluster. Read-only.",
            "inputSchema":{"type":"object","required":["cluster_id","topic"],"properties":{
                "cluster_id":{"type":"string"},"topic":{"type":"string"}}}}),
        json!({"name":"otto.list_consumer_groups","mutating":false,"category":"Message Brokers",
            "description":"List a cluster's consumer groups (state + lag summary). Read-only.",
            "inputSchema":{"type":"object","required":["cluster_id"],"properties":{"cluster_id":{"type":"string"}}}}),
        json!({"name":"otto.consume_broker_messages","mutating":false,"category":"Message Brokers",
            "description":"Read recent messages from a topic (the latest `limit`, no offset commits — purely a read). Off by default (streams payloads); enable to inspect message content.",
            "inputSchema":{"type":"object","required":["cluster_id","topic"],"properties":{
                "cluster_id":{"type":"string"},"topic":{"type":"string"},"partition":{"type":"integer"},
                "limit":{"type":"integer"},"value_filter":{"type":"string","description":"substring filter on the decoded value"}}}}),
        json!({"name":"otto.produce_broker_message","mutating":true,"category":"Message Brokers",
            "description":"Produce a message to a topic. `value` required; optional `key`/`partition`. Guarded clusters need `confirm=true`. DANGEROUS: writes to a broker — approval-gated.",
            "inputSchema":{"type":"object","required":["cluster_id","topic","value"],"properties":{
                "cluster_id":{"type":"string"},"topic":{"type":"string"},"value":{"type":"string"},
                "key":{"type":"string"},"partition":{"type":"integer"},"confirm":{"type":"boolean"}}}}),

        // ================= Connections =================
        json!({"name":"otto.list_connections","mutating":false,"category":"Database",
            "description":"List a workspace's connections (DB/SSH) — id, name, kind, environment. Secrets are never included. Read-only.",
            "inputSchema":{"type":"object","required":["workspace_id"],"properties":{"workspace_id":{"type":"string"}}}}),

        // ================= Git =================
        json!({"name":"otto.list_repos","mutating":false,"category":"Git",
            "description":"List a workspace's git repositories (id, name, branch, remote). Read-only.",
            "inputSchema":{"type":"object","required":["workspace_id"],"properties":{"workspace_id":{"type":"string"}}}}),
        json!({"name":"otto.git_status","mutating":false,"category":"Git",
            "description":"Get a repo's git status (current branch, staged/unstaged/untracked files). Read-only.",
            "inputSchema":{"type":"object","required":["repo_id"],"properties":{"repo_id":{"type":"string"}}}}),
        json!({"name":"otto.list_prs","mutating":false,"category":"Git",
            "description":"List a repo's pull requests. Optional `state` filter (open|merged|declined|all). Read-only.",
            "inputSchema":{"type":"object","required":["repo_id"],"properties":{
                "repo_id":{"type":"string"},"state":{"type":"string"}}}}),
        json!({"name":"otto.get_pr","mutating":false,"category":"Git",
            "description":"Get one pull request's detail (title, description, state, branches) by number. Read-only.",
            "inputSchema":{"type":"object","required":["repo_id","number"],"properties":{
                "repo_id":{"type":"string"},"number":{"type":"integer"}}}}),
        json!({"name":"otto.create_pr","mutating":true,"category":"Git",
            "description":"Open a pull request on a repo's provider. DANGEROUS: outward-facing publish — approval-gated.",
            "inputSchema":{"type":"object","required":["repo_id","title","description","source_branch","target_branch"],"properties":{
                "repo_id":{"type":"string"},"title":{"type":"string"},"description":{"type":"string"},
                "source_branch":{"type":"string"},"target_branch":{"type":"string"}}}}),
        json!({"name":"otto.comment_pr","mutating":true,"category":"Git",
            "description":"Post a comment on a pull request — general, inline when `path` (and optionally `line`) anchor it to a file in the diff, or a threaded reply when `in_reply_to` names an existing comment id. DANGEROUS: outward-facing — approval-gated.",
            "inputSchema":{"type":"object","required":["repo_id","number","body"],"properties":{
                "repo_id":{"type":"string"},"number":{"type":"integer"},"body":{"type":"string"},
                "path":{"type":"string"},"line":{"type":"integer"},"in_reply_to":{"type":"string"}}}}),
        json!({"name":"otto.start_pr_review","mutating":true,"category":"Code Review",
            "description":"Start Otto's multi-agent review of a pull request (fan-out). DANGEROUS: spawns agents — approval-gated.",
            "inputSchema":{"type":"object","required":["repo_id","pr_number"],"properties":{
                "repo_id":{"type":"string"},"pr_number":{"type":"integer"}}}}),

        // ================= Issues (Jira / Confluence) =================
        json!({"name":"otto.search_issues","mutating":false,"category":"Issues",
            "description":"Search Jira issues for an issue account. `query` is JQL (empty → recent). Optional `project`. Read-only.",
            "inputSchema":{"type":"object","required":["account_id"],"properties":{
                "account_id":{"type":"string"},"query":{"type":"string"},"project":{"type":"string"}}}}),
        json!({"name":"otto.get_issue","mutating":false,"category":"Issues",
            "description":"Get one Jira issue's full detail (description, comments, changelog, links) by key. Read-only.",
            "inputSchema":{"type":"object","required":["account_id","key"],"properties":{
                "account_id":{"type":"string"},"key":{"type":"string"}}}}),
        json!({"name":"otto.search_confluence","mutating":false,"category":"Issues",
            "description":"Search Confluence pages for an issue account. `query` is the search text; optional `space`. Read-only.",
            "inputSchema":{"type":"object","required":["account_id","query"],"properties":{
                "account_id":{"type":"string"},"query":{"type":"string"},"space":{"type":"string"}}}}),
        json!({"name":"otto.comment_issue","mutating":true,"category":"Issues",
            "description":"Add a comment to a Jira issue. DANGEROUS: outward-facing — approval-gated.",
            "inputSchema":{"type":"object","required":["account_id","key","body"],"properties":{
                "account_id":{"type":"string"},"key":{"type":"string"},"body":{"type":"string"}}}}),
        json!({"name":"otto.transition_issue","mutating":true,"category":"Issues",
            "description":"Transition a Jira issue to a new status. `transition_id` from the issue's available transitions. DANGEROUS — approval-gated.",
            "inputSchema":{"type":"object","required":["account_id","key","transition_id"],"properties":{
                "account_id":{"type":"string"},"key":{"type":"string"},"transition_id":{"type":"string"}}}}),

        // ================= Swarm =================
        json!({"name":"otto.list_swarms","mutating":false,"category":"Swarm",
            "description":"List a workspace's agent swarms. Read-only.",
            "inputSchema":{"type":"object","required":["workspace_id"],"properties":{"workspace_id":{"type":"string"}}}}),
        json!({"name":"otto.get_swarm","mutating":false,"category":"Swarm",
            "description":"Get a swarm's detail (agents, projects, counts) by id. Read-only.",
            "inputSchema":{"type":"object","required":["swarm_id"],"properties":{"swarm_id":{"type":"string"}}}}),
        json!({"name":"otto.list_swarm_runs","mutating":false,"category":"Swarm",
            "description":"List a workspace's swarm runs. Read-only.",
            "inputSchema":{"type":"object","required":["workspace_id"],"properties":{"workspace_id":{"type":"string"}}}}),
        json!({"name":"otto.get_swarm_board","mutating":false,"category":"Swarm",
            "description":"Read a swarm's shared message board. Read-only.",
            "inputSchema":{"type":"object","required":["swarm_id"],"properties":{"swarm_id":{"type":"string"}}}}),
        json!({"name":"otto.post_swarm_board","mutating":true,"category":"Swarm",
            "description":"Post a message to a swarm's shared board. Optional `project_id`/`task_id` context. DANGEROUS: drives swarm agents — approval-gated.",
            "inputSchema":{"type":"object","required":["swarm_id","body"],"properties":{
                "swarm_id":{"type":"string"},"body":{"type":"string"},
                "project_id":{"type":"string"},"task_id":{"type":"string"}}}}),

        // ================= Vault (docs home) =================
        json!({"name":"otto.vault_list","mutating":false,"category":"Vault",
            "description":"List markdown doc vaults (id, name, root, OKF flag, note/link counts). Vaults are a global library — every workspace sees them all. Read-only.",
            "inputSchema":{"type":"object","required":[],"properties":{"workspace_id":{"type":"string","description":"Optional — vaults are global; defaults to an accessible workspace."}}}}),
        json!({"name":"otto.vault_dir","mutating":false,"category":"Vault",
            "description":"One level of a vault's folder tree (folders, notes, attachments). Read-only.",
            "inputSchema":{"type":"object","required":["vault_id"],"properties":{
                "workspace_id":{"type":"string","description":"Optional — vaults are global; defaults to an accessible workspace."},"vault_id":{"type":"integer"},"path":{"type":"string"}}}}),
        json!({"name":"otto.vault_read","mutating":false,"category":"Vault",
            "description":"A note's raw markdown + metadata + outgoing links. Read-only.",
            "inputSchema":{"type":"object","required":["vault_id","path"],"properties":{
                "workspace_id":{"type":"string","description":"Optional — vaults are global; defaults to an accessible workspace."},"vault_id":{"type":"integer"},"path":{"type":"string"}}}}),
        json!({"name":"otto.vault_search","mutating":false,"category":"Vault",
            "description":"Full-text (FTS5) search over a vault's notes with snippets; tag:/path:/type: operators. Read-only.",
            "inputSchema":{"type":"object","required":["vault_id","query"],"properties":{
                "workspace_id":{"type":"string","description":"Optional — vaults are global; defaults to an accessible workspace."},"vault_id":{"type":"integer"},"query":{"type":"string"},"limit":{"type":"integer"}}}}),
        json!({"name":"otto.vault_backlinks","mutating":false,"category":"Vault",
            "description":"Notes linking TO a given note, with context snippets. Read-only.",
            "inputSchema":{"type":"object","required":["vault_id","path"],"properties":{
                "workspace_id":{"type":"string","description":"Optional — vaults are global; defaults to an accessible workspace."},"vault_id":{"type":"integer"},"path":{"type":"string"}}}}),
        json!({"name":"otto.vault_tags","mutating":false,"category":"Vault",
            "description":"Every tag in a vault with note counts. Read-only.",
            "inputSchema":{"type":"object","required":["vault_id"],"properties":{
                "workspace_id":{"type":"string","description":"Optional — vaults are global; defaults to an accessible workspace."},"vault_id":{"type":"integer"}}}}),
        json!({"name":"otto.vault_graph","mutating":false,"category":"Vault",
            "description":"The vault link graph (compact arrays; local neighborhood when `path` given). Read-only.",
            "inputSchema":{"type":"object","required":["vault_id"],"properties":{
                "workspace_id":{"type":"string","description":"Optional — vaults are global; defaults to an accessible workspace."},"vault_id":{"type":"integer"},"mode":{"type":"string"},"path":{"type":"string"},"depth":{"type":"integer"}}}}),
        json!({"name":"otto.vault_okf_validate","mutating":false,"category":"Vault",
            "description":"Deterministic OKF v0.1 conformance report (E1-E3 errors, W1-W5 warnings). Read-only.",
            "inputSchema":{"type":"object","required":["vault_id"],"properties":{
                "workspace_id":{"type":"string","description":"Optional — vaults are global; defaults to an accessible workspace."},"vault_id":{"type":"integer"}}}}),
        json!({"name":"otto.vault_write","mutating":true,"category":"Vault",
            "description":"Create/update a markdown note in a doc vault (OKF preferred). DANGEROUS: writes files — approval-gated.",
            "inputSchema":{"type":"object","required":["vault_id","path","content"],"properties":{
                "workspace_id":{"type":"string","description":"Optional — vaults are global; defaults to an accessible workspace."},"vault_id":{"type":"integer"},"path":{"type":"string"},
                "content":{"type":"string"},"if_hash":{"type":"string"}}}}),
        json!({"name":"otto.vault_rename","mutating":true,"category":"Vault",
            "description":"Rename/move a note or folder; rewrites every referencing link across the vault. DANGEROUS — approval-gated.",
            "inputSchema":{"type":"object","required":["vault_id","from","to"],"properties":{
                "workspace_id":{"type":"string","description":"Optional — vaults are global; defaults to an accessible workspace."},"vault_id":{"type":"integer"},"from":{"type":"string"},"to":{"type":"string"}}}}),
        json!({"name":"otto.vault_delete","mutating":true,"category":"Vault",
            "description":"Soft-delete a note into the vault's .trash/ (never destroys files). DANGEROUS — approval-gated.",
            "inputSchema":{"type":"object","required":["vault_id","path"],"properties":{
                "workspace_id":{"type":"string","description":"Optional — vaults are global; defaults to an accessible workspace."},"vault_id":{"type":"integer"},"path":{"type":"string"}}}}),


        // ================= Sessions =================
        json!({"name":"otto.list_sessions","mutating":false,"category":"Sessions",
            "description":"List a workspace's agent/terminal sessions (id, title, kind, status). Read-only.",
            "inputSchema":{"type":"object","required":["workspace_id"],"properties":{"workspace_id":{"type":"string"}}}}),
        json!({"name":"otto.get_session","mutating":false,"category":"Sessions",
            "description":"Get one session's detail by id. Read-only.",
            "inputSchema":{"type":"object","required":["session_id"],"properties":{"session_id":{"type":"string"}}}}),
        json!({"name":"otto.broadcast_message","mutating":true,"category":"Sessions",
            "description":"Relay a literal text message to a workspace's live agent sessions. DANGEROUS: drives running agents — approval-gated.",
            "inputSchema":{"type":"object","required":["workspace_id","text"],"properties":{
                "workspace_id":{"type":"string"},"text":{"type":"string"}}}}),

        // ================= Code Review / Findings =================
        json!({"name":"otto.list_findings","mutating":false,"category":"Code Review",
            "description":"List a code review's findings (with workflow state) by review id. Read-only.",
            "inputSchema":{"type":"object","required":["review_id"],"properties":{"review_id":{"type":"string"}}}}),
        json!({"name":"otto.get_finding","mutating":false,"category":"Code Review",
            "description":"Get one review finding's detail + event timeline by id. Read-only.",
            "inputSchema":{"type":"object","required":["finding_id"],"properties":{"finding_id":{"type":"string"}}}}),

        // ================= Product =================
        json!({"name":"otto.list_product_stories","mutating":false,"category":"Product",
            "description":"List a workspace's product stories (Jira/Confluence-backed). Read-only.",
            "inputSchema":{"type":"object","required":["workspace_id"],"properties":{"workspace_id":{"type":"string"}}}}),
        json!({"name":"otto.get_product_story","mutating":false,"category":"Product",
            "description":"Get one product story's detail by id. Read-only.",
            "inputSchema":{"type":"object","required":["story_id"],"properties":{"story_id":{"type":"string"}}}}),

        // ================= Channels =================
        json!({"name":"otto.list_integrations","mutating":false,"category":"Channels",
            "description":"List a workspace's channel integrations (Slack/Telegram/webhook). Read-only.",
            "inputSchema":{"type":"object","required":["workspace_id"],"properties":{"workspace_id":{"type":"string"}}}}),
        json!({"name":"otto.test_integration","mutating":true,"category":"Channels",
            "description":"Send a test message to a configured channel integration (`channel` = slack|telegram|webhook). DANGEROUS: outward-facing send — approval-gated.",
            "inputSchema":{"type":"object","required":["workspace_id","channel"],"properties":{
                "workspace_id":{"type":"string"},"channel":{"type":"string"}}}}),

        // ================= Usage =================
        json!({"name":"otto.get_usage_summary","mutating":false,"category":"Usage",
            "description":"Token-usage rollups by provider/day/session/feature (root-only endpoint; non-root callers get a clean 403). Optional `days` (default 30). Read-only.",
            "inputSchema":{"type":"object","properties":{"days":{"type":"integer"},"otto_only":{"type":"boolean"}}}}),

        // ================= Skills =================
        json!({"name":"otto.list_bundled_skills","mutating":false,"category":"Skills",
            "description":"List Otto's bundled skill catalogue (name, version, install state). Read-only.",
            "inputSchema":{"type":"object","properties":{}}}),

        // ================= Self-Improvement =================
        json!({"name":"otto.get_self_improvement_config","mutating":false,"category":"Self-Improvement",
            "description":"Get a workspace's self-improvement config (cadence, autonomy). Read-only.",
            "inputSchema":{"type":"object","required":["workspace_id"],"properties":{"workspace_id":{"type":"string"}}}}),
        json!({"name":"otto.list_improvement_runs","mutating":false,"category":"Self-Improvement",
            "description":"List a workspace's self-improvement runs (status + summary). Read-only.",
            "inputSchema":{"type":"object","required":["workspace_id"],"properties":{"workspace_id":{"type":"string"}}}}),
        json!({"name":"otto.get_improvement_run","mutating":false,"category":"Self-Improvement",
            "description":"Get one self-improvement run's detail by id. Read-only.",
            "inputSchema":{"type":"object","required":["run_id"],"properties":{"run_id":{"type":"string"}}}}),
        json!({"name":"otto.list_improvement_edits","mutating":false,"category":"Self-Improvement",
            "description":"List a workspace's self-improvement edit suggestions (pending/applied) with their status. Read-only.",
            "inputSchema":{"type":"object","required":["workspace_id"],"properties":{"workspace_id":{"type":"string"}}}}),
        json!({"name":"otto.run_self_improvement","mutating":true,"category":"Self-Improvement",
            "description":"Trigger a self-improvement pass for a workspace now. DANGEROUS: spawns an analysis agent — approval-gated.",
            "inputSchema":{"type":"object","required":["workspace_id"],"properties":{"workspace_id":{"type":"string"}}}}),
        json!({"name":"otto.approve_improvement_edit","mutating":true,"category":"Self-Improvement",
            "description":"Approve (apply) a self-improvement edit suggestion. DANGEROUS: mutates skills/config — approval-gated.",
            "inputSchema":{"type":"object","required":["edit_id"],"properties":{"edit_id":{"type":"string"}}}}),
        json!({"name":"otto.reject_improvement_edit","mutating":true,"category":"Self-Improvement",
            "description":"Reject (deny) a pending self-improvement edit suggestion. DANGEROUS — approval-gated.",
            "inputSchema":{"type":"object","required":["edit_id"],"properties":{"edit_id":{"type":"string"}}}}),
        json!({"name":"otto.rollback_improvement_edit","mutating":true,"category":"Self-Improvement",
            "description":"Roll back (remove) a previously-applied self-improvement edit. DANGEROUS — approval-gated.",
            "inputSchema":{"type":"object","required":["edit_id"],"properties":{"edit_id":{"type":"string"}}}}),

        // ---- Scheduled Tasks ----
        json!({"name":"otto.list_scheduled_tasks","mutating":false,"category":"Scheduled Tasks",
            "description":"List a workspace's scheduled tasks (recurring agent jobs). Read-only.",
            "inputSchema":{"type":"object","required":["workspace_id"],"properties":{
                "workspace_id":{"type":"string"}}}}),
        json!({"name":"otto.list_scheduled_task_runs","mutating":false,"category":"Scheduled Tasks",
            "description":"List the recent run history (status + summary) of a scheduled task. Read-only.",
            "inputSchema":{"type":"object","required":["task_id"],"properties":{
                "task_id":{"type":"string"}}}}),
        json!({"name":"otto.create_scheduled_task","mutating":true,"category":"Scheduled Tasks",
            "description":"Create a scheduled task: a recurring job that runs an agent (or hands off to a workflow) on a cadence, writes a Markdown report, and delivers it to a destination. DANGEROUS: an autonomous recurring capability — approval-gated. `schedule` = {cadence:'interval'|'daily'|'weekly'|'cron', every_min, at:'HH:MM', weekday, expr:'<5-field cron>'} interpreted in `timezone` (IANA). `provider` = claude|codex|agy|shell|<custom>. `kind` = 'agent_prompt'|'workflow' (workflow requires workflow_id). `sandbox` = 'none'|'worktree'. `max_retries` 0..5. `notify_on_change` only delivers when the report changes. `attach_proof` builds a proof pack. `destination` = {type:'none'|'slack'|'telegram'|'email'|'webhook', ...}.",
            "inputSchema":{"type":"object","required":["workspace_id","name","prompt"],"properties":{
                "workspace_id":{"type":"string"},"name":{"type":"string"},"prompt":{"type":"string"},
                "kind":{"type":"string"},"provider":{"type":"string"},"model":{"type":"string"},
                "schedule":{"type":"object"},"destination":{"type":"object"},"timezone":{"type":"string"},
                "workflow_id":{"type":"string"},"sandbox":{"type":"string"},"max_retries":{"type":"integer"},
                "notify_on_change":{"type":"boolean"},"attach_proof":{"type":"boolean"},
                "cwd":{"type":"string"},"skill":{"type":"string"},"enabled":{"type":"boolean"}}}}),
        json!({"name":"otto.update_scheduled_task","mutating":true,"category":"Scheduled Tasks",
            "description":"Update a scheduled task's fields (name/prompt/schedule/destination/provider/timezone/sandbox/max_retries/notify_on_change/attach_proof/workflow_id/skill/enabled). DANGEROUS — approval-gated.",
            "inputSchema":{"type":"object","required":["task_id"],"properties":{
                "task_id":{"type":"string"},"name":{"type":"string"},"prompt":{"type":"string"},
                "provider":{"type":"string"},"schedule":{"type":"object"},"destination":{"type":"object"},
                "timezone":{"type":"string"},"workflow_id":{"type":"string"},"sandbox":{"type":"string"},
                "max_retries":{"type":"integer"},"notify_on_change":{"type":"boolean"},
                "attach_proof":{"type":"boolean"},"skill":{"type":"string"},"enabled":{"type":"boolean"}}}}),
        json!({"name":"otto.set_scheduled_task_enabled","mutating":true,"category":"Scheduled Tasks",
            "description":"Enable or disable a scheduled task. DANGEROUS — approval-gated.",
            "inputSchema":{"type":"object","required":["task_id","enabled"],"properties":{
                "task_id":{"type":"string"},"enabled":{"type":"boolean"}}}}),
        json!({"name":"otto.run_scheduled_task","mutating":true,"category":"Scheduled Tasks",
            "description":"Run a scheduled task once now (does not change its schedule). Returns the run. DANGEROUS — approval-gated.",
            "inputSchema":{"type":"object","required":["task_id"],"properties":{
                "task_id":{"type":"string"}}}}),
        json!({"name":"otto.delete_scheduled_task","mutating":true,"category":"Scheduled Tasks",
            "description":"Delete a scheduled task and its run history. DANGEROUS — approval-gated.",
            "inputSchema":{"type":"object","required":["task_id"],"properties":{
                "task_id":{"type":"string"}}}}),
    ]
}

async fn enabled_tools(ctx: &ServerCtx) -> Vec<String> {
    SettingsRepo::new(ctx.pool.clone())
        .get("mcp_otto_server_tools")
        .await
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
        .unwrap_or_else(|| DEFAULT_ENABLED.iter().map(|s| s.to_string()).collect())
}

async fn outward_enabled(ctx: &ServerCtx) -> bool {
    SettingsRepo::new(ctx.pool.clone())
        .get("mcp_otto_server_enabled")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Internal session-scoped MCP credentials are not an outward integration and
/// therefore do not depend on the admin's outward-server toggle/tool list. Their
/// immutable per-token scope has already run before this check. External MCP
/// tokens retain the existing master-toggle + enabled-tool behavior.
fn mcp_tool_enabled_for_token(
    internal: bool,
    outward_on: bool,
    globally_enabled: &[String],
    short: &str,
) -> bool {
    internal || (outward_on && globally_enabled.iter().any(|tool| tool == short))
}

async fn require_approval_dangerous(ctx: &ServerCtx) -> bool {
    SettingsRepo::new(ctx.pool.clone())
        .get("mcp_require_approval_dangerous")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

/// Whether an explicit per-token write grant (`McpScope.allow_writes`) counts as
/// having already approved DANGEROUS tools for that token. Default **true**:
/// minting a read+write MCP token IS the approval decision, and asking again per
/// call strands automation without adding a real check. Set
/// `mcp_trust_token_write_grant: false` to restore the per-call prompt for
/// write-scoped tokens too.
async fn trust_token_write_grant(ctx: &ServerCtx) -> bool {
    SettingsRepo::new(ctx.pool.clone())
        .get("mcp_trust_token_write_grant")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

/// Short tool names exempted from the DANGEROUS approval gate — set via the
/// `mcp_approval_exempt_tools` setting (a JSON array of short names, e.g.
/// `["comment_pr"]`).
///
/// WHY this exists: the gate was all-or-nothing. An operator who deliberately
/// enabled ONE outward-facing tool (say, PR comments for the review workflow)
/// could only stop the second approval prompt by clearing
/// `mcp_require_approval_dangerous`, which simultaneously disarms `create_pr`,
/// `run_workflow`, `produce_broker_message`, `vault_delete`, `broadcast_message`
/// and every other write. Enabling one tool should not require disarming all of
/// them, so exemption is per-tool and opt-in.
///
/// A tool must STILL be enabled (`tool_enabled`) to run at all — this only skips
/// the approval prompt for a capability already granted. Calls remain audited.
async fn approval_exempt_tools(ctx: &ServerCtx) -> Vec<String> {
    SettingsRepo::new(ctx.pool.clone())
        .get("mcp_approval_exempt_tools")
        .await
        .ok()
        .flatten()
        .and_then(|v| {
            v.as_array().map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(|s| s.trim().trim_start_matches("otto.").to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// Build the human-facing approval detail. For scheduled-task create/update it
/// surfaces the prompt + cadence + destination so the approver knows exactly what
/// recurring autonomous capability they are granting (security review fix).
fn dangerous_detail(tool: &str, args: &Value) -> String {
    let short = tool.strip_prefix("otto.").unwrap_or(tool);
    match short {
        "create_scheduled_task" | "update_scheduled_task" => {
            let name = args.get("name").and_then(Value::as_str).unwrap_or("(unnamed)");
            let sched = args.get("schedule");
            let cadence = sched
                .and_then(|s| s.get("cadence"))
                .and_then(Value::as_str)
                .unwrap_or("interval");
            let cad = match sched.and_then(|s| s.get("every_min")).and_then(Value::as_i64) {
                Some(m) => format!("{cadence} (every {m} min)"),
                None => cadence.to_string(),
            };
            let dest = args
                .get("destination")
                .and_then(|d| d.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("none");
            let prompt: String = args
                .get("prompt")
                .and_then(Value::as_str)
                .unwrap_or("")
                .chars()
                .take(160)
                .collect();
            format!(
                "Recurring agent job '{name}' — cadence: {cad}; destination: {dest}; prompt: {prompt}"
            )
        }
        // Surface the concrete target of each new outward-facing / mutating tool so
        // the approver knows exactly what capability they are granting.
        "run_workflow" => format!(
            "Run workflow '{}'",
            args.get("workflow_id").and_then(Value::as_str).unwrap_or("?")
        ),
        "cancel_workflow_run" => format!(
            "Cancel workflow run '{}'",
            args.get("run_id").and_then(Value::as_str).unwrap_or("?")
        ),
        "produce_broker_message" => format!(
            "Produce a message to topic '{}' on cluster '{}'",
            args.get("topic").and_then(Value::as_str).unwrap_or("?"),
            args.get("cluster_id").and_then(Value::as_str).unwrap_or("?")
        ),
        "create_pr" => format!(
            "Open a PR on repo '{}': {} ({} → {})",
            args.get("repo_id").and_then(Value::as_str).unwrap_or("?"),
            args.get("title").and_then(Value::as_str).unwrap_or(""),
            args.get("source_branch").and_then(Value::as_str).unwrap_or("?"),
            args.get("target_branch").and_then(Value::as_str).unwrap_or("?")
        ),
        "comment_pr" => format!(
            "Comment on PR #{} of repo '{}'",
            args.get("number").and_then(Value::as_i64).unwrap_or(0),
            args.get("repo_id").and_then(Value::as_str).unwrap_or("?")
        ),
        "start_pr_review" => format!(
            "Start a multi-agent review of PR #{} on repo '{}'",
            args.get("pr_number").and_then(Value::as_i64).unwrap_or(0),
            args.get("repo_id").and_then(Value::as_str).unwrap_or("?")
        ),
        "comment_issue" => format!(
            "Comment on issue '{}'",
            args.get("key").and_then(Value::as_str).unwrap_or("?")
        ),
        "transition_issue" => format!(
            "Transition issue '{}' (transition '{}')",
            args.get("key").and_then(Value::as_str).unwrap_or("?"),
            args.get("transition_id").and_then(Value::as_str).unwrap_or("?")
        ),
        "post_swarm_board" => format!(
            "Post to swarm '{}' board",
            args.get("swarm_id").and_then(Value::as_str).unwrap_or("?")
        ),
        "test_integration" => format!(
            "Send a test message to the '{}' channel of a workspace",
            args.get("channel").and_then(Value::as_str).unwrap_or("?")
        ),
        "broadcast_message" => {
            let text: String = args
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .chars()
                .take(120)
                .collect();
            format!("Broadcast a message to live agent sessions: {text}")
        }
        "run_self_improvement" => format!(
            "Run a self-improvement pass on workspace '{}'",
            args.get("workspace_id").and_then(Value::as_str).unwrap_or("?")
        ),
        "approve_improvement_edit" => format!(
            "Apply self-improvement edit '{}' (mutates skills/config)",
            args.get("edit_id").and_then(Value::as_str).unwrap_or("?")
        ),
        "reject_improvement_edit" => format!(
            "Reject self-improvement edit '{}'",
            args.get("edit_id").and_then(Value::as_str).unwrap_or("?")
        ),
        "rollback_improvement_edit" => format!(
            "Roll back applied self-improvement edit '{}'",
            args.get("edit_id").and_then(Value::as_str).unwrap_or("?")
        ),
        _ => format!("External agent requests the dangerous tool '{tool}'."),
    }
}

// ===========================================================================
// POST /mcp/otto-tools/invoke  (the governed choke point for every otto.* tool)
// ===========================================================================

#[derive(Deserialize)]
pub struct OttoInvokeReq {
    pub tool: String,
    #[serde(default)]
    pub arguments: Value,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub wait_seconds: Option<u64>,
}

/// True iff the bare tool name (`"run_workflow"`, no `otto.` prefix) is a
/// **mutating** tool. The mutating set is exactly [`DANGEROUS`] (every catalog
/// entry with `mutating:true` is approval-gated), so this is the single source of
/// truth the per-token read-only axis keys on.
pub(crate) fn tool_is_mutating(bare: &str) -> bool {
    DANGEROUS.contains(&bare)
}

pub async fn otto_tools_invoke(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<OttoInvokeReq>,
) -> ApiResult<Json<Value>> {
    governed_invoke(
        &ctx,
        &auth,
        &req.tool,
        &req.arguments,
        req.dry_run,
        req.wait_seconds,
    )
    .await
    .map(Json)
}

/// The governed choke point for every `otto.*` tool call, shared by the bespoke
/// `POST /mcp/otto-tools/invoke` endpoint AND the MCP HTTP transport's
/// `tools/call`. It enforces — in order — the **per-token scope** (multi-token
/// access control), the global enable + per-tool allow-list, dangerous→approval,
/// dry-run, then executes the capability AS `auth.effective_user` (native RBAC
/// reused via an ephemeral self-call) and audits the whole thing.
///
/// Returns the governed envelope `Value` (`{decision, executed, content, …}`).
pub(crate) async fn governed_invoke(
    ctx: &ServerCtx,
    auth: &AuthContext,
    tool: &str,
    arguments: &Value,
    dry_run: bool,
    wait_seconds: Option<u64>,
) -> ApiResult<Value> {
    let user = &auth.effective_user;
    let short = tool.strip_prefix("otto.").unwrap_or(tool).to_string();
    // Vault calls may omit `workspace_id` (vaults are a global library) —
    // resolve the effective workspace up front so the token's workspace pin,
    // the audit record, and the approval args-hash all see the same scoped
    // arguments.
    let filled = fill_vault_workspace(ctx, auth, &short, arguments)
        .await
        .map_err(ApiError)?;
    let arguments = filled.as_ref().unwrap_or(arguments);
    let mut audit = NewCallLog {
        tool: tool.to_string(),
        direction: "inbound".into(),
        server_name: Some("otto".into()),
        caller_user_id: Some(user.id.clone()),
        caller_kind: Some("mcp_server".into()),
        args_redacted_json: otto_core::redact::redact_json(arguments).value.to_string(),
        ..Default::default()
    };

    // PER-TOKEN SCOPE (multi-token access control). A `kind='mcp'` token carries an
    // [`McpScope`]; a NULL column resolved to the unrestricted scope, so legacy
    // tokens are unaffected. Normal (session/api) tokens have `mcp_scope == None`
    // and are bounded only by the global enable + the user's own RBAC below. This
    // is the gate that makes different tokens / users have different accesses, and
    // it is identical for the HTTP transport and the legacy stdio path.
    if let Some(scope) = &auth.mcp_scope {
        let mutating = tool_is_mutating(&short);
        let ws_arg = arguments.get("workspace_id").and_then(Value::as_str);
        if let Some(reason) = scope.deny_reason(&short, mutating, ws_arg) {
            return Ok(deny_audit(ctx, &mut audit, &format!("token scope: {reason}")).await);
        }
    }

    let outward_on = outward_enabled(ctx).await;
    let enabled = enabled_tools(ctx).await;
    if !mcp_tool_enabled_for_token(auth.mcp_internal, outward_on, &enabled, &short) {
        let reason = if outward_on {
            "this tool is not enabled on the Otto MCP server"
        } else {
            "the Otto MCP server is disabled"
        };
        return Ok(deny_audit(ctx, &mut audit, reason).await);
    }

    let dangerous = DANGEROUS.contains(&short.as_str());
    // An operator who granted this specific tool in the control plane has already
    // made the call — don't ask a second time for the same decision. Exemption is
    // per-tool and opt-in, so the rest of DANGEROUS stays gated.
    let exempt = approval_exempt_tools(ctx)
        .await
        .iter()
        .any(|t| t == short.as_str());
    // A `kind='mcp'` token carrying an EXPLICIT write grant (`allow_writes`) has
    // already cleared this decision at issue time: someone deliberately minted a
    // read+write token for this caller. Re-prompting per call asks the same
    // question twice and strands automation (a workflow step posting 21 review
    // comments filed 21 approvals and posted none). The scope check above still
    // runs first, so a read-only token, a tool outside the token's allowed set,
    // or a workspace-pinned mismatch is denied outright rather than reaching here.
    // Session/api callers have `mcp_scope == None` — no explicit grant, so they
    // stay gated. Governed by `mcp_trust_token_write_grant` for operators who
    // want the belt-and-braces prompt back. Calls remain audited either way.
    let token_write_grant = auth
        .mcp_scope
        .as_ref()
        .is_some_and(|scope| scope.allow_writes)
        && trust_token_write_grant(ctx).await;
    let needs_approval =
        dangerous && !exempt && !token_write_grant && require_approval_dangerous(ctx).await;
    let args_hash = canonical_hash(arguments);
    let ws = arguments.get("workspace_id").and_then(Value::as_str).map(str::to_string);

    if needs_approval && !dry_run {
        match ctx
            .mcp
            .approvals()
            .find_usable(ws.as_deref(), None, tool, &args_hash)
            .await
            .map_err(ApiError)?
        {
            Some(appr_id) => {
                if !ctx.mcp.approvals().consume(&appr_id).await.map_err(ApiError)? {
                    return Ok(deny_audit(ctx, &mut audit, "approval already used").await);
                }
                audit.approval_id = Some(appr_id);
            }
            None => {
                let appr = ctx
                    .mcp
                    .approvals()
                    .create(NewApproval {
                        workspace_id: ws.clone(),
                        kind: "tool_call".into(),
                        server_id: None,
                        server_name: Some("otto".into()),
                        tool: Some(tool.to_string()),
                        title: format!("otto MCP server → {tool}"),
                        detail: Some(dangerous_detail(tool, arguments)),
                        args_redacted_json: audit.args_redacted_json.clone(),
                        args_hash: Some(args_hash.clone()),
                        risk_label: Some("dangerous".into()),
                        requested_by: Some(user.id.clone()),
                        requested_by_kind: Some("mcp_server".into()),
                        expires_at: Some((chrono::Utc::now() + chrono::Duration::minutes(120)).to_rfc3339()),
                    })
                    .await
                    .map_err(ApiError)?;
                match wait_for_decision(ctx, &appr.id, wait_seconds).await {
                    Some(true) => {
                        let _ = ctx.mcp.approvals().consume(&appr.id).await;
                        audit.approval_id = Some(appr.id.clone());
                    }
                    Some(false) => {
                        audit.decision = "denied".into();
                        audit.decision_reason = Some("human denied the request".into());
                        let _ = ctx.mcp.call_log().insert(audit).await;
                        return Ok(json!({"decision":"denied","executed":false,"reason":"human denied the request"}));
                    }
                    None => {
                        audit.decision = "pending_approval".into();
                        audit.approval_id = Some(appr.id.clone());
                        let _ = ctx.mcp.call_log().insert(audit).await;
                        return Ok(json!({"decision":"pending_approval","executed":false,
                            "approval_id":appr.id,"reason":"awaiting human approval — resubmit after it is approved"}));
                    }
                }
            }
        }
    }

    if dry_run {
        audit.decision = "dry_run".into();
        audit.dry_run = true;
        audit.ok = true;
        let _ = ctx.mcp.call_log().insert(audit).await;
        return Ok(json!({"decision":"dry_run","executed":false,"dry_run":true,
            "preview":{"tool":tool,"arguments":otto_core::redact::redact_json(arguments).value,
                       "note":"dry-run: the tool was NOT executed"}}));
    }

    // Fail-closed audit: insert before executing.
    audit.decision = if audit.approval_id.is_some() { "approved".into() } else { "allowed".into() };
    let audit_id = ctx.mcp.call_log().insert(audit).await.map_err(ApiError)?;

    let started = std::time::Instant::now();
    let result = execute_otto_tool(ctx, user, &short, arguments).await;
    let latency = started.elapsed().as_millis() as i64;
    match result {
        Ok(value) => {
            let bytes = serde_json::to_vec(&value).map(|v| v.len() as i64).unwrap_or(0);
            let _ = ctx.mcp.call_log().finalize(&audit_id, true, None, Some(latency), Some(bytes), None).await;
            Ok(json!({"decision":"allowed","executed":true,"content":value}))
        }
        Err(e) => {
            let err = otto_core::redact::redact_text(&e.to_string()).value;
            let _ = ctx.mcp.call_log().finalize(&audit_id, false, Some(&err), Some(latency), None, None).await;
            Ok(json!({"decision":"error","executed":true,"is_error":true,"content":{"error":err}}))
        }
    }
}

async fn deny_audit(ctx: &ServerCtx, audit: &mut NewCallLog, reason: &str) -> Value {
    audit.decision = "denied".into();
    audit.decision_reason = Some(reason.to_string());
    let _ = ctx.mcp.call_log().insert(audit.clone()).await;
    json!({"decision":"denied","executed":false,"reason":reason})
}

/// Poll an approval up to a bounded wait. `Some(true)`=approved, `Some(false)`=denied,
/// `None`=still pending after the wait (caller resubmits later).
async fn wait_for_decision(ctx: &ServerCtx, approval_id: &str, wait_seconds: Option<u64>) -> Option<bool> {
    let budget = wait_seconds.unwrap_or(0).min(MAX_WAIT_SECS);
    let mut waited = 0u64;
    loop {
        if let Ok(a) = ctx.mcp.approvals().get(&approval_id.to_string()).await {
            match a.status.as_str() {
                "approved" => return Some(true),
                "denied" | "expired" | "cancelled" => return Some(false),
                _ => {}
            }
        }
        if waited >= budget {
            return None;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        waited += 1;
    }
}

// ===========================================================================
// The executor — runs each tool AS the user via an ephemeral self-call.
// ===========================================================================

fn is_read_only_sql(stmt: &str) -> bool {
    let s = stmt.trim().trim_end_matches(';').trim();
    if s.contains(';') {
        return false; // single statement only — no batch tricks
    }
    let up = s.to_uppercase();
    up.starts_with("SELECT")
        || up.starts_with("SHOW")
        || up.starts_with("DESCRIBE")
        || up.starts_with("DESC ")
        || up.starts_with("EXPLAIN")
        || up.starts_with("WITH")
}

/// Vaults are a GLOBAL library (`otto_vault::VaultEngine::list`): the `{ws}`
/// segment on the vault REST routes only picks the workspace the caller's role
/// is checked in — it never narrows which vault a `vault_id` addresses. Outward
/// callers therefore don't have to know a workspace at all: when a `vault_*`
/// call omits `workspace_id`, scope it to the token's workspace pin when one is
/// set, else the caller's first accessible workspace (preferring one they can
/// write in when the tool mutates). Returns `None` when the args need no
/// filling. Runs BEFORE the scope check so a pinned token's filled call still
/// faces `McpScope::deny_reason` with the pin satisfied — never bypassed.
async fn fill_vault_workspace(
    ctx: &ServerCtx,
    auth: &AuthContext,
    tool: &str,
    args: &Value,
) -> Result<Option<Value>, Error> {
    let has_ws = args
        .get("workspace_id")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty());
    if !tool.starts_with("vault_") || has_ws {
        return Ok(None);
    }
    let user = &auth.effective_user;
    if let Some(pin) = auth
        .mcp_scope
        .as_ref()
        .and_then(|s| s.workspace_id.as_deref())
        .filter(|s| !s.is_empty())
    {
        let mut filled = args.clone();
        filled["workspace_id"] = json!(pin);
        return Ok(Some(filled));
    }
    let repo = otto_state::WorkspacesRepo::new(ctx.pool.clone());
    let rows: Vec<(otto_core::domain::Workspace, WorkspaceRole)> = if user.is_root {
        repo.list_all()
            .await?
            .into_iter()
            .map(|w| (w, WorkspaceRole::Admin))
            .collect()
    } else {
        repo.list_for_user(&user.id).await?
    };
    let ws = pick_vault_workspace(&rows, tool_is_mutating(tool)).ok_or_else(|| {
        Error::Invalid("no accessible workspace to scope this vault call — pass 'workspace_id'".into())
    })?;
    let mut filled = args.clone();
    filled["workspace_id"] = json!(ws);
    Ok(Some(filled))
}

/// Pick the workspace an omitted-`workspace_id` vault call is scoped to: the
/// first one whose role satisfies the tool (Editor for mutating, Viewer for
/// reads), falling back to the first membership so the self-call's native RBAC
/// produces the honest 403 rather than an "unknown workspace" here.
fn pick_vault_workspace(
    rows: &[(otto_core::domain::Workspace, WorkspaceRole)],
    mutating: bool,
) -> Option<Id> {
    let need = if mutating { WorkspaceRole::Editor } else { WorkspaceRole::Viewer };
    rows.iter()
        .find(|(_, role)| *role >= need)
        .or_else(|| rows.first())
        .map(|(w, _)| w.id.clone())
}

async fn execute_otto_tool(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    tool: &str,
    args: &Value,
) -> Result<Value, Error> {
    if tool == "ask_human_approval" {
        return ask_human_approval(ctx, user, args).await;
    }
    // Mint a short-lived ephemeral token so the self-call reuses the target
    // endpoint's native RBAC; revoke it on the way out.
    let (token, _) = AuthRepo::new(ctx.pool.clone())
        .issue_api_token(&user.id, Some("mcp-otto-exec"))
        .await?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| Error::Internal(format!("http client: {e}")))?;
    let base = ctx.base_url.trim_end_matches('/').to_string();
    let result = run_tool(&client, &base, &token, tool, args).await;
    let _ = AuthRepo::new(ctx.pool.clone()).revoke(&token).await;
    result
}

fn arg_str(args: &Value, key: &str) -> Result<String, Error> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| Error::Invalid(format!("missing required string argument '{key}'")))
}

fn seg(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Optional required-integer argument extractor (PR numbers etc.).
fn arg_i64(args: &Value, key: &str) -> Result<i64, Error> {
    args.get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| Error::Invalid(format!("missing required integer argument '{key}'")))
}

/// The HTTP verb a tool's self-call uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

/// A resolved self-call: the verb, the `/api/v1/...` path (incl. query string),
/// and an optional JSON body. Built purely from `(tool, args)` by [`route_for`]
/// so the endpoint binding of every tool is unit-testable without a live server.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SelfCall {
    pub method: Method,
    pub path: String,
    pub body: Option<Value>,
}

impl SelfCall {
    fn get(path: String) -> Self {
        Self { method: Method::Get, path, body: None }
    }
    fn post(path: String, body: Value) -> Self {
        Self { method: Method::Post, path, body: Some(body) }
    }
    fn put(path: String, body: Value) -> Self {
        Self { method: Method::Put, path, body: Some(body) }
    }
    fn patch(path: String, body: Value) -> Self {
        Self { method: Method::Patch, path, body: Some(body) }
    }
    fn delete(path: String) -> Self {
        Self { method: Method::Delete, path, body: None }
    }
}

/// Map an outward tool + its (validated) arguments to the exact self-call against
/// the daemon's own REST API. Pure: no I/O, no token — every tool reuses its
/// endpoint's native RBAC when the call is later executed as the user. `ask_human_approval`
/// is handled earlier (in `execute_otto_tool`) and never reaches here.
pub(crate) fn route_for(tool: &str, args: &Value) -> Result<SelfCall, Error> {
    Ok(match tool {
        // ---- Code & context ----
        "search_codebase" => {
            let ws = arg_str(args, "workspace_id")?;
            let q = arg_str(args, "query")?;
            let mut path = format!("/api/v1/workspaces/{}/mcp/code-search?q={}", seg(&ws), seg(&q));
            if let Some(p) = args.get("path").and_then(Value::as_str) {
                path.push_str(&format!("&path={}", seg(p)));
            }
            if let Some(m) = args.get("max_results").and_then(Value::as_u64) {
                path.push_str(&format!("&max={m}"));
            }
            SelfCall::get(path)
        }
        "get_context_packet" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::post(format!("/api/v1/workspaces/{}/mcp/context-packet", seg(&ws)), args.clone())
        }
        "get_proof_pack" => {
            let ws = arg_str(args, "workspace_id")?;
            let mut path = format!("/api/v1/workspaces/{}/mcp/proof-pack?", seg(&ws));
            for k in ["repo_id", "branch", "goal_loop_id"] {
                if let Some(v) = args.get(k).and_then(Value::as_str) {
                    path.push_str(&format!("{k}={}&", seg(v)));
                }
            }
            SelfCall::get(path)
        }
        // ---- Database ----
        "query_db_readonly" => {
            let conn = arg_str(args, "connection_id")?;
            let stmt = arg_str(args, "statement")?;
            // F5: classify ourselves; reject writes/unknown/multi-statement
            // REGARDLESS of the connection's write-guard flag.
            if !is_read_only_sql(&stmt) {
                return Err(Error::Forbidden(
                    "otto.query_db_readonly only permits a single read-only statement (SELECT/SHOW/DESCRIBE/EXPLAIN/WITH)".into(),
                ));
            }
            let body = json!({
                "statement": stmt,
                "max_rows": args.get("max_rows").and_then(Value::as_u64).unwrap_or(200),
                "confirm_write": false, // forced — never honored from the caller
            });
            SelfCall::post(format!("/api/v1/connections/{}/db/query", seg(&conn)), body)
        }
        "list_connections" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::get(format!("/api/v1/workspaces/{}/connections", seg(&ws)))
        }
        // ---- Git ----
        "open_pr_draft" => {
            let repo = arg_str(args, "repo_id")?;
            let base_branch = arg_str(args, "base")?;
            SelfCall::post(format!("/api/v1/repos/{}/pr/draft", seg(&repo)), json!({"base": base_branch}))
        }
        "list_repos" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::get(format!("/api/v1/workspaces/{}/repos", seg(&ws)))
        }
        "git_status" => {
            let repo = arg_str(args, "repo_id")?;
            SelfCall::get(format!("/api/v1/repos/{}/status", seg(&repo)))
        }
        "list_prs" => {
            let repo = arg_str(args, "repo_id")?;
            let mut path = format!("/api/v1/repos/{}/prs", seg(&repo));
            if let Some(s) = args.get("state").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                path.push_str(&format!("?state={}", seg(s)));
            }
            SelfCall::get(path)
        }
        "get_pr" => {
            let repo = arg_str(args, "repo_id")?;
            let n = arg_i64(args, "number")?;
            SelfCall::get(format!("/api/v1/repos/{}/prs/{}", seg(&repo), n))
        }
        "create_pr" => {
            let repo = arg_str(args, "repo_id")?;
            let body = json!({
                "title": arg_str(args, "title")?,
                "description": arg_str(args, "description")?,
                "source_branch": arg_str(args, "source_branch")?,
                "target_branch": arg_str(args, "target_branch")?,
            });
            SelfCall::post(format!("/api/v1/repos/{}/prs", seg(&repo)), body)
        }
        "comment_pr" => {
            let repo = arg_str(args, "repo_id")?;
            let n = arg_i64(args, "number")?;
            let body = json!({
                "body": arg_str(args, "body")?,
                "path": args.get("path").and_then(Value::as_str),
                "line": args.get("line").and_then(Value::as_u64),
                "in_reply_to": args.get("in_reply_to").and_then(Value::as_str),
            });
            SelfCall::post(format!("/api/v1/repos/{}/prs/{}/comments", seg(&repo), n), body)
        }
        "start_pr_review" => {
            let repo = arg_str(args, "repo_id")?;
            let n = arg_i64(args, "pr_number")?;
            SelfCall::post(format!("/api/v1/repos/{}/prs/{}/review", seg(&repo), n), json!({}))
        }
        // ---- Workflows ----
        "list_workflows" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::get(format!("/api/v1/workspaces/{}/workflows", seg(&ws)))
        }
        "get_workflow" => {
            let id = arg_str(args, "workflow_id")?;
            SelfCall::get(format!("/api/v1/workflows/{}", seg(&id)))
        }
        "list_workflow_runs" => {
            let id = arg_str(args, "workflow_id")?;
            SelfCall::get(format!("/api/v1/workflows/{}/runs", seg(&id)))
        }
        "get_workflow_run" => {
            let id = arg_str(args, "run_id")?;
            SelfCall::get(format!("/api/v1/workflow-runs/{}", seg(&id)))
        }
        "run_workflow" => {
            let id = arg_str(args, "workflow_id")?;
            let mut body = json!({});
            if let Some(v) = args.get("input") {
                body["input"] = v.clone();
            }
            if let Some(v) = args.get("start_node").and_then(Value::as_str) {
                body["start_node"] = json!(v);
            }
            SelfCall::post(format!("/api/v1/workflows/{}/run", seg(&id)), body)
        }
        "cancel_workflow_run" => {
            let id = arg_str(args, "run_id")?;
            SelfCall::post(format!("/api/v1/workflow-runs/{}/cancel", seg(&id)), json!({}))
        }
        // ---- Message brokers ----
        "list_broker_clusters" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::get(format!("/api/v1/workspaces/{}/brokers/clusters", seg(&ws)))
        }
        "list_broker_topics" => {
            let id = arg_str(args, "cluster_id")?;
            SelfCall::get(format!("/api/v1/brokers/clusters/{}/topics", seg(&id)))
        }
        "get_broker_topic" => {
            let id = arg_str(args, "cluster_id")?;
            let topic = arg_str(args, "topic")?;
            SelfCall::get(format!("/api/v1/brokers/clusters/{}/topics/{}", seg(&id), seg(&topic)))
        }
        "list_consumer_groups" => {
            let id = arg_str(args, "cluster_id")?;
            SelfCall::get(format!("/api/v1/brokers/clusters/{}/groups", seg(&id)))
        }
        "consume_broker_messages" => {
            let id = arg_str(args, "cluster_id")?;
            let topic = arg_str(args, "topic")?;
            let mut body = json!({});
            if let Some(p) = args.get("partition").and_then(Value::as_i64) {
                body["partition"] = json!(p);
            }
            if let Some(l) = args.get("limit").and_then(Value::as_u64) {
                body["limit"] = json!(l);
            }
            if let Some(f) = args.get("value_filter").and_then(Value::as_str) {
                body["value_filter"] = json!(f);
            }
            SelfCall::post(format!("/api/v1/brokers/clusters/{}/topics/{}/consume", seg(&id), seg(&topic)), body)
        }
        "produce_broker_message" => {
            let id = arg_str(args, "cluster_id")?;
            let topic = arg_str(args, "topic")?;
            let mut body = json!({ "value": arg_str(args, "value")? });
            if let Some(k) = args.get("key").and_then(Value::as_str) {
                body["key"] = json!(k);
            }
            if let Some(p) = args.get("partition").and_then(Value::as_i64) {
                body["partition"] = json!(p);
            }
            if let Some(c) = args.get("confirm").and_then(Value::as_bool) {
                body["confirm"] = json!(c);
            }
            SelfCall::post(format!("/api/v1/brokers/clusters/{}/topics/{}/produce", seg(&id), seg(&topic)), body)
        }
        // ---- Issues (Jira / Confluence) ----
        "search_issues" => {
            let acc = arg_str(args, "account_id")?;
            let mut path = format!("/api/v1/issue/search?account_id={}", seg(&acc));
            if let Some(q) = args.get("query").and_then(Value::as_str) {
                path.push_str(&format!("&q={}", seg(q)));
            }
            if let Some(p) = args.get("project").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                path.push_str(&format!("&project={}", seg(p)));
            }
            SelfCall::get(path)
        }
        "get_issue" => {
            let acc = arg_str(args, "account_id")?;
            let key = arg_str(args, "key")?;
            SelfCall::get(format!("/api/v1/issue/{}/{}/full", seg(&acc), seg(&key)))
        }
        "search_confluence" => {
            let acc = arg_str(args, "account_id")?;
            let q = arg_str(args, "query")?;
            let mut path = format!("/api/v1/issue/confluence/search?account_id={}&q={}", seg(&acc), seg(&q));
            if let Some(s) = args.get("space").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                path.push_str(&format!("&space={}", seg(s)));
            }
            SelfCall::get(path)
        }
        "comment_issue" => {
            let acc = arg_str(args, "account_id")?;
            let key = arg_str(args, "key")?;
            let body = json!({ "body": arg_str(args, "body")? });
            SelfCall::post(format!("/api/v1/issue/{}/{}/comment", seg(&acc), seg(&key)), body)
        }
        "transition_issue" => {
            let acc = arg_str(args, "account_id")?;
            let key = arg_str(args, "key")?;
            let body = json!({ "transition_id": arg_str(args, "transition_id")? });
            SelfCall::post(format!("/api/v1/issue/{}/{}/transitions", seg(&acc), seg(&key)), body)
        }
        // ---- Swarm ----
        "list_swarms" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::get(format!("/api/v1/workspaces/{}/swarm/swarms", seg(&ws)))
        }
        "get_swarm" => {
            let id = arg_str(args, "swarm_id")?;
            SelfCall::get(format!("/api/v1/swarm/swarms/{}", seg(&id)))
        }
        "list_swarm_runs" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::get(format!("/api/v1/workspaces/{}/swarm/runs", seg(&ws)))
        }
        "get_swarm_board" => {
            let id = arg_str(args, "swarm_id")?;
            SelfCall::get(format!("/api/v1/swarm/swarms/{}/board", seg(&id)))
        }
        "post_swarm_board" => {
            let id = arg_str(args, "swarm_id")?;
            let mut body = json!({ "body": arg_str(args, "body")? });
            if let Some(p) = args.get("project_id").and_then(Value::as_str) {
                body["project_id"] = json!(p);
            }
            if let Some(t) = args.get("task_id").and_then(Value::as_str) {
                body["task_id"] = json!(t);
            }
            SelfCall::post(format!("/api/v1/swarm/swarms/{}/board", seg(&id)), body)
        }
        // ---- Memory / vault ----
        "list_memory" => {
            let ws = arg_str(args, "workspace_id")?;
            let mut q: Vec<String> = Vec::new();
            if let Some(c) = args.get("collection").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                q.push(format!("collection={}", seg(c)));
            }
            if let Some(s) = args.get("story_id").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                q.push(format!("story_id={}", seg(s)));
            }
            let mut path = format!("/api/v1/workspaces/{}/memories", seg(&ws));
            if !q.is_empty() {
                path.push('?');
                path.push_str(&q.join("&"));
            }
            SelfCall::get(path)
        }
        "search_memory" => {
            let ws = arg_str(args, "workspace_id")?;
            // `k` defaults to 0 server-side (MemoryQuery), which would return nothing —
            // supply a useful default so a caller that omits it still gets hits.
            let k = args.get("k").and_then(Value::as_u64).unwrap_or(20);
            let body = json!({ "text": arg_str(args, "query")?, "k": k });
            SelfCall::post(format!("/api/v1/workspaces/{}/memory/search", seg(&ws)), body)
        }
        // ---- Vault v3 (docs home) ----
        "vault_list" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::get(format!("/api/v1/workspaces/{}/vault/vaults", seg(&ws)))
        }
        "vault_dir" => {
            let ws = arg_str(args, "workspace_id")?;
            let v = arg_i64(args, "vault_id")?;
            let path = args.get("path").and_then(Value::as_str).unwrap_or("");
            SelfCall::get(format!("/api/v1/workspaces/{}/vault/vaults/{v}/dir?path={}", seg(&ws), seg(path)))
        }
        "vault_read" => {
            let ws = arg_str(args, "workspace_id")?;
            let v = arg_i64(args, "vault_id")?;
            SelfCall::get(format!(
                "/api/v1/workspaces/{}/vault/vaults/{v}/note?path={}",
                seg(&ws),
                seg(&arg_str(args, "path")?)
            ))
        }
        "vault_search" => {
            let ws = arg_str(args, "workspace_id")?;
            let v = arg_i64(args, "vault_id")?;
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(20);
            let body = json!({ "query": arg_str(args, "query")?, "limit": limit });
            SelfCall::post(format!("/api/v1/workspaces/{}/vault/vaults/{v}/search", seg(&ws)), body)
        }
        "vault_backlinks" => {
            let ws = arg_str(args, "workspace_id")?;
            let v = arg_i64(args, "vault_id")?;
            SelfCall::get(format!(
                "/api/v1/workspaces/{}/vault/vaults/{v}/backlinks?path={}",
                seg(&ws),
                seg(&arg_str(args, "path")?)
            ))
        }
        "vault_tags" => {
            let ws = arg_str(args, "workspace_id")?;
            let v = arg_i64(args, "vault_id")?;
            SelfCall::get(format!("/api/v1/workspaces/{}/vault/vaults/{v}/tags", seg(&ws)))
        }
        "vault_graph" => {
            let ws = arg_str(args, "workspace_id")?;
            let v = arg_i64(args, "vault_id")?;
            let mut path = format!("/api/v1/workspaces/{}/vault/vaults/{v}/graph", seg(&ws));
            let focus = args.get("path").and_then(Value::as_str).filter(|s| !s.is_empty());
            let mode = args
                .get("mode")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or(if focus.is_some() { "local" } else { "full" });
            path.push_str(&format!("?mode={}", seg(mode)));
            if let Some(f) = focus {
                path.push_str(&format!("&path={}", seg(f)));
            }
            if let Some(d) = args.get("depth").and_then(Value::as_u64) {
                path.push_str(&format!("&depth={d}"));
            }
            SelfCall::get(path)
        }
        "vault_okf_validate" => {
            let ws = arg_str(args, "workspace_id")?;
            let v = arg_i64(args, "vault_id")?;
            SelfCall::post(format!("/api/v1/workspaces/{}/vault/vaults/{v}/okf/validate", seg(&ws)), json!({}))
        }
        "vault_write" => {
            let ws = arg_str(args, "workspace_id")?;
            let v = arg_i64(args, "vault_id")?;
            let mut body = json!({
                "path": arg_str(args, "path")?,
                "content": args.get("content").and_then(Value::as_str).unwrap_or(""),
            });
            if let Some(h) = args.get("if_hash").and_then(Value::as_str) {
                body["if_hash"] = json!(h);
            }
            SelfCall::put(format!("/api/v1/workspaces/{}/vault/vaults/{v}/note", seg(&ws)), body)
        }
        "vault_rename" => {
            let ws = arg_str(args, "workspace_id")?;
            let v = arg_i64(args, "vault_id")?;
            let body = json!({ "from": arg_str(args, "from")?, "to": arg_str(args, "to")? });
            SelfCall::post(format!("/api/v1/workspaces/{}/vault/vaults/{v}/rename", seg(&ws)), body)
        }
        "vault_delete" => {
            let ws = arg_str(args, "workspace_id")?;
            let v = arg_i64(args, "vault_id")?;
            SelfCall::delete(format!(
                "/api/v1/workspaces/{}/vault/vaults/{v}/note?path={}",
                seg(&ws),
                seg(&arg_str(args, "path")?)
            ))
        }
        // ---- Sessions ----
        "list_sessions" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::get(format!("/api/v1/workspaces/{}/sessions", seg(&ws)))
        }
        "get_session" => {
            let id = arg_str(args, "session_id")?;
            SelfCall::get(format!("/api/v1/sessions/{}", seg(&id)))
        }
        "broadcast_message" => {
            let ws = arg_str(args, "workspace_id")?;
            let body = json!({ "text": arg_str(args, "text")? });
            SelfCall::post(format!("/api/v1/workspaces/{}/broadcast", seg(&ws)), body)
        }
        // ---- Code review / findings ----
        "list_findings" => {
            let rid = arg_str(args, "review_id")?;
            SelfCall::get(format!("/api/v1/reviews/{}/findings", seg(&rid)))
        }
        "get_finding" => {
            let id = arg_str(args, "finding_id")?;
            SelfCall::get(format!("/api/v1/findings/{}", seg(&id)))
        }
        // ---- Product ----
        "list_product_stories" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::get(format!("/api/v1/workspaces/{}/product/stories", seg(&ws)))
        }
        "get_product_story" => {
            let id = arg_str(args, "story_id")?;
            SelfCall::get(format!("/api/v1/product/stories/{}", seg(&id)))
        }
        // ---- Channels ----
        "list_integrations" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::get(format!("/api/v1/workspaces/{}/integrations", seg(&ws)))
        }
        "test_integration" => {
            let ws = arg_str(args, "workspace_id")?;
            let ch = arg_str(args, "channel")?;
            SelfCall::post(format!("/api/v1/workspaces/{}/integrations/{}/test", seg(&ws), seg(&ch)), json!({}))
        }
        // ---- Usage ----
        "get_usage_summary" => {
            let mut q: Vec<String> = Vec::new();
            if let Some(d) = args.get("days").and_then(Value::as_u64) {
                q.push(format!("days={d}"));
            }
            if let Some(o) = args.get("otto_only").and_then(Value::as_bool) {
                q.push(format!("otto_only={o}"));
            }
            let mut path = "/api/v1/usage/summary".to_string();
            if !q.is_empty() {
                path.push('?');
                path.push_str(&q.join("&"));
            }
            SelfCall::get(path)
        }
        // ---- Skills ----
        "list_bundled_skills" => SelfCall::get("/api/v1/library/bundled".to_string()),
        // ---- Self-improvement ----
        "get_self_improvement_config" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::get(format!("/api/v1/workspaces/{}/self-improvement", seg(&ws)))
        }
        "list_improvement_runs" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::get(format!("/api/v1/workspaces/{}/improvement/runs", seg(&ws)))
        }
        "get_improvement_run" => {
            let id = arg_str(args, "run_id")?;
            SelfCall::get(format!("/api/v1/improvement/runs/{}", seg(&id)))
        }
        "list_improvement_edits" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::get(format!("/api/v1/workspaces/{}/improvement/edits", seg(&ws)))
        }
        "run_self_improvement" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::post(format!("/api/v1/workspaces/{}/self-improvement/run", seg(&ws)), json!({}))
        }
        "approve_improvement_edit" => {
            let id = arg_str(args, "edit_id")?;
            SelfCall::post(format!("/api/v1/improvement/edits/{}/approve", seg(&id)), json!({}))
        }
        "reject_improvement_edit" => {
            let id = arg_str(args, "edit_id")?;
            SelfCall::post(format!("/api/v1/improvement/edits/{}/reject", seg(&id)), json!({}))
        }
        "rollback_improvement_edit" => {
            let id = arg_str(args, "edit_id")?;
            SelfCall::post(format!("/api/v1/improvement/edits/{}/rollback", seg(&id)), json!({}))
        }
        // ---- Goal loop / swarm task / scheduled tasks ----
        "run_goal_loop" => {
            let ws = arg_str(args, "workspace_id")?;
            let mut body = args.clone();
            if let Some(obj) = body.as_object_mut() {
                obj.remove("workspace_id");
                obj.insert("autostart".into(), json!(true));
            }
            SelfCall::post(format!("/api/v1/workspaces/{}/goal-loops", seg(&ws)), body)
        }
        "create_work_item" => {
            let project = arg_str(args, "project_id")?;
            let body = json!({
                "title": arg_str(args, "title")?,
                "description": args.get("description").and_then(Value::as_str),
                "priority": args.get("priority").and_then(Value::as_str),
            });
            SelfCall::post(format!("/api/v1/swarm/projects/{}/tasks", seg(&project)), body)
        }
        "list_scheduled_tasks" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::get(format!("/api/v1/workspaces/{}/scheduled-tasks", seg(&ws)))
        }
        "list_scheduled_task_runs" => {
            let id = arg_str(args, "task_id")?;
            SelfCall::get(format!("/api/v1/scheduled-tasks/{}/runs", seg(&id)))
        }
        "create_scheduled_task" => {
            let ws = arg_str(args, "workspace_id")?;
            let mut body = args.clone();
            if let Some(o) = body.as_object_mut() {
                o.remove("workspace_id");
            }
            SelfCall::post(format!("/api/v1/workspaces/{}/scheduled-tasks", seg(&ws)), body)
        }
        "update_scheduled_task" => {
            let id = arg_str(args, "task_id")?;
            let mut body = args.clone();
            if let Some(o) = body.as_object_mut() {
                o.remove("task_id");
            }
            SelfCall::patch(format!("/api/v1/scheduled-tasks/{}", seg(&id)), body)
        }
        "set_scheduled_task_enabled" => {
            let id = arg_str(args, "task_id")?;
            let enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            SelfCall::patch(format!("/api/v1/scheduled-tasks/{}", seg(&id)), json!({"enabled": enabled}))
        }
        "run_scheduled_task" => {
            let id = arg_str(args, "task_id")?;
            SelfCall::post(format!("/api/v1/scheduled-tasks/{}/run", seg(&id)), json!({}))
        }
        "delete_scheduled_task" => {
            let id = arg_str(args, "task_id")?;
            SelfCall::delete(format!("/api/v1/scheduled-tasks/{}", seg(&id)))
        }
        other => return Err(Error::Invalid(format!("unknown otto tool '{other}'"))),
    })
}

/// Resolve `(tool, args)` to a self-call and execute it as the user. Thin wrapper
/// over the pure [`route_for`] so the routing of every tool is unit-tested.
async fn run_tool(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    tool: &str,
    args: &Value,
) -> Result<Value, Error> {
    let call = route_for(tool, args)?;
    let url = format!("{base}{}", call.path);
    let empty = json!({});
    let body = call.body.as_ref().unwrap_or(&empty);
    match call.method {
        Method::Get => self_get(client, token, &url).await,
        Method::Post => self_post(client, token, &url, body).await,
        Method::Put => self_put(client, token, &url, body).await,
        Method::Patch => self_patch(client, token, &url, body).await,
        Method::Delete => self_delete(client, token, &url).await,
    }
}

async fn self_get(client: &reqwest::Client, token: &str, url: &str) -> Result<Value, Error> {
    let resp = client.get(url).bearer_auth(token).send().await
        .map_err(|e| Error::Upstream(format!("self-call: {e}")))?;
    parse_self(resp).await
}
async fn self_post(client: &reqwest::Client, token: &str, url: &str, body: &Value) -> Result<Value, Error> {
    let resp = client.post(url).bearer_auth(token).json(body).send().await
        .map_err(|e| Error::Upstream(format!("self-call: {e}")))?;
    parse_self(resp).await
}
async fn self_put(client: &reqwest::Client, token: &str, url: &str, body: &Value) -> Result<Value, Error> {
    let resp = client.put(url).bearer_auth(token).json(body).send().await
        .map_err(|e| Error::Upstream(format!("self-call: {e}")))?;
    parse_self(resp).await
}
async fn self_patch(client: &reqwest::Client, token: &str, url: &str, body: &Value) -> Result<Value, Error> {
    let resp = client.patch(url).bearer_auth(token).json(body).send().await
        .map_err(|e| Error::Upstream(format!("self-call: {e}")))?;
    parse_self(resp).await
}
async fn self_delete(client: &reqwest::Client, token: &str, url: &str) -> Result<Value, Error> {
    let resp = client.delete(url).bearer_auth(token).send().await
        .map_err(|e| Error::Upstream(format!("self-call: {e}")))?;
    parse_self(resp).await
}
async fn parse_self(resp: reqwest::Response) -> Result<Value, Error> {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        let snippet: String = text.chars().take(400).collect();
        return Err(Error::Upstream(format!("{status}: {snippet}")));
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
}

async fn ask_human_approval(ctx: &ServerCtx, user: &otto_core::domain::User, args: &Value) -> Result<Value, Error> {
    let title = arg_str(args, "title")?;
    let ws = args.get("workspace_id").and_then(Value::as_str).map(str::to_string);
    let detail = args.get("detail").and_then(Value::as_str).map(str::to_string);
    let appr = ctx
        .mcp
        .approvals()
        .create(NewApproval {
            workspace_id: ws,
            kind: "human_ask".into(),
            server_id: None,
            server_name: Some("otto".into()),
            tool: Some("otto.ask_human_approval".into()),
            title,
            detail,
            args_redacted_json: otto_core::redact::redact_json(args).value.to_string(),
            args_hash: None,
            risk_label: None,
            requested_by: Some(user.id.clone()),
            requested_by_kind: Some("mcp_server".into()),
            expires_at: Some((chrono::Utc::now() + chrono::Duration::hours(24)).to_rfc3339()),
        })
        .await?;
    let wait = args.get("wait_seconds").and_then(Value::as_u64);
    let decided = wait_for_decision(ctx, &appr.id, wait).await;
    Ok(json!({
        "approval_id": appr.id,
        "status": match decided { Some(true) => "approved", Some(false) => "denied", None => "pending" },
        "note": "poll the MCP approvals queue, or pass wait_seconds (≤30) to block briefly",
    }))
}

// ===========================================================================
// GET / PATCH /mcp/otto-server  (status + config + token mint)
// ===========================================================================

pub async fn otto_server_status(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    let enabled = outward_enabled(&ctx).await;
    let on = enabled_tools(&ctx).await;
    let tools: Vec<Value> = otto_tool_specs()
        .into_iter()
        .map(|t| {
            let name = t["name"].as_str().unwrap_or("").to_string();
            let short = name.strip_prefix("otto.").unwrap_or(&name).to_string();
            json!({
                "name": name,
                "description": t["description"],
                "mutating": t["mutating"],
                "category": t["category"],
                "enabled": on.contains(&short),
            })
        })
        .collect();
    let prefix = AuthRepo::new(ctx.pool.clone()).mcp_token_prefix(&user.id).await.map_err(ApiError)?;
    Ok(Json(json!({
        "enabled": enabled,
        "tools": tools,
        "has_token": prefix.is_some(),
        "token_prefix": prefix,
    })))
}

#[derive(Deserialize)]
pub struct OttoServerConfigReq {
    pub enabled: Option<bool>,
    pub tools: Option<Vec<String>>,
    #[serde(default)]
    pub rotate_token: bool,
}

pub async fn otto_server_config(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<OttoServerConfigReq>,
) -> ApiResult<Json<Value>> {
    let settings = SettingsRepo::new(ctx.pool.clone());
    if let Some(en) = req.enabled {
        settings.put("mcp_otto_server_enabled", &json!(en)).await.map_err(ApiError)?;
    }
    if let Some(tools) = &req.tools {
        let known: Vec<String> = otto_tool_specs()
            .iter()
            .filter_map(|t| t["name"].as_str().map(|n| n.strip_prefix("otto.").unwrap_or(n).to_string()))
            .collect();
        // The UI sends full `otto.*` names; the read path (`enabled_tools`) keys on
        // the bare name. Accept either form, validate + STORE the bare name so the
        // stored set matches what the dispatcher/status compare against.
        let mut normalized: Vec<String> = Vec::with_capacity(tools.len());
        for t in tools {
            let bare = t.strip_prefix("otto.").unwrap_or(t).to_string();
            if !known.contains(&bare) {
                return Err(ApiError(Error::Invalid(format!("unknown otto tool '{t}'"))));
            }
            normalized.push(bare);
        }
        settings.put("mcp_otto_server_tools", &json!(normalized)).await.map_err(ApiError)?;
    }
    let mut minted: Option<String> = None;
    if req.rotate_token {
        let repo = AuthRepo::new(ctx.pool.clone());
        repo.revoke_mcp_tokens(&user.id).await.map_err(ApiError)?;
        minted = Some(repo.issue_mcp_token(&user.id, Some("otto-mcp-server")).await.map_err(ApiError)?);
        ctx.audit(otto_state::NewAuditEntry {
            user_id: Some(user.id.clone()),
            action: "mcp.otto_server.token_mint".into(),
            target: None,
            detail: None,
            ip: None,
        })
        .await;
    }
    let mut status = otto_server_status(State(ctx), CurrentUser(user)).await?;
    if let Some(tok) = minted {
        status.0["token"] = json!(tok);
    }
    Ok(status)
}

// ===========================================================================
// MCP `tools/list` projection (scope-aware) — shared by the HTTP transport.
// ===========================================================================

/// The MCP `tools/list` payload for a caller, as `[{name, description,
/// inputSchema}]`. A tool appears iff it is in the server's globally-enabled set
/// AND permitted by the caller's per-token [`McpScope`] (`None` ⇒ no per-token
/// narrowing — a normal token sees every enabled tool). This is what makes a
/// read-only token never even *see* a mutating tool, and a workspace/tool-scoped
/// token see only its slice. Listing is independent of the master on/off switch
/// (so clients can introspect); execution still honours it in [`governed_invoke`].
pub(crate) async fn mcp_tools_list(ctx: &ServerCtx, scope: Option<&McpScope>) -> Vec<Value> {
    let enabled = enabled_tools(ctx).await;
    otto_tool_specs()
        .into_iter()
        .filter(|s| {
            let name = s.get("name").and_then(Value::as_str).unwrap_or("");
            let bare = name.strip_prefix("otto.").unwrap_or(name);
            if !enabled.iter().any(|e| e == bare) {
                return false;
            }
            match scope {
                None => true,
                Some(sc) => sc.deny_reason(bare, tool_is_mutating(bare), None).is_none(),
            }
        })
        .map(|s| json!({"name": s["name"], "description": s["description"], "inputSchema": s["inputSchema"]}))
        .collect()
}

// ===========================================================================
// MCP token management — multiple scoped tokens per user (the access layer).
//   GET    /mcp/tokens        list all (admin)
//   POST   /mcp/tokens        mint a scoped token (admin)
//   DELETE /mcp/tokens/{id}   revoke one (admin)
// ===========================================================================

pub async fn list_mcp_tokens(
    State(ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
) -> ApiResult<Json<Value>> {
    let tokens = AuthRepo::new(ctx.pool.clone())
        .list_mcp_tokens()
        .await
        .map_err(ApiError)?;
    Ok(Json(json!({ "tokens": tokens })))
}

pub async fn create_mcp_token(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CreateMcpTokenReq>,
) -> ApiResult<Json<Value>> {
    // Owner: defaults to the caller. Minting a token for ANOTHER user hands out a
    // credential that authenticates AS that user, so only root may do it (a non-root
    // mcp:admin minting a root-owned token would otherwise be a privilege
    // escalation — mirrors the impersonation "no minting up/sideways" rule).
    let owner = req.user_id.clone().unwrap_or_else(|| user.id.clone());
    if owner != user.id && !user.is_root {
        return Err(ApiError(Error::Forbidden(
            "only root may mint an MCP token owned by another user".into(),
        )));
    }
    // Validate the scope's tool list against the live catalog so a typo can't
    // silently produce a token that reaches nothing / drifts from the UI.
    let scope = req.scope.clone().unwrap_or_else(McpScope::unrestricted);
    if let Some(tools) = &scope.tools {
        let known: Vec<String> = otto_tool_specs()
            .iter()
            .filter_map(|t| t["name"].as_str().map(|n| n.strip_prefix("otto.").unwrap_or(n).to_string()))
            .collect();
        for t in tools {
            let bare = t.strip_prefix("otto.").unwrap_or(t);
            if !known.iter().any(|k| k == bare) {
                return Err(ApiError(Error::Invalid(format!("unknown otto tool '{t}'"))));
            }
        }
    }
    // Normalize tool names to bare form so enforcement (which keys on bare names)
    // matches regardless of whether the UI sent `otto.x` or `x`.
    let scope = McpScope {
        tools: scope.tools.map(|ts| {
            ts.iter()
                .map(|t| t.strip_prefix("otto.").unwrap_or(t).to_string())
                .collect()
        }),
        allow_writes: scope.allow_writes,
        workspace_id: scope.workspace_id.filter(|w| !w.is_empty()),
    };
    let (token, info) = AuthRepo::new(ctx.pool.clone())
        .issue_mcp_token_with_scope(&owner, req.label.as_deref(), &scope)
        .await
        .map_err(ApiError)?;
    ctx.audit(otto_state::NewAuditEntry {
        user_id: Some(user.id.clone()),
        action: "mcp.token.create".into(),
        target: Some(info.id.clone()),
        detail: Some(json!({ "owner": owner, "allow_writes": scope.allow_writes })),
        ip: None,
    })
    .await;
    Ok(Json(json!({ "token": token, "info": info })))
}

pub async fn revoke_mcp_token(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> ApiResult<axum::http::StatusCode> {
    // Use the SHARED auth cache so the revocation takes effect immediately: the
    // authenticator caches mcp tokens by hash, and `revoke_mcp_token_by_id`
    // evicts the owner from this same cache once it knows who owned the token.
    let removed = AuthRepo::with_cache(ctx.pool.clone(), ctx.auth_cache.clone())
        .revoke_mcp_token_by_id(&id)
        .await
        .map_err(ApiError)?;
    if !removed {
        return Err(ApiError(Error::NotFound("mcp token not found".into())));
    }
    ctx.audit(otto_state::NewAuditEntry {
        user_id: Some(user.id.clone()),
        action: "mcp.token.revoke".into(),
        target: Some(id),
        detail: None,
        ip: None,
    })
    .await;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// ===========================================================================
// Gateway — governs LIVE-AGENT downstream calls through the same pipeline.
// ===========================================================================

#[derive(Deserialize)]
pub struct GatewayToolsQuery {
    pub workspace_id: String,
}

/// `GET /mcp/gateway/tools?workspace_id=` — the governed downstream tools for a
/// workspace, namespaced `mcp__<server>__<tool>`. The inward `ottod mcp-tools`
/// surfaces these to the agent and proxies each call through `/mcp/gateway/invoke`.
pub async fn gateway_tools(
    State(ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
    Query(q): Query<GatewayToolsQuery>,
) -> ApiResult<Json<Value>> {
    let servers = ctx.mcp.registry().list_for_ws(&q.workspace_id).await.map_err(ApiError)?;
    let mut tools: Vec<Value> = Vec::new();
    for s in servers.into_iter().filter(|s| s.enabled && s.managed) {
        for t in ctx.mcp.tools().list_for_server(&s.id).await.map_err(ApiError)?.into_iter().filter(|t| t.enabled) {
            tools.push(json!({
                "name": format!("mcp__{}__{}", s.name, t.name),
                "server_id": s.id,
                "server_name": s.name,
                "tool": t.name,
                "description": t.description,
                "inputSchema": t.input_schema,
                "risk_label": t.risk_label,
            }));
        }
    }
    Ok(Json(json!({ "tools": tools })))
}

#[derive(Deserialize)]
pub struct GatewayInvokeReq {
    pub server_id: Id,
    pub tool: String,
    #[serde(default)]
    pub arguments: Value,
    #[serde(default)]
    pub dry_run: bool,
    pub workspace_id: String,
    #[serde(default)]
    pub session_id: Option<String>,
}

/// `POST /mcp/gateway/invoke` — run a downstream call through the SAME governance
/// pipeline (allowlist→policy→approval→dry-run→execute→audit), tagged
/// `caller_kind='gateway'`. This is what puts the control plane in the path of a
/// live agent's every downstream MCP call.
pub async fn gateway_invoke(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<GatewayInvokeReq>,
) -> ApiResult<Json<Value>> {
    let _ = &req.session_id;
    let ictx = InvokeCtx {
        workspace_id: Some(req.workspace_id.clone()),
        dry_run: req.dry_run,
        caller_user_id: Some(user.id.clone()),
        caller_kind: "gateway".into(),
        direction: "outbound".into(),
    };
    let outcome = ctx.mcp.invoke(&req.server_id, &req.tool, &req.arguments, &ictx).await.map_err(ApiError)?;
    let resp = otto_mcp::outcome_to_resp(outcome);
    if resp.decision == "pending_approval" {
        let _ = ctx.events.send(otto_core::event::Event::Notice {
            level: "warn".into(),
            title: "MCP approval needed".into(),
            body: format!("A governed MCP tool '{}' is awaiting approval.", req.tool),
        });
    }
    Ok(Json(serde_json::to_value(resp).unwrap_or(Value::Null)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_names() -> Vec<String> {
        otto_tool_specs()
            .iter()
            .filter_map(|t| t["name"].as_str().map(String::from))
            .collect()
    }

    #[test]
    fn internal_reviewer_scope_does_not_depend_on_outward_server_toggle() {
        assert!(mcp_tool_enabled_for_token(true, false, &[], "vault_read"));
        assert!(!mcp_tool_enabled_for_token(false, false, &[], "vault_read"));
        assert!(mcp_tool_enabled_for_token(
            false,
            true,
            &["vault_read".to_string()],
            "vault_read",
        ));
    }

    #[test]
    fn scheduled_task_tools_are_registered() {
        let names = spec_names();
        for n in [
            "otto.list_scheduled_tasks",
            "otto.list_scheduled_task_runs",
            "otto.create_scheduled_task",
            "otto.update_scheduled_task",
            "otto.set_scheduled_task_enabled",
            "otto.run_scheduled_task",
            "otto.delete_scheduled_task",
        ] {
            assert!(names.contains(&n.to_string()), "missing spec {n}");
        }
    }

    #[test]
    fn write_tools_are_dangerous_reads_are_default_enabled() {
        for w in [
            "create_scheduled_task",
            "update_scheduled_task",
            "delete_scheduled_task",
            "run_scheduled_task",
            "set_scheduled_task_enabled",
        ] {
            assert!(DANGEROUS.contains(&w), "{w} must be DANGEROUS");
            assert!(!DEFAULT_ENABLED.contains(&w), "{w} must be off by default");
        }
        assert!(DEFAULT_ENABLED.contains(&"list_scheduled_tasks"));
        assert!(DEFAULT_ENABLED.contains(&"list_scheduled_task_runs"));
    }

    #[test]
    fn create_tool_is_marked_mutating() {
        let specs = otto_tool_specs();
        let create = specs
            .iter()
            .find(|t| t["name"] == "otto.create_scheduled_task")
            .unwrap();
        assert_eq!(create["mutating"], serde_json::json!(true));
    }

    #[test]
    fn dangerous_detail_surfaces_cadence_and_destination() {
        let args = serde_json::json!({
            "name": "Nightly",
            "schedule": {"cadence": "interval", "every_min": 60},
            "destination": {"type": "slack"},
            "prompt": "do the thing"
        });
        let d = dangerous_detail("otto.create_scheduled_task", &args);
        assert!(d.contains("Nightly"));
        assert!(d.contains("every 60 min"));
        assert!(d.contains("slack"));
        assert!(d.contains("do the thing"));
    }

    // ----- All-features expansion -----------------------------------------

    /// (bare short name, mutating) for every spec.
    fn spec_short_mut() -> Vec<(String, bool)> {
        otto_tool_specs()
            .iter()
            .map(|t| {
                let name = t["name"].as_str().unwrap();
                let short = name.strip_prefix("otto.").unwrap_or(name).to_string();
                (short, t["mutating"].as_bool().unwrap())
            })
            .collect()
    }

    #[test]
    fn every_spec_is_well_formed_and_classified() {
        let specs = otto_tool_specs();
        for (short, mutating) in spec_short_mut() {
            let t = specs
                .iter()
                .find(|s| s["name"].as_str().unwrap().strip_prefix("otto.").unwrap() == short)
                .unwrap();
            // category present + non-empty (drives the control-plane UI grouping).
            assert!(
                t["category"].as_str().map(|c| !c.is_empty()).unwrap_or(false),
                "{short} missing category"
            );
            // inputSchema is an object; every declared `required` key exists in `properties`.
            assert_eq!(t["inputSchema"]["type"], json!("object"), "{short} schema not an object");
            if let Some(reqd) = t["inputSchema"]["required"].as_array() {
                for r in reqd {
                    let key = r.as_str().unwrap();
                    assert!(
                        t["inputSchema"]["properties"].get(key).is_some(),
                        "{short}: required '{key}' missing from properties"
                    );
                }
            }
            // Classification invariant: mutating ⟺ DANGEROUS; reads are default-on XOR opt-in.
            let s = short.as_str();
            if mutating {
                assert!(DANGEROUS.contains(&s), "{short} is mutating but not DANGEROUS");
                assert!(!DEFAULT_ENABLED.contains(&s), "{short} is mutating but default-enabled");
            } else {
                let de = DEFAULT_ENABLED.contains(&s);
                let opt = OPT_IN_READS.contains(&s);
                assert!(de ^ opt, "{short} (read) must be default-enabled XOR opt-in (de={de}, opt={opt})");
                assert!(!DANGEROUS.contains(&s), "{short} (read) must not be DANGEROUS");
            }
        }
    }

    #[test]
    fn classification_lists_reference_real_tools() {
        let shorts: std::collections::HashSet<String> =
            spec_short_mut().into_iter().map(|(s, _)| s).collect();
        for n in DEFAULT_ENABLED.iter().chain(DANGEROUS.iter()).chain(OPT_IN_READS.iter()) {
            assert!(shorts.contains(*n), "classification names a non-existent tool '{n}'");
        }
    }

    #[test]
    fn headline_features_present_and_governed() {
        let names = spec_names();
        for n in [
            "otto.list_workflows",
            "otto.get_workflow_run",
            "otto.run_workflow",
            "otto.cancel_workflow_run",
            "otto.list_broker_clusters",
            "otto.list_broker_topics",
            "otto.consume_broker_messages",
            "otto.produce_broker_message",
        ] {
            assert!(names.contains(&n.to_string()), "missing headline spec {n}");
        }
        assert!(DEFAULT_ENABLED.contains(&"list_workflows"));
        assert!(DEFAULT_ENABLED.contains(&"list_broker_clusters"));
        assert!(DANGEROUS.contains(&"run_workflow"));
        assert!(DANGEROUS.contains(&"produce_broker_message"));
        // Content-heavy reads stay off by default.
        assert!(!DEFAULT_ENABLED.contains(&"consume_broker_messages"));
        assert!(!DEFAULT_ENABLED.contains(&"search_memory"));
    }

    #[test]
    fn route_for_maps_workflows_and_brokers() {
        assert_eq!(
            route_for("list_workflows", &json!({"workspace_id":"ws1"})).unwrap(),
            SelfCall { method: Method::Get, path: "/api/v1/workspaces/ws1/workflows".into(), body: None }
        );
        let c = route_for("run_workflow", &json!({"workflow_id":"wf1","input":{"k":1},"start_node":"n2"})).unwrap();
        assert_eq!(c.method, Method::Post);
        assert_eq!(c.path, "/api/v1/workflows/wf1/run");
        assert_eq!(c.body.unwrap(), json!({"input":{"k":1},"start_node":"n2"}));
        assert_eq!(
            route_for("cancel_workflow_run", &json!({"run_id":"r1"})).unwrap(),
            SelfCall { method: Method::Post, path: "/api/v1/workflow-runs/r1/cancel".into(), body: Some(json!({})) }
        );
        assert_eq!(
            route_for("get_broker_topic", &json!({"cluster_id":"c1","topic":"orders"})).unwrap().path,
            "/api/v1/brokers/clusters/c1/topics/orders"
        );
        let c = route_for("produce_broker_message", &json!({"cluster_id":"c1","topic":"orders","value":"hi","key":"k","confirm":true})).unwrap();
        assert_eq!(c.path, "/api/v1/brokers/clusters/c1/topics/orders/produce");
        assert_eq!(c.body.unwrap(), json!({"value":"hi","key":"k","confirm":true}));
        let c = route_for("consume_broker_messages", &json!({"cluster_id":"c1","topic":"orders","limit":10,"value_filter":"x"})).unwrap();
        assert_eq!(c.path, "/api/v1/brokers/clusters/c1/topics/orders/consume");
        assert_eq!(c.body.unwrap(), json!({"limit":10,"value_filter":"x"}));
    }

    #[test]
    fn route_for_maps_git_issues_swarm_memory_usage() {
        assert_eq!(route_for("get_pr", &json!({"repo_id":"r1","number":7})).unwrap().path, "/api/v1/repos/r1/prs/7");
        let c = route_for("create_pr", &json!({"repo_id":"r1","title":"T","description":"D","source_branch":"feat","target_branch":"main"})).unwrap();
        assert_eq!(c.path, "/api/v1/repos/r1/prs");
        assert_eq!(c.body.unwrap(), json!({"title":"T","description":"D","source_branch":"feat","target_branch":"main"}));
        assert_eq!(route_for("list_prs", &json!({"repo_id":"r1","state":"open"})).unwrap().path, "/api/v1/repos/r1/prs?state=open");

        let c = route_for("search_issues", &json!({"account_id":"a1","query":"a = b","project":"X"})).unwrap();
        assert!(c.path.starts_with("/api/v1/issue/search?account_id=a1"));
        assert!(c.path.contains("&q=a%20%3D%20b"), "got {}", c.path);
        assert!(c.path.contains("&project=X"));
        let c = route_for("transition_issue", &json!({"account_id":"a1","key":"K-1","transition_id":"21"})).unwrap();
        assert_eq!(c.path, "/api/v1/issue/a1/K-1/transitions");
        assert_eq!(c.body.unwrap(), json!({"transition_id":"21"}));

        let c = route_for("post_swarm_board", &json!({"swarm_id":"s1","body":"hello","project_id":"p1"})).unwrap();
        assert_eq!(c.path, "/api/v1/swarm/swarms/s1/board");
        assert_eq!(c.body.unwrap(), json!({"body":"hello","project_id":"p1"}));

        let c = route_for("search_memory", &json!({"workspace_id":"ws1","query":"schema","k":5})).unwrap();
        assert_eq!(c.path, "/api/v1/workspaces/ws1/memory/search");
        assert_eq!(c.body.unwrap(), json!({"text":"schema","k":5}));
        assert_eq!(route_for("list_memory", &json!({"workspace_id":"ws1","collection":"vault"})).unwrap().path, "/api/v1/workspaces/ws1/memories?collection=vault");

        assert_eq!(route_for("get_usage_summary", &json!({"days":7})).unwrap().path, "/api/v1/usage/summary?days=7");
        assert_eq!(route_for("get_usage_summary", &json!({})).unwrap().path, "/api/v1/usage/summary");
        assert_eq!(
            route_for("list_bundled_skills", &json!({})).unwrap(),
            SelfCall { method: Method::Get, path: "/api/v1/library/bundled".into(), body: None }
        );
        assert_eq!(route_for("list_findings", &json!({"review_id":"rv1"})).unwrap().path, "/api/v1/reviews/rv1/findings");
        assert_eq!(route_for("broadcast_message", &json!({"workspace_id":"ws1","text":"hi"})).unwrap().body.unwrap(), json!({"text":"hi"}));
        assert_eq!(route_for("test_integration", &json!({"workspace_id":"ws1","channel":"slack"})).unwrap().path, "/api/v1/workspaces/ws1/integrations/slack/test");
    }

    #[test]
    fn route_for_rejects_missing_args_and_unknown_tool() {
        assert!(route_for("list_workflows", &json!({})).is_err());
        assert!(route_for("get_pr", &json!({"repo_id":"r1"})).is_err()); // missing integer `number`
        assert!(route_for("create_pr", &json!({"repo_id":"r1","title":"T"})).is_err());
        assert!(route_for("transition_issue", &json!({"account_id":"a1","key":"K"})).is_err());
        assert!(route_for("frobnicate", &json!({})).is_err());
    }

    #[test]
    fn query_db_readonly_sql_guard_lives_in_route_for() {
        assert!(route_for("query_db_readonly", &json!({"connection_id":"c1","statement":"SELECT 1"})).is_ok());
        assert!(route_for("query_db_readonly", &json!({"connection_id":"c1","statement":"DELETE FROM t"})).is_err());
        assert!(route_for("query_db_readonly", &json!({"connection_id":"c1","statement":"SELECT 1; DROP TABLE t"})).is_err());
    }

    #[test]
    fn self_improvement_tools_present_classified_and_routed() {
        let names = spec_names();
        for n in [
            "otto.list_improvement_runs",
            "otto.list_improvement_edits",
            "otto.approve_improvement_edit",
            "otto.reject_improvement_edit",
            "otto.rollback_improvement_edit",
            "otto.run_self_improvement",
        ] {
            assert!(names.contains(&n.to_string()), "missing self-improvement spec {n}");
        }
        assert!(DEFAULT_ENABLED.contains(&"list_improvement_edits"));
        assert!(DANGEROUS.contains(&"approve_improvement_edit"));
        assert!(DANGEROUS.contains(&"reject_improvement_edit"));
        assert!(DANGEROUS.contains(&"rollback_improvement_edit"));
        assert_eq!(
            route_for("list_improvement_edits", &json!({"workspace_id":"ws1"})).unwrap().path,
            "/api/v1/workspaces/ws1/improvement/edits"
        );
        assert_eq!(
            route_for("approve_improvement_edit", &json!({"edit_id":"e1"})).unwrap(),
            SelfCall { method: Method::Post, path: "/api/v1/improvement/edits/e1/approve".into(), body: Some(json!({})) }
        );
        assert_eq!(route_for("reject_improvement_edit", &json!({"edit_id":"e1"})).unwrap().path, "/api/v1/improvement/edits/e1/reject");
        assert_eq!(route_for("rollback_improvement_edit", &json!({"edit_id":"e1"})).unwrap().path, "/api/v1/improvement/edits/e1/rollback");
        assert!(dangerous_detail("otto.approve_improvement_edit", &json!({"edit_id":"e9"})).contains("e9"));
    }

    #[test]
    fn dangerous_detail_surfaces_new_tool_targets() {
        assert!(dangerous_detail("otto.run_workflow", &json!({"workflow_id":"wf-9"})).contains("wf-9"));
        assert!(dangerous_detail("otto.produce_broker_message", &json!({"topic":"orders","cluster_id":"c1"})).contains("orders"));
        let d = dangerous_detail("otto.create_pr", &json!({"repo_id":"r1","title":"Fix","source_branch":"f","target_branch":"main"}));
        assert!(d.contains("Fix") && d.contains("main"));
        assert!(dangerous_detail("otto.broadcast_message", &json!({"text":"hello team"})).contains("hello team"));
    }

    // ----- Vault: optional workspace_id ------------------------------------

    #[test]
    fn vault_tools_do_not_require_workspace_id() {
        // Vaults are a global library: `workspace_id` is an optional hint on
        // every vault tool (`fill_vault_workspace` scopes omitted calls
        // server-side), while vault addressing stays strict via `vault_id`.
        for t in otto_tool_specs() {
            if t["category"] != json!("Vault") {
                continue;
            }
            let name = t["name"].as_str().unwrap();
            let reqd = t["inputSchema"]["required"].as_array().unwrap();
            assert!(
                !reqd.iter().any(|r| r == "workspace_id"),
                "{name} must not require workspace_id"
            );
            assert!(
                t["inputSchema"]["properties"].get("workspace_id").is_some(),
                "{name} lost the workspace_id property"
            );
            if name != "otto.vault_list" {
                assert!(reqd.iter().any(|r| r == "vault_id"), "{name} must require vault_id");
            }
        }
    }

    #[test]
    fn pick_vault_workspace_prefers_writable_for_mutating_tools() {
        fn ws(id: &str) -> otto_core::domain::Workspace {
            otto_core::domain::Workspace {
                id: id.into(),
                name: id.into(),
                root_path: format!("/tmp/{id}"),
                settings: json!({}),
                archived: false,
                created_at: chrono::Utc::now(),
            }
        }
        let rows = vec![
            (ws("view-only"), WorkspaceRole::Viewer),
            (ws("editable"), WorkspaceRole::Editor),
        ];
        // Reads take the first accessible workspace; writes skip ahead to the
        // first Editor+ membership.
        assert_eq!(pick_vault_workspace(&rows, false).as_deref(), Some("view-only"));
        assert_eq!(pick_vault_workspace(&rows, true).as_deref(), Some("editable"));
        // Viewer-only memberships still resolve for a mutating tool — the
        // self-call's native RBAC owns the denial.
        let viewer_only = vec![(ws("view-only"), WorkspaceRole::Viewer)];
        assert_eq!(pick_vault_workspace(&viewer_only, true).as_deref(), Some("view-only"));
        assert_eq!(pick_vault_workspace(&[], false), None);
    }
}
