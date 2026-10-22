# Quality iteration 2: fix plan

Ten work packages for ten parallel fixers in `/Users/itziklavon/claude_ade-quality`
(branch `fix/quality-loop-20261021`). Each package names its `owned_paths`. Owned paths are disjoint
across packages. Findings come from `docs/reviews/quality-20261021/iter-2/*.md`; read the full
finding there before fixing it.

This iteration does the big splits that earlier iterations deferred. Each split has its own package,
and that package exclusively owns the god file, its new sibling directory, and every other finding
that touches the file.

## Rules for every package

1. **Ownership.** Edit only your `owned_paths`. A few findings also need a small edit inside a file
   that another package owns. Those edits are listed under **Cross-package hunks** below and are
   done by the owner of the file, as part of the owner's own instructions. Never stash, reset,
   checkout or reformat files you don't own: other fixers are editing the same worktree.
2. **Splits are behaviour-preserving moves.**
   - Put new files in a child directory of the file being split (`workflow_engine/…`,
     `modules/…`, `routes/api_client/…`), so no crate root (`lib.rs` / `main.rs`) needs a new
     `mod` line.
   - Re-export everything that other code imports from the old path (`pub use …` in Rust, `export
     … from` / a thin wrapper component in the UI). Other packages must keep compiling without
     touching their imports.
   - Never change behaviour in the same hunk as a move. Fix findings after the move, in the new
     location.
