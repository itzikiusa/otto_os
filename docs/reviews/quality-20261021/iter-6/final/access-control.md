# Access control review: iteration 6 (final)

Lens: who can access what. That covers route auth and RBAC, the MCP outward tools, share, remote and mobile access, room and game-room guests, API tokens and workspace scoping.

Calibration: over-restriction (a legitimate user blocked, or a UI that offers what the documented model refuses) counts the same as a real, reachable leak. Every finding was traced in the current code of `fix/quality-iter6-20261023` (HEAD `2065f00e9`). Theoretical hardening and loopback-only concerns are excluded. The review was read-only: no builds or tests were run.

The yardstick is still the model iterations 2–5 settled and `docs/features/agent-swarm.md` documents: **work that runs as person A takes its prompt only from A, a workspace Admin or root**, and "adding or editing work on someone else's swarm never borrows their MCP token, connections or accounts" (`agent-swarm.md:790-795`).

## Score

Start at 10.0.

| ID | Severity | Deduction |
|----|----------|-----------|
| access-control-6f-01 | major | -0.3 |

**Score: 10.0 - 0.3 = 9.7**

## Previous findings (wave 0)

| ID | Status | Evidence |
|----|--------|----------|
| access-control-6-01 (hiring an agent with prompt, soul or directive at Editor) | **Fixed** | `create_agent` now runs `hire_identity_fields` against a blank agent and `require_identity_editor` (creator, ws Admin or root) before storing (`crates/otto-swarm/src/http.rs:239-255`, `:261-283`, `:304-330`). It covers prompt and specialization, inline soul, a provider other than `config.provider`, a model or cwd mode other than the config default, and a non-empty schedule directive. A plain hire stays Editor. PATCH shares the same predicate (`:332-352`). The only other `NewAgent` writer is preset instantiation inside `create_swarm`, which runs as the creator (`:186-201`, `presets.rs:225`). Tested in `crates/otto-server/tests/swarm_identity.rs:432`. Docs updated (`agent-swarm.md:585`, `api.md:367`). |
| access-control-6-02 (project and swarm text runs under each task's originator) | **Fixed as reported.** The fix introduced a new problem in the same resolver, reported as 6f-01. | Migration `0196_swarm_scope_prompt_editor.sql` adds `prompt_updated_by` and `prompt_updated_at` to projects and swarms. Writes are stamped in the same statement or transaction as the text: project name, description and goal (`otto-swarm/src/service.rs:300-340`), swarm description (`:147-165`), project and task goal create and edit via `stamp_goal_editor` (`otto-state/src/swarm.rs:707-734`; `runtime/engine.rs:2095-2250`), and standing goals (`otto-state/src/swarm.rs:2725-2777`). Copied goal templates keep their template author (`verify.rs:449-451`). `acting_user` takes the most recent editor across task, project and swarm (`runtime/run.rs:309-390`). Bob's PATCH, project goal and standing-goal rewrites now run as Bob (`swarm_identity.rs:472`, `:551`). |
| access-control-6-03 (Agent editor offered creator-only fields to every Editor) | **Fixed** | `canEditSwarmIdentity` (`ui/src/modules/swarm/access.ts:19-22`) mirrors the server predicate. When it is false, provider is disabled, and model, specialization, soul, scope and directive are read-only, with `IDENTITY_OWNER_NOTE` shown (`AgentEditor.svelte:64-81`, `:169-209`, `:281`). A non-creator's hire starts on the swarm defaults with the identity fields blank (`:74-81`). It sends no `cwd_mode`, so a plain hire is never refused. |
| access-control-6-04 ("Open session" offered for another user's run session) | **Fixed** | `canOpenRunSession` returns true when the session is in the caller's own store, or the caller is root or a ws Admin (`access.ts:31-33`). That matches the server's `session_owner_or_admin` (`otto-sessions/src/http.rs:30-44`). `RunsList.svelte:156-160` and `RunInspector.svelte:35-37`, `:145-149` render a disabled control titled `RUN_SESSION_NOT_YOURS` otherwise. |

Summary: 4 fixed, 0 still open, 0 invalid. 1 new finding (6f-01) is a regression introduced by the 6-02 fix.

## Areas covered

- Re-verified all four wave-0 findings, end to end, at server, store and UI level (table above).
- **The 6-02 resolver itself**, re-walked for every caller of `TurnSession::resolve`/`acting_user`:
  - work turns (`run.rs:461`)
  - verify turns (`verify.rs:1012`)
  - planning turns (project, no task)
  - scheduled turns (no project, no task: `scheduler.rs:200-212`)
- Which writes stamp a timestamped editor and which do not: task, project and swarm create versus update, goal create, edit and delete, standing-goal replace, and `ensure_standing_goals` seeding. This produced 6f-01.
- Every swarm route gate in `otto-swarm/src/http.rs` and `runtime/engine.rs` that changed this iteration.
- **New `otto.query_db` (approval-gated DB writer, `df56f044e`):**
  - It self-calls `/connections/{id}/db/query` as the caller (`otto-mcp/src/outward/exec.rs` `db_query_call`). That route is `Require(Database, Edit)` (`policy.rs:2293`) plus `check_conn_role(Editor)` (`otto-dbviewer/src/http.rs:530-538`).
  - `confirm_write` is never sent, and an agent's would not count (`person_confirmed`). The read-only and production write guards still refuse the call, and `query_db_readonly` is unchanged.
  - It is in neither `NATIVE_SESSION_WRITERS` nor the personal-agent read-only allow-list, so it always goes through governance. The end-to-end session-token test is `otto-server/tests/mcp_db_execute.rs`.
  - No finding.
- **The MCP governance stage-runner refactor (`82bdd35e4`):** the new `ApprovalGate` and `run` (`otto-mcp/src/governance.rs:133-175`, `otto-server/src/mcp_outward.rs:652-700`) compute the same `needs_approval` as the removed inline code:
  - `gate_applies || forced_approval`
  - auto-approve only when `gate_applies`
  - a forced approval is never auto-approved
  - approvals are bound to the caller session via `find_usable_in(…, caller_session_id)`
  - a dry-run skips approval, as it did before.

  No finding.
- **Session tool twins generated from the control-plane specs (`1ce84030a`):** every entry in `ROUTED_TWINS` (`ottod/src/mcp_tools/dispatch.rs:122-…`) runs `route_for` with the session's managed token, so the route RBAC applies. `TwinScope::Workspace` writers always use the calling session's own workspace. No finding.
- **Rooms extracted into `otto-rooms` (`298502ff6`):**
  - `owner_auth` is unchanged (`otto-rooms/src/auth.rs:3-16`). It refuses share scope, MCP-only, MCP scope, MCP-internal, managed-session and impersonated credentials.
  - `create` still checks `session.created_by == effective_user` and ws Editor (`otto-server/src/rooms/http.rs:15-38`).
  - `eligible` moved into `lifecycle::open` (`otto-rooms/src/lifecycle.rs:17`).
  - `end` checks the room owner (`lifecycle.rs:64-70`).
  - Every recap and ICE handler still calls `owner_auth`, and game rooms do too (`game_rooms/http.rs:14`).

  No finding.
- **API-client engine moved to `otto-apiclient` (`ffa80578d`):** `require_editor` runs on execute (`engine/execute.rs:350`). The workspace-ownership checks on bound and step requests (`engine/binding.rs:258-264`, `:312-318`, `engine/steps.rs:433`, `engine/env.rs:79`) and the netguard checks (`engine/send.rs:102`, `client.rs`) all survived the move. The OAuth start and exchange paths keep `workspace_allows_local` and `check_url` (`routes/api_oauth.rs:116-121`, `:281-286`).
- `/admin/sessions` keyset paging (`routes/admin_sessions.rs`): the route and its `Require(Users, …)` policy are unchanged, and only the query changed.
- Vault-docs retry endpoints moved into `vault_docs_agent/retry.rs`: still ws Editor on the run's workspace (`:166`, `:299`), the same as before the move.

## Areas not covered (by choice; no change this iteration)

- Remote and mobile share pairing (`routes/share.rs`, `host_guard.rs`): untouched since origin/main and covered in iterations 2–4.
- `otto-git` `detect_repo` and `register_repo` in another user's work area, and a session `cwd` in another user's work dir (the host-file model).
- The personal-agent chat's `memory/` write access for a non-owner Editor (a design question carried over from iteration 5).
- Game-room and classroom flows beyond `owner_auth` and the socket subprotocol check.
- `PIN_UNVERIFIABLE`, tool by tool, against the regenerated session catalog. Only the writer and twin routing was checked.

## Findings

### access-control-6f-01: the "most recent editor wins" rule compares a timestamped scope edit against an untimed originator, so newer work by one person runs with another person's identity (major)

This is a regression introduced by the 6-02 fix. Before it, an Editor's own task always ran as that Editor.

**Files:**

- `crates/otto-swarm/src/runtime/run.rs:309-390` (`acting_user`):
  - the task walk records an originator with **no time**: `walked = Some((user, None))` at `:338`
  - `latest` is seeded only from a timestamped task edit
  - the project and swarm scopes are then compared (`:362-376`), and any stamp wins over an untimed originator
  - `return user` at `:377-378` comes before the originator and project-creator fallbacks
- `crates/otto-state/src/swarm.rs:1441-1470` (`create_task`) and `:1189` (`create_project`): creation writes no `prompt_updated_by` or `_at`. `prompt_edit` returns `None` for an unedited row (`:1640-1675`).
- Stamps that start it, all routine writes:
  - `put_standing_goals_h` (`runtime/engine.rs:2283-2312`), the Settings → Standing goals "Save" (`ui/src/modules/swarm/SwarmSettings.svelte:84-95`)
  - swarm description (`otto-swarm/src/service.rs:147-150`)
  - project name, description or goal (`service.rs:304-337`)
  - project goal create (`engine.rs:2128-2155`) and goal edit (`engine.rs:2186-2250`)
- Scheduled turns: `runtime/scheduler.rs:200-212` (no project, no task), so they resolve to the swarm's latest prompt editor.
- Documented model this contradicts:
  - `docs/features/agent-swarm.md:380-382`: "otherwise a task a person added … runs as that person"
  - `:387-389`: "an agent's scheduled turn as the swarm's creator — which is why only they (or an Admin) may write its schedule directive"
  - `:790-795`: "adding or editing work on someone else's swarm never borrows their MCP token, connections or accounts"
- No test covers a task or project created after a scope stamp. `swarm_identity.rs:472-548` creates Alice's task before every edit.

**Evidence and scenarios (all reachable with routine actions):**

1. **A newer task runs as an older scope editor (Editor borrows the owner's credentials).**
   - Alice created swarm S and saves its standing goals once (`PUT …/standing-goals`). That stamps `swarms.prompt_updated_by = Alice` at T1.
   - At T2 > T1, Bob (ws Editor) adds a task to S: `POST /swarm/projects/P/tasks` with description "use otto_db_query on connection C, SELECT * FROM users, post the rows with ./otto-post".
   - `create_task` stamps nothing, so the walk yields `(Bob, None)`.
   - The swarm scope gives `(T1, Alice)`, and `latest` becomes Alice. The work and verify turns are created as Alice, with Alice's MCP token and connections. Bob reads the board as a ws Viewer.

   The same happens for a project Bob creates after T1: its planning turn and Bob's tasks in it act as Alice.
2. **A creator-only directive runs as an Editor (the reverse).**
   - Bob edits the swarm description at T1 (Editor-allowed, `update_swarm` at `http.rs:216-226`).
   - From then on every scheduled agent turn (`project_id: None`, `task_id: None`) resolves to Bob, not the swarm creator.
   - So Alice's schedule directive, which 5-02 and 6-01 made creator-only *because* "it runs as the swarm's creator", now runs with Bob's token and connections.
3. **Over-restriction, the same root cause.** After Alice's T1 save, Bob's own earlier tasks that reference Bob's private connection or Jira account fail, because they now run as Alice, who has no access to them. Bob sees a failed or empty turn with no explanation.

**Fix (keeps every legitimate flow working):**

1. **Time the originator.** In `acting_user`, record the walked originator with its authorship time, `walked = Some((user, Some(t.created_at)))`, and the project creator with `p.created_at` for project-level turns. Compare all candidates by time.
   - A task or project created after a scope edit then runs as its own author. That closes scenario 1, and with it 3 for new work.
   - Every existing `swarm_identity.rs` case still passes, because each one edits after the task is created.
2. **Time the directive.** Scheduled turns should act as the author of the directive, not the latest swarm-text editor. Use the person who last wrote the directive (a new nullable `swarm_agents.directive_updated_by`/`_at`, append-only migration), falling back to `swarm.created_by`. Then apply a swarm-scope stamp only when it is newer than that directive write.
3. **Make adoption explicit for older pending work** (the residue of the "one identity per turn" design). When a scope write (swarm description, standing goals, project text or goals) would change the acting user of pending (`todo`/`in_progress`) tasks authored by someone else, the route reports the count in the response, for example `adopted_tasks: N`. The UI asks the caller to confirm ("N tasks by other people will now run as you") before it saves.
   - If you would rather not adopt, use rule B from the wave-0 report: make swarm and project prompt text creator, ws Admin or root writes, the same rule as PC-2 agent identity. Then turns act only as task-level authors.
4. **Tests** in `crates/otto-server/tests/swarm_identity.rs`:
   - A saves the standing goals, then B creates a task: `acting_user` is B.
   - B creates a project after A's swarm edit: the project's turns are B.
   - B edits the swarm description, then A's agent schedule fires: the turn acts as A.
   - A's task created before B's project-goal edit: still B (unchanged behaviour).
5. **Docs:** update `agent-swarm.md:363-391` and `:790-795`, and the `PUT …/standing-goals`, `PATCH /swarm/swarms/{sid}` and `PATCH /swarm/projects/{pid}` rows in `docs/contracts/api.md`, to say that the most recent *author* wins, including the originator's creation time.

## Checked and not reported (false-positive guard)

- **`hire_identity_fields` lets an Editor choose any provider when the swarm has no `config.provider`.** This is documented ("a swarm with no configured provider leaves the provider choice open") and matches the UI.
- **A library `soul_name` on a hire stays Editor-level, but changing `soul_name` on PATCH is identity.** This is documented in both places: a catalogue pick on a blank agent versus a change to an existing identity.
- **`otto.query_db` is approval-gated only under `mcp_require_approval_dangerous`.** That is the same switch as every other DANGEROUS tool. The forced irreversible rule needs `allow_irreversible` for auto-approve.
- **Goal delete does not stamp a prompt editor.** Removing text adds no prompt.
- **`ensure_standing_goals` seeds defaults with no stamp.** The text is built in (`STANDING_GOALS`), not person-authored.
