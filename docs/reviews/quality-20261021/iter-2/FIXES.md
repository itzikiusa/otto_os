# Quality iteration 2: fixes

This file records the outcome of each package in [PLAN.md](./PLAN.md): what was fixed, what was
rejected and why, and what was not done (each with a handoff). Ten fixers worked in parallel on
disjoint paths of the branch `fix/quality-loop-20261021`. This iteration did the large splits that
iteration 1 deferred. A single verifier then reviewed the combined diff, ran
`scripts/check.sh --base origin/main`, and fixed the integration problems listed at the end.

Most fixers could not compile otto-server while they worked: the otto-telemetry crate split (P05)
changed `context::scope` to take `Arc<dyn SpanSink>`, and `otto-server/src/telemetry.rs` (owned by no
package) still passed `Arc<TelemetryService>`. The verifier fixed that call first, and every otto-server
test that a fixer wrote but could not run then ran in the verifier's gate (see the end of this file).

## P01: Workflow engine split, swarm result-router suite, paste-submit locking

### Fixed

- **architecture-1-01 (partly):** the 17 large node kinds moved out of `execute_node` unchanged, one
  file each under `workflow_engine/nodes/` (`agent_prompt`, `prepare_context`, `http_request`,
  `game_engine`, `verifier`, `db_query`, `broker_peek`, `channel_notify`, `budget_gate`,
  `human_approval`, `swarm_task`, `loop_node`, `review_run`, `product`, `canvas`, `git_pr`,
  `self_improve`). `nodes/mod.rs` holds a `NodeCall` struct with the 11 arguments; `execute_node` keeps
  its signature and is now a dispatch match. The post-loop finalize phase of
  `run_workflow_with_budget` moved to `workflow_engine/run/finalize.rs`. `workflow_engine.rs` went from
  8,725 to 5,949 lines.
- **maintainability-2-05:** deleted the uncalled `get_cached_output` / `set_cached_output` from
  `otto-state/src/workflows.rs` (the table and migration stay).
- **maintainability-2-04 (hunk):** deleted the unused `ensure_cwd` wrapper in otto-swarm.
- **concurrency-async-2-02:** all six paste-then-Enter sites now hold the session's submit lock. Swarm
  uses a new `submit_run_prompt` (lock held across paste, wait, Enter and the dispatch-retry Enter; lock
  order is submit lock, then operation guard). The assistant delegation answer and the handover prompt
  now call `manager.submit_text`, which also adds the missing 200 ms paste-to-Enter wait. Regression
  tests in `tests/swarm_router.rs` race each path against a concurrent `submit_text` on a live PTY.
- **test-quality-2-01:** new `tests/swarm_router.rs` drives `route_result_inner` for six cases:
  cancelled mid-turn, paused vs plain stop, retry below and at the attempt ceiling, IC triage to the
  manager, the hop limit, and de-duplication of a second handoff.

### Not done

- **architecture-1-01 (rest):** the admission and schedule phases of `run_workflow_with_budget` are not
  split. They share about 15 mutable locals plus early returns and loop control, so they need a
  hand-designed state struct rather than a verbatim move.

## P02: `modules.rs` split, review lifecycle, transcript and work-graph performance

### Fixed

- **architecture-1-02:** `modules.rs` (7,703 lines) is split into `modules/ctx_impls.rs`,
  `session_input.rs`, `pr_draft.rs`, `skill_staging.rs`, `browser_proxy.rs` and `modules/review/`
  (`lifecycle`, `run`, `summarize`, `retry`, `routes`). `modules.rs` is now about 1,050 lines. The only
  visibility changes are private items widened to `pub(crate)`. The glob re-export of
  `otto_review::engine` is replaced by explicit imports.
- **architecture-1-15:** `review::lifecycle::transition()` (persist, log, broadcast) and `announce()`
  (broadcast only) replace the 16 hand-built review-level `ReviewChanged` events.
