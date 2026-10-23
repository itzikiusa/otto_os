//! The native `tools/list` catalog of this stdio surface (architecture-1-05):
//! the tools only this surface serves ([`base_tool_catalog`] +
//! [`console_tool_catalog`]), the `otto_x` ⇄ `otto.x` twins GENERATED from the
//! control-plane catalog ([`NATIVE_TWINS`], see `listing.rs`), the assistant
//! tools, the per-source scoping, and the governed (`otto.*`) listing.

use super::listing::NativeTwin;
use super::*;

/// The full native catalog: the static first-party tools, the twins generated
/// from the control-plane catalog, and the Otto Assistant tools (which
/// [`tool_catalog_for_source`] shows only to assistant sessions).
pub(super) fn tool_catalog() -> Value {
    TOOL_CATALOG.clone()
}

/// Built once per process: the catalogs are static JSON, and `tools/list`
/// used to rebuild them once per governed spec (O(N²), ~24 ms warm and
/// ~0.7 s on the first call).
pub(super) static TOOL_CATALOG: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| {
    let mut catalog = base_tool_catalog();
    if let Some(tools) = catalog["tools"].as_array_mut() {
        tools.extend(console_tool_catalog());
        tools.extend(listing::generate_twin_entries(
            &GOVERNED_SPECS,
            NATIVE_TWINS,
            governed_stdio_name,
        ));
        tools.extend(assistant_tool_specs());
    }
    catalog
});

/// The control-plane catalog (`otto_mcp::outward::otto_tool_specs()`), built
/// once per process.
pub(super) static GOVERNED_SPECS: std::sync::LazyLock<Vec<Value>> =
    std::sync::LazyLock::new(otto_mcp::outward::otto_tool_specs);

