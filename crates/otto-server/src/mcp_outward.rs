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
use otto_core::{Error, Id};
use otto_mcp::{canonical_hash, InvokeCtx};
use otto_rbac::AuthRepo;
use otto_state::{NewApproval, NewCallLog, SettingsRepo};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::CurrentUser;
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
    // Memory / sessions
    "list_memory",
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
    // Vault v2 (structural reads — no large content)
    "vault_list_repos",
    "vault_search_symbols",
    "vault_code_graph",
    "vault_node_neighborhood",
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
    // Vault v2 writes (filesystem scan / knowledge writes / host install)
    "vault_index_repo",
    "vault_ingest_text",
    "vault_upsert_doc",
    "vault_install_backend",
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
    "search_memory",
    // Vault v2 content-streaming reads.
    "vault_brain",
    "vault_full_graph",
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
            "description":"Post a comment on a pull request. DANGEROUS: outward-facing — approval-gated.",
            "inputSchema":{"type":"object","required":["repo_id","number","body"],"properties":{
                "repo_id":{"type":"string"},"number":{"type":"integer"},"body":{"type":"string"}}}}),
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

        // ================= Memory / Vault =================
        json!({"name":"otto.list_memory","mutating":false,"category":"Memory",
            "description":"List a workspace's vault memories (knowledge items). Optional `collection`/`story_id` filters. Read-only.",
            "inputSchema":{"type":"object","required":["workspace_id"],"properties":{
                "workspace_id":{"type":"string"},"collection":{"type":"string"},"story_id":{"type":"string"}}}}),
        json!({"name":"otto.search_memory","mutating":false,"category":"Memory",
            "description":"Semantic/keyword search of a workspace's vault for a free-text query; returns the top hits. Off by default (streams stored content).",
            "inputSchema":{"type":"object","required":["workspace_id","query"],"properties":{
                "workspace_id":{"type":"string"},"query":{"type":"string"},"k":{"type":"integer"}}}}),

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

        // ================= Vault v2 (code intelligence) =================
        json!({"name":"otto.vault_list_repos","mutating":false,"category":"Vault",
            "description":"List repositories indexed into the Vault for a workspace (with symbol/edge/chunk counts + status). Read-only.",
            "inputSchema":{"type":"object","required":["workspace_id"],"properties":{"workspace_id":{"type":"string"}}}}),
        json!({"name":"otto.vault_search_symbols","mutating":false,"category":"Vault",
            "description":"Search the Vault's tree-sitter symbol index by name substring; returns name/kind/file:line/signature. Read-only.",
            "inputSchema":{"type":"object","required":["workspace_id","query"],"properties":{
                "workspace_id":{"type":"string"},"query":{"type":"string"},"repo_id":{"type":"string"},"limit":{"type":"integer"}}}}),
        json!({"name":"otto.vault_code_graph","mutating":false,"category":"Vault",
            "description":"Get the code dependency graph (nodes + typed edges: calls/imports/http_call/db_call/test_of/documents) for a workspace, optionally scoped to one repo. Read-only.",
            "inputSchema":{"type":"object","required":["workspace_id"],"properties":{
                "workspace_id":{"type":"string"},"repo_id":{"type":"string"}}}}),
        json!({"name":"otto.vault_node_neighborhood","mutating":false,"category":"Vault",
            "description":"Breadth-first dependency neighborhood (subgraph) around a graph node id, up to `depth` hops. Read-only.",
            "inputSchema":{"type":"object","required":["workspace_id","node_id"],"properties":{
                "workspace_id":{"type":"string"},"node_id":{"type":"string"},"depth":{"type":"integer"}}}}),
        json!({"name":"otto.vault_brain","mutating":false,"category":"Vault",
            "description":"Assemble the Repo Brain for a focus: relevant knowledge/docs, symbols, the dependency neighborhood, and git/test context — each annotated with WHY it was selected. Streams recalled content (off by default).",
            "inputSchema":{"type":"object","required":["workspace_id","focus"],"properties":{
                "workspace_id":{"type":"string"},"focus":{"type":"string"},"cwd":{"type":"string"},"budget":{"type":"integer"}}}}),
        json!({"name":"otto.vault_full_graph","mutating":false,"category":"Vault",
            "description":"The unified Vault graph (knowledge memories + code dependency graph) for the full graph view. Read-only; can be large (off by default).",
            "inputSchema":{"type":"object","required":["workspace_id"],"properties":{
                "workspace_id":{"type":"string"},"repo_id":{"type":"string"}}}}),
        json!({"name":"otto.vault_index_repo","mutating":true,"category":"Vault",
            "description":"Scan a repository on disk into the Vault: tree-sitter symbols + dependency graph + embeddings. DANGEROUS: reads arbitrary filesystem paths and writes the index — approval-gated.",
            "inputSchema":{"type":"object","required":["workspace_id","root"],"properties":{
                "workspace_id":{"type":"string"},"root":{"type":"string"},"name":{"type":"string"}}}}),
        json!({"name":"otto.vault_ingest_text","mutating":true,"category":"Vault",
            "description":"Chunk + embed text into a Vault collection (default `code`). DANGEROUS: writes to the knowledge store — approval-gated.",
            "inputSchema":{"type":"object","required":["workspace_id","path","content"],"properties":{
                "workspace_id":{"type":"string"},"collection":{"type":"string"},"path":{"type":"string"},"content":{"type":"string"}}}}),
        json!({"name":"otto.vault_upsert_doc","mutating":true,"category":"Vault",
            "description":"Create/refresh a documentation note and link it into the code dependency graph (a `doc` node with `documents` edges to code node ids). DANGEROUS: writes — approval-gated.",
            "inputSchema":{"type":"object","required":["workspace_id","title","body"],"properties":{
                "workspace_id":{"type":"string"},"repo_id":{"type":"string"},"title":{"type":"string"},"body":{"type":"string"},
                "documents":{"type":"array","items":{"type":"string"}}}}}),
        json!({"name":"otto.vault_install_backend","mutating":true,"category":"Vault",
            "description":"Install a remote Vault backend locally (kind = qdrant|surreal|ollama) via Docker/Homebrew. DANGEROUS: changes the host — approval-gated.",
            "inputSchema":{"type":"object","required":["workspace_id","kind"],"properties":{
                "workspace_id":{"type":"string"},"kind":{"type":"string"}}}}),
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

