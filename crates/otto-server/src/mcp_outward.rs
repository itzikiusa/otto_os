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
    // Personal-agent room tools: deliberately NOT dangerous — every post is
    // persisted, size-capped, membership-checked and rendered to the user (rooms
    // are the ONLY agent-to-agent transport, fully auditable by design).
    "room_post",
    "room_read",
    "list_agent_rooms",
    // Discovery — where every id an agent needs comes from (workspaces, issue
    // accounts, goal loops, …); metadata only.
    "list_workspaces",
    "list_goal_loops",
    "list_issue_accounts",
    "list_issue_transitions",
    "get_scheduled_task",
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
    // API client reads — metadata + masked shapes only, no secret values.
    "api_list",
    "api_get_request",
    "api_history",
    "list_repos",
    "git_status",
    "list_prs",
    "get_pr",
    // Issues (Jira / Confluence)
    "search_issues",
    "get_issue",
    "search_confluence",
    "get_confluence_page",
    "list_confluence_page_comments",
    // Swarm
    "list_swarms",
    "get_swarm",
    "list_swarm_runs",
    "list_swarm_projects",
    "list_swarm_tasks",
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
    // Design Hall — the artifact graph (reads: find + cite earlier work)
    "design_list",
    "list_design_projects",
    "design_get",
    "design_links",
    "design_search",
    // Sessions
    "list_sessions",
    "get_session",
    "wait_session",
    // Code review / product / channels / usage / skills
    "list_findings",
    "get_finding",
    "list_pr_reviews",
    "get_pr_checks",
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
    // AWS console reads (docs/design/aws-k8s-consoles.md §6) — every one is a
    // GET (or the read-only SQS peek POST) behind its per-service feature grant
    // (`aws_s3` / `aws_sqs` / `aws_ec2` / `aws_athena` / `aws_eks`: View).
    "aws_list_accounts",
    "aws_s3_list_buckets",
    "aws_s3_list_objects",
    "aws_s3_preview",
    "aws_sqs_list_queues",
    "aws_sqs_peek",
    "aws_ec2_list_instances",
    "aws_athena_list_tables",
    "aws_athena_get_query",
    "aws_eks_list_clusters",
    // Kubernetes console reads (`kubernetes`: View).
    "k8s_list_clusters",
    "k8s_get_resources",
    "k8s_describe",
    "k8s_logs",
    "k8s_top",
    "k8s_health",
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
    // Merging is outward-facing and irreversible.
    "merge_pr",
    "comment_issue",
    "transition_issue",
    // API client writers send real HTTP requests and/or persist saved requests.
    "api_execute",
    "api_upsert_request",
    "api_run_automation",
    // Confluence page writes publish to a real wiki everyone reads — same tier
    // as commenting on an issue or opening a PR.
    "create_confluence_page",
    "update_confluence_page",
    "comment_confluence_page",
    "post_swarm_board",
    "test_integration",
    "broadcast_message",
    // Delegation: opening a worker session and driving one session by id are
    // the same capability as broadcast (they type into a running agent).
    "open_session",
    "send_message",
    // Self-improvement (writes — apply/reject/rollback code & skill edits, run a pass)
    "run_self_improvement",
    "approve_improvement_edit",
    "reject_improvement_edit",
    "rollback_improvement_edit",
    // Vault v3 doc writes — file mutations (write/rename) and the soft trash
    // move. Approval-gated like every other write.
    "vault_write",
    "vault_write_file",
    "vault_rename",
    "vault_delete",
    // Design Hall writes — starting an agent turn (spawns a session + commits
    // a version) and filing an explicit link. Approving a version stays
    // human-only: there is deliberately no tool for it.
    "design_assist",
    "design_link",
    // AWS / Kubernetes console writers — a billed Athena scan, a produced SQS
    // message, and a kubectl rollout/scale/delete/Argo verb against a live
    // cluster. Each is also Edit-gated per feature by the self-call's RBAC.
    "aws_athena_query",
    "aws_sqs_send",
    "k8s_action",
    // Otto Assistant memory writes: an outside agent writing / erasing the
    // user's personal memory is approval-gated (in-session assistant calls go
    // through the native stdio tools, which chip + Undo every write).
    "assistant_remember",
    "assistant_forget",
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
    // A PR diff streams code, like code search.
    "get_pr_diff",
    // (search_memory + Vault v2 content reads removed — Vault feature disabled.)
    // Recalled personal memory is content — off until the operator opts in.
    "assistant_recall",
];
const MAX_WAIT_SECS: u64 = 30;

/// The git tools whose `repo_id` is a friendly reference resolved server-side
/// (id, name, local path, or remote — across every workspace the caller can
/// read; omitted → the calling session's repo). See [`fill_repo_ref`].
pub(crate) const REPO_REF_TOOLS: &[&str] = &[
    "list_pr_reviews",
    "get_pr_checks",
    "get_pr_diff",
    "merge_pr",
    "git_status",
    "list_prs",
    "get_pr",
    "create_pr",
    "comment_pr",
    "start_pr_review",
    "open_pr_draft",
];

/// Shared schema text for [`REPO_REF_TOOLS`]' `repo_id`.
const REPO_REF_DESC: &str = "Otto repo id — or a repo name, local path, or remote (`owner/repo` or URL). Resolved across EVERY workspace you can read, not just the current one. Omit it inside an Otto session to use the repo the session is working in. An ambiguous or unknown reference returns the candidates to pick from.";

/// Shared schema text for the friendly-reference id arguments resolved by
/// [`fill_refs`] (see `agent_refs`): the id OR a human field, across every
/// workspace the caller can read, with candidates listed on a miss.
const WS_DIR_DESC: &str = "Optional: only this workspace (id or name). Omit to list every workspace you can read (your current one first).";
const WORKFLOW_REF_DESC: &str = "Workflow id or name — names resolve across every workspace you can read (otto.list_workflows); an ambiguous or unknown one returns the candidates.";
const BROKER_REF_DESC: &str = "Broker cluster id or name (otto.list_broker_clusters).";
const CONNECTION_REF_DESC: &str =
    "Connection id or name (otto.list_connections) — resolved across every workspace you can read.";
const API_REQUEST_REF_DESC: &str =
    "Saved request id or name (otto.api_list), within `workspace_id`.";
const ISSUE_ACCOUNT_REF_DESC: &str = "Your Jira/Confluence account: id, label, email or base URL (otto.list_issue_accounts). Omit it when you have exactly one account.";
const PAGE_REF_DESC: &str = "Confluence page id, or the page URL (…/pages/<id>/… or ?pageId=<id>).";
const SWARM_REF_DESC: &str = "Swarm id or name (otto.list_swarms).";
const VAULT_REF_DESC: &str = "Vault id (integer) or vault name (otto.vault_list).";
const DESIGN_REF_DESC: &str =
    "Design artifact id or its exact title (otto.design_list / otto.design_search).";
