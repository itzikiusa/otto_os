# Quality iteration 5: fixes

This file records the outcome of each package in [PLAN.md](./PLAN.md): what was fixed, what was
rejected and why, and what was not done (each with a handoff). Ten fixers worked in parallel on
disjoint paths of the branch `fix/quality-iter5-20261022`. A single verifier then reviewed the
combined diff, ran `scripts/check.sh --base origin/main`, and fixed the integration problems listed at
the end.

Each fixer had a budget of three cargo commands and was not allowed to run `npm run check`, Playwright
or clippy on crates other packages were editing. Several fixers' last cargo run was blocked by another
package's in-progress compile error, so their tests first ran in the verifier's gate (see the end of
this file).

## P01: Workflow engine, expressions, automation and assistant engines, keyed locks

### Fixed

- **backend-correctness-5-03:** the `otto-core` expression parser has a depth counter (`MAX_DEPTH`
  64) over prefix `!`/`-`, `(`, call arguments and `[ ]` indexes, failing with "expression nests too
  deeply". Left-associative binary chains are capped at 1,024 operators ("expression is too long"),
  since a long chain overflows the stack the same way. Test `deep_nesting_is_refused_not_overflowed`.
- **data-integrity-5-03:** `WorkflowsRepo::delete` is one `BEGIN IMMEDIATE` transaction that also
  pauses the scheduled tasks running the workflow (`enabled = 0`, `last_status = 'error'`,
  `admission_generation` bumped) and records an error run reading "workflow deleted – task paused".
  A refused delete pauses nothing. The scheduled-task engine maps a NotFound workflow to the same
  pause. Tests `delete_pauses_the_scheduled_tasks_that_run_it` and
  `a_deleted_workflow_pauses_its_task_once`. No migration and no contract change.
- **error-handling-5-06:** in the scheduled-task and personal-agent engines, a report that can't be
  written records the run as `error` ("The report couldn’t be saved: …") and raises the failure
  notice. Tests `an_unwritable_report_is_recorded_as_the_runs_error` in both crates.
- **test-quality-5-03:** `SETTLE_HOLD` is 2 s under test; the retry loop is `settle_cursor_with_retry`,
  covered by two trigger-driven tests (a transient write failure recovers; a final attempt after the
  hold settles).
- **test-quality-5-08:** `channel_notify` gained pure `notify_targets` and `reply_target` with unit
  tests (workspace scoping, webhook and blank-channel filtering, Slack thread vs Telegram default chat).
- **test-quality-5-07:** `nodes/git_pr_tests.rs` runs `git_pr` through `execute_node` against a temp
  repo (own checkout with a dirty tree; a dirty `git worktree add` target). The R13 rule is the pure
  `git_pr::require_opened`, tested directly.
- **maintainability-5-02:** `api_secrets`, `eval_score`, `workflow_progress`, `backup_git` and
  `goal_loop` use `otto_core::keyed_lock::KeyedLocks`; `KEYED_LOCK_ALLOW` in `scripts/check.sh` is
  pruned to the 17 remaining paths.
- **maintainability-5-01 (check.sh part):** new UI ratchet "module → lib re-export shim" in
  `scripts/check.sh` (`UI_LIB_SHIM_MAX`).
- **maintainability-5-03 (budget part):** `nodes/budget_gate.rs` calls `crate::budget::check_budget`.
- **architecture-1-01r (partial):** `execute_node`'s per-kind match moved into
  `nodes::dispatch(NodeCall)`; `execute_node` is an 18-line shim.
- **architecture-4-04r (review_run part, partial):** `review_run::execute` is split into
  `resolve_targets` and `await_review` (872 → about 615 lines).

### Not done

- **architecture-1-01r (rest):** `RunState` and the split of `run_workflow_with_budget` (1,253 lines)
  into admission / schedule / driver. The per-node loop mutates eight locals on its continue/break
  paths and needs a loop-control enum first.
- **architecture-4-04r (rest):** `review_run::execute` is still about 615 lines. The per-target launch
  and the score/verdict phases need a per-target context struct.
- **test-quality-5-07 case 3** is tested on `git_pr::require_opened`, not the full node: reaching it
  through the node needs a successful draft agent turn, and the crate has no fake-agent seam.

