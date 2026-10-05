//! The outward tool **catalog** and its policy lists: every `otto.*` tool's
//! JSON spec ([`otto_tool_specs`]), which tools are on by default
//! ([`DEFAULT_ENABLED`]), approval-gated ([`DANGEROUS`]) or irreversible
//! ([`IRREVERSIBLE`]), plus the pure enable / approval decisions the governed
//! pipeline in `otto-server` (`mcp_outward::governed_invoke`) keys on.

use otto_core::Error;
use serde_json::{json, Value};

use super::exec::i64_lenient;
use super::ui_commands;

pub const DEFAULT_ENABLED: &[&str] = &[
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
    // GET behind its per-service feature grant
    // (`aws_s3` / `aws_sqs` / `aws_ec2` / `aws_athena` / `aws_eks`: View).
    "aws_list_accounts",
    "aws_s3_list_buckets",
    "aws_s3_list_objects",
    "aws_s3_preview",
    "aws_sqs_list_queues",
    "aws_ec2_list_instances",
    "aws_athena_list_tables",
    "aws_athena_get_query",
    "aws_eks_list_clusters",
    "aws_logs_list_groups",
    "aws_logs_filter",
    "aws_logs_insights",
    "aws_logs_get_insights",
    // Kubernetes console reads (`kubernetes`: View).
    "k8s_list_clusters",
    "k8s_get_resources",
    "k8s_describe",
    "k8s_logs",
    "k8s_top",
    "k8s_health",
    "k8s_pod_actions_list",
    // (Vault v2 structural reads removed — Vault feature disabled.)
];
pub const DANGEROUS: &[&str] = &[
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
    // message, an SQS peek (a receive bumps each message's receive count and
    // can dead-letter it), and a kubectl rollout/scale/delete/Argo verb against
    // a live cluster. Each is also Edit-gated per feature by the self-call's RBAC.
    "aws_athena_query",
    "aws_sqs_send",
    "aws_sqs_peek",
    "k8s_action",
    // An HTTP request to a pod's port (actuator loggers / refresh / env…) —
    // approval-gated even for GET: an actuator GET can dump env/secrets.
    "k8s_pod_http",
    // Otto Assistant memory writes: an outside agent writing / erasing the
    // user's personal memory is approval-gated (in-session assistant calls go
    // through the native stdio tools, which chip + Undo every write).
    "assistant_remember",
    "assistant_forget",
];

/// The **irreversible** tier of [`DANGEROUS`] — the guardrail for auto-approve
/// rules (`crate::mcp_auto_approve`). These tools act on something nobody can
/// take back from inside Otto: a merge into the target branch, a kubectl verb
/// against a live cluster (delete pod / scale / rollback / Argo prune), a
/// message produced to a live topic or queue that consumers act on, an
/// arbitrary HTTP request (an API-client call can be a production write or a
/// DELETE), a hard delete, an erased memory. A category rule never covers one;
/// a per-tool rule covers one only with the second explicit toggle
/// `allow_irreversible`. Reversible writes (opening/commenting a PR, Jira and
/// Confluence edits, messages to agents, vault edits — `vault_delete` is a soft
/// trash move) stay auto-approvable by category.
pub const IRREVERSIBLE: &[&str] = &[
    "merge_pr",
    "k8s_action",
    "k8s_pod_http",
    "produce_broker_message",
    "aws_sqs_send",
    "api_execute",
    "api_run_automation",
    "delete_scheduled_task",
    "assistant_forget",
];

/// True iff the bare tool is mutating + approval-gated ([`DANGEROUS`]).
pub fn tool_is_dangerous(bare: &str) -> bool {
    DANGEROUS.contains(&bare)
}

/// True iff the bare tool is in the [`IRREVERSIBLE`] guardrail tier.
pub fn tool_is_irreversible(bare: &str) -> bool {
    IRREVERSIBLE.contains(&bare)
}