- **error-handling-2-06:** a failed diff write fails the review with "Couldn't write the diff for the
  reviewers: …" before any reviewer is spawned. A retry answers 409 when no stored diff exists and 500
  when the diff can't be read or written.
- **backend-performance-2-01 (partly):** `list_artifacts` registers work artifacts in one batch through
  the new `WorkGraphRepo::artifact_keys`. `register_work_artifact` uses `has_artifact`.
- **backend-performance-2-02:** a persisted transcript path whose file is gone returns
  `TranscriptMissing` without a root scan. The legacy history handler now uses the paged history, with
  `before` as an exclusive keyset cursor. A row with no persisted path still scans once, and the path it
  finds is now saved.
- **backend-correctness-2-03:** shared `otto_state::workgraph::like_escape` (backslash first), used by
  work-item title search, the vault search and the design search.
- **test-quality-2-02:** the state-archive test helper calls the production `build_snapshot_parts`. New
  test: secrets, hashes and tokens are dropped in both portable and non-portable exports.
- **test-quality-2-04 (partly):** `check_many_with_pool` and a unit test for revoked and disabled bound
  rows, in order.
- **maintainability-2-13:** removed the policy branch for a `/connections` route that does not exist
  (it now falls to `Deny`). `rbac_matrix.rs` mounts the real workspace connection routes.
- **maintainability-2-16:** `cancel.rs` and `cancel_signal.rs` document when to use which.
  `state.rs` names the registry type directly.
- **maintainability-2-04 (hunk):** deleted the unused `all_aliases` in otto-vault.

### Rejected

- **backend-performance-2-03:** the prescribed charge (4x source bytes plus 2 KiB per record) would stop
  every transcript over about 7 MB from being cached, so large transcripts would re-fold on every page.
  An accurate charge needs the fold to count retained bytes inside otto-transcript.

### Not done

- **backend-performance-2-01 (live part):** serving live sessions from the tail fold needs a
  `live_artifacts` accessor in `transcript_tail.rs`.
- **test-quality-2-04 (rest):** the two-namespace k8s case and the HTTP sessions case (needs the real
  sessions router).
- **architecture-1-02 (leftover):** `browser_proxy_ticket` stays in `modules.rs`, because
  `feature_guard.rs` maps `/browser/proxy` to that file. Repoint the guard entry, then move the handler.

## P03: API-client split, otto-state write integrity, channel Markdown

### Fixed

- **backend-correctness-2-01:** `prune_history` uses checked date math; `history_max_days` is clamped to
  36,500. Tests cover 99,999,999 and `i64::MAX`.
- **data-integrity-2-03:** every multi-statement PATCH is now one `UPDATE` (api_client collections,
  environments, automations; users; aws_accounts; workspaces; mcp_servers; broker_clusters). Tri-state
  fields use `CASE WHEN`.
- **data-integrity-2-04:** `database_changes::finish_in` runs inside the transaction for both `finish`
  and `reconcile`. Recovery re-classifies any `outcome_unknown` change that has no unknown or running
  attempt. The server retries recording an attempt's outcome instead of dropping the error.
- **data-integrity-2-05:** `parent_id` / `collection_id` must belong to the workspace (400), and a
  collection cannot become its own ancestor. Unchanged references are not re-checked, so existing rows
  can still be edited.
- **test-quality-2-03:** MCP control tests for a pinned risk label surviving rediscovery and for PATCH
  leaving omitted fields alone.
- **integrations-2-03:** new `otto-channels/src/gfm.rs` converts agent Markdown to Slack mrkdwn and
  Telegram HTML. Telegram falls back to plain text when the message is too long.
- **architecture-1-07 (partly):** `execute_service` and `code_search_service` replace fabricated
  extractors (new `AuthContext::automation`). The handlers are split into `routes/api_client/`
  (`collections`, `requests`, `environments`, `history`, `automations`); `api_client.rs` went from
  4,444 to about 3,270 lines.

### Not done

- **architecture-1-07 (rest):** `execute`, `run_step` and `resolve_environment` did not move into
  `otto-apiclient::rest`. The move drags about 2,000 lines of route-local helpers, which could not be
  verified in the time box. The `execute_service` seam is in place for a pure move.
