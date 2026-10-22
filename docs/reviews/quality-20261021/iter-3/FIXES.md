# Quality iteration 3: fixes

This file records the outcome of each package in [PLAN.md](./PLAN.md): what was fixed, what was
rejected and why, and what was not done (each with a handoff). Ten fixers worked in parallel on
disjoint paths of the branch `fix/quality-loop-20261021`. A single verifier then reviewed the combined
diff, ran `scripts/check.sh --base origin/main`, and fixed the integration problems listed at the end.

Each fixer had a budget of three cargo commands and was not allowed to run `npm run check`. Most
otto-server lib tests could not compile while the fixers worked, because P10's
`modules/pr_draft.rs` test module was missing an import (`parse_pr_draft`). The verifier fixed that
first. Every test a fixer wrote but could not run then ran in the verifier's gate (see the end of this
file).

## P01: Access-control blockers, run-context routes and the API contract

### Fixed

- **access-control-3-01 (blocker):** `POST /mcp/tokens/{id}/rotate` loads the token's owner through
  the new `AuthRepo::mcp_token_owner`. A caller who is neither the owner nor root gets 403 "only root
  may rotate an MCP token owned by another user". Revoke stays open to `mcp:admin`. Tests in
  `auth_security::mcp_rotate` run on the real router: a non-root `mcp:admin` rotating root's token
  gets 403 and root's token still works; rotating your own token returns 200 and the old token stops
  working; root rotating another user's token keeps the owner; an unknown id returns 404.
- **data-integrity-3-05:** `AuthRepo::rotate_mcp_token` is one transaction. `DELETE … RETURNING`
  is the guard and runs first, the new token is inserted on the same connection through the shared
  `insert_mcp_token`, then both commit. The old hash is evicted and revocation is signalled. Tests: a
  forced insert failure leaves the old token valid with no orphan, and a second rotate of the same id
  mints nothing.
- **access-control-3-04:** new `routes/workflow_run_files.rs` serves
  `GET /workflow-runs/{id}/context/browse` and `/context/read` (PC-1). The run is looked up first
  (404), then ws Viewer on its workspace is required. Paths are relative; absolute paths, `..` and
  symlinks that escape the context dir get 400. Reads keep the `/fs/read` size cap and binary
  handling, open with `O_NOFOLLOW|O_NONBLOCK`, and refuse key-like file names to non-root callers.
  `/fs/*` is unchanged. New `it` suite `workflow_run_files`.
- **access-control-3-05:** the LSP root check also accepts roots under a repo registered in one of
  the caller's workspaces, and applies the `/fs/*` non-root secret deny (`~/.ssh`, Otto's data dir,
  key-like names) to every non-root LSP root (403).
- **integrations-3-02:** a Bitbucket merge that answers 202 is polled through the task-status
  `Location` (same origin only) every 2 s for up to 60 s. SUCCESS returns 204, FAILED returns 409 with
  Bitbucket's reason, and a merge still pending returns 409 "Bitbucket accepted the merge but hasn't
  finished it yet". Five wiremock tests.
- **api-contracts-3-01 … 3-11:** `api.md` and the feature guides now match the routes: the product
  test-case and PR-commit types, the personal-agent memory and context PUT routes, every review-retry
  and stop outcome, the bounded `/history` scan, no `?token=` fallback anywhere, the missing request
  and response cells, memory merge and split responses, root-only usage attribution and forecast,
  `/ws/lsp`, the insights, workflows and skills-evaluator troubleshooting text, and the provider-skills
  and mission-view access rules.
- **PC-1 … PC-6 documented** in `api.md`, `workflows.md`, `usage-and-cost.md`, `git.md` and the TS
  doc comments.
- **maintainability-3-06:** stale `modules.rs` references point at the real files.
- **maintainability-3-09:** AGENTS.md lists otto-aws, otto-k8s and otto-transcript, and the UI module
  list adds assistant, aws, desktop, kubernetes, personal-agents, rooms and workbench.