const ROOM_REF_DESC: &str = "Agent room id or name (otto.list_agent_rooms).";
const TASK_REF_DESC: &str = "Scheduled task id or name (otto.list_scheduled_tasks).";
const AWS_ACCOUNT_REF_DESC: &str = "AWS account id or name (otto.aws_list_accounts).";
const K8S_CLUSTER_REF_DESC: &str = "Kubernetes cluster id or name (otto.k8s_list_clusters).";
const PR_NUMBER_DESC: &str = "Pull request number (otto.list_prs).";

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
            "description":"Create and start a bounded goal loop (Plan→Execute→Evaluate→Digest). `definition` = {title, acceptance_criteria:[{id, text, verify:\"command\"|\"agent\"|…, verify_cmd (command kind)}] (non-empty)}; `limits` = {max_iterations, max_runtime_secs, per_phase_timeout_secs}; `config` = {executors:[{provider, prompt}] (non-empty), planner, evaluator, digester, definer — each {provider, prompt}}. DANGEROUS: spawns autonomous agents — approval-gated.",
            "inputSchema":{"type":"object","required":["workspace_id","name","repo_path","definition","limits","config"],"properties":{
                "workspace_id":{"type":"string"},"name":{"type":"string"},"repo_path":{"type":"string"},
                "definition":{"type":"object"},"limits":{"type":"object"},"config":{"type":"object"}}}}),
        json!({"name":"otto.create_work_item","mutating":true,"category":"Swarm",
            "description":"Create a work item (a Swarm task) under a project. DANGEROUS: mutates project state — approval-gated.",
            "inputSchema":{"type":"object","required":["project_id","title"],"properties":{
                "project_id":{"type":"string"},"title":{"type":"string"},
                "description":{"type":"string"},"priority":{"type":"string"}}}}),
        json!({"name":"otto.query_db_readonly","mutating":false,"category":"Database",
            "description":"Run a READ-ONLY query against an Otto DB connection. Writes/DDL and multi-statement input are rejected server-side regardless of the connection's guard, and the query executes in the engine's read-only mode with sensitive cells masked. Optional `node` scopes it (e.g. `db:<name>` / `kdb:<n>`).",
            "inputSchema":{"type":"object","required":["connection_id","statement"],"properties":{
                "connection_id":{"type":"string","description":CONNECTION_REF_DESC},"statement":{"type":"string"},"max_rows":{"type":"integer"},
                "node":{"type":"string"}}}}),
        json!({"name":"otto.open_pr_draft","mutating":false,"category":"Git",
            "description":"Draft a PR title + description from a repo's diff vs a base branch. Drafts text only — does NOT open/publish a PR.",
            "inputSchema":{"type":"object","required":["base"],"properties":{
                "repo_id":{"type":"string","description":REPO_REF_DESC},"base":{"type":"string"}}}}),
        json!({"name":"otto.get_proof_pack","mutating":false,"category":"Code & Context",
            "description":"Assemble an evidence bundle for a target: git status/recent-commits/diffstat for a repo and a goal loop's machine-checked acceptance criteria.",
            "inputSchema":{"type":"object","required":["workspace_id"],"properties":{
                "workspace_id":{"type":"string"},"repo_id":{"type":"string","description":"Otto repo id (otto.list_repos) — must live in `workspace_id`."},
                "branch":{"type":"string"},"goal_loop_id":{"type":"string","description":"Goal loop id or name (otto.list_goal_loops) — must live in `workspace_id`."}}}}),
        json!({"name":"otto.list_workspaces","mutating":false,"category":"Code & Context",
            "description":"List the Otto workspaces you can read — `{items}` with id, name, root_path, my_role. Every `workspace_id` argument accepts one of these ids OR the workspace's name. Read-only.",
            "inputSchema":{"type":"object","properties":{}}}),
        json!({"name":"otto.list_goal_loops","mutating":false,"category":"Agents",
            "description":"List goal loops across EVERY workspace you can read — `{items, …}` with id, name, repo_path, status, workspace_id + workspace_name. The ids feed otto.get_proof_pack `goal_loop_id`. Read-only.",
            "inputSchema":{"type":"object","properties":{"workspace_id":{"type":"string","description":WS_DIR_DESC}}}}),
        json!({"name":"otto.ask_human_approval","mutating":false,"category":"Approvals",
            "description":"Request a human's approval for an action and (optionally) wait for the decision. Creates a pending item in the MCP approval queue.",
            "inputSchema":{"type":"object","required":["title"],"properties":{
                "workspace_id":{"type":"string"},"title":{"type":"string"},
                "detail":{"type":"string"},"wait_seconds":{"type":"integer"}}}}),
        // ================= Workflows =================
        json!({"name":"otto.list_workflows","mutating":false,"category":"Workflows",
            "description":"List workflows (visual node-graph automations) across EVERY workspace you can read — `{items, current_workspace_id, workspace_count}`, each item id, name, description, version, workspace_id + workspace_name (graph omitted: use otto.get_workflow). Pass a workflow's id OR its name as `workflow_id` to the other workflow tools. Read-only.",
            "inputSchema":{"type":"object","properties":{"workspace_id":{"type":"string","description":WS_DIR_DESC}}}}),
        json!({"name":"otto.get_workflow","mutating":false,"category":"Workflows",
            "description":"Get one workflow's full definition (graph nodes + edges + metadata). Read-only.",
            "inputSchema":{"type":"object","required":["workflow_id"],"properties":{"workflow_id":{"type":"string","description":WORKFLOW_REF_DESC}}}}),
        json!({"name":"otto.list_workflow_runs","mutating":false,"category":"Workflows",
            "description":"List the most recent runs (newest first, at most 50) of a workflow. `summary` (default true) returns the lightweight run rows (id, status, timing, waiting_approval); pass false for full rows with node states and outputs. Use a run id with otto.get_workflow_run. Read-only.",
            "inputSchema":{"type":"object","required":["workflow_id"],"properties":{"workflow_id":{"type":"string","description":WORKFLOW_REF_DESC},
                "summary":{"type":"boolean","description":"Default true."}}}}),
        json!({"name":"otto.get_workflow_run","mutating":false,"category":"Workflows",
            "description":"Get one workflow run's status, per-node step states and outputs by run id. Read-only.",
            "inputSchema":{"type":"object","required":["run_id"],"properties":{"run_id":{"type":"string"}}}}),
        json!({"name":"otto.run_workflow","mutating":true,"category":"Workflows",
            "description":"Execute a workflow now; returns the new run (poll otto.get_workflow_run). Optionally pass `input` (seed JSON) and `start_node` (run that node + downstream). Optional `review_mode` (\"fan_out\" | \"orchestrator\") overrides every review step's execution mode for this run. DANGEROUS: spawns agents / external effects — approval-gated.",
            "inputSchema":{"type":"object","required":["workflow_id"],"properties":{
                "workflow_id":{"type":"string","description":WORKFLOW_REF_DESC},"input":{"type":"object"},"start_node":{"type":"string"},
                "review_mode":{"type":"string","enum":["fan_out","orchestrator"]}}}}),
        json!({"name":"otto.cancel_workflow_run","mutating":true,"category":"Workflows",
            "description":"Cancel a running workflow run by id. DANGEROUS — approval-gated.",
            "inputSchema":{"type":"object","required":["run_id"],"properties":{"run_id":{"type":"string"}}}}),
        // ================= Message Brokers =================
        json!({"name":"otto.list_broker_clusters","mutating":false,"category":"Message Brokers",
            "description":"List message-broker (Kafka) clusters across EVERY workspace you can read (global profiles included once) — `{items, …}` with id, name, bootstrap_servers, workspace_id + workspace_name. Pass a cluster's id OR name as `cluster_id` to the other broker tools. Read-only.",
            "inputSchema":{"type":"object","properties":{"workspace_id":{"type":"string","description":WS_DIR_DESC}}}}),
        json!({"name":"otto.list_broker_topics","mutating":false,"category":"Message Brokers",
            "description":"List the topics of a broker cluster (name + partition/replication summary). Read-only.",
            "inputSchema":{"type":"object","required":["cluster_id"],"properties":{"cluster_id":{"type":"string","description":BROKER_REF_DESC}}}}),
        json!({"name":"otto.get_broker_topic","mutating":false,"category":"Message Brokers",
            "description":"Get one topic's detail (partitions, offsets, config) on a cluster. Read-only.",
            "inputSchema":{"type":"object","required":["cluster_id","topic"],"properties":{
                "cluster_id":{"type":"string","description":BROKER_REF_DESC},"topic":{"type":"string"}}}}),
        json!({"name":"otto.list_consumer_groups","mutating":false,"category":"Message Brokers",
            "description":"List a cluster's consumer groups (group_id, state, protocol_type, members — no lag; lag is per group in the Brokers module). Read-only.",
            "inputSchema":{"type":"object","required":["cluster_id"],"properties":{"cluster_id":{"type":"string","description":BROKER_REF_DESC}}}}),
        json!({"name":"otto.consume_broker_messages","mutating":false,"category":"Message Brokers",
            "description":"Read recent messages from a topic (the latest `limit`, no offset commits — purely a read). Off by default (streams payloads); enable to inspect message content.",
            "inputSchema":{"type":"object","required":["cluster_id","topic"],"properties":{
                "cluster_id":{"type":"string","description":BROKER_REF_DESC},"topic":{"type":"string"},"partition":{"type":"integer"},
                "limit":{"type":"integer"},"value_filter":{"type":"string","description":"substring filter on the decoded value"}}}}),
        json!({"name":"otto.produce_broker_message","mutating":true,"category":"Message Brokers",
            "description":"Produce a message to a topic. `value` required; optional `key`/`partition`. Guarded clusters need `confirm=true`. DANGEROUS: writes to a broker — approval-gated.",
            "inputSchema":{"type":"object","required":["cluster_id","topic","value"],"properties":{
                "cluster_id":{"type":"string","description":BROKER_REF_DESC},"topic":{"type":"string"},"value":{"type":"string"},
                "key":{"type":"string"},"partition":{"type":"integer"},"confirm":{"type":"boolean"}}}}),
        // ================= Connections =================
        json!({"name":"otto.list_connections","mutating":false,"category":"Database",
            "description":"List connections (DB/SSH/…) across EVERY workspace you can read (global profiles included once) — `{items, …}` with id, name, kind, environment, read_only, workspace_id + workspace_name; connection parameters and secrets are never included. Pass a connection's id OR name as `connection_id`. Read-only.",
            "inputSchema":{"type":"object","properties":{"workspace_id":{"type":"string","description":WS_DIR_DESC}}}}),
        // ================= API Client =================
        json!({"name":"otto.api_list","mutating":false,"category":"API Client",
            "description":"READ-ONLY: discover a workspace's API client collections, saved requests, environments and automations. Request URLs remain templates; environment secret values and tokens are never returned.",
            "inputSchema":{"type":"object","required":["workspace_id"],"properties":{
                "workspace_id":{"type":"string"},"q":{"type":"string","description":"optional substring filter"},
                "collection_id":{"type":"string"},"kind":{"type":"string","enum":["all","requests","environments","automations"]}}}}),
        json!({"name":"otto.api_get_request","mutating":false,"category":"API Client",
            "description":"READ-ONLY: get one saved API request by id, including its agent-facing body and extras. Auth and sensitive header/query values are masked; a body over 64 KiB is capped and ends with `…[truncated]`.",
            "inputSchema":{"type":"object","required":["workspace_id","request_id"],"properties":{
                "workspace_id":{"type":"string"},"request_id":{"type":"string","description":API_REQUEST_REF_DESC}}}}),
        json!({"name":"otto.api_history","mutating":false,"category":"API Client",
            "description":"READ-ONLY: search past API executions, or pass `id` for one history entry and its response. Stored auth and sensitive response values are masked.",
            "inputSchema":{"type":"object","required":["workspace_id"],"properties":{
                "workspace_id":{"type":"string"},"id":{"type":"string"},"limit":{"type":"integer"},
                "q":{"type":"string"},"status":{"type":"integer"},"request_id":{"type":"string"},
                "source":{"type":"string","enum":["agent","human"]}}}}),
        json!({"name":"otto.api_execute","mutating":true,"category":"API Client",
            "description":"Execute one SAVED API request against an environment. Non-GET/HEAD/OPTIONS methods require `confirm:true`; an agent-authored request targeting a new host requires `confirm_new_host:true`. Secrets are resolved server-side and scrubbed from every result; JWTs are returned only as decoded claims. DANGEROUS: sends a real HTTP request — approval-gated.",
            "inputSchema":{"type":"object","required":["workspace_id","request_id"],"properties":{
                "workspace_id":{"type":"string"},"request_id":{"type":"string","description":API_REQUEST_REF_DESC},"environment_id":{"type":"string","description":"Environment id or name (default: the active one)."},
                "vars":{"type":"object","additionalProperties":{"type":"string"}},"timeout_ms":{"type":"integer"},
                "confirm":{"type":"boolean"},"confirm_new_host":{"type":"boolean"},
                "decode_jwt":{"type":"boolean","description":"Return safe JWT claims (default true), never tokens."}}}}),
        json!({"name":"otto.api_upsert_request","mutating":true,"category":"API Client",
            "description":"Create or update a saved API request. Pass `request_id` (id or name) to update; `name`, `method`, and `url` are required for both. On update every omitted field (headers, query, body_mode, body, collection_id, auth, extras) KEEPS its stored value — only what you pass changes. Returns the saved request in the masked agent shape. DANGEROUS: persists a saved request — approval-gated.",
            "inputSchema":{"type":"object","required":["workspace_id","name","method","url"],"properties":{
                "workspace_id":{"type":"string"},"request_id":{"type":"string","description":API_REQUEST_REF_DESC},"collection_id":{"type":["string","null"]},
                "name":{"type":"string"},"method":{"type":"string"},"url":{"type":"string"},
                "headers":{"type":"array"},"query":{"type":"array"},"body_mode":{"type":"string"},
                "body":{"type":"string"},"auth":{"type":"object"},"extras":{"type":"object"}}}}),
        json!({"name":"otto.api_run_automation","mutating":true,"category":"API Client",
            "description":"Run a saved API automation and return its per-step report. DANGEROUS: sends the automation's real HTTP requests — approval-gated.",
            "inputSchema":{"type":"object","required":["workspace_id","automation_id"],"properties":{
                "workspace_id":{"type":"string"},"automation_id":{"type":"string","description":"Automation id or name (see otto.api_list)."}}}}),
        // ================= Git =================
        json!({"name":"otto.list_repos","mutating":false,"category":"Git",
            "description":"List the git repositories in EVERY workspace you can read — `{repos, current_workspace_id, workspace_count}`, each row carrying id, name, path, remote_url, workspace_id + workspace_name (your current workspace first). A repo is registered in exactly one workspace, so look here before concluding a repo is missing. Optional `workspace_id` narrows to one workspace. Read-only.",
            "inputSchema":{"type":"object","properties":{"workspace_id":{"type":"string","description":"Optional: only this workspace's repos."}}}}),
        json!({"name":"otto.git_status","mutating":false,"category":"Git",
            "description":"Get a repo's git status (current branch, staged/unstaged/untracked files). Read-only.",
            "inputSchema":{"type":"object","properties":{"repo_id":{"type":"string","description":REPO_REF_DESC}}}}),
        json!({"name":"otto.list_prs","mutating":false,"category":"Git",
            "description":"List a repo's pull requests as `{items, has_more, page, per_page}`. Optional `state` filter (open|merged|declined|all); page with `page` (1-based) while `has_more`, `per_page` ≤ 100 (default 50). Read-only.",
            "inputSchema":{"type":"object","properties":{
                "repo_id":{"type":"string","description":REPO_REF_DESC},"state":{"type":"string"},
                "page":{"type":"integer"},"per_page":{"type":"integer"}}}}),
        json!({"name":"otto.get_pr","mutating":false,"category":"Git",
            "description":"Get one pull request's detail by number: title, description, state, branches, reviewers, approvals, mergeability and its comments (id, body, path, line, thread_id — reply in-thread with otto.comment_pr `in_reply_to`). Read-only.",
            "inputSchema":{"type":"object","properties":{
                "repo_id":{"type":"string","description":REPO_REF_DESC},"number":{"type":"integer","description":PR_NUMBER_DESC},
                "pr_number":{"type":"integer","description":"Alias of `number`."}}}}),
        json!({"name":"otto.create_pr","mutating":true,"category":"Git",
            "description":"Open a pull request on a repo's provider. `repo_id` may be a repo name, path or remote — the repo is found across all your workspaces. Optional `draft`, `reviewers` (provider usernames/ids) and `proof_pack_id` (ties the PR to a proof pack so the repo's proof gate can pass). DANGEROUS: outward-facing publish — approval-gated.",
            "inputSchema":{"type":"object","required":["title","description","source_branch","target_branch"],"properties":{
                "repo_id":{"type":"string","description":REPO_REF_DESC},"title":{"type":"string"},"description":{"type":"string"},
                "source_branch":{"type":"string"},"target_branch":{"type":"string"},
                "draft":{"type":"boolean"},"reviewers":{"type":"array","items":{"type":"string"}},
                "proof_pack_id":{"type":"string"}}}}),
        json!({"name":"otto.comment_pr","mutating":true,"category":"Git",
            "description":"Post a comment on a pull request — general, inline when `path` (and optionally `line`) anchor it to a file in the diff, or a threaded reply when `in_reply_to` names an existing comment id. DANGEROUS: outward-facing — approval-gated.",
            "inputSchema":{"type":"object","required":["body"],"properties":{
                "repo_id":{"type":"string","description":REPO_REF_DESC},"number":{"type":"integer","description":PR_NUMBER_DESC},
                "pr_number":{"type":"integer","description":"Alias of `number`."},"body":{"type":"string"},
                "path":{"type":"string"},"line":{"type":"integer"},"in_reply_to":{"type":"string"}}}}),
        json!({"name":"otto.start_pr_review","mutating":true,"category":"Code Review",
            "description":"Start Otto's multi-agent review of a pull request (fan-out); returns the review (its `id` feeds otto.list_findings). Optional `context` (extra reviewer instructions) and a linked Jira story (`issue_key` + `issue_account_id`). DANGEROUS: spawns agents — approval-gated.",
            "inputSchema":{"type":"object","properties":{
                "repo_id":{"type":"string","description":REPO_REF_DESC},"pr_number":{"type":"integer","description":PR_NUMBER_DESC},
                "number":{"type":"integer","description":"Alias of `pr_number`."},
                "context":{"type":"string"},"issue_key":{"type":"string"},"issue_account_id":{"type":"string"}}}}),
        json!({"name":"otto.list_pr_reviews","mutating":false,"category":"Code Review",
            "description":"List Otto's multi-agent review runs of a pull request (id, status, verdict, blocker_count, summary) — the `review_id` otto.list_findings takes. Read-only.",
            "inputSchema":{"type":"object","properties":{
                "repo_id":{"type":"string","description":REPO_REF_DESC},"pr_number":{"type":"integer","description":PR_NUMBER_DESC},
                "number":{"type":"integer","description":"Alias of `pr_number`."}}}}),
        json!({"name":"otto.get_pr_checks","mutating":false,"category":"Git",
            "description":"A pull request's CI / build checks `{ci, checks[]}` (name, state, url) from the provider. Read-only.",
            "inputSchema":{"type":"object","properties":{
                "repo_id":{"type":"string","description":REPO_REF_DESC},"number":{"type":"integer","description":PR_NUMBER_DESC},
                "pr_number":{"type":"integer","description":"Alias of `number`."}}}}),
        json!({"name":"otto.get_pr_diff","mutating":false,"category":"Git",
            "description":"A pull request's diff (files + hunks). Off by default (streams code); enable to let agents read PR diffs. Read-only.",
            "inputSchema":{"type":"object","properties":{
                "repo_id":{"type":"string","description":REPO_REF_DESC},"number":{"type":"integer","description":PR_NUMBER_DESC},
                "pr_number":{"type":"integer","description":"Alias of `number`."}}}}),
        json!({"name":"otto.merge_pr","mutating":true,"category":"Git",
            "description":"Merge a pull request on the provider. `strategy` = merge | squash | rebase (provider default when omitted); `delete_source_branch` optional. Check otto.get_pr (mergeable) and otto.get_pr_checks first. DANGEROUS: outward-facing and irreversible — approval-gated.",
            "inputSchema":{"type":"object","properties":{
                "repo_id":{"type":"string","description":REPO_REF_DESC},"number":{"type":"integer","description":PR_NUMBER_DESC},
                "pr_number":{"type":"integer","description":"Alias of `number`."},
                "strategy":{"type":"string","enum":["merge","squash","rebase"]},"delete_source_branch":{"type":"boolean"}}}}),
        // ================= Issues (Jira / Confluence) =================
        json!({"name":"otto.search_issues","mutating":false,"category":"Issues",
            "description":"Search Jira issues. `query` is FREE TEXT (matched against summary + description), or an issue key (`GS-123`) for that issue — it is NOT JQL. Empty `query` → issues assigned to you. Optional `project` key. Returns up to 25 `{key, summary, status, issue_type, url}`; page with `start_at`. Read-only.",
            "inputSchema":{"type":"object","properties":{
                "account_id":{"type":"string","description":ISSUE_ACCOUNT_REF_DESC},"query":{"type":"string"},"project":{"type":"string"},
                "start_at":{"type":"integer","description":"Offset of the first result (default 0)."}}}}),
        json!({"name":"otto.get_issue","mutating":false,"category":"Issues",
            "description":"Get one Jira issue's full detail (description, comments, changelog, links) by key. Available status changes come from otto.list_issue_transitions. Read-only.",
            "inputSchema":{"type":"object","required":["key"],"properties":{
                "account_id":{"type":"string","description":ISSUE_ACCOUNT_REF_DESC},"key":{"type":"string"}}}}),
        json!({"name":"otto.search_confluence","mutating":false,"category":"Issues",
            "description":"Search Confluence pages by title (`query`; a numeric query matches a page id); optional `space` key. Returns up to 25 `{id, title, space_key, url}`. Read-only.",
            "inputSchema":{"type":"object","required":["query"],"properties":{
                "account_id":{"type":"string","description":ISSUE_ACCOUNT_REF_DESC},"query":{"type":"string"},"space":{"type":"string"}}}}),
        json!({"name":"otto.get_confluence_page","mutating":false,"category":"Issues",
            "description":"Read one Confluence page. Returns id, title, space_key, url, version and the body as MARKDOWN (`body_md`) — never storage XHTML. Read-only.",
            "inputSchema":{"type":"object","required":["page_id"],"properties":{
                "account_id":{"type":"string","description":ISSUE_ACCOUNT_REF_DESC},"page_id":{"type":"string","description":PAGE_REF_DESC}}}}),
        json!({"name":"otto.list_confluence_page_comments","mutating":false,"category":"Issues",
            "description":"List the footer comments on a Confluence page (author, body as markdown, created). Use it to collect answers people left on a page. Read-only.",
            "inputSchema":{"type":"object","required":["page_id"],"properties":{
                "account_id":{"type":"string","description":ISSUE_ACCOUNT_REF_DESC},"page_id":{"type":"string","description":PAGE_REF_DESC}}}}),
        json!({"name":"otto.create_confluence_page","mutating":true,"category":"Issues",
            "description":"Create a Confluence page. Supply `body_md` (MARKDOWN, converted server-side) OR `body_html` (Confluence storage XHTML, passed through — use it for panel/expand/status macros, layouts and anything Markdown cannot express). Optional `parent_id` nests it under an existing page. DANGEROUS: outward-facing — approval-gated.",
            "inputSchema":{"type":"object","required":["space_key","title"],"properties":{
                "account_id":{"type":"string","description":ISSUE_ACCOUNT_REF_DESC},"space_key":{"type":"string"},"title":{"type":"string"},
                "body_md":{"type":"string"},"body_html":{"type":"string"},"parent_id":{"type":"string","description":"Parent page id or URL."}}}}),
        json!({"name":"otto.update_confluence_page","mutating":true,"category":"Issues",
            "description":"Replace a Confluence page's body with `body_md` (MARKDOWN) or `body_html` (Confluence storage XHTML, passed through). Pass `base_version` (the page version you read with otto.get_confluence_page) so a newer human edit is never overwritten — the call then fails with a conflict instead; without it the latest version is overwritten. Omit `title` to keep the existing one. DANGEROUS: outward-facing — approval-gated.",
            "inputSchema":{"type":"object","required":["page_id"],"properties":{
                "account_id":{"type":"string","description":ISSUE_ACCOUNT_REF_DESC},"page_id":{"type":"string","description":PAGE_REF_DESC},"body_md":{"type":"string"},
                "body_html":{"type":"string"},"title":{"type":"string"},"base_version":{"type":"integer"}}}}),
        json!({"name":"otto.comment_confluence_page","mutating":true,"category":"Issues",
            "description":"Add a footer comment to a Confluence page. Supply `body_md` (MARKDOWN) or `body_html` (storage XHTML). DANGEROUS: outward-facing — approval-gated.",
            "inputSchema":{"type":"object","required":["page_id"],"properties":{
                "account_id":{"type":"string","description":ISSUE_ACCOUNT_REF_DESC},"page_id":{"type":"string","description":PAGE_REF_DESC},"body_md":{"type":"string"},
                "body_html":{"type":"string"}}}}),
        json!({"name":"otto.comment_issue","mutating":true,"category":"Issues",
            "description":"Add a comment to a Jira issue. DANGEROUS: outward-facing — approval-gated.",
            "inputSchema":{"type":"object","required":["key","body"],"properties":{
                "account_id":{"type":"string","description":ISSUE_ACCOUNT_REF_DESC},"key":{"type":"string"},"body":{"type":"string"}}}}),
        json!({"name":"otto.transition_issue","mutating":true,"category":"Issues",
            "description":"Transition a Jira issue to a new status. `transition_id` is a transition id from otto.list_issue_transitions, or its name / target status name (e.g. \"In Progress\") — resolved against the issue's available transitions. DANGEROUS — approval-gated.",
            "inputSchema":{"type":"object","required":["key","transition_id"],"properties":{
                "account_id":{"type":"string","description":ISSUE_ACCOUNT_REF_DESC},"key":{"type":"string"},"transition_id":{"type":"string","description":"Transition id, name, or target status name."}}}}),
        json!({"name":"otto.list_issue_accounts","mutating":false,"category":"Issues",
            "description":"List YOUR Jira/Confluence accounts — `{items}` with id, label, email, base_url, provider (never the token). Every Issues tool takes `account_id` = one of these ids OR its label / email / base URL, and may omit it when you have exactly one account. Read-only.",
            "inputSchema":{"type":"object","properties":{}}}),
        json!({"name":"otto.list_issue_transitions","mutating":false,"category":"Issues",
            "description":"List a Jira issue's available status transitions `[{id, name, to_status}]` — the ids (or names) otto.transition_issue accepts. Read-only.",
            "inputSchema":{"type":"object","required":["key"],"properties":{
                "account_id":{"type":"string","description":ISSUE_ACCOUNT_REF_DESC},"key":{"type":"string"}}}}),
        // ================= Swarm =================
        json!({"name":"otto.list_swarms","mutating":false,"category":"Swarm",
            "description":"List agent swarms across EVERY workspace you can read — `{items, …}` with id, name, status, workspace_id + workspace_name. Pass a swarm's id OR name as `swarm_id`. Read-only.",
            "inputSchema":{"type":"object","properties":{"workspace_id":{"type":"string","description":WS_DIR_DESC}}}}),
        json!({"name":"otto.get_swarm","mutating":false,"category":"Swarm",
            "description":"Get a swarm's detail (agents, projects, counts) by id. Read-only.",
            "inputSchema":{"type":"object","required":["swarm_id"],"properties":{"swarm_id":{"type":"string","description":SWARM_REF_DESC}}}}),
        json!({"name":"otto.list_swarm_runs","mutating":false,"category":"Swarm",
            "description":"List a workspace's swarm runs (newest first, up to 500). Optional `swarm_id` narrows to one swarm. Read-only.",
            "inputSchema":{"type":"object","required":["workspace_id"],"properties":{"workspace_id":{"type":"string"},
                "swarm_id":{"type":"string","description":SWARM_REF_DESC}}}}),
        json!({"name":"otto.list_swarm_projects","mutating":false,"category":"Swarm",
            "description":"List a swarm's projects (id, name, repo_path, goal) — the `project_id`s otto.list_swarm_tasks / otto.create_work_item take. Read-only.",
            "inputSchema":{"type":"object","required":["swarm_id"],"properties":{"swarm_id":{"type":"string","description":SWARM_REF_DESC}}}}),
        json!({"name":"otto.list_swarm_tasks","mutating":false,"category":"Swarm",
            "description":"List a swarm project's board tasks (id, title, status, assignee, priority, depends_on). Read-only.",
            "inputSchema":{"type":"object","required":["project_id"],"properties":{"project_id":{"type":"string","description":"Swarm project id (otto.list_swarm_projects)."}}}}),
        json!({"name":"otto.get_swarm_board","mutating":false,"category":"Swarm",
            "description":"Read a swarm's shared message board (newest 300). Optional `project_id` / `task_id` narrow it. Read-only.",
            "inputSchema":{"type":"object","required":["swarm_id"],"properties":{"swarm_id":{"type":"string","description":SWARM_REF_DESC},
                "project_id":{"type":"string"},"task_id":{"type":"string"}}}}),
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
                "workspace_id":{"type":"string","description":"Optional — vaults are global; defaults to an accessible workspace."},"vault_id":{"type":["integer","string"],"description":VAULT_REF_DESC},"path":{"type":"string"}}}}),
        json!({"name":"otto.vault_read","mutating":false,"category":"Vault",
            "description":"A note's raw markdown + metadata + outgoing links. Read-only.",
            "inputSchema":{"type":"object","required":["vault_id","path"],"properties":{
                "workspace_id":{"type":"string","description":"Optional — vaults are global; defaults to an accessible workspace."},"vault_id":{"type":["integer","string"],"description":VAULT_REF_DESC},"path":{"type":"string"}}}}),
        json!({"name":"otto.vault_search","mutating":false,"category":"Vault",
            "description":"Full-text (FTS5) search over a vault's notes with snippets; tag:/path:/type: operators. Read-only.",
            "inputSchema":{"type":"object","required":["vault_id","query"],"properties":{
                "workspace_id":{"type":"string","description":"Optional — vaults are global; defaults to an accessible workspace."},"vault_id":{"type":["integer","string"],"description":VAULT_REF_DESC},"query":{"type":"string"},"limit":{"type":"integer"}}}}),
        json!({"name":"otto.vault_backlinks","mutating":false,"category":"Vault",
            "description":"Notes linking TO a given note, with context snippets. Read-only.",
            "inputSchema":{"type":"object","required":["vault_id","path"],"properties":{
                "workspace_id":{"type":"string","description":"Optional — vaults are global; defaults to an accessible workspace."},"vault_id":{"type":["integer","string"],"description":VAULT_REF_DESC},"path":{"type":"string"}}}}),
        json!({"name":"otto.vault_tags","mutating":false,"category":"Vault",
            "description":"Every tag in a vault with note counts. Read-only.",
            "inputSchema":{"type":"object","required":["vault_id"],"properties":{
                "workspace_id":{"type":"string","description":"Optional — vaults are global; defaults to an accessible workspace."},"vault_id":{"type":["integer","string"],"description":VAULT_REF_DESC}}}}),
        json!({"name":"otto.vault_graph","mutating":false,"category":"Vault",
            "description":"The vault link graph (compact arrays; local neighborhood when `path` given). Read-only.",
            "inputSchema":{"type":"object","required":["vault_id"],"properties":{
                "workspace_id":{"type":"string","description":"Optional — vaults are global; defaults to an accessible workspace."},"vault_id":{"type":["integer","string"],"description":VAULT_REF_DESC},"mode":{"type":"string"},"path":{"type":"string"},"depth":{"type":"integer"}}}}),
        json!({"name":"otto.vault_okf_validate","mutating":false,"category":"Vault",
            "description":"Deterministic OKF v0.1 conformance report (E1-E3 errors, W1-W5 warnings). Read-only.",
            "inputSchema":{"type":"object","required":["vault_id"],"properties":{
                "workspace_id":{"type":"string","description":"Optional — vaults are global; defaults to an accessible workspace."},"vault_id":{"type":["integer","string"],"description":VAULT_REF_DESC}}}}),
        json!({"name":"otto.vault_write","mutating":true,"category":"Vault",
            "description":"Create/update a markdown note in a doc vault (OKF preferred). DANGEROUS: writes files — approval-gated.",
            "inputSchema":{"type":"object","required":["vault_id","path","content"],"properties":{
                "workspace_id":{"type":"string","description":"Optional — vaults are global; defaults to an accessible workspace."},"vault_id":{"type":["integer","string"],"description":VAULT_REF_DESC},"path":{"type":"string"},
                "content":{"type":"string"},"if_hash":{"type":"string"}}}}),
        json!({"name":"otto.vault_write_file","mutating":true,"category":"Vault",
            "description":"Create/update a guarded UTF-8 documentation artifact in a vault — OpenAPI YAML, JSON, D2, Mermaid, text or CSV (`path` ending .yaml/.yml/.json/.d2/.mmd/.txt/.csv; markdown notes use otto.vault_write). Pass `if_hash` for optimistic concurrency. DANGEROUS: writes files — approval-gated.",
            "inputSchema":{"type":"object","required":["vault_id","path","content"],"properties":{
                "workspace_id":{"type":"string","description":"Optional — vaults are global; defaults to an accessible workspace."},"vault_id":{"type":["integer","string"],"description":VAULT_REF_DESC},"path":{"type":"string"},
                "content":{"type":"string"},"if_hash":{"type":"string"}}}}),
        json!({"name":"otto.vault_rename","mutating":true,"category":"Vault",
            "description":"Rename/move a note or folder; rewrites every referencing link across the vault. DANGEROUS — approval-gated.",
            "inputSchema":{"type":"object","required":["vault_id","from","to"],"properties":{
                "workspace_id":{"type":"string","description":"Optional — vaults are global; defaults to an accessible workspace."},"vault_id":{"type":["integer","string"],"description":VAULT_REF_DESC},"from":{"type":"string"},"to":{"type":"string"}}}}),
        json!({"name":"otto.vault_delete","mutating":true,"category":"Vault",
            "description":"Soft-delete a note into the vault's .trash/ (never destroys files). DANGEROUS — approval-gated.",
            "inputSchema":{"type":"object","required":["vault_id","path"],"properties":{
                "workspace_id":{"type":"string","description":"Optional — vaults are global; defaults to an accessible workspace."},"vault_id":{"type":["integer","string"],"description":VAULT_REF_DESC},"path":{"type":"string"}}}}),
        // ================= Design Hall (artifact graph) =================
        // Read-only: find and cite earlier design work (the References drawer's
        // library). The design library is global; `workspace_id` only narrows.
        json!({"name":"otto.design_list","mutating":false,"category":"Design",
            "description":"List Design Hall artifacts (frames, graphics, sites, 3D scenes, whiteboards, brand kits) with id, title, studio, format, status, head version, who created / last edited it (created_by_name, last_editor_name) and the product stories it implements (story_ids). Optional filters: project_id, studio, format, status, story_id (artifacts implementing a product story). Newest first; for the next page pass cursor = the last row's `updated_at|id`. Read-only.",
            "inputSchema":{"type":"object","required":[],"properties":{
                "workspace_id":{"type":"string","description":"Optional — the design library is global; narrows to one workspace."},
                "project_id":{"type":"string"},
                "studio":{"type":"string","description":"frames | graphics | site | 3d | whiteboard | brand | spatial"},
                "format":{"type":"string"},
                "status":{"type":"string","description":"draft | review | approved | shipped | archived"},
                "story_id":{"type":"string"},
                "limit":{"type":"integer"},
                "cursor":{"type":"string","description":"Next page: `<updated_at>|<id>` of the previous page's last row."}}}}),
        json!({"name":"otto.list_design_projects","mutating":false,"category":"Design",
            "description":"List Design Hall projects (id, name, workspace_id) — the `project_id` filter of otto.design_list / otto.design_search. Read-only.",
            "inputSchema":{"type":"object","properties":{
                "workspace_id":{"type":"string","description":"Optional — narrows the global library to one workspace."},
                "include_archived":{"type":"boolean"}}}}),
        json!({"name":"otto.design_get","mutating":false,"category":"Design",
            "description":"One design artifact: metadata, head + approved versions, link counts, the working-copy and thumbnail file paths, and (text formats) its source — the head's, or `version` (`v3` or a version id). Cite it as otto://design/<id>@v<seq>. Read-only.",
            "inputSchema":{"type":"object","required":["artifact_id"],"properties":{
                "artifact_id":{"type":"string","description":DESIGN_REF_DESC},
                "version":{"type":"string","description":"Optional `v<seq>` or version id; default the head."},
                "include_content":{"type":"boolean","description":"Default true — inline the (≤ 256 KiB) text source."}}}}),
        json!({"name":"otto.design_links","mutating":false,"category":"Design",
            "description":"A design artifact's links: what it uses (embeds, components, brand tokens, stories it implements, what it was derived from) and where it is used, with the linked artifacts' titles. Broken references are flagged. Read-only.",
            "inputSchema":{"type":"object","required":["artifact_id"],"properties":{
                "artifact_id":{"type":"string","description":DESIGN_REF_DESC},
                "dir":{"type":"string","description":"out | in | both (default)"}}}}),
        json!({"name":"otto.design_search","mutating":false,"category":"Design",
            "description":"Full-text search over the design library (titles, tags, extracted copy/layer/token names, linked story keys, project names) — shipped work first. Use it to find references to build on and cite. Read-only.",
            "inputSchema":{"type":"object","required":["query"],"properties":{
                "query":{"type":"string"},
                "workspace_id":{"type":"string","description":"Optional — narrows the global library to one workspace."},
                "studio":{"type":"string"},"format":{"type":"string"},"status":{"type":"string"},
                "story_id":{"type":"string"},"project_id":{"type":"string"},
                "limit":{"type":"integer"}}}}),
        // Writes — approval-gated (DANGEROUS) unless the operator exempts the
        // tool. The artifact's own workspace scopes the call (see
        // `fill_design_workspace`). No tool approves a version.
        json!({"name":"otto.design_assist","mutating":true,"category":"Design",
            "description":"Start a Design Hall agent turn on an artifact: the agent edits the artifact's working copy, the result is validated and committed as a new version (author agent) whose provenance records the references it was offered and the ones it cited. Returns the turn (turn_id, status, session_id); completion arrives as the design_assist_updated event. mode: generate | refine (default) | critique | a11y. DANGEROUS: spawns an agent session and writes a version — approval-gated.",
            "inputSchema":{"type":"object","required":["artifact_id","prompt"],"properties":{
                "artifact_id":{"type":"string","description":DESIGN_REF_DESC},
                "prompt":{"type":"string","description":"What to design or change."},
                "mode":{"type":"string","description":"generate | refine (default) | critique | a11y"},
                "references":{"type":"array","items":{"type":"string"},"description":"Optional artifacts to build on (`<id>` or `<id>@v12`, ≤ 8) — offered to the agent as [R1] …"},
                "selection":{"type":"object","description":"Optional focus, e.g. {\"node_id\":\"hero\"}."}}}}),
        json!({"name":"otto.design_link","mutating":true,"category":"Design",
            "description":"Create an explicit link from a design artifact — e.g. it `implements` a product story, `references` another artifact or a URL, `describes` frames, `embeds` a 3D model. rel: embeds | uses_component | uses_tokens | describes | derived_from | references | implements | variant_of | resized_from | created_in; dst_kind: artifact | story | session | swarm_project | vault_note | pr | url. Render links that would close a cycle are refused (409). DANGEROUS — approval-gated.",
            "inputSchema":{"type":"object","required":["artifact_id","rel","dst_kind","dst_id"],"properties":{
                "artifact_id":{"type":"string","description":"The link source artifact."},
                "rel":{"type":"string"},"dst_kind":{"type":"string"},"dst_id":{"type":"string"},
                "dst_node":{"type":"string"},"src_node":{"type":"string"},
                "policy":{"type":"string","description":"follow_approved | follow_latest | pinned (default by rel)."},
                "pinned_version_id":{"type":"string"}}}}),
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
        json!({"name":"otto.open_session","mutating":true,"category":"Sessions",
            "description":"Open a new agent session (claude/codex) in a workspace and queue an opening prompt once its TUI is up; returns the session (poll otto.wait_session for status). For a lead delegating work to visible, resumable worker sessions. DANGEROUS: spawns an agent — approval-gated.",
            "inputSchema":{"type":"object","required":["workspace_id","provider"],"properties":{
                "workspace_id":{"type":"string"},"provider":{"type":"string","description":"claude | codex"},
                "title":{"type":"string"},"cwd":{"type":"string"},"model":{"type":"string","description":"optional; provider default when omitted"},
                "prompt":{"type":"string","description":"submitted as the first user message"}}}}),
        json!({"name":"otto.send_message","mutating":true,"category":"Sessions",
            "description":"Send one text message to ONE live agent session by id, as if typed + Enter. DANGEROUS: drives a running agent — approval-gated.",
            "inputSchema":{"type":"object","required":["session_id","text"],"properties":{
                "session_id":{"type":"string"},"text":{"type":"string"}}}}),
        json!({"name":"otto.wait_session","mutating":false,"category":"Sessions",
            "description":"Block (up to 25 s) until a session's status is one of the awaited set (default idle,exited), then return it with `reached`. Loop it for longer waits. Read-only.",
            "inputSchema":{"type":"object","required":["session_id"],"properties":{
                "session_id":{"type":"string"},"status":{"type":"string","description":"comma-separated: running,working,idle,exited,reconnectable"},
                "timeout_secs":{"type":"integer","description":"1..25, default 20"}}}}),
        // ================= Code Review / Findings =================
        json!({"name":"otto.list_findings","mutating":false,"category":"Code Review",
            "description":"List a code review's findings (with workflow state) by review id. Read-only.",
            "inputSchema":{"type":"object","required":["review_id"],"properties":{"review_id":{"type":"string"}}}}),
        json!({"name":"otto.get_finding","mutating":false,"category":"Code Review",
            "description":"Get one review finding's detail + event timeline by id. Read-only.",
            "inputSchema":{"type":"object","required":["finding_id"],"properties":{"finding_id":{"type":"string"}}}}),
        // ================= Product =================
        json!({"name":"otto.list_product_stories","mutating":false,"category":"Product",
            "description":"List product stories — a GLOBAL library shared by every workspace (Jira/Confluence-backed) — `{items}` with id, source_key (the Jira key), title, stage, url, workspace_id. Pass a story's id OR its Jira key as `story_id`. Read-only.",
            "inputSchema":{"type":"object","properties":{"workspace_id":{"type":"string","description":"Optional: the workspace whose role check to use (default: the first you can read)."}}}}),
        json!({"name":"otto.get_product_story","mutating":false,"category":"Product",
            "description":"Get one product story's detail (story, source, counts, swarm link). Read-only.",
            "inputSchema":{"type":"object","required":["story_id"],"properties":{"story_id":{"type":"string","description":"Product story id, or its Jira key (e.g. GS-123) or exact title."}}}}),
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
            "description":"List a workspace's self-improvement edit suggestions. `status` defaults to pending; pass applied (to find ids for otto.rollback_improvement_edit), rejected, rolled_back or conflict. Read-only.",
            "inputSchema":{"type":"object","required":["workspace_id"],"properties":{"workspace_id":{"type":"string"},
                "status":{"type":"string","description":"pending (default) | applied | rejected | rolled_back | conflict"}}}}),
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
        // ---- Personal Agents (rooms) ----
        // Deliberately mutating:false / non-DANGEROUS: a room post cannot leave
        // the machine (channel delivery of reports is separate), it is capped at
        // 16 KB, membership-checked, persisted, and always user-visible — the
        // audit trail IS the feature. The calling session is resolved to its
        // personal agent via the session's `meta.personal_agent` (the engine
        // stamps it on every run + chat session); a caller with no
        // personal-agent session posts/reads as the user via the REST routes.
        json!({"name":"otto.room_post","mutating":false,"category":"Personal Agents",
            "description":"Post a message into an agent room you are a member of. The calling personal-agent session is resolved via its session identity; the message (max 16KB) is persisted and shown to the user live. Rooms are the only agent-to-agent channel.",
            "inputSchema":{"type":"object","required":["room_id","text"],"properties":{
                "room_id":{"type":"string","description":ROOM_REF_DESC},"text":{"type":"string"},
                "session_id":{"type":"string","description":"the calling session (injected automatically by Otto's MCP bridge; used to resolve which personal agent is speaking)"}}}}),
        json!({"name":"otto.room_read","mutating":false,"category":"Personal Agents",
            "description":"Read messages from an agent room you are a member of, oldest first. Pass `after` (the last message id you saw) to page forward.",
            "inputSchema":{"type":"object","required":["room_id"],"properties":{
                "room_id":{"type":"string","description":ROOM_REF_DESC},"after":{"type":"string"},"limit":{"type":"integer"},
                "session_id":{"type":"string","description":"the calling session (injected automatically by Otto's MCP bridge)"}}}}),
        // ---- Otto Assistant (the user's personal memory) ----
        json!({"name":"otto.assistant_remember","mutating":true,"category":"Assistant",
            "description":"Save ONE short, atomic fact about the user (a preference, a person, a recurring plan) to the Otto Assistant's private memory. Never store secrets. Shown to the user with Undo; queued for review when memory approval is on. DANGEROUS: writes the user's memory — approval-gated.",
            "inputSchema":{"type":"object","required":["text"],"properties":{
                "text":{"type":"string"},"kind":{"type":"string","description":"fact | decision | learning … (default fact)"},
                "tags":{"type":"array","items":{"type":"string"}},
                "session_id":{"type":"string","description":"the calling assistant session (injected automatically by Otto's MCP bridge)"}}}}),
        json!({"name":"otto.assistant_forget","mutating":true,"category":"Assistant",
            "description":"Forget the user's assistant memories that match `query` (up to 10; each can be restored with its undo token). DANGEROUS: erases personal memory — approval-gated.",
            "inputSchema":{"type":"object","required":["query"],"properties":{
                "query":{"type":"string"},
                "session_id":{"type":"string","description":"the calling assistant session (injected automatically by Otto's MCP bridge)"}}}}),
        json!({"name":"otto.assistant_recall","mutating":false,"category":"Assistant",
            "description":"Recall the user's profile and the assistant memories matching `query` (keyword recall; omit `query` for the most recent). Read-only; off by default (personal content).",
            "inputSchema":{"type":"object","properties":{
                "query":{"type":"string"},"k":{"type":"integer","description":"max memories (1..20, default 10)"},
                "session_id":{"type":"string","description":"the calling assistant session (injected automatically by Otto's MCP bridge)"}}}}),
        json!({"name":"otto.list_agent_rooms","mutating":false,"category":"Personal Agents",
            "description":"List agent rooms across EVERY workspace you can read — `{items, …}` with id, name, workspace_id + workspace_name. Pass a room's id OR name as `room_id` to otto.room_read / otto.room_post. Read-only.",
            "inputSchema":{"type":"object","properties":{"workspace_id":{"type":"string","description":WS_DIR_DESC}}}}),
        // ---- Scheduled Tasks ----
        json!({"name":"otto.list_scheduled_tasks","mutating":false,"category":"Scheduled Tasks",
            "description":"List scheduled tasks (recurring agent jobs) across EVERY workspace you can read — `{items, …}` with each task's full config + workspace_id / workspace_name. Pass a task's id OR name as `task_id` to the other scheduled-task tools. Read-only.",
            "inputSchema":{"type":"object","properties":{"workspace_id":{"type":"string","description":WS_DIR_DESC}}}}),
        json!({"name":"otto.get_scheduled_task","mutating":false,"category":"Scheduled Tasks",
            "description":"Get one scheduled task's full config (schedule, destination, last/next run). Read-only.",
            "inputSchema":{"type":"object","required":["task_id"],"properties":{
                "task_id":{"type":"string","description":TASK_REF_DESC}}}}),
        json!({"name":"otto.list_scheduled_task_runs","mutating":false,"category":"Scheduled Tasks",
            "description":"List the recent run history (status + summary) of a scheduled task. Read-only.",
            "inputSchema":{"type":"object","required":["task_id"],"properties":{
                "task_id":{"type":"string","description":TASK_REF_DESC}}}}),
        json!({"name":"otto.create_scheduled_task","mutating":true,"category":"Scheduled Tasks",
            "description":"Create a scheduled task: a recurring job that runs an agent (or hands off to a workflow) on a cadence, writes a Markdown report, and delivers it to a destination. DANGEROUS: an autonomous recurring capability — approval-gated. `schedule` = {cadence:'interval'|'daily'|'weekly'|'cron', every_min, at:'HH:MM', weekday, expr:'<5-field cron>'} interpreted in `timezone` (IANA). `provider` = claude|codex|agy|shell|<custom>. `kind` = 'agent_prompt'|'workflow' (workflow requires workflow_id). `sandbox` = 'none'|'worktree'. `max_retries` 0..5. `notify_on_change` only delivers when the report changes. `attach_proof` builds a proof pack. `destination` = {type:'none'|'slack'|'telegram'|'email'|'webhook', ...}.",
            "inputSchema":{"type":"object","required":["workspace_id","name"],"properties":{
                "workspace_id":{"type":"string"},"name":{"type":"string"},"prompt":{"type":"string"},
                "kind":{"type":"string"},"provider":{"type":"string"},"model":{"type":"string"},
                "schedule":{"type":"object"},"destination":{"type":"object"},"timezone":{"type":"string"},
                "workflow_id":{"type":"string"},"sandbox":{"type":"string"},"max_retries":{"type":"integer"},
                "notify_on_change":{"type":"boolean"},"attach_proof":{"type":"boolean"},
                "cwd":{"type":"string"},"skill":{"type":"string"},"enabled":{"type":"boolean"}}}}),
        json!({"name":"otto.update_scheduled_task","mutating":true,"category":"Scheduled Tasks",
            "description":"Update a scheduled task's fields (name/prompt/schedule/destination/provider/model/cwd/timezone/sandbox/max_retries/notify_on_change/attach_proof/workflow_id/skill/enabled); omitted fields keep their value, `null` clears `workflow_id` / `skill`. `kind` is fixed at creation. DANGEROUS — approval-gated.",
            "inputSchema":{"type":"object","required":["task_id"],"properties":{
                "task_id":{"type":"string","description":TASK_REF_DESC},"name":{"type":"string"},"prompt":{"type":"string"},
                "provider":{"type":"string"},"model":{"type":"string"},"cwd":{"type":"string"},
                "schedule":{"type":"object"},"destination":{"type":"object"},
                "timezone":{"type":"string"},"workflow_id":{"type":["string","null"],"description":"Workflow id or name; null clears it."},"sandbox":{"type":"string"},
                "max_retries":{"type":"integer"},"notify_on_change":{"type":"boolean"},
                "attach_proof":{"type":"boolean"},"skill":{"type":["string","null"]},"enabled":{"type":"boolean"}}}}),
        json!({"name":"otto.set_scheduled_task_enabled","mutating":true,"category":"Scheduled Tasks",
            "description":"Enable or disable a scheduled task. DANGEROUS — approval-gated.",
            "inputSchema":{"type":"object","required":["task_id","enabled"],"properties":{
                "task_id":{"type":"string","description":TASK_REF_DESC},"enabled":{"type":"boolean"}}}}),
        json!({"name":"otto.run_scheduled_task","mutating":true,"category":"Scheduled Tasks",
            "description":"Run a scheduled task once now (does not change its schedule). Starts the run in the background and returns it immediately with status `running` — poll otto.list_scheduled_task_runs for the result. DANGEROUS — approval-gated.",
            "inputSchema":{"type":"object","required":["task_id"],"properties":{
                "task_id":{"type":"string","description":TASK_REF_DESC}}}}),
        json!({"name":"otto.delete_scheduled_task","mutating":true,"category":"Scheduled Tasks",
            "description":"Delete a scheduled task and its run history. DANGEROUS — approval-gated.",
            "inputSchema":{"type":"object","required":["task_id"],"properties":{
                "task_id":{"type":"string","description":TASK_REF_DESC}}}}),
        // ================= AWS console =================
        // docs/design/aws-k8s-consoles.md §6. Accounts are global rows (no
        // workspace_id); the self-call reuses the per-service feature grants
        // (`aws_s3`/`aws_sqs`/`aws_ec2`/`aws_athena`/`aws_eks`). `region` is
        // the per-call override every service route accepts.
        json!({"name":"otto.aws_list_accounts","mutating":false,"category":"AWS",
            "description":"List the AWS accounts configured in Otto — id, name, auth_mode, region, environment, identity and the cached per-service permission probe. Call first to obtain an `account_id`. Never includes secrets. Read-only.",
            "inputSchema":{"type":"object","properties":{}}}),
        json!({"name":"otto.aws_s3_list_buckets","mutating":false,"category":"AWS",
            "description":"List the S3 buckets of an AWS account (name, creation_date, region). S3 is read-only in Otto. Read-only.",
            "inputSchema":{"type":"object","required":["account_id"],"properties":{
                "account_id":{"type":"string","description":AWS_ACCOUNT_REF_DESC},"region":{"type":"string"}}}}),
        json!({"name":"otto.aws_s3_list_objects","mutating":false,"category":"AWS",
            "description":"List one folder level of an S3 bucket — `prefixes` + `objects` (key, size, last_modified, storage_class) under `prefix`; page with `token` = previous `next_token`, `max` per page. Read-only.",
            "inputSchema":{"type":"object","required":["account_id","bucket"],"properties":{
                "account_id":{"type":"string","description":AWS_ACCOUNT_REF_DESC},"bucket":{"type":"string"},"prefix":{"type":"string"},
                "token":{"type":"string"},"max":{"type":"integer"},"region":{"type":"string"}}}}),
        json!({"name":"otto.aws_s3_preview","mutating":false,"category":"AWS",
            "description":"Preview the first `max_bytes` (default 64 KiB, cap 1 MiB) of a text-like S3 object as `{text, truncated, content_type}`; binary objects return `{binary:true}`. Read-only.",
            "inputSchema":{"type":"object","required":["account_id","bucket","key"],"properties":{
                "account_id":{"type":"string","description":AWS_ACCOUNT_REF_DESC},"bucket":{"type":"string"},"key":{"type":"string"},
                "max_bytes":{"type":"integer"},"region":{"type":"string"}}}}),
        json!({"name":"otto.aws_sqs_list_queues","mutating":false,"category":"AWS",
            "description":"List an account's SQS queues (`url`, `name`, `fifo`); optional queue-name `prefix`. The `url` is the id the other SQS tools take. Read-only.",
            "inputSchema":{"type":"object","required":["account_id"],"properties":{
                "account_id":{"type":"string","description":AWS_ACCOUNT_REF_DESC},"prefix":{"type":"string"},"region":{"type":"string"}}}}),
        json!({"name":"otto.aws_sqs_peek","mutating":false,"category":"AWS",
            "description":"Peek up to `max` (1..10) messages on an SQS queue WITHOUT consuming them (receive with visibility timeout 0). Read-only.",
            "inputSchema":{"type":"object","required":["account_id","url"],"properties":{
                "account_id":{"type":"string","description":AWS_ACCOUNT_REF_DESC},"url":{"type":"string"},"max":{"type":"integer"},"region":{"type":"string"}}}}),
        json!({"name":"otto.aws_sqs_send","mutating":true,"category":"AWS",
            "description":"Send ONE message (`body`) to an SQS queue `url`; FIFO queues need `group_id` (+ `dedup_id` unless content-based dedup). Optional `delay_seconds`, `message_attributes`. Returns `{message_id}`. DANGEROUS: produces into a live queue — approval-gated.",
            "inputSchema":{"type":"object","required":["account_id","url","body"],"properties":{
                "account_id":{"type":"string","description":AWS_ACCOUNT_REF_DESC},"url":{"type":"string"},"body":{"type":"string"},
                "delay_seconds":{"type":"integer"},"group_id":{"type":"string"},"dedup_id":{"type":"string"},
                "message_attributes":{"type":"object"},"region":{"type":"string"}}}}),
        json!({"name":"otto.aws_ec2_list_instances","mutating":false,"category":"AWS",
            "description":"List EC2 instances (instance_id, name, state, type, az, ips, launch_time, tags); optional `region`, `state` filter and `q` free text. Start/stop/reboot are not exposed. Read-only.",
            "inputSchema":{"type":"object","required":["account_id"],"properties":{
                "account_id":{"type":"string","description":AWS_ACCOUNT_REF_DESC},"region":{"type":"string"},"state":{"type":"string"},"q":{"type":"string"}}}}),
        json!({"name":"otto.aws_athena_list_tables","mutating":false,"category":"AWS",
            "description":"List the tables (with columns) of an Athena/Glue `database`; optional `catalog` (default AwsDataCatalog). Read-only.",
            "inputSchema":{"type":"object","required":["account_id","database"],"properties":{
                "account_id":{"type":"string","description":AWS_ACCOUNT_REF_DESC},"database":{"type":"string"},"catalog":{"type":"string"},"region":{"type":"string"}}}}),
        json!({"name":"otto.aws_athena_query","mutating":true,"category":"AWS",
            "description":"START an Athena SQL query and return `{query_execution_id}` — does not wait; poll otto.aws_athena_get_query. Optional `database`, `workgroup`, `output_location`. DANGEROUS: Athena bills per byte scanned — approval-gated.",
            "inputSchema":{"type":"object","required":["account_id","sql"],"properties":{
                "account_id":{"type":"string","description":AWS_ACCOUNT_REF_DESC},"sql":{"type":"string"},"database":{"type":"string"},
                "workgroup":{"type":"string"},"output_location":{"type":"string"},"region":{"type":"string"}}}}),
        json!({"name":"otto.aws_athena_get_query","mutating":false,"category":"AWS",
            "description":"Status + results of an Athena query execution: `state` (QUEUED|RUNNING|SUCCEEDED|FAILED|CANCELLED), `reason`, `stats`, and once SUCCEEDED `result` {columns, rows}; page with `token`/`max`. Read-only.",
            "inputSchema":{"type":"object","required":["account_id","query_execution_id"],"properties":{
                "account_id":{"type":"string","description":AWS_ACCOUNT_REF_DESC},"query_execution_id":{"type":"string"},
                "token":{"type":"string"},"max":{"type":"integer"},"region":{"type":"string"}}}}),
        json!({"name":"otto.aws_eks_list_clusters","mutating":false,"category":"AWS",
            "description":"List the EKS clusters of an account/region (name, status, version, endpoint, arn, created_at). Read-only.",
            "inputSchema":{"type":"object","required":["account_id"],"properties":{
                "account_id":{"type":"string","description":AWS_ACCOUNT_REF_DESC},"region":{"type":"string"}}}}),
        // ================= Kubernetes console =================
        // §3 routes; everything is `kubectl` with the cluster's own kubeconfig
        // server-side. `kubernetes` feature: View for reads, Edit for k8s_action.
        json!({"name":"otto.k8s_list_clusters","mutating":false,"category":"Kubernetes",
            "description":"List the Kubernetes clusters registered in Otto — id, name, source, context_name, default_namespace, environment, cached capabilities (metrics_server/argo_rollouts/argocd). Call first to obtain a `cluster_id`. Read-only.",
            "inputSchema":{"type":"object","properties":{}}}),
        json!({"name":"otto.k8s_get_resources","mutating":false,"category":"Kubernetes",
            "description":"List resources of one `kind` (pods, deployments, statefulsets, daemonsets, replicasets, jobs, cronjobs, services, ingresses, configmaps, secrets, pvcs, hpas, rollouts, applications, events) as normalized rows with `health` and kind-specific `extra`. Omit `namespace` for all namespaces; optional `label` selector and `q` filter. Secret values are never returned. Read-only.",
            "inputSchema":{"type":"object","required":["cluster_id","kind"],"properties":{
                "cluster_id":{"type":"string","description":K8S_CLUSTER_REF_DESC},"kind":{"type":"string"},"namespace":{"type":"string"},
                "label":{"type":"string"},"q":{"type":"string"}}}}),
        json!({"name":"otto.k8s_describe","mutating":false,"category":"Kubernetes",
            "description":"One resource's `manifest` (managedFields stripped, Secret data redacted), `describe` text and recent `events`. `namespace` is required for namespaced kinds and omitted for cluster-scoped ones (nodes, namespaces). Read-only.",
            "inputSchema":{"type":"object","required":["cluster_id","kind","name"],"properties":{
                "cluster_id":{"type":"string","description":K8S_CLUSTER_REF_DESC},"kind":{"type":"string"},"namespace":{"type":"string"},"name":{"type":"string"}}}}),
        json!({"name":"otto.k8s_logs","mutating":false,"category":"Kubernetes",
            "description":"A pod's log tail as `{text}` (no follow). Optional `container`, `tail` (lines, default 500), `since` (e.g. 10m), `previous` (crashed instance), `timestamps`. Read-only.",
            "inputSchema":{"type":"object","required":["cluster_id","namespace","pod"],"properties":{
                "cluster_id":{"type":"string","description":K8S_CLUSTER_REF_DESC},"namespace":{"type":"string"},"pod":{"type":"string"},
                "container":{"type":"string"},"tail":{"type":"integer"},"since":{"type":"string"},
                "previous":{"type":"boolean"},"timestamps":{"type":"boolean"}}}}),
        json!({"name":"otto.k8s_top","mutating":false,"category":"Kubernetes",
            "description":"Live per-pod CPU (millicores) / memory (bytes) from metrics-server, optionally for one `namespace`; `available:false` when the cluster has none. Read-only.",
            "inputSchema":{"type":"object","required":["cluster_id"],"properties":{
                "cluster_id":{"type":"string","description":K8S_CLUSTER_REF_DESC},"namespace":{"type":"string"}}}}),
        json!({"name":"otto.k8s_health","mutating":false,"category":"Kubernetes",
            "description":"Compact health digest for a MONITORED cluster (Kubernetes → Monitor must be enabled): classified restarts (oom / crash / probe / unknown) with pod + memory-limit detail, planned churn (rollouts, scales, drains, Otto actions), memory outliers vs limits, error-rate and p95 spikes vs the 24h baseline, version drift, and the collector + metrics-server status (a `forbidden: …` message is the exact RBAC grant to ask for). `window` = 1h|6h|24h|7d (default 1h). Use this instead of k8s_get_resources for periodic health checks; every list is capped at 20 entries. Read-only.",
            "inputSchema":{"type":"object","required":["cluster_id"],"properties":{
                "cluster_id":{"type":"string","description":K8S_CLUSTER_REF_DESC},"window":{"type":"string","description":"1h|6h|24h|7d"}}}}),
        json!({"name":"otto.k8s_action","mutating":true,"category":"Kubernetes",
            "description":"Run ONE operational action on a resource via kubectl: restart, scale (params.replicas), delete_pod, rollout_status/undo/pause/resume, rollout_promote/abort/retry (Argo Rollouts), argocd_sync/refresh/terminate_op/app_restart, cronjob_trigger/suspend/resume. Destructive actions (delete_pod, scale to 0, rollout_undo, argocd_sync with prune) require `params.confirm_name == name`. DANGEROUS: mutates a live cluster — approval-gated.",
            "inputSchema":{"type":"object","required":["cluster_id","action","kind","namespace","name"],"properties":{
                "cluster_id":{"type":"string","description":K8S_CLUSTER_REF_DESC},"action":{"type":"string"},"kind":{"type":"string"},
                "namespace":{"type":"string"},"name":{"type":"string"},"params":{"type":"object"}}}}),
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