/// The catalog category of a bare tool name (`create_pr` → `Git`), from
/// [`otto_tool_specs`] — built once, the catalog is static.
pub fn tool_category(bare: &str) -> Option<&'static str> {
    use std::collections::HashMap;
    use std::sync::OnceLock;
    static INDEX: OnceLock<HashMap<String, String>> = OnceLock::new();
    INDEX
        .get_or_init(|| {
            otto_tool_specs_cached()
                .iter()
                .filter_map(|s| {
                    let name = s["name"].as_str()?;
                    let cat = s["category"].as_str()?;
                    Some((
                        name.strip_prefix("otto.").unwrap_or(name).to_string(),
                        cat.to_string(),
                    ))
                })
                .collect()
        })
        .get(bare)
        .map(String::as_str)
}

/// `(category, [bare mutating tool])` for every catalog category that has at
/// least one approval-gated tool, in catalog order — what a category
/// auto-approve rule can name.
pub fn mutating_categories() -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for spec in otto_tool_specs_cached() {
        let Some(name) = spec["name"].as_str() else {
            continue;
        };
        let bare = name.strip_prefix("otto.").unwrap_or(name);
        if !tool_is_dangerous(bare) {
            continue;
        }
        let cat = spec["category"].as_str().unwrap_or("Other").to_string();
        match out.iter_mut().find(|(c, _)| *c == cat) {
            Some((_, tools)) => tools.push(bare.to_string()),
            None => out.push((cat, vec![bare.to_string()])),
        }
    }
    out
}

