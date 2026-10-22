# Access control review: iteration 2

Lens: who can access what. That covers route auth and RBAC, the MCP outward tools, share, remote and mobile access, room guests, API tokens and workspace scoping.

Calibration: these findings report over-restriction (a legitimate role blocked, or checks that contradict the documented model) with the same weight as a real, reachable leak. Theoretical hardening and loopback-only concerns are excluded.

## Score

Start at 10.0.

| ID | Severity | Deduction |
|----|----------|-----------|
| access-control-2-01 | major | -0.3 |
| access-control-2-02 | major | -0.3 |
| access-control-2-03 | minor | -0.1 |

**Score: 10.0 - 0.3 - 0.3 - 0.1 = 9.3**

## Previous findings

All three iteration-1 findings are fixed. Each was checked against the current code.

| ID | Status | Evidence |
|----|--------|----------|
| access-control-1-01 (read-only sessions blocked from non-mutating POST tools) | Fixed | `personal_agent_policy.rs:516-523` adds `/workspaces/{}/mcp/context-packet`, `/brokers/clusters/{}/topics/{}/consume` and `/aws/accounts/{}/logs/insights`. `assistant_recall` is a justified exemption (`READ_ONLY_POST_EXEMPT`, `:944-953`): no read-only session carries `meta.assistant_thread`. The catalog-walking test at `:1012-1020` keeps the allow-list from drifting. |
| access-control-1-02 (share revoke limited to the minter, with a false 204) | Fixed | `routes/share.rs:909-964`: the row is loaded by id and `kind='share'` (404 when missing). The minter can revoke, and so can anyone who passes `require_session_owner_or_admin`. The update has no user filter, and the action is audited with `minted_by`. |
| access-control-1-03 (Rooms reads and game hosting required Agents:Edit) | Fixed | `policy.rs:93-117`: GET `/rooms`, `/room-recaps*` and `/room-recap-capabilities` need Agents:View, and so does POST `/game-rooms`. The writes stay at Edit. The route table (`rooms/mod.rs:23-44`, `game_rooms/mod.rs:17`) has no GET on `/rooms/{id}` or `/game-rooms`, so nothing else needs lowering. |

## Areas covered (this iteration)

- Re-verified every iteration-1 finding (see the table above).
- Users and grants:
  - `routes/users.rs`, all of it
  - `routes/grants.rs` (feature grants and plugin grants)
  - `routes/impersonate.rs` guardrails
  - `routes/admin_sessions.rs` (list, terminate, remove)
  - `routes/access_groups.rs` handler gates
  - the `policy.rs` Users arms (`:1036-1063`) and the `/access/*` arms (`:405-416`)
  - the UI gating in `settings/sections.ts`, `Users.svelte`, `AccessGroups.svelte` and `TokensPanel.svelte`
- Personal agents:
  - every handler in `routes/personal_agents.rs` (CRUD, schedules, run, cancel, report, memory and context documents, `chat_session`, `seed_presets`)
  - the run identity in `otto-assistant/personal_agents_engine.rs:1215` and `assistant_host.rs:340`
  - the delivery path at `personal_agents_engine.rs:989`
  - all compared with the S3-05 owner guard on scheduled tasks (`routes/scheduled_tasks.rs:247-283` and `:488-503`)
- Goal loops: `routes/goal_loops.rs` (define, create, patch, start, resume) and the run identity in `otto-automation/goal_loop_roles.rs:36-38`.
- Plugins:
  - the `/plugin-host/*` handlers, which are root-installed trusted code, so not a finding
  - the reverse proxy and `list_enabled`
- `ws_events.rs` `allowed()`: the 60 s TTL on the role and Admin caches, the immutable owner cache, and fail-closed handling of a vanished session.
- `otto-issues/http.rs`: every account-taking handler goes through `load_authorized_account`. Covered.
- `otto-git/http.rs`:
  - repo listing, register and detect (Editor-gated; registering a path anywhere is consistent with the Agents:Edit host-files model in `policy.rs:233-241`)
  - the account `authorize_owner` call sites