### Not done

- `docs/features/mcp-control-plane.md` still described rotate as `mcp:admin` only. The verifier
  updated it.
- `UsageStatus.tailer` (PC-2) and the swarm types are documented in `api.md` only; their TS mirrors
  belong to P08 and P09. `WorkflowTrigger.updated_by` is not in TS because the Rust type does not
  serialize it.

## P02: Workflow-engine run split, workflow identity, data integrity and dbviewer backend

### Fixed

- **access-control-3-02 (PC-3):** migration 0189 adds nullable `workflow_triggers.updated_by` and
  `updated_by_at`. Creating a trigger or changing its spec records the caller; pausing or enabling
  does not. `WorkflowsRepo::run_acting_user` returns the later of the run's pinned graph version
  author and any trigger's spec editor, and the engine's `run_identity` uses it for runs with no
  starter, falling back to the creator. Test `unattended_run_acts_as_the_latest_changer`.
- **data-integrity-3-01:** migration 0190 adds `memories.prev_state`. Forget stores the state once,
  and Undo restores it (a `suggested` memory stays suggested, a `contradicted` one stays inactive).
- **data-integrity-3-04:** the MCP-tool risk override and the review-finding regression guards are
  in the SQL `UPDATE` (`CASE WHEN …`), not a separate read.
- **data-integrity-3-07:** `SettingsRepo::put_many` writes every key of `PUT /settings` in one
  transaction, and the reloads run after the commit.
- **data-integrity-3-02:** deleting a dashboard deletes its widgets in the same transaction, and a
  widget can't point at a dashboard that doesn't exist or belongs to another workspace.
- **data-integrity-3-08:** dashboard, widget and k8s-cluster updates are single `UPDATE`s
  (`COALESCE` / `CASE`, `json_patch` for k8s params).
- **backend-correctness-3-02:** data-modifying CTEs (`WITH … UPDATE/DELETE/INSERT/MERGE`) are never
  auto-limited.
- **backend-correctness-3-04:** a Postgres placeholder mode: `#` operators, nested comments,
  `E''` strings and `$tag$` bodies are handled.
- **test-quality-3-02 (partly):** a non-ignored test covers all six executor-gate refusals.
- **maintainability-3-12 (hunk):** deleted the otto-dbviewer `DbResult` alias.

### Not done

- **architecture-1-01:** moving `run_workflow_with_budget` (about 1,250 lines) into a `RunState`
  with `run/{admit,schedule}.rs` did not fit the time box.
- **test-quality-3-02 (rest):** the native resource-access cases still need a disposable fixture
  database with the `otto_reader`, `shop` and `hidden` objects before CI can drop
  `-E 'not binary(resource_access)'`.
- **test-quality-3-04:** the restricted-reader tests for `schema_root`, `search_objects` and
  `schema_graph` are not written.
- **access-control-3-02 (rest):** no end-to-end test through the `db_query` node's connection check.
  The firing trigger is not stored on the run, so every trigger of the workflow counts; that can only
  select a more recent changer. The verifier documented this in `api.md` and `workflows.md`.

## P03: WorkflowsPage split and its findings, plus Rooms, Home and assistant fixes

### Fixed

- **architecture-1-08:** `WorkflowsPage.svelte` went from 5,156 to 3,703 lines. The per-kind
  inspector forms, the shared pickers, the `NodeParams` dispatcher and the pure helpers moved to
  `workflows/inspector/`; the canvas geometry moved to `canvasGeometry.ts`. Every new file is under
  400 lines.
- **error-handling-3-04 and ui-correctness-3-05:** the final output loads through `readRunContext`
  (PC-1), hides on 404, shows an inline error with Retry otherwise, and renders with line breaks. The
  Context panel lists and reads through the run-scoped routes.