/// Native tool names, built once per process.
pub(super) static NATIVE_TOOL_NAMES: std::sync::LazyLock<Vec<String>> =
    std::sync::LazyLock::new(|| {
        TOOL_CATALOG["tools"]
            .as_array()
            .map(|tools| {
                tools
                    .iter()
                    .filter_map(|t| t["name"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    });

const fn twin(short: &'static str) -> NativeTwin {
    NativeTwin {
        short,
        scoped: &[],
        note: None,
    }
}

const fn scoped_twin(
    short: &'static str,
    scoped: &'static [(&'static str, Option<&'static str>)],
) -> NativeTwin {
    NativeTwin {
        short,
        scoped,
        note: None,
    }
}

const WS_DIR_ARG: &[(&str, Option<&str>)] = &[("workspace_id", Some(WS_DIR_DESC))];
const SESSION_ARG: &[(&str, Option<&str>)] = &[("session_id", None)];
const WS_OPT_ARG: &[(&str, Option<&str>)] =
    &[("workspace_id", Some("Optional workspace id or name."))];

/// The control-plane tools this surface serves NATIVELY under their stdio
/// name (`otto.x` → `otto_x`). Their `tools/list` entry — name, description,
/// `inputSchema` — is generated from the catalog entry (`listing.rs`); only
/// the session-scoped arguments (`scoped`, defaulted from the calling
/// session) and a `note` where this surface's answer differs are this
/// surface's own. How each is dispatched: [`ROUTED_TWINS`] through
/// `route_for`, the rest by their own `run_tool` arm (see `dispatch.rs`).
pub(super) const NATIVE_TWINS: &[NativeTwin] = &[
    NativeTwin {
        short: "list_connections",
        scoped: WS_DIR_ARG,
        note: Some(
            "This surface lists only the queryable DB kinds (mysql, postgres, redis, mongodb, clickhouse).",
        ),
    },
    twin("list_workspaces"),
    scoped_twin("list_workflows", WS_DIR_ARG),
    twin("list_workflow_runs"),
    twin("get_workflow_run"),
    scoped_twin("list_goal_loops", WS_DIR_ARG),
    scoped_twin("list_broker_clusters", WS_DIR_ARG),
    twin("list_broker_topics"),
    twin("list_issue_accounts"),
    twin("search_issues"),
    twin("list_issue_transitions"),
    scoped_twin("list_swarms", WS_DIR_ARG),
    scoped_twin(
        "list_repos",
        &[("workspace_id", Some("Optional: only this workspace's repos."))],
    ),
    scoped_twin(
        "list_sessions",
        &[(
            "workspace_id",
            Some("Optional workspace id or name (default: this session's)."),
        )],
    ),
    scoped_twin("get_session", SESSION_ARG),
    scoped_twin("wait_session", SESSION_ARG),
    twin("open_session"),
    scoped_twin("send_message", SESSION_ARG),
    twin("list_product_stories"),
    twin("list_findings"),
    twin("list_prs"),
    twin("get_pr"),
    twin("comment_pr"),
    scoped_twin("list_improvement_runs", WS_OPT_ARG),
    scoped_twin("list_improvement_edits", WS_OPT_ARG),
    twin("vault_list"),
    twin("vault_dir"),
    twin("vault_read"),
    twin("vault_search"),
    twin("vault_backlinks"),
    twin("vault_tags"),
    twin("vault_graph"),
    twin("vault_okf_validate"),
    twin("vault_write"),
    twin("vault_write_file"),
    twin("vault_rename"),
    twin("vault_delete"),
    scoped_twin("list_agent_rooms", WS_DIR_ARG),
    twin("room_post"),
    twin("room_read"),
    twin("api_list"),
    twin("api_get_request"),
    twin("api_history"),
    twin("api_execute"),
    twin("api_upsert_request"),
    twin("api_run_automation"),
];

/// The tools ONLY this surface serves (no control-plane twin): DB explorer,
/// canvas, workbench, Design Hall, swarm, memory, usage, browser.
pub(super) fn base_tool_catalog() -> Value {
    json!({
        "tools": [
            {
                "name": "otto_db_schema",
                "description": "Read-only: the TOP of a connection's schema tree — databases (SQL/Mongo) or keyspaces (Redis). Returns structure only, no row data. Each node carries an `id` (a NodePath) you pass to otto_db_children / otto_db_object.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "connection_id": { "type": "string", "description": "Otto connection id or name (a DB-kind connection)." }
                    },
                    "required": ["connection_id"]
                }
            },
            {
                "name": "otto_db_children",
                "description": "Read-only: expand ONE node of a connection's schema tree (engine-agnostic, lazy). `path` is a NodePath from otto_db_schema/otto_db_children — e.g. 'db:shop' → its folders, 'db:shop/folder:tables' → its tables, 'db:shop/folder:tables/table:orders' → its columns. Redis: 'kdb:0' → key prefixes (optional `filter`).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "connection_id": { "type": "string", "description": "Otto connection id or name (otto_list_connections)." },
                        "path": { "type": "string", "description": "NodePath of the node to expand, e.g. 'db:shop/folder:tables'." },
                        "filter": { "type": "string", "description": "Optional substring/prefix filter (e.g. a Redis key prefix)." }
                    },
                    "required": ["connection_id", "path"]
                }
            },
            {
                "name": "otto_db_object",
                "description": "Read-only: the FULL structure of one table/collection — columns (name + type), primary key, indexes, foreign keys, and the CREATE/DDL where the engine exposes it. `path` is the object's NodePath, e.g. 'db:shop/folder:tables/table:orders'.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "connection_id": { "type": "string", "description": "Otto connection id or name (otto_list_connections)." },
                        "path": { "type": "string", "description": "NodePath of the table/collection." }
                    },
                    "required": ["connection_id", "path"]
                }
            },
            {
                "name": "otto_db_query",
                "description": "Run a READ-ONLY query against a connection and return rows {columns, rows, truncated}. SQL (mysql/postgres/clickhouse): a SELECT/SHOW/DESCRIBE/EXPLAIN/WITH statement. Redis: a read command per line (GET/HGETALL/SCAN/…). Mongo: a find/aggregate. Writes/DDL are REFUSED server-side. `database` scopes the active database (SQL/Mongo); Redis selects a keyspace via `node` 'kdb:N'. `max_rows` caps rows (server hard cap 200).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "connection_id": { "type": "string", "description": "Otto connection id or name (otto_list_connections)." },
                        "statement": { "type": "string", "description": "The read-only statement / command." },
                        "database": { "type": "string", "description": "Optional active database to scope SQL/Mongo execution." },
                        "node": { "type": "string", "description": "Optional raw node context (e.g. 'kdb:0' for a Redis keyspace); overrides `database`." },
                        "max_rows": { "type": "integer", "description": "Optional row cap (clamped to 200)." }
                    },
                    "required": ["connection_id", "statement"]
                }
            },
            {
                "name": "otto_git_pr_review",
                "description": "Read-only: a pull request (state, reviewers, approvals, comments) plus Otto's multi-agent review runs of it (`reviews[]`: id, status, verdict, blocker_count — the `review_id` otto_list_findings takes; empty when none ran).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "repo_id": { "type": "string", "description": REPO_REF_DESC },
                        "pr_number": { "type": "integer", "description": "Pull request number." }
                    },
                    "required": ["pr_number"]
                }
            },
            {
                "name": "otto_product_story",
                "description": "Read-only: a product story context bundle (the story record and its latest agent-ready inject context) by story id.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "story_id": { "type": "string", "description": "Otto product story id, or its Jira key (e.g. GS-123) / exact title." }
                    },
                    "required": ["story_id"]
                }
            },
            {
                "name": "canvas_list_scenes",
                "description": "Read-only: list the Canvas Studio scenes (id/title/format/section/timestamps) in a workspace (default: this session's). Use canvas_create_scene to draw a new one.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "workspace_id": { "type": "string", "description": "Optional workspace id or name (default: this session's)." }
                    }
                }
            },
            {
                "name": "canvas_get_scene",
                "description": "Read-only: a Canvas Studio scene by id or title — its row plus `doc_json` (a JSON STRING holding `{type:\"otto-canvas\", format, source}`: the mermaid / D2 / excalidraw source). Use canvas_update_scene to edit it.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "scene_id": { "type": "string", "description": "Otto canvas scene id or title." }
                    },
                    "required": ["scene_id"]
                }
            },
            {
                "name": "canvas_create_scene",
                "description": "Create a Canvas scene (format: mermaid | d2 | excalidraw) and reference it to this session — it appears in the session's Canvas panel and the Canvas module. Uses this session's workspace (OTTO_WORKSPACE_ID). `source` is the mermaid/D2 diagram text (omit for excalidraw or to start blank).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "title": { "type": "string", "description": "Scene title." },
                        "format": { "type": "string", "description": "mermaid (default) | d2 | excalidraw." },
                        "source": { "type": "string", "description": "Mermaid/D2 diagram source text." },
                        "section": { "type": "string", "description": "Optional folder path to group the scene under, e.g. \"Platform/Staging\"." }
                    },
                    "required": ["title"]
                }
            },
            {
                "name": "canvas_update_scene",
                "description": "Replace a Canvas scene's diagram source (mermaid/D2 text) with new content, preserving its format. Use canvas_get_scene first to read the current source.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "scene_id": { "type": "string", "description": "Otto canvas scene id." },
                        "source": { "type": "string", "description": "The new mermaid/D2 diagram source text." }
                    },
                    "required": ["scene_id", "source"]
                }
            },
            {
                "name": "workbench_list",
                "description": "Read-only: list the session owner's Workbench scratch files in this workspace, incl. global ones (id, name, scope, language, pinned, folder, tags, size, rev, updated_at) — pinned first, newest first. `query` filters by a case-insensitive name substring; `trash: true` lists the trash instead. Read one with workbench_get.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Optional case-insensitive name substring." },
                        "trash": { "type": "boolean", "description": "List trashed files instead of live ones." }
                    }
                }
            },
            {
                "name": "workbench_get",
                "description": "Read-only: get a Workbench file by doc_id or exact name, with metadata and current content. Set revision to read an older version. history adds newest-first metadata (history_limit: default 100, max 200). Continue with before_seq = next_before_seq; a full page may be followed by an empty page.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "doc_id": { "type": "string" },
                        "name": { "type": "string", "description": "Exact file name (used when doc_id is omitted)." },
                        "revision": { "type": "integer", "description": "Optional revision seq to read." },
                        "history": { "type": "boolean", "description": "Include one newest-first revision metadata page." },
                        "history_limit": { "type": "integer", "default": 100, "minimum": 1, "maximum": 200, "description": "Metadata rows per history page, clamped 1–200." },
                        "before_seq": { "type": "integer", "description": "Exclusive revision cursor for an older history page." }
                    }
                }
            },
            {
                "name": "workbench_write",
                "description": "MUTATING (Agents Edit): write a Workbench scratch file's content — update by `doc_id`, else the live file with exact `name`, else create a new file named `name`. Every write is recorded as its own revision in the file's full history (nothing is overwritten or lost; the user can diff and restore). `language` optionally sets json | md | sql | html | mermaid | d2 | csv | yaml | … (default: auto-detect). `scope` (create only): workspace | global. Only write when the user asked for it.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "doc_id": { "type": "string" },
                        "name": { "type": "string", "description": "File name; required when doc_id is omitted." },
                        "content": { "type": "string" },
                        "language": { "type": "string" }, "scope": { "type": "string", "enum": ["workspace", "global"] }
                    },
                    "required": ["content"]
                }
            },
            {
                "name": "design_list",
                "description": "Read-only: list Design Hall artifacts (frames, graphics, sites, 3D scenes, whiteboards, brand kits) — id, title, studio, format, status, head version, who created / last edited it (created_by_name, last_editor_name) and the product stories it implements (story_ids). The library is global; filter by project_id / studio / format / status / story_id. Newest first; for the next page pass cursor = the last row's `updated_at|id`. Use design_search to find references by content.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "workspace_id": { "type": "string", "description": "Optional — narrows the global library to one workspace." },
                        "project_id": { "type": "string", "description": "Optional design project id." },
                        "studio": { "type": "string", "description": "frames | graphics | site | 3d | whiteboard | brand | spatial" },
                        "format": { "type": "string", "description": "html | mermaid | d2 | excalidraw | scene3d | otto-canvas | otto-site | png | glb | …" },
                        "status": { "type": "string", "description": "draft | review | approved | shipped | archived" },
                        "story_id": { "type": "string", "description": "Only artifacts that implement this product story." },
                        "limit": { "type": "integer", "description": "Max rows (default 100, cap 500)." },
                        "cursor": { "type": "string", "description": "Next page: `<updated_at>|<id>` of the previous page's last row." }
                    }
                }
            },
            {
                "name": "design_get",
                "description": "Read-only: one design artifact — metadata, head + approved versions, link counts, its working-copy and thumbnail file paths, and (text formats) its source, from the head or `version` (`v3` or a version id). Cite what you borrow as otto://design/<id>@v<seq>.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "artifact_id": { "type": "string", "description": "Design artifact id." },
                        "version": { "type": "string", "description": "Optional `v<seq>` or version id (default: head)." },
                        "include_content": { "type": "boolean", "description": "Inline the ≤ 256 KiB text source (default true)." }
                    },
                    "required": ["artifact_id"]
                }
            },
            {
                "name": "design_links",
                "description": "Read-only: a design artifact's links — what it uses (embeds, components, brand tokens, the stories it implements, what it was derived from) and where it is used — with the linked artifacts' titles; broken references are flagged.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "artifact_id": { "type": "string", "description": "Design artifact id." },
                        "dir": { "type": "string", "description": "out | in | both (default)." }
                    },
                    "required": ["artifact_id"]
                }
            },
            {
                "name": "design_search",
                "description": "Read-only: full-text search over the design library (titles, tags, extracted copy / layer / token names, linked story keys, project names), shipped work first. Use it to find earlier work to build on and cite.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Search text." },
                        "workspace_id": { "type": "string", "description": "Optional — narrows to one workspace." },
                        "studio": { "type": "string" },
                        "format": { "type": "string" },
                        "status": { "type": "string" },
                        "story_id": { "type": "string" },
                        "project_id": { "type": "string" },
                        "limit": { "type": "integer", "description": "Max hits (default 50)." }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "swarm_list_projects",
                "description": "Read-only: list a swarm's projects (id, name, repo, goal). Project ids feed swarm_list_tasks / swarm_create_task.",
                "inputSchema": { "type": "object", "properties": { "swarm_id": { "type": "string" } }, "required": ["swarm_id"] }
            },
            {
                "name": "swarm_list_tasks",
                "description": "Read-only: list a project's board tasks (id, title, status, assignee, priority, depends_on).",
                "inputSchema": { "type": "object", "properties": { "project_id": { "type": "string" } }, "required": ["project_id"] }
            },
            {
                "name": "swarm_utilization",
                "description": "Read-only: a swarm's board-utilization snapshot — parallel cap vs live runs, ready vs open tasks by status, and which agents are busy/idle. Use it to spot wasted capacity before dispatching.",
                "inputSchema": { "type": "object", "properties": { "swarm_id": { "type": "string" } }, "required": ["swarm_id"] }
            },
            {
                "name": "swarm_create_task",
                "description": "Create a task on a project board (Editor-gated). Unassigned tasks are auto-assigned to the best-fitting agent when scheduled. Managers: prefer editing the plan over piling on new tasks.",
                "inputSchema": { "type": "object", "properties": { "project_id": { "type": "string" }, "title": { "type": "string" }, "description": { "type": "string" }, "assignee_agent_id": { "type": "string" }, "priority": { "type": "string", "description": "low|medium|high|urgent" } }, "required": ["project_id", "title"] }
            },
            {
                "name": "swarm_update_task",
                "description": "Update a board task (Editor-gated): status (backlog|todo|in_progress|in_review|blocked|done|cancelled), assignee_agent_id, priority, title, description. Use to unblock, reassign, reprioritize or close stale items.",
                "inputSchema": { "type": "object", "properties": { "task_id": { "type": "string" }, "status": { "type": "string" }, "assignee_agent_id": { "type": "string" }, "priority": { "type": "string" }, "title": { "type": "string" }, "description": { "type": "string" } }, "required": ["task_id"] }
            },
            {
                "name": "swarm_run_task",
                "description": "Dispatch a board task now (Editor-gated): creates a run and launches the assignee's agent session immediately instead of waiting for the coordinator tick. Refused (409) when the task is not todo/blocked/backlog, the swarm is aborted or budget-paused, or the agent is busy; 400 when the swarm has no active agent.",
                "inputSchema": { "type": "object", "properties": { "task_id": { "type": "string" } }, "required": ["task_id"] }
            },
            {
                "name": "swarm_stop_run",
                "description": "Stop an in-flight swarm run (Editor-gated): cancels the turn and marks the run stopped. Use on wedged or duplicate dispatches. A run that already finished is returned unchanged — check `run.status`.",
                "inputSchema": { "type": "object", "properties": { "run_id": { "type": "string" } }, "required": ["run_id"] }
            },
            {
                "name": "otto_search_memory",
                "description": "Read-only: keyword (FTS) search of this workspace's agent memories for a free-text query; returns the top hits.",
                "inputSchema": { "type": "object", "properties": { "query": { "type": "string" }, "k": { "type": "integer" } }, "required": ["query"] }
            },
            {
                "name": "otto_usage_summary",
                "description": "Read-only: token-usage rollups by provider/day/session/feature (root-only endpoint; non-root callers get a clean error). Optional `days` (default 30); `otto_only` (default true) — false includes usage outside Otto sessions.",
                "inputSchema": { "type": "object", "properties": { "days": { "type": "integer" }, "otto_only": { "type": "boolean" } } }
            },
            {
                "name": "browser_navigate",
                "description": "Open a reader-mode browser tab on `url` (fetches it, netguard-checked — loopback/private/metadata addresses are refused) and return its fetched title. The tab appears in this workspace's Browser module.",
                "inputSchema": { "type": "object", "properties": { "url": { "type": "string" } }, "required": ["url"] }
            },
            {
                "name": "browser_page",
                "description": "Read-only: fetch a URL (netguard-checked) and return its extracted markdown, title, and which engine rendered it. `degraded:true` means the plain-fetch fallback ran (no JS execution).",
                "inputSchema": { "type": "object", "properties": { "url": { "type": "string" } }, "required": ["url"] }
            },
            {
                "name": "browser_query",
                "description": "Read-only: fetch a URL (netguard-checked, same as browser_page) and return every node matching a CSS `selector` — outer HTML and text content per match.",
                "inputSchema": { "type": "object", "properties": { "url": { "type": "string" }, "selector": { "type": "string" } }, "required": ["url", "selector"] }
            },
            {
                "name": "browser_summarize",
                "description": "Fetch a URL (netguard-checked) and run one short-lived agent turn to summarize its markdown (capped at 30k chars) for a developer notebook.",
                "inputSchema": { "type": "object", "properties": { "url": { "type": "string" } }, "required": ["url"] }
            },
            {
                "name": "browser_marks",
                "description": "Read-only: list the DOM marks (annotations) the user picked in this workspace's Browser — selector, visible text, an HTML excerpt, and the user's note per mark, oldest first. Pass `url` to limit to one page. When the user says \"the element I marked\", this is where to look.",
                "inputSchema": { "type": "object", "properties": { "url": { "type": "string" } } }
            },
            {
                "name": "browser_login",
                "description": "Sign in to a stored site credential for `domain` (a Site Credential the user created and explicitly marked \"allow agent use\" — otherwise this is refused). The daemon resolves the credential server-side, drives the fill+submit over CDP, and returns only whether it worked — the password never enters this tool's arguments, result, or the audit log.",
                "inputSchema": { "type": "object", "properties": { "domain": { "type": "string" } }, "required": ["domain"] }
            }
        ]
    })
}