3. **Size limits.** A new Rust file stays under 4,000 lines (loc-ratchet `new_file_max`). A new
   `.svelte` or `.ts` file stays under 800 lines (P10's new UI ratchet).
4. **Baselines belong to P10 and the integration step.**
   - No package except P10 edits `scripts/loc-baseline.json`, `scripts/ui-guards-baseline.json`,
     `scripts/loc-ratchet.py` or `scripts/ui-guards.mjs`.
   - After all packages land, the orchestrator runs `python3 scripts/loc-ratchet.py
     --update-baseline` and `node scripts/ui-guards.mjs --update-baseline`. Where otto-server's
     total legitimately grew, it uses `--raise` and says why.
   - If your only failure is a ratchet baseline, report it. Don't edit the baseline yourself.
5. **Tests.** Only P01 edits `crates/otto-server/tests/it.rs`, to add `swarm_router`. Everyone
   else adds tests in-module (`#[cfg(test)]`) or in an existing suite they own (listed in
   `owned_paths`).
6. **Contracts and docs.** P07 owns `docs/contracts/**`, `docs/features/**` and
   `ui/src/lib/api/types.ts`, which it splits into `ui/src/lib/api/types/**` behind an export-*
   barrel. When another package changes an API surface, the matching doc row is a cross-package hunk
   that P07 makes from the text given below. Other packages import types from
   `lib/api/types` (the barrel) as they do today.
7. **Machine load.** The user's rule is one heavy command at a time, because ten parallel builds
   froze this Mac.
   - Never run `cargo build --workspace`, `cargo test --workspace` or `scripts/check.sh --all`.
   - Use crate-scoped commands only: `cargo check -p X`, `cargo test -p X --lib <filter>`,
     `cargo clippy -p X --all-targets -- -D warnings`. Cargo serializes on the shared target lock,
     so don't fight it with a second target dir.
   - In the UI, run `npm run check` once, at the end. Errors in files you don't own come from other
     fixers' work in progress: report them, don't fix them.
   - Kill any vite, ottod or ClickHouse process you start.
8. **Report faithfully.** For each finding, say fixed, partly fixed (and what remains) or not
   fixed, plus the exact commands you ran and their results.

## Cross-package hunks

Each line names the finding, the package that owns the finding, the package that owns the file and
does the edit, and what to change.

| Finding (owner) | Edit done by | File and change |
|---|---|---|
| maintainability-2-02 (P09) | P06 | `crates/otto-dbviewer/src/http.rs:865`: replace local `expand_home` with `otto_core::paths::expand_home_strict` (P09 adds it; signature `pub fn expand_home_strict(p: &str) -> Result<PathBuf, String>`, `Err` on `~user`). |
| maintainability-2-02 (P09) | P05 | `crates/otto-git/src/http.rs:1459`: the same replacement, for the clone destination. |
| maintainability-2-04 (P09) | P04 | Delete `otto-assistant/src/personal_agents_engine.rs:1159` `is_one_shot`. |
| maintainability-2-04 (P09) | P06 | Delete `otto-dbviewer/src/types.rs:1179` `unimplemented`. |
| maintainability-2-04 (P09) | P01 | Delete `otto-swarm/src/runtime/workspace.rs:232` `ensure_cwd`. |
| maintainability-2-04 (P09) | P02 | Delete `otto-vault/src/store.rs:1001` `all_aliases`. |
| maintainability-2-08 (P09) | P07 | `api.md:847`: reorder body is `{ids:[...]}`. Remove the connection pin and section-reorder rows, because P09 deletes those routes. |
| maintainability-2-10 (P10) | P06 | Delete `database.svelte.ts:2141` `selectConnection`. |
| maintainability-2-10 (P10) | P07 | Delete `workspace.svelte.ts:277` `connectionSessions` (and the other dead ws members named in the report) and `apiClient.svelte.ts:606` `setTabResult`. |
| maintainability-2-07 / ux-design-2-06 (P10) | P05, P06, P08 | Replace local relative-time helpers with the shared `rel()` (P10 adds an optional `{ compact: true }` style): `git/FocusView.svelte:269` (P05), `database/DatabasePage.svelte:939` (P06), `shell/NotificationBell.svelte:286` (P08). |
| error-handling-2-02 (P10) | P07 | `apiClient.svelte.ts:334`: replace `errMsg` with `loadErrorText` at all its call sites. |
| error-handling-2-02 (P10) | P09 | `settings/PluginsSettings.svelte:54`: use `loadErrorText`. |
| ux-design-2-08 (P10) | P06, P07 | Copy fixes (plural()/formatCount, "Couldn't …", no "unknown error"): `DatabasePage.svelte:1143`, `ResultsGrid.svelte:211` and `StructureView.svelte:673` (P06); `apiClient.svelte.ts:1150` (P07). |
| ux-design-2-03 (P10) | P06 | Inline field errors (aria-invalid/aria-describedby, primary disabled while required fields are empty) in `database/ImportDialog.svelte:99`, `ExportDialog.svelte:141` and `VerticalView.svelte:257`. P10 adds `validate?: (v) => string \| null` to `promptText`. |
| ux-design-2-09 (P04) | P06 | `database/VerticalView.svelte:284`: one verb across the confirm body, title and button. |
| accessibility-responsive-2-08 (P08) | P05, P06 | `aria-expanded={selectedPath===path}` plus `aria-controls` on `.wp-name` in `git/WipPanel.svelte:740` (P05). `aria-expanded` on both toggles in `database/StructureView.svelte:1089` (P06). |
| accessibility-responsive-2-11 (P08) | P04, P05, P10 | Skeletons get `role=status` or sr-only "Loading …" text; counters get `aria-hidden` glyph plus sr-only text. Edits: `personal-agents/AgentPage.svelte:426` (P04); `git/GitToolbar.svelte:246`, `git/RepoView.svelte:362` and `git/DiffViewer.svelte:1503` (P05); `product/OverviewTab.svelte:1342` (P10). |
| architecture-1-11 (P10) | P04 | `WorkflowsPage.svelte`: route its direct `marked` call through `lib/md` (`renderMarkdown`, which already exists). |
| architecture-1-11 (P10) | P06 | Move `database/JsonTree.svelte` to `lib/components/JsonTree.svelte`. Leave a thin wrapper at the old path so the other 20 importers keep working. |
| architecture-1-12 (P10) | P07 | Break the `product/types.ts` ↔ `swarm/types.ts` cycle by moving the shared contract types into `lib/api/types/<area>.ts`. Both module files re-export them. |
| architecture-1-06 (P05) | none | P05 owns every file this finding touches. |
| integrations-2-01 (P09) | P05 | Make `resolve_git_common_dir` (`manager.rs:6851`) `pub(crate)`, signature unchanged, so `mcp.rs` can call `crate::manager::resolve_git_common_dir` without a new otto-git use. |
| access-control-2-01 / 2-03 (P04) | P07 | `api.md` personal-agent rows and goal-loop PATCH row: owner-only fields need the owner, a workspace Admin or root, else 403. `docs/features/personal-agents.md` and `goal-loops.md` say the same. |
| error-handling-2-04 (P09) | P07 | `api.md` plugin admin list: add `running: bool` and `last_error: string \| null`. `docs/features/plugins.md` gets a troubleshooting row for "not running". |
| backend-performance-2-02 (P02) | P07 | Only if P02 retires legacy `/workspaces/{wid}/history`: mark it in `api.md` as delegating to the paged history. |

## Packages

### P01: Workflow engine split, plus the swarm result router and paste-submit locking

- **Findings:** architecture-1-01, maintainability-2-05, test-quality-2-01, concurrency-async-2-02
- **Owned paths:**
  - `crates/otto-server/src/workflow_engine.rs` and `crates/otto-server/src/workflow_engine/**`
  - `crates/otto-state/src/workflows.rs`
  - `crates/otto-swarm/**`
  - `crates/otto-assistant/src/assistant/tasks.rs`
  - `crates/otto-server/src/routes/handover.rs`
  - `crates/otto-server/src/swarm_host.rs`
  - `crates/otto-server/tests/it.rs` and `crates/otto-server/tests/swarm_router.rs`

**Instructions:**

1. **Split `execute_node` (architecture-1-01).** It spans `workflow_engine.rs:3835–6665`.
   - Add `workflow_engine/nodes/mod.rs` with a `NodeCall` struct that bundles the arguments
     `execute_node` passes around today.
   - Add one `workflow_engine/nodes/<kind>.rs` per node kind, each with `pub(super) async fn
     execute(call: NodeCall<'_>) -> …`. Start with `review_run`, which is 972 lines, then `git_pr`
     and `loop`, then the rest.
   - `execute_node` becomes a dispatch `match` that calls them.
   - Split `run_workflow_with_budget` (`:1850–3257`) into `workflow_engine/run/{admission,
     schedule, finalize}.rs`.
   - Keep every public item reachable at its old path through `pub use`. `crate::workflow_engine::…`
     is imported across otto-server.
   - The move is behaviour-preserving: don't change logic, ordering or error text. Run `cargo test
     -p otto-server --lib workflow` and `cargo test -p otto-server --test it workflow` before and
     after.
2. **maintainability-2-05.** Delete the two node-cache repo methods in
   `otto-state/src/workflows.rs:849`. Keep the table, because migrations are append-only.
3. **test-quality-2-01.** Add the `crates/otto-server/tests/swarm_router.rs` suite plus its
   `#[path]` line in `it.rs`.
   - Use `ServerCtx::for_tests`, which implements `SwarmHost`. Drive `route_result_inner`
     (`otto-swarm/src/runtime/engine.rs:1009`) with hand-built `SwarmTurnResult` values.
   - Cover all six cases in the report: an operator cancel mid-turn; a paused stop versus a plain
     Stop; a failure below the retry ceiling versus at it; an IC with a manager and 2 subtasks
     producing one triage task labelled `hops:1`; a `hops:3` handoff escalating with no new task;
     a duplicate handoff deduped.
   - If `route_result_inner` is private, expose it `#[doc(hidden)] pub`.
4. **concurrency-async-2-02.** Route the paste, delay, Enter sequence (plus the dispatch-retry
   Enter) through `manager.submit_text(&sid, text)`, or hold `manager.submit_lock(&sid)` around it.
   Do this in:
   - `otto-swarm/src/runtime/run.rs:779` and `agent_run.rs:360`, inside the `guarded_dispatch`
     closure;
   - `otto-assistant/src/assistant/tasks.rs:890`, which also needs the paste-to-Enter delay;
   - `routes/handover.rs:805`.

   `manager.rs` belongs to P05: use the existing API and don't edit it. Add a
   `concurrent_submits_do_not_interleave`-style test per path.
5. **Cross-package hunk for P09 (maintainability-2-04).** Delete `ensure_cwd` at
   `otto-swarm/src/runtime/workspace.rs:232`.

### P02: modules.rs split, review lifecycle, transcript and history performance

- **Findings:** architecture-1-02, architecture-1-15, error-handling-2-06, backend-performance-2-01,
  backend-performance-2-02, backend-performance-2-03, backend-correctness-2-03, test-quality-2-02,
  test-quality-2-04, maintainability-2-13, maintainability-2-16
- **Owned paths:**
  - `crates/otto-server/src/modules.rs` and `crates/otto-server/src/modules/**`
  - `crates/otto-server/src/review_launch.rs`
  - `crates/otto-review/**`
  - `crates/otto-server/src/resource_sessions.rs`
  - `crates/otto-server/src/routes/transcript.rs`, `crates/otto-server/src/routes/history_page.rs`
    and `crates/otto-server/src/transcript_cache.rs`
  - `crates/otto-server/src/state_archive.rs` and `crates/otto-server/src/state_archive/**`
  - `crates/otto-server/src/policy.rs`, `crates/otto-server/tests/rbac_matrix.rs` and
    `crates/otto-server/src/state.rs`
  - `crates/otto-core/src/cancel.rs` and `crates/otto-core/src/cancel_signal.rs`
  - `crates/otto-state/src/workgraph.rs`, `crates/otto-vault/src/store.rs` and
    `crates/otto-design/src/store.rs`

**Instructions:**

1. **Split `modules.rs` (architecture-1-02).** Move code, unchanged, into:
   - `modules/ctx_impls.rs`;
   - `modules/review/{routes,run,retry,summarize,lifecycle}.rs`, exposing a `review::service` API;
   - `modules/pr_draft.rs`, `modules/session_input.rs`, `modules/browser_proxy.rs` and
     `modules/skill_staging.rs`.

   Replace `pub use otto_review::engine::*;` with explicit imports. `modules.rs` must keep
   re-exporting every item that the 8 importing files use (`crate::modules::…` appears 42 times,
   23 of them from `workflow_engine.rs`, which P01 owns). Don't change routes in
   `routes/mod.rs`; the browser-proxy handlers stay reachable through the re-export.
2. **architecture-1-15.** Add `review::lifecycle::transition(ctx, ws, review, status, error)`,
   which persists and broadcasts `Event::ReviewChanged` in one call. Replace the 17 hand-built
   sites in `modules` and `review_launch.rs`.
3. **error-handling-2-06** (`modules.rs:2092`). Make the diff write fatal. Settle the review as
   `error` with "Couldn't write the diff for the reviewers: <cause>" before spawning any agent. On
   retry, return a Problem instead of launching when the diff can't be re-materialised.
4. **backend-performance-2-01.**
   - In `routes/transcript.rs`, make `list_artifacts` resolve the work item once, load the existing
     (kind, ref) set once without payload, and insert only the missing artifacts.
   - Add `WorkGraphRepo::has_artifact(item_id, kind, ref)` in `otto-state/workgraph.rs` and use it
     in `register_work_artifact` instead of `artifacts_for` plus a scan.
   - Serve live sessions from the tail fold (a `live_artifacts` accessor beside
     `transcript_tail::live_tool`). `transcript_tail.rs` is unowned, so add the accessor in
     `transcript_cache.rs` or `routes/transcript.rs` if possible. Otherwise, skip that part and
     report it.
5. **backend-performance-2-02** (`history_page.rs:131`). When a persisted path exists but the file
   is gone, return `TranscriptMissing` without the root scan. The list view resolves only persisted
   paths. Make the legacy `transcript::history` handler delegate to `history_page::list`.
6. **backend-performance-2-03** (`transcript_cache.rs:161`). Charge with
   `fold_charge(stamp.target.bytes, folded.record_count())` plus the sidecar sizes, and drop the
   serialization pass.
7. **backend-correctness-2-03.**
   - Add a shared `like_escape(&str) -> String` that escapes `\` first, then `%`, then `_`. Put it
     in `otto-state/workgraph.rs`, or copy it per crate if the crates don't share a dependency.
   - Use `title LIKE ? ESCAPE '\'` at `workgraph.rs:747` and `:755`.
   - Also escape a literal backslash in `otto-vault/src/store.rs:1205-1219` and `:1388`, and in
     `otto-design/src/store.rs:2358`.
   - Add a test for the queries `50%` and `a_b`.
8. **test-quality-2-02.** Replace the `snapshot()` test helper in `state_archive` with
   `build_snapshot_parts`. Add the secret-filter test for portable true and false with the seeds
   the report lists: `slack_bot_token`, `openai_api_key`, `smtp.password`, `password_hash`, and an
   api_tokens and an auth_sessions row. `default_provider` must survive.
9. **test-quality-2-04.** Add `check_many` tests in `resource_sessions.rs` for:
   - two k8s sessions in namespaces a and b where policy allows only a;
   - a revoked SSH grant;
   - a disabled caller.

   Assert the order of the returned rows. Put the HTTP case (GET `/workspaces/{id}/sessions` as an
   Admin without the shell grant) in `tests/rbac_matrix.rs`.
10. **maintainability-2-13.** Delete the `/connections` branch at `policy.rs:511` and point the
    `rbac_matrix` assertions at `/workspaces/{id}/connections`.
11. **maintainability-2-16.**
    - Cross-reference `cancel.rs` and `cancel_signal.rs`: the polled `AtomicBool` versus the
      wakeable `CancelSignal`, and when to use which.
    - Spell `otto_core::cancel::CancelRegistry` one way everywhere in `state.rs`.
12. **Cross-package hunk for P09 (maintainability-2-04).** Delete `all_aliases` at
    `otto-vault/src/store.rs:1001`.

### P03: API-client REST engine extraction and otto-state write integrity

- **Findings:** architecture-1-07, backend-correctness-2-01, data-integrity-2-03,
  data-integrity-2-04, data-integrity-2-05, test-quality-2-03, integrations-2-03
- **Owned paths:**
  - `crates/otto-server/src/routes/api_client.rs` and `crates/otto-server/src/routes/api_client/**`
  - `crates/otto-server/src/workflow_api_request.rs` and `crates/otto-server/src/mcp_capabilities.rs`
  - `crates/otto-apiclient/**`
  - `crates/otto-core/src/auth.rs`
  - `crates/otto-state/src/api_client.rs`, `users.rs`, `aws_accounts.rs`, `workspaces.rs`,
    `mcp_servers.rs`, `broker_clusters.rs`, `mcp_control.rs` and `database_changes.rs`
  - `crates/otto-server/src/database_changes.rs`
  - `crates/otto-channels/**`

**Instructions:**

1. **architecture-1-07.** Extract `ApiClientService::execute(ctx, ws, actor, req)` and
   `run_step` / `resolve_environment` from the 4,444-line `routes/api_client.rs` into
   `otto-apiclient::rest`, behind an `ApiClientCtx` trait.
   - Implement the trait for `ServerCtx` in `routes/api_client/ctx.rs`.
   - Split the remaining handlers into `routes/api_client/{collections,requests,environments,
     history,automations}.rs`, re-exported from `routes/api_client.rs` so `routes/mod.rs` is
     unchanged.
   - Add `AuthContext::automation(user, kind)` in `otto-core/src/auth.rs` and use it in
     `workflow_api_request.rs:49-66` instead of the literal.
   - Make `workflow_api_request.rs` and the MCP capability call the service, never the axum
     handler.
   - Give code search a service function, `code_search_service(ctx, wid, user, q, path, max)`, and
     have `mcp_capabilities.rs:292` and the handler both call it.
   - For otto-apiclient's Cargo.toml, use only dependencies that already exist in the root
     `[workspace.dependencies]`. P05 owns the root `Cargo.toml`.
2. **backend-correctness-2-01** (`otto-state/api_client.rs:871`). Compute the cutoff with
   `TimeDelta::try_days(d).and_then(|d| Utc::now().checked_sub_signed(d))`; `None` means no age
   limit, so skip the DELETE. Clamp `history_max_days` to 36,500 in `history_retention`. Apply the
   same checked math in `auto_archive_stale`. Add a test with `max_age_days = 99_999_999`. The
   dialog clamp is UI work in `modules/api`, which P10 owns; mention it in your report.
3. **data-integrity-2-03.** Collapse each multi-statement PATCH into one `UPDATE … SET col =
   COALESCE(?, col), …, updated_at = ? WHERE id = ?`, or into one `pool.begin()` transaction:
   - `api_client.rs`: `update_environment`, `update_collection` and `update_automation`;
   - `users.rs`, `aws_accounts.rs` and `workspaces.rs`;
   - `mcp_servers.rs:128-186`;
   - `broker_clusters.rs:184-253`.

   In `broker_clusters`, write the safety flags in the same statement as the connection fields.
4. **data-integrity-2-04.** In the change ledger (`otto-state/database_changes.rs:374` plus the
   server side):
   - inside `finish`'s transaction, flip `running` attempts to `outcome_unknown` first, and make the
     classifier treat `running` as unknown;
   - propagate or retry `finish_attempt_progress` errors;
   - run the reconcile update and the finish classification in one transaction through a shared
     `finish_in(&mut tx)`;
   - extend `recover_interrupted` to any `outcome_unknown` change that has no unknown attempt.

   Add tests.
5. **data-integrity-2-05.** In the API-client routes (now `routes/api_client/*`), verify that a
   referenced collection's `workspace_id == wid` for both `parent_id` and `collection_id`. On
   update, reject `parent_id == id` and any ancestor cycle (recursive CTE in otto-state). Add tests.
6. **test-quality-2-03.** Add a pinned-risk-label rediscovery test in `mcp_control.rs`: discover as
   read, pin to dangerous, rediscover as read with a new description. The label stays dangerous,
   `risk_overridden` is set and the description is refreshed. Add the un-pinned control case and
   the omitted-field tests for patch and for the policy update.
7. **integrations-2-03.** Convert GitHub-flavoured Markdown before sending to Slack and Telegram
   (`otto-channels/src/slack.rs:449` and the Telegram sender):
   - Slack: `**x**` → `*x*`, `[l](u)` → `<u|l>`, headings → bold lines, tables → code block.
   - Telegram: render to `parse_mode=HTML` with escaping.

   Unit-test a typical agent answer through each converter.

### P04: WorkflowsPage split and automation ownership (personal agents, goal loops, scheduled tasks)

- **Findings:** architecture-1-08, maintainability-2-12, access-control-2-01,
  access-control-2-03, error-handling-2-03, ux-design-2-01, ux-design-2-04, ux-design-2-09,
  ui-performance-2-02
- **Owned paths:**
  - `ui/src/modules/workflows/**`, `ui/src/lib/api/workflows.ts` and `ui/src/lib/api/workflows.test.ts`
  - `ui/src/modules/personal-agents/**` and `ui/src/modules/scheduled-tasks/**`
  - `ui/src/lib/stores/personalAgents.svelte.ts`, `ui/src/lib/stores/scheduledTasks.svelte.ts`
    and `ui/src/lib/api/personalAgents.ts`
  - `ui/src/lib/cadence.ts` and `ui/src/lib/confirmOutward.ts`
  - `crates/otto-server/src/routes/personal_agents.rs` and
    `crates/otto-server/src/routes/personal_agents/**`
  - `crates/otto-server/src/routes/goal_loops.rs`, `crates/otto-server/src/routes/scheduled_tasks.rs`
    and `crates/otto-server/src/assistant_host.rs`
  - `crates/otto-automation/**` and `crates/otto-assistant/src/personal_agents_engine.rs`
  - `crates/otto-server/tests/personal_agent_policy.rs`

**Instructions:**

1. **Split `WorkflowsPage.svelte` (architecture-1-08, 5,158 lines).** Extract:
   - `NodeInspector.svelte`, with per-kind sub-forms in `workflows/inspector/<Kind>Form.svelte`;
   - a `workflowRun.svelte.ts` run controller;
   - `WorkflowList.svelte`.

   Each new file stays under 800 lines. The split is behaviour-preserving. While doing it, make the
   `marked` hunk for architecture-1-11: route the call through `lib/md`.

   P07 moves workflow-run tracking out of `ws` but keeps `ws` getters that delegate to the new
   store, so your reads of `ws.*` keep working. Don't depend on the new store's internals.
2. **maintainability-2-12.** Switch the four untyped workflow PATCHes to `patchWorkflow`. Delete
   `getWorkflowVersion` and its test assertion.
3. **access-control-2-01** (`routes/personal_agents.rs:345`). Mirror
   `scheduled_tasks::owner_only_changes`.
   - **Owner-only fields:** `soul_md`, `provider`, `model`, `cwd`, `browser` and `delivery`, plus a
     schedule's `directive` and `permission`, and `save_memory` / `save_context`. A change to any
     of them needs `created_by == caller`, a workspace Admin or root. Otherwise return 403 with "This
     agent runs as its owner; only the owner or a workspace admin can change …".
   - **Open to Editors:** name and avatar, enable and pause, cadence, run_now, cancel and rooms.
   - **Presets:** seed them with `created_by NULL` (the no-owner path).
   - **`chat_session`:** reuse the pinned session only when its `created_by` matches the caller;
     otherwise pin one per user.
   - **Tests** in `tests/personal_agent_policy.rs`: an Editor who isn't the owner gets 403; the
     owner and an Admin get 200.
4. **access-control-2-03** (`goal_loops.rs:288`). When `req.config` differs from `loop_.config`,
   require `created_by == caller`, a workspace Admin or root. Name and limits edits, and start,
   pause, resume and stop, stay open to Editors. Add in-module or policy-suite tests: a non-owner
   Editor gets 403, the owner and an Admin get 200.
5. **error-handling-2-03.** Port the personal-agents settle-retry loop (5 attempts with backoff,
   `warn!` per attempt, holding the in-flight guard across one scan) to both settle sites in
   `otto-automation/src/scheduled_tasks_engine.rs:525` and `:581`. Log `set_last_status` failures.
6. **ux-design-2-01.**
   - Move `savedDestWhere` and `destWho` from `ScheduledTasksPage` into `lib/confirmOutward.ts`.
   - Gate both personal-agent run-now functions (`AgentPage.svelte:262` and the run-once action)
     with `confirmOutward` whenever `delivery.type != 'none'`: verb "Run and deliver", where from
     `agent.delivery`, what is "the report of <agent>", plus who.
7. **ux-design-2-04.** Move `everyLabel` and `cadenceLabel` from `ScheduledTasksPage` into
   `lib/cadence.ts`, taking `(schedule, timezone)`. Use them in `AgentPage` / `personal-agents/
   cadence.ts` and in `TriggersPanel` (`describeSpec`). Delete the local copies. A 1440-minute
   cadence must read "Daily …", not "every 1440 min".
8. **ux-design-2-09.** In `TriggersPanel.svelte:176-189`, the toasts become "Trigger deleted" and
   "Couldn't delete the trigger", and the confirm message uses `KIND_LABEL[t.kind]`. P06 makes the
   `VerticalView` part.
9. **ui-performance-2-02** (`AgentPage.svelte:93`). Look the id up once per (workspace, agentId):
   keep a `lookedUp` key and do the lookup inside `untrack`, so "Agent not found" doesn't refetch in
   a loop.
10. **Cross-package hunks.**
    - accessibility-responsive-2-11: `AgentPage.svelte:426`.
    - maintainability-2-04: delete `is_one_shot` at `personal_agents_engine.rs:1159`.

### P05: GraphView split, git accessibility, CI-check rollups, and the telemetry and git-command crates

- **Findings:** architecture-1-09, ui-correctness-2-03, accessibility-responsive-2-01,
  accessibility-responsive-2-07, integrations-2-02, integrations-2-04, architecture-1-06,
  test-quality-2-06
- **Owned paths:**
  - `ui/src/modules/git/**`
  - `crates/otto-git/**`, `crates/otto-telemetry/**`, `crates/otto-telemetry-collector/**` (new)
    and `crates/otto-git-cmd/**` (new)
  - `Cargo.toml` (root), `crates/otto-server/Cargo.toml` and `crates/ottod/Cargo.toml`
  - `crates/otto-sessions/Cargo.toml`, `crates/otto-sessions/src/manager.rs` and
    `crates/otto-sessions/src/manager/**`
  - `crates/otto-server/tests/git_spawn_guard.rs`

**Instructions:**

1. **Split `GraphView.svelte` (architecture-1-09, 5,320 lines).** Extract:
   - `git/graph-lanes.ts`, the pure lane layout, with a unit test (vitest, as other `*.test.ts` files
     do);
   - `git/ref-menus.ts`;
   - `git/RefSidebar.svelte`.

   The split is behaviour-preserving, and each new file stays under 800 lines.
2. **ui-correctness-2-03** (`PrMergeModal.svelte:209`). Key the checks by index (`as c, i (i)`).
   Don't add a PrCheck id: that would be a contract change.
3. **accessibility-responsive-2-01** (`WipPanel.svelte:733`).
   - Before stage, unstage or discard, record the row's section and index.
   - After `onstatus` plus `await tick()`, focus the checkbox at the same index in the same section.
     Fall back to the last row, then to the section heading when the list is empty.
   - Add a polite sr-only region ("Staged src/a.ts").
   - Do the same after `discardPaths`.
4. **accessibility-responsive-2-07** (`GitTabs.svelte:232`).
   - Use a `div.git-tab role=presentation` wrapper holding `button role=tab` plus a sibling close
     button. Keep `tabindex=-1` and the Delete key.
   - Add `aria-label="Open repositories"` to the tablist.
   - Append ", uncommitted changes" to the tab name when `isDirty(r.id)`.
5. **integrations-2-02** (`providers/github.rs:286`).
   - Follow `Link rel=next` for check-runs, or report pending when `total_count` exceeds the rows
     read.
   - Use `GET /commits/{sha}/status` (one row per context) instead of the raw statuses list.
   - Follow Bitbucket's `next`.
   - Add a wire test with a failing run on page 2.
6. **integrations-2-04.** Map `("completed","stale")` to a terminal not-passed state and add it to
   the wire test table.
7. **architecture-1-06.** Use the least invasive shape:
   - **Telemetry.** Keep `otto-telemetry` as the leaf crate (scope / measure / measure_result plus
     a `SpanSink` trait) and drop its `otto-usage` dependency. Move the ClickHouse collector into
     a new `crates/otto-telemetry-collector` that only otto-server and ottod depend on. Add it to
     the root `members`.
   - **Git command.** Move `hardened_command` / `hardened_std_command` / `harden` /
     `HARDENED_GIT_CONFIG` / `test_hermetic` from `otto-git/src/local.rs:262` into a new
     `crates/otto-git-cmd` (deps: tokio `process`). `otto-git` re-exports them.
   - **otto-sessions.** Switch `manager.rs:6852` and `manager/tests/agent_cwd.rs` to
     `otto_git_cmd`, and drop `otto-git` from `otto-sessions/Cargo.toml`, provided nothing else in
     otto-sessions uses otto-git (check with grep first). Make `resolve_git_common_dir`
     `pub(crate)`; P09 calls it.
   - Update the `no_bare_git_spawns` guard allowlist if it names the old path.
   - Check with `cargo check -p otto-sessions -p otto-git -p otto-telemetry -p
     otto-telemetry-collector -p otto-server`, one at a time.
8. **test-quality-2-06.** In the `manager.rs` retire and remove tests, write the creds file and two
   server env files for the session, plus one env file for another session. After archive,
   suspend, kill or remove, the first three are gone and the other session's file remains.
9. **Cross-package hunks.**
   - maintainability-2-02: `otto-git/src/http.rs:1459`.
   - maintainability-2-07: `FocusView.svelte:269`.
   - accessibility-responsive-2-08: `WipPanel.svelte:740`.
   - accessibility-responsive-2-11: `GitToolbar.svelte:246`, `RepoView.svelte:362` and
     `DiffViewer.svelte:1503`.

   Don't edit `lib/stores/git.svelte.ts`; P10 owns it.

### P06: DatabaseStore split, database UI correctness and accessibility, SQL classification

- **Findings:** architecture-1-10, ui-correctness-2-01, ui-correctness-2-05, ui-correctness-2-06,
  ui-correctness-2-07, ux-design-2-02, error-handling-2-05, accessibility-responsive-2-02,
  accessibility-responsive-2-12, architecture-2-02
- **Owned paths:**
  - `ui/src/lib/stores/database.svelte.ts`, `ui/src/modules/database/**` and
    `ui/src/modules/home/boxes/DbDashboardBox.svelte`
  - `ui/src/lib/confirmProd.ts`, `ui/src/lib/stores/dbPrefs.svelte.ts` and
    `ui/src/lib/stores/db-result-budget.ts`
  - `ui/src/lib/components/JsonTree.svelte` (new)
  - `crates/otto-dbviewer/**`

**Instructions:**

1. **Split `DatabaseStore` (architecture-1-10, 4,383 lines).**
   - Move it to `modules/database/store/` as composed sub-stores (connections, tabs/query run,
     results paging, dashboards, multi-run, changes, prefs …) behind a thin `DatabaseStore` facade
     that keeps every public member name.
   - `lib/stores/database.svelte.ts` becomes `export { database, … } from
     '../../modules/database/store'`, so all importers keep working.
   - The split is behaviour-preserving, and each file stays under 800 lines.
2. **ui-correctness-2-01** (`WidgetCard.svelte:60`). `run` returns `'ok' | 'failed' | 'aborted'`.
   - Set `ranKey` and `okAt` only on `'ok'`.
   - On abort, clear `loading` unless a newer run owns it (a per-card run token).
   - The poller doesn't stamp freshness on abort.
3. **ui-correctness-2-05.** Remember `prev = t.offset` and restore it when the run returns null with
   no new result and `t.offset` is still `next`.
4. **ui-correctness-2-06** (`DbDashboardBox.svelte:46`). Drive the box's state from
   `database.dashboardsError`, with Retry calling `ensureLoaded(true)`.
5. **ui-correctness-2-07** (`MultiRunDialog.svelte:530`). Key the warnings by index.
6. **ux-design-2-02.**
   - Title the write gate by environment, and drop the ⚠ glyph.
   - On a non-null mismatch, toast "Nothing was changed — what you typed didn't match" (the
     confirmProd wording).
   - Make the store gate public and reuse it from `ImportDialog.svelte:175`.
7. **error-handling-2-05** (`QueryEditor.svelte:243`). Keep a separate `probeError =
   loadErrorText(e)` and show "Couldn't check for mongosh/Blender: <cause>" with Retry. Show the
   install hint only after a successful probe that reports the tool missing.
8. **accessibility-responsive-2-02** (`MultiRunDialog.svelte:733`). After each stage change,
   `await tick()` and focus the stage's first meaningful control: the typed-phrase input on
   confirm, a `tabindex=-1` heading on preview and results.
9. **accessibility-responsive-2-12** (`ResultsGrid.svelte:1384`). Add
   `<span class=sr-only>, failed</span>` to an errored result tab.
10. **architecture-2-02.**
    - Add `otto-dbviewer/src/sql_class.rs` with one `first_keyword()` and `ReadClass::of(engine,
      sql)`, built on `access::read_is_provable` for MySQL/Postgres and the tokenizer for
      ClickHouse, with the per-engine keyword lists in one table.
    - Point the drivers and `types.rs:1528` at it, keeping today's classification for every engine.
      Pin it with a table test built from the four current copies, and flag any difference.
    - Move `float_to_json`, `acquire_error` and the bulk-graph assembler into
      `drivers/sql_common.rs`.
11. **Cross-package hunks.**
    - maintainability-2-02: `otto-dbviewer/src/http.rs:865`.
    - maintainability-2-04: delete `unimplemented` at `otto-dbviewer/src/types.rs:1179`.
    - maintainability-2-10: delete `selectConnection`.
    - maintainability-2-07: `DatabasePage.svelte:939`.
    - ux-design-2-08: `DatabasePage.svelte:1143`, `ResultsGrid.svelte:211` and
      `StructureView.svelte:673`.
    - ux-design-2-03: `ImportDialog`, `ExportDialog` and `VerticalView` inline errors.
    - ux-design-2-09: `VerticalView.svelte:284`.
    - accessibility-responsive-2-08: `StructureView.svelte:1089`.
    - architecture-1-11: move `JsonTree` to `lib/components/JsonTree.svelte`, with a wrapper at the
      old path.

### P07: lib/api/types.ts split, contract docs, users access, typed workspace settings

- **Findings:** architecture-1-17, architecture-1-13, architecture-2-03, access-control-2-02,
  api-contracts-2-01 through api-contracts-2-13, maintainability-2-09, maintainability-2-15
- **Owned paths:**
  - `docs/contracts/**`, `docs/features/**` and `docs/design/product-design-arena.md`
  - `ui/src/lib/api/types.ts` and `ui/src/lib/api/types/**`
  - `ui/src/modules/swarm/types.ts`, `ui/src/modules/product/types.ts` and
    `ui/src/modules/canvas/types.ts`
  - `ui/src/lib/stores/workspace.svelte.ts`, `ui/src/lib/stores/apiClient.svelte.ts` and
    `ui/src/lib/stores/workflowRuns.svelte.ts` (new)
  - `crates/otto-server/src/routes/users.rs` and `crates/otto-server/src/routes/grants.rs`
  - `ui/src/modules/settings/Users.svelte` and `ui/src/modules/settings/sections.ts`
  - `crates/otto-server/tests/grants_api.rs` and `crates/otto-server/tests/admin_sessions.rs`

**Instructions:**

1. **Split `types.ts` first (architecture-1-17).** Move it mechanically into
   `lib/api/types/<area>.ts` (core, sessions, git, database, workflows, product, swarm, vault,
   design, k8s, aws, brokers, mcp, rooms, …). `types.ts` becomes an `export * from './types/…'`
   barrel.
   - Every name keeps resolving from `lib/api/types`. Run `npm run check` right after the split,
     before any other edit.
   - Watch for duplicate names across areas: the barrel must not create ambiguous re-exports.
   - Each area file stays under 800 lines where practical; split big areas further.
2. **architecture-1-13 and the cycle part of architecture-1-12.**
   - Move the product contract types that `types.ts:12115` re-exports from
     `modules/product/types` into `lib/api/types/product.ts`. `modules/product/types.ts` re-exports
     them.
   - Do the same for the swarm ↔ product shared types, so neither module file imports the other.
   - Remove any other `lib/` → `modules/` import in files you own. P10 ratchets the rest.
3. **architecture-2-03.**
   - Add a typed `WorkspaceSettings` (api_client, default_provider, notes, …) to
     `lib/api/types/core.ts` and document it in `api.md`. The Rust side stays a JSON object.
   - Keep only `patchSettings` and `current.settings` on `ws`.
   - Move the API-client settings accessors into `apiClient.svelte.ts`, and active workflow runs
     into a new `workflowRuns.svelte.ts` store.
   - Keep delegating getters and methods on `ws` with the old names. `events.svelte.ts` (P08),
     `WorkflowsPage` (P04) and `modules/api` (P10) still call them and must keep working unchanged.
   - Remove the `as` casts on settings.
4. **access-control-2-02.**
   - `users::list` relies on the feature guard (Users:Admin): drop its `require_root`.
   - Create, patch, delete and grants stay root.
   - `Users.svelte` shows Create, Disable and Grants only when `auth.isRoot`.
   - Fix the `api.md` rows (`/users/{id}/grants` is root) and `rbac-multiuser-sharing.md:449`.
   - Add tests in `grants_api.rs` / `admin_sessions.rs`: a Users:Admin non-root user can list but
     gets 403 on create and grants.
5. **api-contracts-2-01 through 2-13.** Apply each fix exactly as the report gives it:
   - trail/tasks rows;
   - library and skill-eval reads documented as member;
   - library PUT bodies and returns;
   - channels doc: ws editor + root, plus a troubleshooting row;
   - connections doc: SFTP root, global connections via feature capabilities;
   - brokers doc: replace the removed `/brokers/cluster-sections` routes; this also closes
     maintainability-2-09;
   - `UpdateDraftReq`;
   - product doc routes;
   - `move` returns `ConnectionSection`;
   - real type names;
   - a liveness paragraph for `/ws/events` and `/ws/term`: 30 s ping, 60 s inbound deadline, 10 s
     stalled send;
   - the missing TS mirror fields (2-12) in `swarm/types.ts`, `product/types.ts`,
     `canvas/types.ts` and the type areas;
   - room-recaps: GET is root too.
6. **maintainability-2-15.** Fix `vault.md:51` (`0105_vault_docs.sql`) and
   `docs/design/product-design-arena.md:16` (`crates/otto-skills/src/product.rs`).
7. **Doc hunks for other packages** (see the table): personal-agents and goal-loops owner-only
   rows (P04), plugin `running` / `last_error` (P09), connection pin and reorder route removal plus
   the `{ids:[...]}` body (P09).
8. **Store hunks for P10's findings:** error-handling-2-02 (`errMsg` → `loadErrorText` in
   `apiClient.svelte.ts`), ux-design-2-08 (`apiClient.svelte.ts:1150` copy) and
   maintainability-2-10 (the dead `ws` and apiClient members).

