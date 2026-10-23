# Quality iteration 6 (wave 0): api-contracts lens

Reviewer scope: parity between `docs/contracts/{api,ws,rooms,room-recaps,product,game-rooms}.md`,
the axum routes and WS handlers in `crates/`, the TypeScript mirrors (`ui/src/lib/api/types/*.ts`,
`game-room-types.ts`, `room-types.ts`, module-local `types.ts`) and their UI callers, plus drift in
`docs/features/`. The review was read-only: no cargo, npm, build or test commands were run.
Base: `fix/quality-iter6-20261023` at 02a64019b (the iteration-5 merge).

## Score

Start at 10.0. Open findings: 0 blockers, 0 majors, 3 minors and 3 nits.

`10.0 − 3×0.1 − 3×0.03 = 10.0 − 0.30 − 0.09 =` **9.61**

## Previous-finding status (iteration 5)

I re-checked all five iteration-5 findings against the current code and docs. All five are fixed.

| id | status | evidence now |
|---|---|---|
| 5-01 (minor) | fixed | `api.md:4346` (CP4) now documents the three 409 texts and the "send both maps" recovery path. These match `otto-mcp/src/http.rs:500-512` and `service.rs::resolve_secrets`. The 503 stays only on `PATCH /mcp-servers/{id}` (`api.md:933`). |
| 5-02 (minor) | fixed | `ui/src/modules/brokers/types.ts:52` has `warning?: string`. `GroupsTab.svelte:17,53,500` renders `ResetWarning` and derives `blockedBy` from it, so Reset is disabled while the warning is set. |
| 5-03 (minor) | fixed | `api.md:1880, 1881, 1883, 1884` now read `…?account_id=` with "`account_id` (required; 400 without it)". |
| 5-04 (nit) | fixed | `BrokerReplayStatus` and `status?` are gone from `ui/src/lib/api/types/brokers.ts` (`rg BrokerReplayStatus ui/src` finds nothing). |
| 5-05 (nit) | fixed | `api.md:359` (#67) names `scope_md` / `specialization`, `soul_md` / `soul_name`, `provider`, `model` and `cwd_mode`, and quotes the real "…can change an agent's <changed parts>" text. It also adds the new schedule-directive gate (`otto-swarm/src/http.rs:310-327`). |

## Areas covered (this iteration)

- **Re-verification of the five iteration-5 findings** (see the table above).
- **Scripted route inventory, both directions, `docs/contracts/*.md` and `docs/features/*.md`.**
  Every `.route("…")` literal in `crates/*/src` was matched against every `METHOD /path` in the
  contract files and the feature guides, and the reverse. The contracts are clean. The only
  leftovers are test fixtures (`/botTOKEN/*`, `/rest/api/3/*`, `/__e2e/*`, `/big`, `/chunked`), or
  routes documented in a `| METHOD | path |` table layout (`/access/*`, `/telemetry/*`,
  `/plugin-*`, `/room-recaps*`, `/ws/rooms/*`). The feature-guide leftovers are relative shorthand
  (`…/rescan`, `/requests/{id}`), the design-stage `mcp-control-plane-design.md`, and one stale
  path in `ws.md` (→ 6-05). The repo now also has an exact METHOD+PATH contract test (094bc898f).
- **Scripted UI→route check** (`api.*` and `api.bg.*` template paths in `ui/src`, matched to the
  route table and method). Every leftover is a query-string template artifact. No UI call targets a
  missing route or the wrong verb.
- **Scripted Rust-struct ↔ TS-interface field diff** (every serde struct whose name matches an
  exported TS interface). I read each mismatch by hand. They are one-line TS interfaces the
  scanner mis-parsed, viewer-only options flattened beside the DTO (`ConsumeOpts`, `GrpcOptions`),
  transient list fields (`Session.live/viewers/held`), the live-push-only `ToolResult.elided`
  (`transcript_tail.rs:460-477`), the run-summary fields, or name collisions with unrelated UI
  types. None is real drift.
- **Iteration-5 fix commits** (`9a0639c6d`, `8d6958ffe`, `d2e91c986`):
  - `UsageBudgetStatus.spend_unavailable`: the Rust type, `usage.svelte.ts:285`, `api.md:3045` and
    the 500 on an unreadable config all agree.
  - The swarm prompt-editor identity: row 77 and the goal create row (`api.md:3182`) match the code; row 79 is stale (→ 6-03).
  - The `source-run:` label 400, the schedule-directive 403 and personal-agent ownership.
  - Room and game capability primitives (`rooms/capability.rs::socket_token`) vs `rooms.md:42` and
    `game-rooms.md`.
  - The shared game catalog (`catalog.json`) vs `game-rooms.md:18-19`, plus snapshot versions 4/4
    and the relay retry schedule (1, 2, 4, 5, 5 s) vs `features/room-games.md`.
  - The MCP env-key validation, session-file admission and deadline, the workflow-delete task
    pause and the shutdown refusal. All four are in the feature guides but missing from `api.md`
    (→ 6-02).
- **Game-room payload validation** (the gap iteration 5 left). I checked the relay limits in
  `game_rooms/registry.rs`: snapshot 16384, input 1024, result 4096, object-only `data`,
  monotonic `seq` reset by `advance()` on every generation and round change, the 60/s burst-90
  stream bucket, and the 20480 frame and message limit in `socket.rs:33-34`. All match
  `game-rooms.md`. `shooter/net.ts` and `kart/net.ts` decode into objects and validate before use.
- **Self-improvement rows** (the "prose rows" gap): `otto-improve/src/http.rs` vs `api.md:2025-2041`
  and `features/self-improvement.md:370-388` (→ 6-01).
- **Swarm 404/409 semantics** (the gap): `run_task` (`runtime/engine.rs:2822-2885`), clear-project,
  utilization, waiting, agent-stop, the goal and trigger deletes, and verify / verify-stop. Clean
  apart from row 79 (→ 6-03).
- **Insights rows** (the gap): `otto-insights/src/lib.rs:1176-1223` (`ActiveRun`, the cancel 404
  and 204, root gates) vs `api.md:3054-3061`. Clean.
- **API-client automation rows** (the gap): `routes/api_client/automations.rs` (Viewer and Editor
  gates, `ensure_in_workspace`, the 204 on delete, the synchronous run) vs `api.md:2685-2693`.
  Clean apart from a type name (→ 6-06).
- **Ad-hoc `json!` response handlers.** I swept `Ok(Json(json!({…})))` handlers across the crates
  (swarm, personal agents, scheduled tasks, proof packs, MCP control plane, backup-git, git, k8s,
  product publish) against their rows. All match except the wrapper and omitted-field cases in 6-01.
- **WS:** the `Event::type_name()` arm count (80) still matches `ws.md:602`.
  `ui/unit/eventContract.test.ts` guards tag and field parity. The event table in
  `features/daemon-http-api.md:398-416` names only real events.
- **Classroom / Otto School:** the `api.bg` calls (`/sessions?archived=…&limit=1000|60`,
  `/sessions/{id}/screen`) vs the `ListQuery` cap of 1000 and `SessionScreen`. Clean.

## Areas NOT covered yet (next iteration)

- **Notification rows** (`api.md:2819-2827`). These still describe responses in prose: the 204s on
  `DELETE /notifications`, `/read-all`, `/{id}/read` and `DELETE /{id}`, and the 400 for more than
  500 ids, are not stated. The prose is vague but not wrong, so I did not score it (as in
  iteration 5).
- **`docs/features/*` status-code claims outside this iteration's sample.** In particular, I did
  not read `git.md`, `database-explorer.md` and `kubernetes-*.md` row by row.
- **AWS / K8s monitor `json!` payloads** (`otto-k8s/src/monitor/{http,fleet}.rs`) against the
  `api.md:5810-5870` TypeScript-like shapes, field by field.
- **The PTY-holder protocol** (internal).
- **Product contract (`product.md`)**, re-read only through the publish route this time.

## Findings

### api-contracts-6-01: minor. Three routes are documented as returning a bare DTO, but the code returns a wrapper or more fields

The pattern: the contract cell names a bare row type, or omits keys, while the handler returns a
`json!` wrapper. The UI typed the real shape, so only `api.md` is wrong. External / MCP clients
read `api.md`, and `otto.get_improvement_run` passes the route's body straight through.

| Contract row | Documented | Actual (code) |
|---|---|---|
| `docs/contracts/api.md:2036` `GET /improvement/runs/{run_id}` | `ImprovementRun` | `{ run: ImprovementRun, edits: ImprovementEdit[] }`, from `crates/otto-improve/src/http.rs:188-199`. The UI already types it this way (`ui/src/lib/api/improve.ts:22-25`), and so does `docs/features/self-improvement.md:383`. |
| `docs/contracts/api.md:4807` `GET /agent-rooms/{id}` | `AgentRoom` | `{ room: AgentRoom, members: string[] }`, from `crates/otto-server/src/routes/personal_agents.rs:1037-1047` (`list_members` returns `Vec<String>`, `otto-state/src/personal_agents.rs:1718`). |
| `docs/contracts/api.md:4663` `POST /personal-agents/{id}/chat-session` | `{session_id}` | `{session_id, created: bool}`, from `personal_agents.rs:907, 924, 987`. The UI types `created` (`ui/src/lib/api/personalAgents.ts:75`). |

The neighbouring self-improvement rows are also behind the code and the feature guide:

- `api.md:2034` `POST …/self-improvement/run` says "trigger a run now". The handler returns
  `RunNowResp {run_id}`, and **409** "a run is already in progress" (`otto-improve/src/http.rs:132-145`).
- `api.md:2037` `GET …/improvement/edits` omits `?status=`, which defaults to `pending`
  (`http.rs:201-224`; `features/self-improvement.md:384` documents it).

**Failure.** A client built from the authoritative contract reads `body.status` / `body.summary`
from the run detail, and `body.name` from the room detail. Both are `undefined`, because the data
sits under `body.run` and `body.room`. It also never learns that `…/edits` returns only pending
edits unless it passes `?status=`.

**Fix.** In `api.md`:

- `:2036`: `{run: ImprovementRun, edits: ImprovementEdit[]}`.
- `:2034`: `RunNowResp {run_id}`, and 409 when a run is already in progress.
- `:2037`: `?status=pending|applied|rejected|rolled_back|conflict` (default `pending`; an unknown value also reads `pending`).
- `:4807`: `{room: AgentRoom, members: agent_id[]}`.
- `:4663`: `{session_id, created}`, where `created` is false when the existing pinned chat session was returned.

### api-contracts-6-02: minor. Iteration-5 behaviour changes reached the feature guides but not the authoritative `api.md`

The pattern: the iteration-5 fixes added new refusals and side effects, and the docs commit
(`d2e91c986`) wrote them into `docs/features/`, but not into the contract rows. AGENTS.md says
"Contracts first … update `docs/contracts/*.md` … in lockstep". The cases:

1. **The env-key 400 on four MCP save routes.** `otto_mcp::service::env_key_error`
   (`crates/otto-mcp/src/service.rs:111-124`) refuses any `env` / `secret_env` key that is not
   `[A-Za-z_][A-Za-z0-9_]*` with **400** "env key … must be a shell variable name …". It runs in:
   - CP2 `POST /workspaces/{wid}/mcp/servers` (`otto-mcp/src/http.rs:361`);
   - CP4 `PATCH /mcp/servers/{id}` (`http.rs:493`, supplied maps only);
   - `POST /workspaces/{id}/mcp-servers` (`otto-server/src/routes/mcp_servers.rs:242`);
   - `PATCH /mcp-servers/{id}` (`mcp_servers.rs:324`).

   It is documented only in `docs/features/mcp-control-plane.md:135-137`. Rows `api.md:4344`
   (CP2), `:4346` (CP4), `:932` and `:933`, and the notes at `:940-947`, say nothing. Before
   iteration 5, a key like `LINEAR-API-KEY` was accepted, so this is a new 400 on an existing
   route.
2. **Session-scoped file routes: busy and deadline statuses.** `GET /sessions/{id}/files/browse`
   and `…/files/read` now run through `fs::filesystem_work`
   (`crates/otto-server/src/routes/session_files.rs:171-200, 283`). That is at most 4 per route,
   or **409** "Filesystem access is busy. Retry shortly." A 10 s deadline then gives
   `Error::Upstream`, which is **502** (`routes/fs.rs:170-192`, `error.rs:33`). Rows
   `api.md:2360-2361` list only "400/403/404". The `/fs/read` row (`api.md:3088`) already
   documents "409 when busy, 502 on timeout". Also, the code comment at `session_files.rs:172`
   says "→ 504", but the route answers 502.
3. **`DELETE /workflows/{id}` pauses the scheduled tasks that run the workflow.** In the same
   transaction, the delete sets `enabled = 0` and `last_status = 'error'` on those tasks and records a
   "workflow deleted – task paused" run (`otto-state/src/workflows.rs`, `WORKFLOW_DELETED_NOTE`).
   This is documented in `docs/features/workflows.md:1262` and `scheduled-tasks.md`. The contract
   row `api.md:2351` says only "204; 409 while … pending/running".
4. **Session create/resume returns 409 while `POST /app/kill-sessions` sweeps.**
   `refuse_while_shutting_down` (`crates/otto-sessions/src/manager/shutdown_guard.rs:28-35`,
   called at `manager.rs:2700, 2963`) answers **409** "the daemon is shutting down". The flag is set
   only inside `shutdown_all`, which `POST /app/kill-sessions` calls (`otto-sessions/src/http.rs:297-305`).
   It is not documented anywhere: `api.md:85` (#18) and `:769`. The `:769` row also omits the
   `{killed: n}` response.

**Failure.** An external or MCP client that validates against `api.md` is surprised by a 400 on an
MCP save with a hyphenated env key. It also cannot tell a busy-filesystem 409 from a conflict, or
a 502 timeout from a provider outage. And someone deleting a workflow is not told, by the
contract, that the delete also pauses the scheduled tasks that run it.

**Fix.** Add the following to the named `api.md` rows:

- 4344, 4346, 932 and 933 (and the notes): "env keys (plain and secret) must be shell variable
  names `[A-Za-z_][A-Za-z0-9_]*`, else 400 naming the key; on PATCH, only the supplied maps are
  checked".
- 2360 and 2361: "four admitted per route, 409 when busy, 10 s deadline, 502 on timeout (as
  `/fs/read`)". Correct the `session_files.rs:172` comment to 502.
- 2351: "also pauses (`enabled=false`) every scheduled task whose `workflow_id` is this workflow
  and records a 'workflow deleted – task paused' error run on each, in the same transaction".
- 85 and 769: "409 'the daemon is shutting down' while a kill-sessions sweep runs". 769:
  "→ `{killed: number}`".

### api-contracts-6-03: minor. Swarm row 79 (`POST /swarm/tasks/{tid}/run`) contradicts row 77 on turn identity and gives the wrong status for a workspace budget block

**Where:** `docs/contracts/api.md:375`.

**Evidence:**

- **Identity.** Row 79 still defines "Turn identity (PC-4)" as "each turn's agent session is
  created as the task's human originator — `task.created_by` …". Iteration 5 changed this.
  `acting_user` (`crates/otto-swarm/src/runtime/run.rs:262-286`) first returns the task's
  **prompt editor** (`swarm_tasks.prompt_updated_by`, migration 0195), then falls back to the
  originator walk. Row 77 (`api.md:373`) and `features/agent-swarm.md:364-370` say this. Row 79,
  which is the row that defines the run's identity, does not.
- **Status codes.** Row 79 says "409 when … the swarm is aborted or budget-paused/over budget".
  The handler (`runtime/engine.rs:2822-2885`) answers **400** `Invalid` "Budget exceeded — swarm
  blocked: …" when the workspace usage budget blocks (`ctx.budget_blocked`). Only the swarm's own
  budget (`budget_exceeded`) and the pause reason are 409. It also answers **400** "no active agent
  to run this task", which the row does not list. `api.md:1509` documents the same workspace-budget
  block as 400 for local reviews.

**Failure.** A client or agent that reads row 79 expects a task an Editor rewrote to run as its
author, which is the exact identity bug access-control-5-01 closed. A client that branches on 409
for "over budget" misreads the workspace-budget 400 as a validation error.

**Fix.** In row 79, say "acts as the task's latest prompt editor (row 77), else its human
originator — `task.created_by` …". Split the statuses: "409 when the task is not
todo/blocked/backlog, the swarm is aborted, paused with a reason or over its own budget, or the
agent is busy; 400 when the workspace usage budget blocks ('Budget exceeded — swarm blocked: …')
or no active agent can take the task".

### api-contracts-6-04: nit. CP46 `PATCH /mcp/profiles/{id}` omits the 409 for a rename to an existing name

**Where:** `docs/contracts/api.md:4399`. The row says "McpProfile (`400` for the locked Full
profile …; `404` unknown id)".