- **ux-design-3-02:** the DB connection, broker cluster and topic, product story and Jira account
  fields are pickers that show names. An unknown saved id stays selectable, and the field falls back
  to free text with Retry while the list is loading or has failed.
- **error-handling-3-08:** RunAgents forgets a session only on a 404.
- **accessibility-responsive-3-07:** after a keyboard delete on the canvas, focus moves to a
  neighbour and the deletion is announced.
- **accessibility-responsive-3-01, 3-03, 3-11, 3-12:** Home radios get arrow keys; the room chat keeps
  one submit button and returns focus after sending; admit and decline move focus to the right place;
  LiveStatus loses its nested `role=status`; the zoomed dashboard grid fits narrow screens.
- **ux-design-3-07:** "Could not" became "Couldn’t" across rooms and login, and the typed-confirm
  mismatch wording is shared (`toastTypedMismatch`).
- **data-integrity-3-03:** deleting an assistant thread returns its attachment paths from the same
  transaction, and the files are removed after the commit (only inside the owner's inbox). An hourly
  sweep removes unreferenced inbox files older than 24 hours.

### Not done

- `rooms/RecapImage.svelte` and `RecapSettingsModal.svelte` still said "Could not". The verifier
  changed them.
- `database.svelte.ts`, `ImportDialog.svelte` and `kubernetes/actions.ts` could call
  `toastTypedMismatch()`; kubernetes already uses the shared wording.

## P04: GraphView split, git UI, review backend and brokers

### Fixed

- **maintainability-3-01:** removed the dead PR list state from the git store.
- **error-handling-3-09:** the Pull button says plain "Pull" until the pull mode is known.
- **accessibility-responsive-1-03:** the auto-fetch control is no longer dimmed.
- **backend-correctness-3-01:** a retry whose diff can't be re-materialised puts the settled row
  back. The boot sweep settles orphaned agent rows of settled reviews ("Interrupted by a daemon
  restart"). Stop settles an orphaned `pending` row.
- **backend-performance-3-01:** cancellation checks read only the review status, the summarizer
  polls the in-memory flag and re-reads the database on events or every 5 s, and the draft loop checks
  the database once per 25 comments.
- **concurrency-async-3-01:** the review prompt submit holds the session's submit lock through paste,
  echo wait and Enter. Regression test `submit_prompt_and_submit_text_do_not_interleave`.
- **data-integrity-3-06:** the `.git/info/exclude` edit keeps the user's bytes and only appends the
  missing lines.
- **concurrency-async-3-05:** broker consume `max_wait_ms` is clamped to 30 s.
- **test-quality-3-03:** broker guard tests for prod and read-only clusters, dev clusters, dry-run
  resets and global-cluster grants.
- **Cross hunks:** GraphView dates use `rel()`; "Couldn’t copy"; `loadErrorText` in the git diff
  views and the pull and push toasts; `FINDINGS_POLL` is private; FocusView and FindingActions use the
  new issue client.

### Not done

- **architecture-1-09 (GraphView split):** the refs sidebar uses about 150 GraphView identifiers and
  could not be moved safely without `svelte-check`. Left for a round that can run it.
- **concurrency-async-3-01 (optional part):** a bounded wait for the human composer lives in
  otto-sessions, outside P04.

## P05: DatabaseStore split, the JsonTree move, and database, AWS and Kubernetes UI fixes

### Fixed

- **architecture-1-13 and maintainability-3-08 (JsonTree half):** `bson.ts`, `expansion-plan.ts` and
  `json-find.ts` moved to `lib/jsonTree/`; every viewer uses `lib/components/JsonTree.svelte`, and the
  module copy is deleted.
- **ui-correctness-3-01:** the query builder de-duplicates GROUP BY suggestions and keys chips by
  index.
- **ui-correctness-3-04:** applying a review clears exactly the pending drafts it was built from.
- **ux-design-3-01:** a canceled managed write says why and keeps the modal open.
- **accessibility-responsive-3-02:** the schema tree has levels, positions and set sizes, and the
  search and counts sit outside the tree.
- **ui-performance-3-02:** SQS attribute fetches commit in batches, and auto-refresh waits for them.
- **ui-correctness-3-03:** k8s image lists de-duplicate non-adjacent repeats.
- **ui-performance-3-01:** the k8s monitor's listener effect no longer re-runs on every scroll.
- **accessibility-responsive-3-10:** the connection test result is one always-mounted status chip.
- **Cross hunks:** radio arrow keys in four more groups; dead store fields deleted; the shared
  typed-confirm wording in kubernetes.
- **architecture-1-10 (groundwork):** `loadSource(…, {relative: true})` in the unit harness resolves
  relative imports, so the store tests can follow a split.

### Not done

- **architecture-1-10 (DatabaseStore split):** about 3,700 lines of methods share `this` state, and
  seven store tests read the file as one source. The split and the test port should be one package.

## P06: Product and design-hall decoupling, the issue client, the Atlassian transport, and vault

### Fixed

- **architecture-3-05 (PC-7):** `lib/api/issues.ts` encodes every path segment and query value, and
  `lib/stores/issueAccounts.svelte.ts` caches the accounts. Product, the Jira picker and settings use
  them. The Jira field editor moved to `product/jira/`. OverviewTab went from 3,485 to 2,982 lines.
- **architecture-1-12:** DesignBoard, the templates and `format.ts` moved to `lib/design/`, and the
  Excalidraw builder to `lib/excalidraw/`. design-hall no longer imports from product.
- **maintainability-3-08 (scene3d half):** the 11 scene3d shim files are deleted.
- **maintainability-3-14:** a single `unescapeHtml` in `lib/html.ts`; Excalidraw labels no longer show
  `&quot;`.
- **ux-design-3-06:** relative dates use `rel()` with an absolute tooltip.
- **error-handling-3-06:** the Blender probe error shows inline with Retry.
- **ui-correctness-3-02 and 3-07:** story keys are de-duplicated and chips keyed by index;
  `story_ids_for` returns distinct ids; resolved stories merge into the live map.
- **architecture-3-02:** `otto-issues/src/atlassian.rs` holds one transport for Jira and Confluence:
  a bounded client cache, the TLS check, and status mapping. Linear reuses the client cache.
- **backend-correctness-3-05:** a vault link rewrite copies the frontmatter block byte for byte (CRLF
  and `...` fences survive).
- **Cross hunks:** radio arrow keys in design-hall and scene3d; dead store fields deleted; the
  Skeleton label on OverviewTab.

### Rejected

- **ui-correctness-3-02 (server half):** artifact rows and search hits already de-duplicate
  `story_ids`, so `group_concat(DISTINCT)` would change nothing. Limiting `story_ids` to
  `rel='implements'` would break the story filter, which matches any relation.

### Not done

- SessionView should pass `rel` to `suspendedNote`; `unit/status.test.ts` pins "just now".
- `agents/conversation/codeBlocks.ts` still has its own `unesc`.
- **architecture-3-02 (rest):** most status checks still build their own messages; the TLS rule and
  client cache are shared.
- The stale `product/design` paths in `product.md` and `product-design-arena.md` were updated by the
  verifier.

## P07: Terminal and App shell splits, the workspace store, and the palette and a11y sweep

### Fixed

- **architecture-2-01 (grid sync):** the grid feed and claim negotiation moved to
  `lib/components/termGridSync.ts` with no behaviour change.
- **architecture-2-05:** palette sources and the dock badge moved to `shell/paletteSources.svelte.ts`
  and `shell/installShell.svelte.ts`. App.svelte went from 1,720 to 1,486 lines.
- **architecture-2-03:** the API-client workspace settings moved from the ws store to the apiClient
  store.
- **ui-correctness-3-06:** a workspace switch clears the workflow runs, so a late refresh is dropped.
- **error-handling-3-01 (store part) and 3-03:** session load errors use `loadErrorText`; ids in a
  failed reconcile chunk are never pruned.
- **ux-design-3-03:** one close/archive confirm builder for the palette and the navigator; bulk
  archive confirms when a session is mid-turn.
- **accessibility-responsive-3-04 … 3-09:** the palette status line sits outside the listbox; the
  planner moves focus to the plan heading; Skeleton is named and not busy-nested; roving tabindex and
  arrow keys on the remaining radio groups (plus a ratcheted `radio-key` guard); focus moves sensibly
  after removing a goal, rule, id or share link.
- **maintainability-3-02 and 3-13:** dead fields and the unused `lib/fuzzy.ts`, `lib/cssColor.ts`
  and `parseMermaid` are deleted.

### Not done

- **architecture-2-01 (rest):** the resize half of the negotiation is interleaved with the
  resize-compact queue and stays in Terminal.svelte.
- **architecture-2-05 (rest):** the boot effect, key map and nav prefetch close over App-local state;
  the focused-session palette source stays because `unit/archiveGuard.test.ts` pins it.
- `history.svelte.ts` error text and the stale `commandSearch.ts` comment were fixed by the verifier.

## P08: Usage tailer move, the budget and transcript service layer, sessions and automation

### Fixed

- **backend-correctness-2-01:** the auto-archive cutoff is computed with checked arithmetic; a
  huge day count turns the sweep off instead of panicking.
- **integrations-3-03:** launcher env files are deleted when their server leaves the launcher, and
  deleting, disabling or renaming a server sweeps its files in every cwd.
- **test-quality-3-06:** two real-clock tests for the usage settle retry.
- **test-quality-3-05 (partly):** a k8s namespace memo test and an admin-sessions test that a ws Admin
  doesn't see a managed bound row.
- **backend-performance-3-02:** one query finds the live browser sessions' cwds, and the launcher
  reconciles run on one blocking thread.
- **backend-performance-3-05:** session artifacts come from the live transcript tail when there is
  one.
- **error-handling-3-02:** goal-loop finalize and block retry the status write and report a failure;
  per-iteration writes log their errors.
- **error-handling-3-05 (PC-2):** the usage tailer retries an unavailable journal with backoff, and
  `UsageStatus.tailer` reports `running`, `retrying` or `stopped` with the cause. The Usage page shows a
  warning when usage isn't being recorded.
- **maintainability-3-07:** one `formatUsd` for every cost, and token counts use `formatCount`.
- **ux-design-3-05 (Usage half):** the Otto usage tab's Retry re-imports the module.
- **maintainability-3-10:** the connections guide's pinning section matches the UI.

### Not done

- **architecture-1-16:** the tailer move into otto-usage (about 2,400 lines) did not fit.
- **architecture-3-03:** the `BudgetGate` and transcript resolve layer are not started.
- **backend-performance-3-04:** migration 0191 (`transcript_missing_at`) is not written.
- **ux-design-3-05 (split view half):** `agents/SplitNode.svelte` needs the same retry fix.
- `lib/sessionUsage.ts` `formatTokens` stays because `unit/sessionUsage.test.ts` pins its output.

## P09: Swarm activation lifecycle and identity, `KeyedLocks`, and git and run-service backend

### Fixed

- **architecture-3-01:** `otto-swarm/src/runtime/lifecycle.rs` owns activation (operation guard,
  gate, status write, coordinator start, event) for user start, resume and channel start; pause and
  abort sit beside it. Product uses one `activate_swarm` host method.
- **error-handling-3-07:** a failed roster read fails the tick instead of reading as empty.
- **test-quality-3-01:** the utilization rebalance is three pure helpers with unit tests.
- **architecture-3-04:** `otto_core::keyed_lock::KeyedLocks` replaces the git, swarm and product
  lock registries (pruned, poison-tolerant). `check.sh` guards against new hand-rolled registries.
- **maintainability-3-11:** broken intra-doc links fixed, and `check.sh --all` runs rustdoc with
  `-D rustdoc::broken_intra_doc_links`.
- **concurrency-async-3-02:** cancelling a run waits for in-flight worktree provisioning, and
  `worktree remove` uses `--force --force`.
- **maintainability-3-05 and 3-15:** `~user` paths no longer match; test banners sit above
  `#[cfg(test)]`.
- **concurrency-async-3-03:** the SSH tunnel is ready only after ssh reports this port's forward.
- **concurrency-async-3-04 (otto-context part):** library writes run on the blocking pool.

### Not done

- **access-control-3-03 (PC-4):** not started. Swarm sessions are still created as the swarm's
  creator; originator propagation, `find_agent_session` per acting user and the role-definition owner
  guard remain.
- **concurrency-async-3-04 (rest):** `skill_eval.rs` promote still writes on a Tokio worker.
- **architecture-3-04 (rest):** the remaining registries are on the `check.sh` allowlist.

## P10: MCP catalog generation, the api-client REST engine, and the otto-core API split

### Fixed

- **architecture-3-06:** `otto-core/src/api.rs` (3,986 lines) is split into 18 area files under
  `api/`; import paths are unchanged.
- **architecture-1-07 (first half):** `api_helpers.rs` moved to `otto-apiclient::rest`;
  otto-server keeps a re-export shim.
- **integrations-3-01:** each MCP tool call has its own timeout: `otto_wait_session` gets its wait
  plus 5 s (25 s by default), `browser_summarize` 15 minutes.
- **ux-design-3-04:** the MCP audit tab says "No calls match these filters" with Clear filters.
- **backend-correctness-3-03:** `extract_json_where` finds the first matching object, so a brace in
  prose no longer breaks the PR draft parse.
- **maintainability-3-04 and 3-12 (P10 items):** dead traits, constants and types deleted.
- **backend-performance-3-03:** the state archive orders rows in SQL and assembles the archive on
  the blocking pool.

### Not done

- **architecture-1-05:** the native MCP listings are still hand-written; the 7 drift pairs need
  executor changes.
- **architecture-1-07 (second half):** `execute`, `resolve_environment` and `run_step` are not
  behind an `ApiClientCtx` trait yet.

## Verifier: integration fixes

- **otto-server lib tests didn't compile:** the `pr_draft.rs` test module now imports
  `parse_pr_draft`. This one missing import had kept every fixer's otto-server lib tests from running.
- **Clippy:** `sweep_launcher_env_files` (P08) is a blocking helper that its only caller runs through
  `spawn_blocking`, so it allows `std::fs::read_dir` like the crate's other sync helpers; removed a
  useless `.into()` in the new broker test harness (P04); the test-only `acting_user_id` import is
  `#[cfg(test)]`.
- **LOC ratchet:** three tracked files grew past their caps, so code moved out instead of raising
  them. `run_identity` and `acting_user_id` went to `workflow_engine/run/identity.rs`;
  `SessionManager::sync_workspace_mcp` went unchanged to `otto-sessions/src/manager/mcp_sync.rs`;
  the git noise filters (`is_noise_line`, `strip_noise`) went to `otto-git/src/output_noise.rs`, and
  `worktree_remove` is shorter. The misplaced doc comment above `pub use crate::remote_url::…` in
  `local.rs` is back on `is_git_dir_name`.
- **svelte-check:** the workflow inspector imported `ProductStory` from the `lib/api/types` barrel,
  which doesn't re-export the product types. It now imports from `lib/api/types/product`.
- **UI guards:**
  - `unlabeled-control` flagged the new `IdPicker` and `RepoPicker`, whose controls get their id from
    a required `id` prop. Every caller pairs that id with a `<label for>`. The guard now accepts a
    control bound to a required `id: string` prop, the same way it already accepts a snippet's id
    parameter.
  - `SqsView.svelte` had grown past 800 lines with P05's batching. The fan-out moved into the aws
    store as `fanOutSqsAttrs` (2 in flight, committed 8 at a time, stops when the view starts a
    newer load), and the view is back under 800 lines.
  - `--update-baseline` locked in every shrunk file. It raised 12 `file-loc` entries that grew with
    this iteration's fixes: `EditFlow.svelte.ts` +24, `UsagePage.svelte` +25, `sql-builder.ts` +18,
    `SchemaTree.svelte` +17, `model.ts` +9, `database.svelte.ts` +8, `ConnectionForm.svelte` +7,
    `MonitorCluster.svelte` +6, and `Scene3DViewport.svelte`, `QueryBuilder.svelte`,
    `DiffViewer.svelte` and `graph-commit-diff.svelte` +1 each. Against them, WorkflowsPage
    (−1,453), OverviewTab (−503), App (−234), Terminal (−57) and the other shrunk files were lowered.
    `events.svelte.ts` was set to 815, which includes a separate 4-line commit made on the branch
    during this verification (`5e1473454`, own-key event routing).
- **UI unit test:** `unit/mcpAuditPaging.test.ts` extracts `load` from `AuditTab.svelte`, and P10's
  new `filtered` state was missing from its fixture (`ReferenceError`). The fixture declares it, and
  the test now also checks that a filtered load sets it.
- **Handoffs closed:** `history.svelte.ts` uses `loadErrorText` (its unit test maps `/loadError`);
  the `commandSearch.ts` comment no longer names the deleted `lib/fuzzy.ts`; "Couldn’t" in
  `RecapImage.svelte` and `RecapSettingsModal.svelte`.
- **Docs:** `mcp-control-plane.md` describes the owner-or-root rotate rule (PC-5) in all four places.
  `api.md` and `workflows.md` describe PC-3 as built: the run's pinned graph version, any trigger of
  the workflow (the firing trigger isn't stored), and `updated_by_at`. `product.md` and
  `product-design-arena.md` point at `ui/src/lib/design/`.
- **Restored** `crates/otto-usage/server/config.xml`, whose ClickHouse port a test run rewrote again.
- **Rust LOC baseline:** `scripts/loc-ratchet.py --update-baseline` lowered `local.rs` (5,548),
  `workflow_engine.rs` (5,853), `manager.rs` (6,729), `run_workflow_with_budget` (1,250) and the
  otto-server production total (108,401 to 107,813).
- **Reviewed and kept:** P01's hand edit of `tests/snapshots/policy_decisions.txt` (the
  `policy_coverage` golden test passes against it) and P06's edits to three unit tests outside its
  paths (they now run the real issue client).

### Gate result

`scripts/check.sh --base origin/main` (whole workspace, because `Cargo.lock` changed) passed: rustfmt,
clippy `-D warnings`, nextest (6,199 tests passed, 125 skipped), doc-tests, the LOC ratchet, the
keyed-lock registry guard, `npm run check` (ui-guards, svelte-check with no errors or warnings, and
the three tsc projects), `npm run test:unit` (2,279 passed), the build and the bundle budget. Every
test a fixer wrote but could not run passed here, including the otto-server lib tests (`lsp`
root tests, `workflow_run_files`, the policy assertions, `run_workspace`, `repo_directory`,
`resource_sessions`, `pr_draft`, `state_archive`), the `it` suites `swarm_router` (with the two new
lifecycle tests), `review_agent_retry` and `admin_sessions`, and the otto-review, ottod
`mcp_tools` budget, otto-assistant and otto-sessions tests.

One earlier run failed `snips::raw_15mb_upload_and_annotated_save_within_budget` (annotated save
356 ms) while the machine was under load (load average about 20). No snips code changed in this
iteration; the test passed alone (0.28 s) and in the final full run. The test's time budget is
sensitive to machine load. Playwright E2E was not run.