### P08: Terminal protocol extraction, the event route table, the App shell, shell accessibility

- **Findings:** architecture-2-01, architecture-2-04, architecture-2-05, ux-design-2-07,
  accessibility-responsive-2-03, accessibility-responsive-2-04, accessibility-responsive-2-05,
  accessibility-responsive-2-06, accessibility-responsive-2-08, accessibility-responsive-2-09,
  accessibility-responsive-2-11, ui-performance-2-05, test-quality-2-07
- **Owned paths:**
  - `ui/src/shell/**` and `ui/src/App.svelte`
  - `ui/src/lib/components/Terminal.svelte`, `termProtocol.ts` (new), `termGridSync.ts` (new),
    `termFlow.ts`, `termPark.ts` and `termGrid.ts`, all in `ui/src/lib/components/`
  - `ui/src/lib/events.svelte.ts` and `ui/src/lib/events/**`
  - `ui/src/lib/stores/rightPanel.svelte.ts` and `ui/src/lib/stores/auth.svelte.ts`
  - `ui/src/modules/assistant/**` and `ui/src/modules/workbench/**`
  - `ui/src/modules/agents/SessionView.svelte`,
    `ui/src/modules/agents/conversation/ConversationView.svelte` and
    `ui/src/modules/agents/conversation/PreviewBody.svelte`
  - `ui/src/modules/swarm/SwarmPage.svelte`
  - `ui/e2e/desktop-session-header.spec.ts`