- **backend-correctness-2-01 (otto-sessions half):** `auto_archive_stale` needs the same checked math;
  the file belonged to no package.

## P04: Automation ownership, shared cadence and delivery helpers

### Fixed

- **access-control-2-01:** personal agents. Instructions, provider, model, folder, browser access,
  delivery, a schedule's directive or permission, memory and context need the owner, a workspace Admin
  or root (403 otherwise). Name, avatar, enable and pause, cadence, run-now, cancel and rooms stay open
  to Editors, and a full-form save that resends unchanged values passes. Presets are seeded with no
  owner, and the first Editor who changes an owner-only field adopts the preset (a conditional `UPDATE`,
  so one of two racing Editors wins). `chat_session` reuses the pinned chat only when it is the
  caller's, and never takes over another member's pin.
- **access-control-2-03:** a goal-loop config change needs the owner, an Admin or root. Name, limits,
  an unchanged config and the lifecycle actions stay open.
- **error-handling-2-03:** scheduled-task settlement retries 5 times with backoff, then holds the claim
  for one scan and tries again. A failed manual-run status write is logged.
- **ux-design-2-01:** shared `savedDestWhere`, `destWho` and `confirmRunAndDeliver` in
  `lib/confirmOutward.ts`. Scheduled-task and personal-agent run-now buttons confirm where the report
  goes.
- **ux-design-2-04:** shared `lib/cadence.ts` ("Every day", "Mondays at 09:00 · UTC").
- **ux-design-2-09, ui-performance-2-02, maintainability-2-12:** trigger toast copy, a one-shot agent
  lookup on AgentPage, and the typed `patchWorkflow` in WorkflowsPage.
- **Hunks:** WorkflowsPage renders final output through `lib/md`; AgentPage skeletons get `role=status`;
  the dead `is_one_shot` is deleted.

### Not done

- **architecture-1-08:** WorkflowsPage (5,158 lines) is not split. The inspector forms depend on about
  30 page-local helpers and scoped CSS, and unit tests extract functions from the page by name.
- **error-handling-2-03 test:** a settle-failure test needs a fault-injection seam in otto-state.

## P05: Git UI, provider pagination, otto-git-cmd and the otto-telemetry split

### Fixed

- **architecture-1-06:** new leaf crate `otto-git-cmd` (hardened git command builders), so otto-sessions
  no longer depends on otto-git. `otto-telemetry` is now a leaf (span context plus the `SpanSink` trait).
  The collector, schema, analysis and their tests moved to the new `otto-telemetry-collector`, which
  otto-server and ottod link under the dependency name `otto-telemetry`.
- **integrations-2-02:** GitHub check-runs follow `Link rel=next` (20-page cap). Unread runs add a
  pending "N more checks (not read)" row, so CI is never reported green early. Statuses come from the
  combined status endpoint. Bitbucket CI and checks follow `next`.
- **integrations-2-04:** a `(completed, stale)` check run counts as failure.
- **test-quality-2-06:** the credential-cleanup tests seed the files and assert that a neighbouring
  session's file survives.
- **architecture-1-09 (partly):** GraphView's lane layout moved to `graph-lanes.ts` and its pure ref
  helpers to `ref-menus.ts`, each with unit tests (5,320 to 4,878 lines).
- **Accessibility and correctness:** GitTabs uses a real `role=tab` button plus a sibling close button
  (accessibility-responsive-2-07). WipPanel restores focus after stage, unstage and discard and
  announces the result (2-01). `aria-expanded` on file rows (2-08). Screen-reader text for the
  ahead/behind counts and the loaders (2-11). PrMergeModal checks are keyed by index (ui-correctness-2-03).
- **Hunks:** `rel()` in FocusView; `expand_home_strict` in otto-git's `clone_dir`.

### Not done

- **architecture-1-09 (rest):** RefSidebar is not extracted (about 390 lines of markup over about 40
  pieces of component state).

