# Access control review: iteration 5

Lens: who can access what. That covers route auth and RBAC, the MCP outward tools, share, remote and mobile access, room and game-room guests, API tokens and workspace scoping.

Calibration: over-restriction (a legitimate user blocked, or checks that contradict the documented model) counts the same as a real, reachable leak. Every finding below was traced in the current code of `fix/quality-iter5-20261022` (HEAD `8177c4d85`). Theoretical hardening and loopback-only concerns are excluded. The review was read-only: no builds or tests were run.

The three findings share one pattern. Iterations 2–4 settled a consistent model: **work that runs as person A takes its prompt only from A, a workspace Admin or root** (personal-agent fields, schedule directives, scheduled-task prompts, workflow latest-changer identity, swarm agent identity). Three prompt sources still let another Editor's text run under A's identity.

## Score

Start at 10.0.

| ID | Severity | Deduction |
|----|----------|-----------|
| access-control-5-01 | major | -0.3 |
| access-control-5-02 | major | -0.3 |
| access-control-5-03 | major | -0.3 |

**Score: 10.0 - 0.3 - 0.3 - 0.3 = 9.1**

## Previous findings (iteration 4)

| ID | Status | Evidence |
|----|--------|----------|
| access-control-4-01 (swarm turns always ran as the swarm creator) | Fixed, with residual paths reported as 5-01 and 5-02 | `otto-swarm/src/runtime/run.rs:252-293` `acting_user` walks the task, its parents and `source-run:` labels to a person, then the project creator, then the swarm creator. `find_agent_session` is keyed by (agent, acting user) (`run.rs:232-236`). Verification turns use the same identity (`verify.rs:1008`). `update_agent` requires the swarm creator or a ws Admin for prompt, soul, provider, model and cwd (`http.rs:250-279`, `identity_changes` at `:283`). |
| access-control-4-02 (any Editor could rewrite another user's personal-agent goals, rules and memory items) | Fixed | `routes/personal_agents/autonomy.rs:175-193` `owner_only_autonomy_changes` compares goal and rule texts (order ignored). `save_autonomy` calls `require_agent_owner` with them (`:212-218`). `edit_memory` requires the owner (`:433`). `run_goal` stays at Editor (`:243-252`), as recommended. Documented in `api.md:4712`, `:4716` and `personal-agents.md:300-310`. |
| access-control-4-03 (non-root owners got 403 on their own automation sessions' Files tab and LSP) | Fixed | `routes/session_files.rs`: `GET /sessions/{id}/files/{browse,read}` behind `session_gate` (owner-or-admin + ws Viewer). Paths go through `confine_join` plus a canonical-prefix check. A non-root caller is still refused `otto.db*`, `secrets.json`, key files and protected dirs unless the cwd is one session work dir strictly inside an agent work area (`is_session_work_dir`, `:79-83`, `:88-104`). `/ws/lsp` accepts such a root via `lsp_root_in_own_session` before `secret_root_denial` (`lsp/mod.rs:308-339`). |
| access-control-4-04 (the personal-agent page offered owner-only editors to every Editor) | Fixed | `AgentPage.svelte:379` `canEditOwnerFields` (root, ws admin, owner or an ownerless agent) gates the memory inspector, documents and the goal and rule texts (`:503`, `:688`, `:697`), with an owner note (`:381`). |

## Areas covered (this iteration)

- Re-verified all four iteration-4 findings (see the table above), including the new `session_files.rs` confinement and the LSP session-root path.
- The swarm identity fix traced end to end through every prompt source a turn reads:
  - task title and description (`run.rs:158`)
  - the agent schedule directive (`run.rs:458-481`)
  - task and project goals (verification prompts, `verify.rs:1146-1151`)
  - board context
  - channel-trigger launches (`channels.rs:296-311`)
  - `plan`, which runs as the caller (`engine.rs:3101-3190`)
- Personal-agent run paths: `run_now` with a schedule id (same-agent check), `run_goal`, `chat_session` (a non-owner's chat runs as the caller, `personal_agents.rs:887-960`), `reset` and delete (Editor, as documented in `api.md:4650` and `:4717`), and schedule create and update (owner-only directive and permission, `:537-620`).
- Assistant delegation (`routes/assistant.rs:329-344`, `otto-assistant/src/assistant/tasks.rs:343-437`) and `spawn_directive_run` (`personal_agents_engine.rs:650-690`).
- Scheduled-task `owner_only_changes` (`routes/scheduled_tasks.rs:272`, `:512-525`).
- Game rooms (new):
  - `game_rooms/http.rs`: `owner_auth` on create, public join authorized by the invite in the body
  - `registry.rs`: invites are hashed and atomic under the room lock, expire after ten minutes, have capacity caps, and are lobby-only
  - `socket.rs`: exactly the `otto-game` subprotocol plus a 43-char token, no query string, and an owner bearer is not accepted
  - Policy `/game-rooms` at Agents:View, matching `game-rooms.md`
- The Classrooms home box: it reads `/sessions` (owner-scoped for non-admins, `otto-sessions/src/http.rs:344-387`) and `/sessions/{id}/screen` (ws Viewer + owner-or-admin), so it shows nothing the caller cannot open.
- MCP profiles (new):
  - CRUD at Mcp:Admin + a person's credential (`policy.rs:54-57`)
  - `PATCH /sessions/{id}` `meta.mcp_profiles` is person-only
  - the UI shows the picker and session menu only with `mcp:view` (`NewSession.svelte:664`, `SessionView.svelte:359`), so a user without the grant never sees a 403
- MCP outward Jira create tools (new `issue_route`, `otto-mcp/src/outward/exec.rs:266-386`): every arm self-calls an `otto-issues` route that runs `load_authorized_account` (`otto-issues/src/http.rs:96-104`, `:893-933`).
- Kubernetes `allowed()` operation matrix and namespace child scopes:
  - `otto-k8s/src/access.rs`
  - `http.rs:404-760`: list, detail, logs, selector logs, metrics, pod containers, pod HTTP, actions
  - broad (`child=None`) calls need an all-children allow (`otto-rbac/src/resource_access.rs:280-288`)
- AWS `allowed()` and bucket child scopes (`otto-aws/src/access.rs`). Bucket-only grants still discover the account through `parent_discovery` (`resource_access.rs:194-245`), so they are not over-restricted.
- Design Hall flat item routes: `load_artifact` and `load_project` check the row's workspace; `create_link` checks Viewer on the destination (`otto-design/src/http.rs:331-357`, `:1155-1166`).
- Product flat routes: versions, analyses, questions, notes, test cases, test-case runs, learnings and transcripts all resolve the workspace from the story row (`otto-product/src/http.rs:574-1225`).
- Vault recovery routes (trash, history, revision, asset): ws role per route. Vaults are global by documented design (`otto-vault/src/engine.rs:331-345`).
- Plugin host API (`plugins.rs:415-1093`): sidecar-token auth; admin routes are root-only plus `require_human` on install. A root-installed plugin receiving any Jira account's credentials is the documented host-trust model.
- Backup, state archive and connection export (`routes/backup.rs`, `routes/connection_export.rs`): root-only, with `require_human` on restore, import and password export, plus a fresh root re-check (`fresh_root`).
- Workflow `approve_run`: `require_human` + ws Editor, matching the documented Editor model.
- Mission Control / workgraph routes: every item is read with `(wid, id)`.
- `POST /sessions/{id}/ui-control`: only the owner may grant (in person, not impersonated); the owner, a ws Admin or root may revoke.
- Handover routes: owner-or-admin on both the source and target sessions.

## Areas not covered yet (next iteration)

- **The personal-agent chat as a memory writer.** A non-owner Editor's chat with A's agent runs as the caller in A's persona folder. The agent is told to write `[chat]` bullets to `memory/notes.md`, which A's later runs read. That is agent-mediated (a documented Editor surface, "chat anytime"), so it was not raised. Whether the chat session's sandbox should get write access to `memory/` for a non-owner is a design question for the owner.
- **Agent rooms** (`otto.room_post`, "you can post into any room"). These are the same kind of agent-mediated input to agents that run as their owners. They were not traced.
- MCP outward `route_for`, the remaining arms one by one: k8s, aws, swarm, design, vault and browser. `PIN_UNVERIFIABLE` and `NATIVE_SESSION_WRITERS` tool by tool.
- `otto-git` `detect_repo` and `register_repo` on a path inside another user's data-dir work area. This is the host-file model, as before. The same applies to a non-root person creating a session whose `cwd` is another user's work dir: `unsafe_session_cwd` allows it, and `session_files` then serves that tree. The caller's own shell already reads it.
- The Swarm UI "Open session" for non-originators, now that sessions are per acting user. The server answers 403 as documented, but the UI was not checked for offering the button.
- Session rooms (terminal rooms) were not re-walked this iteration. Only game rooms were.

## Findings

### access-control-5-01: an Editor's edit to another person's swarm task (title, description, labels or goals) runs under the task originator's identity (major)

**Files:**

- `crates/otto-swarm/src/http.rs:397-408` (`update_task`: ws Editor only, with no originator or latest-editor record)
- `crates/otto-swarm/src/service.rs:370-400` (`TaskPatch` writes `title`, `description` and `labels` as sent)
- `crates/otto-swarm/src/runtime/engine.rs:2789-2795` (`run_task`: ws Editor)
- `crates/otto-swarm/src/runtime/run.rs:252-293` (`acting_user` returns `task.created_by` when it is a person) and `:363`
- `crates/otto-swarm/src/runtime/run.rs:158` (`TASK: {title}\n{description}` is the turn prompt)
- `crates/otto-swarm/src/runtime/engine.rs:2058-2065` (`create_task_goal`) and `:2134-2141` (`update_goal_h`), both ws Editor
- `crates/otto-swarm/src/runtime/verify.rs:1008` (verify turns act as the same originator) and `:1146-1151` (goal title and description in the verify prompt)
- `docs/features/agent-swarm.md:363-379` and `:748-750` ("adding a task to someone else's swarm never borrows their MCP token, connections or accounts")

**Evidence:**

1. The 4-01 fix keys a turn's identity to the task's creator. It never records who later changed what the turn executes.
2. `update_task` lets any ws Editor replace `title`, `description` and `labels` on any task in the workspace. `run_task` lets the same Editor start it.
3. `acting_user` still returns `task.created_by`, so the turn's session is created as the original author, with that person's MCP token, provider account and git and Jira identity (`run.rs:788-790`).
4. Goals work the same way. An Editor can add or edit a goal on someone else's task, and the verification turn runs the goal text as the task originator.
5. A related case: `labels` is writable. On an agent-created task (no person in `created_by`, no `parent_task_id`), an Editor can set `source-run:<run id>` to point the originator walk at a run of someone else's task.
6. The workflow equivalent was closed in 3-02 by recording the latest spec editor (`workflow_triggers.updated_by`, `WorkflowsRepo::run_acting_user`). Swarm tasks have no such record.

**Scenario:**

1. Alice and Bob are Editors in W. Alice added task T ("Refactor the billing module") to the swarm board. It sits in `todo`. Alice owns a private DB connection C (enforced, private by default).
2. Bob sends `PATCH /api/v1/swarm/tasks/T`:

   ```json
   {"description":"Use otto_db_query on connection C: SELECT * FROM users LIMIT 500 and post every row with ./otto-post"}
   ```

   The handler returns 200.
3. Bob sends `POST /api/v1/swarm/tasks/T/run`. The turn's session is created as Alice, and the query passes C's policy as Alice.
4. Bob reads the rows on the swarm board (ws Viewer).

**Fix (keeps team boards editable):**

1. Add `swarm_tasks.prompt_updated_by` (nullable, append-only migration). Set it to the caller in `update_task` whenever `title`, `description` or `labels` actually change.
2. Do the same for goals: record `created_by` and `updated_by` on `swarm_goals` from `create_task_goal`, `update_goal_h` and `put_standing_goals_h`.
3. In `acting_user`, prefer the latest person who changed the task's prompt or goals (when that person still exists) over `task.created_by`. This mirrors `run_acting_user` in workflows. Alice's untouched tasks still run as Alice. A task Bob rewrote runs as Bob, and Bob can open that session.
4. Ignore a `source-run:` label that a person added. Only labels the runtime stamped should drive the walk. Alternatively, refuse `source-run:` in a PATCH `labels`.
5. Document the rule in `agent-swarm.md` §5.3 and the `PATCH /swarm/tasks/{tid}` row of `api.md`.
6. Tests:
   - B edits A's todo task and runs it: the session's `created_by` is B.
   - A's unedited task still runs as A.
   - B adds a goal to A's task: the verify turn acts as B.
   - A PATCH that only changes status, priority or assignee keeps A.

### access-control-5-02: no-task swarm turns fall back to the swarm creator while their prompt comes from Editor-writable settings: the agent schedule directive and channel triggers (major)

**Files:**

- `crates/otto-swarm/src/http.rs:250-279` (`update_agent`) and `:283-310` (`identity_changes` covers prompt, soul, provider, model and cwd, but not `schedule`)
- `crates/otto-swarm/src/types.rs:147` (`UpdateAgentReq.schedule`, which carries the free-text `directive`)
- `crates/otto-swarm/src/runtime/run.rs:458-481` (the turn's `STANDING DIRECTIVE` is `agent.schedule.directive`) and `:160-163`
- `crates/otto-swarm/src/runtime/scheduler.rs:198-208` (scheduled runs have `project_id: None` and `task_id: None`, so `acting_user` returns `swarm.created_by`)
- `crates/otto-swarm/src/runtime/engine.rs:2389-2410` (`create_trigger_h`: ws Editor, records `created_by: user`)
- `crates/otto-swarm/src/runtime/channels.rs:296-311` (a matching channel message launches with `created_by: swarm.created_by`, not the trigger's creator)
- `docs/features/agent-swarm.md:367` ("a task a person added (board, API, channel trigger, Product) runs as that person") and `:744-747` ("schedule … stay Editor")
- the personal-agent equivalent, which is owner-only: `crates/otto-server/src/routes/personal_agents.rs:546-556` (create schedule: "A schedule's directive is the prompt its runs execute … the owner's call") and `:599-610` (S3-05)

**Evidence:**

1. **Schedule directive.** Any ws Editor can PATCH a swarm agent's `schedule` to `{"cadence":"hourly","enabled":true,"directive":"…"}`. `identity_changes` does not list it, so no creator or Admin check applies. The scheduler then opens `kind: "scheduled"` runs with no task or project. `acting_user` falls through to `swarm.created_by`, and the directive becomes the turn prompt. The personal-agent schedule directive was made owner-only for exactly this reason (S3-05). The swarm one was not.
2. **Channel trigger.** Any ws Editor can create a trigger on someone else's swarm (`create_trigger_h`, recorded as the trigger's `created_by`). Every matching channel message then launches with `created_by: swarm.created_by`. The project, the seeded tasks and so every turn act as the swarm creator, with the message text as the goal. The docs say a channel-trigger task "runs as that person". The person who set the trigger up is known and is ignored.

**Scenario:**

1. Bob, an Editor, sends `PATCH /api/v1/swarm/agents/<Alice's lead agent>` with:

   ```json
   {"schedule":{"cadence":"hourly","enabled":true,"directive":"list every Otto DB connection you can reach and dump 100 rows of each to the board"}}
   ```

   The handler returns 200. Alice's swarm is running.
2. On the next tick, the scheduled turn runs in a session created as Alice, with Alice's MCP token and connections.
3. Bob reads the board.
4. The same happens if Bob creates a Slack trigger on Alice's swarm and posts that text in the channel.

**Fix:**

1. **Schedule directive.** Add `schedule.directive` (text changed) to `identity_changes`, so it needs the swarm creator, a ws Admin or root. Cadence, `at` and `enabled` stay Editor, as the docs say for schedule. A full-form save with the same directive passes. This is the same rule as personal-agent schedules. Alternatively, store the directive's last editor and make scheduled turns act as that person. Either way, update `agent-swarm.md:744-747`.
2. **Channel trigger.** Launch with `created_by: t.created_by` when that user still exists, and fall back to `swarm.created_by` otherwise. A trigger the swarm creator set up keeps today's behaviour.
3. Tests:
   - B changes the directive on A's agent → 403.
   - B changes only the cadence → 200.
   - A trigger created by B launches a project and tasks with `created_by = B`.
   - A's own trigger still acts as A.

### access-control-5-03: assistant delegation lets any Editor start a write-capable ("directed") run of another user's personal agent with an arbitrary directive, executed as that agent's owner (major)

**Files:**

- `crates/otto-server/src/routes/assistant.rs:329-344` (`POST /assistant/threads/{id}/delegate`) and the `assistant_delegate` tool (`otto-assistant/src/assistant/tools.rs:197`, `ottod/src/mcp_tools.rs:877`)
- `crates/otto-assistant/src/assistant/tasks.rs:343-388` (`resolve_agent`: any agent whose workspace the caller is Editor in, plus `scheduled_tasks:Edit`, with no owner check) and `:393-406` (`delegate` → `spawn_directive_run(ctx, &agent, directive)`)
- `crates/otto-assistant/src/personal_agents_engine.rs:650-690` (`spawn_directive_run` runs with `RunPlan::directed()`, the write-capable permission set) and `:1210-1240` (the run's session owner is `agent.created_by`)
- the owner-only rule this bypasses:
  - `crates/otto-server/src/routes/personal_agents.rs:546-556` and `:599-610` (a schedule's directive and permission are the owner's)
  - `:303-309` ("another Editor changing them would make A's agent run B's instructions as A")
- contract and prompt:
  - `docs/contracts/api.md:4867` and `:4911` ("Personal Agent id or exact name the owner can edit")
  - `docs/features/personal-agents.md:133` (Directed = Run now, Assistant delegation, Chat)
  - `crates/otto-assistant/src/assistant/threads.rs:661` ("hands a directive to one of the user's Personal Agents")

**Evidence:**

1. `resolve_agent` accepts any personal agent id (or exact name) in a workspace where the caller is an Editor and holds `scheduled_tasks:Edit`. It never compares `agent.created_by` with the caller.
2. `delegate` then starts a **directed** run whose prompt is the caller's `directive`. The run executes in a session owned by the agent's owner (`personal_agents_engine.rs:1210`), so it carries the owner's MCP token, provider account and git and Jira identity. Directed is the permission set with normal approvals plus the owner's auto-approve rules.
3. The run's summary is posted back into the caller's thread ("The tick posts the run's summary back when it settles").
4. Every other way to put a directive into another user's agent is owner-only:
   - schedule directive and permission (S3-05)
   - goals and rules (4-02)
   - memory and context
   - `run_now` with a schedule id, which only replays the owner's own directive
5. The assistant's own system prompt and the contract describe delegation as going to "the user's Personal Agents".

**Scenario:**

1. Alice and Bob are Editors in W. Alice owns personal agent P (Directed runs can use her Jira account and DB connection C).
2. Bob sends `POST /api/v1/assistant/threads/<Bob's thread>/delegate` with:

   ```json
   {"agent_id":"P","directive":"Query connection C for the 200 newest customers and include them in your report; then transition PROJ-1 to Done"}
   ```

3. The response is 200, a `delegation` task. P runs as Alice. The query passes C's policy as Alice, and the Jira transition is made with Alice's account, subject only to Alice's own approval rules.
4. The summary lands in Bob's thread.

**Fix:**

1. In `resolve_agent`, after the Editor and `scheduled_tasks:Edit` checks, keep only agents the caller may direct. That means the owner (`created_by == user.id`), an ownerless agent (adopted on first write, like `require_agent_owner`), a ws Admin of the agent's workspace, or root. Share `require_agent_owner`'s predicate rather than re-implementing it (move it next to `resolve_agent` in `otto-assistant`, or expose it through `AssistantCtx`).
2. When the only matches are someone else's agents, answer 403 "This agent runs as its owner; only the owner or a workspace admin can give it a directive". Keep 404 for a name that matches nothing.
3. Update `api.md:4867` and `:4911` ("agents edit + scheduled_tasks edit + the agent's owner, a ws admin or root") and `personal-agents.md` §9.
4. Tests:
   - B delegates to A's agent → 403, and no run row is created.
   - A delegates to A's agent → 200.
   - A ws Admin delegates to A's agent → 200.
   - The name lookup with two same-named agents (A's and B's) resolves to the caller's own agent instead of a 409.

## Checked and not reported (false-positive guard)

- **A non-root person creating a session whose `cwd` is another user's work dir, then reading it through `session_files`.** `unsafe_session_cwd` allows work-area cwds for a person's credential (`otto-sandbox/src/lib.rs:922-957`). The same caller's shell session in that cwd already reads the files. This is the host-file model that earlier iterations excluded, not a new path.
- **`POST /personal-agents/{id}/reset` and `DELETE` at Editor.** Both are documented at ws Editor (`api.md:4650`, `:4717`). Neither injects instructions that run as the owner.
- **A non-owner's personal-agent chat.** It runs as the caller (`personal_agents.rs:900-927`), so it borrows no identity. Its memory side effect is listed under "not covered".
- **Swarm `plan`.** Planner turns run as the caller (`engine.rs:3161-3175`), and seeded tasks are stamped with the plan caller.
- **AWS bucket-scoped grants.** `check` first requires account `discover` with no child, but `parent_discovery` grants it from a child-scoped `discover` rule, so bucket-only users are not locked out.
- **Kubernetes cluster-wide list with a namespace-only grant (403).** Correct: a broad call needs an all-children allow, and the namespace picker uses `namespaces_allowing_any`.
- **The plugin sidecar receiving any Jira account's credentials, and `agents/run`.** Plugins are root-installed host code (`require_root` + `require_human` on install).
- **Vaults visible from every workspace.** This is the documented "global library" design (`engine.rs:331-345`).
- **Workflow `approve_run` by a non-starter Editor.** It is documented at Editor, `require_human` refuses the supervised agent's own token, and the approver is recorded.
- **MCP profiles CRUD needing Mcp:Admin.** A profile only narrows tools. Reads need `mcp:view`, and the UI hides the picker without it, so no legitimate flow hits a 403.