**Instructions:**

1. **architecture-2-01** (3,632 lines).
   - Add `termProtocol.ts` with `decodeTermFrame(ev, sock)`. It owns the binary-header state and
     returns a discriminated union typed from `lib/api/types`.
   - Add one shared `applySnapshot(target, epoch, bytes)` for the live and parked engines.
   - `wireParked` and `wireSocket` become thin switches over the union.
   - Then move the resize/claim negotiation (`deferLocalResize` … `safeFit`) into a
     `termGridSync.ts` controller, following the `termFlow` / `termPark` pattern.
   - The extraction is behaviour-preserving. Run the terminal e2e specs (one slot, isolated daemon)
     at the end.
2. **architecture-2-04.**
   - Replace the 230-line if/else chain in `events.svelte.ts:706-965` with a typed route table,
     `{ [K in OttoEvent['type']]?: (ev: Extract<OttoEvent,{type:K}>) => void }`.
   - Split it per feature into `lib/events/routes/<feature>.ts`, each keeping its own
     `lazyModule` loader.
   - Add a unit test that every `OttoEvent` type is either routed or explicitly handled through
     `liveEvents` only.
   - Keep calling the `ws` methods for workflow runs: P07 keeps them as delegates.
3. **architecture-2-05.**
   - Move the palette sources from `App.svelte:669-940` to `shell/paletteSources.svelte.ts` with
     `installPaletteSources(): () => void`. Keep the git, mcp, vault and connections commands
     grouped per module inside that file; module directories belong to other packages.
   - Collect the one-shot global installers (menu bridge, nav prefetch, key map, badge) into one
     `installShell()` called from a single `onMount`.