/// The AWS and Kubernetes console tools (split out of [`base_tool_catalog`]
/// to keep it under the function-size ratchet).
pub(super) fn console_tool_catalog() -> Vec<Value> {
    let console = json!([
            // ---- AWS console (docs/design/aws-k8s-consoles.md §6). Every tool
            // shells `aws` CLI v2 through the daemon with the ACCOUNT's own
            // credentials; the caller needs the per-service feature grant
            // (`aws_s3` / `aws_sqs` / `aws_ec2` / `aws_athena` / `aws_eks`).
            // A `login required:` error means the account's SSO session
            // expired — tell the user to press "Sign in" in the AWS module.
            {
                "name": "aws_list_accounts",
                "description": "Read-only: list the AWS accounts configured in Otto — id, name, auth_mode, region, environment and (for users who may configure the account) identity (account/arn) + the cached per-service permission probe. Every other aws_* tool takes an account's id OR name as `account_id`; never contains secrets.",
                "inputSchema": { "type": "object", "properties": {} }
            },
            {
                "name": "aws_s3_list_buckets",
                "description": "Read-only: list the S3 buckets visible to an AWS account (name, creation_date, region). Needs `account_id` from aws_list_accounts. S3 in Otto is read-only by design — there is no upload/delete tool.",
                "inputSchema": { "type": "object", "properties": { "account_id": { "type": "string" }, "region": { "type": "string", "description": "Optional region override (defaults to the account's region)." } }, "required": ["account_id"] }
            },
            {
                "name": "aws_s3_list_objects",
                "description": "Read-only: list one level of an S3 bucket like a folder — `prefixes` (sub-folders) and `objects` (key, size, last_modified, storage_class, etag) under `prefix` (use a trailing `/`). Page with `token` = the previous result's `next_token` while `is_truncated`; `max` caps keys per page (default 500, max 1000).",
                "inputSchema": { "type": "object", "properties": { "account_id": { "type": "string" }, "bucket": { "type": "string" }, "prefix": { "type": "string" }, "token": { "type": "string" }, "max": { "type": "integer" }, "region": { "type": "string" } }, "required": ["account_id", "bucket"] }
            },
            {
                "name": "aws_s3_preview",
                "description": "Read-only: preview the first bytes of a TEXT-like S3 object (text/JSON/CSV/YAML/log) as `{text, truncated, content_type}`. `max_bytes` default 64 KiB, cap 1 MiB. Binary objects return `{binary: true}` — there is no download tool; ask the user to download from the AWS module.",
                "inputSchema": { "type": "object", "properties": { "account_id": { "type": "string" }, "bucket": { "type": "string" }, "key": { "type": "string" }, "max_bytes": { "type": "integer" }, "region": { "type": "string" } }, "required": ["account_id", "bucket", "key"] }
            },
            {
                "name": "aws_sqs_list_queues",
                "description": "Read-only: list an account's SQS queues (`url`, `name`, `fifo`). Optional `prefix` filters by queue-name prefix. The returned `url` is the id every other SQS tool takes.",
                "inputSchema": { "type": "object", "properties": { "account_id": { "type": "string" }, "prefix": { "type": "string" }, "region": { "type": "string" } }, "required": ["account_id"] }
            },
            {
                "name": "aws_sqs_peek",
                "description": "MUTATING (aws_sqs Edit): peek up to `max` (1..10) SQS messages (visibility timeout 0, they stay visible). Each peek bumps their receive count — can dead-letter.",
                "inputSchema": { "type": "object", "properties": { "account_id": { "type": "string" }, "url": { "type": "string", "description": "Queue URL from aws_sqs_list_queues." }, "max": { "type": "integer" }, "region": { "type": "string" } }, "required": ["account_id", "url"] }
            },
            {
                "name": "aws_sqs_send",
                "description": "MUTATING (aws_sqs Edit): send ONE message with `body` to an SQS queue `url`; returns `{message_id}`. FIFO queues (`.fifo`) require `group_id` (and `dedup_id` unless content-based dedup is on). Optional `delay_seconds` (0..900). Only send when the user asked you to produce a message.",
                "inputSchema": { "type": "object", "properties": { "account_id": { "type": "string" }, "url": { "type": "string" }, "body": { "type": "string" }, "delay_seconds": { "type": "integer" }, "group_id": { "type": "string" }, "dedup_id": { "type": "string" }, "message_attributes": { "type": "object" }, "region": { "type": "string" } }, "required": ["account_id", "url", "body"] }
            },
            {
                "name": "aws_ec2_list_instances",
                "description": "Read-only: list EC2 instances in an account/region — instance_id, name (Name tag), state, type, az, private_ip, public_ip, launch_time, platform, vpc_id, subnet_id, tags. Optional `state` filter (pending|running|stopping|stopped|shutting-down|terminated) and `q` free-text filter on name/id/ip. Start/stop/reboot are NOT exposed to agents — use the AWS module.",
                "inputSchema": { "type": "object", "properties": { "account_id": { "type": "string" }, "region": { "type": "string" }, "state": { "type": "string" }, "q": { "type": "string" } }, "required": ["account_id"] }
            },
            {
                "name": "aws_athena_list_tables",
                "description": "Read-only: list the tables of an Athena/Glue `database` with their columns (name, type) — the schema you need before writing SQL for aws_athena_query. Optional `catalog` (default AwsDataCatalog).",
                "inputSchema": { "type": "object", "properties": { "account_id": { "type": "string" }, "database": { "type": "string" }, "catalog": { "type": "string" }, "region": { "type": "string" } }, "required": ["account_id", "database"] }
            },
            {
                "name": "aws_athena_query",
                "description": "MUTATING (aws_athena Edit — Athena bills per byte scanned): START an Athena SQL query and return `{query_execution_id}` immediately. It does NOT wait: poll aws_athena_get_query with that id until `state` is SUCCEEDED (rows in `result`), FAILED (`reason`) or CANCELLED. Pass `database` and/or `workgroup`; if the workgroup has no output location the daemon answers 400 with a hint. Prefer `LIMIT`ed queries — every byte scanned costs money.",
                "inputSchema": { "type": "object", "properties": { "account_id": { "type": "string" }, "sql": { "type": "string" }, "database": { "type": "string" }, "workgroup": { "type": "string" }, "output_location": { "type": "string", "description": "s3://bucket/prefix/ — only when the workgroup has none." }, "region": { "type": "string" } }, "required": ["account_id", "sql"] }
            },
            {
                "name": "aws_athena_get_query",
                "description": "Read-only: status + results of an Athena query execution — `state` (QUEUED|RUNNING|SUCCEEDED|FAILED|CANCELLED), `reason`, `stats` {data_scanned_bytes, execution_ms} and, once SUCCEEDED, `result` {columns, rows, truncated} (header row already dropped). Page more rows with `token` = the previous `next_token`; `max` caps rows per page. Poll every few seconds while QUEUED/RUNNING.",
                "inputSchema": { "type": "object", "properties": { "account_id": { "type": "string" }, "query_execution_id": { "type": "string" }, "token": { "type": "string" }, "max": { "type": "integer" }, "region": { "type": "string" } }, "required": ["account_id", "query_execution_id"] }
            },
            {
                "name": "aws_eks_list_clusters",
                "description": "Read-only: list the EKS clusters of an account/region — name, status, version, endpoint, arn, created_at (list + describe, max 20). To inspect workloads inside one, the user must import it into the Kubernetes module first (`k8s_list_clusters` shows imported clusters with `source: eks`).",
                "inputSchema": { "type": "object", "properties": { "account_id": { "type": "string" }, "region": { "type": "string" } }, "required": ["account_id"] }
            },
            // CloudWatch Logs (read-only; `aws` View on the account's CloudWatch
            // read). Times are epoch MILLISECONDS.
            {
                "name": "aws_logs_list_groups",
                "description": "Read-only: list CloudWatch Logs log groups of an account/region — name, retention_days, stored_bytes, created_ms. Optional `prefix` (log-group-name prefix, e.g. `/aws/eks/` or `/aws/lambda/my-fn`); page with `token` = the previous `next_token`. EKS control-plane logs live in `/aws/eks/<cluster>/cluster`, RDS exports in `/aws/rds/instance/<id>/<log>`.",
                "inputSchema": { "type": "object", "properties": { "account_id": { "type": "string" }, "prefix": { "type": "string" }, "token": { "type": "string" }, "max": { "type": "integer" }, "region": { "type": "string" } }, "required": ["account_id"] }
            },
            {
                "name": "aws_logs_filter",
                "description": "Read-only: read log events from one CloudWatch log `group` (oldest first inside the window) — id, stream, timestamp (epoch ms), message. Narrow with `pattern` (CloudWatch filter pattern: `ERROR`, `\"timed out\"`, `{ $.level = \"error\" }`), `streams` (comma-separated names), `start`/`end` (epoch ms; ALWAYS pass a window — e.g. the last 15 min — or the scan covers the whole retention), `max` (default 200, cap 1000); page with `token` = the previous `next_token`.",
                "inputSchema": { "type": "object", "properties": { "account_id": { "type": "string" }, "group": { "type": "string" }, "pattern": { "type": "string" }, "streams": { "type": "string" }, "start": { "type": "integer" }, "end": { "type": "integer" }, "token": { "type": "string" }, "max": { "type": "integer" }, "region": { "type": "string" } }, "required": ["account_id", "group"] }
            },
            {
                "name": "aws_logs_insights",
                "description": "Read-only (but Logs Insights bills per GB scanned — keep the window tight): START a CloudWatch Logs Insights `query` over `groups` (array, ≤ 50) between `start` and `end` (epoch ms, ≤ 31 days) and return `{query_id}` immediately; poll aws_logs_get_insights until `done`. Example query: `fields @timestamp, @message | filter @message like /ERROR/ | sort @timestamp desc | limit 50`.",
                "inputSchema": { "type": "object", "properties": { "account_id": { "type": "string" }, "groups": { "type": "array", "items": { "type": "string" } }, "query": { "type": "string" }, "start": { "type": "integer" }, "end": { "type": "integer" }, "limit": { "type": "integer" }, "region": { "type": "string" } }, "required": ["account_id", "groups", "query", "start", "end"] }
            },
            {
                "name": "aws_logs_get_insights",
                "description": "Read-only: status + rows of a Logs Insights query started with aws_logs_insights — `status` (Scheduled|Running|Complete|Failed|Cancelled|Timeout), `done`, `result` {columns, rows} (the `@ptr` field dropped), records_matched/scanned, bytes_scanned. Poll every 1–2 s until `done`. Pass the same `region` you started it in.",
                "inputSchema": { "type": "object", "properties": { "account_id": { "type": "string" }, "query_id": { "type": "string" }, "region": { "type": "string" } }, "required": ["account_id", "query_id"] }
            },
            // ---- Kubernetes console (§6). Everything runs `kubectl` with the
            // cluster's own kubeconfig/context server-side; the caller needs the
            // `kubernetes` feature (View for reads, Edit for `k8s_action`).
            {
                "name": "k8s_list_clusters",
                "description": "Read-only: list the Kubernetes clusters registered in Otto — id, name, source (kubeconfig|imported|eks), default_namespace, environment (and, for users who may configure it, context_name + cached `capabilities`). Every other k8s_* tool takes a cluster's id OR name as `cluster_id`.",
                "inputSchema": { "type": "object", "properties": {} }
            },
            {
                "name": "k8s_get_resources",
                "description": "Read-only: list resources of one `kind` (pods | deployments | statefulsets | daemonsets | replicasets | jobs | cronjobs | services | ingresses | configmaps | secrets | pvcs | hpas | rollouts | applications | events) as normalized rows — name, namespace, status, ready, restarts, age_seconds, node, ip, cpu/mem (when metrics-server exists), images, labels, `health` (ok|warn|bad|progressing) and kind-specific `extra` (e.g. deployments: desired/updated/available; rollouts: strategy/step/weight/phase; applications: sync/health/revision). Omit `namespace` for ALL namespaces. Optional `label` selector (`app=web,tier!=cache`) and `q` free-text filter on names. Secret values are never returned.",
                "inputSchema": { "type": "object", "properties": { "cluster_id": { "type": "string" }, "kind": { "type": "string" }, "namespace": { "type": "string" }, "label": { "type": "string" }, "q": { "type": "string" } }, "required": ["cluster_id", "kind"] }
            },
            {
                "name": "k8s_describe",
                "description": "Read-only: full detail of ONE resource — `manifest` (JSON, managedFields stripped, Secret data redacted), `describe` (kubectl describe text) and its recent `events` (type, reason, message, count, last_seen). `kind` is the plural from k8s_get_resources; `namespace` + `name` identify the object (omit `namespace` for cluster-scoped kinds: nodes, namespaces).",
                "inputSchema": { "type": "object", "properties": { "cluster_id": { "type": "string" }, "kind": { "type": "string" }, "namespace": { "type": "string" }, "name": { "type": "string" } }, "required": ["cluster_id", "kind", "name"] }
            },
            {
                "name": "k8s_logs",
                "description": "Read-only: the log tail of a pod as `{text, truncated}` (no follow/stream). `container` selects one container of a multi-container pod (required by kubectl when there is more than one); `tail` = number of lines (default 500); `since` = duration like `10m`/`1h`; `previous:true` reads the crashed previous container instance (use it for CrashLoopBackOff); `timestamps:true` prefixes RFC3339 timestamps. The text is capped at 256 KiB keeping the NEWEST lines.",
                "inputSchema": { "type": "object", "properties": { "cluster_id": { "type": "string" }, "namespace": { "type": "string" }, "pod": { "type": "string" }, "container": { "type": "string" }, "tail": { "type": "integer" }, "since": { "type": "string" }, "previous": { "type": "boolean" }, "timestamps": { "type": "boolean" } }, "required": ["cluster_id", "namespace", "pod"] }
            },
            {
                "name": "k8s_top",
                "description": "Read-only: live CPU (millicores) and memory (bytes) usage per pod (with per-container breakdown) from metrics-server, optionally limited to `namespace`. `available:false` means the cluster has no metrics-server — nothing else to fetch.",
                "inputSchema": { "type": "object", "properties": { "cluster_id": { "type": "string" }, "namespace": { "type": "string" } }, "required": ["cluster_id"] }
            },
            {
                "name": "k8s_health",
                "description": "Read-only: compact health digest for a MONITORED cluster (Kubernetes → Monitor enabled): classified restarts (oom / crash / probe) with pod + memory-limit detail, planned churn, memory outliers vs limits, error-rate / p95 spikes vs the 24h baseline, version drift and the collector + metrics-server status. `window` = 1h|6h|24h|7d (default 1h). Every list is capped at 20; prefer it over k8s_get_resources for periodic health checks.",
                "inputSchema": { "type": "object", "properties": { "cluster_id": { "type": "string" }, "window": { "type": "string" } }, "required": ["cluster_id"] }
            },
            {
                "name": "k8s_action",
                "description": "MUTATING (kubernetes Edit): run ONE operational action on a resource via kubectl and return `{ok, message, output}`. `action` ∈ restart (deployments/statefulsets/daemonsets/rollouts) · scale (params.replicas) · delete_pod (pods; params.grace) · rollout_status · rollout_undo (params.to_revision) · rollout_pause / rollout_resume · rollout_promote (rollouts; params.full) · rollout_abort · rollout_retry · argocd_sync (applications; params.revision, params.prune) · argocd_refresh (params.hard) · argocd_terminate_op · argocd_app_restart (params.resource_kind) · cronjob_trigger · cronjob_suspend / cronjob_resume. DESTRUCTIVE actions (delete_pod, scale to 0, rollout_undo, argocd_sync with prune) are refused unless `params.confirm_name` equals `name` — set it only after the user explicitly confirmed. Cluster RBAC denials surface as 403.",
                "inputSchema": { "type": "object", "properties": { "cluster_id": { "type": "string" }, "action": { "type": "string" }, "kind": { "type": "string" }, "namespace": { "type": "string" }, "name": { "type": "string" }, "params": { "type": "object" } }, "required": ["cluster_id", "action", "kind", "namespace", "name"] }
            },
            {
                "name": "k8s_pod_actions_list",
                "description": "Read-only: the saved pod HTTP actions of a cluster as `{actions: [{id, namespace, workload_kind, workload, name, method, port, path, headers, body_template}]}` — `path`/`body_template` may carry `{{logger}}` / `{{level}}` variables to fill before calling k8s_pod_http. Optional `namespace`, `workload_kind`, `workload` filters.",
                "inputSchema": { "type": "object", "properties": { "cluster_id": { "type": "string" }, "namespace": { "type": "string" }, "workload_kind": { "type": "string" }, "workload": { "type": "string" } }, "required": ["cluster_id"] }
            },
            {
                "name": "k8s_pod_http",
                "description": "MUTATING (kubernetes Edit for non-GET): send ONE HTTP request to a pod's container `port` through the API-server pod proxy (port-forward fallback) and return `{results: [{pod, status, duration_ms, headers, body, body_base64, truncated, error, via}], target_name, mutating}`. Target exactly one of `pod` or `workload` {kind: deployment|statefulset|daemonset|replicaset|job, name} (every running pod, ≤50, `max_concurrency` ≤8). Spring Boot actuator examples: GET /actuator/loggers · GET /actuator/loggers/<logger> · POST /actuator/loggers/<logger> with body {\"configuredLevel\":\"DEBUG\"} and header Content-Type: application/json · GET /actuator/health|info|env · POST /actuator/refresh · GET /actuator/threaddump. `timeout_ms` ≤30000; bodies are capped at 256 KiB. A non-GET method on a PROD cluster is refused (409 confirm_required) unless `confirm_name` equals the pod/workload name — set it only after the user explicitly confirmed.",
                "inputSchema": { "type": "object", "properties": { "cluster_id": { "type": "string" }, "namespace": { "type": "string" }, "pod": { "type": "string" }, "workload": { "type": "object", "properties": { "kind": { "type": "string" }, "name": { "type": "string" } } }, "port": { "type": "integer" }, "method": { "type": "string" }, "path": { "type": "string" }, "headers": { "type": "object" }, "body": { "type": "string" }, "timeout_ms": { "type": "integer" }, "max_concurrency": { "type": "integer" }, "confirm_name": { "type": "string" } }, "required": ["cluster_id", "namespace", "port", "method", "path"] }
            }
    ]);
    match console {
        Value::Array(tools) => tools,
        _ => unreachable!("a JSON array literal"),
    }
}