/// Short tool names exempted from the DANGEROUS approval gate — the
/// `mcp_approval_exempt_tools` setting (a JSON array of short names, e.g.
/// `["comment_pr"]`), managed by `PATCH /mcp/otto-server
/// {approval_exempt_tools}` (MCP Admin) — the MCP → Otto server "Ask before
/// each call" toggle — and pruned to the enabled set on every change.
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

/// Whether a governed `otto.*` call is subject to the human-approval gate
/// (before the global `mcp_require_approval_dangerous` switch): a DANGEROUS
/// tool, unless the operator exempted it (`mcp_approval_exempt_tools` — the
/// MCP → Otto server "Ask before each call" toggle) or the caller's
/// `kind='mcp'` token carries a trusted write grant. Pure, so the decision
/// table is unit-tested.
fn approval_gated(dangerous: bool, exempt: bool, token_write_grant: bool) -> bool {
    dangerous && !exempt && !token_write_grant
}

/// Validate + normalize a requested `mcp_approval_exempt_tools` list: bare
/// names (an `otto.` prefix is accepted), each a known MUTATING tool — the gate
/// only ever applies to those, so exempting a read would be a silent no-op —
/// de-duplicated in request order.
fn normalize_exempt_tools(requested: &[String]) -> Result<Vec<String>, Error> {
    let mut out: Vec<String> = Vec::with_capacity(requested.len());
    for t in requested {
        let bare = t
            .trim()
            .strip_prefix("otto.")
            .unwrap_or(t.trim())
            .to_string();
        if !DANGEROUS.contains(&bare.as_str()) {
            let known = otto_tool_specs()
                .iter()
                .any(|s| s["name"].as_str() == Some(&format!("otto.{bare}")));
            return Err(Error::Invalid(if known {
                format!("'{t}' is not a mutating tool — only mutating tools ask for approval")
            } else {
                format!("unknown otto tool '{t}'")
            }));
        }
        if !out.contains(&bare) {
            out.push(bare);
        }
    }
    Ok(out)
}