## P02: Git, code review, Slack channels, inline-edit contract

### Fixed

- **data-integrity-5-01 (major):** new `otto-git/src/collections.rs`. Collection push resolves every
  `collections/<name>.json` through `LocalGit::confined_worktree_file`, refuses a symlinked or
  non-directory collections dir, validates every target before writing any, and writes through a
  sibling temp file plus rename. Pull skips symlinked dirs and entries. Three symlink tests.
- **data-integrity-5-05:** `GitStore::delete_repo` deletes the repo's `pr_reviews` and the repo row in
  one transaction (comments and findings cascade). Test
  `delete_repo_removes_its_pr_reviews_and_their_comments`.
- **ux-design-5-15:** GraphView no longer marks "Revert commit…" as danger; "Delete tag on origin"
  goes through `confirmOutward`.
- **accessibility-responsive-5-01 (git and mission-control part):** shared `ui/src/lib/inlineEdit.ts`
  (moved from product). PrDetail and WorkItemDetail focus the first field on edit and return focus to
  the Edit trigger after Cancel or Save.
- **accessibility-responsive-5-04 (graph-commit-diff part):** one always-mounted status announces
  "Loading file changes"; the per-file skeletons no longer announce.
- **accessibility-responsive-5-14:** DiffViewer's "expand line" refocuses the row through
  `diffRowFocus.ts`.
- **error-handling-5-03:** WipPanel renders the diff load error with `LoadState` and Retry.
- **error-handling-5-04 (FocusView part):** load errors use `providerErrorText`.
- **error-handling-5-05:** the summarizer's fallback cause rides in the existing note
  (`fallback_note()`); `reviewFallback.ts` parses it, and ReviewPanel, ReviewAgents and
  LocalReviewPanel name the real provider instead of a hard-coded "claude". Tests
  `fallback_note_carries_the_cause` and `ui/unit/gitReviewFallback.test.ts`.
- **error-handling-5-10:** `kept_comments` propagates a failed `get_review` read instead of
  `unwrap_or_default`.
- **concurrency-async-5-02:** a `ClaimGuard` restores a claimed retry row if the handler future is
  dropped before the spawn. Test `dropped_retry_claim_restores_the_settled_row`.
- **maintainability-5-03 (P02 part):** review routes call `crate::budget::check_budget`; the retry
  test calls `otto_sessions::submit::submit_prompt`.
- **backend-correctness-5-04:** Slack `drop_reason(event)` is the one predicate for non-user events,
  run before admission in the events-API branch. Test
  `drop_reason_skips_non_user_events_before_admission`.
- **integrations-5-04:** Slack file-upload bytes use a client with a 120 s timeout.
- **integrations-5-05:** `mirror.rs` `upload_or_note()` retries once on a short rate-limit hint and
  otherwise posts a thread note naming the file. Two stub-adapter tests.

### Not done

- **architecture-1-09** (split GraphView.svelte, 4,887 lines): needs `npm run check` to verify a
  behaviour-identical move of about 120 state-bound handlers. Open for a package that can type-check.
- **data-integrity-5-05, optional 0196 orphan cleanup:** left out on purpose; the transaction fixes
  new orphans and a blanket delete of existing ones was judged unnecessary risk.

## P03: Product module and Jira/Confluence

### Fixed

- **backend-correctness-5-02:** `storage_check::parse` refuses nesting deeper than 256 and a close
  tag with only `#root` left. Test `deep_nesting_is_refused_not_a_stack_overflow`.
- **integrations-5-01:** Jira `createmeta` and create go through `send_raw`/`t.fail`: 401 and 403 are
  Forbidden (403 is no longer "does not exist"), a short 429 is retried once. Loopback wire tests.
- **Confluence verify TLS:** `read_back` calls `ensure_tls()` before sending credentials; two tests in
  `tests/confluence_tls.rs`.
- **backend-performance-5-03:** the story watcher calls the new `refresh_source` with the story and
  account it already holds; no detail build or re-read per story.
- **maintainability-5-03 (P03 part):** the `md_storage` re-export from `confluence.rs` is gone; callers
  use `otto_issues::md_storage`.