/// Session sources that do ONE narrow job — review a PR, summarize a review,
/// draft a PR/commit message, review or evaluate a skill. Their `tools/list`
/// leaves out the infrastructure consoles (AWS, Kubernetes, Kafka brokers),
/// the canvas and the in-app browser — ~20 KB of schema Codex would otherwise
/// re-read every turn (perf2/10-mcp R2). LISTING only: a call by name still
/// runs through the normal dispatch and the daemon's RBAC, exactly as before.
pub(super) const NARROW_SOURCES: &[&str] = &[
    "review",
    "review_summarizer",
    "pr-draft",
    "commit-draft",
    "skillreview",
    "skilleval",
];

/// Tool-name prefixes a [`NARROW_SOURCES`] session is not shown.
pub(super) const NARROW_HIDDEN_PREFIXES: &[&str] = &[
    "aws_",
    "workbench_",
    "k8s_",
    "canvas_",
    "browser_",
    "otto_list_broker_",
    "otto_get_broker_",
    "otto_consume_broker_",
    "otto_produce_broker_",
    "otto_list_consumer_groups",
];

pub(super) fn is_narrow_source(source: Option<&str>) -> bool {
    source.is_some_and(|s| NARROW_SOURCES.contains(&s))
}

/// Drop the families a narrow session does not need from a listing.
pub(super) fn scope_listing_for_source(tools: &mut Vec<Value>, source: Option<&str>) {
    if !is_narrow_source(source) {
        return;
    }
    tools.retain(|t| {
        t["name"]
            .as_str()
            .is_some_and(|n| !NARROW_HIDDEN_PREFIXES.iter().any(|p| n.starts_with(p)))
    });
}