## P06: Database UI correctness and accessibility, SQL classification module

### Fixed

- **architecture-2-02:** new `otto-dbviewer/src/sql_class.rs`: one comment-aware first-keyword scanner,
  the per-engine keyword tables and `ReadClass::of(engine, sql)`. The four local copies are deleted. A
  pinning test keeps the old copies as oracles over a 43-statement corpus and finds no difference.
  `drivers/sql_common.rs` holds the shared float, pool-error and bulk-graph helpers.
- **ui-correctness-2-01, 2-05, 2-06, 2-07:** widget runs stamp freshness only on success; a failed Next
  page restores the offset; the dashboards box shows its load error with Retry; plan warnings are
  keyed by index.
- **ux-design-2-02 (partly):** the guarded-write confirm names the environment and reports a typed-name
  mismatch once.
- **error-handling-2-05 (mongosh half):** a failed mongosh probe shows its cause with Retry; the
  install hint appears only after a successful probe.
- **accessibility-responsive-2-02, 2-12:** MultiRunDialog moves focus to each new stage; a failed result
  tab reads "Result N, failed".
- **Hunks:** `expand_home_strict` for export and import paths; the dead `types::unimplemented` and
  `selectConnection` deleted; `rel()` in DatabasePage; row-count and connect-error copy; inline field
  errors in Import and Export; `promptText` validation in VerticalView; `aria-expanded` in
  StructureView; JsonTree moved to `lib/components` behind a thin wrapper.

### Not done

- **architecture-1-10:** DatabaseStore is not split. Four unit suites load `database.svelte.ts` as one
  source file with import mocks keyed by its exact paths; the harness must resolve relative imports
  first.
- **ux-design-2-02 (reuse half):** ImportDialog keeps its own gate until `dataToolsDialogs.test.ts` gets
  a `confirmGuardedWrite` fixture.
- **error-handling-2-05 (Blender half):** in DesignArena, which P06 did not own.

## P07: `types.ts` split, contract and feature docs, user list access

### Fixed

- **architecture-1-17:** the 12,188-line `types.ts` is split by script into 42 area files under
  `lib/api/types/`. `types.ts` is an `export *` barrel. A scoped type-check imported all 1,171 original
  names through it.
- **architecture-1-13, architecture-1-12 (cycle):** product and swarm types moved into
  `lib/api/types/`, so the two module type files only re-export and no longer import each other.
- **architecture-2-03 (partly):** `WorkspaceSettings` is typed and documented; the `as` casts are gone.
  Active workflow runs moved to a new `workflowRuns` store, which `ws` delegates to.
- **access-control-2-02:** `GET /users` is open to `Users:Admin` through the feature guard. Create,
  patch, delete and grants stay root-only. The Users page shows the root-only controls to root only.
- **api-contracts-2-01 to 2-13:** corrected roles, request and response types, removed or renamed rows,
  WebSocket liveness, missing fields, and the room-recap root rule.
- **maintainability-2-15:** doc citations corrected.
- **Hunks:** doc rows for P04 (owner-only agent and loop fields), P09 (plugin `running` and
  `last_error`); `loadErrorText` / `providerErrorText` in the API-client store; dead store members
  deleted.

### Not done

- **architecture-2-03 (rest):** the API-client settings accessors stay on `ws`; moving them would create
  a `ws` ↔ `apiClient` import cycle until `modules/api` calls `apiClient` directly.

## P08: Terminal protocol, event route table, shell and chat accessibility

### Fixed

- **architecture-2-01 (partly):** new `lib/components/termProtocol.ts` decodes terminal frames into a
  typed union, with one shared `applySnapshot`. The duplicated header map and untyped parse are gone.
- **architecture-2-04:** the 230-line event `if`/`else` chain is a typed route table split across
  `lib/events/routes/*.ts`. `eventRoutes.test.ts` fails on any event type that is neither routed nor
  listed as live-only.
- **ux-design-2-07:** the permissions-failure banner is an in-flow strip with Dismiss, not a toast-layer
  overlay.