/// The exemptions that survive the enabled set: disabling a tool drops its
/// "don't ask" so re-enabling it later starts from the secure default (gated)
/// instead of silently inheriting an old skip.
fn prune_exempt_tools(exempt: &[String], enabled: &[String]) -> Vec<String> {
    exempt
        .iter()
        .filter(|t| enabled.contains(t))
        .cloned()
        .collect()
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
            let cad = match sched.and_then(|s| s.get("every_min")).and_then(i64_lenient) {
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
            args.get("number").and_then(i64_lenient).unwrap_or(0),
            args.get("repo_id").and_then(Value::as_str).unwrap_or("?")
        ),
        "merge_pr" => format!(
            "MERGE PR #{} of repo '{}' (strategy: {}{})",
            args.get("number").and_then(i64_lenient).unwrap_or(0),
            args.get("repo_id").and_then(Value::as_str).unwrap_or("?"),
            args.get("strategy")
                .and_then(Value::as_str)
                .unwrap_or("provider default"),
            if args
                .get("delete_source_branch")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                ", deleting the source branch"
            } else {
                ""
            }
        ),
        "start_pr_review" => format!(
            "Start a multi-agent review of PR #{} on repo '{}'",
            args.get("pr_number").and_then(i64_lenient).unwrap_or(0),
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
        "api_execute" => format!(
            "API request '{}' executed against environment '{}' (confirm={}, confirm_new_host={}) in workspace '{}'",
            args.get("request_id").and_then(Value::as_str).unwrap_or("?"),
            args.get("environment_id").and_then(Value::as_str).unwrap_or("active"),
            args.get("confirm").and_then(Value::as_bool).unwrap_or(false),
            args.get("confirm_new_host").and_then(Value::as_bool).unwrap_or(false),
            args.get("workspace_id").and_then(Value::as_str).unwrap_or("?")
        ),
        "api_upsert_request" => format!(
            "Save API request '{}' ({} {}) in workspace '{}'",
            args.get("name").and_then(Value::as_str).unwrap_or("?"),
            args.get("method")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_ascii_uppercase(),
            args.get("url").and_then(Value::as_str).unwrap_or("?"),
            args.get("workspace_id").and_then(Value::as_str).unwrap_or("?")
        ),
        "api_run_automation" => format!(
            "Run API automation '{}' in workspace '{}'",
            args.get("automation_id").and_then(Value::as_str).unwrap_or("?"),
            args.get("workspace_id").and_then(Value::as_str).unwrap_or("?")
        ),
        "post_swarm_board" => format!(
            "Post to swarm '{}' board",
            args.get("swarm_id").and_then(Value::as_str).unwrap_or("?")
        ),
        "assistant_remember" => {
            let text: String = args
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .chars()
                .take(160)
                .collect();
            format!("Remember about you (Otto Assistant memory): {text}")
        }
        "assistant_forget" => format!(
            "Forget your Otto Assistant memories matching '{}'",
            args.get("query").and_then(Value::as_str).unwrap_or("?")
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
        "open_session" => format!(
            "Open a new '{}' agent session in workspace '{}' and hand it an opening prompt",
            args.get("provider").and_then(Value::as_str).unwrap_or("?"),
            args.get("workspace_id").and_then(Value::as_str).unwrap_or("?")
        ),
        "send_message" => {
            let text: String = args
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .chars()
                .take(120)
                .collect();
            format!(
                "Send a message to agent session '{}': {text}",
                args.get("session_id").and_then(Value::as_str).unwrap_or("?")
            )
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
        // AWS / Kubernetes console writers: show the approver the exact target.
        "aws_athena_query" => {
            let sql: String = args
                .get("sql")
                .and_then(Value::as_str)
                .unwrap_or("")
                .chars()
                .take(200)
                .collect();
            format!(
                "Run an Athena query (billed per byte scanned) on AWS account '{}' (database '{}', workgroup '{}'): {sql}",
                args.get("account_id").and_then(Value::as_str).unwrap_or("?"),
                args.get("database").and_then(Value::as_str).unwrap_or("-"),
                args.get("workgroup").and_then(Value::as_str).unwrap_or("-")
            )
        }
        "aws_sqs_send" => format!(
            "Send a message to SQS queue '{}' on AWS account '{}'",
            args.get("url").and_then(Value::as_str).unwrap_or("?"),
            args.get("account_id").and_then(Value::as_str).unwrap_or("?")
        ),
        "k8s_action" => format!(
            "Kubernetes action '{}' on {}/{} in namespace '{}' of cluster '{}'",
            args.get("action").and_then(Value::as_str).unwrap_or("?"),
            args.get("kind").and_then(Value::as_str).unwrap_or("?"),
            args.get("name").and_then(Value::as_str).unwrap_or("?"),
            args.get("namespace").and_then(Value::as_str).unwrap_or("?"),
            args.get("cluster_id").and_then(Value::as_str).unwrap_or("?")
        ),
        "design_assist" => {
            let prompt: String = args
                .get("prompt")
                .and_then(Value::as_str)
                .unwrap_or("")
                .chars()
                .take(160)
                .collect();
            format!(
                "Start a design agent turn ({}) on design artifact '{}' — it spawns an agent session and commits a new version: {prompt}",
                args.get("mode").and_then(Value::as_str).unwrap_or("refine"),
                args.get("artifact_id").and_then(Value::as_str).unwrap_or("?")
            )
        }
        "design_link" => format!(
            "Link design artifact '{}' {} {} '{}'",
            args.get("artifact_id").and_then(Value::as_str).unwrap_or("?"),
            args.get("rel").and_then(Value::as_str).unwrap_or("?"),
            args.get("dst_kind").and_then(Value::as_str).unwrap_or("?"),
            args.get("dst_id").and_then(Value::as_str).unwrap_or("?")
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
    // `pr_number` / `number` aliases and stringified integers settle into the
    // one canonical spelling each route reads, before anything hashes them.
    let normalized = normalize_args(&short, arguments);
    let arguments = normalized.as_ref().unwrap_or(arguments);
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
        // A workspace NAME is resolved below and the pin re-checked on the
        // resolved id (`pin_verdict`); only a literal id is comparable here.
        let ws_arg = arguments
            .get("workspace_id")
            .and_then(Value::as_str)
            .filter(|w| crate::agent_refs::looks_like_id(w));
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

    // Git tools take a FRIENDLY repo reference (name / path / remote, or none
    // → the calling session's repo), resolved across every workspace the
    // caller can read. Deliberately AFTER the scope + enable gates above, so a
    // token that may not call this tool never learns the repo directory from a
    // resolution error. Like `fill_vault_workspace`, the resolved arguments
    // (canonical `repo_id` + the repo's own `workspace_id`) replace the
    // caller's for everything below — audit, approval scope, args-hash (so a
    // name and an id for the same repo reuse one approval), and execution.
    let repo_fill = match fill_repo_ref(ctx, auth, &short, arguments).await {
        Ok(fill) => fill,
        Err(Error::Forbidden(reason)) => {
            return Ok(deny_audit(ctx, &mut audit, &reason).await);
        }
        Err(e) => return Ok(unresolved_audit(ctx, &mut audit, &e).await),
    };
    let repo_label = repo_fill.as_ref().and_then(|f| f.label.clone());
    let arguments = repo_fill.as_ref().map_or(arguments, |f| &f.args);

    // Every OTHER friendly reference — a workspace / workflow / connection /
    // swarm / scheduled-task / cluster / account / vault / artifact NAME, a
    // Jira key for a story, a Confluence page URL, a Jira transition name —
    // resolved the same way (`agent_refs`: across every workspace the caller
    // can read, as the caller, candidates listed on a miss), plus the owning
    // workspace of an id-only object for a workspace-pinned token. Same
    // placement and contract as the repo fill: after the scope + enable gates,
    // and the resolved arguments replace the caller's for everything below.
    let ref_fill = match fill_refs(ctx, auth, &short, arguments).await {
        Ok(fill) => fill,
        Err(Error::Forbidden(reason)) => {
            return Ok(deny_audit(ctx, &mut audit, &reason).await);
        }
        Err(e) => return Ok(unresolved_audit(ctx, &mut audit, &e).await),
    };
    let arguments = ref_fill.as_ref().unwrap_or(arguments);
    // Design calls carry the ARTIFACT's workspace (never a caller-supplied
    // one), so a workspace-pinned token, the audit row and the approval scope
    // all see where the call really lands.
    let design_filled = fill_design_workspace(ctx, &short, arguments).await;
    let arguments = design_filled.as_ref().unwrap_or(arguments);
    if repo_fill.is_some() || ref_fill.is_some() || design_filled.is_some() {
        audit.args_redacted_json = otto_core::redact::redact_json(arguments).value.to_string();
    }
    // The workspace pin, re-checked against the RESOLVED arguments — the
    // workspace a named object, a repo or an artifact actually lives in. A
    // pinned token calling a tool whose workspace Otto cannot establish is
    // denied (fail closed) rather than let through unchecked.
    if let Some(scope) = &auth.mcp_scope {
        if let Some(reason) = pin_verdict(scope, &short, arguments) {
            return Ok(deny_audit(ctx, &mut audit, &reason).await);
        }
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
    let needs_approval = approval_gated(dangerous, exempt, token_write_grant)
        && require_approval_dangerous(ctx).await;
    let args_hash = canonical_hash(arguments);
    let ws = arguments
        .get("workspace_id")
        .and_then(Value::as_str)
        .map(str::to_string);

    if needs_approval && !dry_run {
        match ctx
            .mcp
            .approvals()
            .find_usable(ws.as_deref(), None, tool, &args_hash)
            .await
            .map_err(ApiError)?
        {
            Some(appr_id) => {
                if !ctx
                    .mcp
                    .approvals()
                    .consume(&appr_id)
                    .await
                    .map_err(ApiError)?
                {
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
                        detail: Some(match &repo_label {
                            // The resolved repo by name, so the approver isn't
                            // judging an opaque id.
                            Some(label) => {
                                format!("{} — repo {label}", dangerous_detail(tool, arguments))
                            }
                            None => dangerous_detail(tool, arguments),
                        }),
                        args_redacted_json: audit.args_redacted_json.clone(),
                        args_hash: Some(args_hash.clone()),
                        risk_label: Some("dangerous".into()),
                        requested_by: Some(user.id.clone()),
                        requested_by_kind: Some("mcp_server".into()),
                        expires_at: Some(
                            (chrono::Utc::now() + chrono::Duration::minutes(120)).to_rfc3339(),
                        ),
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
                        return Ok(
                            json!({"decision":"denied","executed":false,"reason":"human denied the request"}),
                        );
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
    audit.decision = if audit.approval_id.is_some() {
        "approved".into()
    } else {
        "allowed".into()
    };
    let audit_id = ctx.mcp.call_log().insert(audit).await.map_err(ApiError)?;

    let started = std::time::Instant::now();
    // An internal per-session MCP credential carries an immutable session
    // binding; for the room tools it OVERRIDES any client-supplied session_id
    // so a bound token can only ever speak as its own session's agent.
    let bound_session = auth.mcp_session_id.as_deref();
    let rebound;
    let arguments = match bound_session {
        Some(sid)
            if matches!(
                short.as_str(),
                "room_post"
                    | "room_read"
                    | "assistant_remember"
                    | "assistant_forget"
                    | "assistant_recall"
            ) =>
        {
            let mut a = arguments.clone();
            if let Some(o) = a.as_object_mut() {
                o.insert("session_id".into(), json!(sid));
            }
            rebound = a;
            &rebound
        }
        _ => arguments,
    };
    let result = match directory_kind(&short) {
        // Cross-workspace list tools: the `agent_refs` directory (as the
        // caller, pin applied), not a single-workspace route.
        Some(kind) => {
            let ws = arguments
                .get("workspace_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty());
            let current = crate::agent_refs::caller_session_ws(ctx, auth).await;
            crate::agent_refs::directory_json(ctx, auth, kind, ws, current.as_deref().or(ws)).await
        }
        None => execute_otto_tool(ctx, user, &short, arguments).await,
    };
    let latency = started.elapsed().as_millis() as i64;
    match result {
        Ok(value) => {
            let bytes = serde_json::to_vec(&value)
                .map(|v| v.len() as i64)
                .unwrap_or(0);
            let _ = ctx
                .mcp
                .call_log()
                .finalize(&audit_id, true, None, Some(latency), Some(bytes), None)
                .await;
            Ok(json!({"decision":"allowed","executed":true,"content":value}))
        }
        Err(e) => {
            let err = otto_core::redact::redact_text(&e.to_string()).value;
            let _ = ctx
                .mcp
                .call_log()
                .finalize(&audit_id, false, Some(&err), Some(latency), None, None)
                .await;
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

/// Audit + envelope for a reference (repo, workflow, account, …) that did not resolve (unknown or
/// ambiguous). Nothing executed, so it is an `error` row, not a denial; the
/// message lists the candidates / what IS available so the agent can retry
/// with a concrete id in one step.
async fn unresolved_audit(ctx: &ServerCtx, audit: &mut NewCallLog, err: &Error) -> Value {
    let msg = otto_core::redact::redact_text(&err.to_string()).value;
    audit.decision = "error".into();
    audit.decision_reason = Some("reference did not resolve".into());
    audit.error = Some(msg.clone());
    let _ = ctx.mcp.call_log().insert(audit.clone()).await;
    json!({"decision":"error","executed":false,"is_error":true,"content":{"error":msg}})
}

/// Poll an approval up to a bounded wait. `Some(true)`=approved, `Some(false)`=denied,
/// `None`=still pending after the wait (caller resubmits later).
async fn wait_for_decision(
    ctx: &ServerCtx,
    approval_id: &str,
    wait_seconds: Option<u64>,
) -> Option<bool> {
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
/// [`DESIGN_WS_TOOLS`] arguments with `workspace_id` set to the target
/// artifact's own workspace (overriding any caller value). `None` when the
/// tool doesn't address one artifact or it doesn't resolve (the self-call then
/// answers 404 under the caller's own RBAC).
async fn fill_design_workspace(ctx: &ServerCtx, tool: &str, args: &Value) -> Option<Value> {
    if !DESIGN_WS_TOOLS.contains(&tool) || !args.is_object() {
        return None;
    }
    let id = args
        .get("artifact_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;
    let a = crate::design_hall::service(ctx)
        .store()
        .get_artifact(id)
        .await
        .ok()
        .flatten()?;
    let mut filled = args.clone();
    filled["workspace_id"] = json!(a.workspace_id);
    Some(filled)
}

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
    // User-facing variants: the system-owned scratch workspace must never be
    // picked here — for root it is the oldest row, and a vault rooted at
    // `$HOME` is not what an omitted `workspace_id` means.
    let rows: Vec<(otto_core::domain::Workspace, WorkspaceRole)> = if user.is_root {
        repo.list_user_all()
            .await?
            .into_iter()
            .map(|w| (w, WorkspaceRole::Admin))
            .collect()
    } else {
        repo.list_user_for_user(&user.id).await?
    };
    let ws = pick_vault_workspace(&rows, tool_is_mutating(tool)).ok_or_else(|| {
        Error::Invalid(
            "no accessible workspace to scope this vault call — pass 'workspace_id'".into(),
        )
    })?;
    let mut filled = args.clone();
    filled["workspace_id"] = json!(ws);
    Ok(Some(filled))
}

// ===========================================================================
// Friendly references + the workspace pin for every non-git tool
// ===========================================================================

/// `(tool, argument, agent_refs kind)` — a governed argument that takes a
/// friendly reference (the id, or a name / title / Jira key / label) resolved
/// by [`fill_refs`] through `agent_refs`. A `Workspace`-scoped kind resolves
/// within the call's `workspace_id` when it carries one, else across every
/// workspace the caller can read (and then fills in the object's own
/// `workspace_id` for the pin, the audit and the approval scope).
pub(crate) const REF_ARGS: &[(&str, &str, &str)] = &[
    ("query_db_readonly", "connection_id", "connection"),
    ("get_workflow", "workflow_id", "workflow"),
    ("list_workflow_runs", "workflow_id", "workflow"),
    ("run_workflow", "workflow_id", "workflow"),
    ("list_broker_topics", "cluster_id", "broker_cluster"),
    ("get_broker_topic", "cluster_id", "broker_cluster"),
    ("list_consumer_groups", "cluster_id", "broker_cluster"),
    ("consume_broker_messages", "cluster_id", "broker_cluster"),
    ("produce_broker_message", "cluster_id", "broker_cluster"),
    ("api_get_request", "request_id", "api_request"),
    ("api_history", "request_id", "api_request"),
    ("api_execute", "request_id", "api_request"),
    ("api_execute", "environment_id", "api_environment"),
    ("api_upsert_request", "request_id", "api_request"),
    ("api_run_automation", "automation_id", "api_automation"),
    ("search_issues", "account_id", "issue_account"),
    ("get_issue", "account_id", "issue_account"),
    ("list_issue_transitions", "account_id", "issue_account"),
    ("comment_issue", "account_id", "issue_account"),
    ("transition_issue", "account_id", "issue_account"),
    ("search_confluence", "account_id", "issue_account"),
    ("get_confluence_page", "account_id", "issue_account"),
    (
        "list_confluence_page_comments",
        "account_id",
        "issue_account",
    ),
    ("create_confluence_page", "account_id", "issue_account"),
    ("update_confluence_page", "account_id", "issue_account"),
    ("comment_confluence_page", "account_id", "issue_account"),
    ("start_pr_review", "issue_account_id", "issue_account"),
    ("get_swarm", "swarm_id", "swarm"),
    ("list_swarm_runs", "swarm_id", "swarm"),
    ("list_swarm_projects", "swarm_id", "swarm"),
    ("get_swarm_board", "swarm_id", "swarm"),
    ("post_swarm_board", "swarm_id", "swarm"),
    ("vault_dir", "vault_id", "vault"),
    ("vault_read", "vault_id", "vault"),
    ("vault_search", "vault_id", "vault"),
    ("vault_backlinks", "vault_id", "vault"),
    ("vault_tags", "vault_id", "vault"),
    ("vault_graph", "vault_id", "vault"),
    ("vault_okf_validate", "vault_id", "vault"),
    ("vault_write", "vault_id", "vault"),
    ("vault_write_file", "vault_id", "vault"),
    ("vault_rename", "vault_id", "vault"),
    ("vault_delete", "vault_id", "vault"),
    ("design_get", "artifact_id", "design_artifact"),
    ("design_links", "artifact_id", "design_artifact"),
    ("design_assist", "artifact_id", "design_artifact"),
    ("design_link", "artifact_id", "design_artifact"),
    ("get_product_story", "story_id", "product_story"),
    ("get_context_packet", "story_id", "product_story"),
    ("get_scheduled_task", "task_id", "scheduled_task"),
    ("list_scheduled_task_runs", "task_id", "scheduled_task"),
    ("update_scheduled_task", "task_id", "scheduled_task"),
    ("set_scheduled_task_enabled", "task_id", "scheduled_task"),
    ("run_scheduled_task", "task_id", "scheduled_task"),
    ("delete_scheduled_task", "task_id", "scheduled_task"),
    ("create_scheduled_task", "workflow_id", "workflow"),
    ("update_scheduled_task", "workflow_id", "workflow"),
    ("get_proof_pack", "goal_loop_id", "goal_loop"),
    ("room_post", "room_id", "agent_room"),
    ("room_read", "room_id", "agent_room"),
    ("aws_s3_list_buckets", "account_id", "aws_account"),
    ("aws_s3_list_objects", "account_id", "aws_account"),
    ("aws_s3_preview", "account_id", "aws_account"),
    ("aws_sqs_list_queues", "account_id", "aws_account"),
    ("aws_sqs_peek", "account_id", "aws_account"),
    ("aws_sqs_send", "account_id", "aws_account"),
    ("aws_ec2_list_instances", "account_id", "aws_account"),
    ("aws_athena_list_tables", "account_id", "aws_account"),
    ("aws_athena_query", "account_id", "aws_account"),
    ("aws_athena_get_query", "account_id", "aws_account"),
    ("aws_eks_list_clusters", "account_id", "aws_account"),
    ("k8s_get_resources", "cluster_id", "k8s_cluster"),
    ("k8s_describe", "cluster_id", "k8s_cluster"),
    ("k8s_logs", "cluster_id", "k8s_cluster"),
    ("k8s_top", "cluster_id", "k8s_cluster"),
    ("k8s_health", "cluster_id", "k8s_cluster"),
    ("k8s_action", "cluster_id", "k8s_cluster"),
];

/// Governed list tools served by the cross-workspace `agent_refs` directory:
/// omit `workspace_id` to list every workspace the caller can read (a pinned
/// token: its pin), each row annotated with its workspace.
pub(crate) const DIRECTORY_TOOLS: &[(&str, &str)] = &[
    ("list_workspaces", "workspace"),
    ("list_workflows", "workflow"),
    ("list_connections", "connection"),
    ("list_broker_clusters", "broker_cluster"),
    ("list_swarms", "swarm"),
    ("list_scheduled_tasks", "scheduled_task"),
    ("list_goal_loops", "goal_loop"),
    ("list_agent_rooms", "agent_room"),
    ("list_issue_accounts", "issue_account"),
    ("list_product_stories", "product_story"),
];

/// The directory kind a governed list tool is served by, if any.
fn directory_kind(tool: &str) -> Option<&'static crate::agent_refs::RefKind> {
    DIRECTORY_TOOLS
        .iter()
        .find(|(t, _)| *t == tool)
        .and_then(|(_, k)| crate::agent_refs::kind_of(k))
}

/// Tools that address ONE design artifact: the artifact's own workspace is
/// filled in ([`fill_design_workspace`]).
const DESIGN_WS_TOOLS: &[&str] = &["design_get", "design_links", "design_assist", "design_link"];

/// Pin probes for id-only objects: `(tool, arg, route prefix, JSON pointer of
/// the object's workspace)`. Run only for a workspace-pinned token, whose pin
/// must be checked against the object's REAL workspace.
const PIN_PROBES: &[(&str, &str, &str, &str)] = &[
    (
        "get_workflow_run",
        "run_id",
        "/api/v1/workflow-runs/",
        "/workspace_id",
    ),
    (
        "cancel_workflow_run",
        "run_id",
        "/api/v1/workflow-runs/",
        "/workspace_id",
    ),
    (
        "get_session",
        "session_id",
        "/api/v1/sessions/",
        "/workspace_id",
    ),
    (
        "wait_session",
        "session_id",
        "/api/v1/sessions/",
        "/workspace_id",
    ),
    (
        "send_message",
        "session_id",
        "/api/v1/sessions/",
        "/workspace_id",
    ),
    (
        "get_improvement_run",
        "run_id",
        "/api/v1/improvement/runs/",
        "/run/workspace_id",
    ),
];

/// Tools that may run under a workspace pin WITHOUT a workspace — they
/// address no workspace-owned object: global rows (AWS accounts, K8s
/// clusters, the product-story and design libraries), the caller's own issue
/// accounts, root-only usage, the skill catalogue, and the directory tools
/// (which the pin itself narrows).
fn pin_global(tool: &str) -> bool {
    tool.starts_with("aws_")
        || tool.starts_with("k8s_")
        || DIRECTORY_TOOLS.iter().any(|(t, _)| *t == tool)
        || matches!(
            tool,
            "search_issues"
                | "get_issue"
                | "list_issue_transitions"
                | "comment_issue"
                | "transition_issue"
                | "search_confluence"
                | "get_confluence_page"
                | "list_confluence_page_comments"
                | "create_confluence_page"
                | "update_confluence_page"
                | "comment_confluence_page"
                | "get_product_story"
                | "get_usage_summary"
                | "list_bundled_skills"
        )
}

/// Tools that narrow to a workspace when given one: a pinned token that omits
/// it is narrowed to its pin (never widened to the whole library).
const PIN_NARROW_TOOLS: &[&str] = &[
    "design_list",
    "design_search",
    "list_design_projects",
    "ask_human_approval",
];

/// Tools whose target's workspace Otto cannot establish from the arguments
/// (no workspace argument, no name to resolve, no probe route) — a
/// workspace-pinned token is DENIED them. Listed so the classification test
/// forces a conscious choice for every new tool.
#[cfg(test)]
const PIN_UNVERIFIABLE: &[&str] = &[
    "create_work_item",
    "list_swarm_tasks",
    "list_findings",
    "get_finding",
    "approve_improvement_edit",
    "reject_improvement_edit",
    "rollback_improvement_edit",
];

/// The workspace-pin verdict on fully resolved arguments: `McpScope`'s own
/// check first; then, for a pinned token, a call that names no workspace is
/// allowed only for a [`pin_global`] tool. Pure — unit-tested.
fn pin_verdict(scope: &McpScope, tool: &str, args: &Value) -> Option<String> {
    let ws = args
        .get("workspace_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    if let Some(reason) = scope.deny_reason(tool, tool_is_mutating(tool), ws) {
        return Some(format!("token scope: {reason}"));
    }
    let pin = scope.workspace_id.as_deref().filter(|s| !s.is_empty())?;
    if ws.is_none() && !pin_global(tool) {
        return Some(format!(
            "token scope: this token is scoped to workspace '{pin}', and Otto cannot \
             establish which workspace this '{tool}' call touches — use a token without a \
             workspace pin for it"
        ));
    }
    None
}

/// Canonical argument spellings, before anything hashes them: the PR tools'
/// `number` / `pr_number` aliases collapse to the one each route reads, and a
/// stringified PR number becomes an integer. `None` when nothing changed.
/// Pure — unit-tested.
fn normalize_args(tool: &str, args: &Value) -> Option<Value> {
    let (want, alias) = match tool {
        "get_pr" | "comment_pr" | "get_pr_checks" | "get_pr_diff" | "merge_pr" => {
            ("number", "pr_number")
        }
        "start_pr_review" | "list_pr_reviews" => ("pr_number", "number"),
        _ => return None,
    };
    let obj = args.as_object()?;
    let value = obj.get(want).or_else(|| obj.get(alias))?;
    let n = i64_lenient(value)?;
    if obj.get(want) == Some(&json!(n)) && !obj.contains_key(alias) {
        return None;
    }
    let mut out = obj.clone();
    out.remove(alias);
    out.insert(want.to_string(), json!(n));
    Some(Value::Object(out))
}

/// A Confluence page id from a page id or URL: `…/pages/12345/Title`,
/// `…/pages/12345`, `…?pageId=12345`. `None` when nothing id-like is found.
/// Pure — unit-tested.
fn confluence_page_id(reference: &str) -> Option<String> {
    let r = reference.trim();
    if !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit()) {
        return Some(r.to_string());
    }
    if let Some(i) = r.find("pageId=") {
        let digits: String = r[i + 7..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if !digits.is_empty() {
            return Some(digits);
        }
    }
    if let Some(i) = r.find("/pages/") {
        let digits: String = r[i + 7..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if !digits.is_empty() {
            return Some(digits);
        }
    }
    None
}

/// Whether [`fill_refs`] needs directory lookups for this call (so a plain-id
/// call from an unpinned caller costs nothing extra). Pure — unit-tested.
fn refs_need_lookup(tool: &str, args: &Value, pinned: bool) -> bool {
    let s = |k: &str| -> Option<String> {
        match args.get(k)? {
            Value::String(v) if !v.trim().is_empty() => Some(v.trim().to_string()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    };
    if s("workspace_id").is_some_and(|w| !crate::agent_refs::looks_like_id(&w)) {
        return true;
    }
    for (t, arg, kind) in REF_ARGS {
        if *t != tool {
            continue;
        }
        let Some(k) = crate::agent_refs::kind_of(kind) else {
            continue;
        };
        match s(arg) {
            None => {
                if k.sole_default {
                    return true;
                }
            }
            Some(v) => {
                if !crate::agent_refs::looks_like_id(&v)
                    || (pinned && k.scope == crate::agent_refs::Scope::Workspace)
                {
                    return true;
                }
            }
        }
    }
    if tool == "transition_issue"
        && s("transition_id").is_some_and(|t| !t.bytes().all(|b| b.is_ascii_digit()))
    {
        return true;
    }
    pinned
        && s("workspace_id").is_none()
        && PIN_PROBES
            .iter()
            .any(|(t, arg, _, _)| *t == tool && s(arg).is_some())
}

/// Resolve every friendly reference in a governed call (see [`REF_ARGS`]), a
/// workspace NAME in `workspace_id`, a Confluence page URL, a Jira transition
/// name, and — for a pinned token — an id-only object's workspace
/// ([`PIN_PROBES`]) and the pin narrowing of [`PIN_NARROW_TOOLS`]. `None` when
/// the arguments are already canonical. Errors: `Forbidden` (pin / RBAC),
/// `NotFound` / `Conflict` listing the candidates.
async fn fill_refs(
    ctx: &ServerCtx,
    auth: &AuthContext,
    tool: &str,
    args: &Value,
) -> Result<Option<Value>, Error> {
    let Some(obj) = args.as_object() else {
        return Ok(None);
    };
    let pin = crate::agent_refs::pin_of(auth);
    let mut out = obj.clone();
    let mut changed = false;

    // Pure normalizations first (no lookups).
    for key in ["page_id", "parent_id"] {
        if let Some(raw) = out.get(key).and_then(Value::as_str) {
            if let Some(id) = confluence_page_id(raw).filter(|id| id != raw) {
                out.insert(key.to_string(), json!(id));
                changed = true;
            }
        }
    }
    let has_ws = out
        .get("workspace_id")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty());
    if let (Some(p), false) = (pin, has_ws) {
        if PIN_NARROW_TOOLS.contains(&tool) {
            out.insert("workspace_id".into(), json!(p));
            changed = true;
        }
    }

    let current = Value::Object(out.clone());
    if !refs_need_lookup(tool, &current, pin.is_some()) {
        return Ok(changed.then_some(current));
    }
    let prefer = crate::agent_refs::caller_session_ws(ctx, auth).await;
    let caller = crate::agent_refs::SelfCaller::open(ctx, &auth.effective_user).await?;
    let r = fill_refs_with(&caller, auth, tool, out, prefer.as_deref()).await;
    caller.close().await;
    r.map(Some)
}

/// The lookup half of [`fill_refs`], over an open caller.
async fn fill_refs_with(
    caller: &crate::agent_refs::SelfCaller,
    auth: &AuthContext,
    tool: &str,
    mut out: serde_json::Map<String, Value>,
    prefer: Option<&str>,
) -> Result<Value, Error> {
    use crate::agent_refs::{kind_of, looks_like_id, resolve_with, Scope};
    let pinned = crate::agent_refs::pin_of(auth).is_some();
    let text = |m: &serde_json::Map<String, Value>, k: &str| -> Option<String> {
        match m.get(k)? {
            Value::String(v) if !v.trim().is_empty() => Some(v.trim().to_string()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    };

    // 1. A workspace given by NAME.
    if let Some(ws) = text(&out, "workspace_id").filter(|w| !looks_like_id(w)) {
        let kind = kind_of("workspace").ok_or_else(|| Error::Internal("kind workspace".into()))?;
        let (c, _) = resolve_with(
            caller,
            auth,
            kind,
            "workspace_id",
            Some(ws.as_str()),
            None,
            prefer,
        )
        .await?;
        out.insert("workspace_id".into(), json!(c.id));
    }

    // 2. Friendly id arguments.
    for (t, arg, kind_key) in REF_ARGS {
        if *t != tool {
            continue;
        }
        let Some(kind) = kind_of(kind_key) else {
            continue;
        };
        let value = text(&out, arg);
        let needs = match &value {
            None => kind.sole_default,
            Some(v) => !looks_like_id(v) || (pinned && kind.scope == Scope::Workspace),
        };
        if !needs {
            continue;
        }
        let ws_filter = match kind.scope {
            Scope::Workspace => text(&out, "workspace_id"),
            _ => None,
        };
        let (c, _) = resolve_with(
            caller,
            auth,
            kind,
            arg,
            value.as_deref(),
            ws_filter.as_deref(),
            prefer,
        )
        .await?;
        let id = if kind.key == "vault" {
            c.id.parse::<i64>()
                .map(|n| json!(n))
                .unwrap_or_else(|_| json!(c.id))
        } else {
            json!(c.id)
        };
        out.insert((*arg).to_string(), id);
        if kind.scope == Scope::Workspace && ws_filter.is_none() {
            if let Some(ws) = &c.workspace_id {
                out.insert("workspace_id".into(), json!(ws));
            }
        }
    }

    // 3. A Jira transition given by name / target status.
    if tool == "transition_issue" {
        if let (Some(acc), Some(key), Some(tr)) = (
            text(&out, "account_id"),
            text(&out, "key"),
            text(&out, "transition_id"),
        ) {
            if !tr.bytes().all(|b| b.is_ascii_digit()) {
                let list = caller
                    .get(&format!(
                        "/api/v1/issue/{}/{}/transitions",
                        seg(&acc),
                        seg(&key)
                    ))
                    .await?;
                out.insert("transition_id".into(), json!(match_transition(&list, &tr)?));
            }
        }
    }

    // 4. Pinned token, id-only object: learn its real workspace.
    if pinned && text(&out, "workspace_id").is_none() {
        if let Some((_, arg, prefix, ptr)) = PIN_PROBES.iter().find(|(t, ..)| *t == tool) {
            if let Some(id) = text(&out, arg) {
                let v = caller.get(&format!("{prefix}{}", seg(&id))).await?;
                if let Some(ws) = v.pointer(ptr).and_then(Value::as_str) {
                    out.insert("workspace_id".into(), json!(ws));
                }
            }
        }
    }
    Ok(Value::Object(out))
}

/// Pick a Jira transition by id, name, or target status name
/// (case-insensitive) from `GET …/transitions` (`[{id, name, to_status}]`).
/// Unknown / ambiguous → an error listing the issue's transitions. Pure.
fn match_transition(list: &Value, wanted: &str) -> Result<String, Error> {
    let rows = list.as_array().map(Vec::as_slice).unwrap_or(&[]);
    let field = |r: &Value, k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let id_of = |r: &Value| match r.get("id") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    };
    let listing = || {
        rows.iter()
            .map(|r| {
                format!(
                    "- {}  {} → {}",
                    id_of(r),
                    field(r, "name"),
                    field(r, "to_status")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    for key in ["name", "to_status"] {
        let hits: Vec<&Value> = rows
            .iter()
            .filter(|r| field(r, key).eq_ignore_ascii_case(wanted.trim()))
            .collect();
        match hits.len() {
            0 => continue,
            1 => return Ok(id_of(hits[0])),
            _ => return Err(Error::Conflict(format!(
                "transition '{wanted}' matches {} transitions — pass one id as transition_id:\n{}",
                hits.len(),
                listing()
            ))),
        }
    }
    Err(Error::NotFound(format!(
        "no transition named '{wanted}' is available for this issue. Available:\n{}",
        listing()
    )))
}

/// Resolved arguments for a git tool (see [`fill_repo_ref`]).
struct RepoFill {
    args: Value,
    /// `name (workspace: X)` of the resolved repo, for the approval prompt.
    label: Option<String>,
}

/// The repo analogue of [`fill_vault_workspace`]. A repo is registered in
/// exactly ONE workspace, but the caller (often an agent session opened in a
/// different one) knows it by name, path or remote — so for
/// [`REPO_REF_TOOLS`] the `repo_id` argument is resolved across every
/// workspace the caller can read (`repo_directory::resolve_repo`: Git:View +
/// workspace Viewer, narrowed to a token's workspace pin; omitted → the
/// calling session's cwd) and rewritten to the canonical id, with the repo's
/// OWN `workspace_id` added so the pin check, the approval's workspace and
/// the args-hash all key on where the repo actually lives. `list_repos`
/// without a `workspace_id` from a pinned token is narrowed to the pin — the
/// executor's self-call is unpinned and would otherwise list every workspace.
/// `None` when the tool needs no filling.
async fn fill_repo_ref(
    ctx: &ServerCtx,
    auth: &AuthContext,
    tool: &str,
    args: &Value,
) -> Result<Option<RepoFill>, Error> {
    let ws_arg = args
        .get("workspace_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    if tool == "list_repos" {
        let pin = auth
            .mcp_scope
            .as_ref()
            .and_then(|s| s.workspace_id.as_deref())
            .filter(|s| !s.is_empty());
        return Ok(match (ws_arg, pin) {
            (None, Some(pin)) => {
                let mut filled = args.clone();
                filled["workspace_id"] = json!(pin);
                Some(RepoFill {
                    args: filled,
                    label: None,
                })
            }
            _ => None,
        });
    }
    if !REPO_REF_TOOLS.contains(&tool) {
        return Ok(None);
    }
    let reference = args.get("repo_id").and_then(Value::as_str);
    let resolved = crate::repo_directory::resolve_repo(ctx, auth, reference, ws_arg).await?;
    let mut filled = args.clone();
    filled["repo_id"] = json!(resolved.entry.repo.id);
    filled["workspace_id"] = json!(resolved.entry.repo.workspace_id);
    Ok(Some(RepoFill {
        args: filled,
        label: Some(resolved.label()),
    }))
}

/// Pick the workspace an omitted-`workspace_id` vault call is scoped to: the
/// first one whose role satisfies the tool (Editor for mutating, Viewer for
/// reads), falling back to the first membership so the self-call's native RBAC
/// produces the honest 403 rather than an "unknown workspace" here.
fn pick_vault_workspace(
    rows: &[(otto_core::domain::Workspace, WorkspaceRole)],
    mutating: bool,
) -> Option<Id> {
    let need = if mutating {
        WorkspaceRole::Editor
    } else {
        WorkspaceRole::Viewer
    };
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
        .timeout(call_timeout(tool, args))
        .build()
        .map_err(|e| Error::Internal(format!("http client: {e}")))?;
    let base = ctx.base_url.trim_end_matches('/').to_string();
    let result = if tool == "api_upsert_request" {
        upsert_request_preserving(&client, &base, &token, args).await
    } else {
        run_tool(&client, &base, &token, tool, args).await
    };
    let _ = AuthRepo::new(ctx.pool.clone()).revoke(&token).await;
    result
}

/// Wall-clock budget of one governed self-call. The flat 30 s cut off calls
/// whose route legitimately runs longer: a saved API request may itself wait
/// up to its `timeout_ms` (≤ 60 s), an automation runs several, pod logs shell
/// out to kubectl with a 60 s budget. Pure — unit-tested.
fn call_timeout(tool: &str, args: &Value) -> Duration {
    let secs = match tool {
        "api_execute" => {
            let ms = args
                .get("timeout_ms")
                .and_then(u64_lenient)
                .unwrap_or(30_000)
                .min(60_000);
            ms / 1000 + 15
        }
        "api_run_automation" => 180,
        "k8s_logs" | "k8s_action" | "aws_athena_query" | "aws_athena_get_query" => 75,
        "consume_broker_messages" | "run_workflow" | "start_pr_review" | "open_session" => 60,
        _ => 30,
    };
    Duration::from_secs(secs)
}

/// Fields of a saved API request that the PATCH route REPLACES wholesale when
/// omitted (only `auth` / `extras` are preserved server-side). An agent update
/// that sends just the field it means to change must not wipe the rest.
const UPSERT_PRESERVED: &[&str] = &[
    "collection_id",
    "headers",
    "query",
    "body_mode",
    "body",
    "ssh_connection_id",
];

/// Fill the fields an update omitted from the STORED request (read raw, as
/// the caller — never returned to the agent). Pure — unit-tested.
fn merge_stored_request(args: &Value, stored: &Value) -> Value {
    let mut out = args.clone();
    if let Some(o) = out.as_object_mut() {
        for k in UPSERT_PRESERVED {
            if !o.contains_key(*k) {
                if let Some(v) = stored.get(*k).filter(|v| !v.is_null()) {
                    o.insert((*k).to_string(), v.clone());
                }
            }
        }
    }
    out
}

/// `api_upsert_request`: on update, merge the stored request under the
/// agent's fields first (see [`UPSERT_PRESERVED`]); either way, return the
/// saved request in the masked agent shape — the route answers the raw row.
async fn upsert_request_preserving(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    args: &Value,
) -> Result<Value, Error> {
    let ws = arg_str(args, "workspace_id")?;
    let merged = match args
        .get("request_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        Some(id) => {
            let stored = self_get(
                client,
                token,
                &format!(
                    "{base}/api/v1/workspaces/{}/api-client/requests/{}",
                    seg(&ws),
                    seg(id)
                ),
            )
            .await?;
            merge_stored_request(args, &stored)
        }
        None => args.clone(),
    };
    let saved = run_tool(client, base, token, "api_upsert_request", &merged).await?;
    match saved.get("id").and_then(Value::as_str) {
        Some(id) => {
            self_get(
                client,
                token,
                &format!(
                    "{base}/api/v1/workspaces/{}/api-client/requests/{}?shape=agent",
                    seg(&ws),
                    seg(id)
                ),
            )
            .await
        }
        None => Ok(saved),
    }
}

fn arg_str(args: &Value, key: &str) -> Result<String, Error> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| Error::Invalid(format!("missing required string argument '{key}'")))
}

pub(crate) fn seg(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Required-integer argument extractor (PR numbers etc.). Accepts a numeric
/// string too — some MCP clients stringify every argument, and `"52"` must not
/// read as "missing".
fn arg_i64(args: &Value, key: &str) -> Result<i64, Error> {
    args.get(key)
        .and_then(i64_lenient)
        .ok_or_else(|| Error::Invalid(format!("missing required integer argument '{key}'")))
}

/// A JSON number, or a string holding one (see [`arg_i64`]).
fn i64_lenient(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

/// Unsigned counterpart of [`i64_lenient`] (limits, sizes, delays).
fn u64_lenient(v: &Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

/// Render the optional query filters of the AWS/K8s console reads: for every
/// `(query_key, arg_name)` whose argument is a non-empty string, a number or a
/// bool, append `&key=value` (strings percent-encoded). Returns the string with
/// the leading `&` stripped so callers can place it right after `?`.
fn opt_query(args: &Value, pairs: &[(&str, &str)]) -> String {
    let mut q = String::new();
    for (key, arg) in pairs {
        match args.get(arg) {
            Some(Value::String(s)) if !s.is_empty() => q.push_str(&format!("&{key}={}", seg(s))),
            Some(Value::Number(n)) => q.push_str(&format!("&{key}={n}")),
            Some(Value::Bool(b)) => q.push_str(&format!("&{key}={b}")),
            _ => {}
        }
    }
    q.trim_start_matches('&').to_string()
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
        Self {
            method: Method::Get,
            path,
            body: None,
        }
    }
    fn post(path: String, body: Value) -> Self {
        Self {
            method: Method::Post,
            path,
            body: Some(body),
        }
    }
    fn put(path: String, body: Value) -> Self {
        Self {
            method: Method::Put,
            path,
            body: Some(body),
        }
    }
    fn patch(path: String, body: Value) -> Self {
        Self {
            method: Method::Patch,
            path,
            body: Some(body),
        }
    }
    fn delete(path: String) -> Self {
        Self {
            method: Method::Delete,
            path,
            body: None,
        }
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
            let mut path = format!(
                "/api/v1/workspaces/{}/mcp/code-search?q={}",
                seg(&ws),
                seg(&q)
            );
            if let Some(p) = args.get("path").and_then(Value::as_str) {
                path.push_str(&format!("&path={}", seg(p)));
            }
            if let Some(m) = args.get("max_results").and_then(u64_lenient) {
                path.push_str(&format!("&max={m}"));
            }
            SelfCall::get(path)
        }
        "get_context_packet" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::post(
                format!("/api/v1/workspaces/{}/mcp/context-packet", seg(&ws)),
                args.clone(),
            )
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
            // Routed through `/db/mcp-query` (not `/db/query`): that path adds the
            // engine-specific read-only classifier AND executes inside the
            // engine's read-only mode (read-only transaction / ClickHouse
            // `readonly`), with cell masking forced — so "read-only" holds even
            // if a statement slips past the classifier above.
            let body = json!({
                "statement": stmt,
                "max_rows": args.get("max_rows").and_then(u64_lenient).unwrap_or(200),
                "node": args.get("node").and_then(Value::as_str),
            });
            SelfCall::post(
                format!("/api/v1/connections/{}/db/mcp-query", seg(&conn)),
                body,
            )
        }
        "list_connections" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::get(format!("/api/v1/workspaces/{}/connections", seg(&ws)))
        }
        // ---- API Client ----
        "api_list" => {
            let ws = arg_str(args, "workspace_id")?;
            let query = opt_query(
                args,
                &[
                    ("q", "q"),
                    ("collection_id", "collection_id"),
                    ("kind", "kind"),
                ],
            );
            let mut path = format!("/api/v1/workspaces/{}/api-client/overview", seg(&ws));
            if !query.is_empty() {
                path.push('?');
                path.push_str(&query);
            }
            SelfCall::get(path)
        }
        "api_get_request" => {
            let ws = arg_str(args, "workspace_id")?;
            let request = arg_str(args, "request_id")?;
            SelfCall::get(format!(
                "/api/v1/workspaces/{}/api-client/requests/{}?shape=agent",
                seg(&ws),
                seg(&request)
            ))
        }
        "api_history" => {
            let ws = arg_str(args, "workspace_id")?;
            if let Some(id) = args
                .get("id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                SelfCall::get(format!(
                    "/api/v1/workspaces/{}/api-client/history/{}",
                    seg(&ws),
                    seg(id)
                ))
            } else {
                let query = opt_query(
                    args,
                    &[
                        ("limit", "limit"),
                        ("q", "q"),
                        ("status", "status"),
                        ("request_id", "request_id"),
                        ("source", "source"),
                    ],
                );
                let mut path = format!("/api/v1/workspaces/{}/api-client/history", seg(&ws));
                if !query.is_empty() {
                    path.push('?');
                    path.push_str(&query);
                }
                SelfCall::get(path)
            }
        }
        "api_execute" => {
            let ws = arg_str(args, "workspace_id")?;
            let request = arg_str(args, "request_id")?;
            if let Some(vars) = args.get("vars").and_then(Value::as_object) {
                for (key, value) in vars {
                    if value.as_str().is_some_and(|s| s.contains("{{")) {
                        return Err(Error::Invalid(format!(
                            "vars override '{key}' must not contain '{{{{' (no nested substitution)"
                        )));
                    }
                }
            }
            let mut body = json!({"shape": "agent"});
            for key in [
                "environment_id",
                "vars",
                "timeout_ms",
                "confirm",
                "confirm_new_host",
                "decode_jwt",
            ] {
                if let Some(value) = args.get(key) {
                    body[key] = value.clone();
                }
            }
            SelfCall::post(
                format!(
                    "/api/v1/workspaces/{}/api-client/requests/{}/execute",
                    seg(&ws),
                    seg(&request)
                ),
                body,
            )
        }
        "api_upsert_request" => {
            let ws = arg_str(args, "workspace_id")?;
            let name = arg_str(args, "name")?;
            let method = arg_str(args, "method")?;
            let url = arg_str(args, "url")?;
            let mut body = json!({"name": name, "method": method, "url": url});
            for key in [
                "collection_id",
                "headers",
                "query",
                "body_mode",
                "body",
                "auth",
                "extras",
                "ssh_connection_id",
            ] {
                if let Some(value) = args.get(key) {
                    body[key] = value.clone();
                }
            }
            if let Some(request) = args
                .get("request_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                SelfCall::patch(
                    format!(
                        "/api/v1/workspaces/{}/api-client/requests/{}",
                        seg(&ws),
                        seg(request)
                    ),
                    body,
                )
            } else {
                SelfCall::post(
                    format!("/api/v1/workspaces/{}/api-client/requests", seg(&ws)),
                    body,
                )
            }
        }
        "api_run_automation" => {
            let ws = arg_str(args, "workspace_id")?;
            let automation = arg_str(args, "automation_id")?;
            SelfCall::post(
                format!(
                    "/api/v1/workspaces/{}/api-client/automations/{}/run",
                    seg(&ws),
                    seg(&automation)
                ),
                json!({}),
            )
        }
        // ---- Git ----
        "open_pr_draft" => {
            let repo = arg_str(args, "repo_id")?;
            let base_branch = arg_str(args, "base")?;
            SelfCall::post(
                format!("/api/v1/repos/{}/pr/draft", seg(&repo)),
                json!({"base": base_branch}),
            )
        }
        "list_repos" => {
            // Every workspace the caller can read (the workspace-annotated
            // directory), unless narrowed to one. A pinned token never gets
            // here without its pin filled in (`fill_repo_ref`).
            let mut path = "/api/v1/git/repos/directory".to_string();
            if let Some(ws) = args
                .get("workspace_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                path.push_str(&format!("?workspace_id={}", seg(ws)));
            }
            SelfCall::get(path)
        }
        "git_status" => {
            let repo = arg_str(args, "repo_id")?;
            SelfCall::get(format!("/api/v1/repos/{}/status", seg(&repo)))
        }
        "list_prs" => {
            let repo = arg_str(args, "repo_id")?;
            let q = opt_query(
                args,
                &[
                    ("state", "state"),
                    ("page", "page"),
                    ("per_page", "per_page"),
                ],
            );
            SelfCall::get(if q.is_empty() {
                format!("/api/v1/repos/{}/prs", seg(&repo))
            } else {
                format!("/api/v1/repos/{}/prs?{q}", seg(&repo))
            })
        }
        "list_pr_reviews" => {
            let repo = arg_str(args, "repo_id")?;
            let n = arg_i64(args, "pr_number")?;
            SelfCall::get(format!("/api/v1/repos/{}/prs/{}/reviews", seg(&repo), n))
        }
        "get_pr_checks" => {
            let repo = arg_str(args, "repo_id")?;
            let n = arg_i64(args, "number")?;
            SelfCall::get(format!("/api/v1/repos/{}/prs/{}/checks", seg(&repo), n))
        }
        "get_pr_diff" => {
            let repo = arg_str(args, "repo_id")?;
            let n = arg_i64(args, "number")?;
            SelfCall::get(format!("/api/v1/repos/{}/prs/{}/diff", seg(&repo), n))
        }
        "merge_pr" => {
            let repo = arg_str(args, "repo_id")?;
            let n = arg_i64(args, "number")?;
            let mut body = json!({});
            if let Some(st) = args
                .get("strategy")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                body["strategy"] = json!(st);
            }
            if let Some(d) = args.get("delete_source_branch").and_then(Value::as_bool) {
                body["delete_source_branch"] = json!(d);
            }
            SelfCall::post(
                format!("/api/v1/repos/{}/prs/{}/merge", seg(&repo), n),
                body,
            )
        }
        "get_pr" => {
            let repo = arg_str(args, "repo_id")?;
            let n = arg_i64(args, "number")?;
            SelfCall::get(format!("/api/v1/repos/{}/prs/{}", seg(&repo), n))
        }
        "create_pr" => {
            let repo = arg_str(args, "repo_id")?;
            let mut body = json!({
                "title": arg_str(args, "title")?,
                "description": arg_str(args, "description")?,
                "source_branch": arg_str(args, "source_branch")?,
                "target_branch": arg_str(args, "target_branch")?,
            });
            if let Some(d) = args.get("draft").and_then(Value::as_bool) {
                body["draft"] = json!(d);
            }
            if let Some(r) = args.get("reviewers").filter(|v| v.is_array()) {
                body["reviewers"] = r.clone();
            }
            if let Some(p) = args
                .get("proof_pack_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                body["proof_pack_id"] = json!(p);
            }
            SelfCall::post(format!("/api/v1/repos/{}/prs", seg(&repo)), body)
        }
        "comment_pr" => {
            let repo = arg_str(args, "repo_id")?;
            let n = arg_i64(args, "number")?;
            let body = json!({
                "body": arg_str(args, "body")?,
                "path": args.get("path").and_then(Value::as_str),
                "line": args.get("line").and_then(u64_lenient),
                "in_reply_to": args.get("in_reply_to").and_then(Value::as_str),
            });
            SelfCall::post(
                format!("/api/v1/repos/{}/prs/{}/comments", seg(&repo), n),
                body,
            )
        }
        "start_pr_review" => {
            let repo = arg_str(args, "repo_id")?;
            let n = arg_i64(args, "pr_number")?;
            let mut body = json!({});
            for k in ["context", "issue_key", "issue_account_id"] {
                if let Some(v) = args
                    .get(k)
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    body[k] = json!(v);
                }
            }
            SelfCall::post(
                format!("/api/v1/repos/{}/prs/{}/review", seg(&repo), n),
                body,
            )
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
            // Lightweight rows by default: the full rows (node states, inputs,
            // outputs × 50) blow the result cap for a busy workflow.
            let summary = args.get("summary").and_then(Value::as_bool).unwrap_or(true);
            SelfCall::get(format!(
                "/api/v1/workflows/{}/runs?summary={summary}",
                seg(&id)
            ))
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
            // Per-run review-mode override; the route 400s on anything but
            // "fan_out" / "orchestrator".
            if let Some(v) = args.get("review_mode").and_then(Value::as_str) {
                body["review_mode"] = json!(v);
            }
            SelfCall::post(format!("/api/v1/workflows/{}/run", seg(&id)), body)
        }
        "cancel_workflow_run" => {
            let id = arg_str(args, "run_id")?;
            SelfCall::post(
                format!("/api/v1/workflow-runs/{}/cancel", seg(&id)),
                json!({}),
            )
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
            SelfCall::get(format!(
                "/api/v1/brokers/clusters/{}/topics/{}",
                seg(&id),
                seg(&topic)
            ))
        }
        "list_consumer_groups" => {
            let id = arg_str(args, "cluster_id")?;
            SelfCall::get(format!("/api/v1/brokers/clusters/{}/groups", seg(&id)))
        }
        "consume_broker_messages" => {
            let id = arg_str(args, "cluster_id")?;
            let topic = arg_str(args, "topic")?;
            let mut body = json!({});
            if let Some(p) = args.get("partition").and_then(i64_lenient) {
                body["partition"] = json!(p);
            }
            if let Some(l) = args.get("limit").and_then(u64_lenient) {
                body["limit"] = json!(l);
            }
            if let Some(f) = args.get("value_filter").and_then(Value::as_str) {
                body["value_filter"] = json!(f);
            }
            SelfCall::post(
                format!(
                    "/api/v1/brokers/clusters/{}/topics/{}/consume",
                    seg(&id),
                    seg(&topic)
                ),
                body,
            )
        }
        "produce_broker_message" => {
            let id = arg_str(args, "cluster_id")?;
            let topic = arg_str(args, "topic")?;
            let mut body = json!({ "value": arg_str(args, "value")? });
            if let Some(k) = args.get("key").and_then(Value::as_str) {
                body["key"] = json!(k);
            }
            if let Some(p) = args.get("partition").and_then(i64_lenient) {
                body["partition"] = json!(p);
            }
            if let Some(c) = args.get("confirm").and_then(Value::as_bool) {
                body["confirm"] = json!(c);
            }
            SelfCall::post(
                format!(
                    "/api/v1/brokers/clusters/{}/topics/{}/produce",
                    seg(&id),
                    seg(&topic)
                ),
                body,
            )
        }
        // ---- Issues (Jira / Confluence) ----
        "search_issues" => {
            let acc = arg_str(args, "account_id")?;
            let mut path = format!("/api/v1/issue/search?account_id={}", seg(&acc));
            if let Some(q) = args.get("query").and_then(Value::as_str) {
                path.push_str(&format!("&q={}", seg(q)));
            }
            if let Some(p) = args
                .get("project")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                path.push_str(&format!("&project={}", seg(p)));
            }
            if let Some(n) = args.get("start_at").and_then(u64_lenient) {
                path.push_str(&format!("&start_at={n}"));
            }
            SelfCall::get(path)
        }
        "list_issue_transitions" => {
            let acc = arg_str(args, "account_id")?;
            let key = arg_str(args, "key")?;
            SelfCall::get(format!(
                "/api/v1/issue/{}/{}/transitions",
                seg(&acc),
                seg(&key)
            ))
        }
        "get_issue" => {
            let acc = arg_str(args, "account_id")?;
            let key = arg_str(args, "key")?;
            SelfCall::get(format!("/api/v1/issue/{}/{}/full", seg(&acc), seg(&key)))
        }
        "search_confluence" => {
            let acc = arg_str(args, "account_id")?;
            let q = arg_str(args, "query")?;
            let mut path = format!(
                "/api/v1/issue/confluence/search?account_id={}&q={}",
                seg(&acc),
                seg(&q)
            );
            if let Some(s) = args
                .get("space")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                path.push_str(&format!("&space={}", seg(s)));
            }
            SelfCall::get(path)
        }
        "get_confluence_page" => {
            let acc = arg_str(args, "account_id")?;
            let pid = arg_str(args, "page_id")?;
            SelfCall::get(format!(
                "/api/v1/issue/confluence/pages/{}?account_id={}",
                seg(&pid),
                seg(&acc)
            ))
        }
        "list_confluence_page_comments" => {
            let acc = arg_str(args, "account_id")?;
            let pid = arg_str(args, "page_id")?;
            SelfCall::get(format!(
                "/api/v1/issue/confluence/pages/{}/comments?account_id={}",
                seg(&pid),
                seg(&acc)
            ))
        }
        "create_confluence_page" => {
            let acc = arg_str(args, "account_id")?;
            let mut body = json!({
                "space_key": arg_str(args, "space_key")?,
                "title": arg_str(args, "title")?,
            });
            for k in ["body_md", "body_html"] {
                if let Some(v) = args
                    .get(k)
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    body[k] = json!(v);
                }
            }
            if let Some(p) = args
                .get("parent_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                body["parent_id"] = json!(p);
            }
            SelfCall::post(
                format!("/api/v1/issue/confluence/pages?account_id={}", seg(&acc)),
                body,
            )
        }
        "update_confluence_page" => {
            let acc = arg_str(args, "account_id")?;
            let pid = arg_str(args, "page_id")?;
            let mut body = json!({});
            for k in ["body_md", "body_html"] {
                if let Some(v) = args
                    .get(k)
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    body[k] = json!(v);
                }
            }
            if let Some(t) = args
                .get("title")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                body["title"] = json!(t);
            }
            if let Some(v) = args.get("base_version").and_then(i64_lenient) {
                body["base_version"] = json!(v);
            }
            SelfCall::put(
                format!(
                    "/api/v1/issue/confluence/pages/{}?account_id={}",
                    seg(&pid),
                    seg(&acc)
                ),
                body,
            )
        }
        "comment_confluence_page" => {
            let acc = arg_str(args, "account_id")?;
            let pid = arg_str(args, "page_id")?;
            let mut body = json!({});
            for k in ["body_md", "body_html"] {
                if let Some(v) = args
                    .get(k)
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    body[k] = json!(v);
                }
            }
            SelfCall::post(
                format!(
                    "/api/v1/issue/confluence/pages/{}/comments?account_id={}",
                    seg(&pid),
                    seg(&acc)
                ),
                body,
            )
        }
        "comment_issue" => {
            let acc = arg_str(args, "account_id")?;
            let key = arg_str(args, "key")?;
            let body = json!({ "body": arg_str(args, "body")? });
            SelfCall::post(
                format!("/api/v1/issue/{}/{}/comment", seg(&acc), seg(&key)),
                body,
            )
        }
        "transition_issue" => {
            let acc = arg_str(args, "account_id")?;
            let key = arg_str(args, "key")?;
            let body = json!({ "transition_id": arg_str(args, "transition_id")? });
            SelfCall::post(
                format!("/api/v1/issue/{}/{}/transitions", seg(&acc), seg(&key)),
                body,
            )
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
            let q = opt_query(args, &[("swarm_id", "swarm_id")]);
            SelfCall::get(if q.is_empty() {
                format!("/api/v1/workspaces/{}/swarm/runs", seg(&ws))
            } else {
                format!("/api/v1/workspaces/{}/swarm/runs?{q}", seg(&ws))
            })
        }
        "list_swarm_projects" => {
            let id = arg_str(args, "swarm_id")?;
            SelfCall::get(format!("/api/v1/swarm/swarms/{}/projects", seg(&id)))
        }
        "list_swarm_tasks" => {
            let id = arg_str(args, "project_id")?;
            SelfCall::get(format!("/api/v1/swarm/projects/{}/tasks", seg(&id)))
        }
        "get_swarm_board" => {
            let id = arg_str(args, "swarm_id")?;
            let q = opt_query(
                args,
                &[("project_id", "project_id"), ("task_id", "task_id")],
            );
            SelfCall::get(if q.is_empty() {
                format!("/api/v1/swarm/swarms/{}/board", seg(&id))
            } else {
                format!("/api/v1/swarm/swarms/{}/board?{q}", seg(&id))
            })
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
            if let Some(c) = args
                .get("collection")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                q.push(format!("collection={}", seg(c)));
            }
            if let Some(s) = args
                .get("story_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
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
            let k = args.get("k").and_then(u64_lenient).unwrap_or(20);
            let body = json!({ "text": arg_str(args, "query")?, "k": k });
            SelfCall::post(
                format!("/api/v1/workspaces/{}/memory/search", seg(&ws)),
                body,
            )
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
            SelfCall::get(format!(
                "/api/v1/workspaces/{}/vault/vaults/{v}/dir?path={}",
                seg(&ws),
                seg(path)
            ))
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
            let limit = args.get("limit").and_then(u64_lenient).unwrap_or(20);
            let body = json!({ "query": arg_str(args, "query")?, "limit": limit });
            SelfCall::post(
                format!("/api/v1/workspaces/{}/vault/vaults/{v}/search", seg(&ws)),
                body,
            )
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
            SelfCall::get(format!(
                "/api/v1/workspaces/{}/vault/vaults/{v}/tags",
                seg(&ws)
            ))
        }
        "vault_graph" => {
            let ws = arg_str(args, "workspace_id")?;
            let v = arg_i64(args, "vault_id")?;
            let mut path = format!("/api/v1/workspaces/{}/vault/vaults/{v}/graph", seg(&ws));
            let focus = args
                .get("path")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty());
            let mode = args
                .get("mode")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or(if focus.is_some() { "local" } else { "full" });
            path.push_str(&format!("?mode={}", seg(mode)));
            if let Some(f) = focus {
                path.push_str(&format!("&path={}", seg(f)));
            }
            if let Some(d) = args.get("depth").and_then(u64_lenient) {
                path.push_str(&format!("&depth={d}"));
            }
            SelfCall::get(path)
        }
        "vault_okf_validate" => {
            let ws = arg_str(args, "workspace_id")?;
            let v = arg_i64(args, "vault_id")?;
            SelfCall::post(
                format!(
                    "/api/v1/workspaces/{}/vault/vaults/{v}/okf/validate",
                    seg(&ws)
                ),
                json!({}),
            )
        }
        // ---- Design Hall (global library; `workspace_id` only narrows) ----
        "design_list" => {
            let q = opt_query(
                args,
                &[
                    ("workspace_id", "workspace_id"),
                    ("project_id", "project_id"),
                    ("studio", "studio"),
                    ("format", "format"),
                    ("status", "status"),
                    ("story_id", "story_id"),
                    ("limit", "limit"),
                    ("cursor", "cursor"),
                ],
            );
            SelfCall::get(if q.is_empty() {
                "/api/v1/design/artifacts".to_string()
            } else {
                format!("/api/v1/design/artifacts?{q}")
            })
        }
        "list_design_projects" => {
            let q = opt_query(
                args,
                &[
                    ("workspace_id", "workspace_id"),
                    ("include_archived", "include_archived"),
                ],
            );
            SelfCall::get(if q.is_empty() {
                "/api/v1/design/projects".to_string()
            } else {
                format!("/api/v1/design/projects?{q}")
            })
        }
        "design_get" => {
            let id = arg_str(args, "artifact_id")?;
            let content = args
                .get("include_content")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let mut path = format!("/api/v1/design/artifacts/{}?content={content}", seg(&id));
            if let Some(v) = args
                .get("version")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                path.push_str(&format!("&version={}", seg(v)));
            }
            SelfCall::get(path)
        }
        "design_links" => {
            let id = arg_str(args, "artifact_id")?;
            let dir = args
                .get("dir")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("both");
            SelfCall::get(format!(
                "/api/v1/design/artifacts/{}/links?dir={}",
                seg(&id),
                seg(dir)
            ))
        }
        "design_search" => {
            let query = arg_str(args, "query")?;
            let mut path = format!("/api/v1/design/search?q={}", seg(&query));
            let q = opt_query(
                args,
                &[
                    ("workspace_id", "workspace_id"),
                    ("studio", "studio"),
                    ("format", "format"),
                    ("status", "status"),
                    ("story_id", "story_id"),
                    ("project_id", "project_id"),
                    ("limit", "limit"),
                ],
            );
            if !q.is_empty() {
                path.push('&');
                path.push_str(&q);
            }
            SelfCall::get(path)
        }
        "design_assist" => {
            let id = arg_str(args, "artifact_id")?;
            let mut body = json!({ "prompt": arg_str(args, "prompt")? });
            for k in ["mode", "references", "selection"] {
                if let Some(v) = args.get(k).filter(|v| !v.is_null()) {
                    body[k] = v.clone();
                }
            }
            SelfCall::post(
                format!("/api/v1/design/artifacts/{}/assist", seg(&id)),
                body,
            )
        }
        "design_link" => {
            let id = arg_str(args, "artifact_id")?;
            let mut body = json!({
                "rel": arg_str(args, "rel")?,
                "dst_kind": arg_str(args, "dst_kind")?,
                "dst_id": arg_str(args, "dst_id")?,
            });
            for k in ["dst_node", "src_node", "policy", "pinned_version_id"] {
                if let Some(v) = args
                    .get(k)
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    body[k] = json!(v);
                }
            }
            SelfCall::post(format!("/api/v1/design/artifacts/{}/links", seg(&id)), body)
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
            SelfCall::put(
                format!("/api/v1/workspaces/{}/vault/vaults/{v}/note", seg(&ws)),
                body,
            )
        }
        "vault_write_file" => {
            let ws = arg_str(args, "workspace_id")?;
            let v = arg_i64(args, "vault_id")?;
            let mut body = json!({
                "path": arg_str(args, "path")?,
                "content": args.get("content").and_then(Value::as_str).unwrap_or(""),
            });
            if let Some(h) = args.get("if_hash").and_then(Value::as_str) {
                body["if_hash"] = json!(h);
            }
            SelfCall::put(
                format!("/api/v1/workspaces/{}/vault/vaults/{v}/file", seg(&ws)),
                body,
            )
        }
        "vault_rename" => {
            let ws = arg_str(args, "workspace_id")?;
            let v = arg_i64(args, "vault_id")?;
            let body = json!({ "from": arg_str(args, "from")?, "to": arg_str(args, "to")? });
            SelfCall::post(
                format!("/api/v1/workspaces/{}/vault/vaults/{v}/rename", seg(&ws)),
                body,
            )
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
        "open_session" => {
            let ws = arg_str(args, "workspace_id")?;
            let mut body = json!({ "provider": arg_str(args, "provider")? });
            for k in ["title", "cwd", "model", "prompt"] {
                if let Some(v) = args
                    .get(k)
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
                {
                    body[k] = json!(v);
                }
            }
            SelfCall::post(
                format!("/api/v1/workspaces/{}/sessions/open", seg(&ws)),
                body,
            )
        }
        "send_message" => {
            let id = arg_str(args, "session_id")?;
            let body = json!({ "text": arg_str(args, "text")? });
            SelfCall::post(format!("/api/v1/sessions/{}/message", seg(&id)), body)
        }
        "wait_session" => {
            let id = arg_str(args, "session_id")?;
            let mut path = format!("/api/v1/sessions/{}/wait?", seg(&id));
            let q = opt_query(
                args,
                &[("status", "status"), ("timeout_secs", "timeout_secs")],
            );
            path.push_str(&q);
            SelfCall::get(path)
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
            SelfCall::post(
                format!(
                    "/api/v1/workspaces/{}/integrations/{}/test",
                    seg(&ws),
                    seg(&ch)
                ),
                json!({}),
            )
        }
        // ---- Usage ----
        "get_usage_summary" => {
            let mut q: Vec<String> = Vec::new();
            if let Some(d) = args.get("days").and_then(u64_lenient) {
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
            let q = opt_query(args, &[("status", "status")]);
            SelfCall::get(if q.is_empty() {
                format!("/api/v1/workspaces/{}/improvement/edits", seg(&ws))
            } else {
                format!("/api/v1/workspaces/{}/improvement/edits?{q}", seg(&ws))
            })
        }
        "run_self_improvement" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::post(
                format!("/api/v1/workspaces/{}/self-improvement/run", seg(&ws)),
                json!({}),
            )
        }
        "approve_improvement_edit" => {
            let id = arg_str(args, "edit_id")?;
            SelfCall::post(
                format!("/api/v1/improvement/edits/{}/approve", seg(&id)),
                json!({}),
            )
        }
        "reject_improvement_edit" => {
            let id = arg_str(args, "edit_id")?;
            SelfCall::post(
                format!("/api/v1/improvement/edits/{}/reject", seg(&id)),
                json!({}),
            )
        }
        "rollback_improvement_edit" => {
            let id = arg_str(args, "edit_id")?;
            SelfCall::post(
                format!("/api/v1/improvement/edits/{}/rollback", seg(&id)),
                json!({}),
            )
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
            SelfCall::post(
                format!("/api/v1/swarm/projects/{}/tasks", seg(&project)),
                body,
            )
        }
        "room_post" => {
            let room = arg_str(args, "room_id")?;
            let text = arg_str(args, "text")?;
            let mut body = json!({"text": text});
            if let Some(sid) = args
                .get("session_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                body["session_id"] = json!(sid);
            }
            SelfCall::post(format!("/api/v1/agent-rooms/{}/messages", seg(&room)), body)
        }
        "room_read" => {
            let room = arg_str(args, "room_id")?;
            let mut q = String::new();
            if let Some(after) = args
                .get("after")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                q.push_str(&format!("&after={}", seg(after)));
            }
            if let Some(limit) = args.get("limit").and_then(i64_lenient) {
                q.push_str(&format!("&limit={limit}"));
            }
            if let Some(sid) = args
                .get("session_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                q.push_str(&format!("&session_id={}", seg(sid)));
            }
            SelfCall::get(format!(
                "/api/v1/agent-rooms/{}/messages?{}",
                seg(&room),
                q.trim_start_matches('&')
            ))
        }
        // Otto Assistant memory: one agent-tool endpoint per verb; the body
        // carries the (bound) calling session so chips land in its thread.
        "assistant_remember" | "assistant_forget" | "assistant_recall" => {
            let verb = tool.strip_prefix("assistant_").unwrap_or(tool);
            let mut body = serde_json::Map::new();
            for k in ["text", "kind", "tags", "query", "k", "session_id"] {
                if let Some(v) = args.get(k).filter(|v| !v.is_null()) {
                    body.insert(k.to_string(), v.clone());
                }
            }
            if verb == "remember" {
                arg_str(args, "text")?;
            }
            if verb == "forget" {
                arg_str(args, "query")?;
            }
            SelfCall::post(
                format!("/api/v1/assistant/agent/{verb}"),
                Value::Object(body),
            )
        }
        "list_scheduled_tasks" => {
            let ws = arg_str(args, "workspace_id")?;
            SelfCall::get(format!("/api/v1/workspaces/{}/scheduled-tasks", seg(&ws)))
        }
        "get_scheduled_task" => {
            let id = arg_str(args, "task_id")?;
            SelfCall::get(format!("/api/v1/scheduled-tasks/{}", seg(&id)))
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
            SelfCall::post(
                format!("/api/v1/workspaces/{}/scheduled-tasks", seg(&ws)),
                body,
            )
        }
        "update_scheduled_task" => {
            let id = arg_str(args, "task_id")?;
            let mut body = args.clone();
            if let Some(o) = body.as_object_mut() {
                o.remove("task_id");
                // Filled by the choke point (the task's own workspace, for the
                // pin + approval scope) — not an updatable field.
                o.remove("workspace_id");
            }
            SelfCall::patch(format!("/api/v1/scheduled-tasks/{}", seg(&id)), body)
        }
        "set_scheduled_task_enabled" => {
            let id = arg_str(args, "task_id")?;
            let enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            SelfCall::patch(
                format!("/api/v1/scheduled-tasks/{}", seg(&id)),
                json!({"enabled": enabled}),
            )
        }
        "run_scheduled_task" => {
            let id = arg_str(args, "task_id")?;
            SelfCall::post(
                format!("/api/v1/scheduled-tasks/{}/run", seg(&id)),
                json!({}),
            )
        }
        "delete_scheduled_task" => {
            let id = arg_str(args, "task_id")?;
            SelfCall::delete(format!("/api/v1/scheduled-tasks/{}", seg(&id)))
        }
        // ---- AWS console (docs/design/aws-k8s-consoles.md §2) ----
        "aws_list_accounts" => SelfCall::get("/api/v1/aws/accounts".into()),
        "aws_s3_list_buckets" => SelfCall::get(format!(
            "/api/v1/aws/accounts/{}/s3/buckets?{}",
            seg(&arg_str(args, "account_id")?),
            opt_query(args, &[("region", "region")])
        )),
        "aws_s3_list_objects" => SelfCall::get(format!(
            "/api/v1/aws/accounts/{}/s3/buckets/{}/objects?{}",
            seg(&arg_str(args, "account_id")?),
            seg(&arg_str(args, "bucket")?),
            opt_query(
                args,
                &[
                    ("prefix", "prefix"),
                    ("token", "token"),
                    ("max", "max"),
                    ("region", "region")
                ]
            )
        )),
        "aws_s3_preview" => {
            let extra = opt_query(args, &[("max_bytes", "max_bytes"), ("region", "region")]);
            let mut path = format!(
                "/api/v1/aws/accounts/{}/s3/buckets/{}/preview?key={}",
                seg(&arg_str(args, "account_id")?),
                seg(&arg_str(args, "bucket")?),
                seg(&arg_str(args, "key")?)
            );
            if !extra.is_empty() {
                path.push('&');
                path.push_str(&extra);
            }
            SelfCall::get(path)
        }
        "aws_sqs_list_queues" => SelfCall::get(format!(
            "/api/v1/aws/accounts/{}/sqs/queues?{}",
            seg(&arg_str(args, "account_id")?),
            opt_query(args, &[("prefix", "prefix"), ("region", "region")])
        )),
        "aws_sqs_peek" => {
            // Read-only POST: receive-message with visibility timeout pinned to
            // 0 (nothing consumed); `max` clamped to SQS's 1..10 window.
            let mut body = json!({"url": arg_str(args, "url")?, "visibility_timeout": 0});
            if let Some(max) = args.get("max").and_then(u64_lenient) {
                body["max"] = json!(max.clamp(1, 10));
            }
            SelfCall::post(
                format!(
                    "/api/v1/aws/accounts/{}/sqs/queues/peek?{}",
                    seg(&arg_str(args, "account_id")?),
                    opt_query(args, &[("region", "region")])
                ),
                body,
            )
        }
        "aws_sqs_send" => {
            let mut body = json!({"url": arg_str(args, "url")?, "body": arg_str(args, "body")?});
            if let Some(d) = args.get("delay_seconds").and_then(u64_lenient) {
                body["delay_seconds"] = json!(d);
            }
            for k in ["group_id", "dedup_id"] {
                if let Some(v) = args
                    .get(k)
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    body[k] = json!(v);
                }
            }
            if let Some(attrs) = args.get("message_attributes").filter(|v| v.is_object()) {
                body["message_attributes"] = attrs.clone();
            }
            SelfCall::post(
                format!(
                    "/api/v1/aws/accounts/{}/sqs/queues/send?{}",
                    seg(&arg_str(args, "account_id")?),
                    opt_query(args, &[("region", "region")])
                ),
                body,
            )
        }
        "aws_ec2_list_instances" => SelfCall::get(format!(
            "/api/v1/aws/accounts/{}/ec2/instances?{}",
            seg(&arg_str(args, "account_id")?),
            opt_query(
                args,
                &[("region", "region"), ("state", "state"), ("q", "q")]
            )
        )),
        "aws_athena_list_tables" => {
            let extra = opt_query(args, &[("catalog", "catalog"), ("region", "region")]);
            let mut path = format!(
                "/api/v1/aws/accounts/{}/athena/tables?database={}",
                seg(&arg_str(args, "account_id")?),
                seg(&arg_str(args, "database")?)
            );
            if !extra.is_empty() {
                path.push('&');
                path.push_str(&extra);
            }
            SelfCall::get(path)
        }
        "aws_athena_query" => {
            let mut body = json!({"sql": arg_str(args, "sql")?});
            for k in ["database", "workgroup", "output_location"] {
                if let Some(v) = args
                    .get(k)
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    body[k] = json!(v);
                }
            }
            SelfCall::post(
                format!(
                    "/api/v1/aws/accounts/{}/athena/query?{}",
                    seg(&arg_str(args, "account_id")?),
                    opt_query(args, &[("region", "region")])
                ),
                body,
            )
        }
        "aws_athena_get_query" => SelfCall::get(format!(
            "/api/v1/aws/accounts/{}/athena/query/{}?{}",
            seg(&arg_str(args, "account_id")?),
            seg(&arg_str(args, "query_execution_id")?),
            opt_query(
                args,
                &[("token", "token"), ("max", "max"), ("region", "region")]
            )
        )),
        "aws_eks_list_clusters" => SelfCall::get(format!(
            "/api/v1/aws/accounts/{}/eks/clusters?{}",
            seg(&arg_str(args, "account_id")?),
            opt_query(args, &[("region", "region")])
        )),
        // ---- Kubernetes console (§3) — `namespace` → `ns`; omitted ⇒ all (-A) ----
        "k8s_list_clusters" => SelfCall::get("/api/v1/k8s/clusters".into()),
        "k8s_get_resources" => {
            let extra = opt_query(args, &[("ns", "namespace"), ("label", "label"), ("q", "q")]);
            let mut path = format!(
                "/api/v1/k8s/clusters/{}/resources?kind={}",
                seg(&arg_str(args, "cluster_id")?),
                seg(&arg_str(args, "kind")?)
            );
            if !extra.is_empty() {
                path.push('&');
                path.push_str(&extra);
            }
            SelfCall::get(path)
        }
        // `ns` is omitted for cluster-scoped kinds (nodes, namespaces); the
        // route requires it for namespaced ones and says so.
        "k8s_describe" => {
            let mut path = format!(
                "/api/v1/k8s/clusters/{}/resource?kind={}&name={}",
                seg(&arg_str(args, "cluster_id")?),
                seg(&arg_str(args, "kind")?),
                seg(&arg_str(args, "name")?)
            );
            if let Some(ns) = args
                .get("namespace")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                path.push_str(&format!("&ns={}", seg(ns)));
            }
            SelfCall::get(path)
        }
        // text/plain route — `run_tool` wraps the body as `{text}`; `follow` is
        // deliberately never forwarded (a stream would hang the call).
        "k8s_logs" => SelfCall::get(format!(
            "/api/v1/k8s/clusters/{}/pods/{}/{}/logs?{}",
            seg(&arg_str(args, "cluster_id")?),
            seg(&arg_str(args, "namespace")?),
            seg(&arg_str(args, "pod")?),
            opt_query(
                args,
                &[
                    ("container", "container"),
                    ("tail", "tail"),
                    ("since", "since"),
                    ("previous", "previous"),
                    ("timestamps", "timestamps"),
                ]
            )
        )),
        "k8s_top" => SelfCall::get(format!(
            "/api/v1/k8s/clusters/{}/metrics?{}",
            seg(&arg_str(args, "cluster_id")?),
            opt_query(args, &[("ns", "namespace")])
        )),
        "k8s_health" => SelfCall::get(format!(
            "/api/v1/k8s/clusters/{}/monitor/health?{}",
            seg(&arg_str(args, "cluster_id")?),
            opt_query(args, &[("window", "window")])
        )),
        "k8s_action" => SelfCall::post(
            format!(
                "/api/v1/k8s/clusters/{}/actions",
                seg(&arg_str(args, "cluster_id")?)
            ),
            json!({
                "action": arg_str(args, "action")?,
                "kind": arg_str(args, "kind")?,
                "ns": arg_str(args, "namespace")?,
                "name": arg_str(args, "name")?,
                "params": args.get("params").cloned().unwrap_or(json!({})),
            }),
        ),
        other => return Err(Error::Invalid(format!("unknown otto tool '{other}'"))),
    })
}