pub(super) fn is_vault_docs_reviewer(source: Option<&str>) -> bool {
    source == Some("vault-docs-review")
}

pub(super) fn tool_catalog_for_source(source: Option<&str>) -> Value {
    let mut catalog = tool_catalog();
    if source != Some(ASSISTANT_SOURCE) {
        if let Some(tools) = catalog["tools"].as_array_mut() {
            tools.retain(|tool| {
                tool["name"]
                    .as_str()
                    .is_some_and(|name| assistant_segment(name).is_none())
            });
        }
    }
    if is_vault_docs_reviewer(source) {
        if let Some(tools) = catalog["tools"].as_array_mut() {
            tools.retain(|tool| {
                tool["name"].as_str().is_some_and(|name| {
                    VAULT_REVIEW_READ_TOOLS
                        .iter()
                        .any(|(internal, _)| *internal == name)
                })
            });
        }
    }
    catalog
}

// ---------------------------------------------------------------------------
// Control-plane (governed `otto.*`) tools on this stdio surface.
//
// The operator's MCP → Otto server checklist (`GET /mcp/otto-server`, backed
// by `otto_tool_specs()`) is the ONE catalog the control plane manages. Before
// this bridge existed, a session saw only the hand-written catalog above, so a
// tool enabled in the control plane (`otto.create_pr`, `otto.open_pr_draft`,
// `otto.run_workflow`…) was simply absent from the agent's `tools/list` — the
// two catalogs drifted with nothing to grep for. Now every ENABLED governed
// tool that this surface does not already serve natively is advertised under
// the stdio naming (`otto.create_pr` → `otto_create_pr`) and its calls are
// proxied through `POST /mcp/otto-tools/invoke`, the same allow-list →
// approval → audit choke point the outward server and the HTTP transport use.
// Native tools keep winning by name: they are session-aware (workspace
// injection, reviewer scoping) and the governed twin would only add noise.
// ---------------------------------------------------------------------------