- **ui-correctness-5-01:** `storyReset.svelte.ts` `onStoryChange` resets a tab only on a story switch
  (TestCasesTab, RewriteTab, PlanTab). Test `unit/productStoryReset.test.ts`.
- **ui-correctness-5-02 (call sites):** PublishDialog's and ImportDialog's mount loaders run untracked.
- **ui-correctness-5-03:** DesignArena's `loadAll` ignores stale responses (sequence token plus
  captured selection).
- **accessibility-responsive-5-02:** PlanTab has one always-mounted status.
- **accessibility-responsive-5-01 (product part):** product editors use `lib/inlineEdit`;
  `product/inlineEdit.ts` is deleted; NotesTab and LearningsView follow the contract and name their
  Edit buttons.
- **error-handling-5-04 (PublishDialog part):** `providerErrorText` for preview, projects and spaces.
- **ux-design-5-01 (product part):** ProductPage's Learnings, History and Inject tabs and
  ImportDialog load through `LazyMount`.
- **maintainability-5-01 (product part):** product importers use `lib/render/mermaid`.
- **accessibility-responsive-5-09:** ConfluencePagePicker moves focus after pick and Clear and keeps
  the active option in view.
- **ux-design-5-13:** Design Hall PublishModal shows a headline per failed action plus a detail line.
- **architecture-5-04 (partial):** `attachmentUrls.svelte.ts` is shared by OverviewTab and
  AttachmentsPanel (no blob cached after a story switch). Test `unit/productAttachmentUrls.test.ts`.

### Not done

- **architecture-5-04 (rest):** JiraIssueSection, DraftEditor and StoryTranscripts are not extracted
  (typed ~1,000-line moves that need `npm run check`).
- Handoffs taken by the verifier: the two unit harnesses that needed `providerErrorText`, and the
  `issueAccounts` store loader (see the verifier section).

## P04: Database Explorer

### Fixed

- **architecture-1-10 (partial):** the DB Assistant, saved queries and history moved out of the
  `DatabaseStore` façade into `lib/stores/database/{assist,savedQueries,history}.svelte.ts` behind an
  unchanged `database.*` API (3,661 → 3,312 lines). Test `unit/dbSubStores.test.ts`.
- **maintainability-5-01 (database part):** every importer of the six `modules/database` shims points
  at `lib/database/*`; the shims are deleted.
- **structuredClone of reactive state:** DatabaseChanges and EditFlow snapshot with `$state.snapshot`
  before cloning.
- **ux-design-5-03:** DiagramView renders a failed schema-graph load with `LoadState` and Retry.
- **ux-design-5-05:** "Send to database" joins an in-flight connections load and tells a failed load
  ("Couldn’t load your database connections", Try again) from an empty list.
- **ux-design-5-07:** WidgetCard uses `LoadState`.
- **ux-design-5-08:** the widget dialog names the connection it runs on, disables Add without one,
  and offers the widget's own last-run columns on Edit.
- **ux-design-5-12 (database sites):** clipboard-failure toasts say "Select the text and copy it
  manually."
- **accessibility-responsive-5-08 (QueryBuilder):** Remove buttons are named after their row and
  move focus to the next sensible control.
- **error-handling-5-04 (MultiRunDialog):** `loadErrorText`.
- **backend-correctness-5-01:** the Mongo shell parser caps nesting at 128. Test
  `deep_nesting_is_refused_not_a_stack_overflow`.
- **test-quality-5-02:** `db-drivers.yml` starts disposable MySQL 9.7 and Postgres 17 fixtures and
  runs the `resource_access` native cases; new ignored test
  `mysql_cancellation_mid_run_revokes_the_executing_claim`.
- **backend-performance-5-07:** the telemetry collector's and Lightpanda's logs are copy-truncated
  past a cap every 10 minutes; tests confirm the appender keeps writing after truncation.

### Not done

- **architecture-1-10 (rest):** the connections and schema-tree sub-stores are tightly coupled to
  `connEpoch`, the snapshots and the access scrub; not split.
- The new MySQL in-flight test and the enabled native cases need the live fixtures; the first CI
  `db-drivers` run is their verification.

## P05: App shell, terminal resize, workspace resync, agents UI

### Fixed