/// Non-mutating tools that are defined and enableable but stay **off by default**
/// — either because they stream potentially large/sensitive payload *content*
/// (message bodies, recalled knowledge, code, rows) or pre-date the default-on
/// read policy. Every read tool is therefore in exactly one of `DEFAULT_ENABLED`
/// or `OPT_IN_READS`; the classification invariant test asserts that.
#[cfg(test)]
pub const OPT_IN_READS: &[&str] = &[
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

/// The git tools whose `repo_id` is a friendly reference resolved server-side
/// (id, name, local path, or remote — across every workspace the caller can
/// read; omitted → the calling session's repo). See `mcp_outward::fill_repo_ref` (otto-server).
pub const REPO_REF_TOOLS: &[&str] = &[
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
///
/// Kept SHORT on purpose (perf2/10-mcp R2): these strings repeat in every
/// schema property that takes a reference, and Codex re-reads the whole
/// catalog every turn. Resolution details live once in the list tools.
pub const REPO_REF_DESC: &str =
    "Repo id, name, path or remote (any readable workspace). Omit = this session's repo.";

/// Shared schema text for the friendly-reference id arguments resolved by
/// `mcp_outward::fill_refs` (otto-server) (see `agent_refs`): the id OR a human field, across every
/// workspace the caller can read, with candidates listed on a miss.
pub const WS_DIR_DESC: &str = "Optional workspace id or name; omit = all you can read.";
pub const WORKFLOW_REF_DESC: &str = "Workflow id or name (otto.list_workflows).";
pub const BROKER_REF_DESC: &str = "Broker cluster id or name (otto.list_broker_clusters).";
pub const CONNECTION_REF_DESC: &str = "Connection id or name (otto.list_connections).";
pub const API_REQUEST_REF_DESC: &str =
    "Saved request id or name (otto.api_list), within `workspace_id`.";
pub const ISSUE_ACCOUNT_REF_DESC: &str =
    "Jira/Confluence account id, label, email or URL; omit if you have one.";
pub const PAGE_REF_DESC: &str = "Confluence page id or URL.";
pub const SWARM_REF_DESC: &str = "Swarm id or name (otto.list_swarms).";
pub const VAULT_REF_DESC: &str = "Vault id (integer) or vault name (otto.vault_list).";
pub const DESIGN_REF_DESC: &str = "Design artifact id or exact title.";
pub const ROOM_REF_DESC: &str = "Agent room id or name (otto.list_agent_rooms).";
pub const TASK_REF_DESC: &str = "Scheduled task id or name (otto.list_scheduled_tasks).";
pub const AWS_ACCOUNT_REF_DESC: &str = "AWS account id or name (otto.aws_list_accounts).";
pub const K8S_CLUSTER_REF_DESC: &str = "Kubernetes cluster id or name (otto.k8s_list_clusters).";
pub const PR_NUMBER_DESC: &str = "Pull request number (otto.list_prs).";

/// [`otto_tool_specs`] built ONCE per process. The catalog is static (~216
/// `json!` literals, ~100 KB serialized); every status read, `tools/list` and
/// validation used to rebuild it. Hot paths read this; `otto_tool_specs()`
/// stays for callers that want an owned copy.
pub fn otto_tool_specs_cached() -> &'static [Value] {
    static SPECS: std::sync::LazyLock<Vec<Value>> = std::sync::LazyLock::new(otto_tool_specs);
    &SPECS
}

/// Static catalog of the outward `otto.*` tools. Each entry carries a `category`
/// so the control-plane UI can group the (now large) checklist. Adding a tool here
/// surfaces it in the control plane automatically (`GET /mcp/otto-server`).
pub fn otto_tool_specs() -> Vec<Value> {
    let mut specs = vec![
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
            "description":"A pull request's diff (files + hunks). The whole-PR response is capped (5,000 lines / 200 KB per file, 20,000 lines / 4 MB in total): files past a cap come back with `too_large` or `hunks_omitted` and no hunks (`truncated` marks the response). For a big PR call it with `summary: true` for the file list + counts, then once per file with `path` (+ `old_path` for a rename; `full: true` lifts the per-file cap to 50,000 lines). Off by default (streams code); enable to let agents read PR diffs. Read-only.",
            "inputSchema":{"type":"object","properties":{
                "repo_id":{"type":"string","description":REPO_REF_DESC},"number":{"type":"integer","description":PR_NUMBER_DESC},
                "pr_number":{"type":"integer","description":"Alias of `number`."},
                "summary":{"type":"boolean","description":"File list + per-file counts only, no hunks."},
                "path":{"type":"string","description":"Only this file (its current path; a deleted file's old path)."},
                "old_path":{"type":"string","description":"A renamed file's origin, alongside `path`."},
                "full":{"type":"boolean","description":"With `path`: lift the per-file cap (5,000 lines) to 50,000 lines."}}}}),
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
            "inputSchema":{"type":"object","required":[],"properties":{"workspace_id":{"type":"string","description":"Optional; vaults are global."}}}}),
        json!({"name":"otto.vault_dir","mutating":false,"category":"Vault",
            "description":"One level of a vault's folder tree (folders, notes, attachments). Read-only.",
            "inputSchema":{"type":"object","required":["vault_id"],"properties":{
                "workspace_id":{"type":"string","description":"Optional; vaults are global."},"vault_id":{"type":["integer","string"],"description":VAULT_REF_DESC},"path":{"type":"string"}}}}),
        json!({"name":"otto.vault_read","mutating":false,"category":"Vault",
            "description":"A note's raw markdown + metadata + outgoing links. Read-only.",
            "inputSchema":{"type":"object","required":["vault_id","path"],"properties":{
                "workspace_id":{"type":"string","description":"Optional; vaults are global."},"vault_id":{"type":["integer","string"],"description":VAULT_REF_DESC},"path":{"type":"string"}}}}),
        json!({"name":"otto.vault_search","mutating":false,"category":"Vault",
            "description":"Full-text (FTS5) search over a vault's notes with snippets; tag:/path:/type: operators. Read-only.",
            "inputSchema":{"type":"object","required":["vault_id","query"],"properties":{
                "workspace_id":{"type":"string","description":"Optional; vaults are global."},"vault_id":{"type":["integer","string"],"description":VAULT_REF_DESC},"query":{"type":"string"},"limit":{"type":"integer"}}}}),
        json!({"name":"otto.vault_backlinks","mutating":false,"category":"Vault",
            "description":"Notes linking TO a given note, with context snippets. Read-only.",
            "inputSchema":{"type":"object","required":["vault_id","path"],"properties":{
                "workspace_id":{"type":"string","description":"Optional; vaults are global."},"vault_id":{"type":["integer","string"],"description":VAULT_REF_DESC},"path":{"type":"string"}}}}),
        json!({"name":"otto.vault_tags","mutating":false,"category":"Vault",
            "description":"Every tag in a vault with note counts. Read-only.",
            "inputSchema":{"type":"object","required":["vault_id"],"properties":{
                "workspace_id":{"type":"string","description":"Optional; vaults are global."},"vault_id":{"type":["integer","string"],"description":VAULT_REF_DESC}}}}),
        json!({"name":"otto.vault_graph","mutating":false,"category":"Vault",
            "description":"The vault link graph (compact arrays; local neighborhood when `path` given). Read-only.",
            "inputSchema":{"type":"object","required":["vault_id"],"properties":{
                "workspace_id":{"type":"string","description":"Optional; vaults are global."},"vault_id":{"type":["integer","string"],"description":VAULT_REF_DESC},"mode":{"type":"string"},"path":{"type":"string"},"depth":{"type":"integer"}}}}),
        json!({"name":"otto.vault_okf_validate","mutating":false,"category":"Vault",
            "description":"Deterministic OKF v0.1 conformance report (E1-E3 errors, W1-W5 warnings). Read-only.",
            "inputSchema":{"type":"object","required":["vault_id"],"properties":{
                "workspace_id":{"type":"string","description":"Optional; vaults are global."},"vault_id":{"type":["integer","string"],"description":VAULT_REF_DESC}}}}),
        json!({"name":"otto.vault_write","mutating":true,"category":"Vault",
            "description":"Create/update a markdown note in a doc vault (OKF preferred). DANGEROUS: writes files — approval-gated.",
            "inputSchema":{"type":"object","required":["vault_id","path","content"],"properties":{
                "workspace_id":{"type":"string","description":"Optional; vaults are global."},"vault_id":{"type":["integer","string"],"description":VAULT_REF_DESC},"path":{"type":"string"},
                "content":{"type":"string"},"if_hash":{"type":"string"}}}}),
        json!({"name":"otto.vault_write_file","mutating":true,"category":"Vault",
            "description":"Create/update a guarded UTF-8 documentation artifact in a vault — OpenAPI YAML, JSON, D2, Mermaid, text or CSV (`path` ending .yaml/.yml/.json/.d2/.mmd/.txt/.csv; markdown notes use otto.vault_write). Pass `if_hash` for optimistic concurrency. DANGEROUS: writes files — approval-gated.",
            "inputSchema":{"type":"object","required":["vault_id","path","content"],"properties":{
                "workspace_id":{"type":"string","description":"Optional; vaults are global."},"vault_id":{"type":["integer","string"],"description":VAULT_REF_DESC},"path":{"type":"string"},
                "content":{"type":"string"},"if_hash":{"type":"string"}}}}),
        json!({"name":"otto.vault_rename","mutating":true,"category":"Vault",
            "description":"Rename/move a note or folder; rewrites every referencing link across the vault. DANGEROUS — approval-gated.",
            "inputSchema":{"type":"object","required":["vault_id","from","to"],"properties":{
                "workspace_id":{"type":"string","description":"Optional; vaults are global."},"vault_id":{"type":["integer","string"],"description":VAULT_REF_DESC},"from":{"type":"string"},"to":{"type":"string"}}}}),
        json!({"name":"otto.vault_delete","mutating":true,"category":"Vault",
            "description":"Soft-delete a note into the vault's .trash/ (never destroys files). DANGEROUS — approval-gated.",
            "inputSchema":{"type":"object","required":["vault_id","path"],"properties":{
                "workspace_id":{"type":"string","description":"Optional; vaults are global."},"vault_id":{"type":["integer","string"],"description":VAULT_REF_DESC},"path":{"type":"string"}}}}),
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
            "description":"Read messages from an agent room you are a member of (oldest first within the page). With no cursor it returns the room's NEWEST messages (default 50); pass `after` (the last message id you saw) for only newer ones, or `before` (the oldest id you hold) to page back.",
            "inputSchema":{"type":"object","required":["room_id"],"properties":{
                "room_id":{"type":"string","description":ROOM_REF_DESC},"after":{"type":"string"},"before":{"type":"string"},"limit":{"type":"integer"},
                "session_id":{"type":"string","description":"the calling session (injected automatically by Otto's MCP bridge)"}}}}),
        // ---- Otto Assistant (the user's personal memory) ----
        json!({"name":"otto.assistant_remember","mutating":true,"category":"Assistant",
            "description":"Save ONE short, atomic fact about the user (a preference, a person, a recurring plan) to the Otto Assistant's private memory. Never store secrets. Shown to the user with Undo; queued for review when memory approval is on. DANGEROUS: writes the user's memory — approval-gated.",
            "inputSchema":{"type":"object","required":["text"],"properties":{
                "text":{"type":"string"},"kind":{"type":"string","description":"fact | decision | learning … (default fact)"},
                "tags":{"type":"array","items":{"type":"string"}},
                "session_id":{"type":"string","description":"the calling assistant session (injected by Otto)"}}}}),
        json!({"name":"otto.assistant_forget","mutating":true,"category":"Assistant",
            "description":"Forget the user's assistant memories that match `query` (up to 10; each can be restored with its undo token). DANGEROUS: erases personal memory — approval-gated.",
            "inputSchema":{"type":"object","required":["query"],"properties":{
                "query":{"type":"string"},
                "session_id":{"type":"string","description":"the calling assistant session (injected by Otto)"}}}}),
        json!({"name":"otto.assistant_recall","mutating":false,"category":"Assistant",
            "description":"Recall the user's profile and the assistant memories matching `query` (keyword recall; omit `query` for the most recent). Read-only; off by default (personal content).",
            "inputSchema":{"type":"object","properties":{
                "query":{"type":"string"},"k":{"type":"integer","description":"max memories (1..20, default 10)"},
                "session_id":{"type":"string","description":"the calling assistant session (injected by Otto)"}}}}),
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
        json!({"name":"otto.aws_sqs_peek","mutating":true,"category":"AWS",
            "description":"Peek up to `max` (1..10) messages on an SQS queue (receive-message with visibility timeout 0, so they stay visible). NOT read-only: every peek increments each message's receive count, so on a queue with a redrive policy repeated peeks can move messages to the dead-letter queue — approval-gated.",
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
        // CloudWatch Logs (read-only; times are epoch milliseconds).
        json!({"name":"otto.aws_logs_list_groups","mutating":false,"category":"AWS",
            "description":"List CloudWatch Logs log groups (name, retention_days, stored_bytes); optional name `prefix` (e.g. `/aws/eks/`), page with `token`. Read-only.",
            "inputSchema":{"type":"object","required":["account_id"],"properties":{
                "account_id":{"type":"string","description":AWS_ACCOUNT_REF_DESC},"prefix":{"type":"string"},
                "token":{"type":"string"},"max":{"type":"integer"},"region":{"type":"string"}}}}),
        json!({"name":"otto.aws_logs_filter","mutating":false,"category":"AWS",
            "description":"Read events from one log `group`: optional filter `pattern`, comma-separated `streams`, `start`/`end` (epoch ms — always pass a window), `max` (≤ 1000), `token` to page. Read-only.",
            "inputSchema":{"type":"object","required":["account_id","group"],"properties":{
                "account_id":{"type":"string","description":AWS_ACCOUNT_REF_DESC},"group":{"type":"string"},"pattern":{"type":"string"},
                "streams":{"type":"string"},"start":{"type":"integer"},"end":{"type":"integer"},
                "token":{"type":"string"},"max":{"type":"integer"},"region":{"type":"string"}}}}),
        json!({"name":"otto.aws_logs_insights","mutating":false,"category":"AWS",
            "description":"START a Logs Insights `query` over `groups` (≤ 50) between `start` and `end` (epoch ms, ≤ 31 days); returns `{query_id}` — poll otto.aws_logs_get_insights. Read-only, but billed per GB scanned.",
            "inputSchema":{"type":"object","required":["account_id","groups","query","start","end"],"properties":{
                "account_id":{"type":"string","description":AWS_ACCOUNT_REF_DESC},"groups":{"type":"array","items":{"type":"string"}},
                "query":{"type":"string"},"start":{"type":"integer"},"end":{"type":"integer"},"limit":{"type":"integer"},"region":{"type":"string"}}}}),
        json!({"name":"otto.aws_logs_get_insights","mutating":false,"category":"AWS",
            "description":"Status + rows of a Logs Insights query: `status`, `done`, `result` {columns, rows}, bytes_scanned. Poll until `done`. Read-only.",
            "inputSchema":{"type":"object","required":["account_id","query_id"],"properties":{
                "account_id":{"type":"string","description":AWS_ACCOUNT_REF_DESC},"query_id":{"type":"string"},"region":{"type":"string"}}}}),
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
        json!({"name":"otto.k8s_pod_actions_list","mutating":false,"category":"Kubernetes",
            "description":"List the saved pod HTTP actions of a cluster (per workload: name, method, port, path, headers, body_template with {{logger}}/{{level}} variables). Optional `namespace`, `workload_kind`, `workload` filters. Read-only.",
            "inputSchema":{"type":"object","required":["cluster_id"],"properties":{
                "cluster_id":{"type":"string","description":K8S_CLUSTER_REF_DESC},"namespace":{"type":"string"},
                "workload_kind":{"type":"string"},"workload":{"type":"string"}}}}),
        json!({"name":"otto.k8s_pod_http","mutating":true,"category":"Kubernetes",
            "description":"Send ONE HTTP request to a pod's container port through the API-server pod proxy (port-forward fallback) — e.g. Spring Boot actuator: GET /actuator/loggers, POST /actuator/loggers/<logger> body {\"configuredLevel\":\"DEBUG\"}, GET /actuator/health|info|env|threaddump, POST /actuator/refresh. Target one `pod` OR every running pod of a `workload` {kind,name} (per-pod results: status, duration_ms, headers, body ≤256 KiB). A non-GET method on a prod cluster is refused unless `confirm_name` equals the pod/workload name — set it only after the user explicitly confirmed. DANGEROUS: can change a live app's runtime state — approval-gated.",
            "inputSchema":{"type":"object","required":["cluster_id","namespace","port","method","path"],"properties":{
                "cluster_id":{"type":"string","description":K8S_CLUSTER_REF_DESC},"namespace":{"type":"string"},
                "pod":{"type":"string"},"workload":{"type":"object","properties":{"kind":{"type":"string"},"name":{"type":"string"}}},
                "port":{"type":"integer"},"method":{"type":"string","description":"GET|POST|PUT|PATCH|DELETE"},
                "path":{"type":"string"},"headers":{"type":"object"},"body":{"type":"string"},
                "timeout_ms":{"type":"integer"},"max_concurrency":{"type":"integer"},"confirm_name":{"type":"string"}}}}),
    ];
    // Agent UI control: one governed tool per `docs/contracts/ui-commands.json`
    // entry (`otto.ui_*`, category "UI control") — see `crate::ui_commands`.
    specs.extend(ui_commands::specs());
    specs
}