/// Governed tools (short name, no `otto.` prefix) whose capability this surface
/// already serves natively under a DIFFERENT name. They are not re-advertised —
/// the agent should see one tool per capability — and a call by their stdio
/// name is still routed to the governed path, which is harmless (same RBAC).
pub(super) const GOVERNED_ALIASED_BY_NATIVE: &[(&str, &str)] = &[
    ("get_usage_summary", "otto_usage_summary"),
    ("get_product_story", "otto_product_story"),
    ("query_db_readonly", "otto_db_query"),
    // Design Hall reads are served natively under their bare names.
    ("design_list", "design_list"),
    ("design_get", "design_get"),
    ("design_links", "design_links"),
    ("design_search", "design_search"),
    // Otto Assistant memory: served natively (session-bound, chip + Undo).
    ("assistant_remember", "assistant_remember"),
    ("assistant_forget", "assistant_forget"),
    ("assistant_recall", "assistant_recall"),
    // Swarm board reads + PR review runs are native under other names.
    ("list_swarm_projects", "swarm_list_projects"),
    ("list_swarm_tasks", "swarm_list_tasks"),
    ("list_pr_reviews", "otto_git_pr_review"),
    // The cloud consoles are native under their bare names; without these the
    // session saw every AWS/K8s tool twice (`k8s_top` AND `otto_k8s_top`).
    ("aws_list_accounts", "aws_list_accounts"),
    ("aws_s3_list_buckets", "aws_s3_list_buckets"),
    ("aws_s3_list_objects", "aws_s3_list_objects"),
    ("aws_s3_preview", "aws_s3_preview"),
    ("aws_sqs_list_queues", "aws_sqs_list_queues"),
    ("aws_sqs_peek", "aws_sqs_peek"),
    ("aws_sqs_send", "aws_sqs_send"),
    ("aws_ec2_list_instances", "aws_ec2_list_instances"),
    ("aws_athena_list_tables", "aws_athena_list_tables"),
    ("aws_athena_query", "aws_athena_query"),
    ("aws_athena_get_query", "aws_athena_get_query"),
    ("aws_eks_list_clusters", "aws_eks_list_clusters"),
    ("aws_logs_list_groups", "aws_logs_list_groups"),
    ("aws_logs_filter", "aws_logs_filter"),
    ("aws_logs_insights", "aws_logs_insights"),
    ("aws_logs_get_insights", "aws_logs_get_insights"),
    ("k8s_list_clusters", "k8s_list_clusters"),
    ("k8s_get_resources", "k8s_get_resources"),
    ("k8s_describe", "k8s_describe"),
    ("k8s_logs", "k8s_logs"),
    ("k8s_top", "k8s_top"),
    ("k8s_health", "k8s_health"),
    ("k8s_action", "k8s_action"),
    ("k8s_pod_http", "k8s_pod_http"),
    ("k8s_pod_actions_list", "k8s_pod_actions_list"),
];

