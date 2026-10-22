# Quality iteration 4: fixes

This file records the outcome of each package in [PLAN.md](./PLAN.md): what was fixed, what was
rejected and why, and what was not done (each with a handoff). Ten fixers worked in parallel on
disjoint paths of the branch `fix/quality-loop-20261021`. A single verifier then reviewed the combined
diff, ran `scripts/check.sh --base origin/main`, and fixed the integration problems listed at the end.

Each fixer had a budget of three cargo commands and was not allowed to run `npm run check`, Playwright
or the otto-server `it` suite. Every test a fixer wrote but could not run then ran in the verifier's
gate (see the end of this file).

## P01: Contracts, personal-agent and session-file access control, product UI

### Fixed

- **access-control-4-03 (major, PC-1):** new `routes/session_files.rs` serves
  `GET /sessions/{id}/files/{browse,read}`. The gate is the session's owner, a ws Admin or root plus
  ws Viewer. Paths are relative, joined with `confine_join` and confined to the canonical cwd. A
  non-root caller is still refused key-like files, `otto.db*`, `secrets.json` and protected dirs
  outside the agent work areas. Policy maps the routes to Agents:View, and the feature guard's
  `is_host_files_route` covers them, so agent-session tokens get 403. `/ws/lsp` accepts a root inside
  a work area that is at or under the cwd of a session the caller may open, and the access check now
  runs before the language lookup. UI: `lib/api/fs.ts` (`sessionFs`) tries `/fs/*` first and retries
  the session route on a 403 for a path inside the cwd. Tests: unit tests in `session_files.rs` and
  the new `it` suite `session_files`.
- **access-control-4-02 (major, PC-3):** `save_autonomy` requires the agent owner only when the
  standing-goal texts or rule texts change (order and ids ignored). Toggles, proactive, budget and
  primary stay at Editor, and an ownerless preset is still adopted. `edit_memory` requires the owner.
  Tests: an in-module unit test and the new `it` suite `personal_agents_owner`.
- **access-control-4-04:** `AgentPage` gates the documents, the memory inspector and the goal and rule
  texts on `canEditOwnerFields` (root, ws admin, owner, or an ownerless agent). Non-owners see "Runs as
  its owner; only the owner or a workspace admin can change this."
- **api-contracts-4-01 (major):** the product store builds the analyze, rewrite, plan, test-case and
  inject-session paths from the story's own `workspace_id`. `AttachProductStory` lists only stories
  of the session's workspace. Documented in `api.md`, `contracts/product.md` and
  `features/product.md`. New test `unit/productStoryWorkspace.test.ts`.
- **api-contracts-4-02 … 4-06:** auth cells, request and response cells, the attach-product rows,
  the upload shape and the agy MCP config path now match the code.
- **PC-9 (prerequisite for ux-design-4-07):** new shared `ConfluencePagePicker.svelte` (a typeahead
  combobox over `searchConfluence` scoped to a space) and `ConfluenceSpaceSelect.svelte`.
  `PublishDialog` and `TestCasesTab` use them.
- **ui-correctness-4-01:** `RefineChat` and `DiscoveryChat` load their thread inside `untrack`.
- **ui-correctness-4-03:** the `issueAccounts` cache is keyed by `auth.me?.id`, so a stale load never
  fills the next identity's cache. New test `unit/issueAccountsIdentity.test.ts`.
- **error-handling-4-04:** a failed `ensureEditmeta` shows a compact LoadState with Retry instead of a
  toast.
- **accessibility-responsive-4-03:** the OverviewTab inline editors move focus into the field on open
  and back to the "Edit {field}" trigger on cancel or save (`modules/product/inlineEdit.ts`).
- **accessibility-responsive-4-08:** `FileTree` is a `role=tree` with `treeitem` rows, levels,
  positions, selection and a roving tabindex; Shift+F10 and the context-menu key open a row menu.
  New test `unit/fileTreeA11y.test.ts`.
