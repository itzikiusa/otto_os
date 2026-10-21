# Quality iteration 1: fixes

This file records the outcome of each package in [PLAN.md](./PLAN.md): what was fixed, what was
rejected and why, and what was not done (each with a handoff). Ten fixers worked in parallel on disjoint
paths of the branch `fix/quality-loop-20261021`. A single verifier then reviewed the combined diff, ran
`scripts/check.sh --base main`, and fixed the integration problems listed at the end.

## P1: Git, cadence, launcher artifacts, workbench and logs accessibility

### Fixed

- **backend-correctness-1-01:** `cadence::served_occurrence` returns the latest occurrence after the
  cursor that is not later than now. A backlog of missed slots now becomes a single catch-up run, for
  interval, daily, weekly and cron schedules alike. Cron uses widening lookback windows, each capped at
  2,000 steps. Test: `a_multi_slot_backlog_collapses_into_one_catch_up_run`.
- **backend-correctness-1-02:** `validate()` rejects `every_min` above 525,600. The time arithmetic is
  now checked (`add_minutes`, `checked_mul`), so an old row with a huge value no longer panics. Test:
  `a_huge_every_min_is_rejected_and_never_panics`.
- **backend-correctness-1-03:** `merge_branch` rejects a branch merged into itself. A merge commit on an
  identical tip now reports up-to-date instead of creating a commit. Test:
  `merge_commit_of_an_identical_tip_reports_up_to_date_without_a_commit`.
- **backend-correctness-1-04:** when only `origin/<base>` exists, the integration bases resolve to it.
  Labels are de-duplicated after stripping the `origin/` prefix. Tests:
  `merged_into_reports_a_remote_only_default_branch` and `integration_labels_dedupe_after_stripping_origin`.
- **backend-performance-1-09:** the merged-set lookup reads the base oid from the ref listings it already
  has. One `git for-each-ref --merged` call now replaces two `git branch --merged` calls. Test:
  `merged_sets_split_local_and_remote_from_one_listing`.
- **integrations-1-01:** added `otto_core::paths::LAUNCHER_ARTIFACTS`, the list of launcher files
  (`.mcp.json`, `.agents/mcp_config.json` and `.grok/config.toml`). `commit_all_if_dirty` excludes every
  one of them.
- **integrations-1-02:** the proof gate ignores launcher and runtime files (`.env*`, `.DS_Store`) only
  while they are untracked, so a tracked edit to one of them still blocks the gate.
- **maintainability-1-12:** removed write-only state from `confluence.rs`. The output is unchanged.
- **ux-design-1-06:** the DiffViewer nav height follows the measured height of the scroll area instead
  of `100vh`.
- **error-handling-1-08:** ConflictResolverView shows a load error with a Retry button instead of
  "No conflicted files."
- **accessibility-responsive-1-01:** focus now returns to the right place when DockedDrawer, the
  Workbench side panels and the Logs detail close.
- **accessibility-responsive-1-05:** the Workbench issue line no longer has `role=alert`, and `.wb-save`
  is no longer a live region.
- **accessibility-responsive-1-06:** Quick Open is now a proper combobox and listbox, with no control
  nested inside an option.
- **ui-performance-1-06:** LogsView stops polling after it is destroyed.

### Not done

- The optional part of integrations-1-01: writing the launcher paths into `info/exclude` belongs in P2's
  `mcp.rs`. P2 can use `LAUNCHER_ARTIFACTS` for it.

## P2: Sessions, MCP config, channel bridge and MCP client

### Fixed

- **error-handling-1-07:** a launcher file that can't be read (permission denied, or not UTF-8) is no
  longer treated as empty and overwritten. Test: `unreadable_launcher_is_not_overwritten`.
- **error-handling-1-06:** if the user-server lookup fails, the managed user entries are now kept, not
  removed. The failure is recorded in `meta.mcp_servers_error`. Test:
  `unknown_user_set_preserves_managed_user_entries`.