4. **ux-design-2-07.** Move the permissions-failure banner (`ui/src/App.svelte:167` and
   `shell/App.svelte:1384`) into the StatusBar or an in-flow strip under the toolbar. If it must
   float, place it bottom-centre like Toasts, with a dismiss that holds until the next failed
   retry.
5. **accessibility-responsive-2-03.** In `collapse()`, focus the active rail `.strip-btn`, or reuse
   DockedDrawer's "refocus the opener when focus fell to body" effect.
6. **accessibility-responsive-2-04.** Remove `aria-live` from `.notes-foot`. Announce only failures,
   through an always-mounted region that holds the `role=alert` error branch.
7. **accessibility-responsive-2-05.** Keep one always-mounted `find-status` span (`aria-live=polite`
   `aria-atomic=true`) while the find bar is open, holding the count or "No results".
8. **accessibility-responsive-2-06** (`ChatView.svelte:195`). Add an always-mounted sr-only polite
   region that says "Otto is working" and "Otto replied" on the working true → false edge, as
   ConversationView does.
9. **accessibility-responsive-2-08.** Add `aria-expanded` to the disclosures in `shell/App.svelte`,
   `Navigator.svelte:1389` and `BottomNav.svelte:100`. P05 and P06 make their hunks.
10. **accessibility-responsive-2-09** (`Navigator.svelte:1573`). Add `aria-current` on the active
    session row.
