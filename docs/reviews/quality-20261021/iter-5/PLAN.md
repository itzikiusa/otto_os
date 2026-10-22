# Quality iteration 5: fix plan

Worktree: `/Users/itziklavon/claude_ade-iter5` (branch `fix/quality-iter5-20261022`, base `main` 8177c4d85).
Inputs: the 15 lens reports in this folder plus five coordinator items (C1 to C5).
Output: 10 work packages (P01 to P10) for 10 parallel fixers in one shared worktree.

## Rules every package follows

1. **Owned paths are exclusive.** Edit only paths your package owns. A finding that also touches another
   package's file has a "part" entry in that package's instructions; do not edit it yourself.
   - If you discover that a fix needs a file you do not own, stop. Put it in your final report as a hand-off
     (path, exact change). Do not edit the file.
   - Files nobody owns are read-only. This includes `scripts/loc-baseline.json`,
     `scripts/ui-guards-baseline.json` and `ui/src/lib/components/{LoadState,EmptyState}.svelte`.
     `LazyMount.svelte` is the exception: P05 owns it.
2. **Splits are behaviour-preserving moves.** Keep the old path compiling with a re-export (`pub use`, or a
   TS re-export) so other packages keep building. Do not change behaviour in the same edit as a move.
   - Every function you create must stay under the `function_max: 400` cap.
   - Ratchet baselines are only lowered, and only by the integrator after all packages land
     (`python3 scripts/loc-ratchet.py --update-baseline`, `node scripts/ui-guards.mjs --update-baseline`).
   - If a move needs a baseline entry renamed, report the exact entry; do not edit the baseline.
3. **Contracts.** Only P06 edits `docs/contracts/**`, `ui/src/lib/api/types.ts`, `ui/src/lib/api/types/**`
   and `crates/otto-core/src/api/**`. Every other package must pick a fix that keeps the wire shape as it
   is. Each package's instructions name that choice.
4. **Migrations.** Only P06 adds `crates/otto-state/migrations/0195_*.sql`. Only P02 may add `0196_*.sql`
   (optional). No other package adds a migration. Both follow the AGENTS.md rollback-compat rules.
5. **Integration tests.** `crates/otto-server/tests/it.rs` belongs to P08. Everyone else adds tests to suite
   files they own, or to `#[cfg(test)]` modules in their own sources. Do not create a new `tests/<suite>.rs`.
6. **Cargo.**
   - Only P07 runs `cargo update`, `cargo audit` or edits the root `Cargo.toml`.
   - Other packages may edit their own crate's `Cargo.toml`. The lockfile then changes implicitly, and you
     never edit it by hand.
   - Use scoped commands: `cargo check -p <crate>`, `cargo test -p <crate> --lib <filter>`,
     `cargo clippy -p <crate> --all-targets -- -D warnings`.
   - Run at most one heavy cargo or npm command at a time. This Mac freezes under ~10 parallel builds.
   - Never run `cargo clean`.
7. **UI checks.** Run `cd ui && npm run check`, plus the unit tests you touched (`npx vitest run unit/<file>`).
   - New unit tests go in new files under `ui/unit/` with a package-specific name.
   - Edit an existing `ui/unit` or `ui/e2e` file only when it tests code you own (they are listed per package).
8. **Git.** No commits, stashes, resets or checkouts; the integrator commits. Other agents (games-splits,
   kart-stuck-fix, mcp-profiles, mcp-jira-create) are active in this session. Never overwrite changes
   you did not make.
9. **Final report.** For each finding id, report fixed, partial or skipped, with the reason, the tests you
   added, the commands you ran and their results, and any hand-offs.

## Package overview

| Pkg | Theme | Main items | Load |
|-----|-------|------------|------|
| P01 | Workflow engine split | arch-1-01r RunState, 4-04r, TQ-5-07/08, DI-5-03, keyed locks | ~25 |
| P02 | Git and review | arch-1-09 GraphView split, DI-5-01 (major), review summarizer, Slack and channels | ~25 |
| P03 | Product and Jira/Confluence | arch-5-04 OverviewTab split, UC-5-01 (major), BC-5-02 (major), C1 | ~23 |
| P04 | Database | arch-1-10 DatabaseStore split, BC-5-01 (major), DB UX, C4 | ~24 |
| P05 | Shell, Terminal and Workflows UI | arch-1-08r RunMonitor, arch-2-01r Terminal resize, UX-5-01 (major) | ~25 |
| P06 | Contracts, swarm and assistant access control | AC-5-01/02/03 (majors), TQ-5-06, api-contracts, budget, brokers | ~25 |
| P07 | API client, deps and Kubernetes | arch-1-07r REST engine move, TQ-5-05, gRPC roots, C2 deps, k8s/AWS | ~25 |
| P08 | Sessions, transcript and desktop | session_files/transcript perf, TQ-5-04 (major), EH-5-01 (major), serve_terminal split | ~25 |
| P09 | MCP and agent runs | arch-5-02 agent_session move, arch-1-05 MCP twins, arch-4-02 interim, MCP fixes, C3 | ~26 |
| P10 | Rooms, games and Home | arch-5-01 capability, arch-5-05 three stage, arch-2-08 catalog, UP-5-01 (major), games a11y | ~27 |

## P01: Workflow engine split, workflow integrity and keyed locks

**Owns:**
- `crates/otto-server/src/workflow_engine.rs` and `crates/otto-server/src/workflow_engine/**`
- `crates/otto-server/tests/workflow_run_files.rs`
- `crates/otto-state/src/workflows.rs` and `crates/otto-state/src/workflow_progress.rs`
- `crates/otto-automation/**`
- `crates/otto-assistant/src/personal_agents_engine.rs`
- `crates/otto-core/src/expr.rs`
- `crates/otto-server/src/api_secrets.rs`, `crates/otto-server/src/eval_score.rs` and `crates/otto-server/src/routes/backup_git.rs`
- `scripts/check.sh`
- `docs/features/workflows.md`, `docs/features/scheduled-tasks.md` and `docs/features/goal-loops.md`

**Findings:** architecture-1-01r, architecture-4-04r (review_run part), test-quality-5-07, test-quality-5-08,
data-integrity-5-03, backend-correctness-5-03, maintainability-5-02, error-handling-5-06, test-quality-5-03.