- DB Assist: the `x-assist-key` query route (`db_assist.rs:483-541`), with a constant per-assist key, a hard read-only statement gate, and queries run as `entry.user_id`.
- `otto-dbviewer/access.rs`: the per-action user reload and the Database:View gate.
- `otto-aws/access.rs` and `otto-k8s/access.rs`: the `allowed()` and `initialize()` operation matrices. The SQS peek gate matches the catalog's `mutating:true`, and the logs routes use `metrics`.
- Swarm ingest: `verified_swarm_scope`, which checks the workspace and swarm on every meta id.
- Flat by-id routes, all of which resolve the workspace from the row:
  - Product (`get_version`, `get_analysis`, `update_testcase`, `list_events`, `get_inject`)
  - Design (`approve`, site `export` and `preview`)
  - Canvas (`get_file`, which needs Viewer in any workspace that references it)
  - Browser (tabs and annotations)
- Browser login and cookie isolation (`otto-browser/cdp.rs:635-720`): each fetch and login uses its own disposable context.
- `routes/transcript.rs`:
  - live transcripts (owner or admin)
  - history (non-admins see only their own sessions; `on_disk` rows are admin-only through `history_path_gate`)
- `routes/handover.rs`, `routes/workbench.rs` (per-owner repo scoping) and `routes/backup*.rs`/`connection_export.rs` (root, plus `require_human` on writes; exports carry no secrets).

## Areas not covered yet (next iteration)

- The `otto-swarm` CRUD router: whether swarm agents and runs that act as a creator have the same "editor rewrites, the run acts as the owner" gap as finding 2-01. The automations that run as their owner are swarm runs, workflow runs started by triggers, and channel triggers.
- `routes/workflows.rs` trigger ownership: event and chat triggers that run a workflow as its creator after another Editor edits the graph.
- `routes/channel_webhook.rs` run scoping, and the Slack/Telegram inbound identity mapping in `otto-channels`.
- `lsp/mod.rs`, `browser_live.rs` and `api_stream.rs`: the handler side of the root-route gate.
- `otto-connections`: the Enforced and Legacy resource-policy transitions and the credential connection override (`credential_connection_id`).
- `routes/api_client.rs` and `routes/grpc.rs` (heavily changed in the last 3 weeks): collection ownership and the OAuth token store per user.
- UI gating compared with the server for the remaining modules, beyond Settings.

## Findings

### access-control-2-01: any workspace Editor can rewrite another user's personal agent, which then runs and delivers as that user (major)

**Files:**

- `crates/otto-server/src/routes/personal_agents.rs:345-395` (`update`), `:454-486` (`create_schedule`), `:489-547` (`update_schedule`), `:577-603` (`run_now`), `:698-710` and `:727-765` (`save_memory` and `save_context`), `:771-843` (`chat_session`), `:1125-1150` (`seed_presets`)
- `crates/otto-assistant/src/personal_agents_engine.rs:1215` (the owner is `agent.created_by`) and `:985-991` (delivery goes through `agent.created_by`)
- `crates/otto-server/src/assistant_host.rs:340` (`ctx2.manager.create(&ws2, &owner2, req, None)`)

**Evidence:**

1. A personal-agent run opens its session with `created_by = agent.created_by`. Every self-call, git account, Jira account (`authorize_owner`), DB connection grant and MCP tool call in that session therefore acts as the agent's creator. Delivery to Slack, Telegram, email or a webhook is also sent as the creator.
2. Yet every write handler checks only `require_ws_role(…, WorkspaceRole::Editor)` on `agent.workspace_id`:
   - `update` lets an Editor change `soul_md` (the instructions), `provider`, `model`, `cwd`, `browser` and `delivery`.
   - `create_schedule` and `update_schedule` set the `directive` and the `permission`.
   - `save_memory` and `save_context` rewrite the context that is injected into runs.
   - `run_now` starts a run at once.