/// Tools whose route answers `text/plain` rather than JSON; [`run_tool`] wraps
/// the body as `{"text": …}` for them (today only the pod-logs route).
const TEXT_TOOLS: &[&str] = &["k8s_logs"];
/// Cap on the text handed back for a [`TEXT_TOOLS`] call (keeps the tail —
/// the newest log lines). The route's own non-follow cap is 5 MiB.
const MAX_TEXT_CHARS: usize = 256 * 1024;

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
    if TEXT_TOOLS.contains(&tool) && call.method == Method::Get {
        return self_get_text(client, token, &url).await;
    }
    match call.method {
        Method::Get => self_get(client, token, &url).await,
        Method::Post => self_post(client, token, &url, body).await,
        Method::Put => self_put(client, token, &url, body).await,
        Method::Patch => self_patch(client, token, &url, body).await,
        Method::Delete => self_delete(client, token, &url).await,
    }
}

async fn self_get(client: &reqwest::Client, token: &str, url: &str) -> Result<Value, Error> {
    let resp = client
        .get(url)
        .bearer_auth(token)
        .header("X-Otto-Agent", "mcp-outward")
        .send()
        .await
        .map_err(|e| Error::Upstream(format!("self-call: {e}")))?;
    parse_self(resp).await
}
async fn self_post(
    client: &reqwest::Client,
    token: &str,
    url: &str,
    body: &Value,
) -> Result<Value, Error> {
    let resp = client
        .post(url)
        .bearer_auth(token)
        .header("X-Otto-Agent", "mcp-outward")
        .json(body)
        .send()
        .await
        .map_err(|e| Error::Upstream(format!("self-call: {e}")))?;
    parse_self(resp).await
}
async fn self_put(
    client: &reqwest::Client,
    token: &str,
    url: &str,
    body: &Value,
) -> Result<Value, Error> {
    let resp = client
        .put(url)
        .bearer_auth(token)
        .header("X-Otto-Agent", "mcp-outward")
        .json(body)
        .send()
        .await
        .map_err(|e| Error::Upstream(format!("self-call: {e}")))?;
    parse_self(resp).await
}
async fn self_patch(
    client: &reqwest::Client,
    token: &str,
    url: &str,
    body: &Value,
) -> Result<Value, Error> {
    let resp = client
        .patch(url)
        .bearer_auth(token)
        .header("X-Otto-Agent", "mcp-outward")
        .json(body)
        .send()
        .await
        .map_err(|e| Error::Upstream(format!("self-call: {e}")))?;
    parse_self(resp).await
}
async fn self_delete(client: &reqwest::Client, token: &str, url: &str) -> Result<Value, Error> {
    let resp = client
        .delete(url)
        .bearer_auth(token)
        .header("X-Otto-Agent", "mcp-outward")
        .send()
        .await
        .map_err(|e| Error::Upstream(format!("self-call: {e}")))?;
    parse_self(resp).await
}
async fn parse_self(resp: reqwest::Response) -> Result<Value, Error> {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(Error::Upstream(self_call_error(status, &text)));
    }
    Ok(parse_self_ok(&text))
}