11. **accessibility-responsive-2-11.** Fix the shell, assistant, conversation, swarm and workbench
    sites: `SidePane.svelte:120`, `shell/App.svelte:1367`, `assistant/*`,
    `ConversationView.svelte:867`, `PreviewBody.svelte:201`, `SwarmPage.svelte:631` and
    `WorkbenchPage.svelte:543`. P04, P05 and P10 make their hunks.
12. **ui-performance-2-05** (`SessionView.svelte:229`). Ensure once per id: keep an `ensuredFrom`
    guard and check for the session inside `untrack`.
13. **test-quality-2-07.** For counts 1 and 2, assert the chip unconditionally with `toBeVisible()`.
    For count 6, assert `toBeHidden()`.
14. **Cross-package hunk for P10 (maintainability-2-07).** `NotificationBell.svelte:286`: use
    `rel()`.

### P09: Backend hardening (secrets, file durability, k8s, plugins, path expansion, dead code, usage driver)

- **Findings:** integrations-2-01, data-integrity-2-01, data-integrity-2-02, data-integrity-2-06,
  concurrency-async-2-01, concurrency-async-2-03, concurrency-async-2-04,
  concurrency-async-1-05, error-handling-2-04, test-quality-2-05, backend-correctness-2-02,
  maintainability-2-01, maintainability-2-02, maintainability-2-03, maintainability-2-04,
  maintainability-2-08, maintainability-2-14, architecture-1-16
- **Owned paths:**
  - `crates/otto-sessions/src/mcp.rs`, `crates/otto-sessions/src/grid.rs` and
    `crates/otto-server/src/routes/mcp_servers.rs`
  - `crates/otto-context/**`, `crates/otto-server/src/skill_eval.rs` and `crates/otto-insights/**`
  - `crates/otto-improve/**`, `crates/otto-server/src/routes/meta.rs` and `crates/otto-k8s/**`
  - `crates/otto-server/src/plugins.rs`, `ui/src/modules/plugins/**`,
    `ui/src/lib/stores/plugins.svelte.ts` and `ui/src/modules/settings/PluginsSettings.svelte`
  - `crates/otto-brokers/**`, `crates/otto-server/src/boot/build.rs` and
    `crates/otto-server/src/test_support.rs`
  - `crates/otto-core/src/paths.rs` and `crates/otto-core/src/aws_env.rs`
  - `crates/otto-connections/**` and `crates/otto-aws/**`
  - `crates/otto-server/src/routes/workspaces.rs`, `crates/otto-server/src/routes/fs.rs` and
    `crates/otto-server/src/skill_review_static.rs`
  - `crates/otto-vault/src/engine.rs`
  - `crates/otto-memory/**`, `crates/otto-transcript/**`, `crates/otto-product/**` and
    `crates/otto-pty/**`
  - `crates/otto-usage/**` and `crates/ottod/src/usage_tailer.rs`

**Instructions:**