- **accessibility-responsive-4-09:** always-mounted sr-only status regions in AnalysisTab, RewriteTab
  and TestCasesTab.
- **maintainability-4-09:** deleted the unused `__issuePathForTest`, `MockupFormat` and `GlobalRole`.
- **Pinned contract rows:** PC-2 (swarm agent-update owner fields), PC-4 (`respawn_failed`), PC-5
  (consumer-group reset 409 and replay status), PC-6 (CP4 503), PC-7 (convert-to-workflow) and PC-8
  (connection delete, mcp-query timeout).
- **Sweep hunks:** "Refresh" wording, "Couldn’t" rules in `content.md`, doc paths that now name
  `lib/render/markdown.ts`.

### Not done

- **accessibility-responsive-4-03** is done in OverviewTab only, as planned. PrDetail, WorkItemDetail,
  NotesTab, LearningsView and AgentDocuments still swap the Edit button without moving focus.
- **accessibility-responsive-4-09:** `PlanTab.svelte` has the same pattern and was not in the plan.
- **ui-correctness-4-03:** `resyncForIdentity` in `events.svelte.ts` does not call the new `reset()`;
  the identity-keyed store makes that call unnecessary.
- `lsp_root_in_own_session` scans every session row and canonicalizes each cwd. It only runs for a
  non-root LSP open of a data-dir work area. A SQL prefix filter is a cheap follow-up.
- The PC-7 TS type was left to P02; the verifier made `trigger_id` required (see below).

## P02: Workflow triggers, function ratchet, rooms, forge-post tests

### Fixed

- **test-quality-4-02 (major):** new `it` suite `review_forge_post` runs the real router against a
  wiremock GitLab: a 201 posts once and a second approve sends nothing; a 500 releases the claim and a
  later approve posts; a 400 on the positioned discussion falls back to one `/notes` post.
- **test-quality-4-07:** `workflow_run_files` seeds `id_rsa`, a 500 KiB log and a binary blob: a
  Viewer gets 403 on the key, the log is truncated within 400 KiB, the blob comes back empty and
  truncated.
- **test-quality-4-09:** the rooms media `sequence()` guard takes the per-source map, with a unit test
  for replayed, reordered and too-frequent chunks. `rooms_api` asserts 409 on a replayed sequence and
  403 on a wrong member generation or capture epoch.
- **error-handling-4-03 (PC-7):** convert-to-workflow returns the trigger-creation error, pauses the
  task only after the trigger exists, and deletes the new workflow on either failure. A success always
  carries `trigger_id`. The UI (`convertOutcome.ts`) treats a missing `trigger_id` as a failure. New
  test `unit/scheduledConvertOutcome.test.ts`.
- **concurrency-async-3-04:** skill-eval promote runs the backup and the atomic write in one
  `spawn_blocking`.
- **accessibility-responsive-4-05:** room host actions keep focus on the member's next control or the
  People heading; the non-host control button is always mounted. New test
  `unit/roomsHostFocus.test.ts`.
- **ux-design-4-10:** a pending guest's Cancel asks "Cancel your request to join?".
- **architecture-4-03:** trigger validation moved to `otto_workflows::triggers::validate_spec`, with
  `FIREABLE_EVENT_KINDS` and a test that it equals the UI's `EVENT_KINDS` (PC-11). The schedule tick
  moved behind `WorkflowCtx` into `otto-workflows/src/triggers/schedule.rs`.
- **architecture-4-04 (part 1):** `scripts/loc-ratchet.py` caps every production function in
  `crates/**/src` at 400 lines (`function_max`). Functions over the cap are listed by `path::name`
  and can only shrink.
- **Sweep hunks:** "Couldn’t" in the rooms clients; `summarize.rs` comments name
  `resolve_absent_scoped`.

### Not done

- **architecture-1-01r** (`run_workflow_with_budget` RunState split): not started; the 1,250-line
  ratchet entry still caps it.