/// The effective enabled set. Never saved → [`DEFAULT_ENABLED`] + every UI
/// tool. Saved → the saved list, PLUS each UI tool that did not exist when it
/// was saved (`ui_known`): agent UI control is default-on — it does nothing
/// without the per-session human grant — so a list saved before a UI command
/// shipped must not silently switch the new command off. A UI tool the
/// operator saw and unchecked stays off. Pure.
pub fn merge_enabled(
    stored: Option<Vec<String>>,
    ui_known: &[String],
    ui_tools: &[String],
) -> Vec<String> {
    match stored {
        None => DEFAULT_ENABLED
            .iter()
            .map(|s| s.to_string())
            .chain(ui_tools.iter().cloned())
            .collect(),
        Some(mut list) => {
            for t in ui_tools {
                if !ui_known.contains(t) && !list.contains(t) {
                    list.push(t.clone());
                }
            }
            list
        }
    }
}

/// Internal session-scoped MCP credentials are not an outward integration and
/// therefore do not depend on the admin's outward-server toggle/tool list. Their
/// immutable per-token scope has already run before this check. External MCP
/// tokens retain the existing master-toggle + enabled-tool behavior.
///
/// Agent UI control (`ui_session`: a `ui_*` tool called with a credential bound
/// to an Otto session) is not an outward integration either: the master
/// switch does not apply, only the per-tool enable (and, in the bridge, the
/// per-session human grant).
pub fn mcp_tool_enabled_for_token(
    internal: bool,
    ui_session: bool,
    outward_on: bool,
    globally_enabled: &[String],
    short: &str,
) -> bool {
    internal || ((outward_on || ui_session) && globally_enabled.iter().any(|tool| tool == short))
}