- **ux-design-5-01 (shell part, major):** every bare `{#await import()}` overlay in `App.svelte`,
  Navigator and TabBar is a module-level `lazyComponent` (`shell/overlays.svelte.ts`) rendered with
  `<LazyMount quiet onerror>`. LazyMount gained `onerror`, a "Try again" toast action and a retry on a
  fresh mount.
- **ui-correctness-5-04 (ToolStep):** the image list is keyed by index.
- **accessibility-responsive-5-03:** NewSession's per-provider steppers are moved out of the
  radiogroup in the accessibility tree (`aria-owns`).
- **accessibility-responsive-5-15 (picker):** JiraIssuePicker announces the result count.
- **accessibility-responsive-5-10 (embed):** embed titles are real links.
- **accessibility-responsive-5-11:** BrowserPanel returns focus after adding an annotation.
- **ux-design-5-04:** SessionsBox uses `LoadState` with Retry and shows "—" stats until loaded.
- **data-integrity-5-02:** `resyncAutomation(wsId)` reloads Scheduled Tasks and Run with Otto stores
  after an events-socket resync. Tests `unit/eventsAutomationResync.test.ts`, `eventsIdentity`.
- **maintainability-5-04:** `ws.injectInput` and the Terminal injection effect are deleted.
- **architecture-2-01r:** the Terminal resize decisions are pure functions in `termGridSync.ts`.
  Test `unit/terminalResize.test.ts`.
- **maintainability-5-07 (router):** the uncalled `router.lastRouteFor` is deleted.
- **architecture-1-13 (partial):** `uiCommands/k8s.ts` imports from `lib/k8s`; the canvas types moved
  to `lib/canvas/types.ts`.
- **maintainability-5-01 (canvas/vault part):** importers repointed to `lib/render/*`;
  `modules/vault/mdRender.ts` deleted.
- **ux-design-5-02 (ConversationView), error-handling-5-04 (NetworkProfiles, CanvasPanel,
  AccessGroups), ux-design-5-12 (ScheduledTasksPage).**
- **error-handling-5-05 (RunAgents):** uses P02's `reviewFallback.ts` to name the real summarizer and
  its cause.

### Rejected

- **accessibility-responsive-5-10 (tags):** already handled: vault tags get `role=button` and
  `tabindex=0` after sanitizing, and a real `<button>` would be stripped by the sanitizer.

### Not done

- **architecture-1-08r** (RunMonitor out of WorkflowsPage, 3,710 lines): state-bound; needs
  `npm run check` and the workflows e2e.
- **architecture-1-13 (rest):** the router's room-invite imports (window-memory Maps must move to
  `lib/rooms`), the canvas `scene.ts` move (two unit tests stub its path), and the remaining
  `uiCommands` imports of feature modules.

## P06: Contracts, swarm and assistant access control, budgets, broker replay

### Fixed

- **access-control-5-01:** migration `0195_swarm_prompt_editor.sql` adds nullable
  `swarm_tasks.prompt_updated_by` and `swarm_goals.updated_by`. Editing a task's title, description or
  labels, or adding/editing its goals, records the editor, and swarm turns run as the latest prompt
  editor (when that user still exists) instead of the task's creator. A person-added `source-run:`
  label is a 400. Tests in otto-state and `tests/swarm_identity.rs`.
- **access-control-5-02:** changing a swarm schedule's directive needs the creator, a ws Admin or
  root; cadence/enabled stay Editor-level. Channel triggers launch as the trigger's creator.
- **access-control-5-03:** `AgentOwnership` / `agent_ownership()` is the one predicate for directing a
  Personal Agent; assistant delegation (`resolve_agent`) now needs the owner, an ownerless agent, a ws
  Admin or root (403 otherwise, 404 when nothing matches). Unit tests plus
  `tests/personal_agents_owner.rs`.
- **test-quality-5-06 (partial):** `reusable_for` extracted from `find_agent_session` with a unit test.
- **error-handling-5-07:** `BudgetGate::read_config()` returns `Result<Option<_>>`; an unreadable
  config fails admission closed, `GET /usage/budgets` errors, and `spend_unavailable` marks a status
  the usage engine couldn't compute. UsagePage shows "Spend unavailable".