/// Cap on a self-call error MESSAGE handed back to the agent. The message is
/// often the actionable part (a reference that did not resolve lists the
/// candidates, a validation error names the bad field), so it is kept whole up
/// to this bound — the old 400-char raw snippet cut candidate lists mid-row.
const MAX_ERROR_MESSAGE_CHARS: usize = 4000;

/// The agent-facing text of a non-2xx self-call: `"<status>: <message>"`, the
/// `message` (or a module's `error`) of a JSON problem body when there is one
/// (≤ [`MAX_ERROR_MESSAGE_CHARS`]), else a short raw snippet so an HTML/huge
/// body never floods the transcript. Pure, so it is unit-tested.
pub(crate) fn self_call_error(status: reqwest::StatusCode, body: &str) -> String {
    let message = serde_json::from_str::<Value>(body).ok().and_then(|v| {
        v.get("message")
            .or_else(|| v.get("error"))
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    match message {
        Some(m) => format!(
            "{status}: {}",
            m.chars().take(MAX_ERROR_MESSAGE_CHARS).collect::<String>()
        ),
        None => format!("{status}: {}", body.chars().take(400).collect::<String>()),
    }
}

/// A 2xx self-call body as JSON. An empty body (`204 No Content` — a Jira
/// transition, a delete) is `{"ok": true}`, not `null`, so the agent reads a
/// success as one.
pub(crate) fn parse_self_ok(body: &str) -> Value {
    if body.trim().is_empty() {
        return json!({ "ok": true });
    }
    serde_json::from_str(body).unwrap_or(Value::Null)
}
/// GET a `text/plain` route (pod logs) and wrap it as `{text, truncated}`,
/// keeping the newest [`MAX_TEXT_CHARS`] — `parse_self` would turn a non-JSON
/// body into `null`.
async fn self_get_text(client: &reqwest::Client, token: &str, url: &str) -> Result<Value, Error> {
    let resp = client
        .get(url)
        .bearer_auth(token)
        .header("X-Otto-Agent", "mcp-outward")
        .send()
        .await
        .map_err(|e| Error::Upstream(format!("self-call: {e}")))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(Error::Upstream(self_call_error(status, &text)));
    }
    let n = text.chars().count();
    let (text, truncated) = if n > MAX_TEXT_CHARS {
        let start = text
            .char_indices()
            .nth(n - MAX_TEXT_CHARS)
            .map(|(i, _)| i)
            .unwrap_or(text.len());
        (text[start..].to_string(), true)
    } else {
        (text, false)
    };
    Ok(json!({"text": text, "truncated": truncated}))
}

async fn ask_human_approval(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    args: &Value,
) -> Result<Value, Error> {
    let title = arg_str(args, "title")?;
    let ws = args
        .get("workspace_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let detail = args
        .get("detail")
        .and_then(Value::as_str)
        .map(str::to_string);
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
    let wait = args.get("wait_seconds").and_then(u64_lenient);
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
    let exempt = approval_exempt_tools(&ctx).await;
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
                // "Ask before each call" is OFF for this tool: an operator
                // opted it out of the approval gate (calls stay audited).
                "approval_exempt": exempt.contains(&short),
            })
        })
        .collect();
    let prefix = AuthRepo::new(ctx.pool.clone())
        .mcp_token_prefix(&user.id)
        .await
        .map_err(ApiError)?;
    let require_dangerous = require_approval_dangerous(&ctx).await;
    Ok(Json(json!({
        "enabled": enabled,
        "tools": tools,
        "has_token": prefix.is_some(),
        "token_prefix": prefix,
        // The global switch the per-tool gate sits under: when false no
        // otto.* call asks, whatever the per-tool setting says.
        "require_approval_dangerous": require_dangerous,
        "approval_exempt_tools": exempt,
    })))
}

#[derive(Deserialize)]
pub struct OttoServerConfigReq {
    pub enabled: Option<bool>,
    pub tools: Option<Vec<String>>,
    /// The COMPLETE set of mutating tools that skip the per-call approval
    /// (`mcp_approval_exempt_tools`); replaces the stored list. Bare or
    /// `otto.`-prefixed names; non-mutating/unknown names are a 400.
    pub approval_exempt_tools: Option<Vec<String>>,
    #[serde(default)]
    pub rotate_token: bool,
}