3. Nothing compares the caller with `agent.created_by`.
4. Scheduled tasks are the feature this module says it is "same shape as" (`personal_agents.rs:3`). For the same risk they already enforce S3-05: `owner_only_changes` (`scheduled_tasks.rs:247-283`) plus the owner-or-workspace-Admin check (`:488-503`). That check exists to stop exactly this: "another Editor changing them would make A's agent run B's prompt and mail it to B's address". Personal agents never got that guard.

**Scenario:**

1. Alice and Bob are both Editors in workspace W, and Alice created agent X.
2. Bob sends `PATCH /personal-agents/X` with `{soul_md: "<Bob's instructions>", delivery: {type: "webhook", url: <Bob's>}}`.
3. Bob sends `POST /personal-agents/X/run`.
4. The run opens a session owned by Alice. It acts with Alice's git, Jira and connection identities, and sends the report to Bob's webhook.

`seed_presets` makes this worse in shared workspaces. The six example agents are created with `created_by` set to whoever first opened the list (`:1139`, a GET at Viewer). Any Editor can then enable them, rewrite them and run them as that first user.

`chat_session` has a related over-restriction. It creates the chat under the caller (`:830`) but pins it on the shared agent row (`set_chat_session`, `:840`). After Bob opens Alice's agent chat, Alice's own "Chat" returns Bob's session id. She cannot attach to it, because it is not her session and she is not an admin, until that session dies.

**Fix:** mirror the scheduled-task guard.

1. Add `owner_only_changes(agent, req)` for `soul_md`, `provider`, `model`, `cwd`, `browser` and `delivery`.
2. Treat a schedule's `directive` and `permission` (on create and patch) as owner-only, and treat `save_memory` and `save_context` the same way.
3. For each of these changes, require `agent.created_by == caller`, a workspace Admin, or root. Return 403 with a message like "only the agent's owner or a workspace admin can change its X — it runs as its owner".
4. Keep these open to Editors, as for scheduled tasks, because they do not change what runs or where the result goes:
   - name and avatar
   - enable and pause
   - schedule cadence, timezone and enabled flag
   - `run_now`
   - cancel
   - rooms
5. In `seed_presets`, leave `created_by` NULL. The engine's no-owner path already exists at `:1215-1235`. The first user to edit a preset then adopts it, or the presets are seeded on the first Editor write rather than on a GET.
6. In `chat_session`, reuse the pinned session only when `session.created_by == user.id`. Otherwise create a new one, and pin chats per user or leave them unpinned.
7. Add a test that runs as Editor B against agent A for each owner-only field, and keep the owner and Admin paths green.

### access-control-2-02: Settings → Users is shown to Users:Admin, but every `/users` and grants handler is root-only (major)

**Files:**

- `crates/otto-server/src/routes/users.rs:17-23` (`list`), `:25-45`, `:48-57` and `:139-147` (all `require_root`)
- `crates/otto-server/src/routes/grants.rs:163-169` and `:182-189` (`require_root`)
- `crates/otto-server/src/policy.rs:1036-1063`
- `ui/src/modules/settings/sections.ts:32` and `:80`
- `ui/src/modules/settings/Users.svelte:303-313` and `:363`
- `docs/contracts/api.md:2811-2812`
- `docs/features/rbac-multiuser-sharing.md:449`

**Evidence:**

1. `Users:Admin` is a real, grantable role. The policy maps `/users*`, `/admin/sessions*` and `/admin/impersonate*` to `Require(Users, Admin)`, and the handlers for admin sessions and impersonation deliberately add no root check. `impersonate.rs:22-24` says: "Caller authority — root OR `Users:Admin` … we rely on it and add no extra `require_root`".
2. The UI gates the Users section at `{ feature: 'users', level: 'admin' }`.
3. That page's first call, `GET /users` (`Users.svelte:306`), is `require_root` (`users.rs:21`), and so are `GET` and `PUT /users/{id}/grants` (`grants.rs:168` and `:189`).
4. The contract and the feature guide both document the grants routes as "Users:Admin or root". The code is root-only and has been since it was introduced (`f0fce8b48`).