- **api-contracts-5-01 / 5-03 / 5-05:** `api.md` rows for the MCP server PATCH 409 cases, the
  required `account_id` on issue-tracker listings, and the personal-agent PATCH field names and 403.
- **api-contracts-5-02:** the broker reset dry-run `warning` is typed and rendered (`ResetWarning`),
  and Reset is disabled while it is set. Test `unit/brokersResetWarning.test.ts`.
- **api-contracts-5-04:** the unused `BrokerReplayStatus` TS types are removed.
- **concurrency-async-5-01 / data-integrity-5-04:** the broker replay runs in its own task, and a drop
  guard records acknowledged messages as `interrupted`. Test
  `a_dropped_replay_is_marked_interrupted_with_its_acknowledgements`.
- **maintainability-5-08:** `injection_for_home` naming and the stray "Design spec:" lines.
- **architecture-4-01r (partial):** `swarm_host.rs` no longer goes through the `review_session` alias.
- **Planner part items:** assistant `task_lock` on `KeyedLocks`; budget and submit callers repointed;
  AgentDocuments follows the inline-edit contract; RunInspector's clipboard toast.

### Not done

- **architecture-4-01r (rest):** the swarm's paste+Enter blocks are not converted to
  `otto_sessions::submit::submit_prompt`; the swarm needs a guard-returning run-active closure.
- **test-quality-5-06 (rest):** no manager-backed `find_agent_session` test.
- **architecture-1-13 (swarm part):** `lib/stores/swarm.svelte.ts` still imports `modules/swarm`.
- **access-control-5-01 residual:** swarm-wide and project-level standing-goal edits do not change
  the turn identity of every task; a follow-up should decide the rule.

## P07: API client, Kubernetes, AWS, dependency advisories

### Fixed

- **integrations-5-03:** gRPC TLS trusts the OS store and the bundled webpki roots (webpki only when
  the OS store is empty). Test `tls_config_trusts_the_os_store_and_the_bundled_roots`.
- **integrations-5-02:** the port-forward probe body is read capped (`read_body_capped`) and cut on a
  char boundary. Two tests.
- **Dependency advisories:** vite 8.3.2 → 8.3.3 (six Dependabot alerts), dompurify override 3.4.17
  (`npm audit`: 0), rustls-webpki 0.103.16, webpki-roots / webpki-root-certs 1.0.10. Unfixable or
  unmaintained-only advisories are documented in `docs/security/dependency-advisories.md`.
- **architecture-1-07r (net_guard part):** callers use `otto_netguard::` directly; the re-export is
  deleted.
- **test-quality-5-05:** `agent_run_step_refuses_a_script_retargeted_secret_and_a_person_run_sends_it`.
- **ux-design-5-02 / error-handling-5-02 / EH-5-04:** AWS and Kubernetes views render errors with
  `LoadState` and `awsLoadText` / `providerErrorText`.
- **ux-design-5-09:** MonitorSettings shows field errors inline with `aria-invalid`, focuses the
  first one and summarises the count.
- **ux-design-5-16:** ResourceTable's filtered-empty state offers "Clear filter".
- **maintainability-5-01 (k8s part):** three k8s shims deleted after repointing importers.
- **accessibility-responsive-5-08 (API client):** `rowFocus.ts`; Remove buttons named after their row.
- **accessibility-responsive-5-15 (CollectionsTree):** the status stays mounted.

### Not done

- **architecture-1-07r (engine move):** `execute` / `execute_service` touch about 15 otto-server
  internals; a faithful move into `otto-apiclient::rest` is its own pass.
- **integrations-5-03 (optional):** no `tls_verify=false` toggle for gRPC.
- `rustls-pemfile` (RUSTSEC-2025-0134) can be dropped from `ottod/src/main.rs`; not done.

## P08: Sessions, session files, transcripts, desktop supervisor

### Fixed

- **test-quality-5-04:** the `session_files` suite runs with the data dir protected and adds
  `a_home_cwd_session_cannot_reach_another_persona_folder`.
- **backend-performance-5-01:** `lsp_root_in_own_session` narrows candidates in SQL (unarchived; own
  or in a workspace the caller administers; cwd under the data dir) before any realpath.