1. **integrations-2-01** (`otto-sessions/src/mcp.rs:262`).
   - On every reconcile, append the root-anchored `LAUNCHER_ARTIFACTS` paths to
     `<git-common-dir>/info/exclude`, idempotently. Resolve the directory with
     `crate::manager::resolve_git_common_dir`, which P05 makes `pub(crate)`. Never add an
     `otto_git::` use to otto-sessions: P05 drops that dependency.
   - Create launcher files with mode 0600.
   - Preferably, keep secrets out of the work tree for JSON/TOML launchers by reusing the codex
     `ENV_EXEC_SCRIPT` mechanism: a 0600 env file in `temp_dir/otto-mcp`, with no `env` in the
     launcher.
   - Add a test that a reconcile with secret env never writes the value under cwd.
2. **data-integrity-2-01** (`otto-context/src/library.rs:247`). Store `(mtime, len)` with each
   cached skill and re-stat it in `get_skill`, re-reading on a mismatch. Add a regression test: an
   external write to SKILL.md is visible to the next `get_skill`.
3. **data-integrity-2-02.**
   - Add `write_atomic` to otto-context: a temp file in the same directory, `sync_all`, rename,
     then a directory fsync. Use it in all four writers.
   - Write a `.bak` before a skill-eval promotion overwrites an existing skill
     (`skill_eval.rs:2673`, `promote_skill`).
4. **data-integrity-2-06** (`otto-insights/src/lib.rs:160`). Write `config.json.tmp`, then rename it
   over `config.json`. Log a warning when an existing config fails to parse.
5. **concurrency-async-2-01** (`grid.rs:145`). Route `send_grid` through the bounded send helper, or
   wrap it in `timeout(SEND_TIMEOUT, …)`.
6. **concurrency-async-2-03** (`otto-improve/src/engine.rs:782`). Claim first: CAS `Applied` →
   `RolledBack` before the file I/O, and revert the status on a failed write. Both conflict exits
   use `set_edit_status_if(edit_id, Applied, Conflict, …)`. Add a concurrent-rollback test.
7. **concurrency-async-2-04.** Add `.kill_on_drop(true)` to the three probes in `meta.rs:210`.
8. **The plugin cluster** (`plugins.rs`): error-handling-2-04, concurrency-async-1-05 and
   test-quality-2-05.
   - **2-04:** expose `running` and `last_error` in the admin list. PluginsSettings shows "Enabled ·
     not running — <cause>" with Start again. PluginFrame checks `running` and shows an inline
     "service stopped" state with Start again.
   - **1-05:** fix the `free_port` bind-then-release race by echoing `OTTO_PLUGIN_TOKEN` in
     `/health` and verifying it in the readiness probe, or by having the plugin bind `:0` and
     report its port.
   - **test-quality-2-05:** make the watcher test reach the generation guard, or extract the
     guarded removal into a function and unit-test it with a stale generation.
   - Also make the error-handling-2-02 hunk in `PluginsSettings.svelte:54`.
9. **backend-correctness-2-02** (`otto-k8s/src/monitor/queries.rs:201`). Split at
   `char_indices().last()`, and use the checked `TimeDelta::try_*` constructors, mapping `None` to
   Invalid("window must be at most 90d"). Add the `1é` and `9999999999999d` test cases.
10. **maintainability-2-01.** Make the ops repo a required argument of `BrokersService::new` and
    delete `with_ops`. Update `boot/build.rs:62` and `test_support.rs:109`. Add a router test that
    GET `/brokers/clusters/{id}/lag-alerts` returns 200, in-module or in a brokers test.
11. **maintainability-2-02.**
    - Add `otto_core::paths::expand_home_strict(p: &str) -> Result<PathBuf, String>`: it is
      `expand_tilde` plus the `~user` refusal from `fs.rs`. Move the `~bob` test into otto-core.
    - Use it for the user-supplied paths: SFTP and transfers in otto-connections, `s3_download`
      in otto-aws, `routes/workspaces.rs`, `routes/fs.rs` and `skill_review_static.rs`.
    - In `otto-vault/src/engine.rs`, replace `shellexpand_home`.
    - P05 and P06 replace their copies (see the table).
12. **maintainability-2-03.** Loop over `AWS_ENV_REMOVE` with `cmd.env_remove` in the k8s spawn
    helpers (`cli.rs`, `logs.rs`, `monitor/scrape.rs`, `monitor/gateway.rs`) and in the four
    otto-aws sites. Rename the k8s wrapper to `aws_env_from_row`.