- **integrations-1-03:** Codex sessions with the browser toggle on get the `otto-browser` MCP server.
- **integrations-1-04:** a Codex user server's environment no longer appears on argv. It is passed
  through a 0600 env file and a `/bin/sh` wrapper, and the file is removed when the session's
  credentials are revoked. Test: `codex_user_server_env_stays_off_argv`.
- **integrations-1-05:** `meta.mcp_servers` now lists, per provider, only the servers that were actually
  delivered.
- **concurrency-async-1-02:** a per-session submit lock keeps two concurrent paste-then-Enter sequences
  from interleaving. It covers the session manager and the Slack/Telegram bridge. Test:
  `concurrent_submits_do_not_interleave`.
- **maintainability-1-23:** server retirement now checks all three launcher files before writing to any
  of them.
- **test-quality-1-03:** added retirement coverage for the agy launcher file. Test:
  `retiring_refuses_untracked_agy_entry`.
- **test-quality-1-04:** added edge cases for the agent working-directory check. Test:
  `agent_cwd_confinement_edges`.
- **maintainability-1-11** (P6 handoff): deleted `with_mcp_tools_bin`.
- **integrations-1-06:** the MCP client negotiates protocol versions 2024-11-05, 2025-03-26 and
  2025-06-18, and sends the `MCP-Protocol-Version` header.
- **integrations-1-07:** stdio stderr goes into a 4 KiB ring buffer. A redacted tail of it is added to
  handshake and stdio errors.
- **error-handling-1-10:** a failed Control Plane rollback is now reported, with an "Open Control Plane"
  action.
- **architecture-1-05** (interim): a parity test now checks the input schema of each native and governed
  tool pair. It brought 24 of the 31 drifted pairs back in line.

### Not done

- **architecture-1-05, remainder:** the native catalog is not yet generated from the outward catalog.
  Seven pairs stay in a list that is only allowed to shrink (`list_connections` and six `api_*` tools).
- **Handoffs:**
  - `otto_core::hooks::McpServerProvider` has no implementor left. Delete it, or fold
    `McpServerSource` into it.
  - Other paste-then-Enter injectors could take `submit_lock`: `handover.rs`, assistant `tasks.rs`,
    otto-review `session.rs` and `claude_pty.rs`.

## P3: Server hot paths, cancel registry, AWS env builder and `expand_tilde`

### Fixed

- **architecture-1-03:** `CancelRegistryExt` and an RAII `CancelGuard` in otto-core replace five local
  copies, in skill_eval, skill_review, modules, product_host, otto-swarm and otto-product.
- **backend-performance-1-06:** `wait_session` wakes on session events instead of polling, with a
  safety re-check every 5 seconds.
- **backend-performance-1-07:** broadcast and relay load the user and role once and check every session
  in one batch (`InputAuthority`). They enforce the same rules as before, and `run_command` now
  authorizes the session before acting on it.
- **backend-performance-1-08:** the projects sessions page checks its rows in one batch.
- **backend-performance-1-05:** the workflow review await loop wakes on `ReviewChanged`, and reads only
  the review's status and agents (`ReviewsRepo::review_agents`).
- **backend-performance-1-10:** the human-approval gate wakes on the event for that gate, with a
  30-second safety read.
- **maintainability-1-01** (workflow_engine part): rewrote the stale node-cache comments.
- **maintainability-1-15:** removed the local copies of `expand_tilde`; they now use the one in
  `otto_core::paths`.
- **backend-correctness-1-05:** `expand_home` in `fs.rs` refuses `~user` paths.
- **maintainability-1-02:** added a shared `otto_core::aws_env` builder that otto-aws and otto-k8s now
  both use. The blank `AWS_PROFILE` is gone, and the account endpoint URL is now passed.
- **maintainability-1-05:** the EKS kubeconfig import inserts through `K8sClustersRepo::create`.

### Not done

- `WorkflowsRepo::get_cached_output` and `set_cached_output` have no callers and can be deleted from
  otto-state `workflows.rs`.
- The k8s spawns should call `env_remove` for each entry in `AWS_ENV_REMOVE`, in `cli.rs`, `logs.rs`,
  `monitor/*` and the k9s spec.