- **backend-performance-5-05:** session-file browse and read go through `filesystem_work` (4 per
  route, 10 s deadline, cancel per entry). Test `a_canceled_browse_stops_before_listing`.
- **backend-performance-5-02:** `history_path_gate` tries one indexed query first and skips fresh
  missing marks in the fallback scan.
- **backend-performance-5-06:** the workspace transcript touch uses the live-session index.
- **architecture-3-03r:** `transcript/fold.rs` and `transcript/artifacts.rs` hold the fold options
  and work-artifact registration; callers repointed.
- **backend-performance-5-04 (projection):** `GET /admin/sessions` selects only the columns it
  returns.
- **concurrency-async-5-04:** `SessionManager::create` refuses with Conflict while the daemon shuts
  down, before and after the PTY spawn. Test `create_refuses_while_the_daemon_shuts_down`.
- **architecture-4-04r (serve_terminal):** a `TerminalConn` struct with one method per frame type;
  `serve_terminal` is under the 400-line cap.
- **test-quality-5-10:** new `it` suite `mcp_profiles_edges`.
- **error-handling-5-01:** the desktop supervisor publishes `otto://daemon-state`, single-flights
  ensure, keeps the last unhealthy detail, and the tray gains "Restart Daemon"; the offline screen
  shows the detail with Restart / Reinstall. Test `unit/desktopDaemonState.test.ts`.
- **maintainability-5-03 (P08 part):** the `sha256_hex` alias is gone; goal-loop and product-host
  budget callers use `crate::budget`.

### Not done

- `otto-review`'s `review_session` re-export block stays: about ten otto-server files still call
  `crate::review_session::*`.
- **backend-performance-5-04:** pagination and an `archived=0` default for `/admin/sessions` are a
  product decision (deferred).

## P09: MCP correctness, assist-engine interim steps, agy git exclude

### Fixed

- **backend-correctness-5-05:** `ottod mcp-server --profile` trims ids and fails closed on a bad id.
- **concurrency-async-5-03:** both writers of an MCP server's secret blob hold one per-server lock
  (`McpService::lock_secrets`). Test `secret_writers_share_one_lock_per_server`.
- **error-handling-5-08 (part):** MCP env keys are validated at save (400 naming the key) on all four
  routes and inline in ServerForm; Codex attaches a server whose name isn't a bare TOML key under a
  sanitized key.
- **agy git exclude:** a real-git test confirms `.agents/mcp_config.json` is excluded once in repos and
  linked worktrees.
- **test-quality-5-09 / 5-01:** MCP approval and audit tests cover another workspace's admin and
  server-bound approvals.
- **ux-design-5-06:** approvals show user names (`lib/userLabel.ts`). Test `unit/mcpUserLabel.test.ts`.
- **ux-design-5-14, ui-correctness-5-05, accessibility-responsive-5-08 (AllowlistsTab).**
- **architecture-4-02 (interim part):** `db_assist` uses `otto_agent_run::fence::extract_fenced`;
  `mockup_assist` uses the StatGate poll; the Excalidraw format moved to `otto_design::format`.
- **maintainability-5-02 (otto-design):** `commit_lock` uses `KeyedLocks`.

### Not done

- **architecture-5-02** (move `agent_session*` into `otto-agent-run::session_turn`, then drop the
  forwarders): a cross-crate move with 16 callers; deferred to the next iteration.
- **architecture-1-05** (route the ottod native MCP twins through `route_for`): the twins differ from
  the `route_for` paths in defaults and aliases; deferred.
- **architecture-4-02** (`design_scene3d` into otto-design): the code is in otto-product.
- **error-handling-5-08 (rest):** runtime `mcp_skipped` meta and the session MCP menu; the Settings
  `McpServers.svelte` inline env-key rule.

## P10: Rooms, games, Home classrooms, 3D stages

### Fixed

- **ui-performance-5-01 / 5-02 / 5-03 / 5-04:** the classroom screen poller keys on the open room id
  and a screens key, status nudges are throttled, loads go through `freshness`, the grid copy releases
  its WebGL school while zoomed, and relative times no longer need a ticking `now()`.