/// How long a governed call waits for a human decision before returning
/// `pending_approval`. Below [`CALL_TIMEOUT`] so the daemon answers before this
/// bridge gives up; a later identical call reuses the approval once granted.
pub(super) const GOVERNED_WAIT_SECS: u64 = otto_core::mcp_budgets::GOVERNED_WAIT_SECS;

/// The stdio-facing name of a governed catalog entry: `otto.create_pr` →
/// `otto_create_pr`. Claude/Codex mangle a dotted tool name, and the native
/// tools already use this underscore form.
pub(super) fn governed_stdio_name(spec_name: &str) -> String {
    format!(
        "otto_{}",
        spec_name.strip_prefix("otto.").unwrap_or(spec_name)
    )
}

/// Names of the tools served natively by this binary (the static catalog).
#[cfg(test)]
pub(super) fn native_tool_names() -> Vec<String> {
    NATIVE_TOOL_NAMES.clone()
}

/// The governed spec a stdio tool name proxies to, or `None` when the name is
/// served natively (by the same or an aliased name) or is not a governed tool.
pub(super) fn governed_spec_for_stdio_name(name: &str) -> Option<Value> {
    governed_spec_ref(name).cloned()
}

/// [`governed_spec_for_stdio_name`] without the clone: an index lookup.
pub(super) fn governed_spec_ref(name: &str) -> Option<&'static Value> {
    GOVERNED_BY_STDIO_NAME
        .get(name)
        .map(|&i| &GOVERNED_SPECS[i])
}