**Evidence:** `McpProfilesRepo::update` (`crates/otto-state/src/mcp_profiles.rs:185-209`) runs the
`UPDATE … SET name = ?` with `.map_err(dberr_unique("update mcp profile", DUPLICATE_NAME))`. That is
the same unique-index mapping as create, a **409** against `idx_mcp_profiles_name … COLLATE NOCASE`.
`fields_from` also gives the CP45 validation 400s (blank or over-60-character name, unknown group or
tool) on PATCH. The CP45 row (`:4398`) lists both, but the CP46 row lists neither. Iteration 5 noted
the 409 but did not score it.

**Fix:** add "the CP45 validation 400s; `409` when the new name duplicates another profile's
(case-insensitive)" to the CP46 row.

### api-contracts-6-05: nit. `ws.md` names the workflow progress route by a path that does not exist

**Where:** `docs/contracts/ws.md:967`: "exactly the node shape `GET /workflows/runs/{id}/progress`
serves".

**Evidence:** the route is `/workflow-runs/{id}/progress` (`crates/otto-server/src/routes/mod.rs:761`).
`api.md:6262` and the UI (`ui/src/lib/api/workflows.ts:53`) both use that path. There is no
`/workflows/runs/…` route.

**Fix:** change `ws.md:967` to `GET /workflow-runs/{id}/progress`.

### api-contracts-6-06: nit. The automation list row names a type that does not exist

**Where:** `docs/contracts/api.md:2685`. `GET /workspaces/{wid}/api-client/automations` → `Automation[]`.

**Evidence:** the handler returns `Json<Vec<ApiAutomation>>`
(`crates/otto-server/src/routes/api_client/automations.rs:7-14`). The UI calls
`api.get<ApiAutomation[]>` (`ui/src/lib/stores/apiClient.svelte.ts:1818`). The neighbouring rows
(`:2686-2687`) say `ApiAutomation`. Neither the Rust nor the TS side has an `Automation` type.

**Fix:** `ApiAutomation[]`.