- **architecture-5-01:** `rooms/capability.rs` (`Capability::issue`, constant-time `Digest`,
  `TokenBucket`, `socket_token`) is shared by both relays. The stricter behaviour won where they had
  drifted: constant-time game credential and invite compares, a 43-character rule for room socket
  tokens and invites. UI: `lib/relaySocket.ts` under RoomClient and GameClient.
- **architecture-2-08:** one game catalog (`games/catalog.json`) read by Rust and the UI. Tests
  `catalog_lists_every_game_kind`, `unit/roomGamesCatalog.test.ts`.
- **error-handling-5-09:** a gave-up game connection shows "Couldn’t reconnect to the game room."
  with Reconnect.
- **maintainability-5-05 / 5-06:** snapshot-version constants; dead exports deleted.
- **accessibility-responsive-5-05 / 5-06 / 5-07 / 5-12 / 5-13:** game status announcements on edges
  only, Resume focus and Esc, scrollable compact results, RoomRecap's stable controls, KidCard focus
  return.
- **ux-design-5-10:** opening a kid in detention selects it instead of un-archiving; an explicit
  Release button.
- **ux-design-5-11:** game create/join errors read as sentences.
- **architecture-5-05 (partial):** `lib/three/stage.ts` (capped DPR, context-loss remount with a
  budget, renderer release) used by the games and the school.

### Rejected

- **structuredClone in KartGame/ShooterGame:** the cloned values are plain objects, not `$state`
  proxies; no change.

### Not done

- **architecture-5-05 (rest):** the studio embed has no context-loss remount (its caller owns the
  mount), and `watchVisibility` is not adopted.

## Verifier: integration fixes and checks

### Fixes made while integrating

- **Clippy:** two `needless_update` struct literals (the personal-agent and scheduled-task run
  finishers now name every field), `manual_is_multiple_of` in the broker replay checkpoint, and a
  `disallowed_methods` allow on the `git_pr_tests.rs` fixture helper.
- **Guards:** `git_pr_tests.rs` spawned a bare `git`; it now uses `otto_git::hardened_std_command`
  (`git_spawn_guard`). `lib/relaySocket.ts` took over the WebSocket heartbeat from `room-client.ts`,
  so the `raw-set-interval` allow entry moved with it. JiraIssuePicker's new result status uses
  `plural()`.