- otto-workflows no longer uses its `dirs` dependency, which can be removed.

## P4: Access control, the public-route pin, security-adjacent tests and the capabilities UX

### Fixed

- **access-control-1-01:** read-only sessions can now call three POST tools that don't change anything:
  the context packet, broker consume and Logs Insights. A test walks the full tool catalog to keep this
  allow-list complete.
- **access-control-1-02:** a share link can now be revoked by the person who minted it or by the session
  owner or a workspace admin. Revoking twice still returns 204, an unknown id returns 404, and anyone else
  gets 403. Tests cover the owner, the minter, an unrelated user and an unknown id.
- **access-control-1-03:** this change makes access less strict. Room and recap reads, and hosting a game
  room, need Agents:View (it was Edit). Session rooms, invites and recap writes still need Agents:Edit.
  The policy snapshot was regenerated, and exactly 13 lines changed from Edit to View.
- **maintainability-1-14:** replaced a stale TODO in `impersonate.rs`.
- **test-quality-1-01:** the `router_mount` test now pins every route that answers without credentials
  (`PUBLIC_ROUTES`, 30 entries). It compares in both directions, so it fails when a route is opened or
  closed.
- **test-quality-1-07:** `ui_control` polls the UI state instead of sleeping for a fixed time.
- **test-quality-1-09:** the support bundle redacts the recent audit rows. Test:
  `support_bundle_redacts_settings_and_audit`.
- **test-quality-1-08:** the report and plugin frame message handling moved into `reportFrame.ts` and
  `pluginFrame.ts`. These have behaviour tests in `frameMessages.test.ts`.
- **error-handling-1-05:** when the capabilities request fails, the app now says so, shows Retry, and
  retries with backoff and on reconnect. It no longer shows a silently empty shell.

### Rejected

- **The `assistant_recall` part of access-control-1-01.** No read-only session is an assistant thread,
  so allowing it would not make recall work. It would also let the other assistant tools past the guard.
  It is documented as `READ_ONLY_POST_EXEMPT`.

### Not done

- **maintainability-1-21:** the dead `/connections` branch in `policy.rs` has to be removed together
  with the three `rbac_matrix.rs` assertions that still use that branch.

## P5: Contracts, TS types, feature docs, the issue-tracker UI, swarm RunGraph removal and test hygiene

### Fixed

- **API contracts and types:**
  - api-contracts-1-02..13: removed `fingerprint` and `state` from the TS `ReviewFinding`.
  - Added `ReviewAgentCfg.skill`.
  - Documented `/state/restore`, `/settings/pr-review` and the scratch `workbench` family.
  - Added `on_restart` and a typed `patchWorkflow`.
  - Fixed the proof-pack cwd 403 docs.
  - Added a git "landed marks" section and the `cleanup-base` routes.
  - Replaced 23 type names in `api.md` that don't exist in the code.
  - Widened `ImprovementTrigger`.
  - The `ws.md` event count now points at `Event::type_name()`.
  - Added the `K8sCluster` fields.
- **Contracts for P4's changes:** documented the share revoke rules and the new room permission levels
  in the API contract and the feature docs.
- **ux-design-1-01 and 1-02:** an attached issue now stores its real provider, URL and status. The issue
  copy on the surfaces P5 owns no longer assumes Jira.
- **maintainability-1-03:** deleted the unused `RunGraph.svelte` and the store's graph pipeline. The
  `/graph` route is now documented as API-only.
- **maintainability-1-16:** fixed four doc paths that pointed to moved files.
- **test-quality-1-05** (usage half): the Report and Download HTML assertions always run, instead of
  being skipped when a button isn't visible.
- **test-quality-1-06:** two wall-clock waits now use mock timers.
- **accessibility-responsive-1-09:** added `workbench` to the e2e `PAGES` list. A unit guard
  (`pagesCoverage.test.ts`) now checks that every sidebar module is in that list.

### Not done

- **`getWorkflowVersion`:** delete it together with its two assertions in `workflowVersionPaging.test.ts`.
- **`desktop-session-header.spec.ts:183-189`:** still checks the details chip only if it happens to be
  visible.