/// stdio name → index into [`GOVERNED_SPECS`], for every governed spec this
/// surface PROXIES (not served natively under the same or an aliased name).
/// Built once; the per-spec linear scans it replaces made `tools/list`
/// O(N²) at N≈216.
pub(super) static GOVERNED_BY_STDIO_NAME: std::sync::LazyLock<
    std::collections::HashMap<String, usize>,
> = std::sync::LazyLock::new(|| {
    let native: std::collections::HashSet<&str> =
        NATIVE_TOOL_NAMES.iter().map(String::as_str).collect();
    GOVERNED_SPECS
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            let full = s["name"].as_str()?;
            let stdio = governed_stdio_name(full);
            let short = stdio.strip_prefix("otto_")?;
            let aliased = GOVERNED_ALIASED_BY_NATIVE
                .iter()
                .any(|(g, n)| *g == short && native.contains(n));
            (!native.contains(stdio.as_str()) && !aliased).then_some((stdio, i))
        })
        .collect()
});

/// The governed (`otto.*`) name a stdio tool name proxies to, if any.
#[cfg(test)]
pub(super) fn governed_tool_for_stdio_name(name: &str) -> Option<String> {
    governed_spec_for_stdio_name(name).and_then(|s| s["name"].as_str().map(str::to_string))
}

/// The `tools/list` entries for the control-plane-enabled governed tools that
/// have no native equivalent here. `enabled` holds full `otto.*` names. Every
/// enabled `otto_ui_*` tool is included — see [`governed_tools_for_session`]
/// for the grant-aware listing.
#[cfg(test)]
pub(super) fn governed_tools_for(enabled: &[String]) -> Vec<Value> {
    governed_tools_listed(enabled, true)
}

/// The governed `tools/list` entries for THIS session: the ~90 `otto_ui_*`
/// tools only when the session holds the "Allow UI control" grant — without
/// it every one of them answers `pending_grant`, yet they made up ~40% of a
/// session's tool-schema context. Ungranted, ONE stub
/// ([`UI_REQUEST_CONTROL_TOOL`]) stands in for them (when `otto.ui_state` is
/// enabled), and a call to a UI tool by name still works exactly as before.
pub(super) fn governed_tools_for_session(info: &EnabledInfo) -> Vec<Value> {
    let mut out = governed_tools_listed(&info.names, info.ui_granted);
    if !info.ui_granted && info.names.iter().any(|n| n == "otto.ui_state") {
        out.push(json!({
            "name": UI_REQUEST_CONTROL_TOOL,
            "description": "Ask the person to let you drive the Otto app's UI (open pages, \
                run queries, use the API client, browser, git…) for this session. Otto shows \
                them an Allow prompt; once they allow it, the full set of otto_ui_* tools \
                appears in your tool list. Call it only when the task needs the Otto UI.",
            "inputSchema": { "type": "object", "properties": {} },
        }));
    }
    out
}

pub(super) fn governed_tools_listed(enabled: &[String], include_ui: bool) -> Vec<Value> {
    let enabled: std::collections::HashSet<&str> = enabled.iter().map(String::as_str).collect();
    GOVERNED_SPECS
        .iter()
        .filter_map(|s| {
            let full = s["name"].as_str()?;
            if !enabled.contains(full) || (!include_ui && is_ui_spec_name(full)) {
                return None;
            }
            let stdio = governed_stdio_name(full);
            if !GOVERNED_BY_STDIO_NAME.contains_key(&stdio) {
                return None;
            }
            Some(json!({
                "name": stdio,
                "description": s["description"],
                "inputSchema": s["inputSchema"],
            }))
        })
        .collect()
}

/// Whether a full governed name is an agent-UI-control tool (`otto.ui_*`).
pub(super) fn is_ui_spec_name(full: &str) -> bool {
    otto_server::ui_commands::is_ui_tool(full.strip_prefix("otto.").unwrap_or(full))
}

/// Schema text for the cross-workspace list tools' optional `workspace_id`.
pub(super) const WS_DIR_DESC: &str = "Optional workspace id or name; omit = all you can read.";

/// Kept SHORT (perf2/10-mcp R2): it repeats in every repo-taking schema of a
/// catalog Codex re-reads each turn; an ambiguous/unknown reference still
/// answers with the candidates.
pub(super) const REPO_REF_DESC: &str =
    "Repo id, name, path or remote (any readable workspace). Omit = this session's repo.";