- **UI compile errors:** `AttachmentsPanel.svelte` had a stray `}` after `loadAttUrl` (P03's
  extraction), which made svelte-check lose every later declaration; `MonitorSettings.svelte`
  imported `plural` twice (P07); `unit/roomsRelaySocket.test.ts` typed its test context by hand
  instead of as `TestContext` (P10). `unit/orchestrationOwnership.test.ts` extracts WorkItemDetail's
  edit functions and now injects `refocusEditTrigger` and `EDIT_KEY` (P02's inline-edit change).
- **New test fixed:** `mcp_profiles_edges::the_profile_query_narrows_the_http_transport` named the
  tool `k8s_logs`; the control plane lists it as `otto.k8s_logs`. With the bare name the "no
  `k8s_*` tool under `?profile=database`" assertion could never fail.
- **Handoffs taken:**
  - `ui/unit/knowledgeRecovery.test.ts` and `publishDestinationOwnership.test.ts` inject
    `providerErrorText` (P03's PublishDialog change).
  - `ui/e2e/desktop-rpanel-tabs.spec.ts` finds the query-row button as `Remove parameter …` (P07).
  - `tests/unit/api_client.rs` imports `otto_netguard::check_url`; the test-only `net_guard` alias
    in `routes/api_client.rs` is gone (P07).
  - `modules/canvas/mermaid.ts` had no importer left after P03's repoint: deleted, and
    `UI_LIB_SHIM_MAX` lowered to 3.
  - `spend_unavailable?: boolean` is on the canonical `UsageBudgetStatus` in
    `lib/api/usage.svelte.ts`; the module-local `BudgetStatusWire` cast is gone (P06).
  - ui-correctness-5-02 root cause: `issueAccounts.load()` reads its cache gates untracked, and
    JiraIssuePicker's mount loader runs untracked, so a filled cache no longer re-runs a caller's
    effect and re-picks its account (P03). The identity harness stubs `svelte`.
  - ui-correctness-5-04 siblings: NeedsYouCard options and MemoryTab tags are keyed by index (P05).
  - Docs: `conversation-view.md` no longer says the UI pings the workspace touch every 60 s (P08);
    `conversation-view.md` and `vault.md` no longer name the deleted `mdRender` (P05); the stale
    `injectInput` comment in ResultsGrid is gone (P05).
- **LOC ratchet:** `otto-sessions/src/manager.rs` grew 28 lines with the shutdown refusal (P08). The
  refusal, the spawn bracket and `shutdown_all` moved to `manager/shutdown_guard.rs`, so the file is
  now 12 lines below its old cap.
- **Baselines:**
  - `scripts/loc-baseline.json` was lowered (`workflow_engine.rs`, `manager.rs`,
    `summarize_and_persist`, `review_run::execute` 872 → 615, `execute_node` 206 → 35;
    `serve_terminal` dropped under the 400-line cap). The otto-server total was raised on purpose by
    390 production lines (108,300 → 108,690). The growth is the new behaviour, not moved code:
    `rooms/capability.rs` (+129, the shared primitives; the two registries shrank by 61), the
    transcript path gate and artifact registration (+87 net), the budget fail-closed path (+46), the
    review `ClaimGuard` (+46), session-file admission and the LSP SQL narrowing (+43), the
    `/admin/sessions` projection (+29), `channel_notify` targeting (+25), the MCP env-key and secret
    lock (+21) and the game catalog loader (+21).
  - `ui/scripts/ui-guards-baseline.json` was rewritten. It lowers paid-down debt (for example
    `database.svelte.ts` 3,661 → 3,312, `OverviewTab` 2,997 → 2,971, `Terminal.svelte` 3,551 →
    3,532, and the raw-error entries of the AWS, Kubernetes, git and product files) and accepts
    file-loc growth in 14 files already over 800 lines (the largest: QueryBuilder +23, DesignArena
    +16, App.svelte +15, DiagramView +14). Two files crossed 800 lines and are now tracked:
    `home/boxes/ClassroomsBox.svelte` (796 → 832) and `rooms/games/shooter/ShooterGame.svelte`
    (788 → 832). Both need a split next; the games team is splitting the game modules on a parallel
    branch.

### Checks

- `python3 scripts/loc-ratchet.py`: ok (otto-server 108690/108690).
- `python3 -B -m unittest discover -s scripts -p 'test_*.py'`: 31 tests, OK.
- `scripts/check.sh --base origin/main`: exit 0. It selected the whole workspace because `Cargo.toml`
  and `Cargo.lock` changed. Steps: rustfmt; clippy `--workspace --all-targets -D warnings`; nextest
  over the workspace (6,412 tests run, 6,412 passed, 126 skipped) plus doc-tests; the LOC ratchet;
  the keyed-lock and prompt-submit guards; the new module → lib shim ratchet (3 of 3); and in `ui/`,
  `npm run check` (svelte-check: 0 errors, 0 warnings), `test:unit` (2,479 passed, 0 failed), `build`
  and the bundle budget (`page:rooms` lowered to its new size).
- The tests the fixers could only write ran here and passed, among them: the `it` suites
  `session_files`, `mcp_profiles_edges`, `swarm_identity`, `personal_agents_owner`,
  `mcp_auto_approve`, `review_agent_retry` and `rooms_api`; the otto-server lib tests for the
  `git_pr` node, `channel_notify`, the review `ClaimGuard` and `fallback_note`, the rooms capability
  primitives and the run-step binding test; otto-issues and otto-product (Jira createmeta/create wire
  tests, Confluence verify TLS, storage depth); otto-state with `migration_compat` over migration
  0195; and the ottod MCP profile parse. The otto-state proof timing budget that failed under load
  during the fixer runs passed.
- The Tauri desktop workspace (`apps/desktop/src-tauri`, P08's supervisor and tray edits): with the
  placeholder sidecar macOS CI stages, `cargo fmt --check`, `cargo clippy --all-targets -D warnings`
  and `cargo test` (55 passed) all pass.
- Not run: Playwright e2e. The MySQL/Postgres native cases in `db-drivers.yml` need the CI fixtures.
