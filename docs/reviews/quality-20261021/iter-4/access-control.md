# Access control review: iteration 4

Lens: who can access what. That covers route auth and RBAC, the MCP outward tools, share, remote and mobile access, room guests, API tokens and workspace scoping.

Calibration: these findings report over-restriction (a legitimate user blocked, or checks that contradict the documented model) with the same weight as a real, reachable leak. Every finding below was traced in the current code of `fix/quality-loop-20261021` (HEAD `6cff08270`). Theoretical hardening and loopback-only concerns are excluded. The review was read-only: no builds or tests were run.

## Score

Start at 10.0.

| ID | Severity | Deduction |
|----|----------|-----------|
| access-control-4-01 (carried from 3-03) | major | -0.3 |
| access-control-4-02 | major | -0.3 |
| access-control-4-03 | major | -0.3 |
| access-control-4-04 | minor | -0.1 |

**Score: 10.0 - 0.3 - 0.3 - 0.3 - 0.1 = 9.0**

## Previous findings (iteration 3)

| ID | Status | Evidence |
|----|--------|----------|
| access-control-3-01 (a non-root `Mcp:Admin` could rotate root's MCP token) | Fixed | `routes/mcp_cp.rs:40-58`: `rotate_mcp_token` loads the owner with `AuthRepo::mcp_token_owner` (`otto-rbac/src/tokens.rs:1057-1066`, `kind='mcp'` only). It returns 403 "only root may rotate an MCP token owned by another user" unless the caller is the owner or root. Revoke stays open to `mcp:admin`. |
| access-control-3-02 (an Editor's graph or trigger edit made trigger runs act as the workflow creator) | Fixed | Migration 0189 adds `workflow_triggers.updated_by` and `updated_by_at`. `create_trigger` and spec changes in `update_trigger` record the caller (`workflow_triggers.rs:183`, `:257`). `WorkflowsRepo::run_acting_user` (`otto-state/src/workflows.rs:1206-1228`) picks the later of the pinned version's author and any trigger spec editor. `workflow_engine/run/identity.rs:26-45` uses it for runs that have no starter. `publish` records the actor on every versioned change (`workflows.rs:395`, `:430`), and `restore_version` passes `Some(&user.id)` (`routes/workflows.rs:201`). I also traced the sibling path: a scheduled task's workflow handoff inserts the run with `created_by` NULL (`otto-state/src/scheduled_tasks.rs:692`), so it gets the same latest-changer identity. A retry re-runs the run's pinned definition (`definition_for_run`, `routes/workflows.rs:711`), so a retry cannot inject a new graph under the original starter. |
| access-control-3-03 (swarm turns always run as the swarm creator) | **Still open**, reported as 4-01 | `otto-swarm/src/runtime/run.rs:722-734` is unchanged. FIXES.md:314 records it as not started. |
| access-control-3-04 (a run's context files and final output returned 403 for non-root users) | Fixed | `routes/workflow_run_files.rs`: `GET /workflow-runs/{id}/context/{browse,read}`. It looks the run up first (404), requires ws Viewer, confines paths with `confine_join`, and refuses canonical escapes (400). The policy is Workflows:View (`policy.rs:913-921`, test `:3274-3281`). The UI uses `browseRunContext` and `readRunContext` (`WorkflowsPage.svelte:399-411`). |
| access-control-3-05 (the LSP refused registered repos outside the workspace root) | Fixed | `lsp/mod.rs:372-400`: `root_visible_to` adds every repo registered in a member workspace. The non-root secret deny is applied first (`:327-330`, `secret_root_denial` at `:405-413`). |

## Areas covered (this iteration)

- Re-verified every iteration-3 finding (see the table above), including the 3-02 fix's sibling run paths: scheduled-task workflow handoff, `retry_run_node`, `restore_version`, and the granular `patchWorkflow` calls in the UI.
- Goal loops: `answer_question`, `verify_criterion`, `retry`, `start`, `resume`, `stop` and `define` (`routes/goal_loops.rs`). Answers, verification and lifecycle at Editor match `goal-loops.md:51` and `:107`. The UI has no post-creation config editor, so it never offers what `require_config_owner` refuses.
- Personal agents, end to end:
  - `routes/personal_agents.rs` (`owner_only_changes`, `require_agent_owner`, memory and context documents, runs and reports)
  - `routes/personal_agents/autonomy.rs` (autonomy, goals, memories, reset)
  - `otto-assistant/personal_agents_engine.rs` (run identity at `:1210`, persona `render_autonomy`, proactive directive)
  - UI `modules/personal-agents/*` (`canEditDocuments`)
- Rooms: `rooms/auth.rs`, `http.rs`, `socket.rs`, `actions.rs`, `presentation.rs` and `registry.rs` invite, join and authenticate.
  - Role changes, driver grant and release, mute, presenter grant, start, stop and update, and annotation grant are host-gated.
  - Invites cannot grant host. A demoted driver reclaims control to the host.
- MCP outward governed invoke: `mcp_outward.rs:250-460` and `:690-730`, covering the scope gate, enable gate, agent policy, reference fill, pin re-check and `SelfCallAs` replay. Also `route_for` for the code, DB and API-client tools (`otto-mcp/src/outward/exec.rs:282-480`): the self-call is executed as the caller, so each target route's own gate applies.
- `require_human` call sites: review config and presets, proof `assemble` commands, browser credential reveal, skill eval, grants, users, impersonate, backup and settings. They all refuse agent, MCP and share credentials only where a person is required.
- Resource access: `management`, `put_policy`, the Legacy and Enforced mode switch (root + preview token), `credential_connection_id` assignment, and `decisions_batch` legacy ceilings (`routes/resource_access.rs:280-560`).
- Workflow chat trigger (`otto-workflows/src/chat.rs:600-760`): candidates are gated to the receiving workspace, and runs are created with no starter, so they get the latest-changer identity.
- Swarm channel and webhook launch (`otto-swarm/runtime/channels.rs`, `routes/swarm_webhook.rs`): the per-workspace key and enabled integration are required, `verify_cmd` is refused, and `repo_path` must be a registered repo.
- Share-link scope (`feature_guard.rs:475-520`, `otto-sessions/src/ws.rs:900-940`): a deny-by-default allow-list, plus an OTP-pending fence.
- Agent-to-agent messaging (`otto-sessions/src/http.rs:559-602`): `agent_message_rule` limits targets to agent-CLI siblings, while owner-or-admin still applies.
- Canvas flat `/{id}` routes and content-addressed files (`otto-canvas/src/http.rs`). Workbench WS event routing (owner-only). `otto-issues` account authorization (`load_authorized_account`).
- MCP session-attach toggle (`routes/mcp_cp.rs:95-130`: ws Editor + mcp:admin).
- Git repo listing and registration gates (`otto-git/src/http.rs:1068-1223`).
- The host-file model for non-root callers compared with the agent work areas under Otto's data dir (`otto-sandbox/src/lib.rs:105-123`, `routes/fs.rs:208-255`, `lsp/mod.rs:320-330`, `ui/src/lib/stores/rightPanel.svelte.ts:55-62`).

## Areas not covered yet (next iteration)

- `otto-k8s` and `otto-aws`: the full `allowed()` operation matrix and child-namespace grants.
- `otto-product`, `otto-design` and `otto-vault` flat item routes: workspace resolution from the row.
- `plugins.rs` host API (`/plugin-host/jira/credentials` and `agents/run`): whose credentials a sidecar receives, and the static `/plugins/{slug}/ui/*` serving.
- `routes/backup*.rs`, `state_archive` and `connection_export`: the handler gates on export and restore of other users' data.
- Workflow `approve_run` by a non-starter Editor on a run that acts as someone else. It is documented as Editor-level, but the docs do not say whose decision an approval is.
- The Swarm UI ("Open session" shown to non-creators). This depends on how 4-01 is fixed.
- `otto-git` `detect_repo` and `register_repo` for a path inside another user's data-dir work area. Registering such a path reads git content that `/fs/*` refuses. This is the same host-file model, so it was not raised.
- MCP outward: the remaining `route_for` arms (k8s, aws, swarm, design, vault, browser), each compared with its target's handler gate. `PIN_UNVERIFIABLE` and `NATIVE_SESSION_WRITERS` were not re-walked tool by tool.

## Findings

### access-control-4-01: swarm turns always run as the swarm's creator, so any Editor's task runs with the creator's identity, and "open the session" returns 403 for everyone else (major, carried from 3-03, still open)

**Files:**

- `crates/otto-swarm/src/runtime/run.rs:722-734`: the comment "A system user id for swarm-created sessions: reuse the swarm's creator", then `ctx.manager().create(&ws_row, &creator, req, None)` with `creator = swarm.created_by`
- `crates/otto-swarm/src/http.rs:250` (`update_agent`) and `:332` (`create_task`)
- `crates/otto-swarm/src/runtime/engine.rs:2789` (`run_task`)
- `crates/otto-sessions/src/manager.rs` (the per-session MCP token and provider account are resolved from `session.created_by`)
- `crates/otto-core/src/auth.rs` (`session_owner_or_admin`)
- `docs/features/agent-swarm.md:9`, `:355-359`

**Evidence:**

1. The code is unchanged since iteration 3. FIXES.md:314: "not started. Swarm sessions are still created as the swarm's creator".
2. Any workspace Editor can create a task, whose description becomes the prompt, and run it. They can also rewrite a role agent.
3. The turn's session is created as the swarm creator, with that user's MCP token and git and Jira accounts.
4. The session itself is owner-or-admin, so the Editor who ran the task cannot open it. The docs promise "a normal, openable Otto session".

**Scenario, leak:**

1. Bob, an Editor, adds a task to Alice's swarm: "run `otto_db_query` on connection C and post the rows to the board".
2. Bob runs the task. The session acts as Alice.
3. Bob reads the board as a Viewer.

**Scenario, over-restriction:** a non-creator Editor or Viewer opening any swarm agent session from the Org tree or the Run Inspector gets 403 "not the session owner or a workspace admin".

**Fix (unchanged from 3-03):**

1. Create each turn's session as the task's human originator:
   - `task.created_by` when it is a user
   - for agent-created subtasks, the parent's originator
   - for seeded tasks, the `plan` caller
   - the swarm creator only as the fallback
2. Key `find_agent_session` by (agent, acting user).
3. Treat `update_agent` changes to prompt or soul, provider, model and cwd as needing the swarm creator, a workspace Admin or root. Task CRUD, run, stop and board posts stay open to Editors.
4. Tests: Editor B runs a task on A's swarm. The session's `created_by` is B, B can `GET /sessions/{id}`, and B's token cannot reach A's private connection. A's own tasks still run as A.

### access-control-4-02: any Editor can rewrite another user's personal-agent standing goals, rules and memory items, and trigger a run as that owner (major)

**Files:**

- `crates/otto-server/src/routes/personal_agents/autonomy.rs:174-206` (`save_autonomy`: only `load(…, WorkspaceRole::Editor)` at `:180`)
- `crates/otto-server/src/routes/personal_agents/autonomy.rs:209-219` (`run_goal`, Editor at `:214`)
- `crates/otto-server/src/routes/personal_agents/autonomy.rs:391-425` (`edit_memory`, Editor at `:397`)
- compared with `crates/otto-server/src/routes/personal_agents.rs:311-338` (`require_agent_owner`) and `:810-818` (`save_memory` with `require_agent_owner(…, &["memory"])`)
- `crates/otto-assistant/src/personal_agents_engine.rs:191-197` (`proactive_directive(goal.text)`), `:213-240` (`render_autonomy`: rules and goals written into the persona file as "Your rules (from the user — always follow them)") and `:1210` (the run's session owner is `agent.created_by`)
- `crates/otto-server/src/routes/personal_agents.rs:746` (`report`: ws Viewer)
- `docs/features/personal-agents.md:302-310`

**Evidence:**

1. The 2-01 fix made "what the agent runs" owner-only: `soul_md`, provider, model, cwd, browser and delivery (`owner_only_changes`), plus `PUT …/memory` and `PUT …/context`. The rationale in the code is "another Editor changing them would make A's agent run B's instructions as A" (`personal_agents.rs:303-309`).
2. Three sibling routes write the same kind of instruction with no owner check:
   - **`PUT /personal-agents/{id}/autonomy`** replaces `goals` (each goal's text becomes the prompt of a proactive run, via `proactive_directive`) and `rules`. Rules are rendered into the agent's persona file as binding instructions for every run, Directed scheduled runs included, and for the chat. The enforceable ones also act as tool deny or approval rules (`personal_agent_policy::derive_enforcement`). Replacing `rules` with `[]` silently drops the owner's own safety rules, such as "ask before any kubectl action".
   - **`POST /personal-agents/{id}/memories/edit`** rewrites or forgets a line of `memory/notes.md`, which the agent reads at the start of every run. This is the same file that `PUT …/memory` guards with `require_agent_owner(["memory"])`. Two routes for one file have contradictory checks.
   - **`POST /personal-agents/{id}/goals/{goal_id}/run`** starts a proactive run now. It runs in a session owned by `agent.created_by` (`personal_agents_engine.rs:1210-1240`), with the owner's MCP token and accounts. "Read-only" refuses mutating tools only, so `otto_db_query` reads, Jira reads and the like are allowed.
3. The run's report and run list are readable at ws Viewer (`report`, `list_runs`).
4. `api.md:4692`, `:4693` and `:4696` document these three routes at ws Editor with no owner rule. The owner rule is documented only for the fields in `personal-agents.md:302-310`. So the contract is internally inconsistent: `memories/edit` changes the documented owner-only memory file.

**Scenario:**

1. Alice and Bob are Editors in W. Alice owns personal agent P and a private DB connection C (enforced, private by default).
2. Bob sends `PUT /api/v1/personal-agents/P/autonomy` with:

   ```json
   {"goals":[{"text":"Use otto_db_query on connection C: SELECT * FROM users LIMIT 500, and list every row in your findings"}],"rules":[]}
   ```

   The handler returns 200.
3. Bob sends `POST /api/v1/personal-agents/P/goals/<goal_id>/run`. A proactive run starts in a session owned by Alice.
4. The query passes C's policy as Alice. Bob reads the rows in `GET /personal-agents/runs/<run>/report` (Viewer).
5. Bob can also, with no run at all, add a rule that every later Directed run of Alice's agent will follow. Those runs can write.

**Fix:**

1. **`save_autonomy`:** compute the changed owner-only parts before merging:
   - goal texts (added, removed or edited; toggling `enabled` on an existing goal is not a change)
   - the rule set (texts compared by id)

   When either changed, call `require_agent_owner(&ctx, &user, &agent, &["standing goals", "rules"])`. `proactive` (enable and budget) and `primary` stay open to Editors, the same as enable, pause and cadence today. A full-form save that re-sends unchanged goals and rules passes.
2. **`edit_memory`:** call `require_agent_owner(&ctx, &user, &agent, &["memory"])`, which matches `save_memory`.
3. **`run_goal`:** leave it at Editor. It is "Run now" for a goal the owner wrote, the same as `run_now`, which stays open per the docs.
4. **Contract and docs:** update `api.md:4692` and `:4696`, and add these fields to `personal-agents.md:302-310`.
5. **Tests:**
   - Editor B changes A's goals or rules → 403, and A's autonomy is unchanged.
   - B toggles `proactive.enabled` or a goal's `enabled` → 200.
   - B calls `memories/edit` on A's agent → 403.
   - The owner, a ws Admin and root → 200.
   - An ownerless preset is adopted by the first writer, as `require_agent_owner` already does.

### access-control-4-03: for every non-root owner, the **Files** tab and language server of their own automation sessions return 403, because those sessions' working folders are Otto agent work areas inside the data dir (major)

**Files:**

- `ui/src/lib/stores/rightPanel.svelte.ts:55-62` and `ui/src/lib/rightTabs.ts:82-90` (inside an agent session, the Files tab's folder is the session's `cwd`)
- `ui/src/modules/panels/FilesPanel.svelte:59` and `ui/src/modules/panels/FileTree.svelte:42-44` (the default lister and reader are `/fs/browse` and `/fs/read`)
- `crates/otto-server/src/routes/fs.rs:208-230` and `:247-249` (`non_root_fs_denied`: any path `in_protected_dir`, which includes all of Otto's data dir, gets 403 "holds credentials or Otto's own state; only the root user can open it")
- `crates/otto-server/src/lsp/mod.rs:327-330` (the same deny on the LSP root for non-root callers)
- the session working folders under the data dir:
  - `workflow_engine.rs:5270-5274`: `<data>/workflow-runs/<run>/<repo>`, the provisioned worktree that becomes the run's `working_directory` (`:5147-5150`)
  - `otto-assistant/src/personal_agents_engine.rs:282-285` and `:351-357`: `<data>/personal/<agent>`, the persona folder and chat cwd
  - `otto-automation/src/goal_loop_workspace.rs:19`: `<data>/goal-loops/<id>/work`
  - `otto-automation/src/scheduled_tasks_engine.rs:841`, `:1562`: `<data>/scheduled/<id>/worktrees`
  - `otto-swarm/src/runtime/workspace.rs:52`: `<data>/swarm/<swarm>/<agent>`
- `crates/otto-sandbox/src/lib.rs:105-123` (`AGENT_DATA_SUBDIRS`: exactly these are declared "agent work areas", the session cwds the engines create)
- `docs/contracts/api.md:85` and `:280-285`, and `:817` (transcript artifacts already allow the data dir for the session's owner-or-admin)

**Evidence:**

1. Otto's own engines deliberately put these sessions' cwds in the data dir. The sandbox model names them "agent work areas" and re-opens them for the agent.
2. For example, a non-root Editor's own workflow run creates step sessions owned by that Editor (the run acts as its starter). The same is true of the Editor's own personal agent's chat, their goal loop's turns, and their scheduled task's runs.
3. When that user opens the session, the Side panel's Files tab calls `/fs/browse?path=<cwd>`, and `non_root_fs_denied` answers 403, because the path is inside the protected data dir. Opening any file in the editor also fails, because `/fs/read` gets the same deny and `/ws/lsp` returns 403 from `secret_root_denial`.
4. Root is unaffected. Iteration 3 fixed the same root cause only for one sub-tree, the run context dir, by adding run-scoped routes (3-04). It listed the persona dirs as "same root cause, not traced end to end". They are now traced, along with the four other work areas.
5. The deny protects nothing here. The user already owns the session and can make the agent print any file in its cwd. The real secrets under the data dir are `otto.db*`, `secrets.json`, `tls/`, `kube/` and other accounts' `provider-accounts/<id>`, and none of them is inside a work area.

**Scenario:**

1. Carol is a non-root Editor. She runs her workflow (worktree mode) and clicks **Open session** on a step.
2. The Files tab shows an error: "…/Library/Application Support/Otto/workflow-runs/<run>/<repo> holds credentials or Otto's own state; only the root user can open it".
3. The same happens on her personal agent's **Chat** session and her goal loop and scheduled task sessions.

**Fix (session-scoped, so no other user's work area opens up):**

1. Add `GET /sessions/{id}/files/browse?path=<rel>` and `GET /sessions/{id}/files/read?path=<rel>`, mirroring `routes/workflow_run_files.rs`:
   - gate them with `require_session_owner_or_admin` plus ws Viewer, and refuse share, MCP-only and agent-session tokens as `/fs/*` does
   - root them at the session's canonical `cwd`
   - join with `confine_join`, and refuse canonical escapes and symlinks that leave the cwd
   - keep `is_denied_file` (keys, `.env*`) and the data-dir deny-read list (`otto.db*`, `secrets.json`)
2. In the UI, when the Side panel is in session mode, pass this lister and reader to `FileTree` (it already takes `lister` and `reader` props since 3-04).
3. In `/ws/lsp`, accept a non-root root that equals or is under the canonical cwd of a session the caller may open (owner-or-admin) and that lies in an `AGENT_DATA_SUBDIRS` work area. Keep `secret_root_denial` for every other data-dir path.
4. Leave `/fs/*`'s global data-dir deny unchanged.
5. Tests:
   - A non-root owner browses and reads their workflow step session's worktree → 200.
   - Another non-admin member opening the same session's files → 403.
   - `path=../../otto.db` → 400.
   - The LSP opens on the owner's persona dir, and an LSP rooted at another user's persona dir → 403.

### access-control-4-04: the personal-agent page offers **Edit memory**, **Edit context** and autonomy editing to every Editor, but the server refuses non-owners (minor)

**Files:**

- `ui/src/modules/personal-agents/AgentPage.svelte:371-372` (`canEditDocuments = auth.can('scheduled_tasks','edit') && (isRoot || my_role ∈ {editor, admin})`, which ignores `agent.created_by`)
- `:492`, `:677`, `:686` (passed as `editable` to `AgentAutonomy`, `AgentMemoryInspector` and `AgentDocuments`)
- compared with `crates/otto-server/src/routes/personal_agents.rs:818` and `:849` (`PUT …/memory` and `PUT …/context`: owner, ws Admin or root, else 403)

**Evidence:**

1. `PersonalAgent.created_by` is in the UI type (`ui/src/lib/api/types/personal-agents.ts:36`), but no personal-agents component reads it.
2. A non-owner Editor sees the **Edit memory** and **Edit context** buttons (`AgentDocuments.svelte:112`), edits the text, and gets 403 "This agent runs as its owner; only the owner or a workspace admin can change its memory" on save.
3. Once 4-02 is fixed, the autonomy goals and rules and the memory inspector's edit and forget actions will behave the same way.
4. This is a checks-inconsistent-between-UI-and-server issue. The server is right, and the UI leads a legitimate user into a dead end.

**Fix:**

1. Add `canEditOwnerFields = auth.isRoot || myRole === 'admin' || agent.created_by === auth.user.id || !agent.created_by`. The ownerless preset is adopted on first write.
2. Use it for `AgentDocuments` (memory and context), the memory inspector's edit and forget, and the goals and rules sections of `AgentAutonomy`.
3. Keep `canEditDocuments` for the proactive toggle and budget, `primary`, and Run now.
4. Show a one-line note for non-owners: "Runs as <owner>; only the owner or a workspace admin can change this."

## Checked and not reported (false-positive guard)

- **Goal-loop answers and human verification by a non-owner Editor.** `goal-loops.md:51` documents "definition, mutation, verification and answers require Editor". The routes refuse agent and MCP credentials (`require_human_credential`) and record the verifier identity.
- **Room actions.** Every privileged action calls `self.host(actor)?` (`rooms/actions.rs`, `presentation.rs`). `invite` re-checks `room.owner` in the registry (`registry.rs:247`). Join tokens are single-use and expire after 10 minutes, and socket authentication uses a constant-time hash compare. `owner_auth` refusing impersonation and agent credentials matches `session-rooms.md`.
- **The swarm webhook can launch work as the swarm creator.** The key and integration are root-configured, `verify_cmd` is refused, and `repo_path` must be registered (`swarm_webhook.rs:120-200`). It is the documented design.
- **The workflow chat trigger starts any workflow by name in the receiving workspace.** The candidates are gated to that workspace, the bridge is root-configured, and the run acts as the latest changer (3-02).
- **`/canvas/scenes` returns the caller's own scenes across workspaces.** They are rows the caller created (`created_by = ?`). `/canvas/files/{sha}` checks Viewer on some workspace that holds the file, and otherwise returns 404.
- **The MCP session-attach toggle needs both ws Editor and `mcp:admin`.** Both are reasonable for a per-workspace switch that changes what every session there receives.
- **The proof `assemble` commands, review config and browser credential reveal routes require a person.** Each one guards a host-shell or secret sink (S11 and S8 series).