pub async fn otto_server_config(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<OttoServerConfigReq>,
) -> ApiResult<Json<Value>> {
    let settings = SettingsRepo::new(ctx.pool.clone());
    // Validate the exemption list BEFORE any write, so a bad name never
    // leaves a half-applied config behind.
    let requested_exempt = req
        .approval_exempt_tools
        .as_deref()
        .map(normalize_exempt_tools)
        .transpose()
        .map_err(ApiError)?;
    if let Some(en) = req.enabled {
        settings
            .put("mcp_otto_server_enabled", &json!(en))
            .await
            .map_err(ApiError)?;
    }
    if let Some(tools) = &req.tools {
        let known: Vec<String> = otto_tool_specs()
            .iter()
            .filter_map(|t| {
                t["name"]
                    .as_str()
                    .map(|n| n.strip_prefix("otto.").unwrap_or(n).to_string())
            })
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
        settings
            .put("mcp_otto_server_tools", &json!(normalized))
            .await
            .map_err(ApiError)?;
    }
    // Approval exemptions: an explicit list replaces the stored one (audited —
    // it loosens the posture); either way the result is pruned to the enabled
    // set, so turning a tool off also drops its "don't ask".
    let current_exempt = approval_exempt_tools(&ctx).await;
    if requested_exempt.is_some() || req.tools.is_some() {
        let next = prune_exempt_tools(
            requested_exempt.as_ref().unwrap_or(&current_exempt),
            &enabled_tools(&ctx).await,
        );
        if next != current_exempt {
            settings
                .put("mcp_approval_exempt_tools", &json!(next))
                .await
                .map_err(ApiError)?;
            ctx.audit(otto_state::NewAuditEntry {
                user_id: Some(user.id.clone()),
                action: "mcp.otto_server.approval_exempt".into(),
                target: None,
                detail: Some(json!({ "from": current_exempt, "to": next })),
                ip: None,
            })
            .await;
        }
    }
    let mut minted: Option<String> = None;
    if req.rotate_token {
        let repo = AuthRepo::new(ctx.pool.clone());
        repo.revoke_mcp_tokens(&user.id).await.map_err(ApiError)?;
        minted = Some(
            repo.issue_mcp_token(&user.id, Some("otto-mcp-server"))
                .await
                .map_err(ApiError)?,
        );
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
            .filter_map(|t| {
                t["name"]
                    .as_str()
                    .map(|n| n.strip_prefix("otto.").unwrap_or(n).to_string())
            })
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
    CurrentUser(user): CurrentUser,
    Query(q): Query<GatewayToolsQuery>,
) -> ApiResult<Json<Value>> {
    crate::auth::require_ws_role(&ctx, &user, &q.workspace_id, WorkspaceRole::Viewer).await?;
    let servers = ctx
        .mcp
        .registry()
        .list_for_ws(&q.workspace_id)
        .await
        .map_err(ApiError)?;
    let mut tools: Vec<Value> = Vec::new();
    for s in servers.into_iter().filter(|s| s.enabled) {
        let policy = otto_state::ResourceAccessRepo::new(ctx.pool.clone())
            .get_live_policy(otto_core::access::ResourceKind::McpServer, &s.id)
            .await?;
        if !s.managed && policy.mode == otto_core::access::AccessMode::Legacy {
            continue;
        }
        if !ctx
            .mcp
            .resource_allowed(&s, &user, "discover", None)
            .await?
        {
            continue;
        }
        for t in ctx
            .mcp
            .visible_tools(&s, &user)
            .await
            .map_err(ApiError)?
            .into_iter()
            .filter(|t| t.enabled)
        {
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
    let server = ctx.mcp.registry().get(&req.server_id).await?;
    if server.workspace_id != req.workspace_id {
        return Err(Error::Forbidden("MCP workspace mismatch".into()).into());
    }
    let policy = otto_state::ResourceAccessRepo::new(ctx.pool.clone())
        .get_policy(otto_core::access::ResourceKind::McpServer, &server.id)
        .await?;
    let role = if policy.mode == otto_core::access::AccessMode::Legacy {
        WorkspaceRole::Editor
    } else {
        WorkspaceRole::Viewer
    };
    crate::auth::require_ws_role(&ctx, &user, &server.workspace_id, role).await?;
    if policy.mode == otto_core::access::AccessMode::Legacy {
        otto_state::GrantsRepo::new(ctx.pool.clone())
            .check_global(
                &user,
                otto_core::domain::Feature::Mcp,
                otto_core::domain::Capability::Edit,
                "legacy MCP invocation requires edit",
            )
            .await?;
    }
    if !ctx
        .mcp
        .resource_allowed(&server, &user, "invoke", Some(&req.tool))
        .await?
    {
        return Err(Error::Forbidden("MCP tool access denied".into()).into());
    }
    let _ = &req.session_id;
    let ictx = InvokeCtx {
        workspace_id: Some(req.workspace_id.clone()),
        dry_run: req.dry_run,
        caller_user_id: Some(user.id.clone()),
        caller_kind: "gateway".into(),
        direction: "outbound".into(),
    };
    let outcome = ctx
        .mcp
        .invoke(&req.server_id, &req.tool, &req.arguments, &ictx)
        .await
        .map_err(ApiError)?;
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
    fn self_call_errors_keep_the_whole_actionable_message() {
        let status = reqwest::StatusCode::NOT_FOUND;
        // A candidate listing (the resolver's 404) survives past 400 chars.
        let long = format!("not found: {}", "- r1  repo  (workspace: w)\n".repeat(40));
        let body = json!({"code":"not_found","message": long}).to_string();
        let msg = self_call_error(status, &body);
        assert!(msg.starts_with("404 Not Found: not found: - r1"), "{msg}");
        assert!(msg.len() > 1000, "{}", msg.len());
        // `{error}` bodies too; a raw non-JSON body stays a short snippet.
        assert!(self_call_error(status, r#"{"error":"nope"}"#).ends_with("nope"));
        assert!(self_call_error(status, &"y".repeat(5000)).len() < 450);
        // 204 No Content is a success the agent can read, not `null`.
        assert_eq!(parse_self_ok(""), json!({"ok": true}));
        assert_eq!(parse_self_ok("[1]"), json!([1]));
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
                t["category"]
                    .as_str()
                    .map(|c| !c.is_empty())
                    .unwrap_or(false),
                "{short} missing category"
            );
            // inputSchema is an object; every declared `required` key exists in `properties`.
            assert_eq!(
                t["inputSchema"]["type"],
                json!("object"),
                "{short} schema not an object"
            );
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
                assert!(
                    DANGEROUS.contains(&s),
                    "{short} is mutating but not DANGEROUS"
                );
                assert!(
                    !DEFAULT_ENABLED.contains(&s),
                    "{short} is mutating but default-enabled"
                );
            } else {
                let de = DEFAULT_ENABLED.contains(&s);
                let opt = OPT_IN_READS.contains(&s);
                assert!(
                    de ^ opt,
                    "{short} (read) must be default-enabled XOR opt-in (de={de}, opt={opt})"
                );
                assert!(
                    !DANGEROUS.contains(&s),
                    "{short} (read) must not be DANGEROUS"
                );
            }
        }
    }

    #[test]
    fn classification_lists_reference_real_tools() {
        let shorts: std::collections::HashSet<String> =
            spec_short_mut().into_iter().map(|(s, _)| s).collect();
        for n in DEFAULT_ENABLED
            .iter()
            .chain(DANGEROUS.iter())
            .chain(OPT_IN_READS.iter())
        {
            assert!(
                shorts.contains(*n),
                "classification names a non-existent tool '{n}'"
            );
        }
    }

    #[test]
    fn api_client_tools_present_classified_and_routed() {
        const READS: &[&str] = &["api_list", "api_get_request", "api_history"];
        const WRITES: &[&str] = &["api_execute", "api_upsert_request", "api_run_automation"];
        let specs = otto_tool_specs();
        for name in READS.iter().chain(WRITES) {
            let spec = specs
                .iter()
                .find(|spec| spec["name"] == format!("otto.{name}"))
                .unwrap_or_else(|| panic!("missing spec otto.{name}"));
            assert_eq!(spec["category"], json!("API Client"));
        }
        for read in READS {
            assert!(
                DEFAULT_ENABLED.contains(read),
                "{read} must be default-enabled"
            );
            assert!(!DANGEROUS.contains(read), "{read} must not be DANGEROUS");
        }
        for write in WRITES {
            assert!(DANGEROUS.contains(write), "{write} must be DANGEROUS");
            assert!(
                !DEFAULT_ENABLED.contains(write),
                "{write} must be off by default"
            );
        }

        assert_eq!(
            route_for(
                "api_list",
                &json!({"workspace_id":"w1","q":"login","kind":"requests"}),
            )
            .unwrap()
            .path,
            "/api/v1/workspaces/w1/api-client/overview?q=login&kind=requests"
        );
        assert_eq!(
            route_for(
                "api_get_request",
                &json!({"workspace_id":"w1","request_id":"r1"})
            )
            .unwrap()
            .path,
            "/api/v1/workspaces/w1/api-client/requests/r1?shape=agent"
        );
        assert_eq!(
            route_for(
                "api_history",
                &json!({"workspace_id":"w1","limit":5,"source":"agent"}),
            )
            .unwrap()
            .path,
            "/api/v1/workspaces/w1/api-client/history?limit=5&source=agent"
        );
        assert_eq!(
            route_for("api_history", &json!({"workspace_id":"w1","id":"h1"}))
                .unwrap()
                .path,
            "/api/v1/workspaces/w1/api-client/history/h1"
        );

        let execute = route_for(
            "api_execute",
            &json!({"workspace_id":"w1","request_id":"r1","confirm":true}),
        )
        .unwrap();
        assert_eq!(execute.method, Method::Post);
        assert_eq!(
            execute.path,
            "/api/v1/workspaces/w1/api-client/requests/r1/execute"
        );
        assert_eq!(
            execute.body.unwrap(),
            json!({"shape":"agent","confirm":true})
        );

        let update = route_for(
            "api_upsert_request",
            &json!({"workspace_id":"w1","request_id":"r1","name":"Login","method":"post","url":"https://api.example/login"}),
        )
        .unwrap();
        assert_eq!(update.method, Method::Patch);
        assert_eq!(update.path, "/api/v1/workspaces/w1/api-client/requests/r1");
        let create = route_for(
            "api_upsert_request",
            &json!({"workspace_id":"w1","name":"Login","method":"POST","url":"https://api.example/login"}),
        )
        .unwrap();
        assert_eq!(create.method, Method::Post);
        assert_eq!(create.path, "/api/v1/workspaces/w1/api-client/requests");

        let run = route_for(
            "api_run_automation",
            &json!({"workspace_id":"w1","automation_id":"a1"}),
        )
        .unwrap();
        assert_eq!(run.method, Method::Post);
        assert_eq!(
            run.path,
            "/api/v1/workspaces/w1/api-client/automations/a1/run"
        );
        assert_eq!(run.body.unwrap(), json!({}));
    }

    #[test]
    fn api_execute_rejects_brace_overrides_in_route_for() {
        let error = route_for(
            "api_execute",
            &json!({
                "workspace_id":"w1",
                "request_id":"r1",
                "vars":{"base_url":"https://evil.example/?token={{api_token}}"}
            }),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "invalid: vars override 'base_url' must not contain '{{' (no nested substitution)"
        );
    }

    #[test]
    fn dangerous_detail_surfaces_api_targets() {
        assert_eq!(
            dangerous_detail(
                "otto.api_execute",
                &json!({
                    "workspace_id":"w1",
                    "request_id":"r1",
                    "environment_id":"staging",
                    "confirm":true,
                    "confirm_new_host":false
                }),
            ),
            "API request 'r1' executed against environment 'staging' (confirm=true, confirm_new_host=false) in workspace 'w1'"
        );
        assert_eq!(
            dangerous_detail(
                "otto.api_upsert_request",
                &json!({"workspace_id":"w1","name":"Login","method":"post","url":"https://api.example/login"}),
            ),
            "Save API request 'Login' (POST https://api.example/login) in workspace 'w1'"
        );
        assert_eq!(
            dangerous_detail(
                "otto.api_run_automation",
                &json!({"workspace_id":"w1","automation_id":"smoke"}),
            ),
            "Run API automation 'smoke' in workspace 'w1'"
        );
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
    fn room_tools_are_default_enabled_and_route() {
        // Rooms are the auditable agent-to-agent channel: enabled by default,
        // never DANGEROUS (the persistence + user visibility IS the control).
        assert!(DEFAULT_ENABLED.contains(&"room_post"));
        assert!(DEFAULT_ENABLED.contains(&"room_read"));
        assert!(!DANGEROUS.contains(&"room_post"));
        let c = route_for(
            "room_post",
            &json!({"room_id":"r1","text":"hi","session_id":"s1"}),
        )
        .unwrap();
        assert_eq!(c.method, Method::Post);
        assert_eq!(c.path, "/api/v1/agent-rooms/r1/messages");
        assert_eq!(c.body.unwrap(), json!({"text":"hi","session_id":"s1"}));
        let c = route_for(
            "room_read",
            &json!({"room_id":"r1","after":"m9","limit":50}),
        )
        .unwrap();
        assert_eq!(c.method, Method::Get);
        assert_eq!(c.path, "/api/v1/agent-rooms/r1/messages?after=m9&limit=50");
        // No optional args → no dangling query separator.
        let c = route_for("room_read", &json!({"room_id":"r1"})).unwrap();
        assert_eq!(c.path, "/api/v1/agent-rooms/r1/messages?");
    }

    #[test]
    fn route_for_maps_workflows_and_brokers() {
        assert_eq!(
            route_for("list_workflows", &json!({"workspace_id":"ws1"})).unwrap(),
            SelfCall {
                method: Method::Get,
                path: "/api/v1/workspaces/ws1/workflows".into(),
                body: None
            }
        );
        let c = route_for(
            "run_workflow",
            &json!({"workflow_id":"wf1","input":{"k":1},"start_node":"n2","review_mode":"fan_out"}),
        )
        .unwrap();
        assert_eq!(c.method, Method::Post);
        assert_eq!(c.path, "/api/v1/workflows/wf1/run");
        assert_eq!(
            c.body.unwrap(),
            json!({"input":{"k":1},"start_node":"n2","review_mode":"fan_out"})
        );
        assert_eq!(
            route_for("cancel_workflow_run", &json!({"run_id":"r1"})).unwrap(),
            SelfCall {
                method: Method::Post,
                path: "/api/v1/workflow-runs/r1/cancel".into(),
                body: Some(json!({}))
            }
        );
        assert_eq!(
            route_for(
                "get_broker_topic",
                &json!({"cluster_id":"c1","topic":"orders"})
            )
            .unwrap()
            .path,
            "/api/v1/brokers/clusters/c1/topics/orders"
        );
        let c = route_for(
            "produce_broker_message",
            &json!({"cluster_id":"c1","topic":"orders","value":"hi","key":"k","confirm":true}),
        )
        .unwrap();
        assert_eq!(c.path, "/api/v1/brokers/clusters/c1/topics/orders/produce");
        assert_eq!(
            c.body.unwrap(),
            json!({"value":"hi","key":"k","confirm":true})
        );
        let c = route_for(
            "consume_broker_messages",
            &json!({"cluster_id":"c1","topic":"orders","limit":10,"value_filter":"x"}),
        )
        .unwrap();
        assert_eq!(c.path, "/api/v1/brokers/clusters/c1/topics/orders/consume");
        assert_eq!(c.body.unwrap(), json!({"limit":10,"value_filter":"x"}));
    }

    #[test]
    fn run_workflow_forwards_review_mode() {
        // The override rides alone (no input / start_node) and reaches the route
        // verbatim — the route, not the tool, validates the value.
        let c = route_for(
            "run_workflow",
            &json!({"workflow_id":"wf1","review_mode":"orchestrator"}),
        )
        .unwrap();
        assert_eq!(c.body.unwrap(), json!({"review_mode":"orchestrator"}));
        // Absent ⇒ no key at all, so pre-field callers post exactly what they did.
        let c = route_for("run_workflow", &json!({"workflow_id":"wf1"})).unwrap();
        assert_eq!(c.body.unwrap(), json!({}));
        let spec = otto_tool_specs()
            .into_iter()
            .find(|t| t["name"] == "otto.run_workflow")
            .expect("run_workflow spec");
        assert_eq!(
            spec["inputSchema"]["properties"]["review_mode"]["enum"],
            json!(["fan_out", "orchestrator"])
        );
        assert!(spec["description"]
            .as_str()
            .unwrap()
            .contains("review_mode"));
    }

    /// "I enabled create_pr, why does it still ask?" — the per-tool exemption
    /// is what skips the prompt; without it a mutating tool stays gated.
    #[test]
    fn approval_gate_honours_the_per_tool_exemption() {
        assert!(DANGEROUS.contains(&"create_pr"));
        // Enabled + mutating + not exempted → gated (the secure default).
        assert!(approval_gated(true, false, false));
        // "Ask before each call" off → no prompt.
        assert!(!approval_gated(true, true, false));
        // A trusted `kind='mcp'` write grant also clears it.
        assert!(!approval_gated(true, false, true));
        // Reads never ask.
        assert!(!approval_gated(false, false, false));
    }

    #[test]
    fn exempt_list_is_validated_normalized_and_pruned() {
        let ok = normalize_exempt_tools(&[
            "otto.create_pr".to_string(),
            "comment_pr".to_string(),
            "create_pr".to_string(),
        ])
        .unwrap();
        assert_eq!(ok, ["create_pr", "comment_pr"]);
        let e = normalize_exempt_tools(&["list_repos".to_string()])
            .unwrap_err()
            .to_string();
        assert!(e.contains("not a mutating tool"), "{e}");
        let e = normalize_exempt_tools(&["nope".to_string()])
            .unwrap_err()
            .to_string();
        assert!(e.contains("unknown otto tool"), "{e}");
        // Disabling a tool drops its exemption (re-enabling starts gated).
        let enabled = vec!["create_pr".to_string(), "list_repos".to_string()];
        assert_eq!(prune_exempt_tools(&ok, &enabled), ["create_pr"]);
    }

    #[test]
    fn git_tools_take_a_friendly_repo_ref_and_list_repos_spans_workspaces() {
        let specs = otto_tool_specs();
        let spec = |short: &str| {
            specs
                .iter()
                .find(|t| t["name"] == format!("otto.{short}"))
                .unwrap_or_else(|| panic!("otto.{short}"))
                .clone()
        };
        for short in REPO_REF_TOOLS {
            let s = spec(short);
            let schema = &s["inputSchema"];
            assert_eq!(
                schema["properties"]["repo_id"]["description"],
                json!(REPO_REF_DESC),
                "{short}"
            );
            // Optional: omitted inside a session → the session's repo. And no
            // `workspace_id` property, so the in-session bridge never narrows
            // the cross-workspace resolution to the session's own workspace.
            let required = schema["required"].as_array().cloned().unwrap_or_default();
            assert!(!required.contains(&json!("repo_id")), "{short}");
            assert!(
                schema["properties"].get("workspace_id").is_none(),
                "{short}"
            );
        }
        let lr = spec("list_repos");
        assert!(
            lr["inputSchema"]["required"].is_null(),
            "workspace_id is optional"
        );
        assert!(lr["description"]
            .as_str()
            .unwrap()
            .contains("EVERY workspace"));
        assert_eq!(
            route_for("list_repos", &json!({})).unwrap().path,
            "/api/v1/git/repos/directory"
        );
        assert_eq!(
            route_for("list_repos", &json!({"workspace_id":"ws 1"}))
                .unwrap()
                .path,
            "/api/v1/git/repos/directory?workspace_id=ws%201"
        );
    }

    #[test]
    fn route_for_maps_git_issues_swarm_memory_usage() {
        assert_eq!(
            route_for("get_pr", &json!({"repo_id":"r1","number":7}))
                .unwrap()
                .path,
            "/api/v1/repos/r1/prs/7"
        );
        let c = route_for("create_pr", &json!({"repo_id":"r1","title":"T","description":"D","source_branch":"feat","target_branch":"main"})).unwrap();
        assert_eq!(c.path, "/api/v1/repos/r1/prs");
        assert_eq!(
            c.body.unwrap(),
            json!({"title":"T","description":"D","source_branch":"feat","target_branch":"main"})
        );
        assert_eq!(
            route_for("list_prs", &json!({"repo_id":"r1","state":"open"}))
                .unwrap()
                .path,
            "/api/v1/repos/r1/prs?state=open"
        );
        // The route returns a PAGE, not a bare array — a caller that doesn't
        // know that reads `items` off an array and gets nothing.
        let spec = otto_tool_specs()
            .into_iter()
            .find(|t| t["name"] == "otto.list_prs")
            .expect("list_prs spec");
        assert!(
            spec["description"]
                .as_str()
                .unwrap_or_default()
                .contains("has_more"),
            "list_prs description must describe the page shape: {}",
            spec["description"]
        );

        let c = route_for(
            "search_issues",
            &json!({"account_id":"a1","query":"a = b","project":"X"}),
        )
        .unwrap();
        assert!(c.path.starts_with("/api/v1/issue/search?account_id=a1"));
        assert!(c.path.contains("&q=a%20%3D%20b"), "got {}", c.path);
        assert!(c.path.contains("&project=X"));
        let c = route_for(
            "transition_issue",
            &json!({"account_id":"a1","key":"K-1","transition_id":"21"}),
        )
        .unwrap();
        assert_eq!(c.path, "/api/v1/issue/a1/K-1/transitions");
        assert_eq!(c.body.unwrap(), json!({"transition_id":"21"}));

        let c = route_for(
            "post_swarm_board",
            &json!({"swarm_id":"s1","body":"hello","project_id":"p1"}),
        )
        .unwrap();
        assert_eq!(c.path, "/api/v1/swarm/swarms/s1/board");
        assert_eq!(c.body.unwrap(), json!({"body":"hello","project_id":"p1"}));

        let c = route_for(
            "search_memory",
            &json!({"workspace_id":"ws1","query":"schema","k":5}),
        )
        .unwrap();
        assert_eq!(c.path, "/api/v1/workspaces/ws1/memory/search");
        assert_eq!(c.body.unwrap(), json!({"text":"schema","k":5}));
        assert_eq!(
            route_for(
                "list_memory",
                &json!({"workspace_id":"ws1","collection":"vault"})
            )
            .unwrap()
            .path,
            "/api/v1/workspaces/ws1/memories?collection=vault"
        );

        assert_eq!(
            route_for("get_usage_summary", &json!({"days":7}))
                .unwrap()
                .path,
            "/api/v1/usage/summary?days=7"
        );
        assert_eq!(
            route_for("get_usage_summary", &json!({})).unwrap().path,
            "/api/v1/usage/summary"
        );
        assert_eq!(
            route_for("list_bundled_skills", &json!({})).unwrap(),
            SelfCall {
                method: Method::Get,
                path: "/api/v1/library/bundled".into(),
                body: None
            }
        );
        assert_eq!(
            route_for("list_findings", &json!({"review_id":"rv1"}))
                .unwrap()
                .path,
            "/api/v1/reviews/rv1/findings"
        );
        assert_eq!(
            route_for(
                "broadcast_message",
                &json!({"workspace_id":"ws1","text":"hi"})
            )
            .unwrap()
            .body
            .unwrap(),
            json!({"text":"hi"})
        );
        assert_eq!(
            route_for(
                "test_integration",
                &json!({"workspace_id":"ws1","channel":"slack"})
            )
            .unwrap()
            .path,
            "/api/v1/workspaces/ws1/integrations/slack/test"
        );
    }

    // ----- AWS / Kubernetes consoles (docs/design/aws-k8s-consoles.md §6) ----

    const AWS_READS: &[&str] = &[
        "aws_list_accounts",
        "aws_s3_list_buckets",
        "aws_s3_list_objects",
        "aws_s3_preview",
        "aws_sqs_list_queues",
        "aws_sqs_peek",
        "aws_ec2_list_instances",
        "aws_athena_list_tables",
        "aws_athena_get_query",
        "aws_eks_list_clusters",
    ];
    const K8S_READS: &[&str] = &[
        "k8s_list_clusters",
        "k8s_get_resources",
        "k8s_describe",
        "k8s_logs",
        "k8s_top",
        "k8s_health",
    ];
    const CONSOLE_WRITES: &[&str] = &["aws_athena_query", "aws_sqs_send", "k8s_action"];

    #[test]
    fn aws_k8s_tools_present_and_classified() {
        let names = spec_names();
        let specs = otto_tool_specs();
        for n in AWS_READS.iter().chain(K8S_READS).chain(CONSOLE_WRITES) {
            assert!(
                names.contains(&format!("otto.{n}")),
                "missing spec otto.{n}"
            );
            let spec = specs
                .iter()
                .find(|s| s["name"] == format!("otto.{n}"))
                .unwrap();
            let cat = spec["category"].as_str().unwrap();
            assert!(
                (n.starts_with("aws_") && cat == "AWS")
                    || (n.starts_with("k8s_") && cat == "Kubernetes"),
                "{n} in unexpected category {cat}"
            );
        }
        for r in AWS_READS.iter().chain(K8S_READS) {
            assert!(DEFAULT_ENABLED.contains(r), "{r} must be default-enabled");
            assert!(!DANGEROUS.contains(r), "{r} must not be DANGEROUS");
        }
        for w in CONSOLE_WRITES {
            assert!(DANGEROUS.contains(w), "{w} must be DANGEROUS");
            assert!(!DEFAULT_ENABLED.contains(w), "{w} must be off by default");
            assert!(tool_is_mutating(w));
            let spec = specs
                .iter()
                .find(|s| s["name"] == format!("otto.{w}"))
                .unwrap();
            assert_eq!(spec["mutating"], json!(true));
        }
    }

    #[test]
    fn route_for_maps_aws_console() {
        assert_eq!(
            route_for("aws_list_accounts", &json!({})).unwrap(),
            SelfCall {
                method: Method::Get,
                path: "/api/v1/aws/accounts".into(),
                body: None
            }
        );
        assert_eq!(
            route_for("aws_s3_list_buckets", &json!({"account_id":"a1"}))
                .unwrap()
                .path,
            "/api/v1/aws/accounts/a1/s3/buckets?"
        );
        assert_eq!(
            route_for(
                "aws_s3_list_buckets",
                &json!({"account_id":"a1","region":"eu-west-1"})
            )
            .unwrap()
            .path,
            "/api/v1/aws/accounts/a1/s3/buckets?region=eu-west-1"
        );
        assert_eq!(
            route_for(
                "aws_s3_list_objects",
                &json!({"account_id":"a1","bucket":"b","prefix":"logs/","token":"t","max":50})
            )
            .unwrap()
            .path,
            "/api/v1/aws/accounts/a1/s3/buckets/b/objects?prefix=logs%2F&token=t&max=50"
        );
        assert_eq!(
            route_for(
                "aws_s3_preview",
                &json!({"account_id":"a1","bucket":"b","key":"a b.json","max_bytes":1024})
            )
            .unwrap()
            .path,
            "/api/v1/aws/accounts/a1/s3/buckets/b/preview?key=a%20b.json&max_bytes=1024"
        );
        assert_eq!(
            route_for(
                "aws_s3_preview",
                &json!({"account_id":"a1","bucket":"b","key":"k"})
            )
            .unwrap()
            .path,
            "/api/v1/aws/accounts/a1/s3/buckets/b/preview?key=k"
        );
        assert_eq!(
            route_for(
                "aws_sqs_list_queues",
                &json!({"account_id":"a1","prefix":"orders"})
            )
            .unwrap()
            .path,
            "/api/v1/aws/accounts/a1/sqs/queues?prefix=orders"
        );
        // Peek: read-only POST, visibility timeout pinned to 0, max clamped 1..10.
        let c = route_for(
            "aws_sqs_peek",
            &json!({"account_id":"a1","url":"https://sqs/q","max":99}),
        )
        .unwrap();
        assert_eq!(c.method, Method::Post);
        assert_eq!(c.path, "/api/v1/aws/accounts/a1/sqs/queues/peek?");
        assert_eq!(
            c.body.unwrap(),
            json!({"url":"https://sqs/q","visibility_timeout":0,"max":10})
        );
        let c = route_for("aws_sqs_send", &json!({"account_id":"a1","url":"https://sqs/q.fifo","body":"{}","group_id":"g1","delay_seconds":5})).unwrap();
        assert_eq!(c.method, Method::Post);
        assert_eq!(c.path, "/api/v1/aws/accounts/a1/sqs/queues/send?");
        assert_eq!(
            c.body.unwrap(),
            json!({"url":"https://sqs/q.fifo","body":"{}","group_id":"g1","delay_seconds":5})
        );
        assert_eq!(
            route_for(
                "aws_ec2_list_instances",
                &json!({"account_id":"a1","region":"us-east-1","state":"running","q":"web"})
            )
            .unwrap()
            .path,
            "/api/v1/aws/accounts/a1/ec2/instances?region=us-east-1&state=running&q=web"
        );
        assert_eq!(
            route_for(
                "aws_athena_list_tables",
                &json!({"account_id":"a1","database":"db","catalog":"AwsDataCatalog"})
            )
            .unwrap()
            .path,
            "/api/v1/aws/accounts/a1/athena/tables?database=db&catalog=AwsDataCatalog"
        );
        let c = route_for(
            "aws_athena_query",
            &json!({"account_id":"a1","sql":"SELECT 1","database":"db","workgroup":"primary"}),
        )
        .unwrap();
        assert_eq!(c.method, Method::Post);
        assert_eq!(c.path, "/api/v1/aws/accounts/a1/athena/query?");
        assert_eq!(
            c.body.unwrap(),
            json!({"sql":"SELECT 1","database":"db","workgroup":"primary"})
        );
        assert_eq!(
            route_for(
                "aws_athena_get_query",
                &json!({"account_id":"a1","query_execution_id":"q-1","token":"t2","max":100})
            )
            .unwrap()
            .path,
            "/api/v1/aws/accounts/a1/athena/query/q-1?token=t2&max=100"
        );
        assert_eq!(
            route_for(
                "aws_eks_list_clusters",
                &json!({"account_id":"a1","region":"eu-west-1"})
            )
            .unwrap()
            .path,
            "/api/v1/aws/accounts/a1/eks/clusters?region=eu-west-1"
        );
        // Required ids are enforced.
        assert!(route_for("aws_s3_list_objects", &json!({"account_id":"a1"})).is_err());
        assert!(route_for("aws_athena_query", &json!({"account_id":"a1"})).is_err());
        assert!(route_for("aws_sqs_send", &json!({"account_id":"a1","url":"u"})).is_err());
    }

    #[test]
    fn route_for_maps_k8s_console() {
        assert_eq!(
            route_for("k8s_list_clusters", &json!({})).unwrap(),
            SelfCall {
                method: Method::Get,
                path: "/api/v1/k8s/clusters".into(),
                body: None
            }
        );
        assert_eq!(
            route_for(
                "k8s_get_resources",
                &json!({"cluster_id":"c1","kind":"pods","namespace":"prod","label":"app=web"})
            )
            .unwrap()
            .path,
            "/api/v1/k8s/clusters/c1/resources?kind=pods&ns=prod&label=app%3Dweb"
        );
        // No namespace ⇒ no `ns=` (route default = all namespaces).
        assert_eq!(
            route_for(
                "k8s_get_resources",
                &json!({"cluster_id":"c1","kind":"deployments"})
            )
            .unwrap()
            .path,
            "/api/v1/k8s/clusters/c1/resources?kind=deployments"
        );
        assert_eq!(
            route_for(
                "k8s_describe",
                &json!({"cluster_id":"c1","kind":"deployments","namespace":"prod","name":"web"})
            )
            .unwrap()
            .path,
            "/api/v1/k8s/clusters/c1/resource?kind=deployments&name=web&ns=prod"
        );
        // Cluster-scoped kinds (nodes, namespaces) have no namespace.
        assert_eq!(
            route_for(
                "k8s_describe",
                &json!({"cluster_id":"c1","kind":"nodes","name":"ip-10-0-0-1"})
            )
            .unwrap()
            .path,
            "/api/v1/k8s/clusters/c1/resource?kind=nodes&name=ip-10-0-0-1"
        );
        let c = route_for("k8s_logs", &json!({"cluster_id":"c1","namespace":"prod","pod":"web-1","container":"app","tail":200,"since":"10m","previous":true,"follow":true})).unwrap();
        assert_eq!(c.method, Method::Get);
        assert_eq!(c.path, "/api/v1/k8s/clusters/c1/pods/prod/web-1/logs?container=app&tail=200&since=10m&previous=true");
        assert!(!c.path.contains("follow"), "follow must never be forwarded");
        assert!(TEXT_TOOLS.contains(&"k8s_logs"));
        assert_eq!(
            route_for("k8s_top", &json!({"cluster_id":"c1","namespace":"prod"}))
                .unwrap()
                .path,
            "/api/v1/k8s/clusters/c1/metrics?ns=prod"
        );
        assert_eq!(
            route_for("k8s_health", &json!({"cluster_id":"c1","window":"6h"}))
                .unwrap()
                .path,
            "/api/v1/k8s/clusters/c1/monitor/health?window=6h"
        );
        assert_eq!(
            route_for("k8s_health", &json!({"cluster_id":"c1"}))
                .unwrap()
                .path,
            "/api/v1/k8s/clusters/c1/monitor/health?"
        );
        let c = route_for("k8s_action", &json!({"cluster_id":"c1","action":"scale","kind":"deployments","namespace":"prod","name":"web","params":{"replicas":3}})).unwrap();
        assert_eq!(c.method, Method::Post);
        assert_eq!(c.path, "/api/v1/k8s/clusters/c1/actions");
        assert_eq!(
            c.body.unwrap(),
            json!({"action":"scale","kind":"deployments","ns":"prod","name":"web","params":{"replicas":3}})
        );
        // params defaults to {} so the route's confirm_name check sees an object.
        let c = route_for("k8s_action", &json!({"cluster_id":"c1","action":"restart","kind":"deployments","namespace":"prod","name":"web"})).unwrap();
        assert_eq!(c.body.unwrap()["params"], json!({}));
        assert!(route_for("k8s_action", &json!({"cluster_id":"c1","action":"restart"})).is_err());
        assert!(route_for("k8s_describe", &json!({"cluster_id":"c1","kind":"pods"})).is_err());
    }

    #[test]
    fn dangerous_detail_surfaces_console_targets() {
        let d = dangerous_detail(
            "otto.k8s_action",
            &json!({"cluster_id":"c1","action":"delete_pod","kind":"pods","namespace":"prod","name":"web-1"}),
        );
        assert!(
            d.contains("delete_pod")
                && d.contains("pods/web-1")
                && d.contains("prod")
                && d.contains("c1"),
            "{d}"
        );
        let d = dangerous_detail(
            "otto.aws_sqs_send",
            &json!({"account_id":"a1","url":"https://sqs/q"}),
        );
        assert!(d.contains("https://sqs/q") && d.contains("a1"), "{d}");
        let d = dangerous_detail(
            "otto.aws_athena_query",
            &json!({"account_id":"a1","sql":"SELECT * FROM t","database":"db"}),
        );
        assert!(
            d.contains("SELECT * FROM t") && d.contains("db") && d.contains("a1"),
            "{d}"
        );
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
        assert!(route_for(
            "query_db_readonly",
            &json!({"connection_id":"c1","statement":"SELECT 1"})
        )
        .is_ok());
        assert!(route_for(
            "query_db_readonly",
            &json!({"connection_id":"c1","statement":"DELETE FROM t"})
        )
        .is_err());
        assert!(route_for(
            "query_db_readonly",
            &json!({"connection_id":"c1","statement":"SELECT 1; DROP TABLE t"})
        )
        .is_err());
    }

    #[test]
    fn assistant_memory_tools_present_classified_and_routed() {
        let names = spec_names();
        for n in [
            "otto.assistant_remember",
            "otto.assistant_forget",
            "otto.assistant_recall",
        ] {
            assert!(names.contains(&n.to_string()), "missing assistant spec {n}");
        }
        // Writes are approval-gated; the recall read is opt-in (personal content).
        assert!(DANGEROUS.contains(&"assistant_remember"));
        assert!(DANGEROUS.contains(&"assistant_forget"));
        assert!(OPT_IN_READS.contains(&"assistant_recall"));
        assert!(!DEFAULT_ENABLED.contains(&"assistant_recall"));
        let c = route_for(
            "assistant_remember",
            &json!({"text":"prefers aisle seats","session_id":"s1","bogus":1}),
        )
        .unwrap();
        assert_eq!(c.method, Method::Post);
        assert_eq!(c.path, "/api/v1/assistant/agent/remember");
        let body = c.body.unwrap();
        assert_eq!(body["text"], "prefers aisle seats");
        assert_eq!(body["session_id"], "s1");
        assert!(body.get("bogus").is_none(), "only declared args are forwarded");
        assert_eq!(
            route_for("assistant_forget", &json!({"query":"seats"})).unwrap().path,
            "/api/v1/assistant/agent/forget"
        );
        assert_eq!(
            route_for("assistant_recall", &json!({})).unwrap().path,
            "/api/v1/assistant/agent/recall"
        );
        assert!(route_for("assistant_remember", &json!({})).is_err());
        assert!(route_for("assistant_forget", &json!({})).is_err());
        assert!(dangerous_detail("otto.assistant_remember", &json!({"text":"likes tea"}))
            .contains("likes tea"));
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
            assert!(
                names.contains(&n.to_string()),
                "missing self-improvement spec {n}"
            );
        }
        assert!(DEFAULT_ENABLED.contains(&"list_improvement_edits"));
        assert!(DANGEROUS.contains(&"approve_improvement_edit"));
        assert!(DANGEROUS.contains(&"reject_improvement_edit"));
        assert!(DANGEROUS.contains(&"rollback_improvement_edit"));
        assert_eq!(
            route_for("list_improvement_edits", &json!({"workspace_id":"ws1"}))
                .unwrap()
                .path,
            "/api/v1/workspaces/ws1/improvement/edits"
        );
        assert_eq!(
            route_for("approve_improvement_edit", &json!({"edit_id":"e1"})).unwrap(),
            SelfCall {
                method: Method::Post,
                path: "/api/v1/improvement/edits/e1/approve".into(),
                body: Some(json!({}))
            }
        );
        assert_eq!(
            route_for("reject_improvement_edit", &json!({"edit_id":"e1"}))
                .unwrap()
                .path,
            "/api/v1/improvement/edits/e1/reject"
        );
        assert_eq!(
            route_for("rollback_improvement_edit", &json!({"edit_id":"e1"}))
                .unwrap()
                .path,
            "/api/v1/improvement/edits/e1/rollback"
        );
        assert!(
            dangerous_detail("otto.approve_improvement_edit", &json!({"edit_id":"e9"}))
                .contains("e9")
        );
    }

    #[test]
    fn dangerous_detail_surfaces_new_tool_targets() {
        assert!(
            dangerous_detail("otto.run_workflow", &json!({"workflow_id":"wf-9"})).contains("wf-9")
        );
        assert!(dangerous_detail(
            "otto.produce_broker_message",
            &json!({"topic":"orders","cluster_id":"c1"})
        )
        .contains("orders"));
        let d = dangerous_detail(
            "otto.create_pr",
            &json!({"repo_id":"r1","title":"Fix","source_branch":"f","target_branch":"main"}),
        );
        assert!(d.contains("Fix") && d.contains("main"));
        assert!(
            dangerous_detail("otto.broadcast_message", &json!({"text":"hello team"}))
                .contains("hello team")
        );
    }

    // ----- Friendly references + the workspace pin (every non-git tool) ----

    #[test]
    fn every_ref_arg_names_a_real_tool_argument_and_kind() {
        let specs = otto_tool_specs();
        for (tool, arg, kind) in REF_ARGS {
            let spec = specs
                .iter()
                .find(|s| s["name"] == format!("otto.{tool}"))
                .unwrap_or_else(|| panic!("REF_ARGS names unknown tool {tool}"));
            assert!(
                spec["inputSchema"]["properties"].get(*arg).is_some(),
                "{tool}: REF_ARGS arg '{arg}' is not in its schema"
            );
            assert!(
                crate::agent_refs::kind_of(kind).is_some(),
                "{tool}: unknown kind {kind}"
            );
        }
        for (tool, kind) in DIRECTORY_TOOLS {
            let spec = specs
                .iter()
                .find(|s| s["name"] == format!("otto.{tool}"))
                .unwrap_or_else(|| panic!("DIRECTORY_TOOLS names unknown tool {tool}"));
            assert!(crate::agent_refs::kind_of(kind).is_some(), "{tool}");
            // Cross-workspace by default: never REQUIRES a workspace.
            let reqd = spec["inputSchema"]["required"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            assert!(
                !reqd.contains(&json!("workspace_id")),
                "{tool} must not require workspace_id"
            );
            assert_eq!(spec["mutating"], json!(false), "{tool}");
        }
        // An issue account may be omitted (sole account), so no Issues tool
        // requires it any more.
        for s in &specs {
            if s["category"] == json!("Issues") {
                let reqd = s["inputSchema"]["required"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                assert!(!reqd.contains(&json!("account_id")), "{}", s["name"]);
            }
        }
    }

    /// The workspace pin must be verifiable for EVERY tool: it either carries
    /// a workspace, resolves one (a named object / repo / artifact / probe),
    /// addresses no workspace-owned object, or is consciously denied to
    /// pinned tokens. A new tool that fits none of these fails here.
    #[test]
    fn every_tool_has_a_workspace_pin_story() {
        for spec in otto_tool_specs() {
            let name = spec["name"].as_str().unwrap();
            let short = name.strip_prefix("otto.").unwrap();
            let has_ws = spec["inputSchema"]["properties"]
                .get("workspace_id")
                .is_some();
            let ws_kind_ref = REF_ARGS.iter().any(|(t, _, k)| {
                *t == short
                    && crate::agent_refs::kind_of(k)
                        .is_some_and(|k| k.scope == crate::agent_refs::Scope::Workspace)
            });
            let covered = has_ws
                || ws_kind_ref
                || REPO_REF_TOOLS.contains(&short)
                || short == "list_repos"
                || DESIGN_WS_TOOLS.contains(&short)
                || PIN_PROBES.iter().any(|(t, ..)| *t == short)
                || pin_global(short)
                || PIN_UNVERIFIABLE.contains(&short);
            assert!(covered, "{short}: no workspace-pin classification");
        }
    }

    #[test]
    fn pin_verdict_denies_what_it_cannot_verify() {
        let pinned = McpScope {
            tools: None,
            allow_writes: true,
            workspace_id: Some("ws-a".into()),
        };
        // Resolved into the pin → allowed; into another workspace → denied.
        assert!(pin_verdict(
            &pinned,
            "run_workflow",
            &json!({"workflow_id":"W","workspace_id":"ws-a"})
        )
        .is_none());
        let d = pin_verdict(
            &pinned,
            "run_workflow",
            &json!({"workflow_id":"W","workspace_id":"ws-b"}),
        )
        .unwrap();
        assert!(d.contains("scoped to workspace 'ws-a'"), "{d}");
        // No workspace established (e.g. a finding) → fail closed.
        let d = pin_verdict(&pinned, "get_finding", &json!({"finding_id":"F"})).unwrap();
        assert!(d.contains("cannot"), "{d}");
        // Global rows are fine without a workspace.
        assert!(pin_verdict(&pinned, "k8s_top", &json!({"cluster_id":"C"})).is_none());
        assert!(pin_verdict(&pinned, "list_workflows", &json!({})).is_none());
        // An unpinned scope never needs a workspace.
        let open = McpScope::unrestricted();
        assert!(pin_verdict(&open, "get_finding", &json!({"finding_id":"F"})).is_none());
    }

    #[test]
    fn pr_number_aliases_and_strings_normalize_before_hashing() {
        assert_eq!(
            normalize_args("get_pr", &json!({"repo_id":"r","pr_number":"52"})).unwrap(),
            json!({"repo_id":"r","number":52})
        );
        assert_eq!(
            normalize_args("start_pr_review", &json!({"number":7})).unwrap(),
            json!({"pr_number":7})
        );
        // Already canonical → untouched (no churn in the audit/hash).
        assert!(normalize_args("comment_pr", &json!({"number":3,"body":"x"})).is_none());
        assert!(normalize_args("list_workflows", &json!({"number":3})).is_none());
    }

    #[test]
    fn confluence_page_urls_and_transition_names_resolve() {
        assert_eq!(confluence_page_id("12345").as_deref(), Some("12345"));
        assert_eq!(
            confluence_page_id("https://x.atlassian.net/wiki/spaces/ST/pages/98765/My+Page")
                .as_deref(),
            Some("98765")
        );
        assert_eq!(
            confluence_page_id("https://x/wiki/pages/viewpage.action?pageId=4242").as_deref(),
            Some("4242")
        );
        assert!(confluence_page_id("My Page").is_none());
        let list = json!([
            {"id":"11","name":"Start progress","to_status":"In Progress"},
            {"id":"21","name":"Done","to_status":"Done"}
        ]);
        assert_eq!(match_transition(&list, "in progress").unwrap(), "11");
        assert_eq!(match_transition(&list, "Start Progress").unwrap(), "11");
        let e = match_transition(&list, "Reopen").unwrap_err().to_string();
        assert!(e.contains("21") && e.contains("Done"), "{e}");
    }

    #[test]
    fn plain_ids_from_an_unpinned_caller_need_no_lookup() {
        let id = "01KZTKNK3Z8N6VD9Q0MTDQSJ3V";
        assert!(!refs_need_lookup(
            "get_workflow",
            &json!({"workflow_id": id}),
            false
        ));
        assert!(refs_need_lookup(
            "get_workflow",
            &json!({"workflow_id": "Nightly"}),
            false
        ));
        // A pinned token must learn a workspace-owned object's workspace.
        assert!(refs_need_lookup(
            "get_workflow",
            &json!({"workflow_id": id}),
            true
        ));
        // …but not a global row's.
        assert!(!refs_need_lookup(
            "k8s_top",
            &json!({"cluster_id": id}),
            true
        ));
        // Omitted issue account → the sole account is looked up.
        assert!(refs_need_lookup(
            "search_issues",
            &json!({"query":"x"}),
            false
        ));
        // A workspace NAME, a transition NAME, a pinned probe.
        assert!(refs_need_lookup(
            "list_sessions",
            &json!({"workspace_id":"Casino"}),
            false
        ));
        assert!(refs_need_lookup(
            "transition_issue",
            &json!({"account_id": id, "key":"K-1", "transition_id":"Done"}),
            false
        ));
        assert!(refs_need_lookup(
            "get_session",
            &json!({"session_id": id}),
            true
        ));
        assert!(!refs_need_lookup(
            "get_session",
            &json!({"session_id": id}),
            false
        ));
    }

    #[test]
    fn an_update_keeps_every_field_it_did_not_send() {
        let stored = json!({"id":"q","name":"Login","method":"POST","url":"u",
            "headers":[{"key":"X-Tenant","value":"7","enabled":true}],"query":[],
            "body_mode":"json","body":"{\"a\":1}","collection_id":"c1","ssh_connection_id":null});
        let merged = merge_stored_request(
            &json!({"workspace_id":"w","request_id":"q","name":"Login","method":"POST","url":"u2"}),
            &stored,
        );
        assert_eq!(merged["url"], "u2", "sent fields win");
        assert_eq!(merged["headers"][0]["key"], "X-Tenant");
        assert_eq!(
            (merged["body_mode"].as_str(), merged["body"].as_str()),
            (Some("json"), Some("{\"a\":1}"))
        );
        assert_eq!(merged["collection_id"], "c1");
        assert!(
            merged.get("ssh_connection_id").is_none(),
            "a null stays absent"
        );
        // An explicit value (even empty) is the agent's choice.
        let merged = merge_stored_request(&json!({"headers":[]}), &stored);
        assert_eq!(merged["headers"], json!([]));
    }

    #[test]
    fn long_running_tools_get_a_longer_self_call_budget() {
        assert_eq!(call_timeout("list_workflows", &json!({})).as_secs(), 30);
        assert_eq!(
            call_timeout("api_execute", &json!({"timeout_ms": 60000})).as_secs(),
            75
        );
        assert_eq!(
            call_timeout("api_execute", &json!({"timeout_ms": 999999})).as_secs(),
            75
        );
        assert!(call_timeout("api_run_automation", &json!({})).as_secs() >= 120);
        assert!(call_timeout("k8s_logs", &json!({})).as_secs() > 60);
    }

    #[test]
    fn new_discovery_and_git_tools_route_and_classify() {
        for r in [
            "list_workspaces",
            "list_goal_loops",
            "list_agent_rooms",
            "list_issue_accounts",
            "list_issue_transitions",
            "get_scheduled_task",
            "list_swarm_projects",
            "list_swarm_tasks",
            "list_design_projects",
            "list_pr_reviews",
            "get_pr_checks",
        ] {
            assert!(
                DEFAULT_ENABLED.contains(&r),
                "{r} should be a default-on read"
            );
        }
        assert!(OPT_IN_READS.contains(&"get_pr_diff"));
        assert!(DANGEROUS.contains(&"merge_pr"));
        let c = route_for(
            "merge_pr",
            &json!({"repo_id":"r","number":4,"strategy":"squash"}),
        )
        .unwrap();
        assert_eq!(
            (c.method, c.path.as_str()),
            (Method::Post, "/api/v1/repos/r/prs/4/merge")
        );
        assert_eq!(c.body.unwrap(), json!({"strategy":"squash"}));
        assert!(
            dangerous_detail("otto.merge_pr", &json!({"repo_id":"r","number":4})).contains("#4")
        );
        assert_eq!(
            route_for(
                "list_prs",
                &json!({"repo_id":"r","state":"all","page":2,"per_page":100})
            )
            .unwrap()
            .path,
            "/api/v1/repos/r/prs?state=all&page=2&per_page=100"
        );
        assert_eq!(
            route_for("list_prs", &json!({"repo_id":"r"})).unwrap().path,
            "/api/v1/repos/r/prs"
        );
        assert_eq!(
            route_for("list_pr_reviews", &json!({"repo_id":"r","pr_number":9}))
                .unwrap()
                .path,
            "/api/v1/repos/r/prs/9/reviews"
        );
        assert_eq!(
            route_for("get_pr_checks", &json!({"repo_id":"r","number":9}))
                .unwrap()
                .path,
            "/api/v1/repos/r/prs/9/checks"
        );
        assert_eq!(
            route_for(
                "list_issue_transitions",
                &json!({"account_id":"a","key":"K-1"})
            )
            .unwrap()
            .path,
            "/api/v1/issue/a/K-1/transitions"
        );
        assert_eq!(
            route_for(
                "search_issues",
                &json!({"account_id":"a","query":"x","start_at":25})
            )
            .unwrap()
            .path,
            "/api/v1/issue/search?account_id=a&q=x&start_at=25"
        );
        assert_eq!(
            route_for("get_scheduled_task", &json!({"task_id":"t"}))
                .unwrap()
                .path,
            "/api/v1/scheduled-tasks/t"
        );
        assert_eq!(
            route_for("list_workflow_runs", &json!({"workflow_id":"w"}))
                .unwrap()
                .path,
            "/api/v1/workflows/w/runs?summary=true"
        );
        assert_eq!(
            route_for(
                "list_improvement_edits",
                &json!({"workspace_id":"ws","status":"applied"})
            )
            .unwrap()
            .path,
            "/api/v1/workspaces/ws/improvement/edits?status=applied"
        );
        assert_eq!(
            route_for("list_swarm_projects", &json!({"swarm_id":"s"}))
                .unwrap()
                .path,
            "/api/v1/swarm/swarms/s/projects"
        );
        assert_eq!(
            route_for("get_swarm_board", &json!({"swarm_id":"s","task_id":"t"}))
                .unwrap()
                .path,
            "/api/v1/swarm/swarms/s/board?task_id=t"
        );
        let c = route_for(
            "create_pr",
            &json!({"repo_id":"r","title":"T","description":"D",
            "source_branch":"f","target_branch":"main","draft":true,"reviewers":["ann"]}),
        )
        .unwrap();
        assert_eq!(c.body.unwrap()["reviewers"], json!(["ann"]));
        let c = route_for(
            "start_pr_review",
            &json!({"repo_id":"r","pr_number":3,"context":"focus auth"}),
        )
        .unwrap();
        assert_eq!(c.body.unwrap(), json!({"context":"focus auth"}));
        // update_scheduled_task never PATCHes the filled-in workspace_id.
        let c = route_for(
            "update_scheduled_task",
            &json!({"task_id":"t","workspace_id":"ws","name":"n"}),
        )
        .unwrap();
        assert_eq!(c.body.unwrap(), json!({"name":"n"}));
        // Stringified numbers from clients that stringify every argument.
        assert_eq!(
            route_for("get_pr", &json!({"repo_id":"r","number":"12"}))
                .unwrap()
                .path,
            "/api/v1/repos/r/prs/12"
        );
        let c = route_for(
            "aws_sqs_peek",
            &json!({"account_id":"a","url":"u","max":"3"}),
        )
        .unwrap();
        assert_eq!(c.body.unwrap()["max"], json!(3));
    }

    // ----- Vault: optional workspace_id ------------------------------------

    #[test]
    fn confluence_page_tools_route_and_are_tiered() {
        // Reads are default-enabled like the other Confluence read; every write
        // is outward-facing (it publishes to a real wiki) and must be DANGEROUS.
        let names = spec_names();
        for n in [
            "otto.get_confluence_page",
            "otto.list_confluence_page_comments",
            "otto.create_confluence_page",
            "otto.update_confluence_page",
            "otto.comment_confluence_page",
        ] {
            assert!(names.contains(&n.to_string()), "missing spec {n}");
        }
        for r in ["get_confluence_page", "list_confluence_page_comments"] {
            assert!(
                mcp_tool_enabled_for_token(true, false, &[], r),
                "{r} should be a default-enabled read"
            );
        }

        let acc = "acc1";
        let pid = "12345";
        let read = route_for(
            "get_confluence_page",
            &json!({"account_id": acc, "page_id": pid}),
        )
        .unwrap();
        assert_eq!(read.method, Method::Get);
        assert_eq!(
            read.path,
            "/api/v1/issue/confluence/pages/12345?account_id=acc1"
        );
        assert!(read.body.is_none());

        // Create sends Markdown, and omits parent_id entirely when not supplied
        // (an empty string would make Confluence reject the call).
        let create = route_for(
            "create_confluence_page",
            &json!({"account_id": acc, "space_key": "STOR", "title": "T", "body_md": "# hi"}),
        )
        .unwrap();
        assert_eq!(create.method, Method::Post);
        assert_eq!(
            create.path,
            "/api/v1/issue/confluence/pages?account_id=acc1"
        );
        let body = create.body.unwrap();
        assert_eq!(body["space_key"], json!("STOR"));
        assert_eq!(body["body_md"], json!("# hi"));
        assert!(body.get("parent_id").is_none());

        let nested = route_for(
            "create_confluence_page",
            &json!({"account_id": acc, "space_key": "STOR", "title": "T",
                    "body_md": "x", "parent_id": "999"}),
        )
        .unwrap();
        assert_eq!(nested.body.unwrap()["parent_id"], json!("999"));

        // Update never carries a version — the server resolves it.
        let upd = route_for(
            "update_confluence_page",
            &json!({"account_id": acc, "page_id": pid, "body_md": "b"}),
        )
        .unwrap();
        assert_eq!(upd.method, Method::Put);
        assert_eq!(
            upd.path,
            "/api/v1/issue/confluence/pages/12345?account_id=acc1"
        );
        let ub = upd.body.unwrap();
        assert!(
            ub.get("version").is_none(),
            "callers must not send a version"
        );
        assert!(ub.get("title").is_none(), "absent title must stay absent");

        let cmt = route_for(
            "comment_confluence_page",
            &json!({"account_id": acc, "page_id": pid, "body_md": "answer"}),
        )
        .unwrap();
        assert_eq!(cmt.method, Method::Post);
        assert_eq!(
            cmt.path,
            "/api/v1/issue/confluence/pages/12345/comments?account_id=acc1"
        );
        assert_eq!(cmt.body.unwrap()["body_md"], json!("answer"));
    }

    #[test]
    fn design_tools_are_default_on_reads_and_route_to_the_design_api() {
        const READS: &[&str] = &["design_list", "design_get", "design_links", "design_search"];
        let specs = otto_tool_specs();
        for r in READS {
            let spec = specs
                .iter()
                .find(|s| s["name"] == format!("otto.{r}"))
                .unwrap_or_else(|| panic!("missing spec otto.{r}"));
            assert_eq!(spec["category"], json!("Design"));
            assert_eq!(spec["mutating"], json!(false));
            assert!(DEFAULT_ENABLED.contains(r), "{r} must be default-enabled");
            assert!(!DANGEROUS.contains(r), "{r} must not be DANGEROUS");
            assert!(!tool_is_mutating(r));
            // The library is global: no design tool requires a workspace.
            let reqd = spec["inputSchema"]["required"].as_array().unwrap();
            assert!(!reqd.iter().any(|x| x == "workspace_id"), "{r}");
        }
        assert_eq!(
            route_for("design_list", &json!({})).unwrap().path,
            "/api/v1/design/artifacts"
        );
        assert_eq!(
            route_for("design_list", &json!({"studio": "3d", "limit": 5}))
                .unwrap()
                .path,
            "/api/v1/design/artifacts?studio=3d&limit=5"
        );
        assert_eq!(
            route_for(
                "design_list",
                &json!({"limit": 2, "cursor": "2026-09-23T10:00:00Z|A9"})
            )
            .unwrap()
            .path,
            "/api/v1/design/artifacts?limit=2&cursor=2026-09-23T10%3A00%3A00Z%7CA9"
        );
        assert_eq!(
            route_for("design_get", &json!({"artifact_id": "A1", "version": "v3"}))
                .unwrap()
                .path,
            "/api/v1/design/artifacts/A1?content=true&version=v3"
        );
        assert_eq!(
            route_for("design_links", &json!({"artifact_id": "A1"}))
                .unwrap()
                .path,
            "/api/v1/design/artifacts/A1/links?dir=both"
        );
        let c = route_for(
            "design_search",
            &json!({"query": "hero card", "studio": "site"}),
        )
        .unwrap();
        assert_eq!(c.method, Method::Get);
        assert_eq!(c.path, "/api/v1/design/search?q=hero%20card&studio=site");
        assert!(route_for("design_get", &json!({})).is_err());
        assert!(route_for("design_search", &json!({})).is_err());
    }

    #[test]
    fn design_writes_are_approval_gated_and_route_to_the_design_api() {
        let specs = otto_tool_specs();
        for w in ["design_assist", "design_link"] {
            let spec = specs
                .iter()
                .find(|s| s["name"] == format!("otto.{w}"))
                .unwrap_or_else(|| panic!("missing spec otto.{w}"));
            assert_eq!(spec["category"], json!("Design"));
            assert_eq!(spec["mutating"], json!(true));
            assert!(DANGEROUS.contains(&w), "{w} must be approval-gated");
            assert!(!DEFAULT_ENABLED.contains(&w), "{w} must be off by default");
            assert!(tool_is_mutating(w));
            // The artifact carries the workspace — no design tool requires one.
            let reqd = spec["inputSchema"]["required"].as_array().unwrap();
            assert!(!reqd.iter().any(|x| x == "workspace_id"), "{w}");
            assert!(
                dangerous_detail(&format!("otto.{w}"), &json!({"artifact_id": "A1"}))
                    .contains("A1"),
                "{w}"
            );
            // Gated by default; the per-tool exemption is the only opt-out.
            let exempt = normalize_exempt_tools(&[format!("otto.{w}")]).unwrap();
            assert_eq!(exempt, vec![w.to_string()]);
            assert!(approval_gated(DANGEROUS.contains(&w), false, false));
            assert!(!approval_gated(DANGEROUS.contains(&w), true, false));
        }
        // Approving a version stays human-only: no design tool approves.
        assert!(!specs.iter().any(|s| s["name"]
            .as_str()
            .is_some_and(|n| n.starts_with("otto.design_") && n.contains("approve"))));

        let c = route_for(
            "design_assist",
            &json!({"artifact_id": "A 1", "prompt": "bolder hero", "mode": "refine",
                    "references": ["B2@v3"], "workspace_id": "ignored"}),
        )
        .unwrap();
        assert_eq!(c.method, Method::Post);
        assert_eq!(c.path, "/api/v1/design/artifacts/A%201/assist");
        let b = c.body.unwrap();
        assert_eq!(b["prompt"], "bolder hero");
        assert_eq!(b["mode"], "refine");
        assert_eq!(b["references"][0], "B2@v3");
        assert!(b.get("workspace_id").is_none());
        let l = route_for(
            "design_link",
            &json!({"artifact_id": "A1", "rel": "implements", "dst_kind": "story",
                    "dst_id": "S1", "policy": ""}),
        )
        .unwrap();
        assert_eq!(l.method, Method::Post);
        assert_eq!(l.path, "/api/v1/design/artifacts/A1/links");
        let b = l.body.unwrap();
        assert_eq!(b["rel"], "implements");
        assert_eq!(b["dst_kind"], "story");
        assert_eq!(b["dst_id"], "S1");
        assert!(b.get("policy").is_none(), "empty optional args are dropped");
        assert!(route_for("design_assist", &json!({"artifact_id": "A1"})).is_err());
        assert!(route_for(
            "design_link",
            &json!({"artifact_id": "A1", "rel": "embeds"})
        )
        .is_err());
    }

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
                assert!(
                    reqd.iter().any(|r| r == "vault_id"),
                    "{name} must require vault_id"
                );
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
        assert_eq!(
            pick_vault_workspace(&rows, false).as_deref(),
            Some("view-only")
        );
        assert_eq!(
            pick_vault_workspace(&rows, true).as_deref(),
            Some("editable")
        );
        // Viewer-only memberships still resolve for a mutating tool — the
        // self-call's native RBAC owns the denial.
        let viewer_only = vec![(ws("view-only"), WorkspaceRole::Viewer)];
        assert_eq!(
            pick_vault_workspace(&viewer_only, true).as_deref(),
            Some("view-only")
        );
        assert_eq!(pick_vault_workspace(&[], false), None);
    }
}