1. **arch-1-01r.** Introduce `RunState` (the run's mutable bookkeeping) in
   `workflow_engine/run/state.rs`. Split `run_workflow_with_budget` (`:1850`, about 1,250 lines) into
   `run/admission.rs` and `run/schedule.rs` plus a thin driver.
   - Also split `execute_node` (`:3677`) so its per-kind dispatch lives in `nodes/mod.rs`.
   - Make the moves first, then the extraction. Keep the `pub(crate)` names reachable from
     `workflow_engine.rs` with re-exports.
2. **arch-4-04r.** Split `nodes/review_run.rs::execute` (872 lines) along `resolve_targets` / `launch` /
   `await_and_score`. P08 does the `serve_terminal` half of this finding.
3. **TQ-5-07.** Add tests for the `git_pr` node, reusing the harness in `workflow_product_publish_tests.rs:166-203`:
   - Own-path target, dirty tree and `open:false`: HEAD is unchanged and the file is still uncommitted.
   - A dirty `git worktree add` target: exactly one new commit.
   - `open:true`, no git account and a forge 422: the error contains "asked to open a PR but none opened".
4. **TQ-5-08.** Extract the pure functions `notify_targets(all, ws, preferred)` and
   `reply_target(integ, origin_channel, origin_chat, origin_thread)` from `channel_notify.rs`, then unit-test:
   - Two workspaces: only the run's workspace is targeted.
   - A webhook or a blank `channel_id` is dropped.
   - The preferred filter applies.
   - A Slack-originated run replies in its thread. On Telegram the reply goes to the default chat with no thread.
5. **DI-5-03.** Workflow delete runs one transaction in `otto-state/src/workflows.rs`. It sets `enabled = 0`
   on `scheduled_tasks WHERE workflow_id = ?` and records why, using an existing note or last-error column.
   - No new column and no contract change.
   - `execute_workflow` (`otto-automation/src/scheduled_tasks_engine.rs:1311`) treats a missing workflow as
     one "workflow deleted – task paused" outcome that disables the task.
   - Add tests at both layers.
6. **BC-5-03.** Add a `depth` field to the expr `Parser`, counted per prefix operator in `parse_unary` and per
   `(` and call argument in `parse_primary`.
   - Past 64, return `ExprError::Parse("expression nests too deeply")`.
   - Tests: 10k `!` and 10k `(` both return `Err`.
7. **maintainability-5-02.** Replace these hand-rolled lock maps with `otto_core::keyed_lock::KeyedLocks`:
   `api_secrets.rs` `request_guard`, `eval_score.rs` `update_guard`, `workflow_progress.rs` `repair_gate`,
   `routes/backup_git.rs` `repo_lock` and `otto-automation/src/goal_loop.rs` `operations`.
   - The other owners convert theirs: P06 does `otto-assistant/src/assistant/tasks.rs`, P09 does
     `agent_session_lifecycle.rs` during its move and `otto-design/src/service.rs`.
   - At the very end, remove from `KEYED_LOCK_ALLOW` in `scripts/check.sh` every path that no longer
     matches the registry grep. Re-run that grep to decide.
8. **maintainability-5-01, check.sh part.** In `scripts/check.sh`, add a ratchet that forbids any new
   `export \* from '\.\./\.\./lib/` shim under `ui/src/modules`.
   - The count can only go down. Seed it with the count left after P03, P04, P05 and P07 finish; compute it
     last.
9. **EH-5-06.** A report write failure must no longer be recorded as a clean OK run.
   - Record it through the run's existing error/status fields as a failed run whose error reads "The report
     couldn’t be saved: {e}", and raise the run-failure notice.
   - No new field and no contract change. Add a test.
10. **TQ-5-03.** Make the 60 s hold a `cfg(test)` constant, like `SETTLE_HOLD` in `scheduled_tasks_engine.rs:669-671`.
    - Case 1: fail `UPDATE … personal_agent_schedules` with a `BEFORE UPDATE` trigger, drop the trigger after
      300 ms, and assert the cursor advanced.
    - Case 2: the trigger holds through every retry, and the final post-hold attempt settles.
11. **maintainability-5-03, budget part.** Repoint `nodes/budget_gate.rs:23` to `crate::budget::…` and every
    `routes::transcript::` use inside `workflow_engine/**` to `crate::transcript`.

**Verify:**
- `cargo test -p otto-server --lib workflow_engine`
- `cargo test -p otto-automation`, `-p otto-assistant`, `-p otto-core` and `-p otto-state`
- clippy on those crates
- `bash scripts/check.sh --no-ui --dry-run` to sanity-check the ratchet

## P02: Git, code review and channels

**Owns:**
- `ui/src/modules/git/**`
- `ui/src/modules/mission-control/WorkItemDetail.svelte`
- `ui/src/lib/inlineEdit.ts` (new)
- `ui/e2e/desktop-git-*.spec.ts` and `ui/unit/git*.test.ts`
- `crates/otto-git/**`
- `crates/otto-state/src/git.rs` and the optional migration `crates/otto-state/migrations/0196_*.sql`
- `crates/otto-server/src/modules/review/**` and `crates/otto-server/src/modules/review.rs`
- `crates/otto-server/tests/{review_agent_retry,review_comment_states,review_forge_post}.rs`
- `crates/otto-channels/**`
- `docs/features/git.md`, `docs/features/code-review.md` and `docs/features/channels-slack-telegram.md`

**Findings:** architecture-1-09, data-integrity-5-01, data-integrity-5-05, ux-design-5-15,
accessibility-responsive-5-01 (git + mission-control part), accessibility-responsive-5-04,
accessibility-responsive-5-14, error-handling-5-03, error-handling-5-04 (FocusView part), error-handling-5-05,
error-handling-5-10, concurrency-async-5-02, backend-correctness-5-04, integrations-5-04, integrations-5-05.

0. **Do this first (about one minute).** Copy `ui/src/modules/product/inlineEdit.ts` verbatim to
   `ui/src/lib/inlineEdit.ts`. P03 repoints the product importers and deletes the old file. Use only the lib
   path.
1. **arch-1-09.** Split `GraphView.svelte` (4,887 lines) into:
   - `git/graph/RefSidebar.svelte`
   - a `git/graph/actions.svelte.ts` store or handler module holding the git action handlers (checkout,
     merge, rebase, tag, revert, push, delete…)
   - any further pieces the file needs
   Keep it behaviour-identical. Fold in UX-5-15 afterwards:
   - drop `danger` from the "Revert commit…" row;
   - confirm "Delete tag on origin" with `confirmOutward` (where = origin · tag; who = everyone with remote
     access; note that CI may react).
2. **DI-5-01 (major).** In api-collections push/pull (`otto-git/src/http.rs:2156`):
   - Resolve every target through `confined_worktree_file(&rel, true)` and refuse a symlinked collections
     dir or file.
   - Make pull skip symlinks (`symlink_metadata`).
   - Validate all names and targets before writing anything, and write each file through a sibling temp
     file plus `rename`.
   - Test: a symlinked collection file is refused and its target is left byte-identical.
3. **DI-5-05.** Delete `pr_reviews WHERE repo_id = ?` in the same transaction as `delete_repo`; comments
   cascade.
   - Optionally add migration `0196` with a rollback-safe `DELETE` of orphan `pr_reviews`.
   - Add a test.
4. **A11y-5-01, git part.**
   - `PrDetail.svelte:251-272, 520-538` and `mission-control/WorkItemDetail.svelte:84-91, 402`: put
     `use:focusFirstField` on the editor container and `data-edit-for` on the trigger, and call
     `refocusEditTrigger(key)` after Cancel and after a save.
   - P03 does NotesTab and LearningsView. P06 does AgentDocuments.
5. **A11y-5-04.** In `graph-commit-diff.svelte:546` and its sibling, drop `aria-busy`.
   - Per-file Skeletons get `announce={false}`.
   - Announce the diff load once, from one always-mounted status.
6. **A11y-5-14.** In `DiffViewer.svelte:1364`, after `expandedLines.add(key)`, `await tick()` and focus that row.
7. **EH-5-03.** In `WipPanel.svelte:231`, store `loadErrorText(e)` and render a compact
   `LoadState what="this file’s diff"`. Its `onretry` bumps a counter that the select effect reads.
8. **EH-5-04, FocusView part.** `FocusView.svelte:118, 136, 252` → `providerErrorText(e)`.
9. **EH-5-05.** Store the summarizer failure cause on the summarizer row's existing
   `agent_states[idx].error`. That is the clipped error, or "returned no comments for N findings". No new
   field.
   - Render it in the badge tooltip and the note line in `ReviewPanel`, `ReviewAgents` and
     `LocalReviewPanel`, naming the row's real provider instead of "claude".
   - P05 makes the same text change in `workflows/RunAgents.svelte`.
10. **EH-5-10.** In `summarize.rs:288`, propagate a failed kept-comments read with `?` instead of treating it
    as an empty list. Add a test.
11. **CA-5-02.** In `review/retry.rs:236`, move everything after the claim into the spawned task, or add a
    drop guard that restores `review.agents[index]`. Add a test that drops the future before the spawn.
12. **maintainability-5-03, part.** Repoint `modules/review/routes.rs:367` to `crate::budget::…` and
    `tests/review_agent_retry.rs:253` to `otto_sessions::submit::…`.
13. **BC-5-04.** Factor `handle_event`'s drop predicate out of `slack.rs`:
    - keep only message and app_mention events;
    - drop anything with a `bot_id` at either level;
    - drop `message_deleted` and membership subtypes;
    - drop a `message_changed` that is not a human edit.
    Apply it in the `events_api` branch before `admit_chat`: ack, record the event in `seen`, and give it no
    admission or ingress ticket. Add tests.
14. **Int-5-04.** Add a Slack upload client with a 120 s `UPLOAD_TIMEOUT`, mirroring `build_download_client`,
    for the step-2 byte POST.
15. **Int-5-05.** In `mirror.rs:1090`, on an upload `Err`, retry once after `retry_after_hint` (capped like the
    reply path).
    - If it still fails, post "Otto could not attach `{filename}`: {short error}" in the thread.
    - Stub-adapter tests: fail once then succeed; always fail, which must produce the note.

**Verify:**
- `cargo test -p otto-git -p otto-channels -p otto-state`
- `cargo test -p otto-server --test it review_` and `--lib review`
- clippy on those crates
- `npm run check`, plus the git e2e specs: `npx playwright test e2e/desktop-git` (isolated harness)

## P03: Product module and Jira/Confluence

**Owns:**
- `ui/src/modules/product/**`
- `ui/src/lib/components/ConfluencePagePicker.svelte`
- `ui/src/modules/design-hall/site/**`
- `ui/unit/product*.test.ts`
- `crates/otto-product/**` and `crates/otto-issues/**`
- `docs/features/product.md` and `docs/features/jira-confluence.md`

**Findings:** architecture-5-04, ui-correctness-5-01, ui-correctness-5-02, ui-correctness-5-03,
accessibility-responsive-5-02, accessibility-responsive-5-09, ux-design-5-13, backend-correctness-5-02,
integrations-5-01, backend-performance-5-03, coordinator-1-confluence-verify-ensure-tls.

1. **arch-5-04.** Split `OverviewTab.svelte` (2,997 lines). Extract, as moves:
   - `product/jira/JiraIssueSection.svelte` (issue load, dev-status, title and description editors)
   - `product/attachmentUrls.svelte.ts`, shared with `AttachmentsPanel`
   - `DraftEditor.svelte`
   - `StoryTranscripts.svelte`
2. **UC-5-01 (major).** In TestCasesTab, RewriteTab and PlanTab, stop the reset effect from subscribing to
   the poller.
   - Keep the poller in a plain `let` with a separate `polling = $state(false)`, as AnalysisTab does, or use
     `untrack(clearPoll)`.
   - Unit test: after `generate()` the poller is still running and the button is still disabled.
3. **UC-5-02.** Read the gates untracked in the issue-account store loader, or wrap the loader calls in the
   three mount effects in `untrack`. `product.selectedId` should then be PublishDialog's only dependency.
4. **UC-5-03.** DesignArena: capture ownership with `product.captureSelection()` or a `++loadSeq` token, and
   assign results only while it is current. Clear `ownAtts`, `childAtts` and `selectedId` before `loadAll()`.
5. **A11y-5-02.** PlanTab: one always-mounted sr-only `role=status` (copy `RewriteTab:273`), and drop the role
   from the visible spans.
6. **A11y-5-01, product part.**
   - Repoint `OverviewTab`, `jira/IssueEditor.svelte` and `jira/issueEditorState.svelte.ts` to
     `$lib/inlineEdit` (P02 creates it first), then delete `product/inlineEdit.ts`.
   - Apply the contract to `NotesTab.svelte:52-57, 148-192` and `LearningsView.svelte:400, 524`. Name the
     buttons "Edit {title}".
7. **EH-5-04, PublishDialog part.** At `:126, 195, 237`, use `providerErrorText(e)`.
8. **UX-5-01, product part.** Render Product tabs with `<LazyMount variant='panel'>` instead of a bare
   `{#await import()}`. P05 owns `LazyMount` and adds `quiet` / `variant` if missing.
9. **maintainability-5-01, product part.** Repoint the `canvas/mermaid` importers in
   `MockupLivePreview.svelte:10`, `MockupViewer.svelte:15`, `ActionCard.svelte:25` and
   `design/exporters.ts:5` to `lib/render/mermaid.ts`.
10. **A11y-5-09.** ConfluencePagePicker:
    - after a pick, `await tick()` and focus Clear;
    - after Clear, focus the input;
    - after `active` changes, `scrollIntoView({block:'nearest'})` the active option.
11. **UX-5-13.** `design-hall/site/PublishModal.svelte:170` stores `{what, detail: loadErrorText(e)}`.
    - Render the headline "Couldn’t export the site." / "Couldn’t publish the preview." / "Couldn’t open the
      preview."
    - Put the detail on a dim second line.
12. **BC-5-02 (major).** In `storage_check.rs:159`, return `Err` once `stack.len() > 256`.
    - In the close-tag branch, refuse when `stack.len() == 1`, or validate the name.
    - Tests: `</#root>`, and 10k-deep nesting.
13. **Int-5-01.**
    - Route the Jira create `get_json` through `self.t.send_raw`.
    - Map 401/403 through `self.t.fail` before the "does not exist" arm, and drop 403 from that arm.
    - Send the POST through `send_raw` and map its non-2xx through `fail`.
    - Wire tests: createmeta 401 gives Forbidden; a 429 with `Retry-After: 0` is retried.
14. **C1.** In `otto-issues/src/confluence/verify.rs` `read_back`, call `self.ensure_tls()?` before attaching
    `Authorization` (the CodeQL 478 follow-up).
    - Test: an `http://` base URL is refused, and no request reaches a wiremock.
15. **maintainability-5-03, part.** Remove the `md_storage` re-export at `otto-issues/src/confluence.rs:1544-1546`
    and repoint its callers inside `otto-issues` / `otto-product`. For callers outside your paths, report a
    hand-off.
16. **BP-5-03.** Split `refresh_story` into `refresh_source(&story, &account) -> bool`, which the watcher
    calls, and a route wrapper that adds `story_detail`. Pass the loaded story and account into
    `list_new_comments`.

**Verify:**
- `cargo test -p otto-issues -p otto-product` and clippy on both
- `npm run check`
- `npx vitest run unit/product*`

## P04: Database Explorer (DatabaseStore split) and DB backend

**Owns:**
- `ui/src/lib/stores/database.svelte.ts` and `ui/src/lib/stores/database/**`
- `ui/src/lib/database/**` and `ui/src/lib/jsonTree/**`
- `ui/src/modules/database/**` and `ui/src/modules/workbench/**`
- `ui/unit/db*.test.ts` and `ui/e2e/desktop-db-*.spec.ts`
- `crates/otto-dbviewer/**`
- `.github/workflows/db-drivers.yml`
- `crates/otto-browser/src/lightpanda.rs` and `crates/otto-telemetry-collector/**`
- `docs/features/database-explorer.md` and `docs/features/workbench.md`

**Findings:** architecture-1-10, backend-correctness-5-01, test-quality-5-02, coordinator-4-structuredclone,
maintainability-5-01 (database shims, primary), ux-design-5-03, ux-design-5-05, ux-design-5-07,
ux-design-5-08, ux-design-5-12 (database sites), accessibility-responsive-5-08 (QueryBuilder part),
error-handling-5-04 (MultiRunDialog part), backend-performance-5-07.

1. **arch-1-10.** Split `DatabaseStore` (3,661 lines) into sub-stores behind the existing facade:
   - `stores/database/assist.svelte.ts`
   - `connections.svelte.ts` (connection lifecycle)
   - `schemaTree.svelte.ts`
   - `savedQueries.svelte.ts`
   - `history.svelte.ts`
   The public `database.*` API stays identical through delegating getters and methods.
2. **maintainability-5-01, database part.**
   - Retarget every importer of `modules/database/{sql-util,sql-dialect,filter-chips,error-normalize,mongo-format,grid-tab-state}.ts`
     to `lib/database/*`. This covers `src`, `ui/unit` (for example `dbSqlDialect`, `dbClipGrid` and
     `dbErrorNormalize`), `ui/e2e/desktop-db-query-filter.spec.ts:48`, and
     `modules/workbench/lib/{format,placeholders}.ts`.
   - Then delete those six shims, the store re-exports at `database.svelte.ts:61-62`, and the unused
     `lib/database/filter-chips.ts` `condLabel`.
   - Fix the comment at `stores/database/statement.ts:52-54`.
3. **C4.**
   - `DatabaseChanges.svelte:91`: `structuredClone($state.snapshot(selected.targets))`.
   - Check `lib/jsonTree/expansion-plan.ts:196`. If `root` can be a `$state` proxy, snapshot it first.
4. **UX-5-03.** Make `fetchSchemaGraph` throw. DiagramView keeps `graphError` and renders
   `<LoadState what='the diagram' error={graphError} empty onretry={loadGraph}/>` before the `!graph` branch.
5. **UX-5-05.** In `workbench/send/sendTo.ts:56`, when `connectionsError` is set and the list is empty, toast
   "Couldn’t load your database connections" with a "Try again" action. Say "No database connections" only
   for a successful empty load.
6. **UX-5-07.** WidgetCard renders through
   `<LoadState what={widget.title} variant='compact' loading={loading && !result} {error} empty={!result} onretry={() => run(true)}>`.
   Drop `.wc-stale`.
7. **UX-5-08.**
   - On Add, show "Runs on {connection}", and disable the primary button with a reason when no connection is
     selected.
   - On Edit, offer the columns from the widget's own last run, and update the hint.
8. **UX-5-12, database part.** Clipboard-failure toasts in `StructureView.svelte:84`,
   `ErrorPanel.svelte:83`, `QueryBuilder.svelte:797`, `ResultsGrid.svelte:1159` and `results-format.ts`:
   use the body "Select the text and copy it manually.", or route them through `copyWithToast`.
   - P05 does `ScheduledTasksPage.svelte:1383`. P06 does `swarm/RunInspector.svelte:115`.
9. **A11y-5-08, QueryBuilder part.** At `QueryBuilder.svelte:1114, 1136`, after removing a row,
   `await tick()` and focus the same control on the row now at that index, else the previous row, else Add.
   Name the buttons after their row. P07 does the API-client editors and P09 does `mcp/AllowlistsTab`.
10. **EH-5-04, MultiRunDialog part.** At `:98, 198, 318`, use `loadErrorText(e)`.
11. **BC-5-01 (major).** Add a depth counter to the `mongo_parse.rs` Parser (object, array, constructor).
    Past 128, return "nesting is too deep". Test: 10k `[` returns `Err`.
12. **TQ-5-02.**
    - Seed `otto_reader`, `shop` and `hidden` in the `db-drivers.yml` MySQL service, and drop the test filter
      so the native `resource_access` and governed-root cases run.
    - Cover the in-flight revocation recheck in `otto-dbviewer/src/service/changes.rs` with `SELECT SLEEP(2)`
      plus a mid-run `cancellation_requested=1` write, asserting `Forbidden`.
13. **BP-5-07.** For the collector and the Lightpanda sidecar logs, add a periodic (about 10 min) size check
    that copies and truncates the `O_APPEND` file once it passes the cap, or pipe stderr through a capped
    rotating writer. Add a unit test of the rotation helper.

**Verify:**
- `cargo test -p otto-dbviewer -p otto-browser -p otto-telemetry-collector` and clippy
- `npm run check`
- `npx vitest run unit/db*`
- `npx playwright test e2e/desktop-db` (isolated harness)

## P05: App shell, Terminal, Workflows UI, agents and workspace events

**Owns:**
- `ui/src/shell/**`
- `ui/src/lib/components/LazyMount.svelte`, `ui/src/lib/components/Terminal.svelte` and `ui/src/lib/components/termGridSync.ts`
- `ui/src/lib/stores/workspace.svelte.ts` and `ui/src/lib/stores/canvas.svelte.ts`
- `ui/src/lib/router.svelte.ts`, `ui/src/lib/events.svelte.ts`, `ui/src/lib/events/**` and `ui/src/lib/uiCommands/**`
- `ui/src/lib/render/**`
- `ui/src/modules/workflows/**`, `ui/src/modules/agents/**`, `ui/src/modules/scheduled-tasks/**` and `ui/src/modules/run-with-otto/**`
- `ui/src/modules/canvas/**`, `ui/src/modules/vault/**` and `ui/src/modules/panels/**`
- `ui/src/modules/connections/**` and `ui/src/modules/settings/AccessGroups.svelte`
- `ui/src/modules/home/boxes/SessionsBox.svelte`
- `ui/unit/{chatV2,events*,resync*,terminal*,workflows*}.test.ts`

**Findings:** architecture-1-08r, architecture-2-01r, architecture-1-13 (lib side), ux-design-5-01 (shell,
primary), maintainability-5-04, maintainability-5-07, data-integrity-5-02, ui-correctness-5-04,
accessibility-responsive-5-03, accessibility-responsive-5-10, accessibility-responsive-5-11,
accessibility-responsive-5-15 (Jira picker part), ux-design-5-04.

1. **arch-1-08r.** Move live run sync (`refetchRun` `:285`, `waitRunTerminal` `:1003`), the final-output panel
   and approvals (`approveRun` `:1445`) out of `WorkflowsPage.svelte` (3,710 lines) into a new
   `workflows/RunMonitor.svelte` (plus a `runMonitor.svelte.ts` if useful). Behaviour stays identical.
2. **arch-2-01r.** Move the resize half of `Terminal.svelte` into `termGridSync.ts`: `deferLocalResize`
   (`:1108`) and `sendResize` (`:1324`). Keep it behaviour-identical, and extend the existing terminal unit
   test.
3. **maintainability-5-04.** Delete `injectInput`, the `injections` state, the Terminal effect and both
   `lastInjN` re-syncs (`workspace.svelte.ts:1140`), and reword the `sendInput` doc.
4. **UX-5-01 (major).**
   - Hoist each `{#await import()}` in `shell/App.svelte:1206` (and its siblings) into a module-level
     `lazyComponent()`.
   - Render overlays and dialogs with `<LazyMount quiet …/>`.
   - When a chunk fails, reset the open flag or offer "Try again".
   - Add `quiet` and `variant='panel'` to LazyMount if missing; P03 uses `variant='panel'`.
5. **DI-5-02.**
   - Add `scheduledTasksStore.peek()?.resync()` (reload the list and the runs of expanded tasks) and
     `runWithOttoStore.peek()?.resync()` (reload the list, the open run and its events) to `resyncStores()`,
     or register the page loaders with `liveEvents.onResync`.
   - Extend the unit test that pins the resync set.
6. **arch-1-13, lib side.** Remove the `lib → modules` imports in `uiCommands/k8s.ts` (point it at
   `lib/k8s/viewState`), `uiCommands/home.ts`, `stores/canvas.svelte.ts`, `router.svelte.ts` and the single
   imports inside your owned lib paths.
   - Move each helper into `lib/`, and leave a re-export at the module path only if a non-owned importer
     remains.
   - P06 does `stores/swarm.svelte.ts`.
7. **maintainability-5-07.** Delete the four uncalled accessors in `router.svelte.ts:530`, point the
   query-filter doc at `mongoIdFilterFor`, and reword the `from_k8s_kind` doc.
8. **maintainability-5-01, canvas/vault part.**
   - Repoint `agents/conversation/Markdown.svelte:86` and `panels/CanvasPanel.svelte:98` to
     `lib/render/mermaid.ts`, and `ui/unit/chatV2.test.ts:40` to `lib/render/markdown.ts`.
   - When `rg` shows no importer left, which includes P03's product ones, delete `modules/canvas/mermaid.ts`
     and `modules/vault/mdRender.ts`.
9. **UC-5-04.** In ToolStep and its siblings, key the each-blocks by index: `as id, i (i)`, `as o, i (i)`,
   `as t, i (i)`.
10. **A11y-5-03.** NewSession: give each card a `role=none` wrapper, and put the steppers in a sibling
    `role=group` (`aria-label="{p} sessions"`) outside the radiogroup.
11. **A11y-5-15, picker part.** JiraIssuePicker gets one always-mounted polite status ("{n} issues" /
    "No issues found"). P07 does CollectionsTree.
12. **A11y-5-10.** Vault NoteView: render tags as `<button type=button class=tag>`, and make the embed title a
    real link.
13. **A11y-5-11.** BrowserPanel: end `addAnnotation` with `void tick().then(() => takeoverBtnEl?.focus())`.
14. **UX-5-04.** SessionsBox:
    - wrap it in `LoadState` with `loading={ws.sessionsLoading && !ws.sessions.length}` and
      `error={ws.sessionsError}`;
    - move the EmptyState into `emptyView`;
    - show "—" in the stats until the list loads.
15. **Parts of other findings:**
    - UX-5-02: `agents/conversation/ConversationView.svelte:865` → LoadState.
    - EH-5-04: `connections/NetworkProfiles.svelte:43, 56`, `panels/CanvasPanel.svelte:101, 128` and
      `settings/AccessGroups.svelte:131` use `loadErrorText(e)`.
    - UX-5-12: `ScheduledTasksPage.svelte:1383` clipboard body.
    - EH-5-05: in `workflows/RunAgents.svelte`, name the real provider and show the cause instead of
      "claude unavailable".
    - maintainability-5-01: repoint `vault/LiveContext.svelte:79` to `lib/k8s/viewState`. P07 deletes the
      k8s shims.

**Verify:**
- `npm run check`
- `npx vitest run` for the touched unit files
- `npx playwright test` for the workflows and terminal specs (isolated harness)

## P06: API contracts, swarm and assistant access control, budgets and brokers

**Owns:**
- `docs/contracts/**`, `ui/src/lib/api/types.ts`, `ui/src/lib/api/types/**` and `crates/otto-core/src/api/**`
- `crates/otto-swarm/**`, `crates/otto-sessions/src/submit.rs` and `crates/otto-server/src/swarm_host.rs`
- `crates/otto-state/src/swarm.rs` and `crates/otto-state/src/swarm/**`
- migration `crates/otto-state/migrations/0195_*.sql`
- `crates/otto-assistant/src/assistant/**`
- `crates/otto-server/src/routes/personal_agents.rs`, `crates/otto-server/src/budget/**` and `crates/otto-server/src/routes/usage.rs`
- `crates/otto-brokers/**`
- `crates/otto-server/tests/{swarm_identity,swarm_router,swarm_scope_api,personal_agent_policy,personal_agents_owner}.rs`
- `ui/src/modules/swarm/**`, `ui/src/lib/stores/swarm.svelte.ts`, `ui/src/modules/personal-agents/**`,
  `ui/src/modules/usage/**` and `ui/src/modules/brokers/**`
- `docs/features/{agent-swarm,personal-agents,usage-and-cost,message-brokers,agent-context-injection}.md`

**Findings:** access-control-5-01, access-control-5-02, access-control-5-03, test-quality-5-06,
api-contracts-5-01, api-contracts-5-02, api-contracts-5-03, api-contracts-5-04, api-contracts-5-05,
error-handling-5-07, architecture-4-01r, concurrency-async-5-01, data-integrity-5-04, maintainability-5-08.

1. **AC-5-01 (major).**
   - Migration `0195`: nullable `swarm_tasks.prompt_updated_by`, plus `created_by` / `updated_by` on goals.
   - Set `prompt_updated_by` in `update_task` when the title, description or labels change. Record the
     goal editors.
   - `acting_user` prefers the latest person who changed the prompt or goals, mirroring
     `WorkflowsRepo::run_acting_user`.
   - Ignore or refuse labels that a person adds with a `source-run:` prefix.
   - Tests:
     - B edits and runs A's task: the session's `created_by` is B.
     - A's untouched task runs as A.
     - A status-only PATCH keeps A.
2. **AC-5-02 (major).**
   - Add a changed `schedule.directive` to `identity_changes`: only the swarm creator, a workspace Admin or
     root may change it. Cadence, `at` and `enabled` stay Editor-level, and a full-form save with the same
     directive passes.
   - Channel-trigger work launches with `created_by = trigger.created_by` when that user exists, otherwise
     the swarm creator.
   - Tests:
     - B changes the directive on A's agent: 403.
     - A cadence-only change: 200.
     - B's trigger runs as B, and A's trigger still runs as A.
3. **AC-5-03 (major).** `resolve_agent` (`otto-assistant/src/assistant/tasks.rs:343`) keeps only agents the
   caller may direct: their own, an ownerless one, a ws Admin of that workspace, or root.
   - Share the predicate with `require_agent_owner`.
   - Return 403 "This agent runs as its owner…" when only other people's agents match, and keep 404 for no
     match.
   - Tests:
     - B delegating to A's agent: 403, and no run row is created.
     - A delegating to their own agent: 200.
     - A ws Admin: 200.
     - A same-name tie resolves to the caller's own agent.
4. **TQ-5-06.** Extract `reusable_for(session, agent_id, user)` from `runtime/run.rs:232-237` and unit-test it:
   - another creator → false;
   - archived or exited → false;
   - the same agent and user → true.
   Add a manager-backed `find_agent_session` test, and pin `:790` with a host that records the creator.
5. **arch-4-01r.**
   - Give `otto_sessions::submit::submit_prompt` a run-active guard closure.
   - Convert the swarm's paste + Enter assembly (`runtime/host.rs:105-107`, `agent_run.rs:355-401`,
     `run.rs:832-876`) to use it.
   - Drop the `review_session` alias use in `swarm_host.rs:126-139`.