13. **maintainability-2-04.** Delete the dead pub functions you own: `librdkafka_version`,
    `group_access_denied`, `new_keyword_only`, `default_grok_home` (check that `usage_tailer`
    doesn't need it), `discover_agy_conversation_dbs`, `cancellation_flag` plus `Story.flag` and
    its store in `begin_delete`, and `wait_output_closed_blocking`. P01, P02, P04 and P06 delete
    theirs.
14. **maintainability-2-08.** Decision: remove the unused connection pin and section-reorder
    routes, `set_pinned` and `reorder_sections`, and keep the columns. P07 updates `api.md`.
    The routes are mounted in `routes/mod.rs` only if they are otto-server routes. Check:
    `otto-connections/src/http.rs:153` owns its router, so you can drop them there.
15. **maintainability-2-14.** Move the six `// Tests` banners in `otto-product/src/chat.rs` so each
    sits directly above `#[cfg(test)] mod tests`.
16. **architecture-1-16.** Move the usage ingestion driver from `ottod/src/usage_tailer.rs` into
    `otto-usage`. Leave `usage_tailer.rs` as a thin call, because ottod's `main.rs` is unowned.
    Use one `otto_transcript::codex::model_of` for all three Codex model extractions.
17. **ui-correctness-2-02 (scanner part).** De-duplicate warnings in
    `otto-connections/src/conn_import.rs`, mirroring `dedupe(found)`. P10 owns the dialog key fix.

### P10: UI consistency (shared helpers, load errors, renderers) and the size and layering guards

- **Findings:** error-handling-2-02, error-handling-2-01, error-handling-2-07,
  ui-correctness-2-02, ui-correctness-2-04, ui-correctness-2-08, ui-correctness-2-09,
  ui-performance-2-01, ui-performance-2-03, ui-performance-2-04, ux-design-2-03, ux-design-2-05,
  ux-design-2-06, ux-design-2-08, accessibility-responsive-2-10, accessibility-responsive-1-03,
  maintainability-2-06, maintainability-2-07, maintainability-2-10, maintainability-2-11,
  architecture-1-11, architecture-1-12, architecture-2-07
- **Owned paths:**
  - `scripts/ui-guards.mjs`, `scripts/ui-guards-baseline.json`, `scripts/loc-ratchet.py`,
    `scripts/loc-baseline.json` and `scripts/test_loc_ratchet.py`
  - `docs/design/guidelines/**`
  - In `ui/src/lib/`: `html.ts` (new), `hl.ts`, `md.ts`, `render/**` (new), `metric-format.ts`,
    `loadError.ts`, `confirm.svelte.ts`, `components/ConfirmDialog.svelte` and `design/**` (new)
  - In `ui/src/lib/stores/`: `now`, `ui`, `git`, `transcript`, `k8s`, `aws`, `canvas`,
    `assistant`, `clipHistory`, `product` and `browserLive`, each `.svelte.ts`
  - Whole module directories under `ui/src/modules/`: `connections/**`, `mission-control/**`,
    `design-hall/**`, `brokers/**`, `vault/**`, `kubernetes/**`, `aws/**`, `api/**`, `mcp/**`,
    `browser/**`, `usage/**`, `insights/**` and `panels/**`
  - `ui/src/modules/canvas/**` except `canvas/types.ts` (P07), and `ui/src/modules/product/**`
    except `product/types.ts` (P07)
  - `ui/src/modules/rooms/room-media.ts`
  - `ui/src/modules/home/HomeToday.svelte`, `ui/src/modules/home/boxes/K8sBox.svelte` and
    `ui/src/modules/home/boxes/SessionsBox.svelte`
  - `ui/src/modules/settings/BrowserSettings.svelte`, `ui/src/modules/settings/Logs.svelte` and
    `ui/src/modules/settings/Channels.svelte`
  - `ui/src/modules/agents/NewSession.svelte`, `ui/src/modules/agents/conversation/codeBlocks.ts`
    and `ui/src/modules/agents/conversation/chatMarkdown.ts`
  - `ui/src/modules/swarm/RunInspector.svelte`

Never touch `ui/src/modules/rooms/games/**`, `ui/src/modules/home/school/**` or
`ClassroomsBox.svelte`.

**Instructions:**

1. **Create the shared helpers first.** Other packages consume them.
   - `lib/html.ts`: `export function escapeHtml(s: string): string`, escaping `& < > " '`.
   - `rel()` in `lib/stores/now.svelte.ts` gains an optional `{ compact?: boolean }` style.
   - `promptText` in `lib/confirm.svelte.ts` gains `validate?: (v: string) => string | null`, and
     `ConfirmDialog` shows the error inline.
2. **maintainability-2-11.** Use `escapeHtml` in all 12 files listed: `hl.ts`, `Logs`, `md.ts`,
   `mdRender`, `codeBlocks`, `chatMarkdown`, `OverviewTab` (`escapeHtmlAttr`), `design-hall/site/
   engine/render.ts`, `usage/reportHtml`, `noteRenderPlan`, `SearchPanel`, `InsightsPage` and
   `svgSanitize`. Keep any numeric-entity output where an attribute context needs it.
3. **error-handling-2-02.** Use `loadErrorText` in `git.svelte.ts:399` and `:440`,
   `transcript.svelte.ts:285` and `:310`, `k8s.svelte.ts` (keep the `CLUSTER_SCOPE_HINT` prefix),
   `aws.svelte.ts`, `vault.svelte.ts:1366` and `design-hall/library.svelte.ts:15`. P07 and P09 make
   their hunks.
4. **error-handling-2-01.** RewriteTab and TestCasesTab pollers copy PlanTab:
   `pollFailures` / `lastPollError`, an inline "Couldn't check progress: <cause>" with Retry, a
   timeout toast naming the cause, and `return false` so the poller backs off.
5. **error-handling-2-07.** In `design-hall/library.svelte.ts:220`, delete the memo key unless the
   error status is 403 or 404, and clear `tried` on the events-resync hook.
6. **ui-correctness-2-02.** `{#each warnings as w, i (i)}` in `ConnectionImportDialog.svelte:286`.
7. **ui-correctness-2-04.** In `MissionControlPage.svelte:234`, guard every commit with
   `id === ws.currentId`.
8. **ui-correctness-2-08.** `K8sBox` returns an aborted result that `freshness.run` ignores.
9. **ui-correctness-2-09.** In `canvas.svelte.ts:257`, capture the id and `#openSequence` before the
   await and assign only when both still match.
10. **ui-performance-2-01** (`room-media.ts:169`). Compute a graph signature and rebuild only when it
    changes. Call `schedule()` only when the peer set or generations change.
11. **ui-performance-2-03** (`BrowserSettings.svelte:26`). Add `&& !browserLive.loadError` to the
    guard.
12. **ui-performance-2-04** (`DesignArena.svelte:265`).
    - Effect (a) tracks only `product.selectedId`; everything else runs inside `untrack`.
    - Effect (b) keeps `lastIds` and skips its first run.
    - `loadChildren` gets a sequence token.
13. **ux-design-2-03.** Per-form `fieldErrors` rendered inline in `RequestBuilder`,
    `CredentialsPanel`, `PoliciesTab`, `TestCasesTab` and `Channels`. Update
    `docs/design/guidelines/components.md:100`. P06 makes the database dialogs.
14. **ux-design-2-05** (`TopicDetail.svelte:441`). After a non-tail consume, select the previously
    selected offset if it is still present, otherwise the first message, on desktop and tablet. A
    live-tail append leaves the selection alone.
15. **ux-design-2-06 and maintainability-2-07.**
    - Replace the local relative-time helpers with `rel()` in `HomeToday`, `SessionsBox`,
      `ConversationPanel`, `WorkItemDetail`, `mission-control/lib.ts relTime`, `LinkedCanvases`,
      `monitor-util fmtAgo` and `aws/util fmtAgo`.
    - Rename `RunInspector.rel` to `fmtWhen`.
    - P05, P06 and P08 make their hunks.
16. **ux-design-2-08.** Copy fixes in `DesignArena:778`, `AnalysisTab:855`, `NewSession:399` and
    `BrowserSettings:216`. P06 and P07 make their hunks.
17. **accessibility-responsive-2-10** (`vault/FileTree.svelte:361`). The checkbox gets
    `tabindex=-1` and `aria-label="Select {name}"`. Add a row key (⌘/Ctrl+Space) that calls
    `toggleSelect`, plus `aria-selected` and `aria-multiselectable`.
18. **accessibility-responsive-1-03.** Drop the opacity on `BrokersPage.svelte:838`.
19. **maintainability-2-06.**
    - `k8s-util`: use `formatBytes(n, {iec:true, empty:''})`.
    - `aws/util`: use `formatBytes`.
    - Point `storageGauge` and `scene3d/presets.ts` at the shared helper.
    - Delete the four local copies.
20. **maintainability-2-10.** Delete the dead members in the files you own (`ui.svelte.ts` layouts
    and their LS keys and clamps, `assistant`, `clipHistory`, `k8s` and `product` stores). Keep
    `wfSideWidth` as the legacy seed. P06 and P07 make their hunks.
21. **architecture-1-11.**
    - Create `lib/render/markdown.ts` and `lib/render/mermaid.ts`. Move `canvas/mermaid` and
      `vault/mdRender` there and leave re-exports.
    - Route the direct `marked` calls in `panels/FileTree`, `api/RequestBuilder` and
      `kubernetes/monitor/MonitorInsights` through `lib/md`.
    - P04 does WorkflowsPage, and P06 moves JsonTree.
22. **architecture-1-12.** Move `scene3d`, `DeviceFrame`, `DesignBoard` and the token grammar from
    `product/design/` to `lib/design/`, with re-exports at the old paths. Update the six
    design-hall importers to the new path. P07 breaks the types cycle.
23. **architecture-2-07.**
    - `loc-ratchet.py` counts production lines only: stop at the top-level `#[cfg(test)]` /
      `mod tests`. Update `test_loc_ratchet.py`.
    - Add per-function caps keyed by function name (searched across `crates/otto-server/src`, not
      by file, because P01 and P02 move them) for `execute_node`, `run_workflow_with_budget` and
      `run_review_core`, at today's sizes.
    - Add a file-LOC ratchet to `ui-guards.mjs` over `src/**/*.{svelte,ts}`, with
      `new_file_max: 800`.
    - Add a `lib`-imports-`modules` rule baselined at today's 34.
    - Add the raw-error rule (`e instanceof Error ? e.message : String(e)` under `lib/stores` and
      `modules`) and the `replace(/&/g, '&amp;')` rule outside `lib/html.ts`.
    - Baseline every rule against the current tree. The orchestrator re-baselines after all
      packages land.

## Deferred

| Finding | Reason |
|---|---|
| architecture-1-04 (generic N-agents + summarizer runner) | This is not a move. It is a new abstraction (agent-row state machine, per-agent CancelGuard, explicit Summarizer stage, persistence trait) across three engines whose summarizer semantics differ: name match, last row, and `SUM_RETRY_IDX`. That is several days of design and migration, and it should start from the `modules/review/*` layout P02 creates in this iteration. |
| architecture-1-05 (two MCP tool catalogs) | Needs a product decision. Generating native listings from `otto_mcp::outward::catalog` changes the schemas live agent sessions see for the 7 pairs in `DEFERRED_SCHEMA_DRIFT`, and the user's parity rule ("whatever is enabled in the control plane must be visible to sessions") means someone must choose the winning schema per pair. The parity test already contains the drift. |
| architecture-2-06 (rooms and game rooms out of otto-server into `otto-rooms`) | Another team is rewriting `game_rooms` this iteration (kart track and shortcut fixes, the 10x shooter, classroom rework). Moving roughly 9.7k lines into a new crate while they edit it would break their work. Schedule it right after their changes merge. |
| architecture-2-08 (one `game-catalog.json`) | The UI half lives in `ui/src/modules/rooms/games/**`, which another team owns and which this loop must not edit. It also depends on 2-06. |