- **`BrowserPanel.svelte` (~701):** still opens an empty URL for legacy attached issues.
- **`WorkflowsPage`:** should call the typed `patchWorkflow`.
- **RecoveryOwnership and the Jira copy:** the verifier fixed the `recoveryOwnership` failure and the
  SessionView and App Jira copy. See the end of this file.

## P6: Data integrity, Rust dead code and tag de-duplication

### Fixed

- **data-integrity-1-01:**
  - Deleting a swarm or project now halts its live runs and sessions first, under the operation guard,
    just as Abort does. The DELETE routes moved to `runtime/engine.rs`. The paths and the Editor role
    are unchanged.
  - Test: `deleting_a_swarm_halts_its_live_run_and_leaves_no_rows`.
- **data-integrity-1-02:** `delete_swarm` and `delete_project` each run in one transaction. They now
  also remove goals and channel triggers.
- **data-integrity-1-03:** improvement approve and rollback write a timestamped backup and then an atomic
  rename. If the target is a symlink, the link is kept.
- **data-integrity-1-04** (`mcp_control.rs`): each server, tool and policy PATCH is now one
  `UPDATE … COALESCE` statement.
- **data-integrity-1-05:** design project delete is one conditional `DELETE`. It returns Conflict if the
  project still has artifacts, or NotFound if it doesn't exist.
- **data-integrity-1-06:** the blob store fsyncs the file, renames it, then fsyncs the directory. A torn
  blob is repaired the next time it is written.
- **data-integrity-1-07:** notification de-duplication runs in a single `BEGIN IMMEDIATE` transaction.
- **backend-performance-1-02:** migration `0188` adds covering indexes for palette search. It only adds
  indexes.
- **maintainability-1-11:** deleted about 25 functions that had no callers, after searching the whole
  workspace for each one.
- **maintainability-1-22:** `mcp_audit` now uses the shared `sha256_hex`.
- **ui-correctness-1-03 and 1-08:** tags are de-duplicated in Golden Tasks and in vault frontmatter,
  with matching Rust-side de-duplication and tests.

### Not done

- The multi-field PATCHes in `users.rs`, `aws_accounts.rs`, `workspaces.rs` and `api_client.rs` (the
  rest of data-integrity-1-04).

## P7: Concurrency, sockets, plugins, the usage tailer and small Rust cleanups

### Fixed

- **concurrency-async-1-01:** reattaching to a mirror after a cancel installs a new tailer. The old
  tailer's teardown removes only its own entry.
- **concurrency-async-1-03:** the `/ws/events` and `/ws/term` sockets now check that the client is still
  alive, using Ping/Pong, and every send times out after 10 seconds.
- **concurrency-async-1-04:** a watcher task now owns each plugin sidecar process. A crashed sidecar is
  cleaned up and unpublished, and a generation check stops the watcher from removing a newer process.
- **concurrency-async-1-06:** the `mongosh --version` probe uses `kill_on_drop`.
- **error-handling-1-14:** an archive that fails to load no longer stops the recap store from opening.
- **backend-performance-1-01, 1-03 and 1-04:**
  - Usage tailer native state is now bounded by forgetting deleted Grok files and missing agy databases,
    with a cap of 100k entries.
  - The watcher now also watches the Grok and agy directories.
  - Antigravity databases are read incrementally, and the totals match a full read.
- **integrations-1-08:** the lightpanda log is appended to and rotated at 4 MiB. A respawned child that
  fails to become ready is killed and retried.
- **maintainability-1-17, 1-19 and 1-20:**
  - The date helpers now live only in `util.rs`.
  - Removed stale `allow(dead_code)` attributes, test shims, `brief_name`, and an unused
    `AUTHORIZATION` import.

### Not done

- **concurrency-async-1-05:** skipped, because the plugin protocol has no identity handshake. The
  remaining port race is documented on `free_port`.
- **concurrency-async-1-03:** `send_grid` in `otto-sessions/src/grid.rs` still sends without a timeout.

## P8: Database dashboards, multi-run, core UI stores, the session view and the composer