6. **EH-5-07.**
   - `stored_config` returns a `Result`.
   - `GET /usage/budgets` maps an error to an `ApiError`, which the UsagePage shows as LoadState + Retry.
   - `check_with` treats it like unavailable spend.
   - `UsageBudgetStatus` gains `spend_unavailable`, rendered as "Spend unavailable". Update `api.md`
     (`:185`, `:3041`) and the TS types together.
7. **API contracts.**
   - **5-01:** rewrite the CP4 `PATCH /mcp/servers/{id}` cell to match the route: one secret map re-reads the
     blob, and a failure is 409 (unavailable, invalid, or missing while `has_secret`) with nothing written;
     both maps replace the blob.
   - **5-03:** mark `account_id` as required (400 without it) on the four issue-tracker listing rows.
   - **5-05:** fix `api.md:355` to use the real field names and the real 403 message, including the new
     directive rule from item 2.
   - **5-04:** remove `status` and `BrokerReplayStatus` from `types/brokers.ts`.
   - **5-02:** add `warning?: string` to `DryRunResp` (`modules/brokers/types.ts:44`). GroupsTab renders it
     above the preview (`--warning`, `role=status`) and disables or relabels Reset while it is set. Add a unit
     test.
   - Also update api.md for AC-5-03 (`:4867`, `:4911`).