- **architecture-4-04 parts 2 and 3:** `review_run::execute` (872 lines) and `serve_terminal` (584
  lines) were not split. Both are now capped by the function ratchet.

## P03: Workbench editor, workflows UI, API client runs

### Fixed

- **ui-performance-4-01 (major):** the Workbench editor path is stable per doc and the language is a
  live CodeMirror compartment, so typing keeps the caret and undo history. New e2e spec
  `workbench-typing.spec.ts` (not run in this loop).
- **ui-performance-4-02:** one language sniff over a bounded prefix (16 KB, 200 lines) serves both the
  language and the Auto label; PlaceholdersPanel scans a debounced copy.
- **ui-correctness-4-04:** the proof store tracks `wantedId`, so a late refresh can't show another
  pack. Regression test in `proofOwnership.test.ts`.
- **ui-correctness-4-05:** a failed run poll retries with backoff; after 5 failures the editor says
  "Lost track of this run". New test `unit/apiClientRunPoll.test.ts`.
- **ux-design-4-03:** automations, run history and cookies load through LoadState with Retry instead
  of toasts.
- **ux-design-4-08:** `IdPicker` shows a disabled "Loading {what}…" select, and free text only after an
  error; connection options name the engine and environment in words.
- **ux-design-4-09:** the browser marks button reads "Send N to {session}" and explains why it is
  disabled.
- **ux-design-4-07 (adoption):** ProductPublish uses the shared space select and page picker; the
  approval preview names the parent page by title.
- **error-handling-4-08:** bulk archive and kill name the failed sessions and state a shared cause
  once. New test `unit/workspaceBulkFailures.test.ts`.