### Fixed

- **ui-correctness-1-01 and 1-02:** duplicate column names no longer crash MultiRun or the dashboard
  pickers.
- **ui-correctness-1-04 and 1-05:** dashboards and brokers are scoped to the workspace and use request
  tickets, so a late response from a previous workspace can't overwrite the current one. Tests:
  `databaseDashboardsScope.test.ts` and `brokersStoreScope.test.ts`.
- **ui-correctness-1-06:** if a widget save fails, its sheet stays open.
- **ui-correctness-1-07:** a split pane no longer handles the same key twice.
- **error-handling-1-02** (database, product and usage): these stores now use `toastError` and
  `loadErrorText` for their errors.
- **error-handling-1-11:** pulling collections from git now counts imported, skipped and failed separately,
  and postmanSync no longer counts failures as imported.
- **error-handling-1-09:** saving directories and restarting the session are now reported separately.
- **ui-performance-1-03:** the composer measures its height budget only on resize, not on every keystroke.
- **error-handling-1-15:** a failed repo lookup is no longer cached, and the cache is refreshed when the
  connection resyncs.
- **accessibility-responsive-1-03** (QueryEditor): the close glyph is shown at full contrast.
- **accessibility-responsive-1-07:** query tabs are tab buttons with a separate close button, and Delete
  closes the focused tab.

### Not done

- The `apiClient.svelte.ts` part of error-handling-1-02. `apiClientOwnership.test.ts` needs fixture
  entries first.
- The `GitTabs` and `BrokersPage` opacity cases of accessibility-responsive-1-03.

## P9: Rooms UI (other than games), toggle names, product pollers and glyph state

### Fixed

- **ui-performance-1-01 and 1-05:** RoomMedia no longer rebuilds the audio graph on every chat message.
  RoomAnnotations uses a single timer set for the next expiry, instead of a 500 ms interval.
- **ux-design-1-04, 1-05 and 1-07:**
  - The room shows separate first-connect and reconnecting states.
  - A failed speech capture can be retried.
  - The recaps page has breadcrumbs.
- **accessibility-responsive-1-02:** the host sees pending join and control requests, with a count badge
  and a polite live announcement.
- **error-handling-1-13:** when the host can't be reached, the join error now says so instead of
  "Load failed".
- **ux-design-1-03:** the accessible names of toggles are consistent at all 13 sites.
- **error-handling-1-01 and 1-03:** PlanTab and AnalysisTab now report poll failures with the cause and
  a way to retry, stop on a 403 or 404, and use a 30-minute limit. The timeout message now names the
  cause.
- **ui-performance-1-02 and 1-04:** WorkflowsPage's effect now reloads only when the workspace changes,
  and a destroyed Studio3D stops polling.
- **accessibility-responsive-1-04:** status glyphs (unread, connection state, dirty marks) now have
  text equivalents for screen readers.
- **api-contracts-1-01:** ProofPage no longer calls `GET /repos/{id}`, which doesn't exist. It now gets
  the repo path from the workspace repo list.

### Not done

- **RewriteTab and TestCasesTab:** these have the same swallowed-poll pattern as PlanTab.
- **`room-recap.spec.ts` heading:** the verifier fixed this. See the end of this file.

## P10: UI shared helpers and dead UI code

### Fixed

- **maintainability-1-04:** clipboard writes go through `lib/clipboard`. A new ratcheted ui-guards rule,
  `clipboard-write`, flags any new direct call.
- **error-handling-1-12:** "Copied" is shown only when the copy actually worked.
- **maintainability-1-06:** there is now one shared `formatBytes`, with an IEC option. Test:
  `metricFormatBytes.test.ts`.
- **maintainability-1-08, 1-09 and 1-10:**
  - Deleted the unused API wrappers, along with `workspaces.ts` and `modelCatalog.ts`.
  - Removed the canvas flow code and the `@xyflow/svelte` dependency.
  - Deleted about 25 unused helpers.