async fn require_approval_dangerous(ctx: &ServerCtx) -> bool {
    SettingsRepo::new(ctx.pool.clone())
        .get("mcp_require_approval_dangerous")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
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

pub async fn otto_tools_invoke(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<OttoInvokeReq>,
) -> ApiResult<Json<Value>> {
    let short = req.tool.strip_prefix("otto.").unwrap_or(&req.tool).to_string();
    let mut audit = NewCallLog {
        tool: req.tool.clone(),
        direction: "inbound".into(),
        server_name: Some("otto".into()),
        caller_user_id: Some(user.id.clone()),
        caller_kind: Some("mcp_server".into()),
        args_redacted_json: otto_core::redact::redact_json(&req.arguments).value.to_string(),
        ..Default::default()
    };

    if !outward_enabled(&ctx).await {
        return Ok(Json(deny_audit(&ctx, &mut audit, "the Otto MCP server is disabled").await));
    }
    if !enabled_tools(&ctx).await.contains(&short) {
        return Ok(Json(deny_audit(&ctx, &mut audit, "this tool is not enabled on the Otto MCP server").await));
    }

    let dangerous = DANGEROUS.contains(&short.as_str());
    let needs_approval = dangerous && require_approval_dangerous(&ctx).await;
    let args_hash = canonical_hash(&req.arguments);
    let ws = req.arguments.get("workspace_id").and_then(Value::as_str).map(str::to_string);

    if needs_approval && !req.dry_run {
        match ctx
            .mcp
            .approvals()
            .find_usable(ws.as_deref(), None, &req.tool, &args_hash)
            .await
            .map_err(ApiError)?
        {
            Some(appr_id) => {
                if !ctx.mcp.approvals().consume(&appr_id).await.map_err(ApiError)? {
                    return Ok(Json(deny_audit(&ctx, &mut audit, "approval already used").await));
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
                        tool: Some(req.tool.clone()),
                        title: format!("otto MCP server → {}", req.tool),
                        detail: Some(dangerous_detail(&req.tool, &req.arguments)),
                        args_redacted_json: audit.args_redacted_json.clone(),
                        args_hash: Some(args_hash.clone()),
                        risk_label: Some("dangerous".into()),
                        requested_by: Some(user.id.clone()),
                        requested_by_kind: Some("mcp_server".into()),
                        expires_at: Some((chrono::Utc::now() + chrono::Duration::minutes(120)).to_rfc3339()),
                    })
                    .await
                    .map_err(ApiError)?;
                match wait_for_decision(&ctx, &appr.id, req.wait_seconds).await {
                    Some(true) => {
                        let _ = ctx.mcp.approvals().consume(&appr.id).await;
                        audit.approval_id = Some(appr.id.clone());
                    }
                    Some(false) => {
                        audit.decision = "denied".into();
                        audit.decision_reason = Some("human denied the request".into());
                        let _ = ctx.mcp.call_log().insert(audit).await;
                        return Ok(Json(json!({"decision":"denied","executed":false,"reason":"human denied the request"})));
                    }
                    None => {
                        audit.decision = "pending_approval".into();
                        audit.approval_id = Some(appr.id.clone());
                        let _ = ctx.mcp.call_log().insert(audit).await;
                        return Ok(Json(json!({"decision":"pending_approval","executed":false,
                            "approval_id":appr.id,"reason":"awaiting human approval — resubmit after it is approved"})));
                    }
                }
            }
        }
    }

    if req.dry_run {
        audit.decision = "dry_run".into();
        audit.dry_run = true;
        audit.ok = true;
        let _ = ctx.mcp.call_log().insert(audit).await;
        return Ok(Json(json!({"decision":"dry_run","executed":false,"dry_run":true,
            "preview":{"tool":req.tool,"arguments":otto_core::redact::redact_json(&req.arguments).value,
                       "note":"dry-run: the tool was NOT executed"}})));
    }

    // Fail-closed audit: insert before executing.
    audit.decision = if audit.approval_id.is_some() { "approved".into() } else { "allowed".into() };
    let audit_id = ctx.mcp.call_log().insert(audit).await.map_err(ApiError)?;

    let started = std::time::Instant::now();
    let result = execute_otto_tool(&ctx, &user, &short, &req.arguments).await;
    let latency = started.elapsed().as_millis() as i64;
    match result {
        Ok(value) => {
            let bytes = serde_json::to_vec(&value).map(|v| v.len() as i64).unwrap_or(0);
            let _ = ctx.mcp.call_log().finalize(&audit_id, true, None, Some(latency), Some(bytes), None).await;
            Ok(Json(json!({"decision":"allowed","executed":true,"content":value})))
        }
        Err(e) => {
            let err = otto_core::redact::redact_text(&e.to_string()).value;
            let _ = ctx.mcp.call_log().finalize(&audit_id, false, Some(&err), Some(latency), None, None).await;
            Ok(Json(json!({"decision":"error","executed":true,"is_error":true,"content":{"error":err}})))
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
            let body = json!({ "body": arg_str(args, "body")? });
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
        // ---- Vault v2: code intelligence ----
        "vault_list_repos" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::get(format!("/api/v1/workspaces/{}/vault/repos", seg(&ws)))
        }
        "vault_search_symbols" => {
            let ws = arg_str(args, "workspace_id")?;
            let mut q = vec![format!("q={}", seg(&arg_str(args, "query")?))];
            if let Some(r) = args.get("repo_id").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                q.push(format!("repo_id={}", seg(r)));
            }
            if let Some(l) = args.get("limit").and_then(Value::as_u64) {
                q.push(format!("limit={l}"));
            }
            SelfCall::get(format!("/api/v1/workspaces/{}/vault/symbols?{}", seg(&ws), q.join("&")))
        }
        "vault_code_graph" => {
            let ws = arg_str(args, "workspace_id")?;
            let mut path = format!("/api/v1/workspaces/{}/vault/graph", seg(&ws));
            if let Some(r) = args.get("repo_id").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                path.push_str(&format!("?repo_id={}", seg(r)));
            }
            SelfCall::get(path)
        }
        "vault_full_graph" => {
            let ws = arg_str(args, "workspace_id")?;
            let mut path = format!("/api/v1/workspaces/{}/vault/fullgraph", seg(&ws));
            if let Some(r) = args.get("repo_id").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                path.push_str(&format!("?repo_id={}", seg(r)));
            }
            SelfCall::get(path)
        }
        "vault_node_neighborhood" => {
            let ws = arg_str(args, "workspace_id")?;
            let node = arg_str(args, "node_id")?;
            let depth = args.get("depth").and_then(Value::as_u64).unwrap_or(2);
            SelfCall::get(format!("/api/v1/workspaces/{}/vault/graph/{}?depth={depth}", seg(&ws), seg(&node)))
        }
        "vault_brain" => {
            let ws = arg_str(args, "workspace_id")?;
            let mut body = json!({ "focus": arg_str(args, "focus")? });
            if let Some(c) = args.get("cwd").and_then(Value::as_str) {
                body["cwd"] = json!(c);
            }
            if let Some(b) = args.get("budget").and_then(Value::as_u64) {
                body["budget"] = json!(b);
            }
            SelfCall::post(format!("/api/v1/workspaces/{}/vault/brain", seg(&ws)), body)
        }
        "vault_index_repo" => {
            let ws = arg_str(args, "workspace_id")?;
            let mut body = json!({ "root": arg_str(args, "root")? });
            if let Some(n) = args.get("name").and_then(Value::as_str) {
                body["name"] = json!(n);
            }
            SelfCall::post(format!("/api/v1/workspaces/{}/vault/repos/index", seg(&ws)), body)
        }
        "vault_ingest_text" => {
            let ws = arg_str(args, "workspace_id")?;
            let mut body = json!({ "path": arg_str(args, "path")?, "content": arg_str(args, "content")? });
            if let Some(c) = args.get("collection").and_then(Value::as_str) {
                body["collection"] = json!(c);
            }
            SelfCall::post(format!("/api/v1/workspaces/{}/memory/ingest-text", seg(&ws)), body)
        }
        "vault_upsert_doc" => {
            let ws = arg_str(args, "workspace_id")?;
            let mut body = json!({ "title": arg_str(args, "title")?, "body": arg_str(args, "body")? });
            if let Some(r) = args.get("repo_id").and_then(Value::as_str) {
                body["repo_id"] = json!(r);
            }
            if let Some(d) = args.get("documents") {
                body["documents"] = d.clone();
            }
            SelfCall::post(format!("/api/v1/workspaces/{}/vault/docs", seg(&ws)), body)
        }
        "vault_install_backend" => {
            let ws = arg_str(args, "workspace_id")?;
            let kind = arg_str(args, "kind")?;
            SelfCall::post(
                format!("/api/v1/workspaces/{}/vault/backends/{}/install", seg(&ws), seg(&kind)),
                json!({}),
            )
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
}