- **maintainability-4-11**, **PC-11** comment, and sweep hunks (clipboard copy, "Couldn’t parse the
  table", `lib/render/*` imports).

### Not done

- **architecture-1-08r** (RunMonitor split) and **architecture-1-07r** (REST engine move into
  `otto-apiclient`): not done; both are large moves that need svelte-check or a trait extraction.
- `CanvasPanel.svelte`'s dynamic import still goes through the `canvas/mermaid` shim, because
  `unit/canvasLivePush.test.ts` pins that import.

## P04: Git UI, shell, agents UI and the live-region sweep

### Fixed

- **accessibility-responsive-4-01 (major):** the impersonation banner is a region, not an alert; an
  sr-only status speaks only at 5 minutes, 1 minute and expiry.
- **accessibility-responsive-4-02 (major):** ticking clocks are `aria-hidden` inside live regions in
  AgentDrivingBar, InsightsPage and Channels.
- **accessibility-responsive-4-06:** Skeletons carry their own label instead of a busy wrapper.
- **accessibility-responsive-4-09 (PrList):** one always-mounted status for the load-more error.
- **accessibility-responsive-4-04 (partial):** provider-card keys act on the focused card, and one
  sr-only status announces the session counts.
- **ui-correctness-4-02:** GraphView reloads when a new ref's tip is outside the spliced page. Two new
  tests in `gitGraphSplice.test.ts`.
- **ui-correctness-4-07:** PrDetail keys reviewers by index.
- **error-handling-4-01:** `loadErrorText` at the commit diff, stage fallback, notifications bell,
  ⌘K failure and PrList load-more.
- **ux-design-4-01:** the Database split pane has a loading skeleton and a LoadState with Retry.
- **ux-design-4-02**, **maintainability-4-08**, **maintainability-4-03** (sweep), and the "Refresh"
  and "Couldn’t" sweeps.

### Not done

- **architecture-1-09** (GraphView split) and **architecture-2-01r** (Terminal resize half): not
  started; both need svelte-check before and after a pure move of thousands of lines.
- **accessibility-responsive-4-04 (rest):** the ± count controls are still inside the radiogroup;
  moving them out needs a layout change.

## P05: DatabaseStore split, lib-to-modules imports, database, connections and AWS UI

### Fixed

- **architecture-1-10 (partial):** `database.svelte.ts` went from 4,423 to about 3,670 lines. It is a
  facade over `lib/stores/database/{model.ts,statement.ts,dashboards.svelte.ts}`; importers are
  unchanged.
- **architecture-1-13:** six database helpers moved to `lib/database/` and three k8s helpers to
  `lib/k8s/`, with re-export stubs at the old paths. `database.svelte.ts` now has no lib-to-modules
  import (was 8) and `k8s.svelte.ts` has 1 (was 5).
- **accessibility-responsive-4-02 (ResultsGrid, Athena):** the ticking "Running… Ns" text is no longer
  live; an sr-only status announces start and outcome once.
- **accessibility-responsive-4-07:** SchemaTree's tree keys ignore events from inputs and buttons.
- **ux-design-4-04:** the query builder's table palette shows a LoadState with Retry, and the import
  path names the real cause.
- **ux-design-4-06:** a bad DSN paste keeps the field open with an inline error.
- **ux-design-4-05:** AWS pages show a LoadState when the account list fails, not "Account not found".
- **error-handling-4-02:** new `awsLoadText` at 13 AWS load-error sites. New test
  `unit/awsLoadText.test.ts`.

### Not done

- **architecture-1-10 (rest):** tabs, schema cache, query runs and saved queries are still in the
  facade class.
- **architecture-1-13 (rest):** `k8s.svelte.ts` still imports `k8s-util` from modules, and the other
  lib-to-modules imports in the baseline are out of scope.
- No unit test for `listBuilderTables` throwing.

## P06: MCP native listings, MCP budgets, MCP tests, audit tab

### Fixed

- **architecture-1-05 (major):** ottod's native tool listing is generated from the control-plane
  catalog (`otto_mcp::outward::otto_tool_specs`) in the new `mcp_tools/listing.rs`.
  `DEFERRED_SCHEMA_DRIFT` is empty; a new test requires every catalog entry to be a native twin,
  alias, proxy or listed absence.
- **integrations-4-01 (major):** the gateway call uses `GATEWAY_CALL_TIMEOUT` (65 s); a test answers
  after 21 s.
- **integrations-4-03 (major):** every bridge budget lives in `otto_core::mcp_budgets`; Codex gets
  `tool_timeout_sec=960` for Otto's servers and 600 for user servers.
- **test-quality-4-01 (major, partial):** two decide-route refusal tests: a grant holder who is Admin
  only of another workspace, and an impersonated requester.
- **integrations-4-05 (PC-6):** a failed or unparsable keychain read is an error, not `{}`. Reconcile
  keeps existing entries and `PATCH /mcp-servers/{id}` answers 503 `keychain_unavailable` with
  nothing written.
- **architecture-4-05:** the prefix-based `is_read_only_sql` check is gone; `ensure_read_only` in the
  DB viewer is the only gate.
- **error-handling-4-07**, **ui-correctness-4-06** (AuditTab Load more after a filter change, new test
  `unit/mcpAuditPaging.test.ts`) and the "Couldn’t" sweep in OttoServerHome.

### Not done

- **test-quality-4-01 case (2):** the server-bound approval card refused under an enforced resource
  policy has no test yet.
- User MCP servers get the default 600 s tool timeout; `UserMcpServer` has no timeout field.

## P07: Sessions, unified prompt submit, automation, assistant, insights, channels

### Fixed

- **architecture-4-01 (major):** new `otto-sessions/src/submit.rs` owns the TUI wait, bracketed
  paste, echo probe and `SessionManager::submit_prompt`, which holds the per-session submit lock
  through paste, Enter and re-Enter. Scheduled tasks, goal loops, product, personal agents, insights
  and the channel bridge use it. `scripts/check.sh` gains a prompt-submit guard (swarm allowlisted
  until its conversion).
- **concurrency-async-4-01:** all five unlocked submitters go through the lock; a new test races two
  submitters.
- **error-handling-4-05 (PC-4):** a failed respawn stamps
  `meta.suspended = {reason: "respawn_failed", error, at}` and emits the status only after the write.
- **data-integrity-4-03:** removing a session deletes its `<data>/sessions/<id>` folder; a daily sweep
  removes orphans older than an hour.
- **backend-performance-4-03:** name allocation and broadcast read projections instead of every
  session row.
- **test-quality-4-06 (goal-loop half)** and **test-quality-4-08** (agy MCP delivery).
- **maintainability-4-01, 4-02, 4-04, 4-05:** dead functions removed, `KeyedLocks` reused in the
  bridge, stale comment paths fixed.

### Rejected

- **codeql-431** (`rust/command-line-injection`, `otto-automation/src/goal_loop_commands.rs:29`): by
  design, not an injection. The sink runs `sh -c <verify_cmd>` in the loop's own worktree. The command
  is the user-authored `verify_cmd` of an acceptance criterion, written only through the goal-loop
  routes, which require ws Editor, and config edits also require the owner or a ws Admin. The executor
  reads the loop's own row, resolved inside the caller's workspace, so no other workspace's loop and
  no unauthenticated input reaches it. It runs as the local user, in its own process group, under a
  timeout and cancellation. An allowlist would break legitimate loops.

### Not done

- **test-quality-4-06 (personal-agent half):** `advance_cursor` has no test yet.
- The swarm runtime still uses its own paste/Enter primitives (see P08).
- The channel bridge now uses the shared 90 s TUI wait instead of its own 20 s cap; a failed first
  paste now counts as not submitted.

## P08: Swarm turn identity, swarm runtime, DB Explorer backend

### Fixed

- **access-control-4-01 (major, PC-2, carries 3-03):** a swarm turn acts as the task's originating
  user (`acting_user`: the task creator, walking up subtasks and handoffs, then the project creator,
  then the swarm creator). Sessions are reused per agent and acting user. Changing an agent's prompt,
  soul, provider, model or cwd needs the swarm creator or a ws Admin. New `it` suite
  `swarm_identity`.
- **test-quality-4-03 (major) + backend-correctness-4-04:** verdict, turn-result and planner JSON is
  picked by shape, so a `{}` preamble or an earlier example can't be read as the answer.
- **data-integrity-4-06:** deleting a task or clearing a board removes its goals in the same
  transaction.
- **data-integrity-4-02 (PC-8):** deleting a connection removes its widgets and history and detaches
  its saved queries in one transaction.
- **integrations-4-02 (major, PC-8):** an MCP read-only query always carries a timeout of at most 15 s.
- **error-handling-4-06**, **backend-correctness-4-03**, **test-quality-4-04**,
  **test-quality-4-05 (partial)**, **maintainability-4-10** and the "Refresh" sweep in
  SwarmSettings.

### Not done

- **test-quality-4-05 (rest):** `.github/workflows/db-drivers.yml` still filters out the
  `resource_access` binary; the in-flight recheck test needs a live DB.
- **Swarm half of architecture-4-01:** converting `SwarmHost` to `submit_prompt` would drop
  `send_run_input`'s run-still-active check between paste and Enter; it needs a design call.
- **PC-2 scope:** a turn acts as the task's originator, not the user who pressed Run.

## P09: Usage tailer move, budget and transcript services, state cleanups, Atlassian transport

### Fixed

- **architecture-1-16:** the usage tailer driver moved from ottod into
  `otto-usage/src/tailer/driver*`; one Codex model reader.
- **architecture-3-03:** `otto-server/src/budget` (`BudgetGate`) and `otto-server/src/transcript`
  service modules; old route paths re-export them.
- **backend-performance-4-06:** migration 0191 adds `sessions.transcript_missing_at`, so History
  skips a root scan for a recently missing transcript.
- **backend-correctness-4-01:** migration 0192 adds `memories.prev_active`; undo restores it.
- **data-integrity-4-01:** undo-forget is one guarded `UPDATE … RETURNING`; a second undo fails.
- **backend-performance-4-01:** restore writes and fsyncs every file before the lock, then publishes
  with a no-replace rename.
- **backend-performance-4-04:** review heads without comments for the work-graph projector.
- **integrations-4-06:** the Atlassian transport classifies 429/503 with `Retry-After`, 401/403 and
  other errors, retries a GET or HEAD once and never re-sends a write.
- **architecture-3-02r:** every remaining Jira and Confluence call goes through the transport.
- **maintainability-4-02, 4-06, 4-07:** dead functions removed; `formatTokens` uses `formatCount`.

### Not done

- `rate_limit_wait` in `otto-issues/src/atlassian.rs` is a local copy of otto-git's; it should move
  into otto-core.
- `BrokerAuditRow` is still re-exported though nothing reads it now.

## P10: Design-assist phase 1, canvas and render shims, vault, brokers, SSH, CodeQL triage

### Fixed

- **backend-correctness-4-02:** SSH tunnels pass `ControlMaster=no` and `ControlPath=none`.
- **data-integrity-4-05 + integrations-4-04 (PC-5):** migration 0193 adds `broker_replays.status`. A
  replay records its intent before the first send, checkpoints evidence and ends `done` or `failed`;
  boot marks interrupted replays. A consumer-group reset is refused with 409 unless the group is Empty
  or Dead, and a partial reset names the partitions it missed.
- **data-integrity-4-04:** the skills manifest is written atomically, and a malformed manifest refuses
  instead of disowning skills.
- **backend-performance-4-02:** the vault scan publishes file changes in batches of 500.
- **concurrency-async-4-02:** a watcher started for a vault that was just unregistered is stopped.
- **backend-performance-4-05:** live-file polls read only when length or mtime change (`StatGate`).
- **architecture-4-02 (phase 1, partial):** one `extract_fenced`, the Excalidraw split/merge adapter
  in `otto-design-assist`, one `AgentTurn` type, and `DesignFormat` as a view over
  `otto_design::format`.
- **maintainability-4-03 (partial):** canvas, design-hall and vault import `lib/render/*` and
  `lib/excalidraw/*` directly; `canvas/renderQueue.ts` and `canvas/excalidraw-build.ts` are deleted.
- **maintainability-4-02, 4-06**, **ux-design-4-11** and **ux-design-4-12** (P10 sites).

### Rejected

- **codeql-457/458/459/460** (`rust/path-injection`, `otto-k8s/src/clusters.rs`): a false positive,
  with visible safety added. Only root authority reaches the typed path (add needs setup authority;
  update returns 403 to a non-root caller who changes it). The new `validated_kubeconfig_path`
  requires a regular file of at most 4 MiB that does not resolve into Otto's data dir, and the check
  no longer blocks on a FIFO.
- **codeql-461** (`rust/weak-sensitive-data-hashing`, `otto-state/src/workbench.rs`): a false
  positive. The helper is a content digest (blob keys, unchanged checks, de-duplication, audit
  correlation) and never hashes a secret. Renamed `content_digest_hex`.
- **codeql-404/397** (`rust/path-injection`, `otto-core/src/secret_paths.rs`): a false positive. Both
  sites are the deny-list guard itself, which only stats the candidate path. Both go through one
  named `guard_stat` helper.

### Not done

- **architecture-4-02 phase 1, scene3d move:** needs files P10 did not own
  (`otto-product/src/design_scene3d.rs`, `lib.rs`).
- **architecture-4-02 phase 2:** deferred (see SCORES.md).
- **maintainability-4-03 (rest):** `canvas/mermaid.ts` and `vault/mdRender.ts` shims stay while other
  modules import them.

## Verifier: integration fixes and checks

### Fixes made while integrating

- **Session files and LSP were too open (PC-1):** the work-area exception let ANY session cwd that
  contains the data dir reach every work area. A non-root user whose session cwd is `~` could list
  and read other users' workflow worktrees and persona folders through
  `/sessions/{id}/files/*`, and root a language server there through `/ws/lsp`. The exception now
  applies only when the session's canonical cwd is itself one work dir strictly inside an agent
  work area (`Restrict::is_session_work_dir`), for both routes. `api.md` and the module docs say
  so, and the unit test covers the area root, the data dir and an outside cwd.
- **Policy golden snapshot:** the new session-file routes are GET-only, so their other verbs now sit
  at the Agents:Edit tier like `/sessions/{id}/transcript`. `tests/snapshots/policy_decisions.txt`
  is regenerated with the 10 new lines.
- **Clippy** (`-D warnings`, never run on the whole workspace by a fixer): `manual_is_multiple_of` in
  the new Atlassian retry test, and `type_complexity` in `state_archive::publish_staged` (a
  `Published` alias).
- **macOS CI paths:** `otto-canvas` now depends on `otto-agent-run` and `otto-design-assist`, so it
  is a reverse dependent of macOS crates. `.github/workflows/macos.yml` lists `crates/otto-canvas/**`
  (`scripts/test_macos_ci_crates.py` failed without it).
- **UI guards:** a radius token in `ConfluencePagePicker`; an optional `ariaLabel` on
  `ConfluenceSpaceSelect` (callers name it with `<label for>`); the deliberate idle wait in
  `workbench-typing.spec.ts` carries `ui-guards: allow`.
- **Contracts:** `api.md` documents the 503 `keychain_unavailable` on the legacy
  `PATCH /mcp-servers/{id}` (the route P06 changed); `ws.md` gives the `respawn_failed` shape with
  `at` as an RFC 3339 string, as P07 implemented; `convertToWorkflow`'s TS type makes `trigger_id`
  required (PC-7); `features/canvas.md` names the moved renderer files.
- **Leftovers from P09:** ottod no longer declares the unused `notify` dependency, and the otto-git
  doc link to the deleted `pull_autostash` is reworded.
- **Baselines:**
  - `scripts/loc-baseline.json` was lowered (mongodb.rs, local.rs, manager.rs) and seeded with
    `function_max = 400` and the 23 functions over it.
  - Two entries were raised on purpose. The otto-server total grew by 524 production lines, mostly
    the new session-file routes (PC-1), the keychain handling in `mcp_servers.rs` (PC-6), the staged
    restore in `state_archive.rs` and the personal-agent owner checks (PC-3). `ottod/src/mcp_tools.rs`
    grew by 4 lines.
  - `ui/scripts/ui-guards-baseline.json` was rewritten. It lowers paid-down debt and accepts small
    file-loc growth in 27 files that were already over 800 lines. The growth is the new
    status/error/announce states listed above.

### Checks

- `python3 scripts/loc-ratchet.py`: ok (otto-server 108337/108337).
- `python3 -B -m unittest discover -s scripts -p 'test_*.py'`: 31 tests, OK.
- `scripts/check.sh --base origin/main`: exit 0. It selected the whole workspace because `Cargo.toml`
  and `Cargo.lock` changed. Steps: rustfmt; clippy `--workspace --all-targets -D warnings`; nextest
  over the workspace (6,271 tests run, 6,271 passed, 125 skipped) plus doc-tests; the LOC ratchet;
  the keyed-lock and prompt-submit guards; and in `ui/`, `npm run check`, `test:unit` (2,305 passed,
  0 failed), `build` and the bundle budget.
- The tests the fixers could only compile ran here and passed: the `it` suites `session_files`,
  `personal_agents_owner`, `swarm_identity`, `review_forge_post`, `workflow_run_files` and
  `rooms_api`, the otto-vault batch and watcher tests, and otto-state's `migration_compat` against
  migrations 0191 to 0193. The otto-state proof timing budget that failed under load during the fixer
  runs also passed.
- Not run: Playwright e2e, including the new `workbench-typing.spec.ts`.