- **maintainability-1-13:** BarHost uses a plain dynamic `import()` and shows a load error if it fails.
- **maintainability-1-18:** the canvas pan and zoom code is shared (`PanZoom`). Test:
  `canvasPanZoom.test.ts`.
- **architecture-1-14 and error-handling-1-04:** `authedResponse` and `authedText` replace the
  hand-written fetches. `problemOf` now reads text error bodies too. Test: `apiProblemText.test.ts`.
- **test-quality-1-02:** the write-memory rules moved to `writeMemory.ts`. Test:
  `uiCommandsWriteMemory.test.ts`.
- **accessibility-responsive-1-08:** LinksPanel and OutputsPanel have correct listbox semantics and use
  a roving tabindex.

### Rejected

- **`altLoopbackState` and 11 helpers flagged as dead:** each one is used by a unit test, and several of
  those tests mirror server behaviour on purpose.

### Not done

- **Direct clipboard calls:** `Markdown.svelte`, `Studio3D.svelte` and `GameMatch.svelte` still call the
  clipboard directly. They are listed in the ratchet baseline.
- **Byte formatters:** `k8s-util` and `aws/util` need fixture entries in `infraViewers.test.ts` before they
  can use the shared `formatBytes`. `storageGauge` and `presets` need their pinned precision updated
  first.
- **e2e step:** the e2e step for test-quality-1-02 was not added.

## Verifier: integration fixes

These are the changes the verifier made after the fixers finished:

- **ui-guards regressions:**
  - The capabilities banner in `App.svelte` now uses `LoadState`.
  - The poll-failure buttons in PlanTab and AnalysisTab are labelled "Check again" (an immediate
    re-poll), the same as AnalysisTab's existing button.
  - Fixed a straight apostrophe in `auth.svelte.ts`.
  - Locked in the baseline improvements with `--update-baseline`.
- **`frameMessages.test.ts`:** fixed a type error. `assert.deepEqual(posts, [])` narrowed `posts` to
  `never[]`.
- **`recoveryOwnership.test.ts`:** the AnalysisTab fixture now provides the new poll state (`polling`,
  `pollFailures`, `lastPollError`, `pollStopped`, `loadErrorText`) and the 30-minute limit. The
  "after two minutes" test now really runs past two minutes.
- **`e2e/room-recap.spec.ts`:** the heading match now allows the breadcrumb.
- **Issue copy:** "Attach Jira issue…" is now "Attach issue…" in SessionView, in the focused-session
  command and in the help text.
- **`newSessionRecovery.test.ts`:** the fixture now defines `BROWSER_PROVIDERS`. P2 added this constant
  to NewSession.
- **`git_spawn_guard::no_bare_git_spawns`:** P2's new git fixture in
  `otto-sessions/src/manager/tests/mod.rs` now spawns git with `otto_git::hardened_std_command()`
  instead of a bare `Command::new("git")`.
- **Rust LOC ratchet** (`scripts/loc-ratchet.py`):
  - **Code moved out:** most of the growth was new tests, so those tests moved into their own files:
    - `ottod/src/mcp_tools.rs`: twin parity, the PR-number alias, the byte budget and narrow scoping
      moved to `mcp_tools/tests/catalog_tests.rs`. The file went from 7,582 lines to 7,394, against a
      limit of 7,431.
    - `otto-sessions/src/ws.rs`: the existing input-queue tests moved to `ws/input_queue_tests.rs`. The
      file went from 4,065 lines to 3,755, against a limit of 4,015.
    - `otto-sessions/src/manager/tests/mod.rs`: the agent-cwd tests moved to `manager/tests/agent_cwd.rs`.
      The file went from 4,162 lines to 3,978, under the 4,000-line cap for a new file.
  - **Raised:** the otto-server source total went from 130,939 to 131,578, an increase of 639 lines. It
    was raised with `--raise otto-server`, because almost all of the growth is regression tests for this
    iteration's fixes (share revoke, the read-only catalog walk, plugin reaping, project paging,
    ws liveness, support-bundle redaction and the proof gate). Those tests use private items, so they
    can't move to `tests/`.
  - **Lowered:** `--update-baseline` lowered every other entry to its current count.