8. **CA-5-01 / DI-5-04.**
   - Run the broker replay publish in `tokio::spawn` and await the `JoinHandle`, with the final audit inside
     the task.
   - Alternatively, use a drop guard that checkpoints the evidence and marks the row `interrupted`.
   - Test: drop the replay future after N sends; the row is not left `running`.
9. **Parts of other findings:**
   - **A11y-5-01:** `personal-agents/AgentDocuments.svelte:106, 112` uses `$lib/inlineEdit`.
   - **arch-1-13:** remove the `modules/` imports from `stores/swarm.svelte.ts`.
   - **maintainability-5-02:** convert `assistant/tasks.rs` `task_lock` to `KeyedLocks`.
   - **maintainability-5-03:**
     - Repoint `swarm_host.rs:105` and `routes/personal_agents.rs:971`.
     - Delete the `routes/usage.rs:466` budget re-export once `rg "routes::usage::(check_budget|BudgetVerdict)"`
       comes back empty. If any hit remains, keep the re-export and report it.
   - **UX-5-12:** `swarm/RunInspector.svelte:115` clipboard body.
   - **maintainability-5-08:** in `agent-context-injection.md:128`, rename the step to `injection_for_home`.
     Drop the uncommitted "Design spec:" lines in `agent-context-injection.md` and `personal-agents.md`. P07
     drops the one in `kubernetes-monitoring.md`.