- **Accessibility:** focus returns to the rail tab when the right panel closes (2-03); live regions for
  notes failures, terminal find and chat working state (2-04, 2-05, 2-06); `aria-expanded` and
  `aria-current` in the shell (2-08, 2-09); `role=status` on 10 skeletons (2-11).
- **ui-performance-2-05, test-quality-2-07, maintainability-2-07 (hunk):** one handover-source fetch per
  id; the session-header E2E assertions run unconditionally; NotificationBell uses `rel()`.

### Rejected

- **accessibility-responsive-2-08 (BottomNav) and 2-11 (shell placeholder):** both already had the
  attributes. The findings cite them as the correct pattern.

### Not done

- **architecture-2-01 (rest):** the resize and claim negotiation (about 450 lines) is not moved to
  `termGridSync.ts`.
- **architecture-2-05:** the palette sources stay in `shell/App.svelte`.

## P09: Backend hardening

### Fixed

- **integrations-2-01:** MCP launchers no longer carry a server's secret env inline. The entry runs
  through a 0600 env file under the temp dir, and the launcher files are added to the repo's
  `info/exclude`.
- **data-integrity-2-01, 2-02, 2-06:** the skill cache re-stats on every read and drops stale entries;
  library writes are atomic (temp, fsync, rename, directory fsync); a promoted skill backs up the old
  `SKILL.md`; the insights config is written atomically.
- **concurrency-async-2-01, 2-03, 2-04:** grid sends time out after 10 s; improvement rollback claims
  the edit by compare-and-swap first; the version probes kill their child when dropped.
- **error-handling-2-04, test-quality-2-05:** plugins report `running` and `last_error`; the settings
  page and plugin frame show "not running" with Start again. Only the owning generation unpublishes a
  port.
- **backend-correctness-2-02:** k8s time windows split on a character boundary and use checked math.
- **maintainability-2-01 (functional part):** the brokers ops repo is wired in the daemon and in tests,
  so lag alerts and replay evidence work.
- **maintainability-2-02, 2-03:** shared `expand_home_strict` and `apply_aws_env` replace local copies.
- **maintainability-2-04, 2-14:** dead code deleted; test banners moved.
- **ui-correctness-2-02 (scanner part):** connection-import warnings are de-duplicated.

### Not done

- **maintainability-2-01 (structural part):** the ops repo is not a required constructor argument yet
  (an unowned test calls the old arity).
- **maintainability-2-08:** the connection pin and section reorder routes stay; a policy snapshot, a
  policy special case and a grants test use them. Their contract rows were kept (see the verifier
  section).
- **concurrency-async-1-05:** closing the plugin port race changes the plugin protocol and every
  example plugin.
- **architecture-1-16:** the usage ingestion driver is not moved into otto-usage.

## P10: Shared UI helpers and the size and layering guards

### Fixed

- **Shared helpers:** `lib/html.ts` `escapeHtml`, `rel(ts, {compact})`, `promptText({validate})` with
  inline errors.
- **maintainability-2-11:** `escapeHtml` replaces 14 hand-written escapers.
- **error-handling-2-01, 2-02, 2-07:** RewriteTab and TestCasesTab pollers back off and show "Couldn't
  check progress" with Check again; stores use `loadErrorText` (k8s and aws keep the provider's 403 text
  through `providerErrorText`); the design library stops memoising transient errors.
- **ui-correctness-2-02, 2-04, 2-08, 2-09:** keyed warnings, stale-workspace guards in Mission Control,
  aborted K8s reads not marked fresh, canvas session refresh guarded.
- **ui-performance-2-01, 2-03, 2-04:** room media rebuilds the audio graph only when its signature
  changes; BrowserSettings and DesignArena stop re-fetching.
- **ux-design-2-03, 2-05, 2-06, 2-08:** inline field errors on five forms; TopicDetail keeps the
  selection after a consume; shared `rel()` on six surfaces; copy fixes.