**Scenario:**

1. Root grants Carol `Users:Admin`.
2. Carol opens Settings → Users. The section is listed for her, and `GET /users` returns 403, so the page shows "Couldn't load".
3. Impersonation is allowed for Carol on the server, but the only UI entry to it is the per-user menu on this page (`Users.svelte:363`), so she can never reach it.

The model is also inconsistent: `GET /admin/sessions`, which Carol can call, already returns every user's id and username (`admin_sessions.rs:98-120`), while the plain user list is root-only.

**Fix:**

1. Let `users::list` rely on the feature guard (`Users:Admin`) instead of `require_root`. The list is the same id and username data `/admin/sessions` already exposes, and `User` carries no secrets.
2. Keep create, patch, delete and grants root-only. Granting by a non-root admin would let it escalate itself.
3. Correct the contract and the feature-doc rows for `/users/{id}/grants` to say "root".
4. In `Users.svelte`, show the Create, Disable and Grants controls only when `auth.isRoot`. Users:Admin keeps the list, the impersonation menu and the Sessions section.

If the intent is instead that only root manages people, gate the `users` section at `'root'`, and correct the impersonation and admin-sessions docs to match. Either way, the visible section, the handler and the contract must agree.

### access-control-2-03: an Editor can rewrite the prompts of another user's draft goal loop and start it under that user (minor)

**Files:**

- `crates/otto-server/src/routes/goal_loops.rs:288-342` (`patch`) and `:360-395` (`start`)
- `crates/otto-automation/src/goal_loop_roles.rs:36-38` and `:82-84` (`RoleTurn.user` = `loop_.created_by`)

**Evidence:**

1. Every role turn (planner, executors, evaluator, digester) runs as `UsersRepo.get(&loop_.created_by)`.
2. `patch` checks only `loop_for(…, WorkspaceRole::Editor)`. It accepts a full `config`, including each role's `prompt`, `provider` and `model`, `allow_commits`, `skills` and `source_links`, for any loop in `Draft`.
3. `start` likewise checks only Editor.

This is the same identity-laundering class as finding 2-01. A workspace Editor can make another member's loop run Bob's prompts with `allow_commits: true` under Alice's identity and git accounts. It is limited to loops still in Draft, so its severity is minor.

**Fix:**

1. In `patch`, when `req.config` differs from `loop_.config`, require `loop_.created_by == caller`, a workspace Admin, or root. This mirrors `scheduled_tasks::update`.
2. Keep `name` and `limits` edits, and pause, resume and stop, open to Editors.
3. Add a test covering an Editor who is not the owner (403) and the owner and Admin paths (200).

## Checked and not reported (false-positive guard)

- `/plugin-host/jira/credentials` returns any user's Jira token to a plugin sidecar. Plugins are installed by root (`/plugin-admin*` is `require_root`) and are documented as trusted code, and the host API is authenticated by a per-sidecar token.
- Users:Admin can terminate and remove any user's sessions, root's included (`admin_sessions.rs`). This is the documented "sanctioned cross-user view", and the actions are audited.
- Registering a git repo at any host path needs only workspace Editor. That is consistent with the documented model, where reading the host is the same power as running an agent (`policy.rs:233-241`).
- Scheduled-task `run_now` by any Editor runs the owner's own locked prompt. This is intended, and S3-05 owner-locks the fields that matter.
- `ws_events` caches a `None` owner for a session that is missing at lookup time, so non-root users get no events for it. This fails closed, it is bounded by `OWNER_CACHE_CAP`, and `SessionCreated` carries the owner inline.
- The read-only `GET /state/backup` and `GET /settings/export` have no `require_human`. They are root-only and contain no secrets, and the withholding of root from agent credentials is documented as applying to writes only.