**Verify:**
- `cargo test -p otto-swarm -p otto-assistant -p otto-brokers -p otto-state`
  (`migration_compat` is included)
- `cargo test -p otto-server --test it swarm_ personal_agent`
- clippy on those crates
- `npm run check` and the brokers/usage unit tests

## P07: API client REST engine, dependency vulnerabilities, gRPC TLS, Kubernetes and AWS

**Owns:**
- `crates/otto-server/src/routes/api_client.rs` and `crates/otto-server/src/routes/api_client/**`
- `crates/otto-server/src/workflow_api_request.rs`, `crates/otto-server/src/routes/api_automation_runs.rs`,
  `crates/otto-server/src/routes/api_oauth.rs` and `crates/otto-server/src/modules/browser_proxy.rs`
- `crates/otto-apiclient/**` and `crates/otto-k8s/**`
- root `Cargo.toml`, `Cargo.lock`, `ui/package.json`, `ui/package-lock.json`, and any `deny.toml` or
  `.cargo/audit.toml`
- `ui/src/modules/api/**`, `ui/src/modules/kubernetes/**`, `ui/src/modules/aws/**`,
  `ui/src/lib/k8s/**` and `ui/src/lib/api/k8s.ts`
- `ui/unit/k8s*.test.ts` and `ui/unit/aws*.test.ts`
- `docs/features/{api-client,kubernetes-console,kubernetes-monitoring,aws-console}.md`

**Findings:** architecture-1-07r, test-quality-5-05, integrations-5-03, coordinator-2-dependency-vulns,
integrations-5-02, ux-design-5-02 (k8s + AWS sites, primary), ux-design-5-09, ux-design-5-16,
error-handling-5-02.

1. **arch-1-07r.** Move `execute` (`api_client.rs:1808`), `execute_service` (`:1826`) and
   `resolve_environment` (`:2017`) into `otto-apiclient::rest`, behind a small ctx trait implemented in
   otto-server.
   - Keep `pub use` re-exports at the old paths.
   - Repoint the non-route callers (`workflow_api_request.rs:50`, `routes/api_automation_runs.rs:103`,
     `modules/browser_proxy.rs:117, 229`, `routes/api_oauth.rs:120, 286`) to call `otto_netguard` directly
     instead of the `routes::api_client::net_guard` re-export (`:86`). Then delete that re-export.
