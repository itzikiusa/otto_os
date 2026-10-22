# Access control review: iteration 3

Lens: who can access what. That covers route auth and RBAC, the MCP outward tools, share, remote and mobile access, room guests, API tokens and workspace scoping.

Calibration: these findings report over-restriction (a legitimate user blocked, or checks that contradict the documented model) with the same weight as a real, reachable leak. Every finding below was traced in the current code. Theoretical hardening and loopback-only concerns are excluded.

## Score

Start at 10.0.

| ID | Severity | Deduction |
|----|----------|-----------|
| access-control-3-01 | blocker | -1.0 |
| access-control-3-02 | major | -0.3 |
| access-control-3-03 | major | -0.3 |
| access-control-3-04 | major | -0.3 |
| access-control-3-05 | minor | -0.1 |

**Score: 10.0 - 1.0 - 0.3 - 0.3 - 0.3 - 0.1 = 8.0**

## Previous findings

All three iteration-2 findings are fixed. Each was checked against the current code.

| ID | Status | Evidence |
|----|--------|----------|
| access-control-2-01 (any Editor could rewrite another user's personal agent) | Fixed | `personal_agents.rs:280-302` (`owner_only_changes`) and `:311-338` (`require_agent_owner`: owner, workspace Admin or root; an ownerless preset is adopted with a conditional UPDATE). The guard is applied in `update` (`:436`), `create_schedule` (`:548-556`), `update_schedule` (`:595-611`), `save_memory` (`:818`) and `save_context` (`:849`). Presets are seeded with `created_by: None` (`:1305`). `chat_session` reuses only the caller's own pinned or live chat (`:907-927`) and pins only a free slot (`:978-982`). |
| access-control-2-02 (Users section shown to Users:Admin, but `GET /users` was root-only) | Fixed | `users.rs:17-23`: `list` relies on the feature guard (Users:Admin). Create, update and delete stay `require_root` + `require_human`. `Users.svelte:449`, `:463`, `:497` and `:657` show the root-only controls to root only. |
| access-control-2-03 (an Editor could rewrite another user's draft goal-loop config) | Fixed | `goal_loops.rs:325` calls `require_config_owner` (`:353-374`): an unchanged config passes, and otherwise it needs the owner, a workspace Admin or root. The test is at `:654`. |

## Areas covered (this iteration)

- Re-verified every iteration-2 finding (see the table above). I also checked that `GET /users` consumers are consistent: `AccessGroups.svelte` is gated `'root'`, which matches the root-only `access_groups.rs` handlers. `TokensPanel.svelte` treats a 403 as "Me only".
- Workflows, run identity end to end:
  - `workflow_engine.rs`: `acting_user_id` and `resolve_run_user` (`:1950-1960`, `:4279-4300`)
  - `routes/workflows.rs`: every handler gate, the trigger CRUD, and the public `webhook_trigger`
  - `workflow_trigger_scheduler.rs`
  - the `db_query` node and how `agent_session::run_session_turn_with` creates sessions as the acting user
- Swarm, identity end to end:
  - `otto-swarm/src/http.rs`: all CRUD gates
  - `runtime/engine.rs` `run_task`
  - `runtime/run.rs`: how a session is created as `swarm.created_by`
  - `otto-sessions/manager.rs`: the per-session MCP token and provider account resolved from `session.created_by`
  - the session owner-or-admin gates in `otto-sessions/http.rs` and `ws.rs`
- Channels:
  - `routes/channel_webhook.rs`: inbound runs as root
  - `otto-channels/http.rs` `require_owner_for_bridge`: bridge setup is root-only, so a webhook key cannot be minted by a non-root user
- Root-mounted routes, handler side:
  - `lsp/mod.rs` (`root_route_gate`, `root_visible_to`)
  - `routes/api_stream.rs` (workspace Editor + ApiClient:Edit, which matches the policy)
  - the `feature_guard.rs` `root_route_decision` matrix
- API client:
  - every handler in `routes/api_client.rs` and `api_client/*.rs`, `api_oauth.rs`, `grpc.rs`, `api_automation_runs.rs` and `api_response_cache.rs`
  - cookies and the raw response cache are per user
  - an OAuth flow is bound to its actor
  - automation-run reads are Editor-gated as `api.md:2672-2673` documents
- Resource access delegation:
  - `routes/resource_access.rs`, including the `credential_connection_id` rules: a non-root caller cannot assign or swap an execution credential
  - the delegated-authority diff (`:30-80`, `:390-406`)
- MCP control plane:
  - the token list, create, revoke and rotate handlers (`mcp_outward.rs:1722-1815`, `routes/mcp_cp.rs`)
  - `AuthRepo::rotate_mcp_token`
  - the legacy `PATCH /mcp/otto-server` rotate path, which only touches the caller's own token
  - the per-token scope gate in the governed invoke path (`mcp_outward.rs:258-275`, `:384-406`)
- Share mint: the token is issued as the minter, and the session must pass owner-or-admin (`share.rs:277-404`).
- Rooms:
  - the `owner_auth` and `eligible` checks (a shell is an `Agent` kind, so it matches the doc)
  - create, end and invite are owner-only
  - `join` is public by invite
  - terminal input goes through `manager.room_input`, which carries the driver grant epoch
- Host filesystem: `routes/fs.rs` (non-root reads anywhere except `secret_paths` protected dirs, which include the whole Otto data dir).

## Areas not covered yet (next iteration)

- MCP outward, tool by tool: compare the `route_for` self-call map with each target route's own gate (pin verdicts, `PIN_UNVERIFIABLE`, `NATIVE_SESSION_WRITERS`).
- `otto-connections` Enforced ↔ Legacy activation flow (`resource_access.rs:470-560`) and `validate_access_policy`.
- Room participant role changes, and annotation and presentation permissions (`rooms/socket.rs`, `actions.rs`, `presentation.rs`). Also the `game_rooms` socket membership.
- `otto-workflows/src/chat.rs`: who in a bound Slack/Telegram chat may start or steer a run (it acts as the workflow creator).
- The swarm channel triggers (`otto-swarm/runtime/channels.rs`).
- Goal loops: `answer_question`, `verify_criterion` and `retry` by a non-owner Editor while the loop runs as its owner.
- Personal-agent chat and Files panel for persona dirs under `<data_dir>/personal/` (same root cause as 3-04, not traced end to end).
- UI gating compared with the server for agents, git, database and swarm (for example, does the Swarm UI hide **Open session** from non-owners?).

## Findings

### access-control-3-01: a non-root `Mcp:Admin` can rotate any user's MCP token, root's included, and receives a working root-owned token (blocker)

**Files:**

- `crates/otto-server/src/routes/mcp_cp.rs:18-30` (`require_mcp_admin`: root or `Mcp:Admin`) and `:34-58` (`rotate_mcp_token`)
- `crates/otto-rbac/src/tokens.rs:1035-1067` (`AuthRepo::rotate_mcp_token`)
- `crates/otto-server/src/mcp_outward.rs:1722-1731` (`list_mcp_tokens`) and `:1738-1747` (the create guard)
- `crates/otto-rbac/src/tokens.rs:973-998` (the list query)

**Evidence:**

1. `create_mcp_token` explicitly forbids minting for another user: "Minting a token for ANOTHER user hands out a credential that authenticates AS that user, so only root may do it (a non-root mcp:admin minting a root-owned token would otherwise be a privilege escalation)" (`mcp_outward.rs:1738-1747`). The impersonation route has the same rule ("cannot impersonate the root user", `impersonate.rs:129-133`).
2. `GET /mcp/tokens` (`Require(Mcp, Admin)`, `policy.rs:852-853`) returns every `kind='mcp'` token of every user, with `id`, `user_id` and `username`. The query is `tokens.rs:975-979`, with no owner filter.
3. `POST /mcp/tokens/{id}/rotate` calls only `require_mcp_admin`, then `AuthRepo::rotate_mcp_token(&id)`. That function:
   - loads the row's `user_id` and scope (`tokens.rs:1036-1055`)
   - mints a new token for that `user_id` (`:1056-1058`)
   - deletes the old one
4. The handler returns the plaintext `token` to the caller (`mcp_cp.rs:55-57`). There is no comparison between `user_id` and the caller.
5. A NULL or legacy scope resolves to `McpScope::unrestricted()`, which has `allow_writes: true` and no tool or workspace limit (`otto-core/src/auth.rs:94-100`).

**Scenario:**

1. Root grants Carol `Mcp:Admin`, a documented, grantable delegated role.
2. Root has an MCP token, which is the normal setup when root uses Claude Code against Otto.
3. Carol calls `GET /api/v1/mcp/tokens` and picks the row with `username: root`.
4. Carol calls `POST /api/v1/mcp/tokens/{id}/rotate` and receives `{token}`.
5. That token authenticates as root (`kind='mcp'`). Through `/api/v1/mcp/http` Carol can call every enabled Otto tool with root's RBAC and resource-policy bypass: any DB connection, k8s actions, SQS sends, merging PRs, vault writes. Root's own client also stops working, because its old token is deleted.

This is the exact escalation the create guard and impersonation guardrail 2 exist to stop, reached through a sibling route.

**Fix:**

1. In `rotate_mcp_token`, load the owner first. Add an `AuthRepo::mcp_token_owner(id)`, or return the owner from a lookup.
2. Then apply the create rule: `if owner != user.id && !user.is_root { 403 "only root may rotate an MCP token owned by another user" }`.
3. Keep `DELETE /mcp/tokens/{id}` (revoke) open to `Mcp:Admin`. Revoking only removes access, and it is the admin's legitimate kill switch.
4. Keep listing as is; ids are not secrets once rotation is owner-bound.
5. Tests:
   - a non-root `Mcp:Admin` rotating root's token → 403, and root's token still authenticates
   - the same admin rotating their own token → 200
   - root rotating anyone's token → 200

### access-control-3-02: any workspace Editor can rewrite another user's workflow or add a trigger, and trigger-started runs then act as the workflow's creator (major)

**Files:**

- `crates/otto-server/src/workflow_engine.rs:1950-1960` and `:4279-4288` (`acting_user_id`: a run with no starter acts as `workflow.created_by`)
- `crates/otto-server/src/routes/workflows.rs:83-118` (`update_workflow`), `:171` (`restore_version`), `:1526-1561` (`create_trigger`), `:1582-1599` (`update_trigger`) and `:1726-1790` (`webhook_trigger`, `admit_run_if_idle(…, None)`)
- `crates/otto-server/src/workflow_trigger_scheduler.rs:133-163`
- `crates/otto-workflows/src/chat.rs:724`
- `crates/otto-server/src/workflow_engine/nodes/db_query.rs:34-37`
- `docs/contracts/api.md:2422`

**Evidence:**

1. The contract says trigger, schedule and chat runs have `created_by` NULL and "act as the workflow's creator" (`api.md:2422`). Every run spawn on those paths passes `None` as the starter:
   - the webhook (`workflows.rs:1771`)
   - the scheduler (via `admit_schedule_run`)
   - chat (`chat.rs:724`)
2. As a result, `resolve_run_user` returns the creator, and:
   - agent sessions are created as that user (`agent_session.rs:388`, `:400`), which gives them the creator's per-session MCP token and provider account (`otto-sessions/manager.rs:2602-2609`, `:5844`)
   - `db_query` nodes run `db_explorer.run(&conn_id, &user.id, …)` (`db_query.rs:34-37`), which checks the per-user resource policy against the creator (`otto-dbviewer/src/service.rs:2270-2272`)
3. Yet each of these checks only `require_ws_role(…, Editor)`:
   - `update_workflow` (graph, instructions; `require_human_for` only blocks agent credentials)
   - `restore_version`
   - `create_trigger`, which returns the webhook token to the caller
   - `update_trigger`, which changes the spec's `prompt` and result destinations
4. Nothing compares the caller with `wf.created_by`.
5. The three sibling automations that run as their owner now carry the S3-05 owner guard: scheduled tasks, personal agents (2-01) and goal loops (2-03). Workflows are the remaining one. Manual runs are already safe, because they act as the starter (`workflows.rs:612`).

**Scenario:**

1. Alice and Bob are Editors in workspace W.
2. Alice owns workflow X and a private DB connection C. New connections start "enforced and private", per `api.md:5901`.
3. Bob sends `PATCH /workflows/X` with a graph containing a `db_query` node on C (`SELECT * FROM users`).
4. Bob sends `POST /workflows/X/triggers {kind:"webhook"}`, and the response includes the token.
5. Bob sends `POST /workflows/X/webhook/{token}`.
6. The run acts as Alice, the query passes C's policy as Alice, and Bob reads the rows in `GET /workflow-runs/{id}` (Viewer).

Bob can also wait for Alice's existing schedule trigger, with no new trigger at all. An `agent_prompt` node gets Alice's MCP token and git accounts in the same way.

**Fix (option A keeps editing collaborative and is preferred):**

1. A trigger-started run acts as the person who last changed what it runs: the author of the latest graph version (`workflow_versions.created_by`, which `publish` already records via `Some(&user.id)`) or the trigger's last spec editor, whichever is later.
2. Record the spec editor in a new nullable `workflow_triggers.updated_by` column. Use an append-only migration; NULL means the creator, so it stays compatible with the previous build.
3. If both are the creator, nothing changes.

**Fix (option B mirrors S3-05 exactly):**

1. A changed `graph` or `instructions`, `restore_version`, and trigger create or spec changes need `wf.created_by == caller`, a workspace Admin, or root.
2. `name`, `description`, `on_restart`, trigger enable and disable, and manual runs stay open to Editors.
3. An unchanged full-form save passes.

**Tests (either option):** Editor B edits A's graph and fires the webhook. The run must not reach A's private connection: under option A it acts as B and gets a 403 from the connection policy; under option B the PATCH is a 403. Keep A's own trigger runs, and an Admin's edit, green.

### access-control-3-03: swarm turns always run as the swarm's creator, so any Editor's task runs with the creator's identity, and the docs' "open the session" is a 403 for everyone else (major)

**Files:**

- `crates/otto-swarm/src/runtime/run.rs:722-734` (`creator = swarm.created_by`, then `manager().create(&ws_row, &creator, …)`) and `:218`, `:678` (one session per agent, reused across tasks)
- `crates/otto-swarm/src/runtime/engine.rs:2878-2884` (`run_task`: Editor only)
- `crates/otto-swarm/src/http.rs:250-259` (`update_agent`: Editor only)
- `crates/otto-sessions/src/manager.rs:2602-2609` (the MCP token is issued for `session.created_by`) and `:5844` (the provider account is resolved for `session.created_by`)
- `crates/otto-core/src/auth.rs:249-260`
- `crates/otto-sessions/src/http.rs:474-482`
- `crates/otto-sessions/src/ws.rs:1037-1039`
- `docs/features/agent-swarm.md:9` and `:355-359`

**Evidence:**

1. Every swarm role-agent session is created with `created_by = swarm.created_by`. The code comment at `run.rs:722` reads "A system user id for swarm-created sessions: reuse the swarm's creator".
2. That session's Otto MCP token is minted for that user, and the token "can only do what the session's owner is already allowed to do" (`ottod/src/mcp_tools.rs:81-84`). That includes `otto_db_query` on the owner's private connections, and the owner's git and Jira accounts.
3. Any workspace Editor can:
   - create a task (its description is the prompt) and `POST /swarm/tasks/{tid}/run`
   - rewrite a role agent's definition (`update_agent`)
4. Nothing ties the run's identity to the person who asked for the work.

**Scenario, leak:**

1. Bob, an Editor, adds a task to Alice's swarm: "query connection C with `otto_db_query` and post the rows to the board".
2. Bob runs the task. The session acts as Alice, and Bob reads the board (Viewer).

**Scenario, over-restriction:**

1. The docs say each agent "runs as a normal, openable Otto session". They say you can open it from the Org tree, the Runs list, the Graph or the Run Inspector (`agent-swarm.md:9`, `:355-359`).
2. But `GET /sessions/{id}`, `/screen` and the PTY attach are owner-or-admin (`session_owner_or_admin` = root, `created_by` or workspace Admin).
3. So every Editor or Viewer who is not the swarm's creator gets 403 "not the session owner or a workspace admin", even for the task they just ran.

Workflows had the same problem and fixed it by acting as the starter (`workflow_engine.rs:1951-1954`: "Acting as the starter is what lets a non-root editor attach to the sessions their own run spawns").

**Fix:**

1. Create each turn's session as the task's human originator:
   - `task.created_by` when it is a user
   - for agent-created subtasks (`engine.rs:1546`, `:1783` store the agent id), propagate the parent's human originator, as `:1720` already does for delegated copies
   - the `plan` caller for seeded tasks
   - the swarm creator only when no human originated the work
2. Key session reuse by (agent, acting user) instead of by agent alone (`find_agent_session`).
3. Mirroring S3-05, treat role-definition changes that alter what runs (`update_agent`: prompt or soul, provider, model, cwd) as needing the swarm creator, a workspace Admin, or root. Task CRUD, run, stop and board posts stay open to Editors.
4. Tests:
   - Editor B runs a task on A's swarm: the session's `created_by` is B, B can `GET /sessions/{id}`, and the token cannot reach A's private connection
   - A's own tasks still run as A

### access-control-3-04: for every non-root user, a workflow run's **Context files** and **Final output** fail with 403 (major)

**Files:**

- `crates/otto-server/src/routes/workflows.rs:871-885` (`context_dir` is `<data_dir>/workflow-context/<run_id>`)
- `crates/otto-server/src/routes/fs.rs:221-230` and `:247-249` (non-root `/fs/browse` and `/fs/read` deny any path `in_protected_dir`)
- `crates/otto-core/src/secret_paths.rs:154-160` and `:227-229` (Otto's data dirs are protected)
- `crates/otto-server/src/policy.rs:239-241` (`/fs/*` needs Agents:Edit)
- `ui/src/modules/workflows/WorkflowsPage.svelte:399` (Final output via `/fs/read`), `:2583` and `:3653` (Context files via `FileTree` → `/fs/browse`)
- `docs/features/workflows.md:1023-1026` and `:717-718`

**Evidence:**

1. The run view is gated at workspace Viewer (`get_run`, `workflows.rs:895`), and it exposes `context_dir` so the UI can browse it.
2. The UI reads that directory only through the generic host-filesystem routes:
   - `FileTree` calls `/fs/browse?path=<context_dir>` and `/fs/read` (`FileTree.svelte:200`, `:293`)
   - the Final-output panel calls `/fs/read?path=<context_dir>/final-output.md`
3. For any non-root caller, those routes run `non_root_fs_denied`. The context dir sits inside Otto's data dir, a protected dir, so the call returns 403: "holds credentials or Otto's own state; only the root user can open it".
4. Viewers also fail the Agents:Edit policy.
5. The docs promise these files are "browsable in the run view under **Context files**". They also call `final-output.md` "the run's actual deliverable".
6. The result:
   - an Editor or workspace Admin who started their own run sees an error in the tree
   - the Final-output panel silently stays hidden (the `catch {}` at `WorkflowsPage.svelte:404`)
   - only root sees either one

**Fix:**

1. Add run-scoped routes: `GET /workflow-runs/{id}/context` (listing) and `GET /workflow-runs/{id}/context/file?path=`.
2. Gate them at workspace Viewer on `run.workspace_id`, the same as `get_run`.
3. Confine paths with `otto_core::paths::confine_join(data_dir/workflow-context/<run_id>, rel)`, and refuse symlinks that resolve outside it.
4. Point `FileTree` at a pluggable lister/reader for this view, and point the Final-output fetch at the new route.
5. Keep `/fs/*` and its data-dir deny unchanged.
6. Tests:
   - a non-root Viewer reads `final-output.md` of a run in their workspace → 200
   - a member of another workspace → 403 or 404
   - `path=../../secrets.json` → 400

### access-control-3-05: the language server refuses registered repos outside the workspace root, which the same user can already read and run agents in (minor)

**Files:**

- `crates/otto-server/src/lsp/mod.rs:320-333` and `:363-388` (`root_visible_to`: only member workspaces' `root_path` and the scratch root)
- `crates/otto-sessions/src/manager.rs:2973-2980`
- `crates/otto-server/src/routes/fs.rs:221-230`
- `ui/src/modules/panels/FilesPanel.svelte:58-59`
- `ui/src/modules/panels/FileTree.svelte:609-613`

**Evidence:**

1. The agent-cwd check deliberately accepts registered repos outside the workspace root: "A registered repo may live outside the workspace root (sweep 2026-10-11 M2): a person chose it for this workspace" (`manager.rs:2973-2980`).
2. Registering a repo at any path is Editor-level by the documented host-files model (`policy.rs:233-241`).
3. `/fs/read` lets the same non-root Editor read those files.
4. The Files panel roots its tree at the session's cwd and opens files in `CodeEditor` with `lsp` and `root=effectiveRoot`.
5. `/ws/lsp` then answers 403 "root is not inside a workspace you can access" whenever that cwd is a registered repo outside every member workspace's `root_path`.
6. The language server reads only what `/fs/read` already serves to this user, so the denial protects nothing. It just removes hover and go-to-definition for non-root users working in such repos. Root is unaffected (`:320`).

**Fix:** in `root_visible_to`, also accept a root under any repo registered in a workspace where the user is a member. Use `GitStore::list_repos(ws)` for each member workspace, canonicalized the same way the agent-cwd check does. Keep the non-root secret deny (`non_root_canonical_denied`) on the root too, so an LSP can never be rooted in the data dir or `~/.ssh`. Tests:

- a non-root Editor with a repo registered outside the workspace root can open an LSP on it
- a path inside no registered repo and no member workspace → 403

## Checked and not reported (false-positive guard)

- `rooms::eligible` rejects every non-`Agent` session even though its message says "agent and shell sessions". A plain shell is `SessionKind::Agent` (`domain.rs:88`), so this matches `session-rooms.md:4`.
- Inbound `/webhooks/{ws}` runs as root. Configuring the bridge and its key is root-only (`otto-channels/http.rs:175-194`). `delete_integration` at Editor only removes access.
- `/ws/lsp` has no feature-capability check. It is still bounded to member workspaces, and Viewers can already read that code through the git routes.
- API-client history is workspace-shared at Viewer, while automation-run reads are at Editor. Both are documented (`api.md:2672-2673`). Cookies and raw responses are per user.
- A share link minted by a workspace Admin for another user's session is issued as the Admin, keeps a session scope, and drops root. This is consistent with `require_session_owner_or_admin`.
- Resource-access delegation: a non-root caller cannot add or change a `credential_connection_id`, grant itself, or create further delegation (`resource_access.rs:30-80`, `:390-406`).