/// Whether a governed `otto.*` call is subject to the human-approval gate
/// (before the global `mcp_require_approval_dangerous` switch): a DANGEROUS
/// tool, unless the operator exempted it (`mcp_approval_exempt_tools` — the
/// MCP → Otto server "Ask before each call" toggle) or the caller's
/// `kind='mcp'` token carries a trusted write grant. Pure, so the decision
/// table is unit-tested.
pub fn approval_gated(dangerous: bool, exempt: bool, token_write_grant: bool) -> bool {
    dangerous && !exempt && !token_write_grant
}

/// Validate + normalize a requested `mcp_approval_exempt_tools` list: bare
/// names (an `otto.` prefix is accepted), each a known MUTATING tool — the gate
/// only ever applies to those, so exempting a read would be a silent no-op —
/// de-duplicated in request order.
pub fn normalize_exempt_tools(requested: &[String]) -> Result<Vec<String>, Error> {
    let mut out: Vec<String> = Vec::with_capacity(requested.len());
    for t in requested {
        let bare = t
            .trim()
            .strip_prefix("otto.")
            .unwrap_or(t.trim())
            .to_string();
        if !DANGEROUS.contains(&bare.as_str()) {
            let known = otto_tool_specs_cached()
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
pub fn prune_exempt_tools(exempt: &[String], enabled: &[String]) -> Vec<String> {
    exempt
        .iter()
        .filter(|t| enabled.contains(t))
        .cloned()
        .collect()
}

/// Build the human-facing approval detail. For scheduled-task create/update it
/// surfaces the prompt + cadence + destination so the approver knows exactly what
/// recurring autonomous capability they are granting (security review fix).
pub fn dangerous_detail(tool: &str, args: &Value) -> String {
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
        "aws_sqs_peek" => format!(
            "Peek messages on SQS queue '{}' on AWS account '{}' (increments their receive count; can dead-letter)",
            args.get("url").and_then(Value::as_str).unwrap_or("?"),
            args.get("account_id").and_then(Value::as_str).unwrap_or("?")
        ),
        "k8s_pod_http" => format!(
            "HTTP {} {} on port {} of {} in namespace '{}' of cluster '{}'",
            args.get("method").and_then(Value::as_str).unwrap_or("?"),
            args.get("path").and_then(Value::as_str).unwrap_or("?"),
            args.get("port")
                .map(|p| p.to_string())
                .unwrap_or_else(|| "?".into()),
            match (args.get("pod").and_then(Value::as_str), args.get("workload")) {
                (Some(p), _) => format!("pod {p}"),
                (None, Some(w)) => format!(
                    "every pod of {}/{}",
                    w.get("kind").and_then(Value::as_str).unwrap_or("?"),
                    w.get("name").and_then(Value::as_str).unwrap_or("?")
                ),
                _ => "?".into(),
            },
            args.get("namespace").and_then(Value::as_str).unwrap_or("?"),
            args.get("cluster_id").and_then(Value::as_str).unwrap_or("?")
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

/// True iff the bare tool name (`"run_workflow"`, no `otto.` prefix) is a
/// **mutating** tool. The mutating set is exactly [`DANGEROUS`] (every catalog
/// entry with `mutating:true` is approval-gated), so this is the single source of
/// truth the per-token read-only axis keys on.
///
/// Agent UI control tools are the one exception: they are never DANGEROUS (the
/// session grant + the UI's own confirms replace a per-call approval), but a
/// `local_write` / `outward` one still counts as mutating so a read-only token
/// scope refuses it.
pub fn tool_is_mutating(bare: &str) -> bool {
    DANGEROUS.contains(&bare)
        || ui_commands::by_tool(bare).is_some_and(|c| c.risk.mutating())
}