2. **TQ-5-05.** Unit test in `api_client/automations.rs`:
   - Set up a human-saved request on wiremock A with a `$secret` bearer, and an automation step whose
     pre-request script moves the URL to wiremock B.
   - `run_step` with `Some(binding)`, built as `start()` builds it: `!step.ok`, the error names B's host,
     and B receives `.expect(0)` requests.
   - With `None`, the request is sent.
3. **Int-5-03.** Enable tonic `tls-native-roots`, and use `.with_native_roots().with_webpki_roots()` at both
   sites in `grpc.rs`. Optionally honour `tls_verify=false`. Add a test that the TLS config builder includes
   the native roots.
4. **C2, dependency vulnerabilities.**
   - Run `gh api repos/itzikiusa/otto_os/dependabot/alerts?state=open` and `cargo audit`. Run cargo audit
     inside the shared cargo lock or throttle wrapper; no parallel builds.
   - Upgrade to patched versions where a semver-compatible fix exists (`cargo update -p <crate> --precise`,
     and `npm` for UI alerts).
   - For an alert with no compatible fix, document the reason in `docs/security/dependency-advisories.md`
     (new). Add an ignore entry only with a written reason next to it.
   - List every alert with its outcome in your report.
5. **Int-5-02.** On the port-forward transport (`scrape.rs:590`):
   - Read the response through a capped chunk stream (`MAX_BODY+1`).
   - Decode it lossily.
   - Cut at a char boundary, using a helper shared with `fetch_proxy`.
   - Test: a fake forward serves 9 MiB of `0xFF`; the body is capped and nothing panics.
6. **UX-5-02.** Replace the EmptyState-as-error blocks with `<LoadState what=… error={err} empty onretry=…/>`.
   - AWS sites: `EksView:159`, `RdsView:176`, `AthenaView:460, 608`, `Ec2View:238`, `S3Browser:590, 662`,
     `SqsView:376`. Keep "Sign in" through `errorActions`.
   - Kubernetes sites: `KubernetesPage:132`, `MonitorOverview:117`, `MonitorFleet:622, 643, 667, 747, 790`,
     `MonitorCluster:481, 521`.
   - P05 does `ConversationView`.
7. **EH-5-02.** Use `awsLoadText(e)` at `RdsView:75, 122`, `EksView:72, 96`, `Ec2View:87, 172`, `SqsView:110` and
   `MetricsPanel:75`, and render the detail errors through `awsErrorText`.
8. **UX-5-09.** MonitorSettings: drop the "Check the form" toast.
   - Render `<p class='field-error' id=…>` linked through `aria-invalid` and `aria-describedby`.
   - Focus the first invalid field when a save fails.
   - Its `structuredClone(r.config)` calls clone API responses, not `$state`, so leave them.
9. **UX-5-16.** Add an `onclearfilter` prop to ResourceTable, wired to ClusterWorkspace's filter, with
   `actionLabel='Clear filter'`. Name the query in the title.
10. **Parts of other findings:**
    - **EH-5-04:** `kubernetes/LogsView.svelte:184` and `MetricsView.svelte:49` use `providerErrorText(e)`.
    - **maintainability-5-01:** retarget the importers of `kubernetes/{resourcePoll,viewState}.ts` and
      `kubernetes/monitor/tickCoalescer.ts`, including `vault/LiveContext.svelte:79` (hand-off to P05) and
      `ui/unit/k8s*`, to `lib/k8s/*`.
      - P05 repoints `uiCommands/k8s.ts`.
      - Delete the three shims once `rg` shows no importer left.
      - Fix the `lib/api/k8s.ts:105` pointer.
    - **A11y-5-08:** in `api/RequestBuilder.svelte:1180, 1193`, `EnvironmentsView:294, 317` and
      `EnvSelector:164, 177`, focus after a row is removed, and name the buttons after their row.
    - **A11y-5-15:** keep the CollectionsTree status (`:356`) mounted and empty when there is no filter.
    - **maintainability-5-08:** drop the "Design spec:" line in `kubernetes-monitoring.md`.

**Verify:**
- `cargo test -p otto-apiclient -p otto-k8s` and `cargo test -p otto-server --lib api_client`, plus clippy
- `cargo build --workspace` once after the dependency bumps
- `npm run check`
- the k8s/AWS unit tests

## P08: Sessions, files, transcripts, desktop supervisor and review-engine re-exports

**Owns:**
- `crates/otto-server/src/routes/session_files.rs` and `crates/otto-server/src/routes/fs.rs`
- `crates/otto-server/src/routes/transcript.rs`, `crates/otto-server/src/transcript/**`,
  `crates/otto-server/src/transcript_tail.rs` and `crates/otto-server/src/transcript_cache.rs`
- `crates/otto-server/src/{monitor,assistant_host,history_index,agent_tasks_nudge,product_host}.rs`
- `crates/otto-server/src/routes/admin_sessions.rs` and `crates/otto-server/src/routes/goal_loops.rs`
- `crates/otto-sessions/**`, except `src/submit.rs` (P06) and `src/mcp.rs` (P09)
- `crates/otto-review/**`
- `crates/otto-state/src/workbench.rs` and `crates/otto-state/src/mcp_audit.rs`
- `crates/otto-server/tests/{it,session_files,admin_sessions,resume_missing_cwd,session_screen}.rs`
- `apps/desktop/**`, `ui/src/App.svelte` and `ui/src/modules/desktop/**`
- `docs/features/{agent-sessions,daemon-http-api,multi-window}.md`

**Findings:** test-quality-5-04, backend-performance-5-01, backend-performance-5-02, backend-performance-5-04,
backend-performance-5-05, backend-performance-5-06, architecture-3-03r, concurrency-async-5-04,
test-quality-5-10, error-handling-5-01, maintainability-5-03 (primary).

1. **TQ-5-04 (major), in `tests/session_files.rs` `boot()`.**
   - Call `otto_core::secret_paths::protect_dir_for_tests` on the canonical temp data dir.
   - Add a carol session whose cwd is the data dir's parent.
   - Assert:
     - `<data>/personal/agent-2/AGENTS.md` returns 403 through that session;
     - her persona-dir session still gets 200 on `memory/notes.md`;
     - `lsp_status` on `<data>/personal/agent-2` returns 403.