- **accessibility-responsive-2-10, 1-03:** vault tree multi-select semantics; the Brokers tab close button
  no longer fades.
- **maintainability-2-06, 2-10:** byte formatters delegate to `lib/metric-format`; dead store members
  deleted.
- **architecture-1-11:** markdown, mermaid and the render queue moved to `lib/render/`; three more
  renderers go through the sanitising `lib/md` (the FileTree preview was unsanitised).
- **architecture-1-12 (partly):** scene3d and DeviceFrame moved to `lib/design/`; one token-name
  grammar.
- **architecture-2-07:** the Rust LOC ratchet counts production lines only and caps three god functions.
  `ui-guards` adds `file-loc`, `lib-imports-modules`, `raw-error` and `html-escape`.

### Not done

- **architecture-1-12 (rest):** DesignBoard stays in modules until `excalidraw-build` moves to `lib/`.

## Verifier: integration fixes

- **otto-server compile error:** `telemetry.rs` coerced the service to `Arc<dyn SpanSink>` before
  `context::scope`. This was the single error that blocked otto-server for every fixer.
- **Contract docs:** P07 had removed the `PATCH /connections/{id}/pin` and
  `POST /workspaces/{id}/connection-sections/reorder` rows, but P09 kept the routes. The rows are back in
  `api.md` and `connections-ssh-sftp.md`, with the reorder body corrected to `{ids:[…]}`. Added the 400
  rules for foreign or cyclic api-client collection references (P03) and `running` on `GET /plugins`
  rows (P09).
- **CI:** the ignored collector tests now run as `-p otto-telemetry-collector`. `macos.yml` was
  regenerated against `scripts/macos-ci-crates.py` (otto-telemetry-collector in, otto-git, otto-vault
  and otto-telemetry out, because otto-sessions no longer depends on otto-git).
  `docs/testing/telemetry.md` and the AGENTS.md crate map name the new crates.
- **concurrency-async-2-04 (third probe):** `routes/capabilities.rs` `which` gets `kill_on_drop(true)`.
- **LOC ratchet:** `run_review_core` was 5 lines over its cap after the diff-write fix, so
  `materialize_review_diff` was extracted. `--update-baseline` lowered every entry and dropped the
  files that fell under the cap (`modules.rs`, `routes/api_client.rs`, `otto-dbviewer/service.rs`).
  The otto-server production total was raised with `--raise otto-server` (107,576 to 108,401): the
  growth is module headers from the splits, the new owner guards, and the shared helpers.
- **UI guards:** the two new sr-only "Loading …" spans in DiffViewer and RepoView now use the shared
  `Skeleton` with a label. QueryEditor's two mongosh Retry buttons are one button. The baseline was
  lowered with `--update-baseline`.
- **Clippy:** elided the needless lifetime on `otto_core::aws_env::apply_aws_env`; moved
  `pr_draft.rs`'s test module below the last item; restored the grok test's doc comment, which the new
  `all_files` helper had split off, and allowed `std::fs::read_dir` in that test-only helper.
- **svelte-check:** RunInspector's absolute-time helper was renamed to `fmtWhen`, but four call sites
  still called `rel`. They now call `fmtWhen`.
- **Bundle budget:** `page:personal-agents` (+0.8 KB gzip, the shared delivery confirm and cadence
  helpers) and `page:plugin` (+0.3 KB, the "not running" state) grew on purpose and were recorded with
  `--update`.

### Gate result

`scripts/check.sh --base origin/main` (whole workspace, because `Cargo.toml` changed): rustfmt,
clippy `-D warnings`, nextest (6,132 tests passed, 125 skipped), doc-tests, the LOC ratchet,
`npm run check`, `npm run test:unit` (2,237 passed), the build and the bundle budget. Every test a fixer
wrote but could not run passed here, including `swarm_router`, `personal_agent_policy`, the goal-loop
owner test, the `grants_api` user-list tests, `rbac_matrix`, and the relaxed otto-improve
`concurrent_rollbacks_have_one_winner_and_no_conflict`. Playwright E2E was not run.
