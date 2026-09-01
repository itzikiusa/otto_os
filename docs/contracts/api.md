# Otto API Contract — /api/v1 (FROZEN)

All DTO names refer to types in `crates/otto-core/src/api.rs` (Rust) mirrored in
`ui/src/lib/api/types.ts` (TS). JSON: snake_case fields, RFC3339 timestamps, ULID ids.
Auth: `Authorization: Bearer <token>` unless marked public. Errors: HTTP status per
`otto_core::Error` variant + body `Problem{code,message}`.

Roles: `root` = global; workspace roles `viewer < editor < admin`. Root passes every check.
"member" below means any authenticated user; workspace-scoped routes require at least the
listed role IN THAT WORKSPACE. Sessions/connections/repos/PRs inherit their workspace.

**Global (workspace-less) connections.** `POST /workspaces/{id}/connections` always
persists `workspace_id = NULL` — connections are a *global library*, visible from every
workspace (row #25). A row with no workspace has no workspace role to check, so those
routes fall through to the **feature axis** instead of the workspace axis, on the same
ladder (`viewer→View`, `editor→Edit`, `admin→Admin`): reading a global connection's schema
takes `Database:View`, running a query takes `Database:Edit`, using one takes
`Connections:Edit`, and editing/deleting the shared *record* takes `Connections:Admin` —
so an Edit-level teammate can use the shared library without rewriting it for everyone.
Root still passes everything. (Before, this branch was root-only, which made the entire
connection library unusable for every non-root account.)

| # | Method & path | Auth | Request | Response |
|---|---|---|---|---|
| 1 | GET /api/v1/health | public | — | `{"ok":true}` |
| 2 | GET /api/v1/meta | public | — | MetaResp |
| 3 | POST /api/v1/onboarding/root | public, only while 0 users exist (else 409) | OnboardRootReq | LoginResp |
| 4 | POST /api/v1/auth/login | public | LoginReq | LoginResp (401 on bad creds/disabled) |
| 5 | POST /api/v1/auth/logout | member | — | 204 |
| 6 | GET /api/v1/auth/me | member | — | `MeResp {user, real_user, impersonating}` — `user` = effective (auth target); `real_user` = token owner (= `user` for normal sessions); `impersonating: bool` |
| 7 | GET /api/v1/users | root | — | `User[]` |
| 8 | POST /api/v1/users | root | CreateUserReq | User (409 dup username) |
| 9 | PATCH /api/v1/users/{id} | root | UpdateUserReq | User |
| 10 | DELETE /api/v1/users/{id} | root | — | 204 (soft: sets disabled; root user cannot be disabled → 400) |
| 11 | GET /api/v1/workspaces | member | — | `WorkspaceWithRole[]` (root sees all as admin) |
| 12 | POST /api/v1/workspaces | member | CreateWorkspaceReq | Workspace (creator becomes admin member) |
| 13 | PATCH /api/v1/workspaces/{id} | ws admin | UpdateWorkspaceReq | Workspace |
| 14 | DELETE /api/v1/workspaces/{id} | ws admin | — | 204 (archives) |
| 15 | GET /api/v1/workspaces/{id}/members | ws admin | — | `MemberEntry[]` |
| 16 | PUT /api/v1/workspaces/{id}/members | ws admin | SetMembersReq | `MemberEntry[]` |
| 17 | GET /api/v1/workspaces/{id}/sessions | ws viewer | — | `Session[]` |
| 18 | POST /api/v1/workspaces/{id}/sessions | ws editor | CreateSessionReq | Session |
| 19 | GET /api/v1/sessions/{id} | ws viewer | — | Session |
| 20 | PATCH /api/v1/sessions/{id} | ws editor | UpdateSessionReq | Session |
| 21 | DELETE /api/v1/sessions/{id} | ws editor | — | 204 (kills PTY, removes row) |
| 22 | POST /api/v1/sessions/{id}/restart | ws editor | — | Session (respawn; uses resume args when provider_session_id set) |
| 23 | POST /api/v1/workspaces/{id}/orchestrate | ws editor | OrchestrateReq | OrchestrateResp |
| 24 | POST /api/v1/workspaces/{id}/orchestrate/execute | ws editor | ExecutePlanReq | `{"results":[{"action_index":0,"ok":true,"detail":"...","session_ids":["..."]}]}` |
| 25 | GET /api/v1/workspaces/{id}/connections | ws viewer | — | `Connection[]` (includes global ones; secret never present) |
| 26 | POST /api/v1/workspaces/{id}/connections | ws editor | UpsertConnectionReq | Connection |
| 27 | PATCH /api/v1/connections/{id} | ws editor (global: `Connections:Admin`) | UpsertConnectionReq (PATCH semantics: absent secret = keep; absent `environment`/`read_only` = **preserve** the stored value — never reset to dev/false, so a partial PATCH can't disable the write-guard) | Connection |
| 27a | PATCH /api/v1/connections/{id}/pin | ws editor (global: `Connections:Edit`) | `{pinned: bool}` | Toggle pinned/frecency flag; returns updated Connection |
| 28 | DELETE /api/v1/connections/{id} | ws editor (global: `Connections:Admin`) | — | 204 (deletes Keychain secret too) |
| 29 | POST /api/v1/connections/{id}/open | ws editor | `{"title":null}` optional | Session |
| 30 | POST /api/v1/connections/{id}/test | ws editor | — | TestConnectionResp (`warn_key_perms?: string` — set when the connection's SSH private key file is group/other-readable; carries the `chmod 600 <path>` fix, independent of `ok`) |
| 30a | GET /api/v1/workspaces/{id}/connections/import/sources | ws editor | — | `SourceStatus[]` — detects MySQL Workbench / DBeaver / DataGrip / NoSQLBooster at their default macOS config paths (the daemon runs locally and reads the files itself; the user picks a tool, never a file) |
| 30b | POST /api/v1/workspaces/{id}/connections/import/scan | ws editor | `{source: ImportSource}` | ImportScanResult — locates + reads + parses the chosen tool's default config into `ParsedConnection[]` (ready-to-create Otto params; unsupported engines listed with `supported:false`) |
| 30c | POST /api/v1/workspaces/{id}/connections/import/create | ws editor | ImportCreateReq | ImportCreateResult `{created: Connection[], failed: {name,error}[]}` — best-effort batch create through the normal create path with `secret:null` (tools keep passwords encrypted/in an OS keychain — unrecoverable; the user adds them later via edit) |
| 31 | GET /api/v1/git/accounts | member | — | `GitAccount[]` (own accounts only; token never present) |
| 32 | POST /api/v1/git/accounts | member | CreateGitAccountReq | GitAccount |
| 33 | DELETE /api/v1/git/accounts/{id} | member (owner) | — | 204 |
| 34 | GET /api/v1/workspaces/{id}/repos | ws viewer | — | `Repo[]` |
| 35 | POST /api/v1/workspaces/{id}/repos | ws editor | AddRepoReq | Repo (clone runs async; Notice events report progress/done) |
| 36 | DELETE /api/v1/repos/{id} | ws editor | — | 204 (unregisters; never deletes files) |
| 36b | PATCH /api/v1/repos/{id} | ws editor (+ account owner, S4) | UpdateRepoReq | Repo — (re)bind the repo's hosting account (see the extended row below) |
| 37 | GET /api/v1/repos/{id}/status | ws viewer | — | RepoStatusResp — includes `op_in_progress` (`"merge"\|"rebase"\|"cherry_pick"\|"revert"`, absent when none), detected from the git dir's state files; conflicted files can exist without it (a conflicting stash pop). Local git refusals across the git routes (dirty tree, unresolved index, bad ref, index.lock, …) map to 409 with git's own message; 502 is reserved for genuine remote/auth failures, and forge HTTP errors map by status (401/403 → 403 credential-rejected, 404 → 404, 405/409/422 → 409). |
| 38 | GET /api/v1/repos/{id}/branches | ws viewer | — | `BranchInfo[]` |
| 39 | GET /api/v1/repos/{id}/log?limit=50&skip=0&all=false | ws viewer | — | `CommitInfo[]` — `limit` defaults to 50 and is **uncapped**; `limit=0` means the whole reachable history (no `-n`). Page with `skip` + `limit`; ordering is stable across pages for a fixed set of refs. |
| 40 | GET /api/v1/repos/{id}/diff?target=worktree\|staged\|commit:<sha>\|range:<a>..<b> | ws viewer | — | DiffResp |
| 41 | POST /api/v1/repos/{id}/stage | ws editor | StagePathsReq | RepoStatusResp |
| 42 | POST /api/v1/repos/{id}/unstage | ws editor | StagePathsReq | RepoStatusResp |
| 43 | POST /api/v1/repos/{id}/commit | ws editor | CommitReq | `{"sha":"..."}` |
| 44 | POST /api/v1/repos/{id}/push | ws editor | `{branch?}` (optional; pushes THAT branch explicitly — Create-PR passes its source branch; absent = current branch) | RepoStatusResp |
| 45 | POST /api/v1/repos/{id}/pull | ws editor | `{auto_stash?}` (optional) | `{status: RepoStatusResp, note?}` — a pull whose merge CONFLICTS is a normal 200: the fetch landed and a merge is left in progress, with the unmerged paths returned as `status.changes[].kind="conflicted"` (clients route to the conflict resolver). `auto_stash:true` wraps a dirty tree in stash → pull → pop (`note` says what happened to the stash: restored, kept because the pull conflicted, or pop conflicted); a refused auto-stash pull pops the stash back. Local refusals (dirty tree, no upstream, divergent branches, unfinished merge) are 409 with git's own line; only genuine network/auth failures are 502. |
| 46 | POST /api/v1/repos/{id}/checkout | ws editor | CheckoutReq | RepoStatusResp |
| 47 | POST /api/v1/repos/{id}/stash | ws editor | `{"op":"save"\|"pop"\|"apply"\|"drop","sha"?:"..."}` (`sha` required for apply/drop — SHA-anchored, resolved to the live `stash@{N}`; conflicts on pop/apply return 200 with the tree left for resolution) | RepoStatusResp |
| 48 | GET /api/v1/repos/{id}/prs?state=open\|merged\|declined\|all | ws viewer | — | `PrSummary[]` |
| 49 | POST /api/v1/repos/{id}/prs | ws editor | CreatePrReq (optional `draft` — GitHub native flag, GitLab `Draft:` title prefix, Bitbucket Cloud draft field; optional `reviewers: string[]` of provider-native handles) | PrSummary (`reviewer_warnings: string[]` — reviewer requests/lookups that failed after the PR opened; never fails the creation) |
| 50 | GET /api/v1/repos/{id}/prs/{number} | ws viewer | — | PrDetail |
| 51 | GET /api/v1/repos/{id}/prs/{number}/diff | ws viewer | — | DiffResp |
| 52 | PATCH /api/v1/repos/{id}/prs/{number} | ws editor | UpdatePrReq | 204 |
| 53 | POST /api/v1/repos/{id}/prs/{number}/comments | ws editor | NewPrCommentReq | PrComment (carries `resolved: bool` + `thread_id?: string` on thread heads — Bitbucket comment id, GitLab discussion id, GitHub GraphQL reviewThread node id) |
| 53b | POST /api/v1/repos/{id}/prs/{number}/comments/{cid}/resolve | ws editor | ResolvePrThreadReq `{"resolved": bool}` — `{cid}` is `PrComment.thread_id`; `false` reopens | 204 |
| 54 | POST /api/v1/repos/{id}/prs/{number}/approve | ws editor | — | 204 |
| 55 | POST /api/v1/repos/{id}/prs/{number}/merge | ws editor | MergePrReq | 204 |
| 56 | POST /api/v1/repos/{id}/prs/{number}/decline | ws editor | — | 204 |
| 57 | GET /api/v1/settings | root | — | `{ "<key>": <value_json>, ... }` |
| 58 | PUT /api/v1/settings | root | same shape | same shape |

Usage & metrics (embedded ClickHouse, all root-only; types in `crates/otto-usage`):
- GET /usage/status → UsageStatus (engine + ClickHouse health).
- GET /usage/summary?days=N&otto_only=B → UsageSummary. `days` 1–3650 (default 30),
  `otto_only` (default true) excludes externally-recorded sessions. Carries provider,
  daily, session, and **`by_kind`** (per-feature) rollups.
- GET /usage/by-kind?days=N&otto_only=B → `FeatureUsage[]` — the same per-feature rollup
  on its own. `FeatureUsage{feature, events, input_tokens, output_tokens,
  cache_read_tokens, cache_write_tokens, total_tokens, cost_usd, sessions}`. `feature` is
  the kind of Otto work — `review`|`product`|`channel`|`agent`|`connection`|`external`|…
  — derived server-side from each session's metadata (same label as a session row's
  `kind`). Visibility only; no budgets/enforcement. Pricing is unchanged (per-row
  `cost_usd` summed).
- GET /usage/metrics?minutes=N → `MetricPoint[]` (system CPU/RAM/load time-series).
- PUT /usage/config → UsageStatus (update + persist engine config).
- POST /usage/install → UsageStatus (install/update ClickHouse via the official installer).
- GET /usage/budgets → UsageBudgetStatus — the persisted budget config plus live status rows
  (spend vs cap) over the window. Status is computed even when enforcement is off, so the UI can
  preview caps before turning them on.
- PUT /usage/budgets → UsageBudgetStatus — replace + persist the budget config (returns refreshed
  status). Body is `UsageBudgetConfig{enforce, block_on_exceed, window_days, workspaces[], providers[]}`.
  **Enforcement is opt-in:** `enforce` defaults `false`, so budgets are purely informational
  (warnings only) until a root user turns it on; `block_on_exceed` (default `false`) further gates
  whether an exceeded cap is a hard block or warn-only. `WorkspaceBudget{workspace_id, monthly_usd}`
  and `ProviderBudget{provider, monthly_usd}` cap USD spend over `window_days` (default 30,
  clamped 1..3650); a `0` cap = no cap. `BudgetStatusRow{scope, key, label?, limit_usd, spent_usd,
  used_fraction, warning(≥80%), exceeded(≥100%)}`. The daemon exposes a consultable
  `routes::usage::check_budget(ctx, workspace_id, provider)` that is a no-op while `enforce` is off.
- POST /ingest/usage → 204 — per-session token-usage ingest, gated by the per-session
  ingest token (`X-Otto-Session` + `X-Otto-Token`), not a bearer token. Rows recorded here
  get `origin: "ingest"` when the session's work ref supplies none, so they are never
  mistaken for transcript-tailer rows (the tailer's dedup rebuild purges only dim-less rows).

Notes:
- `Connection` carries `environment` (`dev`|`staging`|`prod`, default `dev`) and `read_only`
  (bool, default `false`). `UpsertConnectionReq` accepts both: on **create** absent → defaults
  (`dev`/`false`); on **PATCH** absent → preserve the stored value. A connection is
  *write-guarded* when `environment=prod` OR `read_only=true`.
- DB Explorer query (`POST /api/v1/connections/{id}/db/query`, body `QueryRequest`) enforces the
  guardrail: on a write-guarded connection a statement classified as a write/DDL is rejected with
  `409 conflict` and a `Problem.message` prefixed `write_blocked: ` unless the request sets
  `confirm_write:true`. Read-vs-write is classified conservatively per engine (unknown → write).
  `explain:true` does NOT exempt a statement (the SQL drivers execute by statement text and ignore
  the flag), so a genuine read still passes on its own classification while a raw write tagged
  `explain:true` is still blocked. The UI requires a typed confirmation before sending
  `confirm_write`.
- DB read-only MCP query (`POST /api/v1/connections/{id}/db/mcp-query`, ws **viewer**;
  global connections: `Database:View`) — the agent-facing query path used by `ottod mcp-tools`. Body
  `{statement, max_rows?, node?}` → `QueryResult`. Read-only is enforced **unconditionally**
  (independent of the connection's write-guard): a statement classified as a write/DDL is
  rejected with `403 forbidden` and a `Problem.message` prefixed `mcp_read_only: ` **before**
  any driver runs; non-queryable connection kinds (ssh/custom) → `400 invalid`. Rows are
  hard-capped (200) and cell values are PII-masked server-side.
- Session create with kind=connection requires `connection_id`; provider is set server-side
  to the connection kind. Title defaults: agent → "<provider> #N", connection → conn name.
- PR routes resolve the provider + account from the repo row (`provider`, `git_account_id`);
  if the repo has no provider/account → 400 `invalid`.
- `/orchestrate` never executes; it only returns a plan. Execution is the separate call #24.
- Settings keys used in v1: `network_listener` `{enabled:bool, port:u16}`, `providers`
  (provider registry overrides), `default_provider` (string), `cli_auto_update`
  `{enabled:bool, time_of_day:"HH:MM", use_utc:bool, reload_sessions:bool}` (daily
  auto-update of the agent CLIs; default `{true,"03:00",true,true}` = 03:00 UTC) and
  the daemon-written cursor `cli_auto_update_last_run` (RFC3339). The scheduler
  catches up a missed window on next boot and, when `reload_sessions`, restarts open
  agent sessions onto the new binary (resume-aware).
- `process_sandbox` `{enabled:bool, network:"full"|"loopback"|"none", providers:str[]}`
  — opt-in **OS-level confinement** for spawned agent/shell sessions (macOS Apple
  Seatbelt / `sandbox-exec`; no-op elsewhere). Default **off**. When enabled, each
  agent CLI runs under a Seatbelt profile that denies filesystem **writes** outside
  the workspace cwd, the resolved git dir (so worktree commits still work), the
  agent CLIs' own config/cache dirs and temp — while leaving reads global. `network`
  defaults to `full` (agents still reach their model API; loopback always allowed);
  `loopback`/`none` are stricter postures suited to non-model shells. `providers`
  defaults to `["claude","codex","agy","shell"]`. Connection sessions are never
  sandboxed. (Settings:Admin via `PUT /settings`.)
- `otto_mcp_enabled` — toggles the first-party `otto` MCP server (Otto's read-only tools +
  the read-only DB connection tools: `otto_list_connections`, `otto_db_schema`/`_children`/
  `_object`, `otto_db_query`) attached to every agent session. **Default ON** (opt-out): an
  absent value or unlisted workspace resolves to enabled. A bare scalar `true`/`false` is the
  global toggle; a `{ "<ws>": bool }` object overrides per workspace. Claude/agy receive it via
  the workspace `.mcp.json`; Codex via per-spawn `-c mcp_servers.otto.*` overrides. (Settings via
  `PUT /settings`.)

## Agent Swarm (#59–#86)

Teams ("swarms", never "companies") of role-specialized agents that work projects
broken into tasks, coordinated by a per-swarm runtime. Reads = `ws viewer`, mutations
+ lifecycle = `ws editor`. JSON snake_case, ULID ids, RFC3339 timestamps,
`Problem{code,message}` errors. Async runtime actions return a record to poll; live
updates also arrive over `/ws/events` (`swarm_*` events). Item routes resolve the
workspace from the row.

| # | Method & path | Auth | Request | Response |
|---|---|---|---|---|
| 59 | GET /api/v1/workspaces/{id}/swarm/swarms | ws viewer | — | `Swarm[]` |
| 60 | POST /api/v1/workspaces/{id}/swarm/swarms | ws editor | CreateSwarmReq | SwarmDetail |
| 61 | GET /api/v1/swarm/swarms/{sid} | ws viewer | — | SwarmDetail |
| 62 | PATCH /api/v1/swarm/swarms/{sid} | ws editor | UpdateSwarmReq | Swarm |
| 63 | DELETE /api/v1/swarm/swarms/{sid} | ws editor | — | 204 |
| 64 | GET /api/v1/swarm/presets | member | — | `SwarmPreset[]` |
| 65 | GET /api/v1/swarm/swarms/{sid}/agents | ws viewer | — | `SwarmAgent[]` |
| 66 | POST /api/v1/swarm/swarms/{sid}/agents | ws editor | CreateAgentReq | SwarmAgent |
| 67 | PATCH /api/v1/swarm/agents/{aid} | ws editor | UpdateAgentReq | SwarmAgent |
| 68 | DELETE /api/v1/swarm/agents/{aid} | ws editor | — | 204 |
| 69 | POST /api/v1/workspaces/{id}/swarm/recruit | ws editor | RecruitReq | RecruitedAgent |
| 70 | GET /api/v1/swarm/swarms/{sid}/projects | ws viewer | — | `SwarmProject[]` |
| 71 | POST /api/v1/swarm/swarms/{sid}/projects | ws editor | CreateProjectReq | SwarmProject |
| 72 | PATCH /api/v1/swarm/projects/{pid} | ws editor | UpdateProjectReq | SwarmProject |
| 73 | DELETE /api/v1/swarm/projects/{pid} | ws editor | — | 204 |
| 74 | POST /api/v1/workspaces/{id}/swarm/projects/{pid}/plan | ws editor | PlanReq | `SwarmTask[]` |
| 74b | POST /api/v1/swarm/projects/{pid}/clear | ws editor | — | `{ok, runs_stopped, tasks_deleted, messages_deleted}` — stops the project's in-flight runs, deletes ALL its tasks + project-scoped feed messages (runs/spend history kept), emits `swarm_project_cleared` |
| 74c | GET /api/v1/swarm/swarms/{sid}/utilization | ws viewer | — | `{parallel_cap, active_runs, ready_tasks, tasks_by_status, agents:[{id,name,title,status,active_run}]}` — board-utilization snapshot (drives the 5-min manager utilization watchdog + the `swarm_utilization` MCP tool) |
| 75 | GET /api/v1/swarm/projects/{pid}/tasks | ws viewer | — | `SwarmTask[]` |
| 76 | POST /api/v1/swarm/projects/{pid}/tasks | ws editor | CreateTaskReq | SwarmTask |
| 77 | PATCH /api/v1/swarm/tasks/{tid} | ws editor | UpdateTaskReq | SwarmTask |
| 78 | DELETE /api/v1/swarm/tasks/{tid} | ws editor | — | 204 |
| 79 | POST /api/v1/swarm/tasks/{tid}/run | ws editor | — | SwarmRun |
| 80 | GET /api/v1/workspaces/{id}/swarm/runs?swarm_id=&project_id=&agent_id=&status= | ws viewer | — | `SwarmRun[]` |
| 81 | GET /api/v1/swarm/runs/{rid} | ws viewer | — | SwarmRun |
| 82 | POST /api/v1/swarm/runs/{rid}/stop | ws editor | — | SwarmRun |
| 83 | GET /api/v1/swarm/swarms/{sid}/graph | ws viewer | — | SwarmGraph |
| 84 | POST /api/v1/workspaces/{id}/swarm/swarms/{sid}/start\|pause\|abort\|resume | ws editor | — | Swarm |
| 85 | GET /api/v1/swarm/swarms/{sid}/board?project_id=&task_id= | ws viewer | — | `SwarmMessage[]` |
| 86 | POST /api/v1/swarm/swarms/{sid}/board | ws editor | PostMessageReq | SwarmMessage |
| — | POST /api/v1/ingest/swarm/board | session token | `{kind?,to_agent_id?,body}` | 204 |
| — | POST /api/v1/ingest/swarm/product | session token | `{title?,body_md}` | 204 |
| — | POST /api/v1/ingest/swarm/mockup | session token | `{title,format,content}` | 204 |
| — | POST /api/v1/ingest/swarm/discovery-report | session token | `{report_md}` | 204 |
| — | POST /api/v1/workspaces/{id}/swarm/swarms/{sid}/agent-stop | ws editor | — | `{ok:true}` |

Notes:
- `config.max_parallel_sessions` is the per-swarm concurrency cap (the Coordinator's
  parallel-worker limit). A blank create uses sensible defaults; create-from-preset
  (`preset_slug`) instantiates the org and maps each agent's provider to an installed
  CLI, falling back to the workspace default.
- **Budget guardrails (D3/D8).** `Swarm` carries four top-level limit columns, all
  nullable = unlimited: `max_total_runs`, `max_cost_usd`, `max_runtime_secs`, and the
  per-task attempt ceiling `max_attempts` (default 3). `CreateSwarmReq`/`UpdateSwarmReq`
  accept all four (on update, `null` clears a limit, an absent key leaves it untouched).
  On every tick the Coordinator checks total runs so far, accumulated `cost_usd`
  (summed from the per-run backfill below), and wall-clock since `run_started_at`; when
  any is exceeded it **auto-pauses** the swarm (status `paused`, a human-facing
  `pause_reason`, idle sessions suspended) instead of spawning more — raise the budget
  and `resume` to continue. `SwarmDetail.counts` surfaces `total_runs` + `cost_usd`
  alongside `running_runs`. `run_started_at` is the wall-clock anchor (set when the
  swarm goes active; cleared on pause/abort, so a resume restarts the clock).
- **Attempt ceiling.** `SwarmTask.attempts` counts the turns the Coordinator has queued
  for a task. A task that keeps returning a non-terminal status (`in_progress`/unknown)
  or whose turn fails is re-queued only until `attempts` reaches the swarm's
  `max_attempts`; after that it is marked `blocked` (with an `escalation` board post +
  notice) rather than re-run forever.
- **Crash recovery.** On daemon start, swarm runs left `queued`/`running`/`waiting` by a
  previous process are marked `error` (their background task died with the process)
  before any coordinator is restored — so they don't permanently consume the parallel
  cap or block an agent. Mirrors the review/skill-eval recovery.
- Lifecycle: `start`/`resume` (re)start the Coordinator and set status `active`;
  `pause` stops new turns + suspends idle swarm sessions (status `paused`); `abort`
  cancels queued/running runs, kills swarm sessions (status `aborted`).
- `POST /ingest/swarm/board` is unauthenticated but **gated by the per-session ingest
  token** (`X-Otto-Session` + `X-Otto-Token`), like `/ingest/claude`; the agent posts
  via the materialized `otto-post` helper. The session's `meta` carries
  `swarm_id`/`agent_id`.
- `POST /ingest/swarm/product` uses the same per-session ingest token and is restricted to
  swarm sessions (the session `meta` must carry `swarm_id`). A PO/feature-design agent
  publishes a feature **draft** (`body_md`, optional `title`) to the Product page via the
  materialized `otto-product` helper; the user/PO reviews it. Fire-and-forget (always 204).
- `POST /ingest/swarm/mockup` and `POST /ingest/swarm/discovery-report` use the same
  per-session ingest token. A discovery/design agent (via the materialized `otto-mockup` /
  `otto-discovery-report` helpers) publishes a generated mockup (`{title,format,content}`,
  `format` ∈ `html`|`mermaid` → stored as a `kind:"mockup"`, `source:"agent"` attachment) or
  the consolidated discovery report (`{report_md}`). The target story/run is derived
  server-side from the session's `meta.project_id` → its discovery run (the agent never
  supplies a story/run id); if no discovery run resolves, nothing is written. Fire-and-forget
  (always 204).
- `POST /workspaces/{id}/swarm/swarms/{sid}/agent-stop` (ws editor) stops a single running
  swarm-agent turn for `{sid}` without pausing the whole swarm; returns `{ok:true}`.
- Assigning a task to a *leader* (an agent with reports) triggers a delegation turn
  that decomposes it into subtasks for the reports.
- `SwarmRun.tokens_input` / `tokens_output` / `cost_usd` are backfilled on the run's
  terminal patch (done/error/stopped) from the embedded usage store (otto-usage),
  keyed on the run's `session_id`. They stay `null` when usage tracking is disabled or
  no usage was recorded for the session yet (e.g. transcript not yet flushed) — never a
  misleading `0`. The Run Inspector surfaces the parsed `result` (summary, `artifacts[]`),
  the run's `cwd`, the board posts tagged with this `run_id`, tokens/cost, and the raw
  result JSON; it is a pure client view (no new endpoint).

## API Tokens (#87–#89)

Long-lived personal access tokens for driving the daemon over HTTP from scripts/CLIs
(skills, CI, automation). They are issued per-user and flow through the same bearer-auth
path as login tokens — use as `Authorization: Bearer <token>` on any route, or as
`?token=<token>` on the WebSocket endpoints. The raw secret is shown exactly once at
creation (only its SHA-256 hash is stored); `kind='api'` tokens have a ~10-year fixed
lifetime whose expiry is never slid (unlike the 30-day sliding login token). A token is
scoped to its owner's roles: a token created by a root user has root; otherwise it has
that user's workspace roles. Bootstrap one with a one-time login, then save it in the
`OTTO_API_TOKEN` env var.

| # | Method & path | Auth | Request | Response |
|---|---|---|---|---|
| 87 | POST /api/v1/auth/tokens | member | CreateApiTokenReq `{label?}` | CreateApiTokenResp `{token, info}` (secret shown once) |
| 88 | GET /api/v1/auth/tokens | member | — | `ApiTokenInfo[]` (never the secret; newest first) |
| 89 | DELETE /api/v1/auth/tokens/{id} | member | — | 204 (404 if not found / not owned) |
| 90 | GET /api/v1/repos/{id}/stashes | ws viewer | — | `StashInfo[]` (read-only `git stash list`) |
| 148 | GET /api/v1/repos/{id}/worktrees | ws viewer | — | `WorktreeInfo[]` (`git worktree list`; first entry = main) |
| 149 | POST /api/v1/repos/{id}/worktrees/remove | ws editor | `{path, force?}` | `WorktreeInfo[]` (refreshed list; 400 on main/unknown path; git refuses dirty/locked without `force`) |
| 150 | POST /api/v1/repos/{id}/worktrees/prune | ws editor | — | `WorktreeInfo[]` (drops stale registrations whose dir is gone) |
| 151 | GET /api/v1/repos/{id}/submodules | ws viewer | — | `SubmoduleInfo[]` (`git submodule status` + `.gitmodules` url/branch) |
| 152 | POST /api/v1/repos/{id}/submodules/update | ws editor | `{path?}` | `SubmoduleInfo[]` (`update --init --recursive`, one module or all) |

Notes:
- `StashInfo` = `{index, ref, sha, parents[], date, message, branch?}` — one entry per
  `git stash list`. `ref` is the `stash@{N}` selector; `parents` are `[base, index, (untracked)]`.
- `WorktreeInfo` = `{path, head, branch?, is_main, locked, lock_reason?, prunable, dirty}` —
  `dirty` is a best-effort uncommitted-changes probe; removal keeps the branch. Remove only
  accepts a path the repo itself lists (never an arbitrary directory) and never the main worktree.
- `SubmoduleInfo` = `{path, sha, state, describe?, url?, branch?}` with `state` one of
  `ok | uninitialized | modified | conflict` (the `git submodule status` prefix char).
- `ApiTokenInfo` = `{id, label?, token_prefix, created_at, last_seen_at, expires_at}`.
  `token_prefix` is the first 12 chars of the raw token (for identifying it in a list);
  the rest is unrecoverable.
- `DELETE` only revokes the caller's own API tokens (scoped by `user_id` + `kind='api'`).
- `last_seen_at` is updated on use, throttled to at most once per hour.

## Share-link tokens (mobile remote-access, Task 1.9)

Scoped, expiring, revocable capability tokens bound to **one session** — the guest-access
primitive for the mobile remote-access feature. The owner mints a share; the raw token is
shown exactly once (only its SHA-256 hash is stored). The `url` field is the ready-to-share
fragment URL (`<origin>/#/s/<session_id>/<token>`).

**Guards (mint + list):** the caller must own the session or be a workspace Admin, must NOT
be impersonated (`real_user != effective_user`), and must NOT hold a scoped share token
(a guest cannot mint sub-shares). Role `"admin"` is rejected; TTL is clamped to `[60, 86400]`.

**Revocation evicts:** after revoking a share, `SessionManager::evict` is called so any
still-attached viewer receives `{"type":"terminated"}` and the WS closes immediately.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| POST /api/v1/sessions/{id}/share | session owner / ws admin | `CreateShareReq {role, ttl_secs?, label?, recipient_email?, duration_secs?}` | `CreateShareResp {token, url, info: ShareInfo}` (token shown once) |
| GET /api/v1/sessions/{id}/shares | session owner / ws admin | — | `ListSharesResp {shares: ShareInfo[]}` (live, non-revoked) |
| DELETE /api/v1/auth/shares/{share_id} | member (self-owned) | — | 204 (revokes + evicts; idempotent) |
| POST /api/v1/auth/shares/revoke-all | member (self-owned) | — | 204 (revokes all caller's shares + evicts) |

`ShareInfo` = `{id, session_id, role, token_prefix, label?, created_at, expires_at}`.
`role` is `"viewer"` (read-only) or `"editor"` (read + input); never `"admin"`.
TTL is FIXED (never slid); `expires_at = created_at + ttl_secs`.

---

## Email sender (Gmail App Password, mobile plan Task 7.1)

The per-user Gmail sender that powers the email-OTP share gate (later tasks email
one-time codes to share-link recipients). Each user configures ONE sender: their
Gmail address + a 16-char **Gmail App Password** (Google Account → Security → App
passwords; requires 2-Step Verification). The app password is stored in the macOS
**Keychain** (`otto-keychain`) under `email-sender-{user_id}` — **never** in the
DB, which holds only the opaque `secret_ref`. Both routes are **self-owned** (any
authed member manages their OWN sender; `Exempt` in the feature policy, like
`/auth/tokens`).

`PUT` stores the secret, upserts the row, then validates the pair via a real
Gmail SMTP login (`smtp.gmail.com:587`, STARTTLS + AUTH) — sending a tiny probe
mail from the address to itself. Only on success is `verified_at` recorded; a bad
app password fails closed (502) and the sender stays unverified. `GET` returns the
configured address + verified flag and **never** the password.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| PUT /api/v1/email-sender | member (self-owned) | `SetEmailSenderReq {gmail_address, app_password}` | `EmailSenderResp {gmail_address, verified}` (502 on SMTP verify failure → not verified) |
| GET /api/v1/email-sender | member (self-owned) | — | `EmailSenderResp {gmail_address?, verified}` (never the password) |

`EmailSenderResp` = `{gmail_address?, verified}` — `gmail_address` is omitted on
`GET` when no sender is configured; `verified` is `true` once a real SMTP login
with the app password succeeded.

---

## Email-OTP gate for share links (mobile plan Tasks 7.2/7.3)

A share link's recipient must enter a one-time code (emailed out-of-band) before
the scoped token reaches **anything** — so a leaked/forwarded link alone is
useless. Layered on top of the scoped-token guard, role cap, and short TTL above.

**Creating an OTP share.** `POST /api/v1/sessions/{id}/share` with a
`recipient_email` mints an OTP-gated share: the owner picks the recipient address
(LOCKED for the share's life) and a `duration_secs` session window
(server-clamped to ≤ 43200s = 12h). Otto generates a **6-digit OTP** (`OsRng`),
stores only its `sha256` (`otp_hash`, ~10-min expiry) plus `recipient_email` and
`max_expires_at`, and **emails the code** to the recipient via the owner's
verified email sender (above). Requires a verified sender — else `400`
("set up a verified email sender first"). Omitting `recipient_email` mints a
plain scoped share with no OTP gate (backward compatible). `duration_secs`
governs the OTP-share window; `ttl_secs` governs a plain share.

**Redeeming (guest).** While a share is OTP-pending the scope reaches NOTHING
except `/share/verify`: the feature guard `403`s every protected route (even
`GET` the session) and `/ws/term` refuses the upgrade (`403`).

| Method & path | Auth | Request | Response |
|---|---|---|---|
| POST /api/v1/share/verify | **public** (the share token is the auth) | `VerifyShareReq {token, otp}` | `VerifyShareResp {verified: true}` on success |
| POST /api/v1/share/extend | **public** (the share token is the auth) | `ExtendShareReq {token}` | `{ "ok": true }` on success |

`POST /api/v1/share/verify` is **Exempt** (public) — the share `token` in the body
is the auth. It is **IP rate-limited** (the share throttle; `429` with
`Retry-After` when locked), checks `otp_hash == sha256(otp)` AND `otp_expires_at >
now`, and on success sets `verified_at` and **clears `otp_hash`** (single-use — a
fresh code requires a resend). A wrong / expired / reused code records a throttle
failure and returns `401`. After verification the guest may attach (`/ws/term`)
and `GET` the session until `max_expires_at` (≤12h); once the window elapses the
share re-pends and must be re-verified (Task 7.4 extension re-emails the LOCKED
original recipient only).

`POST /api/v1/share/extend` is **Exempt** (public) — re-issues a **FRESH OTP** for
an existing OTP share and re-emails it to the **LOCKED original `recipient_email`
ONLY**. The request body carries **no email field by design**: the destination is
read from the share row, never from the request, so access can never be redirected
to a different mailbox. It is **IP rate-limited** (the share throttle), generates a
new 6-digit OTP (`OsRng`), stores only its `sha256` (`otp_hash`, ~10-min expiry),
**clears `verified_at`** (re-pending the share so the guest must re-verify), and
opens a fresh **≤12h** window (`max_expires_at`, the bearer-token `expires_at`
tracks it). Only `kind='share'` rows **with** a `recipient_email` are extendable —
a plain (non-OTP) / missing / revoked share returns `400`. The code is emailed via
the **share owner's** verified email sender; if the owner no longer has a verified
sender → `400`. The guest then re-verifies the new code via
`POST /api/v1/share/verify` to re-open the window.

---

# Otto API Contract — extended surface (v1, mounted)

The tables above (#1–#89) are the original frozen core. The sections below complete the
contract by documenting every other route the daemon actually registers (mounted via the
module routers in `crates/otto-server/src/modules.rs::module_routers`). They follow the same
conventions: all live under `/api/v1` with bearer auth (`Authorization: Bearer <token>` or
`?token=` on WS), JSON snake_case, ULID ids, RFC3339 timestamps, `Problem{code,message}`
errors. Role column meaning is identical (`member`, `ws viewer/editor/admin`, `root`).
Item routes (those keyed by a row id, e.g. `/sessions/{id}`) resolve the owning workspace
from the row and role-check against it. This surface is a completion of the frozen contract,
not a redesign — no path here may change shape without a contract bump.

Mounting summary (all paths below are under `/api/v1` unless the section says "root-level"):
the `/api/v1` nest carries the bearer-auth middleware; root-level WS/proxy routers
self-authenticate via `?token=` and are merged at the server root by `build_router`.

## Activity trail & task tracker (live agent telemetry)

A session's append-only activity trail plus its current task list. The provider's hooks
write these via the per-session ingest token (see Ingest below); humans read them with a
bearer token. `TrailAppended` / `TasksUpdated` events mirror writes over `/ws/events`.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /workspaces/{wid}/sessions/{sid}/trail | ws viewer | — | `TrailEvent[]` (session activity trail) |
| POST /workspaces/{wid}/sessions/{sid}/trail | ws editor | TrailEvent | 204 (append one trail entry) |
| GET /workspaces/{wid}/sessions/{sid}/tasks | ws viewer | — | `AgentTask[]` (current task list) |
| PUT /workspaces/{wid}/sessions/{sid}/tasks | ws editor | `AgentTask[]` | 204 (replace the task list) |
| GET /workspaces/{wid}/activity/summary | ws viewer | — | per-session activity summary for the workspace |

## Sessions (extras beyond #17–#22)

| Method & path | Auth | Request | Response |
|---|---|---|---|
| POST /sessions/{id}/archive | ws editor | — | 204 (archive a channel/agent session) |
| POST /sessions/{id}/unarchive | ws editor | — | 204 (restore an archived session) |
| POST /sessions/{id}/input | ws editor | `SendInputReq{text, submit?}` — writes a keystroke/paste into the PTY (`submit` omitted/true appends a newline) | 200 |
| POST /sessions/{id}/handover | ws editor | — | starts a handover; progress via `SessionMetaUpdated` |
| POST /sessions/{id}/handover/brief | ws editor | — | generates a handover brief for the session |
| POST /sessions/{session_id}/attach-product | ws editor | `{story_id}` | attaches a product story to the session |
| POST /app/kill-sessions | member | — | terminate every live PTY (desktop quit hook) |

## Connection sections (sidebar grouping)

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /workspaces/{id}/connection-sections | ws viewer | — | `ConnectionSection[]` |
| POST /workspaces/{id}/connection-sections | ws editor | CreateSectionReq | ConnectionSection |
| POST /workspaces/{id}/connection-sections/reorder | ws editor | `{order:[id,…]}` | 204 |
| PATCH /connection-sections/{id} | ws editor | RenameSectionReq | ConnectionSection |
| DELETE /connection-sections/{id} | ws editor | — | 204 |
| POST /connection-sections/{id}/move | ws editor | MoveSectionReq | 204 |

## Import connections from other DB tools (`/connections/import/*`)

The daemon runs locally, so it reads each supported tool's config file from its
default macOS location — the user picks a *tool*, never a file. Editor-gated
(the path workspace authorizes; created connections are global, like the normal
create path). Created connections always use `secret: null` because every tool
keeps passwords encrypted or in an OS keychain — unrecoverable here. For
MongoDB, when a username is known the generated `conn_string` carries Otto's
`{secret}` placeholder so the password substitutes in once the user supplies it
via the connection editor.

Endpoints: see rows 30a–30c in the main table.

- `ImportSource` (string enum): `"mysql_workbench" | "dbeaver" | "datagrip" | "nosqlbooster"`.
- `SourceStatus` = `{source: ImportSource, label, present: bool, path?: string, count?: number}` —
  `present`/`count` reflect a stat + cheap parse of the default config path.
- `ParsedConnection` = `{source, name, kind?: ConnectionKind, params, supported: bool,
  needs_password: bool, note?: string}`. For a supported engine, `params` is the ready-to-create
  Otto shape (mysql/clickhouse `{host,port,user,db}`, redis `{host,port,db?}`, mongodb
  `{conn_string}`; plus nested `ssh{host,port,user,identity_file}` when the source had an SSH
  tunnel, and `tls{mode,verify,ca_cert?}` when SSL was enabled — `mode` is a valid `TlsMode`
  (`preferred`/`required`) and `verify` is emitted **explicitly**, defaulting to `false` so an
  import doesn't force certificate verification on a self-signed/staging server. Workbench's
  `useSSL` level maps precisely: 1→`preferred`/no-verify, 2→`required`/no-verify, 3·4→`required`
  +verify). For an unsupported engine
  `kind=null, supported=false`, `params={}`, and `note` explains the skip (e.g. "PostgreSQL is not
  supported by Otto") — still listed so the user sees why it wasn't importable.
- `ImportScanResult` = `{source, path?: string, connections: ParsedConnection[], warnings: string[]}`.
- `ImportCreateReq` = `{connections: ImportCreateItem[], section_id?: id}` where
  `ImportCreateItem` = `{name, kind: ConnectionKind, params, environment?, read_only?}`.
- `ImportCreateResult` = `{created: Connection[], failed: {name, error}[]}` — best-effort; one
  failure never aborts the batch.

Default macOS config paths probed (all under `~/Library`):
- MySQL Workbench: `Application Support/MySQL/Workbench/connections.xml` (always MySQL).
- DBeaver: `DBeaverData/workspace*/<project>/.dbeaver/data-sources.json` (all workspaces merged,
  deduped by name+params).
- DataGrip: IDE-global `Application Support/JetBrains/DataGrip*/options/dataSources.xml` (+ the
  sibling `dataSources.local.xml` for username/SSL, joined by data-source `uuid`), plus a bounded
  `$HOME` walk (depth ≤4, heavy/system dirs skipped, ≤50 files) for project-level
  `**/.idea/dataSources.xml`.
- NoSQLBooster: `Application Support/NoSQLBooster for MongoDB/app.json` (always MongoDB).

## Workspace MCP servers (user-managed `.mcp.json` entries)

User-configured MCP (Model Context Protocol) servers, per workspace. *Enabled* servers are
**reconciled** into the workspace's `.mcp.json` — alongside Otto's own managed entries (the
browser server and the first-party `otto` tools server) — every time an agent session spawns
or restarts there (see `otto-sessions::mcp`). A top-level `ottoManagedServers` marker in the
file lists the names Otto wrote; each reconcile removes marker names no longer enabled (so a
disable/delete propagates), upserts the current set, and never touches entries outside the
marker (hand-added servers survive). Codex sessions receive the same enabled servers as
per-spawn `-c mcp_servers.<name>.*` overrides (Codex doesn't read `.mcp.json`); grok sessions
get `[mcp_servers.<name>]` tables in `<workspace>/.grok/config.toml`, tracked by an
`otto_managed_servers` marker. The resolved managed name list is snapshotted into the
session's meta as `mcp_servers` at each spawn. Nothing is auto-enabled: `enabled` defaults
`false` on create, and a server is only written once the user flips it on and a session then
spawns in the workspace. Reads = `ws viewer`, mutations = `ws editor`. Item routes resolve
the workspace from the row.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /workspaces/{id}/mcp-servers | ws viewer | — | `McpServer[]` |
| POST /workspaces/{id}/mcp-servers | ws editor | CreateMcpServerReq | McpServer |
| PATCH /mcp-servers/{id} | ws editor | UpdateMcpServerReq (partial; absent fields kept) | McpServer |
| DELETE /mcp-servers/{id} | ws editor | — | 204 |

Notes:
- `McpServer` = `{id, workspace_id, name, command, args:[string], env:{string:string},
  secret_env_keys:[string], secret_ref?, enabled, created_by, created_at, updated_at}`. `name`
  is the key under `.mcp.json`'s `mcpServers` map and is unique within the workspace.
- `CreateMcpServerReq{name, command, args?, env?, secret_env?, enabled?}` — `enabled` defaults
  `false` (never auto-enabled). Empty `name`/`command` → 400 `invalid`.
- `env` holds NON-secret pairs only. `secret_env` (write-only, `{key: value}`) values are
  persisted to the macOS Keychain (blob ref `mcp-{id}`, shared with the MCP Control Plane —
  the two surfaces share `mcp_servers` rows) and never to the DB; responses carry only the key
  names in `secret_env_keys`. On PATCH, `secret_env` present = full replacement of the secret
  set; an EMPTY value keeps the currently stored value (`KEY=` sentinel); absent = unchanged.
  A key listed in both `env` and `secret_env` is stored as secret only.
- Secret values are resolved exclusively when `.mcp.json` is rendered at agent spawn — the
  rendered file on disk contains real values (the agent CLI needs them; it is user-local and
  out-of-tree), but Otto's DB does not. The reconcile preserves all `.mcp.json` keys outside
  the `ottoManagedServers` marker, and a user server named `otto` / `otto-browser` (reserved
  for Otto's own entries) is skipped.

## SFTP file browser (`/connections/{id}/sftp/*`)

File browse / read / transfer over an **SSH** connection's existing auth. Otto
drives the system `sftp` binary (one `ControlMaster`/`ControlPersist` socket per
op-session), reusing the connection's keys/ssh-agent/`~/.ssh/config` and
`ProxyJump` exactly as the terminal `open` does — there is no separate password.
Because the daemon runs on the user's machine, `download`/`upload` read/write the
**daemon host's** real local disk. All routes require `kind == ssh` (else 400).
Browse/read = `ws viewer` (`Connections:View`); transfers/mutations = `ws editor`
(`Connections:Edit`). A leading `~` in a local path expands to the daemon user's
home; for downloads the parent dir is created and, if the local path is an
existing directory, the remote file's basename is used.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /connections/{id}/sftp/list?path= | ws viewer | — | SftpListResp `{path, entries: SftpEntry[]}` — empty/absent `path` ⇒ remote `pwd` then list |
| GET /connections/{id}/sftp/read?path= | ws viewer | — | SftpReadResp `{text, truncated}` — downloads to a temp file, returns up to 1 MiB of UTF-8 text |
| POST /connections/{id}/sftp/download | ws editor | SftpDownloadReq `{remote_path, local_path}` | SftpDownloadResp `{local_path, bytes}` |
| POST /connections/{id}/sftp/upload | ws editor | SftpUploadReq `{local_path, remote_path}` | 200 |
| POST /connections/{id}/sftp/mkdir | ws editor | SftpMkdirReq `{path}` | 200 |
| POST /connections/{id}/sftp/remove | ws editor | SftpRemoveReq `{path, dir?}` | 200 — `dir:true` ⇒ `rmdir`, else `rm` |
| POST /connections/{id}/sftp/rename | ws editor | SftpRenameReq `{from, to}` | 200 |

`SftpEntry { name, kind: "dir"|"file"|"symlink"|"other", size, mtime?, perms,
symlink_target? }`. Errors surface the `sftp` client's stderr (e.g. permission
denied, no such file) as a `502 upstream`.

## DB Explorer — engine access (`/connections/{id}/db/*`)

Native data-access for a connection profile (reuses its keychain secret). Reads use the
profile's `ws viewer`; queries that hit the live DB use `ws editor`.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| POST /connections/unsaved/db/test | ws editor (on `workspace_id`) | `{workspace_id, kind, params, secret?}` | connectivity probe of an UNSAVED config (form "Test" button) — nothing is persisted; DB kinds only; SSH tunnels open ephemerally |
| POST /connections/{id}/db/test | ws editor | — | connectivity probe result |
| GET /connections/{id}/db/capabilities | ws viewer | — | engine capability flags: `sql`, `joins`, `transactions`, `multi_statement`, `cancel`, `explain`, `default_port`, `schema_levels`, `query_language` (see the honesty notes below) |
| GET /connections/{id}/db/schema | ws viewer | — | top-level schema tree (roots) |
| POST /connections/{id}/db/search-objects | ws viewer | `{q, schema?, scope?:"schema"\|"all", kinds?, limit?}` | `ObjectSearchResult{hits[{schema,name,kind,path}], truncated, scanned, supported}` — find tables/views/collections by NAME without expanding the tree (the tree's own filter is client-side over loaded nodes only). `scope="all"` is ONE catalog query on MySQL/Postgres/ClickHouse but one `listCollections` round trip PER database on MongoDB, so `scanned` reports the real cost; Postgres cannot cross databases, so "all" means every schema in the CONNECTED database. Redis has no object namespace → `supported:false`. `limit` default 200, capped 1000; a blank `q` returns no hits. `kinds` reserved for later column search. |
| POST /connections/{id}/db/schema/children | ws viewer | `{node, filter?, counts?}` | child schema nodes (lazy expand). `counts:true` fills each node's `detail` with an engine-native ROW ESTIMATE (`information_schema.table_rows` / `reltuples` / `system.parts` / `estimatedDocumentCount` — never `COUNT(*)`); opt-in because collecting it is the slow part of expanding a database on a large server. MySQL databases expose `Tables`/`Views` folders always, plus `Procedures`/`Functions`/`Triggers` folders (with a count) when the database has any; routine leaves carry `kind:"procedure"`/`"function"`, trigger leaves `kind:"trigger"` |
| POST /connections/{id}/db/object | ws viewer | `{ref}` | object detail (columns/DDL/etc.). For a procedure/function: `columns` are its parameters and `ddl` is the `SHOW CREATE` body. For a trigger (MySQL): `ddl` is `SHOW CREATE TRIGGER` and `extra` = `{timing, event, table}`. `indexes[].definition` carries the full engine-native index definition when available (Mongo: raw `listIndexes` doc incl. `partialFilterExpression`; Postgres: `pg_get_indexdef` string). For a Mongo **collection** `columns` is empty and the field list lives in `extra`: `sampled_fields` = `{topLevelField: bsonType}`, `sampled_paths` = dotted paths incl. embedded ones (`meta.brand_id`, depth ≤ 3, capped at 400), and `sampled_path_types` = `{path: bsonType}` for those same paths; plus `sample` (one whole document) and `stats` |
| POST /connections/{id}/db/schema-graph | ws viewer | `{schema, max_tables?}` | DbSchemaGraph — read-only ERD: tables (+PK/FK-flagged columns) and FK edges, walked from the schema tree; `max_tables` default 60, clamped 1..200; engines without FK metadata (Redis/Mongo) return `relationships:false` |
| POST /connections/{id}/db/query | ws editor | RunQueryReq | query result rows / affected count |
| POST /connections/{id}/db/query-plan | ws **viewer** | `{statement, node?}` | `DbQueryPlan` — a normalized query plan from the engine's native EXPLAIN (MySQL `EXPLAIN FORMAT=JSON`, Postgres `EXPLAIN (FORMAT JSON)`, ClickHouse `EXPLAIN json=1` w/ plain-text fallback, Mongo `explain` queryPlanner). The statement is **EXPLAIN-wrapped, never executed raw** — read-only by construction, hence `viewer`. Redis → 400 (no plan surface). |
| POST /connections/{id}/db/cancel | ws editor | `{query_id}` | 204 — cancel an in-flight query engine-side |
| POST /connections/{id}/db/query-status | ws editor | `{query_id}` | `QueryStatus` — re-attach probe for a run whose HTTP wait was lost (queries with a `query_id` execute detached from their request): `{status:"running"}` while it executes, `{status:"done", result?/error?}` while the parked outcome is retained (TTL 10m, capped), `{status:"unknown"}` otherwise. Scoped to the connection — never serves another connection's outcome. |
| POST /connections/{id}/db/completion | ws viewer | `{prefix, suffix?, database?, node?}` | Context-aware completion items (`{items:[DbCompletionItem]}`). The daemon parses `prefix` (text before the cursor) + `suffix` (text after, to resolve a `FROM` that follows the cursor) to decide intent — tables after `FROM`/`JOIN`, columns after `WHERE`/`AND`/`alias.`, Mongo collections/methods/field-keys (incl. embedded `x.a`). Each item carries a `score` (→ CodeMirror `boost`) so **index columns/fields rank first**, then the rest of the schema. Backed by a per-connection schema snapshot **cached until refresh** (see below; ~5-min TTL safety net). |
| POST /connections/{id}/db/completion/refresh | ws viewer | `{}` | 204 — drop the connection's cached completion snapshot so the next completion re-introspects. Wired to the UI "Refresh schema" action. No-op for engines without a snapshot cache (Redis). |
| GET /connections/{id}/db/history | ws viewer | — | recent query history |
| GET /db/mongosh | member (Database:View) | — | `MongoshInfo {available, version?}` — whether the `mongosh` CLI (used to run pasted mongosh **scripts** and by Mongo terminal sessions) is on the daemon's PATH; probed via `mongosh --version`, cached ~60s. The query editor calls this when it detects a script so a missing binary is an inline install hint before the run |
| POST /connections/{id}/db/explain-with-agent | ws editor | `{sql}` | AI explanation of a query (spawns an agent) |
| POST /connections/{id}/db/export | ws editor | `{statement, format?, node?}` | **Uncapped, streamed** CSV/JSON browser download (`Content-Disposition: attachment`). Bytes are produced by the driver's streaming exporter and piped straight to the response body — no row cap and no full-result buffering (fixes the prior silent truncation at the driver default). `format`: `csv` (header + rows) or `json` (array of objects). A write/DDL on a guarded connection is rejected up front. If the driver errors mid-stream the response body terminates early (truncated download + connection reset) rather than reporting success. |
| POST /connections/{id}/db/export-to-path | ws editor | ExportToPathReq | Stream an uncapped result to a **local file** on the daemon host, selectable format. Response is a **streamed `application/x-ndjson`** progress feed (see below). |
| POST /connections/{id}/db/import | ws editor | ImportReq | Import a **local file** (CSV/TSV/NDJSON/JSON) into an existing table/collection, **guarded** (a Prod/read-only connection refuses it without `confirm_write`). SQL engines (MySQL/ClickHouse/**PostgreSQL**) → batched `INSERT`s via the `run` path (identifier quoting is engine-aware — Postgres double-quotes, MySQL/CH backtick); **MongoDB** → `insertMany` batches (`table` = collection; CSV/TSV cells coerced to numbers/bools/null; NDJSON/JSON keep their types). Response is a **streamed `application/x-ndjson`** line: `{ done, rows, batches }` or `{ error }` (text starting `write_blocked:` ⇒ typed confirmation needed). Redis is unsupported (no bulk-load). |
| POST /connections/{id}/db/nl-to-sql | ws editor | NlToSqlReq | Draft a **read** query from natural language, **validated with `EXPLAIN`** against the live schema before returning. Plain JSON → `NlToSqlOutcome`. Never emits a write/DDL. 400 starting "NL-to-SQL is not configured" ⇒ no drafter wired; 400 starting "could not produce a valid read query" ⇒ retry loop exhausted (message carries the last engine error). Unavailable for Redis. |

`ExportToPathReq` = `{ statement, node?, format?, local_path, max_rows? }`. `format`
is one of `csv` (no header), `csv_with_names` (header row), `tsv`, `tsv_with_names`,
`json` (a JSON array of row objects), `ndjson` (one JSON object per line); default
`csv`. `local_path` is a path on the daemon host (leading `~` expands to the daemon
user's home); if it is an existing directory the file is written as
`<dir>/export.<ext>` (ext per format: `csv`/`tsv`/`json`/`ndjson`), else it is the
full file path and its parent directory is created. `max_rows` (optional, blank =
all rows) caps the export, stopping the stream early. The **response is a streamed
`application/x-ndjson` body**: zero or more progress lines `{ bytes }` (bytes
written to the destination file so far, emitted ~every 300ms) followed by exactly
one terminal line — either `{ done: true, local_path, rows, bytes, duration_ms }`
(the absolute file written, rows & bytes written, wall-clock ms) or
`{ error }` (the export failed mid-stream; HTTP status is already 200 by then).
Streaming keeps the connection alive so a large export never idles out the
browser fetch, and lets the client show a live progress bar. The export **streams**
row/chunk-by-chunk from the driver
straight to a buffered file writer so daemon memory stays bounded regardless of
result size — MySQL via the sqlx row cursor, MongoDB by iterating the `Cursor`,
ClickHouse (HTTP) by requesting an explicit `FORMAT` and splicing the response
body (so a tunnelled ClickHouse writes the user's local path, **not** a
server-side `INTO OUTFILE` on the tunnel host). Only row-returning statements are
exportable; a write/DDL is rejected (and a write on a guarded production/read-only
connection is blocked as elsewhere). Gated at the same role as `query` (`ws
editor`; global connections: `Database:Edit`).

`ImportReq` = `{ local_path, format, table, batch_size?, confirm_write? }`.
`format` is one of `csv`/`tsv` (first row = header) or `ndjson`/`json` (objects;
columns are the union of keys, missing keys → `null`). `local_path` is a file on
the daemon host (leading `~` expands to the daemon home). `table` is the target table
(SQL — must already exist) or collection (MongoDB — created on first insert).
`batch_size` is rows per batch (default 500, clamped 1..=5000). SQL engines
(MySQL/ClickHouse/PostgreSQL) build batched `INSERT … VALUES (…),(…)` with
engine-aware identifier quoting (backticks for MySQL/ClickHouse, double quotes
for PostgreSQL) and single-quote-escaped literals, and run each batch **through
the guarded `run` path** — so masking/history apply and a Prod/read-only
connection refuses it unless `confirm_write` is set. MongoDB imports via
`insertMany` batches (CSV/TSV cells type-coerced: numbers/bools/null;
NDJSON/JSON keep their types), guarded the same way. Redis is not supported. The
**response is a streamed `application/x-ndjson` body** with a single terminal
line: `{ done: true, rows, batches }` (rows inserted, batches run) or `{ error }`
— a guarded connection without `confirm_write` yields `{ error }` whose text
starts `write_blocked:` (the client re-sends with `confirm_write: true` after a
typed confirmation). Gated `ws editor` (global connections: `Database:Edit`).

`NlToSqlReq` = `{ question, node?, max_attempts? }`. `max_attempts` is the
draft→validate retry budget (default 3, clamped 1..=4). The server asks the
configured drafter (the agent/LLM, grounded in a compact schema summary) for a
candidate query, **rejects any write/DDL before it touches the engine**,
validates the candidate with `EXPLAIN` (a read — guard-safe even on a
Prod/read-only connection), and feeds any engine error back to the drafter for a
bounded retry. On success it returns `NlToSqlOutcome` =
`{ sql, plan, attempts, warnings[] }` — an `EXPLAIN`-validated **read** query,
its plan text, the attempt count, and any non-fatal notes. Gated `ws editor`
(global connections: `Database:Edit`) because validation runs `EXPLAIN` live; unavailable
for Redis (no plan surface).

`RunQueryReq` may include an optional client-generated `query_id` (string). When
present, the server registers the in-flight query under it; `POST …/db/cancel`
with the same `query_id` then issues **engine-native** cancellation on a
*separate* connection — MySQL `KILL QUERY <connid>`, ClickHouse `KILL QUERY WHERE
query_id = '<id>'` — so the database stops the heavy query and frees the cached
connection, not just the client's HTTP wait. Cancel is gated at the same role as
`query` (`ws editor`; global connections: `Database:Edit`). Cancelling an unknown /
already-finished query, a query on a different connection, or one on an engine
without a native per-query cancel (Redis/MongoDB) is a no-op success (`204`).

`RunQueryReq` also accepts `offset?` (u64, `#[serde(default)]` — back-compat).
It paginates an **auto-limited single SELECT** (Mongo: an unconstrained `find`):
the SQL drivers append `LIMIT n OFFSET m`, Mongo maps it to `skip`. It is applied
**only** when the server auto-injected the LIMIT — an explicit user `LIMIT`/`OFFSET`,
a non-paginatable statement, or a multi-statement batch never gets an `offset`,
so the client's pager and the server's paging can't disagree.

**Multi-statement batches (`SELECT 1; SELECT 2`).** For MySQL/ClickHouse/MongoDB a
`;`-separated script now runs **each statement in order** (a string/comment/quote-
aware splitter decides the boundaries — a `;` inside a literal or comment is not
one). `QueryResult` gained (all back-compat, omitted when empty/default):
`more_results: QueryResult[]` — the later statements' results, in order; the
**top-level fields describe the FIRST statement**. `statement?` — a ≤80-char
single-line preview label, set on each entry of a batch (not for a single
statement). `errored?: bool` — set on the terminal entry when a statement fails:
execution **stops there**, and the response is a `200` carrying the completed
results plus that one errored entry (its `message` holds the engine error). A
**single**-statement failure is still an HTTP error, unchanged. Cancel kills the
statement currently running, surfacing as that entry's error → the same partial
path. A single statement (the common case) is unaffected — no injection changes,
no new fields on the wire. **MongoDB behavior change:** `run_many` previously
returned only the *last* statement's result; it now returns the first on top with
the rest in `more_results` (so the UI's result-set switcher sees them all).

`auto_limited?: u64` on `QueryResult` — the effective `LIMIT` the server injected
for an auto-paginated single SELECT/`find`. Present ⇔ the result was
server-paginated (so the UI shows its pager exactly then, without re-deriving the
injector's bail rules); absent for explicit user LIMIT/OFFSET, non-paginatable
statements, and every batch entry.

**Honest capability flags** (`GET …/db/capabilities`). `transactions` is now
`false` for MySQL and MongoDB (was `true` with no implementation): the explorer
acquires each `run` from a connection pool, so there is no pinned session to hold
a `BEGIN…COMMIT` open on. `multi_statement` is now `true` for MongoDB (it already
ran `;`-separated scripts). Two new flags: `cancel` (server-side per-query cancel
— `true` for MySQL/ClickHouse, `false` for MongoDB/Redis; the UI labels Stop as
client-side-only when false) and `explain` (`true` everywhere except Redis, which
has no plan surface — the UI hides the Explain button there).

**`DbQueryPlan`** (`POST …/db/query-plan`) = `{ engine, root: PlanNode, raw }` where
`PlanNode` = `{ op, object?, detail?, est_rows?, warnings[], children[] }`. `raw` is
the engine's untouched EXPLAIN JSON (for a "raw" toggle). `warnings` flags the
costly access patterns the UI badges red: full scans (`access_type: ALL` on MySQL,
`Seq Scan` on Postgres, `COLLSCAN` on Mongo) and MySQL `Using filesort` / `Using
temporary`. ClickHouse `ReadFromMergeTree` is deliberately **not** flagged (normal
for MergeTree tables). Triggers browse: MySQL databases with triggers expose a
`Triggers` tree folder (`information_schema.TRIGGERS`); a trigger's object detail
carries `SHOW CREATE TRIGGER` DDL + `extra = {timing, event, table}`.

**PostgreSQL engine** (`ConnectionKind`/`DbEngine` `"postgres"`, default port 5432).
First-class parity with MySQL: schema browse, query, streaming export/import, ERD,
JOIN builder, index-first completion. Capabilities: `{ sql:true, joins:true,
transactions:false, multi_statement:true, cancel:true, explain:true,
default_port:5432, query_language:"sql", schema_levels:["Schema","Table","Column"] }`.
Cancel is `pg_cancel_backend(pid)` on a separate connection; `explain` uses
`EXPLAIN (FORMAT JSON)`. A Postgres connection is scoped to one database and
browsed **by schema** (`pg_*`/`information_schema` hidden, `public` first), so the
tree's top level is the database's schemas and the active-database selector maps to
`SET search_path` — the `db:<schema>` node segment carries the schema name, keeping
every downstream node path identical in shape to MySQL's. Import maps
`jdbc:postgresql://` (DataGrip/DBeaver) to this engine.

## DB Assistant — file-backed agent (`/connections/{id}/db/assist`, `/db-assist/{aid}/query`)

A managed, resumable, **file-backed** database agent that replaces the old
"Ask in English" / "Ask AI" drafter (which ran `claude` in an untrusted temp dir →
hung → 502, and seeded an empty schema). Each assist runs the chosen agent as a
real Otto **session** (resumable; hidden from the Agents list via
`meta.source = "db_assist"`) in an Otto-owned **trusted** directory seeded with the
COMPLETE schema (`SCHEMA.md`), the question + working rules (`CONTEXT.md`), an
optional `RESULT.md` (investigate mode), and an executable `q` tool. The agent
cannot reach any DB directly: it runs `./q '<read-only SQL>'`, which POSTs to the
loopback query route below; Otto executes it READ-ONLY and prints the rows. The
agent writes its FINAL query to `ANSWER.sql` and a one-line note to `NOTE.txt`.

Live signals: `db_assist_session_started` (turn start → attach the live terminal)
and `db_assist_updated` (each `ANSWER.sql` change → proposed `sql` + `note`); both
are workspace-scoped WS events (see `ws.md`).

| Method & path | Auth | Request | Response |
|---|---|---|---|
| POST /connections/{id}/db/assist | ws editor (Connections:Edit) | `{ question, mode?("nl"\|"ask"\|"investigate"), node?, provider?, result_context?, assist_id?, workspace_id? }` | `{ assist_id, session_id, sql, note }` — runs ONE agent turn. First call mints the assist (dir + key + session); pass the returned `assist_id` to RESUME the conversation. `provider` is sticky after the first turn. `workspace_id` only needed for global connections. |
| POST /connections/{id}/db/assist/{aid}/summary | ws editor (Connections:Edit) | — | `{ markdown }` — resumes the session, asks it to write `SUMMARY.md`, returns it (the UI downloads it). |
| DELETE /connections/{id}/db/assist/{aid} | ws editor (Connections:Edit) | — | `{ ok: true }` — kills the session, removes the working dir, drops the registry entry (close = discard). |
| POST /db-assist/{aid}/query | **assist-key** (`x-assist-key` header; NOT a user bearer — public route, like `/ingest/*`) | `{ sql }` | `{ columns[], rows[][], error? }` — the `q` tool's backend. Runs the SQL READ-ONLY against the assist's connection (writes/DDL refused → `error`; rows capped at 200). A rejected statement or engine error is returned in `error` (not an HTTP error) so the agent can correct course. |

The per-assist record (dir, key, session id, connection, workspace, provider, node)
lives in an in-memory registry on the daemon — ephemeral by design (discarded on
close or restart). `mode`: `nl` produces a runnable query; `ask` answers a free-form
question; `investigate` is additionally seeded with the current statement + a sample
of its result (`result_context` → `RESULT.md`). The relevant per-engine DB skill
(`db-mysql` / `db-redis` / `db-mongodb` / `db-clickhouse`) is injected into the
prompt when installed (no-op otherwise).

## DB Explorer — saved queries, dashboards, widgets

Saved queries/dashboards/widgets are workspace-scoped (list/create under
`/workspaces/{wid}/db/*`); item mutations are keyed by row id.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /workspaces/{wid}/db/saved-queries | ws viewer | — | `SavedQuery[]` |
| POST /workspaces/{wid}/db/saved-queries | ws editor | CreateSavedQueryReq | SavedQuery |
| PATCH /db/saved-queries/{qid} | ws editor **+ owner/ws-Admin/root** | UpdateSavedQueryReq `{name?, statement?}` | updated `SavedQuery` |
| DELETE /db/saved-queries/{qid} | ws editor **+ owner/ws-Admin/root** | — | 204 |
| GET /workspaces/{wid}/db/dashboards | ws viewer | — | `Dashboard[]` |
| POST /workspaces/{wid}/db/dashboards | ws editor | CreateDashboardReq | Dashboard |
| GET /db/dashboards/{id} | ws viewer | — | Dashboard |
| PATCH /db/dashboards/{id} | ws editor | UpdateDashboardReq | Dashboard |
| DELETE /db/dashboards/{id} | ws editor | — | 204 |
| GET /workspaces/{wid}/db/widgets | ws viewer | — | `Widget[]` |
| POST /workspaces/{wid}/db/widgets | ws editor | CreateWidgetReq | Widget |
| PATCH /db/widgets/{id} | ws editor | UpdateWidgetReq | Widget |
| DELETE /db/widgets/{id} | ws editor | — | 204 |
| POST /db/widgets/{id}/run | ws editor | — | widget query result |

`UpdateSavedQueryReq` = `{ name?, statement? }` — a partial update; an absent
field is left unchanged (so a rename and a statement-edit can be sent
independently). Like DELETE, PATCH requires **Editor on the query's workspace AND
ownership** (the owner, a workspace Admin, or root) — saved queries are
owner-private. An unknown `qid` → 404. The UI uses PATCH to rename a saved query
inline and to update it in place when "Save" is pressed on a tab opened from it
("Save as new" instead POSTs a fresh one).

## Git — repos & PR extras (beyond #34–#56)

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /git/accounts/{id}/remote-repos | member (owner) | — | remote repos visible to the git account |
| POST /git/accounts/{id}/test | member (owner) | — | GitAccountTestResp `{ok, login?, scopes: string[], error?}` — exercises the **stored** token with the provider's cheapest authenticated call (`GET /user`); scopes echoed from `X-OAuth-Scopes` where exposed. Auth failures are `200 {ok:false, error}` (inline rendering), the token never leaves the daemon |
| POST /git/accounts/test | member | TestGitAccountReq `{provider, username, token, api_base_url?}` | GitAccountTestResp — same probe for a **not-yet-saved** form; the draft token travels in the body exactly once and is never persisted or logged |
| GET /repos/{id}/collaborators?q= | ws viewer (+ bound-account owner, S4) | — | `Collaborator[] {name, display_name}` — provider-backed reviewer typeahead (GitHub repo collaborators / GitLab project members / Bitbucket workspace members), unfiltered list cached in-memory per repo for 30 s, `q` filters case-insensitively |
| GET /git/repos | Git:View | — | `Repo[]` (each carries `forge`: `github`\|`bitbucket`\|`gitlab`\|`unrecognized`\|null — computed live from `remote_url`; `unrecognized` = remote exists but isn't a supported forge, null = no remote) across **all** workspaces the caller may view (root → all); workspace-independent list backing the Git page's top-level repo tabs + landing |
| POST /workspaces/{id}/repos/detect | ws editor | DetectRepoReq | detect a local git repo (resolve remote/provider) |
| PATCH /repos/{id} | ws editor (+ account owner, S4) | `UpdateRepoReq {git_account_id?}` | `Repo` — (re)bind the repo's hosting account; the field is authoritative (an id binds, `null` unbinds). Re-reads `origin` from disk first and persists any change to `remote_url`/`provider`, so a repo whose remote was added after registration becomes bindable. 400 when the account's provider differs from the remote's, or when the repo has no supported remote to bind against. Registration is the only other place an account is resolved, so this is how a repo registered BEFORE its account existed reaches a provider at all. Provider routes (PRs, collaborators, …) also self-heal: a repo with no recorded provider re-reads `origin` from disk before failing (the remote snapshot is taken once at registration, and a failed `git` spawn there is indistinguishable from "no remote"), and an unbound repo whose caller owns exactly ONE account for that provider is bound on first use — the same rule registration applies, and always the caller's own credential. With zero or several candidate accounts nothing is guessed; the call 400s asking for an explicit link. |
| GET /repos/{id}/refs | ws viewer | — | `RefsResp` — branch/tag refs. Each `RefBranch` carries `merged_into_base` (tip already contained in the cleanup base branch → safe to delete; the base branch itself is never flagged); `base_branch` echoes the base merged-status was computed against (per-repo override, else detected default; `null` = no resolvable base). Merged sets come from two bulk `git branch --merged <base>` calls (local + remote), not a per-branch spawn. `RefBranch.sha` / `RefTag.sha` give the commit each ref points at, so a client can locate a ref whose commit isn't in the loaded page of history; annotated tags are dereferenced (`%(*objectname)`) so `RefTag.sha` is always a COMMIT. Tags are returned in full (newest-first), not truncated. |
| GET /repos/{id}/cleanup-base | ws viewer | — | `CleanupBaseResp {base_branch, resolved}` — the per-repo cleanup base override (`base_branch`; `null` = follow the detected default) and what it currently resolves to (`resolved`). Drives the "safe to delete (merged)" indicators. |
| PUT /repos/{id}/cleanup-base | ws editor | `SetCleanupBaseReq {base_branch?}` | `CleanupBaseResp` — set/clear (empty/null clears) the per-repo cleanup base override. Indicator-only: never deletes or moves any branch. |
| POST /repos/{id}/fetch | ws editor | — | RepoStatusResp |
| POST /repos/{id}/discard | ws editor | StagePathsReq | RepoStatusResp |
| POST /repos/{id}/merge | ws editor | MergeBranchReq (`auto_stash` → stash→merge→pop on a dirty tree) | MergeResult (`note` carries auto-stash outcome). 409 when ANY operation (merge/rebase/cherry-pick/revert) is already in progress — resolve or abort it first. |
| POST /repos/{id}/merge/preview | ws viewer | MergePreviewReq | MergePreview (dry-run via `git merge-tree`; no tree mutation) |
| GET /repos/{id}/merge/status | ws viewer | — | `MergeConflictStatus` — in-progress RESOLVABLE state: `merging` is true for any op (merge/rebase/cherry-pick/revert, named in `op`) AND for conflicted files with no state file (`op` absent — a conflicting stash pop / squash); `conflicted_files` lists the unmerged paths. |
| POST /repos/{id}/merge/abort | ws editor | — | RepoStatusResp — aborts with the op's own verb (`merge/rebase/cherry-pick/revert --abort`); a staged squash (SQUASH_MSG present) is discarded with `reset --hard`. With NOTHING in progress → 409 (never a fallback hard-reset: a stale Abort must not destroy uncommitted work). |
| POST /repos/{id}/merge/commit | ws editor | MergeCommitReq | MergeResult — concludes the in-progress op: commits a merge/squash, `--continue`s a rebase/cherry-pick/revert. 409 while conflicts remain, and when there is nothing to conclude. |
| GET /repos/{id}/conflict | ws viewer | — | conflict listing |
| POST /repos/{id}/conflict/resolve | ws editor | ResolveConflictReq (`content`, or `side:"ours"\|"theirs"` to take a whole side via `git checkout --ours/--theirs` + stage) | RepoStatusResp |
| POST /repos/{id}/cherry-pick | ws editor | `{sha}` | RepoStatusResp — a CONFLICTING pick is a normal 200: CHERRY_PICK_HEAD is left in place and the status carries `op_in_progress:"cherry_pick"` + the conflicted paths (finish via merge/commit, abort via merge/abort) |
| POST /repos/{id}/revert | ws editor | `{sha}` | RepoStatusResp — a conflicting revert is a normal 200 (see cherry-pick; `op_in_progress:"revert"`) |
| POST /repos/{id}/checkout-update | ws editor | `{branch}` | `{status: RepoStatusResp, summary}` — the graph's "check out branch" gesture: stash local changes when dirty → checkout (creating a tracking branch from origin when the name only exists remotely) → pull the upstream (merge) → pop the stash. `summary` lists the steps that ran; a failed switch pops the stash back, a failed pull leaves changes stashed (said in the error), a conflicted pop keeps the stash entry. |
| POST /repos/{id}/branch | ws editor | `{name, start_point?, checkout?}` | RepoStatusResp (create a branch, optionally from `start_point` and checking it out) |
| POST /repos/{id}/branch/rename | ws editor | `{from, to}` | RepoStatusResp (rename a local branch) |
| POST /repos/{id}/branch/delete | ws editor | `{name, remote?, local?, force?}` | RepoStatusResp (delete the local branch (`local` default true); `remote:true` also deletes `origin/<name>`; `local:false` = remote-only; never the checked-out branch — 400). `force` defaults FALSE (safe `-d`, refuses an unmerged branch with 409 "not fully merged"); clients escalate to `force:true` (`-D`) only after their own explicit confirm. |
| POST /repos/{id}/tag | ws editor | `{name, sha, message?, push?}` | RepoStatusResp (create a tag at `sha`; annotated when `message`; pushes the new tag when `push:true`) |
| POST /repos/{id}/tag/push | ws editor | `{name}` | RepoStatusResp (push an existing tag to origin) |
| POST /repos/{id}/tag/delete | ws editor | `{name, remote?}` | RepoStatusResp (delete the local tag; `remote:true` also deletes it on origin) |
| GET /repos/{id}/prs/{number}/commits | ws viewer | — | `CommitInfo[]` (PR commits) |
| POST /repos/{id}/prs/{number}/request-changes | ws editor | — | 204 (request changes review) |
| POST /repos/{id}/api-collections/pull | ws editor | — | pull API-client collections committed in the repo |
| POST /repos/{id}/api-collections/push | ws editor | — | commit API-client collections into the repo |
| POST /repos/{id}/pr/draft | ws editor | DraftPrReq | DraftPrResp (AI-drafted title+body, plus the `session_id` that drafted it). When the bundled `pull-request` skill is installed it is prepended to the draft prompt; the branch Jira key (if any) is injected as the title prefix (never in the body). No AI attribution is added. Runs as a REAL Otto session titled `PR draft · <branch>` — it appears in Agents as soon as it spawns, so it can be opened in a pane and watched mid-draft. The turn uses the `pr_draft_model` setting (default `haiku`) and `lean_turn` (no MCP servers, no Bash/Edit/Web tools): the prompt already carries the diff and the skill, so both are pure latency. |
| POST /repos/{id}/draft-commit-message | ws editor | DraftCommitMessageReq (empty `{}`) | DraftCommitMessageResp (AI-drafted Conventional-Commits message from the STAGED diff; falls back to the working diff when nothing is staged). When the bundled `commit-message` skill is installed it is prepended to the draft prompt; the branch Jira key (if any) is injected into the subject. No AI attribution is added. |

## PR review agents (multi-agent code review)

| Method & path | Auth | Request | Response |
|---|---|---|---|
| POST /repos/{id}/prs/{number}/review | ws editor | StartReviewReq | Review (starts the agent fan-out) |
| GET /repos/{id}/prs/{number}/review | ws viewer | — | Review (latest, with live agent state) |
| GET /repos/{id}/prs/{number}/reviews | ws viewer | — | `Review[]` (history for the PR) |
| POST /repos/{id}/local-review | ws editor | LocalReviewReq | Review (review the working diff) |
| GET /repos/{id}/local-review | ws viewer | — | latest local Review |
| GET /repos/{id}/local-reviews | ws viewer | — | `Review[]` (local review history) |
| POST /pr-review-comments/{cid}/approve | ws editor | — | post a draft review comment to the PR |
| POST /pr-review-comments/{cid}/decline | ws editor | — | discard a draft review comment |
| POST /reviews/{review_id}/handoff | ws editor | — | hand the review findings to an agent session |
| POST /reviews/{review_id}/cancel | ws editor | — | cancel an in-flight review: signals the run's cancel flag, kills the live agent sessions, marks the run `cancelled`, cleans up temp files and broadcasts `review_changed`. `409` if the review is not `running`. Returns the updated Review. |
| POST /reviews/{review_id}/agents/{index}/retry | ws editor | — | re-run one stuck/failed review agent. The agent's fully-composed prompt (and the run's diff) are DB-persisted at dispatch, so retry survives reboots / temp-dir sweeps / daemon redeploys; the `$TMPDIR` prompt file is the legacy fallback for pre-0100 reviews. `400` when neither source has the prompt. |
| POST /reviews/{review_id}/summarizer/retry | ws editor | — | re-run ONLY the summarize+persist stage from the STORED per-agent findings (no reviewer re-runs). Deletes the review's unposted `draft` comments (approved/declined/posted stay), flips the run back to `running` (live via `review_changed`), re-summarizes with the repo's effective config, and persists the new comments + workflow findings. Falls back to the deterministic Rust-side summary if the summarizer fails OR returns 0 comments while findings exist. `400` if the review is still running or no stored finding has content. Returns the (now `running`) Review. |
| POST /reviews/{review_id}/agents/{index}/stop | ws editor | — | `202` + updated Review: stop one **running/waiting** review agent (trips its cancel flag, kills its session, marks the row `error`/"stopped by user" — still retryable; the rest of the run continues and the summarizer proceeds with the remaining findings). `409` if the row is not running/waiting or is the trailing summarizer; broadcasts `review_changed`. |
| GET /reviews/{review_id}/findings | ws viewer | — | `Finding[]` — **widened** from `ReviewFindingRow[]` to the full workflow `Finding` (all old fields — `id`, `state`, `severity`, `body`, `path`, `line`, `fingerprint` — are retained; the rich workflow fields are added). Non-breaking superset. See "Review findings workflow" below. |
| POST /reviews/{review_id}/findings/{fingerprint}/state | ws editor | `{state, fix_session_id?}` | updated finding (legacy lifecycle transition — **deprecated**, kept for back-compat; new UI uses the id-keyed `/findings/{id}/*` actions below) |
| GET /reviews/{review_id}/merge-readiness | ws viewer | — | `MergeReadiness` (open/total findings + approvals + ci_status + mergeable + conflicts + branch freshness) |

## Review findings workflow

The multi-agent review persists each finding as a tracked workflow record with a
6-state `status` (`open · accepted · false_positive · fixed · verified · waived`)
and an immutable `FindingEvent` audit trail. The action endpoints below are keyed
by the stable finding `id`; each validates the status transition, appends a
`finding_events` row, emits the `finding_updated` WS event, and returns the
updated `Finding`. Agent-backed actions (fix / verify / regression-test) also
return a `session_id` for the spawned, openable agent session. Findings reads are
`Git` **viewer**; writes are `Git` **editor**; repo-rule routes are `Context`
viewer/editor. See the design at
`docs/superpowers/specs/2026-06-26-review-findings-workflow-design.md`.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /findings/{id} | ws viewer (Git) | — | `FindingDetail` `{finding, events}` (the finding + its full event timeline) |
| POST /findings/{id}/accept | ws editor (Git) | — | `Finding` (open → accepted) |
| POST /findings/{id}/waive | ws editor (Git) | `{reason?}` | `Finding` (→ waived) |
| POST /findings/{id}/false-positive | ws editor (Git) | `{reason?}` | `Finding` (→ false_positive) |
| POST /findings/{id}/require-approval | ws editor (Git) | — | `Finding` (sets the human-approval gate; status unchanged) |
| POST /findings/{id}/approve | ws editor (Git) | `{decision, note?}` | `Finding` — `decision` ∈ `approve`\|`reject`; approve clears the gate (open → accepted), reject → false_positive |
| POST /findings/{id}/jira | ws editor (Git) | `{project_key, issue_type?, account_id?}` | `Finding` (creates a Jira issue, stores `jira_key`/`jira_url`). **400 `{code:"invalid"}`** when no Jira account is configured. |
| POST /findings/{id}/repo-rule | ws editor (Context) | `{title?, body?, glob?}` | `RepoRule` (generalizes the finding into a durable rule fed into the Context Engine; links `repo_rule_id`) |
| POST /findings/{id}/fix | ws editor (Git) | — | `FindingActionResp` `{finding, session_id?}` (spawns a fix agent; open\|accepted → accepted, then async → fixed on commit) |
| POST /findings/{id}/verify | ws editor (Git) | — | `FindingActionResp` `{finding, session_id?}` (verifies resolution; accepted\|fixed\|verified → verified on pass) |
| POST /findings/{id}/regression-test | ws editor (Git) | — | `FindingActionResp` `{finding, session_id?}` (spawns an agent to add a guard test; sets `linked_test`) |
| GET /workspaces/{ws}/repo-rules | ws viewer (Context) | — | `RepoRule[]` (the workspace's repo rules) |
| POST /repo-rules/{id}/toggle | ws editor (Context) | `{enabled}` | `RepoRule` (enable/disable; re-materializes the workspace's rules block) |
| DELETE /repo-rules/{id} | ws editor (Context) | — | 204 |
| GET /reviews/{review_id}/proof-pack | ws viewer (Git) | — | `ReviewProofPack` (live-assembled: summary counts + per-finding evidence/timeline/artifacts + the repo rules from this review) |
| POST /reviews/{review_id}/proof-pack/export | ws editor (Git) | `{format?}` | `ReviewProofPackExport` `{id, review_id, format, markdown, created_at}` (persists a markdown snapshot + ingests verified findings into memory; emits `proof_pack_exported`) |

`Finding` fields: `id, review_id, workspace_id, repo_id, pr_number, fingerprint,
severity` (`critical`\|`high`\|`medium`\|`low`\|`info`)`, category, path, line,
line_end, title, body, evidence, agent_reasoning_summary, suggested_fix, status`
(the 6 values)`, linked_commit, linked_test, reviewer, state` (engine detection
axis)`, regressed, requires_human_approval, approval_decision, approved_by,
approved_at, jira_key, jira_url, produced_by_agent, repo_rule_id, fix_session_id,
occurrence_count, created_at, updated_at`.

## Orchestrator & broadcast (beyond #23–#24)

| Method & path | Auth | Request | Response |
|---|---|---|---|
| POST /workspaces/{id}/broadcast | ws editor | BroadcastReq `{text, session_ids?}` | BroadcastResp `{session_ids}` |
| POST /workspaces/{id}/relay | ws editor | RelayReq `{text}` | RelayResp `{session_ids, broadcast, unaddressed, text}` |

Relay delivers a **name-addressed** message: the leading token(s) of `text` may
name session handles (`ronaldo: …`, `ronaldo, messi: …`, bare `ronaldo do X`) or
the broadcast keyword `all`/`everyone`. When nothing matches, the call is a no-op
with `unaddressed:true` so the caller falls back (e.g. AI orchestrate).

## Session name themes (auto-naming new sessions)

New agent sessions are auto-named from the creating user's active **name theme**
(e.g. "Ronaldo", "Messi") instead of `claude #3`, unique among the workspace's
open sessions. Built-in themes are compiled into the daemon; users may add custom
name lists. Per-user library; the handlers add the per-theme owner guard.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /name-themes | agents view | — | NameThemesResp `{themes:[NameThemeInfo], active}` |
| PUT /name-themes/active | agents edit | SetActiveThemeReq `{theme_id}` | NameThemesResp |
| POST /name-themes | agents edit | CreateNameThemeReq `{label, names}` | CustomThemeResp `{id, label, names}` |
| PUT /name-themes/{id} | agents edit | UpdateNameThemeReq `{label, names}` | CustomThemeResp |
| DELETE /name-themes/{id} | agents edit | — | 204 |

`active` is a built-in id (`footballers`), a custom theme id, or `none` (the
legacy `{provider} #N` numbering). `NameThemeInfo` = `{id, label, kind, capacity, sample}`.

## Product (stories, versions, analyses, test cases, learnings)

The Product module manages imported stories and their derived artifacts. Workspace-scoped
collections live under `/workspaces/{ws}/product/*`; item routes resolve the workspace from
the row. AI-producing actions (analyze/rewrite/generate/plan) live under
`/workspaces/{id}/product/...` and return 202 Accepted, streaming progress over `/ws/events`.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /workspaces/{ws}/product/stories | ws viewer | — | `Story[]` |
| POST /workspaces/{ws}/product/stories | ws editor | ImportStoryReq | Story |
| GET /product/stories/{sid} | ws viewer | — | Story |
| PATCH /product/stories/{sid} | ws editor | PatchStoryReq | Story |
| DELETE /product/stories/{sid} | ws editor | — | 204 |
| POST /product/stories/{sid}/refresh | ws editor | — | re-pull the source story |
| GET /product/stories/{sid}/versions | ws viewer | — | `Version[]` |
| GET /product/versions/{vid} | ws viewer | — | Version |
| POST /product/versions/{vid}/publish | ws editor | — | publish a version back to the source |
| GET /product/stories/{sid}/analyses | ws viewer | — | `Analysis[]` |
| GET /product/stories/{sid}/linked-canvases | ws viewer | — | `CanvasSceneSummary[]` — Canvas scenes linked to this story (via `story_id`) |
| GET /product/analyses/{aid} | ws viewer | — | Analysis (with per-agent state) |
| GET /workspaces/{id}/product/lenses | ws viewer | — | `ProductLens[]` (curated analysis-lens catalog: `{skill,label,description,default_on}`) |
| GET /product/stories/{sid}/questions | ws viewer | — | `Question[]` |
| POST /product/stories/{sid}/questions | ws editor | CreateQuestionReq | Question |
| POST /product/stories/{sid}/questions/post | ws editor | — | post questions back to the source story |
| PATCH /product/questions/{qid} | ws editor | UpdateQuestionReq | Question |
| DELETE /product/questions/{qid} | ws editor | — | 204 |
| GET /product/stories/{sid}/notes | ws viewer | — | `Note[]` |
| POST /product/stories/{sid}/notes | ws editor | CreateNoteReq | Note |
| PATCH /product/notes/{nid} | ws editor | UpdateNoteReq | Note |
| DELETE /product/notes/{nid} | ws editor | — | 204 |
| GET /product/stories/{sid}/events | ws viewer | — | story event log |
| GET /product/stories/{sid}/testcases | ws viewer | — | testcase runs for the story |
| PATCH /product/testcases/{tid} | ws editor | UpdateTestcaseReq | Testcase |
| POST /product/testcase-runs/{rid}/approve | ws editor | — | approve a run (triggers skill self-improvement) |
| POST /product/testcase-runs/{rid}/publish | ws editor | — | publish approved test cases |
| POST /product/testcase-runs/{rid}/testcases/bulk-approve | ws editor | `{ids: string[]}` | `{approved: number}` — bulk-approve selected draft cases |
| POST /product/testcase-runs/{rid}/testcases/reorder | ws editor | `{ordered_ids: string[]}` | `Testcase[]` — persist new display order |
| GET /product/stories/{sid}/transcripts | ws viewer | — | `Transcript[]` |
| POST /product/stories/{sid}/transcripts | ws editor | CreateTranscriptReq | Transcript |
| DELETE /product/transcripts/{trid} | ws editor | — | 204 |
| POST /product/stories/{sid}/draft (PATCH) | ws editor | — | create/update the working RFC draft |
| POST /product/stories/{sid}/publish-as-rfc | ws editor | — | publish the draft as an RFC |
| POST /product/stories/{sid}/publish-as-story | ws editor | — | publish the draft as a story |
| GET /workspaces/{ws}/product/learnings | ws viewer | — | `Learning[]` |
| POST /workspaces/{ws}/product/learnings | ws editor | CreateLearningReq | Learning |
| PATCH /product/learnings/{lid} | ws editor | UpdateLearningReq | Learning |
| DELETE /product/learnings/{lid} | ws editor | — | 204 |
| POST /product/learnings/{lid}/accept | ws editor | — | accept a proposed learning |
| GET /workspaces/{ws}/product/drafts | ws viewer | — | `Draft[]` |

### Product AI actions (async; 202 Accepted)

| Method & path | Auth | Request | Response |
|---|---|---|---|
| POST /workspaces/{id}/product/stories/{sid}/analyze | ws editor | AnalyzeReq | Analysis (multi-lens fan-out spawns) |
| POST /workspaces/{id}/product/stories/{sid}/rewrite | ws editor | RewriteReq? | 202 |
| POST /workspaces/{id}/product/stories/{sid}/testcases/generate | ws editor | GenerateTestsReq? | 202 |
| POST /workspaces/{id}/product/stories/{sid}/plan/generate | ws editor | GeneratePlanReq? | 202 (multi-agent: spawns N visible planning sessions + a summarizer when >1; emits `plan_run`) |
| POST /workspaces/{id}/product/stories/{sid}/plan | ws editor | SavePlanReq | 204 (PO checkbox persistence) |
| POST /product/stories/{sid}/to-swarm | ws editor | ToSwarmReq? | ToSwarmResp (create a swarm project from the story + seed tasks from its plan) |
| POST /workspaces/{id}/product/stories/{sid}/inject-session | ws editor | InjectSessionReq | inject story context into a session |
| POST /product/analyses/{aid}/agents/{agent_id}/retry | ws editor | — | 202 (re-run one analysis lens agent) |
| POST /product/analyses/{aid}/agents/{agent_id}/stop | ws editor | — | 202 (stop a running analysis agent) |

### Product story attachments & mockups

Local story attachments (paste/drag/file-picker) stored under
`data_dir/product/attachments/<story_id>/`, served back as bytes; plus pinned
mockup annotations. The story's workspace gates each route (Viewer reads, Editor
mutations). The upload route carries a 40 MB body cap (raw content cap 25 MB).

| Method & path | Auth | Request | Response |
|---|---|---|---|
| POST /product/stories/{sid}/attachments | ws editor | UploadReq (base64) | ProductAttachment |
| GET /product/stories/{sid}/attachments | ws viewer | — | ProductAttachment[] |
| GET /product/attachments/{aid} | ws viewer | — | the file bytes (inline; nosniff) |
| PATCH /product/attachments/{aid} | ws editor | AttachmentPatch | ProductAttachment (e.g. mark as mockup) |
| DELETE /product/attachments/{aid} | ws editor | — | 204 (row + file) |
| GET /product/attachments/{aid}/annotations | ws viewer | — | MockupAnnotation[] |
| POST /product/attachments/{aid}/annotations | ws editor | AnnotationCreateReq | MockupAnnotation |
| PATCH /product/annotations/{id} | ws editor | AnnotationPatchReq | MockupAnnotation |
| DELETE /product/annotations/{id} | ws editor | — | 204 |
| POST /product/stories/{sid}/mockups/assist | ws editor | MockupAssistReq `{prompt, format?, mockup_id?, provider?, model?}` | ProductAttachment — in-place mockup agent: generates (`format`: `html`\|`mermaid`) or refines (`mockup_id`) a `kind:mockup` attachment; streams `mockup_session_started` + `mockup_updated` WS events. `provider`/`model` pick the agent (resolved via configured default when empty; honored on the first/new-mockup session) |

### Product story refinement (talk-to-agent)

A conversational refinement thread on a story. Each turn replays the full thread
history into a one-shot agent run; the agent returns `{reply, updated_story_md?,
summary?}`. When `updated_story_md` is present the backend writes a new
`suggested` story version (which Publish-as-Jira/RFC then picks up). Each thread
has its own working dir; a thread may link a discovery run to seed context. The
story's workspace gates each route (Viewer reads, Editor converse/mutate).

| Method & path | Auth | Request | Response |
|---|---|---|---|
| POST /product/stories/{sid}/refinement-threads | ws editor | CreateThreadReq? ({discovery_run_id?, title?}) | RefinementThread |
| GET /product/stories/{sid}/refinement-threads | ws viewer | — | RefinementThread[] (newest first) |
| GET /product/refinement-threads/{tid} | ws viewer | — | {thread, messages} |
| POST /product/refinement-threads/{tid}/messages | ws editor | {body, provider?, model?} | {user_message, agent_message, story_updated, version_no?} (synchronous; the agent turn runs inline as a managed `run_session_turn` session — `provider`/`model` pick the agent, resolved via the configured default when empty) |
| POST /product/refinement-threads/{tid}/archive | ws editor | — | RefinementThread |

### Product discovery swarm

Launch a repeatable INVESTIGATION swarm from a story (discovery before
implementation). The discovery project is **not** story-linked (the unique
`story_id` index is reserved for the single implementation project); the
`product_discovery_runs` row carries the linkage. Launching auto-starts the
swarm so the discovery agents run. Run status is derived on read from the
discovery project's task statuses (all done → `done`; any error → `error`).

| Method & path | Auth | Request | Response |
|---|---|---|---|
| POST /product/stories/{sid}/discover | ws editor | DiscoverReq? | DiscoverResp (run + auto-started swarm + discovery project + seeded investigation tasks) |
| GET /product/stories/{sid}/discovery-runs | ws viewer | — | DiscoveryRunSummary[] (newest first; derived status + done/total) |
| GET /product/discovery-runs/{rid} | ws viewer | — | DiscoveryRunDetail (tasks, per-task run summaries, `kind=discovery` board messages, report_md) |

## Issue trackers (Jira / Confluence)

Issue accounts are per-user (member, owner-scoped); content reads/writes proxy the
configured Jira/Confluence account.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /issue/accounts | member | — | `IssueAccount[]` (own; token never present) |
| POST /issue/accounts | member | CreateIssueAccountReq | IssueAccount |
| PATCH /issue/accounts/{id} | member (owner) | UpdateIssueAccountReq | IssueAccount |
| DELETE /issue/accounts/{id} | member (owner) | — | 204 |
| GET /issue/projects | member | — | available projects |
| GET /issue/search | member | — | issue search results (JQL) |
| GET /issue/my-work?account_id= | member | — | `MyWorkIssue[]` — the caller's open assigned issues (`assignee = currentUser()`, statusCategory != Done, newest first, one page of 100) with parent/project context for the Focus view hierarchy |
| GET /issue/confluence/spaces | member | — | Confluence spaces |
| GET /issue/confluence/search | member | — | Confluence page search |
| GET /issue/confluence/pages/{page_id}?account_id= | member | — | `ConfluencePageResp` |
| POST /issue/confluence/pages?account_id= | member | CreateConfluencePageReq (`body_md` Markdown **or** `body_html` storage XHTML) | `ConfluencePageResp` (created) |
| PUT /issue/confluence/pages/{page_id}?account_id= | member | UpdateConfluencePageReq (`body_md` **or** `body_html`; version resolved server-side) | `ConfluencePageResp` (updated) |
| GET /issue/confluence/pages/{page_id}/comments?account_id= | member | — | `PageComment[]` |
| POST /issue/confluence/pages/{page_id}/comments?account_id= | member | AddConfluenceCommentReq (`body_md` **or** `body_html`) | `CommentRef` |
| GET /issue/{account_id}/{key} | member | — | issue summary |
| GET /issue/{account_id}/{key}/full | member | — | full issue detail |
| GET /issue/{account_id}/{key}/devstatus?issueId=<id> | member | — | `DevStatus` (branches/commits/PRs; best-effort, empty if no dev tool connected); `issueId` optional — when present skips a round-trip to resolve the numeric id |
| GET /issue/{account_id}/{key}/transitions | member | — | available transitions |
| POST /issue/{account_id}/{key}/transitions | member | DoTransitionReq | apply a transition |
| GET /issue/{account_id}/{key}/assignable | member | — | assignable users |
| PUT /issue/{account_id}/{key}/assignee | member | AssignReq | assign the issue |
| GET /issue/{account_id}/{key}/attachment/{attachment_id} | member | — | attachment bytes |
| POST /issue/{account_id}/{key}/comment | member | AddCommentReq | add a comment |
| GET /issue/{account_id}/{key}/editmeta | member | — | editable fields (`EditableField[]`) |
| PUT /issue/{account_id}/{key}/fields | member | `{ "fields": { "<fieldId>": <value>, ... } }` | full issue detail (re-fetched after update) |
| PUT /issue/{account_id}/{key}/description | member | `{ "body_md": "…markdown…" }` | full issue detail (re-fetched after update) |
| GET /issue/{account_id}/{project_key}/issue-types | member | — | issue types for a project |

Fields body shape: `{ "fields": { <jiraFieldId>: <jiraShapedValue>, … } }` — values are sent
in Jira's native shape (number; `{"id":"…"}` for a single option/version/component/priority;
`[{"id":"…"}]` for an option array; `["a","b"]` for labels; `{"accountId":"…"}` for a user;
`"YYYY-MM-DD"` for a date; `"YYYY-MM-DDTHH:mm:ss.sssZ"` for a datetime). `null` / `[]` clears a non-required field.

Manual edits from the Product story page: the **title** is edited via `PUT .../fields` with
`{ "fields": { "summary": "<new title>" } }` (a plain string in Jira's native shape), and the
**description** via the dedicated `PUT .../description` endpoint, which accepts plain markdown
(`body_md`) and converts it to ADF server-side before writing — the generic `/fields` path can
not carry a description because Jira requires that field in ADF, not a raw string.

## Channel integrations (Telegram / Slack / Webhook / Loom)

`{channel}` is `slack`, `telegram`, or `webhook`. The CRUD endpoints below are
channel-agnostic. For `webhook`, the reused fields carry webhook meanings:
`bot_token` = the inbound secret **key** (set manually or generate one client-side),
`channel_id` = the optional default **reply callback URL**, `allowed_users` = the
optional allowed caller ids (matched against the request's `user`).

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /workspaces/{id}/integrations | ws viewer | — | configured channel integrations |
| PUT /workspaces/{id}/integrations/{channel} | ws editor | UpsertIntegrationReq | Integration |
| DELETE /workspaces/{id}/integrations/{channel} | ws editor | — | 204 |
| POST /workspaces/{id}/integrations/{channel}/test | ws editor | — | sends a test message (webhook: probes the callback URL) |
| POST /workspaces/{id}/integrations/seed-from-loom | ws editor | — | seed integrations from a Loom config |

### Inbound webhook trigger

Public-by-key endpoint that turns an external HTTP `POST` into an agent session
(same engine as Slack/Telegram). The per-webhook secret **key** is the credential —
no Otto session/bearer required — supplied in the `X-Otto-Webhook-Key` header (or
`Authorization: Bearer <key>`) and compared in constant time. Processing is async:
the agent's reply (if any) is POSTed to the per-request `callback_url` or the
integration's configured default. The webhook must be configured and **enabled** via
the CRUD endpoints above first.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| POST /webhooks/{workspace_id} | public-by-key (`X-Otto-Webhook-Key`) | WebhookInboundReq | 202 `{accepted, conversation}` |
| POST /webhooks/swarm/{workspace_id}/{swarm_id} | public-by-key (`X-Otto-Webhook-Key` / `Authorization: Bearer`) | SwarmTriggerReq | 202 `{swarm_id, project_id, started}` |

`SwarmTriggerReq`: `{ goal: string (required), name?: string, repo_path?: string,
start?: bool (default true) }`. An external trigger that starts a swarm fully
automatically: it creates a project (goal = `goal`), runs the planner to seed tasks, sets
the swarm active, and starts the coordinator (agents run in git **worktrees** for parallel
isolation). `start=false` plans only. Auth reuses the **same per-workspace webhook key** as
the channel webhook above (keychain `chan-bot-{ws}-webhook`), via `X-Otto-Webhook-Key` or
`Authorization: Bearer <key>`. Errors: 401 (bad/missing key), 404 (swarm not in workspace),
400 (empty `goal`).

`WebhookInboundReq`: `{ text: string (required), conversation?: string, thread?: string,
user?: string, callback_url?: string }`. The **conversation key** drives session reuse:
explicit `conversation` → `user` → a fresh unique id per call (so distinct callers are
never silently merged into one session). The resolved key is returned as `conversation`
in the 202 body — pass it back as `conversation` to deliberately continue that session.
Errors: 404 (no enabled webhook), 401 (bad/missing key), 400 (empty `text`), 503 (no
root user yet). The callback URL passes through the SSRF guard before each POST. The
callback body is `{kind:"reply", conversation, thread, text}` or, for attachments /
long replies, `{kind:"file", conversation, thread, filename, content_base64}`.

## Self-improvement engine

Per-workspace self-reflection runs and the edits they propose. Reads = `ws viewer`,
config/mutations = `ws editor` (config write = `ws admin`).

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /workspaces/{id}/self-improvement | ws viewer | — | self-improvement config |
| PUT /workspaces/{id}/self-improvement | ws admin | ImproveConfig | config |
| POST /workspaces/{id}/self-improvement/run | ws editor | — | trigger a run now |
| GET /workspaces/{id}/improvement/runs | ws viewer | — | `ImprovementRun[]` |
| GET /improvement/runs/{run_id} | ws viewer | — | ImprovementRun |
| GET /workspaces/{id}/improvement/edits | ws viewer | — | `ImprovementEdit[]` |
| POST /improvement/edits/{eid}/approve | ws editor | — | apply a pending edit |
| POST /improvement/edits/{eid}/reject | ws editor | — | reject a pending edit |
| POST /improvement/edits/{eid}/rollback | ws editor | — | roll back an applied edit |
| POST /sessions/{id}/evolve | ws SelfImprovement:editor | — | trigger a manual per-session live-evolve pass; returns `{ run_id }` |

## Skill evaluations (eval lab)

The eval lab evaluates/iterates a skill against a workspace's sources, scores the
produced code from multiple signals (tests, lint, diff quality, review findings,
human rating) backed by a **Proof Pack**, and gates promotion on score + proof.
Reads = `ws viewer`, run/mutations = `ws editor`; config = root; promote = root.

A run has a `mode`: `generate` (an agent implements the task, default) or
`score_only` (no agent — score an existing `target`: `{kind: working|branch|path}`).
`StartSkillEvalReq` additionally carries `golden_task_id`, `target`, `test_cmd`,
`lint_cmd`, and `weights`. Each iteration gains a `scoring` (`EvalScore`:
per-signal scores + `composite` + `proof_status` + `done_score`), a `proof_pack_id`,
and a `human_rating`.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| POST /workspaces/{id}/skill-evaluations | ws editor | StartSkillEvalReq | SkillEval |
| GET /workspaces/{id}/skill-evaluations | ws viewer | — | `SkillEval[]` |
| GET /workspaces/{id}/skill-sources | ws viewer | — | available evaluation sources |
| GET /skill-evaluations/{id} | ws viewer | — | SkillEval (with iterations) |
| DELETE /skill-evaluations/{id} | ws editor | — | 204 |
| POST /skill-evaluations/{id}/cancel | ws editor | — | cancel a running evaluation |
| POST /skill-evaluations/{id}/promote | root | PromoteSkillReq (`force?`) | promote winning skill; 409 if the score+proof gate is unmet and not forced |
| GET /skill-evaluations/{id}/promote-gate | ws viewer | `?iteration_id` | PromoteGate (allowed + reasons) |
| GET /skill-evaluations/{id}/iterations/{iter_id}/diff | ws viewer | — | iteration impl diff |
| GET /skill-evaluations/{id}/iterations/{iter_id}/score | ws viewer | — | EvalScore |
| GET /skill-evaluations/{id}/iterations/{iter_id}/proof-pack | ws viewer | — | assembled proof pack (header + artifacts) |
| POST /skill-evaluations/{id}/iterations/{iter_id}/rate | ws editor | RateIterationReq | SkillEval (re-scored; no command re-run) |
| POST /skill-evaluations/{id}/iterations/{iter_id}/regression | ws editor | RegressionReq | GoldenTask (origin=regression; deduped by source iter) |
| POST /skill-evaluations/{id}/iterations/{iter_id}/agents/{index}/retry | ws editor | — | re-run one validation agent |
| GET /settings/skill-eval | root | — | skill-eval config (+ weights, promote_min_score, require_proof_pass, default cmds) |
| PUT /settings/skill-eval | root | SkillEvalConfig | config |

### Golden tasks (per-repo evaluation corpus)

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /workspaces/{id}/golden-tasks | ws viewer | `?repo_key` | `GoldenTask[]` |
| POST /workspaces/{id}/golden-tasks | ws editor | GoldenTaskReq | GoldenTask |
| GET /golden-tasks/{id} | ws viewer | — | GoldenTask |
| PUT /golden-tasks/{id} | ws editor | GoldenTaskReq | GoldenTask |
| DELETE /golden-tasks/{id} | ws editor | — | 204 |
| POST /golden-tasks/{id}/run | ws editor | RunGoldenReq | SkillEval (started from the task) |

### Matrices (provider × skill × prompt)

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /workspaces/{id}/eval-matrices | ws viewer | — | `EvalMatrix[]` |
| POST /workspaces/{id}/eval-matrices | ws editor | StartMatrixReq | EvalMatrix (cells fan out as eval runs) |
| GET /eval-matrices/{id} | ws viewer | — | EvalMatrix (with live cell composites/proof) |
| POST /eval-matrices/{id}/cancel | ws editor | — | cancel all still-running cells |

## Skills review (Skills Lab)

Multi-agent audit of a `SKILL.md` package: a deterministic static pass plus (opt-in) N visible
provider agents running the bundled `skills-reviewer` method, folded together by a summarizer.
Reviewer/summarizer sessions are tagged `meta.source="skillreview"` and hidden from the Agents
grid — they embed live in the Review panel. See the `skill_review_updated` WS event.

Every review runs on a staged temp copy of the package with local machine artifacts stripped
(`.mcp.json`, `.DS_Store`, `.git/`, `.env*`, `node_modules`, `__pycache__`) so secrets in those
files are never scanned or quoted. Optional `instructions` ride on the review and are appended
to every reviewer + summarizer prompt. After a review completes, `apply` hands the findings to a
fixer agent (`SkillReview.fix_agent`, same session pattern) that edits the REAL skill directory
— rejected for bundled skills (read-only; install to the library first).

| Method & path | Auth | Request | Response |
|---|---|---|---|
| POST /workspaces/{id}/skill-reviews | ws editor | StartSkillReviewReq (`skill_name`, `skill_source` library\|bundled, `providers[]`, `agent_mode` static\|agents, `instructions?`) | SkillReview (status=running; static pass fills in, agents fan out) |
| GET /workspaces/{id}/skill-reviews | ws viewer | — | `SkillReview[]` |
| GET /skill-reviews/{id} | ws viewer | — | SkillReview (static report + live agents + summary + fix_agent) |
| DELETE /skill-reviews/{id} | ws editor | — | 204 (cancels + archives sessions, fixer included) |
| POST /skill-reviews/{id}/cancel | ws editor | — | cancel a running review |
| POST /skill-reviews/{id}/agents/{index}/retry | ws editor | — | re-run one reviewer agent |
| POST /skill-reviews/{id}/apply | ws editor | ApplySkillFixReq (`provider?` default claude, `instructions?`) | SkillReview (fix_agent spawns; 400 while review/fixer running, for bundled skills, or with no findings) |

## Context library (skills / souls / context)

The shared skill/soul/context library lives under the daemon data dir. Library reads/writes
are root; per-workspace context selection is workspace-scoped.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /library/skills | root | — | `SkillEntry[]` |
| POST /library/skills | root | CreateLibrarySkillReq (`name`, `category?`, `description?`, `body?`) | LibrarySkill (new; 409 if it exists) |
| POST /library/skills/import | root | raw `.zip` body, `?name=` override | LibrarySkill (imported; zip-slip + size guarded) |
| GET /library/skills/{name} | root | — | skill body |
| PUT /library/skills/{name} | root | skill body | 204 |
| DELETE /library/skills/{name} | root | — | 204 (also removes Otto-managed user-level provider copies — see Bundled skills) |
| GET /library/skills/{name}/files | root | — | `SkillFileEntry[]` (multi-file tree) |
| GET /library/skills/{name}/file | root | `?path=<rel>` | SkillFileContentResp (one file's text) |
| PUT /library/skills/{name}/file | root | WriteSkillFileReq (`path`, `content`) | `SkillFileEntry[]` (refreshed tree; evicts cache) |
| DELETE /library/skills/{name}/file | root | `?path=<rel>` | 204 (SKILL.md cannot be deleted) |
| GET /library/provider-skills | any member | — | `ProviderSkillInfo[]` (on-disk `~/.claude|.codex|.agy/skills`, read-only) |
| GET /library/provider-skills/{provider}/{name} | any member | — | ProviderSkillContent (body + file list) |
| GET /library/provider-skills/{provider}/{name}/file | any member | `?path=<rel>` | SkillFileContentResp |
| GET /library/souls | root | — | `SoulEntry[]` |
| GET /library/souls/{name} | root | — | soul body |
| PUT /library/souls/{name} | root | soul body | 204 |
| DELETE /library/souls/{name} | root | — | 204 |
| GET /library/context | root | — | `ContextEntry[]` |
| GET /library/context/{name} | root | — | context body |
| PUT /library/context/{name} | root | context body | 204 |
| DELETE /library/context/{name} | root | — | 204 |
| GET /library/default-soul | root | — | the default soul name |
| PUT /library/default-soul | root | `{name}` | set the default soul |
| GET /workspaces/{id}/context | ws viewer | — | the workspace's active context selection |
| PUT /workspaces/{id}/context | ws admin | UpdateWsContextReq | selection |
| POST /workspaces/{id}/context/materialize | ws editor | — | materialize the active set into the CLIs |
| POST /workspaces/{id}/context/preview | ws viewer | `ContextPreviewReq` | `ContextPreviewResp` — dry-run of what a spawn would materialize |

`POST /workspaces/{id}/context/preview` is a **dry-run**: it returns exactly what
a session spawn would materialize for one or more providers — the skill files,
selected soul, the generated `AGENTS.md` / `CLAUDE.md` content, and the runtime
hooks — **without spawning a session or writing any file**. It is the same
`plan()` the real spawn path uses, so the preview matches the spawn byte-for-byte.

The request body lets the UI preview a not-yet-saved selection: every field is
optional and, when present, overrides the workspace's stored context config (the
same inputs a spawn uses — provider, skills, soul, extra context, memory, cwd).
`provider` omitted ⇒ preview both `claude` and `codex`; `cwd` omitted ⇒ the
workspace root. A supplied `cwd` is confined to the workspace root (resolved
through symlinks/`..`); a path outside it is rejected `403` (a preview reads the
target's `CLAUDE.md`/`AGENTS.md`/settings, so an arbitrary `cwd` would leak host
files to a Viewer). For `skills`/`soul`, omitting the key inherits the stored
value, while an explicit `null` overrides it (all library skills / global default).

```ts
interface ContextPreviewReq {
  provider?: string;            // omit ⇒ claude + codex
  skills?: string[] | null;     // omit ⇒ stored; null ⇒ all library skills
  soul?: string | null;         // omit ⇒ stored; null ⇒ global default
  extra_context_md?: string;    // omit ⇒ stored
  include_memory?: boolean;     // omit ⇒ stored
  include_repo_map?: boolean;   // omit ⇒ stored; opt-in tree-sitter repo map
  cwd?: string;                 // omit ⇒ workspace root
}

interface ContextPreviewResp { providers: ContextPreviewProvider[]; }

interface ContextPreviewProvider {
  provider: string;
  skipped: boolean;             // true for shell/custom (nothing materialized)
  skills: ContextPlanSkill[];   // resolved active skills (advisory)
  soul: string | null;          // applied soul name (advisory)
  files: ContextPlanFile[];     // every file the spawn would write
  generated_instructions: string;        // exact AGENTS.md/CLAUDE.md bytes (advisory)
  instructions_file_name: string | null; // "CLAUDE.md" | "AGENTS.md"
  generated_hooks: string | null;        // settings.local.json JSON (enforced)
}

interface ContextPlanFile {
  path: string;                 // absolute destination path
  kind: string;                 // instructions | skill | skill_asset | hooks | manifest
  enforcement: 'advisory' | 'enforced';
  size: number;                 // bytes
  first_lines: string;          // short excerpt
  truncated: boolean;           // content elided from first_lines
}

interface ContextPlanSkill { name: string; description: string; version: number; }
```

**Advisory vs enforced.** Each artifact is labeled by how binding it is on the
agent: `advisory` — instruction files (`AGENTS.md`/`CLAUDE.md`) and skills are
guidance the model reads and *may ignore*; `enforced` — hooks / runtime settings
(`.claude/settings.local.json`) the daemon imposes regardless of the model's
choices. The UI surfaces this distinction in the preview.

## Bundled skills (first-party skill catalog)

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /library/bundled | root | — | bundled skill catalog |
| GET /library/bundled/{name} | root | — | BundledSkillContent (SKILL.md body + file list; view without installing) |
| POST /library/bundled/{name}/install | root | — | install/update one bundled skill |
| POST /library/bundled/install-all | root | `?category=&backup=` | install all bundled skills (optionally one category) |

Each catalog entry carries `{name, category, version, description, installed_version,
state, update_available}`. `state` is `not_installed | up_to_date | update_available
| ahead`; `update_available` is `true` only when the bundle is strictly newer than
the installed copy (`bundled > installed`) — a hand-edited copy that is `ahead`
stays `false`. The UI uses `update_available` to show an **Update** button.

**Install also materializes user-level copies.** Installing/updating a bundled skill
copies the full multi-file tree into the Otto library AND into each provider CLI's
native global skills dir so the skill is discoverable everywhere: claude →
`~/.claude/skills/<name>/`, codex → `$CODEX_HOME/skills/<name>/` (else
`~/.codex/skills/<name>/`), agy → `~/.gemini/skills/<name>/`. The copy is a
clean-overwrite (so install doubles as update), and each provider dir keeps an
`.otto-managed.json` manifest listing the skills Otto owns there. Nothing is ever
written into a working/repo tree. `DELETE /library/skills/{name}` (below) reconciles
those user-level copies too, removing only skills the manifest owns — a user-authored
skill of the same name is left untouched.

## Workflow engine

Visual node-graph automations and their runs. Templates/node-types are member-readable;
workflows are workspace-scoped (reads `ws viewer`, mutations `ws editor`); runs resolve the
workspace from the workflow/run row.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /workflows/node-types | member | — | available node type descriptors |
| GET /workflows/templates | member | — | workflow templates |
| GET /workspaces/{wid}/workflows | ws viewer | — | `Workflow[]` |
| POST /workspaces/{wid}/workflows | ws editor | CreateWorkflowReq | Workflow |
| POST /workspaces/{wid}/workflows/from-template | ws editor | FromTemplateReq | Workflow |
| POST /workspaces/{wid}/workflows/generate | ws editor | GenerateWorkflowReq | Workflow (AI-generated) |
| GET /workflows/{id} | ws viewer | — | Workflow |
| PATCH /workflows/{id} | ws editor | UpdateWorkflowReq | Workflow |
| DELETE /workflows/{id} | ws editor | — | 204 |
| POST /workflows/{id}/run | ws editor | RunWorkflowReq? | WorkflowRun — created immediately; may start **queued** (see Run queue) |
| GET /workflows/{id}/runs | ws viewer | — | `WorkflowRun[]` |
| GET /workspaces/{wid}/workflow-runs/active | ws viewer | — | `ActiveWorkflowRun[]` — in-flight runs (pending\|running) across the workspace, newest first; backs the "Running" sidebar list |
| GET /workflow-runs/{id} | ws viewer | — | WorkflowRun |
| POST /workflow-runs/{id}/cancel | ws editor | — | cancel a run |
| POST /workflow-runs/{id}/retry-node | ws editor | `{node_id, include_downstream?}` | WorkflowRun — re-enter a **finished** run in place: the run reopens (back to running), out-of-scope nodes keep their prior state/output, in-scope nodes re-execute (same run id ⇒ same context dir + `otto-wf/<run_id>` worktree/branch — unlike the canvas "run from here", which mints a fresh run/worktree), then the run's final status is recomputed. Scope: the target step only (default; target must be `error`), or target + descendants with `include_downstream: true` (any settled target). Retry re-entries bypass node-cache READS so in-scope nodes genuinely re-execute. `409` while the run is still active; `400` on a bad target |
| GET /workflows/{id}/versions | ws viewer | — | `WorkflowVersion[]` — graph snapshot history, newest first |
| GET /workflows/{id}/versions/{v} | ws viewer | — | `WorkflowVersion` — one snapshot (404 if `v` unknown) |
| POST /workflows/{id}/versions/{v}/restore | ws editor | `RestoreVersionReq {note?}` | Workflow — copies `v`'s graph back in as a **new** version (append-only history) |

**Instructions (0096).** `Workflow`, `WorkflowVersion`, and `WorkflowTemplate` all
carry `instructions: string` — standing free-text guidance every run/step follows,
distinct from `description` (a summary). `CreateWorkflowReq`/`UpdateWorkflowReq`
accept an optional `instructions`; create defaults to `""` when omitted, a
template-created workflow (`POST .../from-template`) inherits the template's
`instructions`, and an AI-generated workflow (`POST .../generate`) always gets
`""` (the generation prompt IS `description`). An instructions-only `PATCH` bumps
the version and snapshots exactly like a graph-changing one (see Versioning).
Backed by migration **0096** (`instructions` column on `workflows` and
`workflow_versions`).

**Versioning.** A `Workflow` carries a monotonic `version` (default 1). A snapshot
is written on create (v1) and on **every graph- or instructions-changing PATCH**
(`bump_version` + `snapshot_version`, note `"edited"`); restoring writes a new
version equal to the chosen one rather than rewinding the counter (note
`"restored from v{n}"`) and restores the graph **and** instructions to the live
row (name/description are not — they only live in the version snapshot).
`WorkflowVersion` = `{id, workflow_id, version, name, description, instructions,
graph, note, created_by, created_at}`. Backed by migration **0089**
(`workflows.version`, `workflow_versions` table).

**Run fields (0089).** A `WorkflowRun` now also carries `workflow_version` (the
version snapshot it executed) and `proof_pack_id` (the Proof Pack assembled on
completion — each node output becomes a `log` artifact, each `human_approval` an
`approval` artifact). Each `NodeRunState` gains `attempts` (retry count; `0` =
cache hit) and `sessions` (openable Otto session ids the node spawned — agent /
product / canvas / loop-inner turns — reported live as they are created;
`review_run` additionally surfaces a `review_id` in its output).

**Run fields (0092).** A `WorkflowRun` additionally carries `rev` — a monotonic
revision bumped on **every** persisted progress write (node transitions, the
human-approval pause, the approve/reject decision). Clients use it to discard
stale/out-of-order snapshots and to order `workflow_run_updated` events; legacy
rows report 0. The approval columns now ride the run too:
`waiting_approval` (bool), `approval_node_id`, `approved_by`, `approval_note`,
`approved_at` — previously only surfaced on `ActiveWorkflowRun`, which made the
run-view approval banner unreachable. Each `NodeRunState` gains `started_at`
(set on the pending→running transition; drives the live elapsed timer on a
running step).

**Run queue.** At most **2** workflow runs execute at once, daemon-wide
(override: `OTTO_WF_MAX_PARALLEL_RUNS`, ≥ 1) — a single run can fan out dozens
of agent PTYs (a `review_run` node launches the whole reviewer fleet), so
uncapped parallel runs exhausted file descriptors and starved interactive
sessions. Every trigger path (manual run, retry-node, webhook, schedule/event
trigger, chat, scheduled task) shares the gate. A run beyond the cap stays
`pending` (the UI shows it as *queued*) and starts FIFO as slots free; its
`workflow_runs` row **is** the queue entry, so the queue is persistent: on
daemon restart, queued runs re-enqueue in creation order while runs that were
*executing* are failed as interrupted (as before). `POST
/workflow-runs/{id}/cancel` on a queued run is honored — it never starts. One
exception on restart: a **retry-node** re-entry that was still queued fails
instead of resuming (its retry scope lives only in the engine's memory;
re-running blind would replay finished steps' side effects).

**Running list.** `GET /workspaces/{wid}/workflow-runs/active` returns
`ActiveWorkflowRun = {run_id, workflow_id, workspace_id, workflow_name, status,
started_at, nodes_total, nodes_done, waiting_approval}` for every in-flight run
(`pending`/`running`) in the workspace — a lightweight join (run × workflow name)
with step progress precomputed, so the "Running" sidebar list + the Workflows nav
count refresh on each `workflow_run_updated` WS event without per-run fetches.

**Node kinds (catalog).** `GET /workflows/node-types` returns each kind's
`NodeTypeSpec`, now including `output_schema` (declared output shape; drives UI
expression hints + warn-only runtime validation) and `params_schema`. The control
flow / wired kinds added in this wave: `condition`, `loop`, `product_analyze`,
`product_rewrite`, `product_plan`, `product_publish`, `review_run`, `canvas`,
`git_pr`, `prepare_context` (see the Workflows feature doc for each kind's params/output).
`WorkflowEdge` carries an optional `condition` (an `otto_core::expr` expression
over `{output, input, node, run}`; the edge is active only when truthy) and
`WorkflowNode` an optional `retry` `{max_attempts(≤5), backoff_ms(≤60000), factor}`.

**New node params (this wave).** Agent-backed steps (`agent_prompt`) accept
`skill`/`skills` (string / string[]) — each named skill's body is inlined ahead of
the prompt. `review_run` runs the multi-agent PR-review engine per step: `providers`
(string[]), `lenses` (string[]; `skills` is an alias), `threshold` (0–100, default
80), `require_pass` (bool — errors the step below threshold); its output adds
`score`/`passed`/`blocking`/`advisory`/`findings`/`providers`/`lenses` (empty
`providers`+`lenses` ⇒ the stored/default review config). `git_pr` accepts
`open` (bool, default `false`) — `true` opens the PR on the remote (gate it on the
incoming edge passing). A run started from a chat (`Action: Workflow`) **streams
live per-step progress** back into the trigger thread (origin `channel`/`chat`/
`thread`, or a `result_chat`/`result_channel`/`result_thread` override) before the
final result delivery (`final-output.md` when the run produced one, else
`summary.md` — see *Run context files* below); manual/webhook-only runs do not
stream. See the Workflows feature doc for full per-kind params/output.

**`prepare_context` node.** App-side context gathering, run before the agent turns
that need it: it resolves a Jira key — `params.key` → `input.jira_ticket` (both
trusted verbatim) → the first Jira-key-shaped token scanned out of `input.prompt`,
then `input.msg` — fetches the ticket through the workspace's configured Jira issue
account (`params.account_id` wins; else the run user's account; else any configured
Jira account), and writes it to `jira-<KEY>.md` in the run's context dir. A fetch
failure writes a loud "could not fetch" placeholder instead of failing the node,
**unless** `params.require: true`, which errors the step. Output:
`{ jira: { found, key?, fetched?, summary?, status?, url?, error? } }`. An optional
`params.prompt` (+ `provider`, default `claude`) runs a second, `agent_prompt`-style
phase over the gathered context — a visible session, its `reply` merged into the
output. `prepare_context` is **excluded from the per-node output cache** (unlike
every other kind): a re-run always re-fetches, since a Jira ticket can change
between runs. The `ui-test-authoring` and `api-acceptance-test-authoring` templates
lead with this node.

**Run input: the `prompt` convention.** Every trigger path converges on
`input.prompt` as the run's ask: a chat message's text, the simplified `run <name>:
<prompt>` command's tail, the manual Run-dialog's Prompt box, a webhook body's
`prompt` field, and a `schedule` trigger's `spec.prompt` all land there before the
graph executes. When a trigger only set `msg` (the chat paths) and left `prompt`
unset or blank, the engine's `normalize_prompt` copies `msg` into `prompt` — so
every agent-facing step and `prompt.md` (below) can rely on `input.prompt` without
caring which trigger started the run. An explicit `prompt` is never overwritten.

**Run input: repos declarations.** The run input accepts a `repos` array naming
every repo/branch/worktree the run operates on — **source and destination** —
which all git-aware steps (`review_run`, `git_pr`) consume:

```json
{ "repos": [
  { "repo": "otto_os", "type": "branch",   "name": "feat/x", "source": "develop" },
  { "repo": "koala",   "type": "worktree", "name": "~/wt/koala-fix" }
] }
```

`repo` = a registered repo's id, name, or path. `type: "branch"` → `name` is the
working branch (resolved to the checkout that has it checked out; error if
nowhere) and `source` is the branch the work diffs/PRs against. `type:
"worktree"` → `name` is the worktree path. A missing `source` resolves to the
repo's **detected default branch** (`origin/HEAD` → `main`/`master`/`develop`/
`trunk` probes) — the engine never fabricates `main`, and an unresolvable base
fails with the candidate list instead of `git` exit 128. At run start the
entries are normalized and seeded into the input (`working_directory`, `base`,
`repo_id` from the first valid entry — explicit keys win — plus normalized
`repos[]`); with **multiple** valid entries and no explicit target, `review_run`
reviews every entry (aggregate: `score` = min, `passed` = all; per-repo detail
under `reviews[]`) and `git_pr` drafts/opens one PR per entry.

**Run context files.** Every run owns `<data_dir>/workflow-context/<run_id>/`,
the file-based step-handoff layer (`workflow_context.rs`). Every write here is
best-effort — a failure logs and the run continues; context files never fail a
node.

| File | Written when | Contents |
|---|---|---|
| `instructions.md` | `workflow.instructions` is non-empty | Verbatim copy of the workflow's standing instructions. |
| `prompt.md` | `input.prompt` is non-empty (after the `prompt` convention above resolves it) | This run's ask, verbatim. |
| `run-brief.md` | always | The mission brief written at run start: trigger, mission (msg/prompt/Jira ticket/goals/relevant info), the repos table, the planned steps, and a "how to use this directory" section naming only the files that actually exist for this run. Renamed from the legacy `wf-<run_id>-instruction.md`. |
| `repos.json` | at least one `repos[]` entry was declared/resolved | Live registry of the repo declarations (see above) — updated whenever a step publishes a repo reference. |
| `jira-<KEY>.md` | a `prepare_context` node resolved a Jira key | The fetched ticket (or a loud "could not fetch" placeholder on failure) — see the `prepare_context` node above. |
| `step{N}-{slug}.md` + `.output.json` | after every node attempt concludes | Curated handoff summary (agent-written, or an engine-rendered fallback with the reply untruncated) + the raw output (pretty JSON, 5 MiB cap with a truncation marker). Loop iterations add `-iter{X}`. |
| `final-output.md` | the run finishes `success` | Copy of the last content-bearing, error-free step's `.md` — the run's deliverable. "Content-bearing" excludes utility/bookkeeping kinds (`manual_trigger`, `log`, `delay`, `channel_notify`, `budget_gate`, `human_approval`); a run whose only successful steps are utility kinds produces no `final-output.md`. |

Agent-backed steps are pointed at the directory in their prompt (an ordered
read list: `instructions.md` → `prompt.md` → `run-brief.md` → `repos.json` →
prior `step*.md`, each named only when it exists) and asked to write their own
`step{N}-{slug}.md` handoff before finishing; the engine writes the
full-fidelity fallback (agent replies untruncated) when they don't — a file
left behind by a failed earlier attempt is replaced, so downstream steps never
read a failed attempt's summary. `GET /workflow-runs/{id}` carries the derived
`context_dir` (absolute path; present when the directory exists — absent on
list endpoints); the run view renders a browsable file tree over it via the
existing sandboxed `/fs/browse` + `/fs/read`, including a dedicated Final
output panel that reads `final-output.md` on a successful run. Context files
are unredacted local artifacts in the same trust domain as `nodes_json`; any
future remote serving (share links) must redact on delivery.

**Delivery.** A run started from a chat finishes by replying in the origin
channel/thread (or a `result_chat` override) with a brief status plus one
attachment: `final-output.md` when the run produced one (success with a
qualifying step), otherwise the always-generated `summary.md` (every step,
its status/duration/attempts, and an output peek). Both attachment kinds are
redacted before leaving the machine. A `result_webhook`/`callback_url` in the
run input receives the same summary via the SSRF-guarded webhook path.
Canceled/errored runs never attach `final-output.md` (only a `success` run
computes it) — they always get `summary.md`.

## API client ("Postman") — collections, requests, environments, automations

A full in-app HTTP/gRPC client. All routes are workspace-scoped (`/workspaces/{wid}/...`);
reads = `ws viewer`, mutations/execution = `ws editor`.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /workspaces/{wid}/api-client/collections | ws viewer | — | `Collection[]` |
| POST /workspaces/{wid}/api-client/collections | ws editor | CreateCollectionReq | Collection |
| PATCH /workspaces/{wid}/api-client/collections/{id} | ws editor | UpdateCollectionReq | Collection |
| DELETE /workspaces/{wid}/api-client/collections/{id} | ws editor | — | 204 |
| GET /workspaces/{wid}/api-client/collections/{id}/openapi | ws viewer | — | export the collection as OpenAPI |
| GET /workspaces/{wid}/api-client/requests | ws viewer | — | `Request[]` |
| POST /workspaces/{wid}/api-client/requests | ws editor | CreateRequestReq | Request |
| GET /workspaces/{wid}/api-client/requests/{id} | ws viewer | — | Request |
| PATCH /workspaces/{wid}/api-client/requests/{id} | ws editor | UpdateRequestReq | Request. Create/Update carry the persisted extras: `pre_request_script?`, `post_response_script?`, `settings?` (`{timeout_ms?, follow_redirects?, verify_ssl?}`), `docs?`, `graphql_variables?` |
| DELETE /workspaces/{wid}/api-client/requests/{id} | ws editor | — | 204 |
| GET /workspaces/{wid}/api-client/environments | ws viewer | — | `Environment[]` |
| POST /workspaces/{wid}/api-client/environments | ws editor | CreateEnvironmentReq | Environment |
| PATCH /workspaces/{wid}/api-client/environments/{id} | ws editor | UpdateEnvironmentReq | Environment |
| DELETE /workspaces/{wid}/api-client/environments/{id} | ws editor | — | 204 |
| POST /workspaces/{wid}/api-client/environments/{id}/activate | ws editor | — | set the active environment |
| GET /workspaces/{wid}/api-client/history | ws viewer | — | request history |
| DELETE /workspaces/{wid}/api-client/history | ws editor | — | clear history |
| POST /workspaces/{wid}/api-client/execute | ws editor | ExecuteRequestReq | execute an HTTP request |
| POST /workspaces/{wid}/api-client/secure-all | ws editor | — | `{requests_secured, env_keys_secured}` — one-pass Keychain sweep |
| POST /workspaces/{wid}/api-client/grpc/describe | ws editor | GrpcDescribeReq | service/method descriptors |
| POST /workspaces/{wid}/api-client/grpc/invoke | ws editor | GrpcInvokeReq | gRPC call result |
| POST /workspaces/{wid}/api-client/grpc/reflect | ws editor | GrpcReflectReq | server reflection listing |
| POST /workspaces/{wid}/api-client/oauth2/token | ws editor | OAuth2TokenReq | fetched OAuth2 token |
| GET /workspaces/{wid}/api-client/cookies | ws editor | — | THIS workspace's cookie jar (jars are per-workspace, never shared; values are live credentials — editor-gated) |
| DELETE /workspaces/{wid}/api-client/cookies | ws editor | — | clear THIS workspace's jar |
| GET /workspaces/{wid}/api-client/automations | ws viewer | — | `Automation[]` |
| POST /workspaces/{wid}/api-client/automations | ws editor | CreateAutomationReq | Automation |
| PATCH /workspaces/{wid}/api-client/automations/{id} | ws editor | UpdateAutomationReq | Automation |
| DELETE /workspaces/{wid}/api-client/automations/{id} | ws editor | — | 204 |
| POST /workspaces/{wid}/api-client/automations/{id}/run | ws editor | — | run an automation |
| POST /workspaces/{wid}/api-client/postman/sync | ws editor | `{api_key?, remember?}` | fetch EVERY collection + environment from the user's Postman account (api.getpostman.com) → `{collections: PostmanV21[], environments: PostmanEnv[], failed: [{name,error}], remembered}`. `api_key` optional when a prior sync stored one (`remember: true` → Keychain, ref `apiclient-postman`; only persisted after the key proved valid). Caps at 200 items per kind (Postman rate limits). The UI imports the returned docs through its normal import pipeline. |
| POST /api-client/import-curl | member | `{curl}` | parsed Request from a curl command |

**Durable request extras.** `CreateRequestReq` / `UpdateRequestReq` → `Request` carry an
optional `extras` object persisting the once-draft-only fields:
`{v, transport?, graphql_variables?, docs_md?, scripts:{pre?,post?}?, settings:{timeout_ms?,
follow_redirects?, tls_verify?}?}`. The UI owns the inner shape (like `auth`); the server
validates only that it is a JSON object ≤ 256 KiB (else 400). `NULL`/absent = never set.
Automation runs honour `extras`: pre/post scripts execute server-side with the same `pm` API
as the interactive runner, `settings` map onto the per-request execution options, and
`graphql_variables` are combined with the query body. Non-`http` transports are not runnable
in automations (the step reports an error). The OpenAPI export serializes `docs_md` (operation
`description`), `settings` (`x-otto-settings`) and `graphql_variables`
(`x-otto-graphql-variables`); scripts map to Postman `prerequest`/`test` events in the
git-sync export.

**Keychain-backed secrets.** Secret auth members (bearer `token`, basic `password`, api_key
`value`, oauth2 `client_secret`/`refresh_token`/`password`/`access_token`) never persist in
SQLite: on save the daemon moves a plaintext member to the macOS Keychain
(`otto.api.request.<request_id>`, one JSON blob per request) and stores a
`{"$secret": "<ref>"}` marker in its place (lazy migration — old rows stay valid until
touched, or swept via `POST …/secure-all`). Environments mirror this: `Environment` gains
`secret_keys:[string]`; `CreateEnvironmentReq`/`UpdateEnvironmentReq` accept `secret_keys` +
write-only `secret_values:{k:v}` (absent keys keep stored values); the row's `variables`
holds non-secret pairs only and GET never returns a secret value. Markers resolve in-memory
only at execute/automation time — the ref must point at a request in the same workspace
(else 400) — and every export path (OpenAPI, git-sync, history) sees markers or `***`, never
values. History snapshots redact secret members to `"***"`. `secure-all` additionally marks
environment variables with secret-shaped NAMES (token/secret/passw/api-key/authorization/
credential) as secret; it is idempotent and requires ws editor. The `oauth2/token` endpoint
accepts `client_secret`/`password`/`refresh_token` as plain strings or `$secret` markers.

**Cookie jar scope.** The cookie jar is per-WORKSPACE (in-memory per daemon run): cookies
captured executing in one workspace are never replayed for another. The cookies endpoints
above operate on the caller's workspace jar.

**SSH tunnel (IP whitelisting).** Both the saved request (`CreateRequestReq` /
`UpdateRequestReq` → `Request`) and `ExecuteRequestReq` accept an optional
`ssh_connection_id` (nullable). When set, the daemon routes the outbound HTTP
request through a SOCKS5 proxy over that `ssh`-kind connection (a `ssh -N -D`
tunnel, reused/cached per bastion), so it egresses from the bastion's
whitelisted IP. The referenced connection must be an `ssh`-kind profile visible
to the workspace (workspace-scoped or global); it must carry `host`+`user` in
its params (auth flows through the system `ssh` client). The SSRF guard stays in
force — the target host is still resolved/classified locally — so this is for
**public, IP-restricted** upstreams, not for reaching private hosts. A
resolution or tunnel failure is reported as a `502` and recorded in history.

## Notifications (notification center)

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /notifications | member | — | `Notice[]` — global/system notices + the caller's own (root sees all) |
| DELETE /notifications | member | — | clears the caller's own notices (root clears all; global/system notices remain for non-root) |
| GET /notifications/settings | member | — | notification settings |
| PUT /notifications/settings | member | NotificationSettings | settings |
| POST /notifications/read-all | member | — | marks the caller's own notices read (root marks all) |
| POST /notifications/{id}/read | member | — | mark one read (own only for non-root; global notices are read-only to them) |
| DELETE /notifications/{id} | member | — | dismiss one (own only for non-root) |

Scoping: a notice is either **global/system** (`user_id = null`, e.g. credential/session/skill-eval producers) or **owned by one user**. Non-root members see global + their own and may mutate only their own; the unread badge counts a member's own unread only (global notices show in the list but aren't counted, since a member can't mark them read). Root sees and mutates everything.

## User Feature Grants (RBAC Task 2.1)

Per-user, per-feature capability grants. Any route under `/users/` requires `Users:Admin`
(feature guard) or root. `/auth/capabilities` is self-scoped and exempt — any authenticated
user may call it.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /users/{id}/grants | Users:Admin or root | — | `UserGrantsResp {grants: GrantEntry[]}` |
| PUT /users/{id}/grants | Users:Admin or root | `UserGrantsReq {grants: GrantEntry[]}` | `UserGrantsResp` (atomically replaces all grants; audited) |
| GET /users/{id}/plugin-grants | root | — | `UserGrantsResp` (plugin grants; `GrantEntry.feature` = plugin slug) |
| PUT /users/{id}/plugin-grants | root | `UserGrantsReq` (`feature` = plugin slug) | `UserGrantsResp` (atomically replaces all plugin grants; audited) |
| GET /auth/capabilities | member (any authed user) | — | `CapabilitiesResp {capabilities: {feature-or-slug: capability}}` |

- `GrantEntry` = `{feature: string, capability: string}` using snake_case strings
  (e.g. `{feature:"database", capability:"view"}`).
- **Custom plugins** reuse `GrantEntry` with `feature` set to the plugin **slug** on the
  `/users/{id}/plugin-grants` endpoints (string-keyed RBAC axis, parallel to the closed
  `Feature` enum). `/auth/capabilities` additionally returns each installed plugin's
  `slug → capability` so the UI can gate the plugin's nav entry. See the Custom Plugins
  section and `docs/plugins/AUTHORING.md`.

## Custom Plugins (runtime, out-of-process)

Plugins are external sidecar processes installed at runtime under `~/otto-plugins`
(no app rebuild). Otto supervises them and reverse-proxies their HTTP. Design:
`docs/superpowers/specs/2026-06-21-runtime-plugins-design.md`; authoring:
`docs/plugins/AUTHORING.md`.

| Method & path | Auth | Notes |
|---|---|---|
| GET `/plugins` | member | Enabled plugins `[{slug,name,icon,has_ui}]` for the sidebar; UI filters by grant. Exempt in policy. |
| ANY `/plugins/{slug}` · ANY `/plugins/{slug}/{*rest}` | plugin `<slug>` grant (GET=view, else=edit); root bypass | Reverse-proxied to the sidecar. Gated by the dedicated plugin branch in the feature guard. |
| GET `/plugins/{slug}/ui` · GET `/plugins/{slug}/ui/{*path}` | public static | Iframe assets served from the plugin's `ui` dir (root-mounted). |
| GET `/plugin-admin` | root | Installed-plugin list (full records, no token). |
| POST `/plugin-admin/install` | root | `{source}` = local path or git URL → installs into the plugins home (disabled). |
| POST `/plugin-admin/{slug}/enable` · POST `/plugin-admin/{slug}/disable` | root | Spawn / stop the sidecar. |
| DELETE `/plugin-admin/{slug}` | root | Stop + unregister (plugin files are kept). |

**Host API** (sidecar-token auth: `Authorization: Bearer $OTTO_PLUGIN_TOKEN`; in
`public_routes`, validated per handler — not user auth):

| Method & path | Returns |
|---|---|
| GET `/plugin-host/repos` | `[{id,name,path,remote_url}]` |
| GET `/plugin-host/jira/accounts` | `[{id,label,base_url,email}]` |
| GET `/plugin-host/jira/credentials?account=<id>` | `{base_url,email,token}` |
| POST `/plugin-host/agents/run` | `{prompt,cwd?,model?}` → `{text}` (claude) |

A sidecar is spawned with env: `OTTO_PLUGIN_SLUG`, `OTTO_PLUGIN_PORT` (it must bind
this), `OTTO_PLUGIN_TOKEN`, `OTTO_HOST_API`, `OTTO_PLUGIN_DATA_DIR`.
- `Capability` ladder: `none` < `view` < `edit` < `admin`.  `Capability::None` is the
  absence of a grant row — never stored; the read returns `"none"` for ungrated features.
- Root ⇒ `capabilities` returns `admin` for all 18 features regardless of stored rows.
- PUT writes a `"grant.changed"` audit entry: `{user_id: actor, target: target_user_id,
  detail: {old: GrantEntry[], new: GrantEntry[]}}`.
- 404 if target user `{id}` does not exist.

## Admin active-sessions overview + terminate (RBAC Task 4.2)

The **sanctioned cross-user view**: a daemon-wide list of every session across
all workspaces and users, plus forced termination. Gated by `Users:Admin`
(feature guard) **or** root — so a non-root user granted `Users:Admin` can use it
too. This intentionally bypasses the per-session owner gate (which everywhere
else confines a user to their own sessions); the handlers add no extra root
check. Both routes are mapped to `Require(Users, Admin)` in the policy table.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /admin/sessions | Users:Admin or root | — | `AdminSessionsResp {sessions: AdminSessionRow[]}` |
| POST /admin/sessions/{id}/terminate | Users:Admin or root | — | `204 No Content` (kills the PTY → `exited`, forcibly evicts attached `/ws/term` viewers; audited) |
| POST /admin/sessions/{id}/remove | Users:Admin or root | — | `204 No Content` (kills the PTY + **deletes** the session row & history, emits `SessionRemoved`; prunes exited/background sessions from the list; audited as `session.removed`) |

- `AdminSessionRow` = `{id, owner_id, owner_username, workspace_id, kind, provider, title, status, live: bool, viewers: number}`.
- Each row is a persisted session enriched with live state from the in-memory
  `SessionManager`: `live` = `is_live(id)`, `viewers` = `attached_count(id)`.
  `owner_username` resolves `created_by` via a single batched user load (falling
  back to the owner id if the user row is gone).
- `terminate` calls `SessionManager::kill_session` (kills the PTY, marks the
  session `exited`, keeps the row + history — non-destructive) then
  `SessionManager::evict`, which fires the per-session disconnect signal so every
  attached `/ws/term` viewer receives a `{"type":"terminated"}` frame and the
  socket closes (see `ws.md`). The session owner can still self-terminate their
  own session via the owner-gated `DELETE /sessions/{id}`.
- Writes a `"session.terminated"` audit entry: `{user_id: actor, target: session_id,
  detail: {owner_id, workspace_id}}`.
- 404 if the session `{id}` does not exist.

## Admin impersonation (act-as, audited; RBAC Task 5.2)

An admin can "act as" another user to see exactly what they see — an
**effective-user overlay**, not a re-login. `start` mints a short-lived
impersonation token whose owner is the admin (the **real** user) and whose
`acting_as_user_id` is the target (the **effective** user). `authenticate`
resolves it to `AuthContext{real_user: admin, effective_user: target}`, so **every
authorization decision runs against the target** while **every audit entry records
the admin**. The UI swaps its bearer to the returned token; `stop` revokes it and
the UI restores the admin's own token.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| POST /admin/impersonate/{user_id} | Users:Admin or root | — | `ImpersonateResp {token}` (audited `impersonate.start`) |
| POST /admin/impersonate/stop | the impersonating session (self-scoped) | — | `204 No Content` (revokes the presented token; audited `impersonate.stop`) |

- `ImpersonateResp` = `{token}` — the raw impersonation token, returned **exactly
  once** (only its hash is stored). Short fixed TTL (30 min); the expiry is never
  slid, so the overlay always times out predictably.
- `start` is gated `Users:Admin`/root by the policy table. The handler then
  enforces the **anti-escalation guardrails** (403 on violation):
  1. **No up/sideways:** the target may not be root, nor hold `Users:Admin`
     (can't impersonate root or a fellow Users-admin).
  2. **No nesting:** an impersonation token (real ≠ effective) may not start
     another impersonation.
  3. **No self:** the target may not be the caller (404 if the target is absent;
     403 if disabled).
  4. **Impersonation cannot mint PATs:** `POST /auth/tokens` is rejected (403)
     when the request is impersonated (real ≠ effective) — an admin acting-as a
     user can't forge a long-lived credential as that user. (The same guard will
     later cover share-link minting.)
- `stop` is **self-scoped** (`Exempt` in the policy table, like `/auth/logout`) —
  the effective user mid-impersonation is a plain user, so it cannot be
  `Users:Admin`-gated or "Exit" would be impossible. It revokes the *presented*
  token. After `stop`, that token returns `401`.
- Audit: `impersonate.start` = `{user_id: admin (real), target: target_id
  (effective), detail: {real_user_id, effective_user_id, effective_username}}`;
  `impersonate.stop` = `{user_id: real, target: effective, detail: {real_user_id,
  effective_user_id}}`.

## Trust & Safety (security audit log + posture)

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /audit-log | root | query: `from?` `to?` (RFC3339, inclusive `ts` bounds) · `action?` · `user_id?` · `limit?` (≤500, default 100) · `offset?` | AuditLogResp `{entries: AuditEntry[], total}` (newest first; `total` ignores paging) |
| GET /security-posture | root | — | SecurityPostureResp `{network_listener, network_listener_port?, loopback_only, active_api_tokens}` |

The audit log is an **append-only** ledger written best-effort by the daemon at security-relevant sites — it is never updated or deleted, and an audit-insert failure never fails the audited request. `AuditEntry` = `{id, ts, user_id?, action, target?, detail?, ip?}` where `action` is a stable snake_case verb. Wired actions today: `login.success`, `login.failure`, `login.lockout` (`user_id` null — the actor is unauthenticated; `target` = attempted username; `ip` = real socket peer), `token.mint` / `token.revoke` (`target` = token id), `settings.change` (`target` = changed key list; `detail.keys`; secret values are NOT captured), `network_listener.toggle` (`target` = `on`/`off`; `detail` = the new listener config), `db.write_confirmed` (a confirmed write on a guarded production/read-only connection; `target` = connection name; `detail.environment` + truncated `detail.statement`), `grant.changed` (`target` = the user whose grants changed; `detail.old`/`detail.new` grant lists), `session.terminated` (an admin force-terminated a session via `POST /admin/sessions/{id}/terminate`; `target` = session id; `detail.owner_id` + `detail.workspace_id`), and `impersonate.start` / `impersonate.stop` (an admin began / ended acting-as another user; `user_id` = the real admin, `target` = the effective/impersonated user, `detail.real_user_id` + `detail.effective_user_id`). The posture summary derives entirely from existing settings + the auth store (no new state): the network listener key drives `network_listener` / `network_listener_port` / `loopback_only`, and `active_api_tokens` counts unexpired API tokens instance-wide.

## Usage tracking & system metrics (embedded ClickHouse)

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /usage/status | root | — | engine status (installed/available) |
| GET /usage/summary | root | — | token/cost breakdown (input/output + cache read/write) |
| GET /usage/metrics | root | — | system CPU/RAM metrics |
| PUT /usage/config | root | UsageConfig | config |
| POST /usage/install | root | — | install the embedded ClickHouse binary |
| GET /usage/budgets | root | — | UsageBudgetStatus (caps + live spend; enforcement opt-in, default off) |
| PUT /usage/budgets | root | UsageBudgetConfig | UsageBudgetStatus (replace + persist budget config) |

## Insights (scheduled usage reports)

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /insights/config | root | — | insights scheduler config (daily/weekly/monthly) |
| PUT /insights/config | root | InsightsConfig | config |
| GET /insights/reports | root | — | generated report list |
| GET /insights/report | root | — | one report's HTML |
| POST /insights/run | root | `{ period, offset? }` | `{ started, run_id?, reason? }` — `run_id` when started; `reason` when not (e.g. skill not installed) |

## LSP (language server bridge)

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /lsp/capabilities | member | — | supported languages/servers |
| POST /workspaces/{id}/lsp/install | ws editor | InstallServersReq | install language servers |

## Provider registry update

| Method & path | Auth | Request | Response |
|---|---|---|---|
| POST /workspaces/{id}/providers/update | ws editor | UpdateProvidersReq | update agent CLI providers for the workspace |

## Filesystem & logs (operator tools)

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /fs/browse?path= | member | — | directory listing (for path pickers) |
| GET /fs/read?path= | member | — | file contents |
| GET /logs/daemon | root | — | recent daemon log lines |

## PR-review config

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /settings/pr-review | root | — | ReviewConfig |
| PUT /settings/pr-review | root | ReviewConfig | config |
| GET /settings/pr-review/presets | member (Git View) | — | ReviewConfigPreset[] |
| PUT /settings/pr-review/presets | root | ReviewConfigPreset[] | ReviewConfigPreset[] (full replace; ids must be unique + non-empty) |
| GET /repos/{id}/review-config | ws viewer | — | RepoReviewConfigResp |
| PUT /repos/{id}/review-config | ws editor | RepoReviewBinding | RepoReviewConfigResp |
| DELETE /repos/{id}/review-config | ws editor | — | RepoReviewConfigResp (reverted to global) |

**Named presets + per-repo binding.** A `ReviewConfigPreset` is `{ id, name, config: ReviewConfig }` — a reusable full review configuration. A repo may bind one of them (`RepoReviewBinding { preset_id }`) or carry a fully custom inline config (`RepoReviewBinding { config }`; inline wins if both are set). Review runs resolve the effective config as **repo inline > repo preset > global `pr_review`**; a dangling `preset_id` (preset deleted) falls back to global rather than failing the run. `RepoReviewConfigResp` is `{ scope: "global"|"preset"|"custom", preset_id?, preset_name?, config }` where `config` is the EFFECTIVE post-resolution config (`preset_name` is null for a dangling reference). Bindings are stored in the settings table under `pr_review_repo:<repo_id>`; presets under `pr_review_presets`. A per-call `cfg_override` (workflow `review_run` steps) still wins over everything.

**`ReviewConfig` DTO additions (A2 — additive, optional):**
- `max_attempts?: number | null` — max total agent attempts per run (default 3); overrides the compiled-in constant.
- `timeout_secs?: number | null` — per-agent timeout in seconds; overrides the diff-size heuristic when set.

**`Review` DTO additions (A2 — additive, optional):**
- `verdict?: "approved" | "changes_requested" | "needs_review" | null`
- `blocker_count?: number | null` — count of bug-severity draft comments (merge-readiness gate).
- `summary_md?: string | null` — short markdown summary of findings.

**`FileDiff` DTO additions (A2 — additive, optional):**
- `too_large?: boolean | null` — true when the file diff was capped server-side (cap = 200 KB rendered text).
- `added?: number | null` / `deleted?: number | null` — line counts for merge-readiness display.
- `language?: string | null` — detected language hint for syntax highlighting.

**`PrSummary` DTO additions (A2 — additive, optional):**
- `draft?: boolean | null` — true for draft PRs (GitHub only currently).
- `ci_status?: string | null` — simplified CI status: `"passing" | "failing" | "pending" | "unknown"`.
- `labels?: string[]` — PR label names.

**`review_findings` table (migration 0049):** fingerprinted persistent finding identity across runs; `review_merge_readiness` view aggregates blocker counts per (repo_id, pr_number). No new HTTP routes — queried internally by the summarizer and surfaced via the `Review` DTO fields above.

## Swarm lifecycle (explicit paths for #84)

Frozen #84 lists the four lifecycle actions as a single combined row; the daemon registers
them as four distinct routes. Each takes no body and returns the updated `Swarm`.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| POST /workspaces/{id}/swarm/swarms/{sid}/start | ws editor | — | Swarm (start/restart the Coordinator) |
| POST /workspaces/{id}/swarm/swarms/{sid}/pause | ws editor | — | Swarm (pause new turns; suspend idle sessions) |
| POST /workspaces/{id}/swarm/swarms/{sid}/abort | ws editor | — | Swarm (cancel runs; kill swarm sessions) |
| POST /workspaces/{id}/swarm/swarms/{sid}/resume | ws editor | — | Swarm (resume from paused) |

## Swarm goals, verification & channel triggers (additive, continues #86)

Additive to the frozen swarm block (#59–#86); these are NOT renumbered against the
frozen #1–#89 core. Reads = `ws viewer`, writes = `ws editor`. JSON snake_case, ULID ids,
RFC3339 timestamps, `Problem{code,message}` errors. The workspace is resolved from the
parent row (task/project/swarm/goal). Goal-status changes also arrive live over
`/ws/events` as `swarm_goal_updated` (see `ws.md`).

**Goals.** A `SwarmGoal` is a verifiable success criterion attached to a task or project
(`kind:"explicit"`) or a swarm-level template applied to every task (`kind:"standing"`). It
carries an optional `metric`/`comparator`(`lte|gte|eq|contains|absent`)/`target_value`/
`block_value`, an optional `verify_cmd`, a `max_retries` budget, a `blocking` flag, a
lifecycle `status` (`pending|verifying|passed|warned|unmet|skipped|error`), the verifier's
`verdict` (`{target_met,blocker,severity,measured,summary,findings[]}`), and `iterations`.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /api/v1/swarm/tasks/{tid}/goals | ws viewer | — | `SwarmGoal[]` |
| GET /api/v1/swarm/projects/{pid}/goals | ws viewer | — | `SwarmGoal[]` |
| POST /api/v1/swarm/tasks/{tid}/goals | ws editor | CreateGoalReq | SwarmGoal |
| POST /api/v1/swarm/projects/{pid}/goals | ws editor | CreateGoalReq | SwarmGoal |
| PATCH /api/v1/swarm/goals/{gid} | ws editor | UpdateGoalReq (all fields optional) | SwarmGoal |
| DELETE /api/v1/swarm/goals/{gid} | ws editor | — | `{}` |
| GET /api/v1/swarm/swarms/{sid}/standing-goals | ws viewer | — | `SwarmGoal[]` (swarm-level templates; seeded on first GET) |
| PUT /api/v1/swarm/swarms/{sid}/standing-goals | ws editor | `{ goals: CreateGoalReq[] }` | `SwarmGoal[]` (replaces the set) |

`CreateGoalReq` = `{ title, description?, metric?, comparator?, target_value?, block_value?,
verify_cmd?, max_retries?, blocking?, order_idx? }`. `UpdateGoalReq` = the same with every
field optional.

**Verification.** Run goal verification on demand for a task (the Coordinator measures each
goal and records a verdict, flipping the task to `verifying` while it runs).

| Method & path | Auth | Request | Response |
|---|---|---|---|
| POST /api/v1/swarm/tasks/{tid}/verify | ws editor | — | `{ started: bool, reason?: string }` |
| POST /api/v1/swarm/tasks/{tid}/verify/stop | ws editor | — | `{ stopped: bool }` |
| GET /api/v1/swarm/tasks/{tid}/verification | ws viewer | — | `{ running: bool, task_status: string, goals: SwarmGoal[] }` |

**Channel triggers.** A `SwarmChannelTrigger` auto-launches swarm work when a matching
message arrives on a channel: `{ id, swarm_id, workspace_id, channel("slack"|"telegram"|
"webhook"), match_chat, keyword, repo_path?, auto_start, reply, enabled, created_by,
created_at, updated_at }`.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /api/v1/swarm/swarms/{sid}/triggers | ws viewer | — | `SwarmChannelTrigger[]` |
| POST /api/v1/swarm/swarms/{sid}/triggers | ws editor | CreateTriggerReq | SwarmChannelTrigger |
| PATCH /api/v1/swarm/triggers/{tid} | ws editor | UpdateTriggerReq | SwarmChannelTrigger |
| DELETE /api/v1/swarm/triggers/{tid} | ws editor | — | `{}` |

`CreateTriggerReq` = `{ channel, match_chat?, keyword?, repo_path?, auto_start?, reply?,
enabled? }`. `UpdateTriggerReq` = the same with every field optional.

**Project & team skills (ride existing routes).** Project-scoped skills travel on the
existing `PATCH /api/v1/swarm/projects/{pid}` (#72) as a top-level `skills` array on
`UpdateProjectReq`; team-wide skills travel on `PATCH /api/v1/swarm/swarms/{sid}` (#62) as a
`skills` array nested inside `config`. `SwarmProject` additionally surfaces
`integration_branch?`, `origin_channel?`, `origin_chat?`, `origin_thread?` (set when a
project was launched from a channel trigger).

## Root-level routers (NOT under /api/v1; `?token=` auth)

These self-authenticate via the `?token=` query parameter and are merged at the server root
(not under the `/api/v1` nest). The two terminal/event WebSockets are specified in detail in
`ws.md`.

| Method & path | Auth | Purpose |
|---|---|---|
| GET /ws/term/{session_id} | `?token=`; ws viewer attach, editor input | terminal stream (see ws.md) |
| GET /ws/events | `Sec-WebSocket-Protocol: otto-bearer, <token>` (preferred — keeps the token out of the URL) or `?token=` fallback; member | daemon event stream (see ws.md) |
| GET /ws/lsp?lang=&root=&token= | `?token=`; ws editor | LSP WebSocket bridge |
| GET /ws/api-client/stream?token= | `?token=`; ws editor | API-client streaming-response bridge |
| GET /browser/proxy?url=&token= | `?token=` | in-app browser HTTP proxy |

## Ingest (per-session token, unauthenticated by bearer)

These are reachable without a user bearer token; each is gated by the per-session ingest
token Otto sets on the agent PTY (`X-Otto-Session` + `X-Otto-Token`), verified inside the
handler. Agent hooks (which have no user session) post to them.

| Method & path | Gate | Request | Response |
|---|---|---|---|
| POST /ingest/claude | session token | Claude hook event | 204 |
| POST /ingest/codex | session token | Codex hook event | 204 |
| POST /ingest/usage | session token | token-usage event | 204 |
| POST /ingest/swarm/board | session token | `{kind?,to_agent_id?,body}` | 204 (also listed at #—, swarm) |
| POST /ingest/swarm/product | session token | `{title?,body_md}` | 204 (also listed at #—, swarm) |
| POST /ingest/swarm/mockup | session token | `{title,format,content}` | 204 (also listed at #—, swarm) |
| POST /ingest/swarm/discovery-report | session token | `{report_md}` | 204 (also listed at #—, swarm) |

Notes:
- The `/api/v1` public exemptions (no bearer required) are exactly: `/health`, `/meta`,
  `/onboarding/root`, `/auth/login`, and the `/ingest/*` routes (session-token gated).
- `kill_all_sessions` (`POST /app/kill-sessions`) is mounted in the sessions api_router, so
  its full path is `/api/v1/app/kill-sessions` and it requires a bearer token.
- Several AI-producing routes (analyze/rewrite/generate/plan/review) return `202 Accepted`
  and stream progress over `/ws/events`; poll the corresponding GET for the latest state.

## Memory layer (workspace-scoped knowledge store)

A workspace-scoped store of distilled knowledge (`item`) and raw evidence (`chunk`) with
keyword (FTS5) recall. Reads require `ws viewer`, mutations `ws editor`. `Memory`,
`NewMemory`, `MemoryPatch`, `MemoryQuery`, `MemoryHit`, `RecallBrief`, `MemoryLink`,
`GraphData` are mirrored in `ui/src/lib/api/types.ts`.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /workspaces/{ws}/memories | ws viewer | query: `collection?,kind?,story_id?,tag?,include_inactive?,limit?` | `Memory[]` |
| POST /workspaces/{ws}/memories | ws editor | `NewMemory` | `Memory` (exact-dup save is a NOOP returning the existing row) |
| GET /workspaces/{ws}/memories/{id} | ws viewer | — | `Memory` |
| PATCH /workspaces/{ws}/memories/{id} | ws editor | `MemoryPatch` | `Memory` |
| DELETE /workspaces/{ws}/memories/{id} | ws editor | — | 204 (soft-delete: `active=false`) |
| GET /workspaces/{ws}/memories/{id}/links | ws viewer | — | `MemoryLink[]` |
| POST /workspaces/{ws}/memory/search | ws viewer | `MemoryQuery` | `MemoryHit[]` (keyword FTS5 → LIKE fallback, re-ranked) |
| POST /workspaces/{ws}/memory/recall | ws viewer | `{story_id, focus?, token_budget?}` | `RecallBrief` (token-budgeted background brief) |
| GET /workspaces/{ws}/memory/graph | ws viewer | query: `collection?` | `GraphData{nodes,edges}` (memory link graph) |
| POST /workspaces/{ws}/memory/ingest-text | ws editor | `{collection?, path, content}` | `{chunks}` (chunk text into a collection) |
| POST /workspaces/{ws}/memory/import-graph | ws editor | `{collection?, graph:{nodes,edges}}` | `ImportStats{nodes,edges}` (graphify graph.json) |
| GET /workspaces/{ws}/memory/entities/{id}/graph | ws viewer | — | `{links, neighbors}` (entity neighborhood) |
| POST /workspaces/{ws}/product/stories/{sid}/memory/ingest | ws editor | — | `{ingested}` (extract a story's artifacts into memory) |

Notes:
- `MemoryQuery.mode` ∈ `{hybrid (default), semantic, keyword}` — ALL execute the
  keyword path since Vault v3 removed embeddings; the legacy values remain accepted
  aliases so existing callers keep working. `k` defaults to 20.
- `MemoryHit` carries `reasons: ContextReason[]` (`{kind, detail, score}`,
  `kind ∈ {keyword, scope}`) alongside `why: string[]`.
- `visibility` ∈ `{shared (default — all workspace members), private (creator-only)}`.
- Sharing across machines: set `OTTO_MEMORY_REMOTE_URL`/`OTTO_MEMORY_REMOTE_TOKEN`
  to point an instance at a shared host, or sync an `OTTO_MEMORY_VAULT_DIR` vault
  folder (git) and re-index. A shared SQLite *file* over a network is unsupported.

## Vault v3 — the docs home (file-backed markdown vaults, OKF)

A **vault** is a registered local directory of markdown files (it may be a live
Obsidian vault). Files are the source of truth; SQLite holds a derived, rebuildable
index (notes, wikilinks/markdown links, tags, FTS5). No embeddings anywhere.
Vaults are a **GLOBAL library** (like connections): every workspace sees every
vault — the `{ws}` in the path is auth context only (`ws_id` on the row is
provenance, not a boundary; `root_path` is globally unique, app-enforced).
Roles: reads = ws viewer (`Product:View`), writes = ws editor
(`Product:Edit`); the read-shaped POSTs `search` and `okf/validate` are View-gated.
DTOs (`Vault`, `VaultStatus`, `VaultDirListing`, `VaultNote`, `VaultNoteMeta`,
`VaultBacklink`, `VaultSearchHit`, `VaultSwitchHit`, `VaultTagCount`,
`VaultGraphPayload`, `VaultRenameResult`, `OkfReport`) are mirrored in
`ui/src/lib/api/types.ts`.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /workspaces/{ws}/vault/vaults | ws viewer | — | `Vault[]` (with note/link counts + scan_state) |
| POST /workspaces/{ws}/vault/vaults | ws editor | `{name, root_path?, okf?}` | `Vault` — registers an existing dir (a non-existent `root_path` is created — Obsidian "create vault" behavior); omitted `root_path` creates `~/.otto/vault/<slug(name)>`. Kicks a full scan. |
| PATCH /workspaces/{ws}/vault/vaults/{id} | ws editor | `{name?, okf?}` | `Vault` |
| DELETE /workspaces/{ws}/vault/vaults/{id} | ws editor | — | 204 — unregister ONLY (files on disk untouched) |
| POST /workspaces/{ws}/vault/vaults/{id}/rescan | ws editor | — | `VaultStatus` — full incremental rescan (awaited) |
| GET /workspaces/{ws}/vault/vaults/{id}/status | ws viewer | — | `VaultStatus{scan_state, notes, links, unresolved, tags, attachments}`; stale (>5s) probes kick a background incremental scan |
| GET /workspaces/{ws}/vault/vaults/{id}/dir | ws viewer | `?path=` | `VaultDirListing` — one level: dirs (with child counts), notes, attachments |
| GET /workspaces/{ws}/vault/vaults/{id}/note | ws viewer | `?path=` | `VaultNote{meta, raw, outgoing}` |
| PUT /workspaces/{ws}/vault/vaults/{id}/note | ws editor | `{path, content, if_hash?}` | `VaultNoteMeta` — create/update; parent folders auto-created; `if_hash` mismatch → 409 (optimistic concurrency; `""` = must-not-exist) |
| PUT /workspaces/{ws}/vault/vaults/{id}/file | ws editor | `{path, content, if_hash?}` | `{path,size,hash}` — create/update a guarded UTF-8 documentation artifact (`.yaml/.yml/.json/.d2/.mmd/.txt/.csv`, max 4 MiB); parent folders auto-created; same optimistic-concurrency, traversal, hidden-segment, and symlink-escape guards as note writes. Markdown stays on `/note`; binary files are rejected. |
| DELETE /workspaces/{ws}/vault/vaults/{id}/note | ws editor | `?path=` | 204 — soft delete → `<vault>/.trash/` (never destroys files) |
| POST /workspaces/{ws}/vault/vaults/{id}/rename | ws editor | `{from, to}` | `VaultRenameResult{links_updated}` — file OR folder move; rewrites every referencing wikilink/markdown link across the vault on disk (style-preserving); case-only renames use a two-step move |
| POST /workspaces/{ws}/vault/vaults/{id}/folder | ws editor | `{path}` | 204 |
| GET /workspaces/{ws}/vault/vaults/{id}/backlinks | ws viewer | `?path=` | `VaultBacklink[]` (linked mentions with a context snippet) |
| POST /workspaces/{ws}/vault/vaults/{id}/search | ws viewer | `{query, tag?, path_prefix?, okf_type?, limit?}` | `VaultSearchHit[]` — FTS5 bm25 + snippets; `tag:`/`path:`/`type:` operators inside `query` |
| GET /workspaces/{ws}/vault/vaults/{id}/switcher | ws viewer | `?q=` | `VaultSwitchHit[]` — server-side fuzzy over title/aliases/path (quick switcher + `[[` completion) |
| GET /workspaces/{ws}/vault/vaults/{id}/tags | ws viewer | — | `VaultTagCount[]` |
| GET /workspaces/{ws}/vault/vaults/{id}/graph | ws viewer | `?mode=full\|local&path=&depth=&tags=&orphans=&reserved=&ghosts=&edge_budget=` | `VaultGraphPayload` — compact parallel arrays (`paths,titles,types,type_labels,services,service_labels,tag_off,tag_ids,tag_labels,flags,edges`) with a flat `[src,dst,…]` edge index list; `flags` bits: 1=ghost, 2=tag, 4=reserved; `types`/`services` are label-table indices per node (types are case-folded, so `Flow`/`flow` share one bucket; `untyped`/`unresolved`/`tag` are synthetic buckets), tags are CSR-encoded (`tag_ids[tag_off[i]..tag_off[i+1]]`, `tag_off` has `n+1` entries) and load regardless of `tags`, which only controls whether tag NODES are drawn. The client filters, rolls up and colors on these attributes without refetching. Full mode enforces a degree-prioritized edge budget (default 2M) with `truncated` |
| POST /workspaces/{ws}/vault/vaults/{id}/okf/validate | ws viewer | — | `OkfReport{conformant, errors[], warnings[], checked_notes}` — deterministic OKF v0.1 conformance: E1 no/unparseable frontmatter, E2 missing `type`, E3 reserved-file structure; W1 title/description, W2 broken link, W3 timestamp, W4 dir missing index.md, W5 log dates |
| POST /workspaces/{ws}/vault/vaults/{id}/okf/indexes | ws editor | — | `{written}` — regenerate per-directory `index.md` files (frontmatter descriptions; root index carries `okf_version`) |
| GET /workspaces/{ws}/vault/vaults/{id}/asset | ws viewer | `?path=` | attachment bytes with sniffed content type (traversal-guarded) |

Notes:
- Every file op canonicalizes and guards paths (no `..`, absolutes, hidden or
  `.trash/` segments; symlink escapes rejected).
- Wikilink resolution follows Obsidian shortest-path rules: exact relative →
  vault-root-relative → unique basename (case-insensitive, `.md` optional);
  OKF `/`-bundle-absolute links also resolve. Ambiguous basenames stay
  UNRESOLVED (surfaced, never silently picked). Broken links are legal (OKF).
- `index.md`/`log.md` are OKF reserved files: flagged `reserved`, excluded from
  the switcher and (by default) the graph.
- Notes >4 MiB are indexed metadata-only (no FTS body).

### Docs agents (AI writers + optional iterative reviewers)

Launch 1–4 writer agents (per-agent provider/model) as managed sessions that
write into a vault through the session-injected Otto MCP tools. The sessions
are background/embedded (`meta.source:"vault-docs"`): they appear only as
inline terminals in the Vault run panel. A single writer writes finals directly
into `target_dir`; with >1 writers, each writes under
`_drafts/docs-run-<run8>/agent-<n>/` and a summarizer consolidates the drafts,
after which the server soft-moves the draft tree to `<vault>/.trash/` and
rescans.

Run state is live in memory and write-through mirrored to SQLite
(`vault_docs_runs`) at every transition. Each refine turn is also recorded as a
`kind:"refine"` run. On startup, any row still
`running|summarizing|reviewing|revising` and every active nested slot become
`interrupted`; orphaned multi-writer drafts are soft-trashed. Cancel stops
orchestration and terminates active writer, summarizer, reviewer, and revision
sessions after a Ctrl+C grace period. The detached refine task cannot be
stranded by a client disconnect, and its per-note session is rehydrated from
history after restart. `written` comes from the final author's JSON result,
with a server-side before/after note-path diff as fallback.

Requested author and reviewer skills are staged as **complete packages** per
run (Library first, then the operator's global skill, then the compiled-in
bundle). The tree exposes both `.claude/skills/<name>/` and provider-neutral
`skills/<name>/`, preserving references, scripts, examples, assets, and evals.
Claude receives the root via `meta.extra_dirs`; other providers receive exact
package paths and file manifests. If one package cannot be staged, only that
package falls back to its `SKILL.md` text. OKF vaults auto-add
`okf-authoring`; prepared repo scans add `vault-repo-docs`.

An optional independent review gate starts after the final author finishes.
One to four reviewers run in parallel per round. Reviewers are read-only Vault
sessions (`meta.source:"vault-docs-review"`): both the MCP catalog and
dispatcher deny `otto_vault_write`, `otto_vault_write_file`,
`otto_vault_rename`, and `otto_vault_delete`. Each reviewer must emit a
structured JSON finding array; missing or malformed output is an error, never a
clean verdict. All reviewers returning `[]` in the same round finishes the run
`done`. Otherwise the same final-author session repairs the bundle and the
cycle repeats. Exhausting the configured limit finishes
`done_with_findings`. A reviewer/revision failure remains visibly `error` and
pauses the same run for targeted retry or cancel; completed peers, findings,
and docs are preserved.

Optional request block (omission skips review):

```json
{
  "review": {
    "max_iterations": 3,
    "reviewers": [
      {
        "provider": "claude",
        "model": "sonnet",
        "skill": "vault-api-review",
        "focus": "Prioritize externally consumed contracts"
      }
    ]
  }
}
```

`reviewers` is required and contains 1–4 rows. `max_iterations` defaults to 3
and accepts 1–10. `skill` defaults to `vault-docs-review` and must be one of
`vault-docs-review`, `vault-api-review`, `vault-data-review`,
`vault-runtime-review`, or `vault-evidence-review`. Optional `focus` adds a
lens without removing the selected method's mandatory checks.

The response/persisted DTOs are mirrored in `ui/src/lib/api/types.ts`:

- `VaultDocsRun.review = {state,max_iterations,current_iteration,outcome,
  reviewers,rounds}`. Top-level `reviewers` are the immutable resolved templates;
  live session/state/findings exist only in each round, preventing contradictory
  duplicate snapshots. Old rows deserialize as `state:"skipped"`.
- Each round is `{iteration,state,reviewers,revision}`. Reviewers carry
  `{index,provider,model,skill,focus,state,session_id,findings,error}`;
  revisions carry `{state,session_id,changed_paths,error}`.
- A finding is `{severity:"blocking|major|minor",category,summary,evidence[],
  missed_item,required_fix}`. Evidence is
  `{repo_path?,line?,doc_path?,section?}` and must identify source or bundle
  proof.
- Run states: `running|summarizing|reviewing|revising|done|
  done_with_findings|error|cancelled|interrupted`. Review states:
  `skipped|pending|reviewing|revising|clean|exhausted|error|cancelled|
  interrupted`. Round states additionally use `revised` for a repaired
  historical round. Reviewer states: `pending|running|done|error|cancelled|
  interrupted`; revision states also allow `skipped`.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| POST /workspaces/{ws}/vault/vaults/{id}/docs-agents/run | ws editor | `{prompt, target_dir?, agents: [{provider, model?}] (1..=4), summarizer?: {provider?, model?}, skills?: string[], review?: {reviewers: [{provider, model?, skill?, focus?}] (1..=4), max_iterations?: 1..=10}}` | `VaultDocsRun` — initial `state:"running"`; `summarizer.state:"skipped"` with one writer; the durable row exists before return. `skills` (≤4, name-validated) are complete packages staged for every provider. Review is optional and starts after authoring. |
| GET /workspaces/{ws}/vault/vaults/{id}/docs-agents/runs | ws viewer | `?limit=` (default 50, cap 200) | `VaultDocsRun[]` — docs + refine runs newest-first; live runs carry their fresher in-memory snapshot. |
| GET /vault/docs-agents/runs/{run_id} | ws viewer (the run's ws, re-checked) | — | `VaultDocsRun` — the UI polls every 1500ms while `running\|summarizing\|reviewing\|revising`; durable history is used when not live. |
| POST /vault/docs-agents/runs/{run_id}/agents/{index}/retry | ws editor (the run's ws, re-checked) | — | `202` — retries one writer with a fresh session: a `running`/`pending` slot is killed and re-spawned; an `error` slot stays retryable while the run is `running` (its loop keeps listening as long as any peer writer is still moving). `409` for a terminal run or non-retryable slot; max 5 user retries per slot. A writer whose CLI process dies mid-turn gets ONE automatic respawn before surfacing the error. |
| POST /vault/docs-agents/runs/{run_id}/summarizer/retry | ws editor (the run's ws, re-checked) | — | `202` — same for the active summarizer; failed consolidation preserves its drafts. |
| POST /vault/docs-agents/runs/{run_id}/review/rounds/{iteration}/reviewers/{index}/retry | ws editor (the run's ws, re-checked) | — | `202` — retries only the active `pending\|running\|error` reviewer in the current `reviewing` round. Completed peers/findings/docs remain. `409` for a stale round, wrong stage, non-retryable slot, or terminal run; max 5 user retries per slot, then terminal `error`. |
| POST /vault/docs-agents/runs/{run_id}/review/rounds/{iteration}/revision/retry | ws editor (the run's ws, re-checked) | — | `202` — retries only the active `pending\|running\|error` final-author revision in the current `revising` round; same preservation, conflict, and retry-cap semantics. |
| POST /vault/docs-agents/runs/{run_id}/cancel | ws editor (the run's ws, re-checked) | — | 204 — marks the run/review/current round and active nested slots `cancelled`, stops orchestration, and terminates active sessions; finished slots keep their results. 404 once terminal. |
| POST /vault/docs-agents/runs/{run_id}/resolve | ws editor (the run's ws, re-checked) | `{outcome: "ok"\|"fixed"}` | `VaultDocsRun` — user disposition for a `done_with_findings` run: flips it to `done` durably and stamps `review.outcome` `resolved_ok`/`resolved_fixed`. `409` for any other state; `400` for an unknown outcome. |
| DELETE /vault/docs-agents/runs/{run_id} | ws editor (the run's ws, re-checked) | — | `204` — history cleanup: drops one TERMINAL run's durable row (and any lingering registry snapshot). `409` while the run is active (cancel first). |
| POST /workspaces/{ws}/vault/vaults/{id}/docs-agents/refine | ws editor | `{path, prompt, provider?, model?}` | `{session_id, reply}` — long request; one resumed session per (vault, note), rehydrated after restart. An explicit `provider` DIFFERENT from the bound session's starts a FRESH session (rebinds the note); same/omitted resumes. |
| GET /workspaces/{ws}/vault/vaults/{id}/docs-agents/refine-session | ws viewer | `?path=` | `{session_id: string\|null, running: boolean}` — poll after posting refine to attach the live shell. |
| DELETE /workspaces/{ws}/vault/vaults/{id}/docs-agents/refine-session | ws editor | `?path=` | `{session_id: null, running: false}` — detach the note's refine session (the old session stays in the sessions list). Writes a tombstone so rehydration doesn't resurrect the binding; the next refine POST starts a fresh agent with any provider. |

## Message Brokers (Kafka viewer)

A Conduktor/Confluent-class Kafka viewer: cluster connection profiles, cluster
overview, topics (browse / peek / produce / configs), consumer groups + lag, broker
CPU/RAM + throughput metrics, and a Schema Registry browser. DTOs live in
`crates/otto-brokers/src/types.rs`, mirrored in `ui/src/lib/api/types.ts`. Reads
require `ws viewer`; cluster management + mutations require `ws editor` (global
clusters: root). Mutations on a guarded cluster (`environment=prod` or `read_only`)
require `confirm=true` (403 otherwise). Cluster secrets (SASL / schema-registry
passwords) are stored in the Keychain — only `has_*_password` flags are ever
returned. `BrokerCluster.workspace_id=null` = global profile.

A cluster may carry an optional `ssh` tunnel (`SshTunnelConfig`:
`{ host, port?, user, identity_file? }`, key/agent auth only) to reach a private
cluster (e.g. AWS MSK in a VPC) through a bastion. When set, the daemon opens one
`ssh -D` SOCKS5 tunnel and runs an in-process Kafka-aware proxy (librdkafka has no
SOCKS support and can't override advertised broker addresses): librdkafka talks
plaintext to a local proxy that dials brokers via SOCKS, terminates TLS to the
broker, and rewrites the broker addresses in `Metadata`/`FindCoordinator`
responses. The Schema Registry + metrics endpoints ride the same SOCKS tunnel. On
`UpsertClusterReq`, `ssh` follows the same PATCH rule as passwords: absent = keep,
`null` = clear, object = set.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /workspaces/{wid}/brokers/clusters | ws viewer | — | `BrokerCluster[]` (workspace + global) |
| POST /workspaces/{wid}/brokers/clusters | ws editor | `UpsertClusterReq` | `BrokerCluster` (201) |
| GET /brokers/clusters/{id} | ws viewer | — | `BrokerCluster` |
| PATCH /brokers/clusters/{id} | ws editor | `UpsertClusterReq` (absent `*_password`/`ssh`=keep, ``/`null`=clear; absent `environment`/`read_only` preserve the guard) | `BrokerCluster` |
| DELETE /brokers/clusters/{id} | ws editor | — | 204 (deletes Keychain secrets too) |
| POST /brokers/clusters/{id}/test | ws editor | — | `TestClusterResp` (never 5xx — `ok:false` carries the error) |
| GET /brokers/clusters/{id}/overview | ws viewer | — | `ClusterOverview` |
| GET /brokers/clusters/{id}/metrics | ws viewer | — | `ClusterMetrics` (throughput sampled per call; broker CPU/RAM when `metrics_url` set) |
| GET /brokers/clusters/{id}/topics | ws viewer | — | `TopicSummary[]` |
| POST /brokers/clusters/{id}/topics | ws editor | `CreateTopicReq` | `TopicSummary` (201; 409 if exists) |
| GET /brokers/clusters/{id}/topics/{topic} | ws viewer | — | `TopicDetail` |
| GET /brokers/clusters/{id}/topics/{topic}/stats | ws viewer | — | `TopicStats` (lazy `message_count` + `cleanup_policy`; the topic list is metadata-only for speed) |
| POST /brokers/clusters/{id}/topics/stats | ws viewer | `BatchStatsReq {names: string[]}` (≤500) | `Record<string, TopicStats>` (bulk load via `WATERMARK_WORKERS` thread pool; replaces N×1 GET calls from topics table) |
| DELETE /brokers/clusters/{id}/topics/{topic}?confirm=B | ws editor | — | 204 |
| GET /brokers/clusters/{id}/topics/{topic}/configs | ws viewer | — | `TopicConfigEntry[]` |
| PUT /brokers/clusters/{id}/topics/{topic}/configs | ws editor | `AlterConfigsReq` | `TopicConfigEntry[]` (merges over existing dynamic overrides) |
| POST /brokers/clusters/{id}/topics/{topic}/consume | ws viewer | `ConsumeReq` | `ConsumeResp` (peek; key/value decoded per `decode`) |
| POST /brokers/clusters/{id}/topics/{topic}/produce | ws editor | `ProduceReq` | `ProduceResp` |
| GET /brokers/clusters/{id}/groups | ws viewer | — | `GroupSummary[]` |
| GET /brokers/clusters/{id}/groups/{group} | ws viewer | — | `GroupDetail` (members + per-partition lag) |
| POST /brokers/clusters/{id}/groups/{group}/reset | ws editor | `GroupResetReq` | `GroupDetail` (updated detail after reset; 403 if guarded + `confirm≠true`) |
| GET /brokers/clusters/{id}/schema-registry/subjects | ws viewer | — | `SchemaSubject[]` (400 if no registry configured) |

Clusters carry an optional `section_id` (sidebar folder; `null`=ungrouped, global clusters always ungrouped); on `UpsertClusterReq` it follows the same PATCH rule as `ssh` (absent=keep, `null`=ungroup, id=set). Since the unified Connections hub, `section_id` points into the SHARED `connection_sections` tree (`/workspaces/{id}/connection-sections`) — the old `/brokers/cluster-sections*` endpoints are gone (migration 0095 merged those rows in, ids preserved).

Notes:
- `ConsumeReq.start` is a tagged union: `{type:beginning}`, `{type:latest}` (last
  `limit`), `{type:offset,offset}`, `{type:timestamp,timestamp_ms}`. `decode` ∈
  `{auto,json,utf8,hex,base64,protobuf,avro}`; `auto` tries JSON → UTF-8 → schemaless
  Protobuf wire-decode → hex, and decodes Confluent-framed Avro via the registry.
- `ClusterMetrics.brokers` is populated from the optional Prometheus `metrics_url`
  (Redpanda `:9644/public_metrics`, or a Kafka JMX exporter); `prometheus_available`
  is false otherwise. Throughput is derived from watermark deltas between calls.
- `ClusterOverview` now includes optional `under_replicated_partitions` (ISR < replicas)
  and `leadership_imbalance` (coefficient of variation of leader counts per broker, 0=balanced).
- `GroupResetReq` body: `{mode: 'earliest'|'latest'|'offset'|'timestamp', offset?: number,
  timestamp_ms?: number, topic?: string, confirm?: boolean}`. Mutations on guarded clusters
  (production / read-only) require `confirm: true`. Writes an audit row to `broker_write_audit`.
- `ProduceReq` now honors `headers: MessageHeader[]`, `key_base64: bool`, `value_base64: bool`
  (already in the DTO). A tombstone is produced by sending an empty string `value` with
  `value_base64: false`.

## Must-have wave (Wave 2) — additional routes

Extensions to existing features (work-graph attribution, broker operator workflows,
product↔swarm closure, vault governance). Auth is covered by the existing per-feature
policy prefixes (`/usage/`→Usage, `/brokers/cluster`→Database, `/product/`→Product,
`/swarm/`→Swarm, `/workspaces/{ws}/memory/`→Product).

**Work-graph attribution (Usage):**

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /usage/attribution | ws viewer (Usage:View) | `?by=repo\|branch\|pr\|story\|swarm_task\|workflow\|channel\|review\|origin` | grouped `{key, cost_usd, tokens, sessions}[]` |
| POST /usage/forecast | ws viewer (Usage:View) | `{feature, provider, est_tokens?}` | `{projected_cost_usd, basis}` |

**Broker operator workflows (Database tier; `/brokers/cluster` prefix):**

| Method & path | Auth | Request | Response |
|---|---|---|---|
| POST /brokers/clusters/{id}/replay | ws editor | `ReplayReq {source_topic, target_topic, selector, transform?}` | `ReplayResp {produced, evidence_id}` |
| GET /brokers/clusters/{id}/schema-registry/subjects/{subject}/versions | ws viewer | — | `SchemaVersion[]` |
| GET /brokers/clusters/{id}/schema-registry/subjects/{subject}/versions/{version} | ws viewer | — | `SchemaVersionDetail` |
| POST /brokers/clusters/{id}/schema-registry/subjects/{subject}/compatibility | ws editor | `{schema}` | `CompatibilityResult {compatible, messages}` |
| GET /brokers/clusters/{id}/lag-alerts | ws viewer | — | `LagAlert[]` |
| POST /brokers/clusters/{id}/lag-alerts | ws editor | `UpsertLagAlertReq` | `LagAlert` |
| DELETE /brokers/clusters/{id}/lag-alerts/{alert_id} | ws editor | — | 204 |

`POST /brokers/clusters/{id}/groups/{group}/reset` now also accepts `?dry_run=true` — returns the computed target vs current offsets + lag delta **without writing**.

**Product↔Swarm closure:**

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /product/stories/{sid}/swarm | ws viewer (Product:View) | — | `StorySwarmLink {project?, tasks, runs, artifacts, prs, reviews, cost_usd}` |
| GET /swarm/tasks/{tid}/story | ws viewer (Swarm:View) | — | `TaskStoryLink {story?, acceptance}` |

**Vault governance (Memory; Product tier; `/workspaces/{ws}/memory/` prefix):**

| Method & path | Auth | Request | Response |
|---|---|---|---|
| POST /workspaces/{ws}/memory/{mid}/state | ws editor | `{state}` (suggested\|accepted\|stale\|contradicted) | updated `Memory` |
| POST /workspaces/{ws}/memory/{mid}/forget | ws editor | — | `{undo_token}` (soft-delete) |
| POST /workspaces/{ws}/memory/{mid}/forget/undo | ws editor | `{undo_token}` | restored `Memory` |
| POST /workspaces/{ws}/memory/merge | ws editor | `{ids}` | merged `Memory` |
| POST /workspaces/{ws}/memory/{mid}/split | ws editor | `{parts}` | `Memory[]` |
| POST /workspaces/{ws}/memory/import | ws editor | `{kind, content}` (AGENTS.md\|CLAUDE.md\|.cursorrules) | `{imported}` |

## Must-have wave (Wave 3) — additional routes

First-party agent context (redacted packets), capability/health registry, and workflow
nodes/triggers. Packet routes are Agents:Edit (+ session owner/admin); capability routes
are root; workflow trigger routes ride the Workflows prefix; the webhook is public-by-token.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| POST /workspaces/{wid}/agents/{sid}/context-packet/preview | ws member (Agents:Edit, session owner/admin) | `{kind, payload}` | `{redacted, redactions, size_bytes}` (preview only) |
| POST /workspaces/{wid}/agents/{sid}/context-packet/send | ws member (Agents:Edit, session owner/admin) | `{kind, payload}` | `{ok, size_bytes, redactions}` (injects the redacted packet) |
| GET /capabilities | root | — | `ModuleCapability[]` (per-feature ready/degraded/missing_setup + deps + fixes) |
| GET /support-bundle | root | — | `SupportBundle` (versions, redacted settings, capabilities, recent audit, migration level) |
| POST /workflows/{id}/webhook/{token} | public-by-token | run input body | `{run_id}` (token validated against workflow_triggers) |
| GET /workflows/{id}/triggers | ws viewer (Workflows:View) | — | `WorkflowTrigger[]` |
| POST /workflows/{id}/triggers | ws editor (Workflows:Edit) | `UpsertTriggerReq {kind, spec}` | `WorkflowTrigger` |
| PATCH /workflow-triggers/{id} | ws editor (Workflows:Edit) | `UpsertTriggerReq` | `WorkflowTrigger` |
| DELETE /workflow-triggers/{id} | ws editor (Workflows:Edit) | — | 204 |
| POST /workflow-runs/{id}/approve | ws editor (Workflows:Edit) | `{node_id, approved}` | resumed run status |

New workflow node kinds (node-types catalog): product_analyze, product_rewrite, product_plan,
product_publish, review_run, canvas, git_pr, condition, loop, swarm_task, api_run, db_query,
broker_peek, channel_notify, budget_gate, human_approval, prepare_context. The four formerly-stub product/review
kinds are now wired (real single-agent turns + the local-review engine); `condition`/`loop` plus
`WorkflowEdge.condition` provide branching and bounded iterate-until control flow. The schedule
trigger scheduler (`workflow_trigger_scheduler::start`) is started at daemon boot (cron + IANA
timezone parity via the shared cadence engine), so all four trigger kinds — webhook, event,
schedule, chat — fire unattended (chat bindings are evaluated live by the channels Bridge, not
polled). A `schedule` trigger's `spec.prompt` (string, optional), when set, is threaded into the
run's input as `input.prompt` (in addition to `input.trigger:"schedule"`) — same input shape at
both `create_run` and the spawned `run_workflow` call — so a fixed instruction reaches the
engine's prompt normalization exactly like a chat-started run.

**Chat trigger (`kind: "chat"`)** and the simplified run command are handled entirely by
`otto-server::workflow_chat` (`WorkflowChatTriggerImpl`), invoked by the channels Bridge for
every inbound Slack/Telegram/webhook message *before* normal session routing. Resolution order:

1. **Legacy structured command** — a message declaring `Action: Workflow` + `Name:` (see
   `parse_workflow_command`) starts the named workflow; the run input additionally carries
   `jira_ticket`/`working_directory`/`relevant_info`/`goals` plus `prompt` (mirrors `msg`,
   belt-and-braces with normalize_prompt).
2. **Simplified command** — `run <name>: <prompt>`, `workflow <name>: <prompt>`, or
   `run workflow <name>: <prompt>` (keywords case-insensitive, tried longest-first; `name` is the
   text up to the first `:` on that line, `prompt` is the rest of that line plus all following
   lines). An unknown name with the explicit `workflow`/`run workflow` keyword replies
   "No workflow named **X**…" without starting a run; an unknown name with the bare `run` keyword
   falls through (bare "run" reads too much like ordinary English to hijack the message).
3. **Channel bindings** — enabled `chat`-kind `WorkflowTrigger`s pin a workflow to a
   channel/chat(/thread): spec shape `{"channel": "slack"|"telegram", "chat": "<id>", "thread"?:
   "<ts>", "mention_only"?: bool}`. `channel`+`chat` must match exactly; an absent `thread` in the
   spec matches any thread (a present one requires an exact match — a thread-pinned binding is
   preferred over an unpinned one when both match); `mention_only` (default false) requires the
   inbound text to contain a Slack mention token (`<@…>`). The run input is
   `{trigger:"chat", origin_workspace_id, channel, chat, thread, user, prompt, msg, raw}` where
   `prompt`/`msg` are the mention-stripped message text and `raw` is the original.

**Security note (chat bindings vs. named commands):** `TriggersRepo::list_enabled_by_kind("chat")`
is GLOBAL across every workspace, so match candidates are walked in preference order and each
candidate's workflow is re-checked against the inbound `workspace_id` before being trusted — a
channel bound by workspace B's Slack/Telegram integration never fires a workflow (or leaks that
channel's messages) into workspace A. This is unlike the legacy/simplified name-addressed
commands above (1 and 2), which intentionally resolve against the GLOBAL workflow library
(`find_by_name`, preferring but not requiring the message's own workspace).

Loop guard: Slack drops any event carrying a `bot_id` (including the nested `message` of a
`message_changed` edit) before it reaches the bridge; Telegram's `getUpdates` long-poll
structurally never returns the bot's own outbound sends. The chat-binding path (only) adds a
second, defensive guard: it never treats a message starting with the bot's own ack prefix
(`"🚀 Started workflow"`) as a binding trigger.

First-party Otto MCP tools (no new HTTP route): the `otto` MCP server is injected into `.mcp.json`
at spawn when the per-workspace `otto_mcp_enabled` setting is on (default off, via `PUT /settings`).
It runs as `ottod mcp-tools` (stdio JSON-RPC) exposing read-only, redacted, row/timeout-capped,
audited tools — `otto_db_schema`, `otto_git_pr_review`, `otto_product_story` (db_query / swarm_task /
broker_topic deferred). Tool calls are logged to `mcp_tool_calls` (migration 0060).

## Must-have wave (Wave 4) — additional routes

Mission Control (work-queue + saved views), cross-module search, and settings/state
portability. DB per-statement timeouts + schema filter + masking ride EXISTING query/peek
routes via request flags (`timeout_ms` / `filter` / `mask`) — no new route.

| Method & path | Auth | Request | Response |
|---|---|---|---|
| GET /workspaces/{id}/mission | ws viewer (Agents:View) | — | `MissionView` (needs_you/working/review_ready/waiting/failed/budget_warn) |
| GET /workspaces/{id}/mission/views | ws viewer (Agents:View) | — | `SavedView[]` |
| POST /workspaces/{id}/mission/views | ws editor (Agents:Edit) | `{name, filter}` | `SavedView` (201) |
| DELETE /mission-views/{id} | ws editor (Agents:Edit, owner) | — | 204 |
| GET /workspaces/{id}/search | ws viewer (Agents:View) | `?q=` | `SearchHit[]` (ranked cross-module: stories/workflows/api-requests/swarm/memories/repos/broker-clusters) |
| GET /settings/export | root | — | redacted settings JSON + `excluded_keys` |
| POST /settings/import | root | settings JSON (secret-keyed entries rejected) | `{accepted, rejected}` |
| GET /state/backup | root | — | non-secret state snapshot (settings + manifest + migration level) |
| POST /state/restore | root | `{backup, confirm:true}` | `{restored}` |

DB Explorer query/peek now honor `timeout_ms` on all engines (ClickHouse/Mongo/Redis, not
just MySQL), a server-side schema-children `filter`, and a `mask` flag that redacts result
cells / broker payloads server-side via `otto_core::redact` (the response carries a `masked`
flag) — all on the EXISTING query/consume routes.

## Goal Loops

Bounded, goal-directed multi-agent iteration. A loop runs Plan → Execute → Evaluate →
Digest cycles on an isolated git branch (`goal-loop/<id>`) until the goal's
acceptance criteria are met or a hard limit (iterations / active time) is hit. Live
updates arrive over `/ws/events` (`goal_loop_updated`). Item routes resolve the
workspace from the loop row; every handler enforces ws Viewer/Editor.

DTOs are `otto_core::api::{DefineGoalReq, GoalLoopDraft, CreateGoalLoopReq,
UpdateGoalLoopReq}` and domain types `otto_core::domain::{GoalLoop, GoalLoopDetail}`.

| # | Method & path | Auth | Request | Response |
|---|---|---|---|---|
| 91 | POST /api/v1/workspaces/{id}/goal-loops/define | ws editor | DefineGoalReq | GoalLoopDraft (runs the AI definer; persists nothing; `feedback` refines) |
| 92 | GET /api/v1/workspaces/{id}/goal-loops | ws viewer | — | `GoalLoop[]` |
| 93 | POST /api/v1/workspaces/{id}/goal-loops | ws editor | CreateGoalLoopReq | GoalLoop (validates non-empty `verify`; starts when `autostart`) |
| 94 | GET /api/v1/goal-loops/{id} | ws viewer | — | GoalLoopDetail (`{loop, iterations}`) |
| 95 | PATCH /api/v1/goal-loops/{id} | ws editor | UpdateGoalLoopReq | GoalLoop (`name` non-terminal; `limits` not while Running; `config` Draft-only) |
| 96 | POST /api/v1/goal-loops/{id}/start | ws editor | — | GoalLoop |
| 97 | POST /api/v1/goal-loops/{id}/pause | ws editor | — | GoalLoop |
| 98 | POST /api/v1/goal-loops/{id}/resume | ws editor | — | GoalLoop |
| 99 | POST /api/v1/goal-loops/{id}/stop | ws editor | — | GoalLoop |
| 100 | POST /api/v1/goal-loops/{id}/iterations/{idx}/agents/{agent}/retry | ws editor | — | 202 (re-run a stuck executor) |
| 101 | DELETE /api/v1/goal-loops/{id} | ws editor | — | 204 (stops + removes worktree; **keeps the branch**) |

## Canvas Studio

Visual scenes (sketches, UML, sequence/flow diagrams, code/JSON blocks, shapes)
stored as ONE portable JSON document (`doc_json`). Workspace-scoped; optionally
linked to a product story. CRUD lives in the `otto-canvas` crate; the
agent-assist endpoints (prompt → diagram blocks) live in `otto-server` because
they need the orchestrator. Gated by `Feature::Canvas` (read=View, write=Edit).
Item routes resolve the workspace from the scene row.

Persistence: `otto_state::canvas` (`CanvasScene`, `CanvasSceneSummary`). The rich
`Scene` schema (nodes/edges/slides) is owned by the UI (`ui/src/modules/canvas/types.ts`).

| # | Method & path | Auth | Request | Response |
|---|---|---|---|---|
| 102 | GET /api/v1/workspaces/{ws}/canvas/scenes | ws viewer | — | `CanvasSceneSummary[]` (newest-updated first) |
| 103 | POST /api/v1/workspaces/{ws}/canvas/scenes | ws editor | `{title, doc?, story_id?, provider?, section?}` | CanvasScene (201; `doc` defaults to an empty scene) |
| 104 | GET /api/v1/canvas/scenes/{id} | ws viewer | — | CanvasScene (full `doc_json`) |
| 105 | PUT /api/v1/canvas/scenes/{id} | ws editor | `{title?, doc?, thumbnail?, provider?, section?, story_id?}` | CanvasScene (partial; omitted fields unchanged, COALESCE) |
| 106 | DELETE /api/v1/canvas/scenes/{id} | ws editor | — | 204 |
| 107 | POST /api/v1/canvas/scenes/{id}/assist | ws editor | `{prompt, mode?}` | AssistResult `{mermaid?, d2?, excalidraw?, format, nodes, edges, note}` (one agent turn edits AND COMMITS the scene's backing file as `doc_json` — not a dry-run preview) |
| 108 | POST /api/v1/canvas/assist/preview | canvas edit | `{prompt, mode?}` | AssistResult (no scene; used by empty-canvas hero + Discovery-Chat "Open in Canvas") |
| 145 | GET /api/v1/sessions/{sid}/canvas-refs | ws viewer | — | `CanvasSceneSummary[]` — scenes referenced by this session |
| 146 | POST /api/v1/sessions/{sid}/canvas-refs | ws editor | `{scene_id}` | 204 (idempotent; 404 if the scene isn't in the session's workspace) |
| 147 | DELETE /api/v1/sessions/{sid}/canvas-refs/{scene_id} | ws editor | — | 204 (detaches; the scene itself is untouched) |

Session references live in `crates/otto-server/src/canvas_refs.rs` (needs
`SessionManager` to resolve a session's workspace, so they can't live in the
`otto-canvas` crate like the routes above). Broadcasts `canvas_refs_changed`
(see `docs/contracts/ws.md`) on attach/detach.

The first-party MCP tool server (`ottod mcp-tools`) additionally exposes two
GOVERNED WRITE tools — `canvas_create_scene` (posts to #103, then best-effort
#146 to reference the new scene to the calling session) and
`canvas_update_scene` (GET #104 then PUT #105, preserving `format`/`sketch`) —
the only two mutating tools in an otherwise read-only MCP surface; both run as
the session owner through the same `WorkspaceRole::Editor` gate a human hits.

## Discovery Chat

A lightweight, interactive conversation with an agent attached to a product
story (works from an empty/Untitled draft) for EARLY discovery and research —
distinct from the swarm discovery run (heavyweight report) and refinement threads
(edit an existing version). Each turn assembles a relevance-bounded context bundle
(latest relevant version + mockups/attachments with text inlined + the most recent
discovery report + open questions + notes) and replays history into one
`run_agent` turn. The agent replies in markdown and may emit an `actions` JSON
array; actions are NEVER auto-applied — the UI applies them via `/apply`. Covered
by the existing `/product/` policy prefix (read=View, write=Edit).

Persistence: `otto_state::product_chat` (`DiscoveryChat`, `DiscoveryChatMessage`).

| # | Method & path | Auth | Request | Response |
|---|---|---|---|---|
| 109 | POST /api/v1/product/stories/{sid}/discovery-chats | ws editor | `{title?}` | DiscoveryChat |
| 110 | GET /api/v1/product/stories/{sid}/discovery-chats | ws viewer | — | `DiscoveryChat[]` (newest first) |
| 111 | GET /api/v1/product/discovery-chats/{cid} | ws viewer | — | `{chat, messages}` |
| 112 | POST /api/v1/product/discovery-chats/{cid}/messages | ws editor | `{body, provider?, model?}` | `{user_message, agent_message}` (one turn; agent_message carries `actions_json`; `provider`/`model` pick the agent, resolved via the configured default when empty) |
| 113 | POST /api/v1/product/discovery-chats/{cid}/archive | ws editor | — | DiscoveryChat |
| 114 | POST /api/v1/product/discovery-chats/{cid}/apply | ws editor | `{action}` | ApplyResult `{story_updated, created_question_ids, created_note_ids, canvas_id}` |

---

## Proof Packs (#115-137)

The evidence layer ("the trust layer"). Every meaningful unit of agent work
carries a **proof pack** whose `status` (`missing | partial | passed | failed |
waived`) is DERIVED from its evidence artifacts, not claimed by the agent, plus a
**done-contract** `done_score` (0..100) with an itemized checklist of what is
present vs. missing. Otto auto-assembles what it can (diff, goal-loop verify
commands, workflow node outputs, review findings, human approvals, **CI status on
PR open**); agents and humans add the rest (build/lint, **screenshots/video**,
**api/db/kafka reads**, **PR-consistency checks**, self-review) via the
artifact/evidence endpoints. All persisted text content is redacted
(`otto_core::redact`) and capped (2 MiB); media blobs are capped at 25 MiB.

Per-repo policy (`repos.proof_config_json`, `RepoProofConfig`) can *strengthen*
the requirement (require a passing test / green CI / resolved review / consistent
PR) — never weaken it. Immutable, content-hashed **snapshots** freeze a pack's
evidence + rendered Markdown/HTML report for audit. Waiving records the
authenticated human approver + reason + timestamp and an immutable approval
artifact (set `OTTO_PROOF_WAIVER_MIN_ROLE=admin` to require workspace Admin).

Feature-gated by `Feature::ProofPack` (`policy.rs`): workspace-axis and flat
routes alike require `ProofPack` View (reads) / Edit (writes); each handler also
checks the caller's workspace role. Persistence: `otto_state::proof`
(`ProofPack`, `ProofArtifact`, `proof_snapshots`, `proof_blobs`); engine:
`otto_server::proof`.

| # | Method & path | Auth | Request | Response |
|---|---|---|---|---|
| 115 | GET /api/v1/workspaces/{id}/proof-packs | ws viewer · ProofPack View | query `status?`, `work_item_kind?`, `work_item_id?` | `ProofPackResp[]` |
| 116 | POST /api/v1/workspaces/{id}/proof-packs | ws editor · ProofPack Edit | CreateProofPackReq `{work_item_kind, work_item_id, title?, parent_pack_id?, repo_id?}` | ProofPackResp (`repo_id` links the pack to a repo so its proof policy applies — strengthen-only) |
| 117 | GET /api/v1/workspaces/{id}/proof-summary | ws viewer · ProofPack View | — | ProofSummaryResp `{rows:[{work_item_kind, work_item_id, proof_pack_id, status, risk_score, done_score, badges[]}]}` |
| 118 | GET /api/v1/proof-packs/{id} | ws viewer · ProofPack View | — | ProofPackDetailResp `{pack, badges[], artifacts[], children[], done_contract, snapshots[]}` (done_contract computed live) |
| 119 | PATCH /api/v1/proof-packs/{id} | ws editor · ProofPack Edit | `{title?, summary?}` | ProofPackResp |
| 120 | DELETE /api/v1/proof-packs/{id} | ws editor · ProofPack Edit | — | `{ok:true}` (cascades artifacts, snapshots, blobs) |
| 121 | POST /api/v1/proof-packs/{id}/artifacts | ws editor · ProofPack Edit | AddArtifactReq `{kind, title, content?, content_url?, status?, metadata?}` | ProofPackResp |
| 122 | POST /api/v1/proof-packs/{id}/assemble | ws editor · ProofPack Edit | AssembleReq `{cwd?, base?, commands?:[{cmd, kind?}]}` | ProofPackResp |
| 123 | POST /api/v1/proof-packs/{id}/waive | ws editor (or Admin if `OTTO_PROOF_WAIVER_MIN_ROLE=admin`) · ProofPack Edit | WaiveReq `{reason}` (≥10 chars) | ProofPackResp |
| 124 | DELETE /api/v1/proof-artifacts/{id} | ws editor · ProofPack Edit | — | `{ok:true}` |
| 125 | GET /api/v1/proof-artifacts/{id}/content | ws viewer · ProofPack View | — | `{content, ref_kind, kind, status, metadata}` (full stored content) |
| 126 | POST /api/v1/proof-packs/{id}/snapshot | ws editor · ProofPack Edit | CreateSnapshotReq `{note?}` | ProofSnapshotResp `{…meta, bundle, report_md, report_html}` (immutable) |
| 127 | GET /api/v1/proof-packs/{id}/snapshots | ws viewer · ProofPack View | — | `ProofSnapshotMeta[]` (newest first) |
| 128 | GET /api/v1/proof-snapshots/{id} | ws viewer · ProofPack View | — | ProofSnapshotResp |
| 129 | POST /api/v1/proof-packs/{id}/media | ws editor · ProofPack Edit | AttachMediaReq `{kind:screenshot\|video, title, mime, data_base64, metadata?}` (≤25 MiB) | ProofPackResp — `415` if `mime` not in the allow-list (png/jpeg/gif/webp/svg, mp4/webm); `413` if the decoded blob exceeds 25 MiB |
| 130 | GET /api/v1/proof-artifacts/{id}/blob | ws viewer · ProofPack View | — | raw bytes (`Content-Type` = blob mime, `Content-Disposition: inline`) |
| 131 | POST /api/v1/proof-packs/{id}/evidence/api | ws editor · ProofPack Edit | ApiEvidenceReq `{title, method, url, status, duration_ms?, request?, response?, metadata?}` | ProofPackResp |
| 132 | POST /api/v1/proof-packs/{id}/evidence/db | ws editor · ProofPack Edit | DbEvidenceReq `{title, engine?, query?, columns?, row_count?, sample?, error?, metadata?}` | ProofPackResp |
| 133 | POST /api/v1/proof-packs/{id}/evidence/kafka | ws editor · ProofPack Edit | KafkaEvidenceReq `{title, topic, message_count?, sample?, truncated?, error?, metadata?}` | ProofPackResp |
| 134 | POST /api/v1/proof-packs/{id}/pr-check | ws editor · ProofPack Edit | PrCheckReq `{title, description, base?, cwd?}` | ProofPackResp (stores a `pr_check` artifact) |
| 135 | POST /api/v1/proof-packs/{id}/ci-refresh | ws editor · ProofPack Edit | CiRefreshReq `{repo_id?, pr_number?}` (default from pack) | ProofPackResp (fetches live CI → `ci` artifact) |
| 136 | GET /api/v1/proof-packs/{id}/report | ws viewer · ProofPack View | query `format=md\|html` | rendered report (text/markdown or text/html) |
| 137 | GET\|PUT /api/v1/repos/{id}/proof-config | ws viewer (GET) / editor (PUT) · ProofPack View/Edit | RepoProofConfig `{require_test?, test_cmd?, require_ci?, require_pr_consistency?, require_review?}` | RepoProofConfigResp |

Artifact kinds: `command | log | screenshot | video | diff | ci | api | db |
kafka | review | approval | pr_check | self_review`. Badges (derived
server-side): `no_proof`, `tests_passed`, `tests_failed`, `human_approved`,
`risky_change`, `ci_missing`, `ci_passed`, `ci_failed`, `ci_pending`,
`db_api_verified`, `ui_verified`, `pr_inconsistent`, `review_unresolved`,
`waived`.

---

## Mission Control (work graph)

The unified work graph: every agentic activity (sessions, swarm projects, goal
loops, workflow runs, PR reviews, product stories, PRs, channel triggers)
projected into one traceable model — `work_items` linked by `work_edges`, each
carrying a `work_events` audit trail, `work_artifacts` (evidence/trace), and
`work_approvals` (human gates). Items are materialized by the
`workgraph_projector` (subscribes to the event bus + a periodic reconcile/backfill;
no module rewiring). The API is read-mostly; writes are human annotation
(risk/goal/result), manual edges, approvals, and a re-derive backfill. Gated by
`Feature::MissionControl` (read=View, write=Edit) plus the workspace-role axis.
A `WorkItem` carries `{id, workspace_id, kind, source_id, title, goal, status,
owner, owner_kind, repo_id, branch, cost_so_far, risk_level, result_summary,
context_summary, last_event_at, created_at, updated_at}`.

Persistence: `otto_state::workgraph` (`WorkGraphRepo`); live signal:
`Event::WorkGraphUpdated` (see `ws.md`).

| # | Method & path | Auth | Request | Response |
|---|---|---|---|---|
| 126 | GET /api/v1/workspaces/{wid}/workgraph/summary | mission_control view | — | MissionSummary `{total, active, needs_approval, total_cost, by_kind[], by_status[], by_risk[]}` |
| 127 | GET /api/v1/workspaces/{wid}/workgraph/items | mission_control view | query `kind?,status?,risk?,q?,limit?` | `WorkItem[]` |
| 128 | GET /api/v1/workspaces/{wid}/workgraph/graph | mission_control view | query `kind?,status?,risk?,limit?` | GraphView `{nodes[], edges[]}` |
| 129 | GET /api/v1/workspaces/{wid}/workgraph/items/{id} | mission_control view | — | WorkItemDetail `{…WorkItem, edges[], events[], artifacts[], approvals[], pending_approvals, needs_approval}` |
| 130 | PATCH /api/v1/workspaces/{wid}/workgraph/items/{id} | mission_control edit | `{risk_level?, goal?, result_summary?}` | WorkItem |
| 131 | POST /api/v1/workspaces/{wid}/workgraph/items/{id}/edges | mission_control edit | `{to_item_id, relation}` | WorkEdge |
| 132 | POST /api/v1/workspaces/{wid}/workgraph/items/{id}/approvals | mission_control edit | `{reason?}` | WorkApproval (pending) |
| 133 | POST /api/v1/workspaces/{wid}/workgraph/approvals/{aid}/decide | mission_control edit | `{decision: approved\|rejected, note?}` | WorkApproval |
| 134 | POST /api/v1/workspaces/{wid}/workgraph/backfill | mission_control edit | — | `{ok, summary: MissionSummary}` |

---

## MCP Control Plane

Governs registered MCP servers/tools (`Feature::Mcp`) and exposes Otto outward as an
MCP server. RBAC: reads/previews = `mcp:view`, mutations/invoke = `mcp:edit`,
posture changes (policy writes/import, outward-server config, approval decisions) =
`mcp:admin`. Registering a `stdio` (command-spawning) server additionally requires
`mcp:admin` in-handler (it runs a command as the daemon). Flat by-id routes also
enforce the entity's workspace role.

| # | Method + Path | Role | Body | Response |
|---|---|---|---|---|
| CP1 | GET /api/v1/workspaces/{wid}/mcp/servers | mcp:view + ws viewer | — | `McpServerDetail[]` |
| CP2 | POST /api/v1/workspaces/{wid}/mcp/servers | mcp:edit + ws editor (stdio→mcp:admin) | CreateServerReq | McpServerDetail |
| CP3 | GET /api/v1/mcp/servers/{id} | mcp:view + ws viewer | — | `{server, tools}` |
| CP4 | PATCH /api/v1/mcp/servers/{id} | mcp:edit + ws editor | UpdateServerReq | McpServerDetail |
| CP5 | DELETE /api/v1/mcp/servers/{id} | mcp:edit + ws editor | — | 204 |
| CP6 | POST /api/v1/mcp/servers/{id}/health | mcp:edit + ws editor | — | McpServerDetail (health probed) |
| CP7 | POST /api/v1/mcp/servers/{id}/discover | mcp:edit + ws editor | — | `McpTool[]` (tool catalog refreshed) |
| CP8 | GET /api/v1/mcp/servers/{id}/tools | mcp:view + ws viewer | — | `McpTool[]` |
| CP9 | POST /api/v1/mcp/servers/{id}/tools/{name}/invoke | mcp:edit + ws editor | `{arguments, dry_run?, workspace_id?}` | InvokeResp (governed) |
| CP10 | PATCH /api/v1/mcp/tools/{tool_id} | mcp:edit + ws editor | `{enabled?,require_approval?,risk_label?,injection_risk?}` | McpTool |
| CP11 | GET /api/v1/workspaces/{wid}/mcp/allowlist | mcp:view + ws viewer | — | `McpAllowlistEntry[]` |
| CP12 | PUT /api/v1/workspaces/{wid}/mcp/allowlist | mcp:edit + ws editor | `{entries:[{server_id,tool_name?,mode}]}` | 204 |
| CP13 | GET /api/v1/mcp/policies | mcp:view | `?workspace_id=` | `McpPolicy[]` |
| CP14 | POST /api/v1/mcp/policies | mcp:admin | CreatePolicyReq | McpPolicy |
| CP15 | PATCH /api/v1/mcp/policies/{id} | mcp:admin | UpdatePolicyReq | McpPolicy |
| CP16 | DELETE /api/v1/mcp/policies/{id} | mcp:admin | — | 204 |
| CP17 | GET /api/v1/mcp/policies/export | mcp:view | — | `{version, policies}` (policy-as-code doc) |
| CP18 | POST /api/v1/mcp/policies/import | mcp:admin | `{policies, replace?}` | `{imported, replaced}` |
| CP19 | POST /api/v1/mcp/policies/evaluate | mcp:view | `{server_id, tool, workspace_id?}` | decision preview |
| CP20 | GET /api/v1/mcp/approvals | mcp:view (ws-filtered) | `?status=` | `McpApproval[]` |
| CP21 | POST /api/v1/mcp/approvals/{id}/decide | mcp:admin (approver≠requester) | `{approved, note?}` | McpApproval |
| CP22 | GET /api/v1/mcp/audit | mcp:view (ws-filtered) | filters | `McpCallLogRow[]` |
| CP23 | GET /api/v1/mcp/stats | mcp:view (ws-filtered) | — | `McpToolStats[]` |

### Otto as an MCP server (outward) + live-agent gateway

| # | Method + Path | Role | Body | Response |
|---|---|---|---|---|
| CP24 | GET /api/v1/mcp/otto-server | mcp:view | — | `{enabled, tools, has_token, token_prefix?}` |
| CP25 | PATCH /api/v1/mcp/otto-server | mcp:admin | `{enabled?, tools?, rotate_token?}` | status + `token?` (shown once) |
| CP26 | POST /api/v1/mcp/otto-tools/invoke | mcp:edit (or the restricted mcp token) | `{tool, arguments, dry_run?, wait_seconds?}` | governed result |
| CP27 | GET /api/v1/mcp/gateway/tools | mcp:view | `?workspace_id=` | `{tools}` (namespaced `mcp__server__tool`) |
| CP28 | POST /api/v1/mcp/gateway/invoke | mcp:edit | `{server_id, tool, arguments, dry_run?, workspace_id, session_id?}` | InvokeResp (governed) |
| CP29 | GET /api/v1/workspaces/{wid}/mcp/code-search | mcp:view + ws viewer | `?q=&path=&max=` | `{query, root, matches, truncated}` |
| CP30 | POST /api/v1/workspaces/{wid}/mcp/context-packet | mcp:edit + ws viewer | `{query?, story_id?, max_excerpts?}` | context packet |
| CP31 | GET /api/v1/workspaces/{wid}/mcp/proof-pack | mcp:view + ws viewer | `?repo_id=&branch=&goal_loop_id=` | evidence bundle |
| CP32 | POST /api/v1/mcp/http | the scoped mcp token (or mcp:edit) | JSON-RPC 2.0 message/batch (`initialize`/`tools/list`/`tools/call`/`ping`) | JSON-RPC result; notifications → `202` |
| CP33 | GET /api/v1/mcp/http | the scoped mcp token (or mcp:view) | — | `405` (no standalone SSE stream — POST requests instead) |
| CP34 | GET /api/v1/mcp/tokens | mcp:admin | — | `{tokens: McpTokenInfo[]}` (all users, no secrets) |
| CP35 | POST /api/v1/mcp/tokens | mcp:admin | `{user_id?, label?, scope?:{tools?, allow_writes?, workspace_id?}}` | `{token, info}` (raw token shown once) |
| CP36 | DELETE /api/v1/mcp/tokens/{id} | mcp:admin | — | `204` (404 if not found) |

**MCP HTTP transport (CP32/CP33).** The outward "Otto as an MCP server" is reachable
over the **Streamable HTTP** transport at `POST /api/v1/mcp/http` — external MCP clients
connect directly with `Authorization: Bearer <mcp-token>` (no local stdio subprocess).
It is served on the loopback listener always, and over the opt-in TLS `network_listener`
(off by default) for remote clients — i.e. **MCP over HTTP, not only locally**. Each
`kind='mcp'` token is route-confined to `/mcp/http` (+ the legacy invoke/status routes)
by the feature guard, and its per-token **McpScope** decides which tools `tools/list`
shows and `tools/call` may run. Every `tools/call` funnels through the same governed
choke point as CP26 (per-token scope → global enable → dangerous→approval → audit).

**McpScope** — the per-token permission set: `{ tools?: string[]  // bare names; omit =
all globally-enabled, allow_writes?: bool  // default false: deny mutating tools,
workspace_id?: string  // optional pin }`. **McpTokenInfo** — `{ id, user_id, username,
label?, token_prefix, scope, created_at, last_seen_at, expires_at }` (never the secret).
Multiple tokens (and multiple users) may each hold a different scope — that is the
mechanism for "different users have different accesses".

---

## Scheduled Tasks

Recurring, workspace-scoped jobs. Each task runs an agent (any provider —
`claude | codex | agy | shell | <custom>`) **as a real, openable session** with a
configurable `prompt` on a cadence, captures the agent's Markdown **report**,
stores it, and delivers it to an optional **destination** (Slack / Telegram /
email / webhook). `kind` is `agent_prompt` (run an agent) or `workflow` (launch a
workflow run via `workflow_id` and report its outcome). Driveable over MCP (see
CP-S below). Gated by `Feature::ScheduledTasks` (read=View, write=Edit) plus the
workspace-role axis; flat by-id routes load the task/run and enforce the role on
its `workspace_id` (IDOR guard).

**v2 capabilities** (all backward-compatible; old tasks behave unchanged):
- **timezone** — `timezone` (IANA, default `UTC`) interprets daily/weekly/cron times DST-correctly.
- **cron** — `schedule = {cadence:"cron", expr:"<5-field cron>"}` (standard Vixie semantics), evaluated in `timezone`.
- **provider** — `provider` may be `claude|codex|agy|shell|<custom slug>`; `shell` runs the prompt as a command.
- **visible session** — every agent run creates a session row (`run.session_id`) you can Open live.
- **sandbox** — `sandbox:"worktree"` runs in a fresh isolated git worktree (when `cwd` is a repo).
- **retry policy** — `max_retries` (0..5); the agent session is retried with backoff (`run.attempts`).
- **notify on change** — `notify_on_change` delivers only when the report hash differs from the last ok run (else `run.skipped_delivery`).
- **proof pack** — `attach_proof` builds a proof pack per run (`run.proof_pack_id`).

`schedule` = `{cadence:"interval"|"daily"|"weekly"|"cron", every_min (≥5), at:"HH:MM",
weekday:0..6, expr}`. `destination` =
`{type:"none"|"slack"|"telegram"|"email"|"webhook", chat_id?, to?, subject?, url?}`.
A `ScheduledTask` carries `{…, provider, model, cwd, schedule, destination, enabled,
timezone, workflow_id?, sandbox, max_retries, notify_on_change, attach_proof,
last_run_at?, last_status?, next_run_at?, …}`. A `ScheduledTaskRun` carries `{…,
status, trigger, started_at, finished_at?, summary, report_path?, report_rel?,
delivered, delivery_error?, error?, session_id?, report_hash?, proof_pack_id?,
attempts, skipped_delivery, workflow_run_id?, created_at}`.

Persistence: `otto_state::scheduled_tasks` (migrations 0084 + 0086); scheduler:
`otto_server::scheduled_tasks_scheduler` (60s tick, in-flight-guard-first,
advance-cursor-on-completion, startup reaper, global run semaphore); engine:
`scheduled_tasks_engine` (session-based provider-agnostic agent runs via
`agent_run`, shell, and workflow handoff); cadence: `cadence` (tz + cron); live
signal: `Event::ScheduledTaskRunUpdated` (see `ws.md`). Delivered report bodies are
redacted (`otto_core::redact`); webhook delivery is SSRF-guarded (`otto_netguard`).

| # | Method & path | Auth | Request | Response |
|---|---|---|---|---|
| 135 | GET /api/v1/workspaces/{id}/scheduled-tasks | scheduled_tasks view + ws viewer | — | `ScheduledTask[]` |
| 136 | POST /api/v1/workspaces/{id}/scheduled-tasks | scheduled_tasks edit + ws editor | `{name, prompt?, kind?, provider?, model?, cwd?, skill?, schedule?, destination?, enabled?, timezone?, workflow_id?, sandbox?, max_retries?, notify_on_change?, attach_proof?}` | ScheduledTask |
| 137 | GET /api/v1/scheduled-tasks/presets | scheduled_tasks view | — | `ScheduledTaskPreset[]` |
| 138 | GET /api/v1/scheduled-tasks/{id} | scheduled_tasks view + ws viewer | — | ScheduledTask |
| 139 | PATCH /api/v1/scheduled-tasks/{id} | scheduled_tasks edit + ws editor | `{name?, prompt?, skill?, provider?, model?, cwd?, schedule?, destination?, enabled?, timezone?, workflow_id?, sandbox?, max_retries?, notify_on_change?, attach_proof?}` | ScheduledTask |
| 140 | DELETE /api/v1/scheduled-tasks/{id} | scheduled_tasks edit + ws editor | — | `{ok:true}` |
| 141 | POST /api/v1/scheduled-tasks/{id}/run | scheduled_tasks edit + ws editor | — | ScheduledTaskRun (the manual run; poll for status) |
| 142 | GET /api/v1/scheduled-tasks/{id}/runs | scheduled_tasks view + ws viewer | — | `ScheduledTaskRun[]` |
| 143 | GET /api/v1/scheduled-tasks/runs/{run_id}/report | scheduled_tasks view + ws viewer | — | `text/markdown` (the stored report) |
| 144 | POST /api/v1/scheduled-tasks/{id}/convert-to-workflow | scheduled_tasks edit + ws editor | `ConvertTaskReq {disable_task?}` | `ConvertTaskResp {workflow_id, trigger_id?}` |

**Convert to workflow (#144)** materializes a scheduled task as a Workflow
(`manual_trigger → agent_prompt [→ channel_notify]`, the notify node added only
when the task delivers to Slack/Telegram) plus a `schedule` trigger mirroring the
task's cadence and timezone; `disable_task:true` pauses the source task. It is the
bridge from the single-step Scheduled-Tasks surface to the multi-step Workflow
engine.

### Scheduled-task MCP tools (on the outward `otto.*` surface, CP25-tunable)

| Tool | mutating | Default | Backing endpoint |
|---|---|---|---|
| `otto.list_scheduled_tasks` | no | on | #135 |
| `otto.list_scheduled_task_runs` | no | on | #142 |
| `otto.create_scheduled_task` | yes (DANGEROUS) | off | #136 |
| `otto.update_scheduled_task` | yes (DANGEROUS) | off | #139 |
| `otto.set_scheduled_task_enabled` | yes (DANGEROUS) | off | #139 |
| `otto.run_scheduled_task` | yes (DANGEROUS) | off | #141 |
| `otto.delete_scheduled_task` | yes (DANGEROUS) | off | #140 |

## Run with Otto

The flagship **one-button** flow: a source item (Jira / Confluence / GitHub issue or
PR / Slack or Telegram thread / Product task / Review finding / Failing test /
Scheduled-task report) becomes an `OttoRun` driven through a fixed stage machine —
resolve source → context packet → isolated branch/worktree → goal-loop or single
agent → proof pack → AI review → **human approval** → PR draft. Workspace-scoped,
gated by `Feature::RunWithOtto` (`run_with_otto`: View for reads, Edit for writes).
Flat by-id routes load the run and re-check the role on its workspace (IDOR guard).
The run advances in the background; subscribe to `Event::OttoRunUpdated` (see `ws.md`).

| Method & path | Auth | Request | Response |
|---|---|---|---|
| POST /api/v1/workspaces/{wid}/runs | run_with_otto edit + ws editor | `LaunchRunReq {source_kind?, source_ref?, url?, seed_text?, mode?, provider?, model?, repo_id?, auto_open_pr?, title?}` | OttoRun (queued; poll/subscribe) |
| GET /api/v1/workspaces/{wid}/runs | run_with_otto view + ws viewer | — | `OttoRun[]` |
| GET /api/v1/workspaces/{wid}/runs/detect?q= | run_with_otto view + ws viewer | — | `{detected: {source_kind, source_ref, url}?}` |
| GET /api/v1/runs/{id} | run_with_otto view + ws viewer | — | OttoRun |
| GET /api/v1/runs/{id}/events | run_with_otto view + ws viewer | — | `RunEvent[]` (the stage timeline) |
| POST /api/v1/runs/{id}/approve | run_with_otto edit + ws editor | `{decision: "approve"\|"reject", note?}` | OttoRun |
| POST /api/v1/runs/{id}/cancel | run_with_otto edit + ws editor | — | OttoRun |
| POST /api/v1/runs/{id}/open-pr | run_with_otto edit + ws editor | — | PrSummary (requires approved + passed/waived proof) |

Webhook entry (public-by-key, same per-workspace `X-Otto-Webhook-Key` as the channel
webhook; classified `Exempt` in `policy.rs`):

| Method & path | Auth | Request | Response |
|---|---|---|---|
| POST /webhooks/{workspace_id}/run | public-by-key (`X-Otto-Webhook-Key` / `Authorization: Bearer`) | `{source_kind?, source_ref?, url?, seed_text?, mode?, provider?, model?, repo_id?, auto_open_pr?, callback_url?}` | 202 `{accepted, run_id, status}` |

When a `callback_url` is supplied, the daemon POSTs the run's result back to it at
the milestones a caller can act on — `awaiting_approval` and every terminal state
(`completed`/`failed`/`rejected`/`cancelled`). The body is the run's public shape:
`{run_id, workspace_id, status, awaiting_approval, terminal, title, source_kind,
source_ref, source_url, mode, proof_status, risk_score, findings_total,
findings_blocking, has_pr_draft, pr_url, approval_decision, error}`. Delivery is
best-effort and SSRF-guarded (`otto_netguard::check_url` + redirect policy — a
loopback/private/metadata target is refused); each attempt is recorded as a
`delivery` `RunEvent`. With no `callback_url` the webhook is a fire-and-forget
trigger (read the result via REST/WS/UI).

Slack/Telegram entry: a `/run <ref>` (or "run with otto …") message launches a run;
an `approve`/`reject` reply in the run's thread resolves the approval gate (authorized
by the integration's `allowed_users`, executed as the daemon root user).

`model` ("" / absent = the provider's default) is the model override handed to the
executing agent and is persisted on the run (`OttoRun.model`). Semantics per mode:
single-agent runs execute on the Claude CLI (the stored `provider` is informational
there) and receive `--model <model>`; goal-loop runs stamp `provider`/`model` onto
the loop's **executors** (real sessions — `--model` for claude/codex), while the
loop's bookkeeping roles keep their tuned defaults. `repo_id` selection in the UI is
backed by `POST /workspaces/{id}/repos/detect` (see Git) — Browse… registers the
picked folder's git toplevel and launches with its id.

---

## Snips (screenshot → annotate → clipboard)

One-gesture screenshot flow: capture (or upload) a PNG, annotate it in the
editor at `#/snip/{id}`, and every step lands on the macOS clipboard
automatically — capture/upload copies the original, each annotated save
re-copies the flattened export, so the latest state is always paste-ready in an
agent session. Storage is file-backed under `data_dir/snips/` (`{id}.png`,
`{id}.annotated.png`, `{id}.json` sidecar; no SQLite table); snips older than
14 days are pruned on create. The bytes last written to the clipboard are
mirrored to `data_dir/snips/clipboard-last.png` (observability + E2E sink;
under `OTTO_E2E` only the mirror is written, the pasteboard is untouched).
Feature gate: `Agents` (GET = View, everything else = Edit).

| Method & path | Auth | Request | Response |
|---|---|---|---|
| POST /snips/capture | member (Agents:Edit) | `{}` | `CaptureSnipResp {cancelled, snip?}` — runs interactive `screencapture -i` (Esc ⇒ `cancelled:true`); 409 while another capture is on screen; 500 with a Screen-Recording hint when macOS blocks the capture |
| POST /snips | member (Agents:Edit) | `UploadSnipReq {data_b64, filename?}` (PNG only, 25 MB raw / 40 MB body) | `Snip` |
| GET /snips | member (Agents:View) | — | `Snip[]` (newest first, cap 100) |
| GET /snips/{id}/image | member (Agents:View) | — | `image/png` (nosniff, inline) |
| GET /snips/{id}/annotated | member (Agents:View) | — | `image/png`, 404 until the first annotated save |
| POST /snips/{id}/annotated | member (Agents:Edit) | `{data_b64}` (PNG, 40 MB body) | `SnipCopyResp {copied}` — saves the flattened export and puts it on the clipboard |
| POST /snips/{id}/copy | member (Agents:Edit) | `{}` | `SnipCopyResp` — re-copy (annotated if present, else original) |
| DELETE /snips/{id} | member (Agents:Edit) | — | 204 |

`Snip {id, created_at, width, height, source: "capture"|"upload",
has_annotated, path}` — `has_annotated` and `path` are both computed from the
filesystem at read time (never trusted from the sidecar, so a moved data dir
can't serve a stale path). `path` is the PNG's absolute path **on the daemon's
machine** — what an agent CLI needs to open it, and why the terminal's
image-paste flow uploads through `POST /snips` before typing the path into the
PTY: the browser may be on a different machine entirely.
Clipboard failure degrades to `copied:false` (the snip itself is always saved).