2. **BP-5-01.** In `/ws/lsp` open, filter in SQL first:
   - `created_by = ?` (indexed) OR `workspace_id IN` (the caller's admin workspaces);
   - `archived = 0`;
   - a `substr(cwd, 1, len) = prefix` test on the raw and canonical data dir, like `unarchived_cwds_under`.
   Canonicalize only the rows that pass.
3. **BP-5-05.**
   - Make `fs::filesystem_work` `pub(crate)`, and route session-files browse and read through it: the
     semaphore, the 10 s deadline, and `browse_canceled` checks in the per-entry loop.
   - TQ-5-04 runs after this, so it guards the change.
4. **BP-5-02.** Authorize History transcript and artifacts with one indexed query:
   `SELECT 1 … WHERE workspace_id=? AND created_by=? AND (transcript_path=? OR provider_session_id=?) LIMIT 1`.
   - Fall back to the scan only on a miss, skipping rows that carry a fresh missing mark.
   - On a fallback hit, persist the path with `record_found`.
5. **BP-5-06.** `POST …/transcript/touch` uses `ctx.manager.live_agent_sessions(&wid)`, and its doc comment is
   corrected. The route stays.
6. **arch-3-03r.**
   - Move `fold_opts` (`transcript.rs:147`) and `register_work_artifact` (`:645`) into `transcript/`.
   - Repoint the callers to `crate::transcript` / `crate::budget`: `transcript_tail.rs:368, 771, 825`,
     `monitor.rs:454`, `assistant_host.rs:90`, `history_index.rs:165`, `agent_tasks_nudge.rs:85` and
     `product_host.rs:480`.
   - P01 and P06 repoint theirs. Keep a re-export at the route path only while a hit remains.
7. **BP-5-04 (projection only).** The admin session overview selects only the 9 needed columns and skips
   `meta_json`. Response shape and defaults stay as they are. Pagination is deferred; see Deferred.
8. **CA-5-04.**
   - `SessionManager::create` refuses with `Conflict('the daemon is shutting down')` before
     `spawn_session_pty`.
   - If the flag flips after the spawn: kill the handle, revoke the MCP token, delete the row.
   - Add a test.
9. **arch-4-04r, serve_terminal part.** Split `otto-sessions/src/ws.rs` `serve_terminal` (584 lines) by moving
   the ClientFrame match into a `TerminalConn` struct with per-frame methods. This is a behaviour-preserving
   move.
10. **TQ-5-10.** Add a new it suite, `tests/mcp_profiles_edges.rs` (P08 owns `it.rs`; add its `#[path]` line):
    - `PATCH /sessions/{id}` with `meta.mcp_profiles=["full"]`: 403 with the session's managed token, 200 for
      the owner, and 200 for an agent's unchanged round-trip.
    - `POST /mcp/http?profile=database`: `tools/list` has no `k8s_*` tool, and `tools/call k8s_logs` returns
      the profile denial. Without `?profile`, the tool is listed.
11. **EH-5-01 (major).**
    - `ui/src/App.svelte` (desktop) listens for `otto://daemon-state`, keeps the last report, and shows
      `report.detail` on the offline screen.
    - Add a "Restart daemon" button that invokes `daemon_restart` and then resumes the health poll, and
      "Reinstall daemon" when the report says the install failed.
    - Call `daemon_status` / `daemon_start` on mount for late windows.
    - Mirror the action in the tray (`apps/desktop/src-tauri`, `modules/desktop/TrayPage.svelte`), and update
      `not_answering_detail`.
    - Check the Tauri workspace separately: `cd apps/desktop/src-tauri && cargo check`.
12. **maintainability-5-03 (primary).**
    - Remove the `bracketed_paste`, `dispatched` and `submit_prompt` re-exports at
      `otto-review/src/session.rs:860-864`, and the `submit.rs:17-18` doc advertising them.
    - Remove the workbench `content_digest_hex` alias and repoint `mcp_audit.rs:29`.
    - Repoint `routes/goal_loops.rs:69, 88, 237, 424, 481` to `crate::budget`.
    - The other owners repoint their own callers: P01 `budget_gate`, P02 `review/routes` and the retry test,
      P06 `swarm_host` and `personal_agents` (and it deletes the `usage.rs` re-export), P09 `agent_session`,
      P03 `md_storage`.
    - Delete each re-export only when `rg` shows zero users. Otherwise leave it and report the stragglers.

**Verify:**
- `cargo test -p otto-sessions -p otto-review -p otto-state`
- `cargo test -p otto-server --test it session_files admin_sessions mcp_profiles_edges`
- `cargo test -p otto-server --lib transcript`
- clippy on those crates
- the Tauri `cargo check`
- `npm run check`

## P09: MCP control plane and agent-run engines (agent_session move)

**Owns:**
- `crates/otto-server/src/agent_session.rs`, `agent_session_lifecycle.rs` and `agent_session_reply.rs`
- `crates/otto-agent-run/**`
- `crates/otto-server/src/{mockup_assist,db_assist,design_scene3d,automation_ctx}.rs` and `crates/otto-server/src/modules/ctx_impls.rs`
- `crates/otto-canvas/**`, `crates/otto-design/**` and `crates/otto-design-assist/**`
- `crates/otto-mcp/**`, `crates/otto-state/src/mcp_control.rs`, `crates/otto-state/src/mcp_profiles.rs` and `crates/otto-state/src/mcp_servers.rs`
- `crates/otto-server/src/mcp_outward.rs`, `crates/otto-server/src/mcp_auto_approve.rs` and `crates/otto-server/src/routes/mcp_servers.rs`
- `crates/otto-server/tests/{mcp_auto_approve,agent_run_recovery}.rs`
- `crates/otto-sessions/src/mcp.rs`
- `crates/ottod/src/mcp_tools.rs` and `crates/ottod/src/mcp_server.rs`
- `ui/src/modules/mcp/**` and `ui/src/lib/userLabel.ts` (new)
- `docs/features/{mcp-control-plane,canvas,design-hall}.md`

**Findings:** architecture-5-02, architecture-4-02 (interim steps), architecture-1-05,
backend-correctness-5-05, concurrency-async-5-03, error-handling-5-08, coordinator-3-agy-git-exclude,
test-quality-5-09, test-quality-5-01, ux-design-5-06, ux-design-5-14, ui-correctness-5-05.

1. **arch-5-02, step 1.**
   - Move `agent_session{,_lifecycle,_reply}.rs` into `otto-agent-run/src/session_turn/`. They take
     `&Arc<SessionManager>` (plus `&DbPool` for `require_owned_resume`) instead of `&ServerCtx`.
   - Keep `pub use otto_agent_run::session_turn as agent_session` and thin `&ServerCtx` wrappers in otto-server,
     so all 16 callers compile unchanged.
   - Convert `acquire_lease` to `KeyedLocks` (maintainability-5-02 part), and call `otto_sessions::submit`
     directly at `:707` (maintainability-5-03 part).
2. **arch-5-02, step 2.** Drop the forwarding host-trait methods for canvas (`otto-canvas/src/assist_ctx.rs:56`
   → `ctx_impls.rs:552`) and design-assist (`otto-design-assist/src/ctx.rs:71` → `ctx_impls.rs:648`).
   - Expose `fn sessions(&self) -> &Arc<SessionManager>` and call `session_turn::run_*` directly, keeping each
     side's current turn options exactly as they are.
   - Keep the AutomationCtx and ProductStudioHost forwarders. Their crates belong to P01 and P03, and this is
     a follow-up.
3. **arch-4-02, interim.**
   - Give `mockup_assist` the StatGate poll.
   - Delete the `db_assist` `extract_fenced` copy and use `otto_agent_run::fence`.
   - Move `otto-design-assist/src/excalidraw.rs` into `otto-design::format`, with a re-export.
   - Move `design_scene3d.rs` into `otto-design`.
   - Phase 2 (the AssistTarget engine) is deferred; see Deferred.
4. **maintainability-5-02, part.** `otto-design/src/service.rs` `CommitLocks` → `KeyedLocks`.
5. **arch-1-05.**
   - Route the native twins in `ottod/src/mcp_tools.rs` (around `:2635`) through `route_for` with one prefix
     strip, and delete the `read_route` / `run_tool` arms for twins.
   - Add a parity test that every twin resolves through `route_for`.
   - Per memory: diff the control-plane catalog against the session registry before and after; they must stay
     identical.
6. **BC-5-05.**
   - Parse `ottod mcp-server --profile` with `otto_mcp::profiles::parse_ids` (trim, drop blanks).
   - An invalid id fails closed: exit with an error that names it.
   - Test with `"database, issues"`.
7. **CA-5-03.**
   - Expose one per-server `KeyedLocks` from `McpService`, and hold it from the read through `set_secret_meta`
     in both writers in `routes/mcp_servers.rs`.
   - `write_secret_env` re-reads the blob under the lock.
8. **EH-5-08.**
   - Validate MCP server names and env keys at save: 400 with the rule, mirrored inline in the mcp form. This
     is the existing 400 contract, not a new shape.
   - Quote non-bare names in the codex TOML (`otto-sessions/src/mcp.rs:374`).
   - Record runtime skips in the session meta JSON (`mcp_skipped`, free-form meta, no typed field), and show
     them in the session MCP menu only if that menu is in your paths; otherwise report a hand-off.
9. **C3.** When Otto writes `<repo>/.agents/mcp_config.json` for agy (`otto-sessions/src/mcp.rs`), append
   `.agents/mcp_config.json` to the repo's `.git/info/exclude` once.
   - Resolve `info/exclude` with `git rev-parse --git-path info/exclude` so worktrees work.
   - Never touch `.gitignore`.
   - Test: idempotent, the file no longer shows as untracked, and an existing exclude file is preserved.
10. **TQ-5-09.**
    - In the cross-workspace decide test, assert that bob's list is `[]` and
      `/mcp/approvals/count?status=pending` is 0.
    - In `workspace_less_otto_rows_are_only_the_callers_own`, insert a `Some("other-ws")` row and assert it is
      absent from the list and the stats.
11. **TQ-5-01.**
    - Create a card with `McpApprovalRepo::create` (`server_id` set, tool `t`) under a policy that lets ws
      Editor U discover but not approve.
    - U's decide gets 403 and the card stays pending.
    - Then add an approve rule and assert 200.
12. **MCP UI.**
    - **UX-5-06:** add a shared `lib/userLabel.ts`: "you", else the member's display name, else a `shortId`.
      Use it for `decided_by` and `requested_by` in ApprovalsTab, with the full ID in a title.
    - **UX-5-14:** add the title "Needs MCP admin" to "Make default" and to the ⋯ menu rows when `!isAdmin`.
    - **UC-5-05:** gate the ProfilesTab load as `if (!force && this.loaded)`, or untrack it.
    - **A11y-5-08, part:** in `mcp/AllowlistsTab.svelte:112, 124`, focus after a row is removed and name the
      buttons after their row.

**Verify:**
- `cargo test -p otto-agent-run -p otto-mcp -p otto-canvas -p otto-design -p otto-design-assist -p otto-sessions`
- `cargo test -p otto-server --test it mcp_ agent_run_recovery`
- `cargo test -p ottod`
- clippy on those crates
- `npm run check`

## P10: Rooms, room games and Home classrooms

**Owns:**
- `crates/otto-server/src/rooms/**` and `crates/otto-server/src/game_rooms/**`
- `crates/otto-server/tests/rooms_api.rs`
- `ui/src/modules/rooms/**`
- `ui/src/modules/home/**`, except `boxes/SessionsBox.svelte` (P05)
- `ui/src/lib/three/**` (new) and `ui/src/lib/relaySocket.ts` (new)
- `ui/src/modules/design-hall/studio3d/embed.ts`
- `ui/unit/{rooms,games,kart,shooter,classroom,home}*.test.ts` and `ui/e2e/*room*.spec.ts`
- `docs/features/{session-rooms,room-games,home}.md`

**Findings:** ui-performance-5-01, ui-performance-5-02, ui-performance-5-03, ui-performance-5-04,
architecture-5-01, architecture-5-05, architecture-2-08, error-handling-5-09, maintainability-5-05,
maintainability-5-06, accessibility-responsive-5-05, accessibility-responsive-5-06,
accessibility-responsive-5-07, accessibility-responsive-5-12, accessibility-responsive-5-13, ux-design-5-10,
ux-design-5-11.

Coordination: the games-splits and kart-stuck-fix agents are active in this session. Before editing
`rooms/games/**`, run `git status` and `git diff` on those paths, build on whatever is there, and never
revert it.

1. **UP-5-01 (major).** Key the classroom poll effect on
   `openRoomId = $derived(view.kind === 'corridor' ? null : view.roomId)`, not on the room object.
   - Read `byId` lazily inside the closures.
   - Drop the room-object dependency of the `pushScreen` effect the same way.
   - Throttle any status-driven `poller.now()` to at most once per `POLL_MS`.
   - Unit test: a status event does not recreate the poller.
2. **UP-5-02.** Wrap the ClassroomsBox load in `freshness()`, like the other Home boxes. Optionally add an
   IntersectionObserver `rootMargin`.
3. **UP-5-03.** Pass `zoomedAway = home.zoomedId === box.id`. The grid copy releases its scene and returns
   early, so only one WebGL context is ever live.
4. **UP-5-04.** In `SchoolList.svelte:78`, use `relTime(k.lastActiveAt)` directly.
5. **arch-5-05.** Add `lib/three/stage.ts`:
   - capped pixel ratio;
   - pause when off-screen or hidden;
   - remount after context loss;
   - dispose plus `forceContextLoss`.
   Use it from `rooms/games/common/view.ts` (GameView), `home/school/scene/stage.ts` (mountSchool) and
   `design-hall/studio3d/embed.ts`, preserving behaviour.
6. **arch-5-01.**
   - Add `crates/otto-server/src/rooms/capability.rs` with:
     - `Capability::issue`
     - a constant-time `Digest::matches`
     - `TokenBucket`
     - `socket_token(headers, protocol)`
   - Use it in both registries (`rooms/registry.rs` and `game_rooms/registry.rs:245`) and in both upgrade
     handlers. Take the stricter of the two current behaviours wherever they have drifted, and list each such
     place.
   - On the UI side, add a shared `lib/relaySocket.ts` and wrap it in RoomClient and GameClient.
   - Tests cover tokens, constant-time compare and rate limits. This is step 1 of arch-2-06.
7. **arch-2-08.** Serve one game catalog (map ids) from the server, or generate it, and read it from
   `rooms/games/catalog.ts`. Remove the hand-synced copy in `game_rooms/types.rs:11`. Add a test that the two
   sides agree.
8. **EH-5-09.**
   - When the reconnect attempts run out, emit a `gave-up` status and render "Couldn’t reconnect" with a
     Reconnect button (reset the attempts, then `connect()`).
   - Also retry on `online` and `visibilitychange`.
9. **maintainability-5-05.** Use `v: typeof SNAPSHOT_VERSION` and the constant in both the encoder and the
   decoder. Add a kart constant used the same way.
10. **maintainability-5-06.**
    - Delete the 16 dead room-game exports, or use them (`teamPlay`, `TEAM_SIZES`).
    - Import `canvas` and `finish` from `common`.
    - Rename kart's `SurfaceKind` and `surfaceTexture`.
    - Move `nums` into `common/math.ts`.
11. **A11y-5-05.**
    - Put the countdown digits in `aria-hidden` spans.
    - One always-mounted sr-only status per game announces the edges: round start, helicopter inbound,
      respawning, and the result headline.
    - Move the class picker out of the respawn status.
    - Add a polite lobby status.
12. **A11y-5-06.**
    - When the menu opens, `await tick()` and focus Resume.
    - Escape on the canvas opens the menu.
    - After `exit()`, focus the selected game's title button.
13. **A11y-5-07.** `.kart-overlay, .br-cover { overflow-y: auto; justify-content: safe center; }`, plus a compact
    result table at ≤ 640 px.
14. **A11y-5-12.** RoomRecap:
    - one always-mounted consent button whose label and action change;
    - one capture toggle;
    - an sr-only status for the finishing and paused states.
15. **A11y-5-13.** KidCard: on close, and after detention or kick, `await tick()` and focus the stage.
16. **UX-5-10.** The SchoolList row click opens the session or selects its KidCard, and Release becomes an
    explicit action. Disable it with a reason for people who can't manage the session.
17. **UX-5-11.** GamesHub create failure:
    `<p class='field-error' role='alert'>Couldn’t create the game room. {error}</p>`.
    Change GameGuest's "Could not" strings to "Couldn’t join …".
18. **C4, games part.** `KartGame.svelte:220, 257` and `ShooterGame.svelte:298, 336` call
    `structuredClone(state ?? view)`. If `view` or `state` is `$state`, use `$state.snapshot`.

**Verify:**
- `cargo test -p otto-server --test it rooms_` and `--lib rooms game_rooms`, plus clippy
- `npm run check`
- the rooms/games/home unit tests
- one isolated room e2e spec

## Deferred (not assigned)

- **architecture-5-03 (MCP governance unification).** This is a product and security decision. Making
  `governed_invoke` call the shared stages would apply existing `mcp_policies` and auto-approve rules to
  `otto.*` tools, and to registered servers, for the first time. That changes what admins' current rules
  allow or deny. It needs an explicit decision and a rule-migration plan, and is multi-day. The
  `mcp-control-plane.md` "One pipeline" row is left for that change, so the docs do not claim parity early.
- **architecture-1-04 (shared fan-out/summarizer runner).** Multi-day, and it depends on arch-5-02 landing
  first; P09 does that move this iteration. Schedule it next iteration on top of `otto-agent-run::session_turn`.
- **architecture-2-06 (extract an otto-rooms crate).** The crate extraction is multi-day (rooms, recap engines,
  ICE, game rooms and their ctx seams). Its step 1, the shared capability primitives, ships this iteration as
  architecture-5-01 in P10.
- **concurrency-async-1-05 (plugin free_port TOCTOU).** The fix changes the plugin protocol: the plugin binds
  `:0` and reports its port, or `/health` echoes `OTTO_PLUGIN_TOKEN`. That breaks existing third-party
  plugins, so it needs a protocol-version decision.
- **Partial deferrals inside assigned items:**
  - BP-5-04: pagination and the default view (live only, or live plus the last N exited) are a product
    decision; the column projection ships in P08.
  - arch-4-02: phase 2 (the AssistTarget engine) is multi-day, after arch-5-02; the interim steps ship in P09.
  - arch-5-02 step 2: the AutomationCtx and ProductStudioHost forwarders are follow-ups, because their crates
    are owned by P01 and P03 this round.
