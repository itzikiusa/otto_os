# Quality iteration 3: fix plan

Ten work packages for ten parallel fixers in `/Users/itziklavon/claude_ade-quality`
(branch `fix/quality-loop-20261021`). Each package names its `owned_paths`, and owned paths are
disjoint across packages. Findings come from `docs/reviews/quality-20261021/iter-3/*.md` (carried
ones also cite `../iter-1/` and `../iter-2/`). Read the full finding before you fix it.

This iteration does the big splits that earlier iterations deferred. Each split gets a package that
exclusively owns the god file and its new sibling directory, and every other finding that touches
that file goes into the same package:

| Split | Package |
|---|---|
| `run_workflow_with_budget` in `workflow_engine.rs` (architecture-1-01) | P02 |
| `WorkflowsPage.svelte` (architecture-1-08) | P03 |
| `GraphView.svelte` (architecture-1-09) | P04 |
| `DatabaseStore` in `database.svelte.ts` (architecture-1-10), plus the JsonTree move (architecture-1-13) | P05 |
| Product/design-hall cycle (architecture-1-12), plus the issue client and `IssueEditor` (architecture-3-05) | P06 |
| `Terminal.svelte` grid sync (architecture-2-01) and `App.svelte` palette sources (architecture-2-05) | P07 |
| Usage tailer into otto-usage (architecture-1-16), plus the budget and transcript service layer (architecture-3-03) | P08 |
| Swarm `activate` lifecycle (architecture-3-01) and `KeyedLocks` (architecture-3-04) | P09 |
| MCP catalog generation (architecture-1-05), the api-client REST engine (architecture-1-07) and the otto-core `api.rs` split (architecture-3-06) | P10 |

## Rules for every package

1. **Ownership.**
   - Edit only your `owned_paths`.
   - Some findings also need a small edit in a file that another package owns. Those edits are
     listed under **Cross-package hunks**, and the file's owner makes them as part of its own
     package.
   - Never stash, reset, checkout or reformat files you don't own, because other fixers are editing
     the same worktree.
2. **Splits are behaviour-preserving moves.**
   - Put new files in a child directory of the file being split, so no crate root needs a new `mod`
     line. Only P08 may add a top-level module to `crates/otto-server/src/lib.rs` (`budget`,
     `transcript`).
   - Re-export everything that other code imports from the old path: `pub use` in Rust, and
     `export … from` or a thin wrapper in the UI. Other packages must keep compiling without
     changing their imports.
   - Never change behaviour in the same hunk as a move. Fix findings after the move, in the new
     location.
3. **Size limits.**
   - A new Rust file stays under 4,000 lines (loc-ratchet `new_file_max`).
   - A new `.svelte` or `.ts` file stays under 800 lines.
4. **Baselines belong to the integration step.**
   - No package edits `scripts/loc-baseline.json`, `ui/scripts/ui-guards-baseline.json` or
     `ui/scripts/bundle-budget.json`.
   - P07 is the only package that edits `ui/scripts/ui-guards.mjs`, to add the radio rule.
   - After all packages land, the orchestrator runs `python3 scripts/loc-ratchet.py
     --update-baseline` and `cd ui && node scripts/ui-guards.mjs --update-baseline` once.
   - If your only failure is a ratchet baseline, or a guard hit in a file you don't own, report it
     and don't fix it.
5. **Migrations** are append-only and must stay compatible with the previous build: nullable
   columns or columns with a DEFAULT only. The numbers are reserved below. Don't take another one.

   | Number | File | Package |
   |---|---|---|
   | 0189 | `0189_workflow_trigger_updated_by.sql` (`workflow_triggers.updated_by TEXT NULL`) | P02 |
   | 0190 | `0190_memory_prev_state.sql` (nullable `prev_state` on the memories table) | P02 |
   | 0191 | `0191_session_transcript_missing.sql` (nullable `transcript_missing_at` on `sessions`) | P08 |
6. **Tests.**
   - Only P01 edits `crates/otto-server/tests/it.rs`, to add the `workflow_run_files` suite.
   - Everyone else adds tests in-module (`#[cfg(test)]`) or in an existing suite file listed in
     their `owned_paths`.
   - UI unit tests that a package owns are listed by glob, such as `ui/unit/db*.test.ts`.
7. **Contracts.**
   - P01 owns `docs/contracts/**` and most of `docs/features/**`. P08 owns
     `connections-ssh-sftp.md` and P09 owns `agent-swarm.md`.
   - When another package changes a documented surface, the wire shape is **pinned in this plan**
     (see **Pinned contracts**), and P01 writes the doc rows from that text. Implement exactly the
     pinned shape. If you can't, stop and report; don't invent a different one.
8. **Machine load.** The user's rule is one heavy command at a time, because ten parallel builds once
   froze this Mac.
   - Never run `cargo build --workspace`, `cargo test --workspace` or `scripts/check.sh --all`.
   - Use crate-scoped commands only: `cargo check -p X`, `cargo test -p X --lib <filter>`,
     `cargo test -p otto-server --test it <suite>::<filter>`, and `cargo clippy -p X --all-targets
     -- -D warnings`.
   - Cargo serializes on the shared target lock, so don't add a second target dir.
   - Add no new external crates. Cargo rewrites `Cargo.lock` for path dependencies, which is
     expected.
   - In the UI, run `npm run check` and the relevant `node --test unit/<file>` once, at the end.
     Errors in files you don't own come from other fixers' work in progress: report them, don't fix
     them.
   - Kill any vite, ottod or ClickHouse process you start.
9. **Report faithfully.** For each finding, say fixed, partly fixed (and what remains) or not fixed.
   Include the exact commands you ran and their results.

## Pinned contracts

**PC-1. Run-scoped context files.** For access-control-3-04 and error-handling-3-04. P01
implements the route and client, and P03 consumes them.

- Routes:
  - `GET /api/v1/workflow-runs/{id}/context/browse?path=<rel>` returns `FsBrowse`, the same shape as
    `/fs/browse?files=true`. Entry `path`s are relative to the run's context dir. An empty or
    absent `path` means the root.
  - `GET /api/v1/workflow-runs/{id}/context/read?path=<rel>` returns `FsRead`, the same shape and
    400 KiB bound as `/fs/read`.
- Root dir: `data_dir/workflow-context/<run_id>`. Paths are joined with
  `otto_core::paths::confine_join`, and the canonical target must stay under the root.
- Auth: ws Viewer on `run.workspace_id`, and root passes. Share, MCP and agent-session tokens get
  403 (the same as `get_run`).
- Errors:
  - Not a member: 403.
  - Unknown run or missing file: 404.
  - Absolute path, `..`, or a symlink that escapes the root: 400.
- The `/fs/*` routes and their data-dir deny don't change.
- Client in `ui/src/lib/api/workflows.ts`. P01 adds it first, before any other work:
  - `browseRunContext(runId: string, path: string, signal?: AbortSignal): Promise<FsBrowse>`
  - `readRunContext(runId: string, path: string, signal?: AbortSignal): Promise<FsRead>`
- Final output is read with `readRunContext(id, 'final-output.md')`.

**PC-2. Usage tailer state.** For error-handling-3-05. P08 implements it, and P01 documents the
`/usage/status` row in `api.md` and the troubleshooting text in `usage-and-cost.md`.

- `UsageStatus` gains an optional `tailer` field:
  `tailer?: { state: 'running' | 'retrying' | 'stopped'; cause?: string | null }`.
- In Rust it is `#[serde(skip_serializing_if = "Option::is_none")] tailer: Option<TailerStatus>`,
  with `state` serialized in lowercase.
- `stopped` is used only for a corrupt checkpoint (a `restore_checkpoint` error).

**PC-3. Workflow run identity.** For access-control-3-02. P02 implements it, and P01 documents it in
the `api.md` workflow and trigger rows and in `workflows.md`.

- A manual run acts as its starter. This is unchanged.
- A trigger, schedule, webhook or chat run acts as whichever is later:
  - the `created_by` of the workflow's latest `workflow_versions` row;
  - the firing trigger's `updated_by` (new nullable column, migration 0189). Trigger create and
    spec changes set it to the caller.
- The run acts as the workflow's creator only when both are unset or are the creator.

**PC-4. Swarm turn identity.** For access-control-3-03. P09 implements it and documents
`agent-swarm.md`, and P01 updates the `api.md` swarm rows.

- Each turn's session is created as the task's human originator:
  - `task.created_by` when it is a user;
  - for agent-created subtasks, the parent task's originator;
  - for seeded tasks, the plan caller;
  - the swarm creator only when no human originated the work.
- Agent sessions are reused per (agent, acting user).
- `update_agent` changes to prompt, provider, model or cwd need the swarm creator, a ws Admin or
  root. Anyone else gets 403 "only the swarm's creator or a workspace admin can change an agent's
  role definition".
- Task CRUD, run, stop and board stay open to Editors.

**PC-5. MCP token rotate.** For access-control-3-01. P01 implements and documents it.

- `POST /mcp/tokens/{id}/rotate` requires the token's owner or root.
- Anyone else gets 403 "only root may rotate an MCP token owned by another user".

**PC-6. Bitbucket async merge.** For integrations-3-02. P01 implements and documents it.

- When Bitbucket answers 202, poll the same-origin `Location` task URL for up to about 60 s:
  - `SUCCESS` returns 204.
  - `FAILED` returns 409 with the merge-conflict message.
  - Still pending returns 409 "Bitbucket accepted the merge but hasn't finished it yet. Refresh
    the PR in a moment."
- The UI is not changed.

**PC-7. Issue-tracker client.** For architecture-3-05. P06 writes it first, before any other work,
and P04 consumes it.

- `ui/src/lib/api/issues.ts` exports:
  - `listIssueAccounts(signal?) → IssueAccount[]`
  - `listIssueProjects(accountId, { meta? }, signal?)`
  - plus the OverviewTab calls (`issueFull`, `transitions`, `setAssignee`, `editMeta`, `comment`,
    …).
- Every path segment is wrapped in `encodeURIComponent`.
- `ui/src/lib/stores/issueAccounts.svelte.ts` exports `issueAccounts` with `.list`, `.loading`,
  `.error` and `load(force?)`.

## Cross-package hunks

Each row names the finding and its owning package, then the package that makes the edit (because it
owns the file), and the change.

| Finding (owner) | Edit done by | File and change |
|---|---|---|
| architecture-3-05 (P06) | P04 | `git/FocusView.svelte` and `git/FindingActions.svelte`: replace hand-built `/issue/accounts` and `/issue/projects` with PC-7. Do this **last**. If `lib/api/issues.ts` is still missing then, leave it and report. |
| ux-design-3-06 (P06) | P04 | `git/GraphView.svelte:2218` `fmtDate`: use `rel(iso, { compact: true })` with `title={new Date(iso).toLocaleString()}`, and delete the local ladder. |
| ux-design-3-07 (P03) | P04 | `git/ProofPackView.svelte:74`: change "Could not …" to "Couldn’t …". |
| ux-design-3-07 (P03) | P05 | `kubernetes/actions.ts:93-96`: `typedConfirm` uses the shared mismatch wording, or `confirmProd({ typed })`. |
| error-handling-3-01 (P07) | P04 | Use `loadErrorText(e)` in `git/graph-commit-diff.svelte:187`, `git/GitPage.svelte:219` and `git/DiffViewer.svelte:860`. In `git/pullFlow.ts:44` and `git/pushFlow.ts:65`, keep `msg` for the classification and pass `loadErrorText(e)` as the toast body. |
| accessibility-responsive-3-05 (P07) | P05 | Add `tabindex={checked ? 0 : -1}` and `onkeydown={radioKey}` to the radios in `database/MongoFilterBar.svelte:94`, `database/DbAssistantPanel.svelte:161`, `kubernetes/ClusterWizard.svelte:275` and `aws/AccountWizard.svelte:288,306`. |
| accessibility-responsive-3-05 (P07) | P06 | Make the same radio change in `design-hall/NewDesignModal.svelte:109`, `design-hall/LinksPanel.svelte:273`, `design-hall/studio3d/StatesBar.svelte:124`, `lib/design/scene3d/Inspector.svelte:270,291` and `lib/design/scene3d/Scene3DViewport.svelte:1011`. |
| accessibility-responsive-3-04 (P07) | P06 | `product/OverviewTab.svelte:1341`: drop the wrapper's `role` and `aria-busy`, or pass `announce={false}` to Skeleton. Follow P07's choice: drop the wrapper role and pass `label`. |
| maintainability-3-02 (P07) | P05 | Delete `database.svelte.ts:831` `objectSearchSupported`, and `k8s.svelte.ts` `statusLoading`, `clustersLoading` and `monitorTickCluster`, with their writes. |
| maintainability-3-02 (P07) | P06 | Delete `product.svelte.ts` `loadingVersions` and `loadingAnalyses`, and `mockup-assist.svelte.ts:53` `lastResult`, together with its misleading doc. |
| maintainability-3-03 (P04) | P05 | `aws/AthenaView.svelte`: delete the write-only `$state` flag. |
| maintainability-3-08 (P06) | P05 | The JsonTree half, done together with architecture-1-13: point the five aws views and `workbench/preview/JsonPreview.svelte` at `$lib/components/JsonTree.svelte`, delete `modules/database/JsonTree.svelte`, and correct the "API client" comment. |
| maintainability-3-04 (P10) | P08 | `otto-state/src/sessions.rs:658`: delete `transcript_paths` if nothing uses it. Keep `transcript_paths_for_workspace`. |
| maintainability-3-04 (P10) | P04 | `otto-review/src/session.rs:938`: use `FINDINGS_POLL`, and make it private. |
| maintainability-3-12 (P10) | P02 | `otto-dbviewer/src/types.rs:1559`: delete `DbResult`. |
| maintainability-3-11 (P09) | P08 | Retarget the broken intra-doc links in `otto-sessions/src/lifecycle.rs` and `otto-usage/src/clickhouse.rs`. |
| architecture-3-03 (P08) | none | `routes::usage::{check_budget, BudgetVerdict, …}` stay as `pub(crate) use crate::budget::…` re-exports. The callers in `swarm_host.rs` and `product_host.rs` (P09), `workflow_engine/nodes/budget_gate.rs` (P02) and `modules/review/routes.rs` (P04) are not touched this round. |
| access-control-3-02 / 3-03, error-handling-3-05, backend-correctness-3-01 | P01 | `api.md` and feature-doc rows written from PC-2, PC-3 and PC-4, plus the review-retry outcomes in api-contracts-3-03: stop settles a `pending` row, and the boot sweep marks orphans "Interrupted by a daemon restart". |

## Packages

### P01: Access-control blockers, run-context routes and the API contract

- **Findings:** access-control-3-01 (blocker), data-integrity-3-05, access-control-3-04,
  access-control-3-05, integrations-3-02, api-contracts-3-01 … 3-11, maintainability-3-06,
  maintainability-3-09
- **Owned paths:**
  - `docs/contracts/**`, `AGENTS.md`, `docs/design/product-design-arena.md`
  - `docs/features/{daemon-http-api,rbac-multiuser-sharing,remote-mobile-access,agent-sessions,api-client,personal-agents,usage-and-cost,workflows,insights,skills-evaluator,aws-console,code-review,git}.md`
  - `crates/otto-rbac/**`
  - `crates/otto-server/src/routes/mod.rs`, `routes/mcp_cp.rs`, `routes/workflow_run_files.rs`
    (new), `routes/browser.rs` (comments only), `policy.rs`, `lsp/**`, `skill_review.rs` (comments
    only)
  - `crates/otto-git/src/providers/bitbucket.rs`, `crates/otto-git/src/providers/client.rs`
  - `crates/otto-aws/src/lib.rs`, `crates/otto-improve/src/http.rs`, `crates/otto-k8s/src/lib.rs`
    (comments only)
  - `crates/otto-server/tests/{it,route_inventory,router_mount,policy_coverage,auth_security,rbac_matrix,workflow_run_files}.rs`
  - `ui/src/lib/api/workflows.ts`, `ui/src/lib/api/types/workflows.ts`

**Instructions:**

1. **First, add the PC-1 client functions** to `ui/src/lib/api/workflows.ts`, because P03 builds
   on them.
2. **access-control-3-01 and data-integrity-3-05.**
   - Add `AuthRepo::mcp_token_owner(id)`.
   - `rotate_mcp_token` loads the owner first and applies PC-5.
   - Rewrite `rotate_mcp_token` in `tokens.rs` as one `begin()` transaction:
     `DELETE … WHERE id=? AND kind='mcp' RETURNING user_id,label,mcp_scope` is the guard, insert the
     new token only if a row was deleted, then commit.
   - Tests:
     - a non-root admin rotating root's token gets 403, and the old token still works;
     - rotating your own token returns 200;
     - root rotating any token returns 200;
     - a failed insert leaves the old token valid.
3. **access-control-3-04.** Implement PC-1 in `routes/workflow_run_files.rs`, mounted in
   `routes/mod.rs`, with a `policy.rs` entry and the route-inventory or policy-coverage updates.
   Add the `workflow_run_files` suite and its `#[path]` line in `it.rs`:
   - a non-root Viewer reads `final-output.md` → 200;
   - another workspace → 403/404;
   - `../x` → 400;
   - a symlink escape → 400.
4. **access-control-3-05.**
   - In `root_visible_to`, also accept roots under any repo registered in a workspace the user
     belongs to (`GitStore::list_repos`), canonicalized like the agent-cwd check.
   - Still apply `non_root_canonical_denied`.
   - Tests: a non-root Editor's out-of-root registered repo opens; an unrelated path gets 403.
5. **integrations-3-02.** Implement PC-6 in `bitbucket.rs`/`client.rs`. Add a wire test with a
   stub: 202 → PENDING → SUCCESS, and 202 → FAILED.
6. **API contracts.** Make every api-contracts-3-01 … 3-11 edit exactly as each report row says.
   - For 3-09, document `/ws/lsp` as it behaves after step 4.
   - Also document PC-1 … PC-6 and the review-retry outcomes (Cross-package hunks).
7. **maintainability-3-06.** Point every stale `modules.rs` reference at the child module. In
   docs, name the directory (`modules/review/`).
8. **maintainability-3-09.** Add the 3 missing crate rows and the 7 missing UI module names to
   `AGENTS.md`.
9. **Verify:**
   - `cargo test -p otto-rbac`
   - `cargo test -p otto-server --lib lsp`
   - `cargo test -p otto-server --test it workflow_run_files`
   - the `it` filters for `route_inventory`, `policy_coverage` and `auth_security`
   - `cargo test -p otto-git --lib bitbucket`
   - clippy on otto-rbac, otto-git and otto-server

### P02: Workflow-engine run split, workflow identity, data integrity and dbviewer backend

- **Findings:** architecture-1-01, access-control-3-02, data-integrity-3-01, data-integrity-3-04,
  data-integrity-3-07, data-integrity-3-02, data-integrity-3-08, backend-correctness-3-02,
  backend-correctness-3-04, test-quality-3-02, test-quality-3-04
- **Owned paths:**
  - `crates/otto-server/src/workflow_engine.rs`, `crates/otto-server/src/workflow_engine/**`,
    `crates/otto-server/src/routes/workflows.rs`,
    `crates/otto-server/src/workflow_trigger_scheduler.rs`
  - `crates/otto-workflows/**`
  - `crates/otto-state/src/{workflows,workflow_triggers,workflow_progress,memory,mcp_control,review_findings,settings,db_explorer,k8s_clusters,database_changes,database_changes_tests}.rs`
  - `crates/otto-state/migrations/0189_workflow_trigger_updated_by.sql`,
    `crates/otto-state/migrations/0190_memory_prev_state.sql`
  - `crates/otto-server/src/routes/settings.rs`, `crates/otto-server/src/memory_gov.rs`,
    `crates/otto-server/src/database_changes.rs`
  - `crates/otto-assistant/src/assistant/memory.rs`
  - `crates/otto-dbviewer/**`, `dev/dbviewer/**`, `.github/workflows/db-drivers.yml`

**Instructions:**

1. **architecture-1-01, the move only.**
   - Add a `RunState` struct holding the shared locals of `run_workflow_with_budget`
     (`workflow_engine.rs:1850`, about 1,250 lines).
   - Extract the admission and schedule phases into `workflow_engine/run/{admit,schedule}.rs`,
     beside the existing `finalize.rs`.
   - Keep every item reachable at its old path.
   - Run `cargo test -p otto-server --lib workflow` before and after.
2. **access-control-3-02.**
   - Implement PC-3: migration 0189, set `updated_by` in the trigger create and spec-change
     handlers, and change `acting_user_id`/`resolve_run_user` to take the latest-changer inputs.
   - Tests:
     - an Editor's graph edit plus a webhook run acts as that Editor and cannot reach the owner's
       private connection (use the `db_query` node's connection check);
     - an owner-only workflow still runs as the owner;
     - a ws Admin's edit runs as the Admin.
3. **data-integrity-3-01.**
   - Migration 0190 adds `prev_state`, and forget stores the current state in it.
   - Undo sets `state=COALESCE(prev_state,'accepted')` and `active=(prev_state<>'contradicted')`.
   - Test: pending, then forget, then undo stays `suggested`.
4. **data-integrity-3-04.** Put each guard in the SQL `CASE` as the finding describes, for
   `mcp_tools` risk labels and `review_findings` state.
5. **data-integrity-3-07.** Add `SettingsRepo::put_many`, which writes every key in one transaction
   and invalidates the cache after the commit. `PUT /settings` runs the reloads only after it
   succeeds.
6. **data-integrity-3-02 and data-integrity-3-08.**
   - Delete a dashboard's widgets and the dashboard in one transaction.
   - Reject a `dashboard_id` that is outside the widget's workspace on create and update. Checking
     it in `otto-dbviewer/src/http.rs` is fine.
   - Make the PATCHes single `UPDATE … COALESCE`/`CASE` statements, using `json_patch` for k8s
     `params_json`.
7. **backend-correctness-3-02.** Add UPDATE, DELETE, INSERT and MERGE to the `inject_row_limit`
   SKIP list. Tests: WITH…UPDATE and WITH…DELETE give `limited:false`.
8. **backend-correctness-3-04.**
   - Add `PlaceholderMode::Postgres`: no `#` comments, mask `$tag$…$tag$` by reusing the
     `split.rs` scanner, and double `''` only in standard strings.
   - Tests for `#>>` and `$$`.
9. **test-quality-3-02.**
   - Add the non-ignored executor-gate test from the report, built on `Fixture::new(kind, 1)` and
     covering all 6 refusals.
   - Seed `otto_reader` plus the `shop` and `hidden` schemas in `dev/dbviewer`.
   - Drop `-E 'not binary(resource_access)'` in `db-drivers.yml`.
10. **test-quality-3-04.** Add the restricted-reader tests for `schema_root`, `search_objects` and
    `schema_graph` in `perf_budget_tests.rs`.
11. **Cross-package hunk:** delete `DbResult` (maintainability-3-12).
12. **Verify:**
    - `cargo test -p otto-state`
    - `cargo test -p otto-dbviewer --lib`
    - `cargo test -p otto-workflows`
    - `cargo test -p otto-server --lib workflow`, plus `memory_gov` and `settings`
    - clippy on otto-state, otto-dbviewer, otto-workflows and otto-server
    - `crates/otto-state/tests/migration_compat.rs`

### P03: WorkflowsPage split and its findings, plus Rooms, Home and assistant fixes

- **Findings:** architecture-1-08, error-handling-3-04, ui-correctness-3-05, ux-design-3-02,
  error-handling-3-08, accessibility-responsive-3-07, accessibility-responsive-3-01,
  accessibility-responsive-3-03, accessibility-responsive-3-12, accessibility-responsive-3-11,
  ux-design-3-07, data-integrity-3-03
- **Owned paths:**
  - `ui/src/modules/workflows/**`, `ui/src/modules/panels/FileTree.svelte`,
    `ui/unit/workflow*.test.ts`
  - `ui/src/modules/home/HomePage.svelte`,
    `ui/src/modules/home/boxes/{K8sBox,UsageBox,DbDashboardBox}.svelte`. Never touch
    `home/school/**` or `ClassroomsBox.svelte`.
  - `ui/src/modules/rooms/{RoomChat,RoomParticipants,RoomPage,RoomLobby,RoomRecap,RecapPanel,RoomSettingsModal,StartRoomModal}.svelte`.
    Never touch `rooms/games/**`.
  - `ui/src/modules/settings/Login.svelte`, `ui/src/lib/confirmProd.ts`,
    `ui/src/modules/agents/conversation/LiveStatus.svelte`
  - `crates/otto-server/src/routes/assistant.rs`, `crates/otto-assistant/src/assistant/tasks.rs`,
    `crates/otto-state/src/assistant.rs`

**Instructions:**

1. **architecture-1-08, the move only.**
   - Move the inspector forms to `workflows/inspector/<kind>.svelte`, with shared helpers in
     `workflows/inspector/util.ts`.
   - Update `ui/unit/workflow*.test.ts` to import functions from modules, not by name from the page.
   - `WorkflowsPage.svelte` (5,156 lines) must shrink substantially. Each new file stays under 800
     lines.
2. **error-handling-3-04.**
   - Final output uses `readRunContext` (PC-1).
   - `loadFinalOutput` hides silently only on 404. Any other error shows an inline `loadErrorText`
     with Retry and resets `finalOutputRunId`.
   - Give `FileTree` optional `lister`/`reader` props, defaulting to `/fs/*`. The Context panel
     passes `browseRunContext`/`readRunContext`.
3. **ui-correctness-3-05.** Render the final output with `renderMarkdownGfm(md, { breaks: true })`.
4. **ux-design-3-02.**
   - Use pickers for DB connection (name, engine, env), broker cluster and then topic, product story
     (key and title) and Jira account, following the swarm-picker pattern.
   - Keep an "Unknown … (id)" option and the text-input fallback for when loading fails.
5. **error-handling-3-08.** `RunAgents`: forget the failed lookup unless the error is an `ApiError`
   with status 404.
6. **accessibility-responsive-3-07.** On a keyboard Delete, focus a neighbour after `tick()` and
   announce "Deleted {label}" in a polite sr-only region.
7. **accessibility-responsive-3-01.** Add `onkeydown={radioKey}` to the K8sBox and UsageBox radios.
   `HomePage` `onKey` skips targets inside `[role=radiogroup]` and `[role=tablist]`.
8. **accessibility-responsive-3-03.**
   - RoomChat keeps one always-mounted submit button whose label switches between Send and Retry.
     It focuses the textarea after delivery, and Cmd/Ctrl+Enter sends.
   - After admit or reject, RoomParticipants focuses the member's new control or the People
     heading.
9. **accessibility-responsive-3-12.** Use `minmax(min(380px, 100%), 1fr)`.
10. **accessibility-responsive-3-11.** Remove `role=status` from LiveStatus's working, stale and
    waiting blocks.
11. **ux-design-3-07.**
    - Change "Could not …" to "Couldn’t …" in RoomPage, RoomLobby, RoomRecap, RecapPanel,
      RoomSettingsModal, StartRoomModal and Login.
    - `confirmProd.ts` gets one shared typed-mismatch wording. P04 and P05 do the other sites.
12. **data-integrity-3-03.**
    - `delete_thread` returns the attachment paths from inside its transaction. Remove those files
      after the commit, confined to the user's inbox.
    - Add a sweep for inbox files that have no `assistant_attachments` row and are older than a
      grace period.
    - Test both.
13. **Verify:**
    - `npm run check`
    - `node --test unit/workflow*.test.ts`
    - `cargo test -p otto-assistant` and `cargo test -p otto-server --lib assistant`
    - clippy on both crates
    - an e2e smoke of the workflows page if time allows: `npm run test:e2e -- workflows`, with an
      isolated slot

### P04: GraphView split, git UI, review backend and brokers

- **Findings:** architecture-1-09, maintainability-3-01, error-handling-3-09,
  accessibility-responsive-1-03, maintainability-3-03, backend-correctness-3-01,
  backend-performance-3-01, concurrency-async-3-01, data-integrity-3-06, concurrency-async-3-05,
  test-quality-3-03
- **Owned paths:**
  - `ui/src/modules/git/**`, `ui/src/lib/stores/git.svelte.ts`, `ui/unit/git*.test.ts`
  - `ui/src/modules/agents/AttachIssue.svelte`, `ui/src/modules/agents/ShareModal.svelte`
  - `crates/otto-review/**`, `crates/otto-server/src/modules/review/**`,
    `crates/otto-server/src/modules/review.rs`, `crates/otto-server/src/boot/recovery.rs`
  - `crates/otto-state/src/reviews.rs`
  - `crates/otto-server/tests/{review_agent_retry,review_comment_states}.rs`
  - `crates/otto-brokers/**`

**Instructions:**

1. **architecture-1-09, the move only.**
   - Extract `git/RefSidebar.svelte` with an explicit props and callbacks interface.
   - Move the action handlers into a `git/graphActions.ts` controller.
   - `GraphView.svelte` (4,878 lines) must shrink substantially. Each new file stays under 800
     lines.
2. **maintainability-3-01.**
   - Delete `prs`, `prsLoading`, `prError`, `prsGen` and `loadPrs`, the two calls, and the resets
     at `:386` and `:395`.
   - Rewrite the unit test to assert that stale `primaryStatus` responses are dropped.
3. **error-handling-3-09.** `pullMode` stays `null` until it is known, and the button reads plain
   "Pull" meanwhile. Reset `pullModeFor = ''` in the catch.
4. **accessibility-responsive-1-03.** Drop the opacity on the paused auto-fetch toggle.
5. **maintainability-3-03.**
   - AttachIssue: add `if (attaching) return;` at the top of `attach`.
   - Delete the write-only flags in ShareModal and ReviewPanel.
   - P05 does the one in AthenaView.
6. **backend-correctness-3-01.**
   - Run every fallible precondition (prompt, root user, diff re-materialisation) before
     `claim_agent_retry`. Otherwise, restore the settled row with `set_agent_at` on any early
     return.
   - Add a boot sweep in `boot/recovery.rs` that turns pending, running and waiting agents of
     non-running reviews into error "Interrupted by a daemon restart".
   - Let stop settle a `pending` row that has no cancel flag.
   - Extend `review_agent_retry.rs`.
7. **backend-performance-3-01.**
   - Add `review_status(review_id)` to `ReviewsRepo` if it is missing.
   - `review_is_cancelled` uses it.
   - The summarizer's cancel future waits on ReviewChanged or the flag, with a ~5 s status recheck.
   - The draft loop checks the in-memory flag per comment and the durable status once per batch.
8. **concurrency-async-3-01.**
   - `submit_prompt` takes `manager.submit_lock(sid)` after `wait_for_tui_strict`, and holds it
     through paste, echo wait, Enter and re-Enter.
   - Add a regression test with concurrent `submit_prompt` and `submit_text` calls.
9. **data-integrity-3-06.** Append only the missing lines to `.git/info/exclude`, using
   `create(true).append(true)`.
10. **concurrency-async-3-05.** Clamp `max_wait_ms` to `30_000` in `consume_with`.
11. **test-quality-3-03.**
    - Add guard tests in `otto-brokers/src/http.rs` mod tests, with a cluster at `127.0.0.1:1`:
      - prod and read-only clusters: all 6 mutating routes return 403 without confirm;
      - reset with `dry_run` passes the guard;
      - a dev cluster passes the guard;
      - global-cluster Database grant: without the edit grant, DELETE returns 403; with it, DELETE
        is not refused.
12. **Cross-package hunks:**
    - FocusView and FindingActions use PC-7 (do this last);
    - GraphView `fmtDate` uses `rel()`;
    - ProofPackView copy;
    - the `loadErrorText` sites in graph-commit-diff, GitPage, DiffViewer, pullFlow and pushFlow;
    - `FINDINGS_POLL` in `otto-review/src/session.rs`.
13. **Verify:**
    - `npm run check` and `node --test unit/git*.test.ts`
    - `cargo test -p otto-review`, `cargo test -p otto-brokers --lib` and `cargo test -p
      otto-state --lib reviews`
    - `cargo test -p otto-server --test it review_agent_retry`
    - clippy on each crate

### P05: DatabaseStore split, the JsonTree move, and database, AWS and Kubernetes UI fixes

- **Findings:** architecture-1-10, architecture-1-13 (JsonTree part), ui-correctness-3-01,
  ui-correctness-3-04, ux-design-3-01, accessibility-responsive-3-02, ui-performance-3-02,
  ui-correctness-3-03, ui-performance-3-01, accessibility-responsive-3-10
- **Owned paths:**
  - `ui/src/lib/stores/database.svelte.ts`, `ui/src/lib/stores/database/**` (new),
    `ui/src/modules/database/**`
  - `ui/src/lib/components/JsonTree.svelte`, `ui/src/lib/jsonTree/**` (new),
    `ui/src/modules/workbench/preview/JsonPreview.svelte`
  - `ui/src/modules/aws/**`, `ui/src/lib/stores/aws.svelte.ts`
  - `ui/src/modules/kubernetes/**`, `ui/src/lib/stores/k8s.svelte.ts`,
    `crates/otto-k8s/src/resources.rs`
  - `ui/src/modules/connections/ConnectionForm.svelte`
  - `ui/unit/db*.test.ts`, `ui/unit/database*.test.ts`, `ui/unit/k8s*.test.ts`,
    `ui/unit/runesHarness.ts`, `ui/unit/sourceHarness.ts`, `ui/unit/strictRequire.ts`

**Instructions:**

1. **architecture-1-10.**
   - First make the unit harness resolve relative imports. The change must be backward compatible,
     because every other package's tests use the harness.
   - Then split `DatabaseStore` (4,415 lines) per concern into
     `lib/stores/database/{connections,tabs,results,filters,savedQueries}.svelte.ts`.
   - `database.svelte.ts` stays the façade, and the `db` export and its methods keep their names.
   - Move only. Run `node --test unit/db*.test.ts unit/database*.test.ts` before and after.
2. **architecture-1-13, the JsonTree part.**
   - Move `bson.ts`, `expansion-plan.ts` and `json-find.ts` from `modules/database/` to
     `lib/jsonTree/`, and update the database importers and unit tests.
   - Point the five aws views and `JsonPreview` at `$lib/components/JsonTree.svelte`, delete
     `modules/database/JsonTree.svelte`, and correct the comment. This is also the maintainability-3-08
     JsonTree half.
3. **ui-correctness-3-01.**
   - De-duplicate refs by `refKey` in `suggestGroupBy` and in "Group by them", using the same guard
     as `addGroupBy`.
   - Key the chips and the issues by index.
   - Add a unit test for selecting a column twice and then adding an aggregate.
4. **ui-correctness-3-04.** On a successful edit review that came from `reviewPending`, clear the
   covered drafts before refreshing. Delete and duplicate reviews leave pending drafts alone.
5. **ux-design-3-01.** `runManagedStatement` returns a cancel reason (`'declined' | 'mismatch' |
   'stale'`). EditFlow toasts "Write canceled" only for `'declined'`.
6. **accessibility-responsive-3-02.**
   - Add `aria-level`, `aria-setsize` and `aria-posinset` to every treeitem.
   - `.node-label` gets `tabindex=-1`.
   - Find the current item with `activeElement.closest('[role=treeitem]')`.
   - Move the search, scope and count controls out of `role=tree`.
7. **ui-performance-3-02.**
   - Auto-refresh awaits the attribute fan-out with `load({ awaitAttrs: true })`.
   - Collect the results and assign `sqsAttrs` once per fan-out.
8. **ui-correctness-3-03.** In `otto-k8s` `images()`, use an order-preserving de-duplication.
   `ResourceDrawer` keys the image loop by index. Add a Rust unit test.
9. **ui-performance-3-01.** In `MonitorCluster`, read `win` untracked in `onMove`, and key the
   re-measure on the slice bounds (see the finding's snippet).
10. **accessibility-responsive-3-10.** Use one always-mounted `role=status` line for Testing…,
    Connected and "Test failed: …". The detail panel is not a live region.
11. **Cross-package hunks:**
    - the radios in MongoFilterBar, DbAssistantPanel, ClusterWizard and AccountWizard;
    - the dead fields in `database.svelte.ts` and `k8s.svelte.ts`;
    - the AthenaView flag;
    - `kubernetes/actions.ts` `typedConfirm` wording.
12. **Verify:**
    - `npm run check`
    - `node --test` on the db, database and k8s unit tests
    - `cargo test -p otto-k8s --lib resources`
    - `cargo clippy -p otto-k8s --all-targets -- -D warnings`

### P06: Product and design-hall decoupling, the issue client, the Atlassian transport, and vault

- **Findings:** architecture-3-05, architecture-1-12, maintainability-3-08, maintainability-3-14,
  ux-design-3-06, error-handling-3-06, ui-correctness-3-02, ui-correctness-3-07,
  architecture-3-02, backend-correctness-3-05
- **Owned paths:**
  - `ui/src/modules/product/**`, `ui/src/modules/design-hall/**`, `ui/src/lib/design/**`
  - `ui/src/modules/canvas/excalidraw-build.ts`, `ui/src/lib/excalidraw/**` (new)
  - `ui/src/lib/api/issues.ts` (new), `ui/src/lib/stores/issueAccounts.svelte.ts` (new),
    `ui/src/lib/html.ts`, `ui/src/lib/status.ts`
  - `ui/src/lib/stores/product.svelte.ts`, `ui/src/lib/stores/mockup-assist.svelte.ts`
  - `ui/src/modules/agents/JiraIssuePicker.svelte`, `ui/src/modules/settings/IssueAccounts.svelte`
  - `ui/unit/product*.test.ts`, `ui/unit/design*.test.ts`, `ui/unit/studio3d.test.ts`
  - `crates/otto-design/**`, `crates/otto-issues/**`, `crates/otto-vault/**`

**Instructions:**

1. **First, write PC-7.** Create `lib/api/issues.ts` and the `issueAccounts` store. Then migrate
   OverviewTab, PublishDialog, SourceSearch, ImportDialog, JiraIssuePicker and IssueAccounts to it.
   Extract `product/jira/IssueEditor.svelte` from OverviewTab, covering `beginEdit` through
   `saveField`, transitions and assignee. This is architecture-3-05.
2. **architecture-1-12.**
   - Move `canvas/excalidraw-build.ts` to `lib/excalidraw/` and leave a re-export at the old path.
   - Then move `DesignBoard`, templates and `format` from `product/design/` to `lib/design/`, and
     leave re-exports.
   - Design-hall must import nothing from `modules/product`. Check with `rg "modules/product|\.\./product/"
     ui/src/modules/design-hall`.
3. **maintainability-3-08, the scene3d half.** Point the importers (`MockupLivePreview`,
   `design/format.ts`, `DesignArena` and `unit/studio3d.test.ts`) at `lib/design/scene3d/`, and
   delete the 11 shim files. P05 does the JsonTree half.
4. **maintainability-3-14.** Add `unescapeHtml` to `lib/html.ts` and use it at the three sites.
   Inline `escapeHtml` in place of the alias.
5. **ux-design-3-06.**
   - Replace the ladders in OverviewTab, DiscoveryTab, ChatTab, RefineTab and DesignArena with
     `rel(iso)` (compact on dense rows) and add an absolute `title`.
   - `lib/status.ts` `agoLabel` and `suspendedNote` use `rel()`.
   - P04 does GraphView.
6. **error-handling-3-06.**
   - Store `blenderProbeError = loadErrorText(e)` and show "Couldn’t check for Blender: <cause>"
     with a Retry that resets `blenderChecked`.
   - Show the install hint only when a successful probe returns `installed: false`.
7. **ui-correctness-3-02.**
   - Server side, in `otto-design/src/store.rs`: `group_concat(DISTINCT l.dst_id)` plus `AND
     l.rel='implements'`, and keep `story_ids_for` consistent. Add a Rust test.
   - UI: de-duplicate in `storyKeys` and `storyKeysOf`, and key both chip loops by index.
8. **ui-correctness-3-07.** Merge into the live map: `this.stories = { ...this.stories, ...next }`.
   Drop `|| this.loaded`.
9. **architecture-3-02.**
   - Add `otto-issues/src/atlassian.rs` with `AtlassianTransport { base, auth_header, http,
     transport_err }`. It owns a bounded client cache, the TLS check enforced in its single
     `send()`, and the status-to-`Error::Upstream` mapping.
   - `JiraClient` and `ConfluenceClient` wrap it. Linear reuses the cache helper.
   - Test: `JiraClient` refuses `http://example.com`.
10. **backend-correctness-3-05.**
    - Return the original fence byte spans from `split_frontmatter` (or a sibling function), and
      have `rewrite_links` copy them verbatim.
    - Extend `rewrite_noop_is_bytes_identical` with a CRLF case and a `...` case.
11. **Cross-package hunks:**
    - the design-hall and `lib/design/scene3d` radios;
    - the OverviewTab Skeleton wrapper;
    - the dead fields in `product.svelte.ts` and `mockup-assist.svelte.ts`.
12. **Verify:**
    - `npm run check` and `node --test` on the product, design and studio3d unit tests
    - `cargo test -p otto-issues`, `cargo test -p otto-design` and `cargo test -p otto-vault`
    - clippy on each

### P07: Terminal and App shell splits, the workspace store, and the palette and a11y sweep

- **Findings:** architecture-2-01, architecture-2-05, architecture-2-03, ui-correctness-3-06,
  error-handling-3-01, error-handling-3-03, ux-design-3-03, accessibility-responsive-3-08,
  accessibility-responsive-3-09, accessibility-responsive-3-04, accessibility-responsive-3-05,
  accessibility-responsive-3-06, maintainability-3-02, maintainability-3-13
- **Owned paths:**
  - `ui/src/shell/**`, `ui/src/lib/stores/workspace.svelte.ts`
  - `ui/src/lib/components/Terminal.svelte`, `ui/src/lib/components/termGridSync.ts` (new)
  - `ui/src/lib/stores/workflowRuns.svelte.ts`, `ui/src/lib/uiCommands/**`
  - `ui/src/lib/stores/apiClient.svelte.ts`, `ui/src/modules/api/**`
  - `ui/src/modules/connections/paletteCommands.ts` (new)
  - `ui/src/modules/agents/history/**`, `ui/src/modules/agents/AgentsPage.svelte`,
    `ui/src/modules/agents/NewSession.svelte`
  - `ui/src/lib/components/FloatingBar.svelte`, `ui/src/lib/components/Skeleton.svelte`
  - `ui/src/modules/assistant/**`, `ui/src/modules/personal-agents/**`,
    `ui/src/modules/swarm/SwarmPage.svelte`,
    `ui/src/modules/mission-control/MissionControlPage.svelte`
  - `ui/src/modules/settings/{ActiveShareLinks,AllowedIdsEditor,Appearance}.svelte`
  - `ui/src/lib/events.svelte.ts`, `ui/src/lib/stores/serviceHealth.svelte.ts`,
    `ui/src/lib/stores/transcript.svelte.ts`
  - `ui/src/lib/cssColor.ts`, `ui/src/lib/fuzzy.ts`, `ui/src/lib/render/mermaid.ts`
  - `ui/scripts/ui-guards.mjs`
  - `ui/unit/term*.test.ts`, `ui/unit/workspace*.test.ts`, `ui/unit/palette*.test.ts`,
    `ui/unit/api*.test.ts`

**Instructions:**

1. **architecture-2-01, the move only.** Move the resize and claim negotiation out of
   `Terminal.svelte` into a `termGridSync.ts` controller, following the `termFlow`/`termPark`
   pattern.
2. **architecture-2-05.**
   - Add `shell/paletteSources.svelte.ts`.
   - Connection commands go to `modules/connections/paletteCommands.ts`.
   - Repo commands go to `shell/paletteSources/repos.ts`, because `modules/git` belongs to P04 this
     round.
   - Add one `installShell()` called from `onMount`, which takes over App's feature effects.
3. **architecture-2-03.** Move the three api-client settings getters from the workspace store to
   the `apiClient` store, and update `modules/api` to read them there.
4. **ui-correctness-3-06.**
   - Call `workflowRuns.clear(id)` in `selectApproved` when the workspace id changes.
   - `refresh` drops a response for a workspace that is no longer its target.
5. **error-handling-3-01.** Use `loadErrorText(e)` at `workspace.svelte.ts:986`, `history.svelte.ts:164`
   and `AgentsPage.svelte:63`. P04 does the git sites.
6. **error-handling-3-03.** Track whether every pinned chunk and the scratch request succeeded.
   On a partial failure, skip `reconcileTabs()`, or prune only ids that a successful `?ids=`
   response positively omitted.
7. **ux-design-3-03.** Navigator bulk delete reuses the Palette's confirm builder. Bulk archive
   confirms when any selected session is working, or gives the batch toast an Undo.
8. **accessibility-responsive-3-08.**
   - Search hits use `role=group` with an `aria-label`.
   - The empty, searching and error text, including Retry, moves to an always-mounted `role=status`
     line under the input.
9. **accessibility-responsive-3-09.**
   - When a plan arrives, focus the plan heading (`tabindex=-1`) or announce "Plan with N steps
     ready".
   - Render the steps as an `<ol>`.
   - Cancel refocuses the textarea.
10. **accessibility-responsive-3-04.**
    - Remove `aria-busy` from Skeleton's status element.
    - At the wrapper sites you own (assistant, personal-agents `AgentPage`, `SwarmPage`,
      `MissionControlPage`), drop the wrapper's `role`, `aria-busy` and `aria-label`, and pass
      `label` to Skeleton.
    - P06 does the OverviewTab site.
11. **accessibility-responsive-3-05.**
    - Fix the radios in `FloatingBar`, `NewSession` (the Workspace group) and `Appearance`.
    - Add a `ui-guards` rule that flags `role="radio"` without `radioKey`.
    - P05 and P06 fix the rest. The baseline is refreshed at integration.
12. **accessibility-responsive-3-06.**
    - After a removal, focus the same index's remove button, else the previous one, else Add.
    - Name each remove button after its item.
    - After Add, focus the new input.
    - This applies to `AgentAutonomy`, `ActiveShareLinks` and `AllowedIdsEditor`.
13. **maintainability-3-02.** Delete `events.svelte.ts` `lastKind`, `lastId` and `findingId`,
    `transcript.svelte.ts` `lastAppendAt`, and `serviceHealth.svelte.ts` `lastDownAt`, with their
    misleading docs. P05 and P06 delete the rest.
14. **maintainability-3-13.**
    - Delete `fuzzy.ts` and `parseMermaid`.
    - Delete `cssColor.ts` only if nothing references it, `home/school/**` included (that is
      another team's code; don't edit it). Otherwise keep it and fix the test reference.
15. **Verify:**
    - `npm run check`
    - `node --test` on the term, workspace, palette and api unit tests
    - a terminal e2e spec if time allows, with an isolated slot

### P08: Usage tailer move, the budget and transcript service layer, sessions and automation

- **Findings:** error-handling-3-05, architecture-1-16, architecture-3-03,
  backend-performance-3-04, backend-performance-3-05, ux-design-3-05, maintainability-3-07,
  backend-correctness-2-01, backend-performance-3-02, integrations-3-03, test-quality-3-05,
  error-handling-3-02, test-quality-3-06, maintainability-3-10
- **Owned paths:**
  - `crates/ottod/src/usage_tailer.rs`, `crates/ottod/src/main.rs`, `crates/ottod/Cargo.toml`
  - `crates/otto-usage/**`, `crates/otto-transcript/**`
  - `crates/otto-server/src/lib.rs`, `crates/otto-server/src/budget.rs` (new),
    `crates/otto-server/src/transcript/**` (new)
  - `crates/otto-server/src/{monitor,transcript_tail,transcript_cache,resource_sessions}.rs`
  - `crates/otto-server/src/routes/{usage,transcript,history_page,mission,goal_loops,mcp_servers}.rs`
  - `crates/otto-sessions/**`
  - `crates/otto-automation/src/goal_loop.rs`,
    `crates/otto-automation/src/scheduled_tasks_engine.rs`
  - `crates/otto-state/src/{sessions,history_page,scheduled_tasks}.rs`,
    `crates/otto-state/migrations/0191_session_transcript_missing.sql`
  - `crates/otto-server/tests/admin_sessions.rs`
  - `docs/features/connections-ssh-sftp.md`
  - `ui/src/modules/usage/**`, `ui/src/lib/api/usage.svelte.ts`, `ui/src/lib/metric-format.ts`,
    `ui/src/lib/sessionUsage.ts`
  - `ui/src/modules/mission-control/lib.ts`, `ui/src/modules/agents/conversation/format.ts`,
    `ui/src/modules/swarm/RunInspector.svelte`
  - `ui/unit/usage*.test.ts`

**Instructions:**

1. **architecture-1-16, the move only.**
   - Move the tailer driver from `ottod/src/usage_tailer.rs` into `otto-usage` behind a small ctx
     trait. `ottod` keeps a thin wiring call.
   - Keep one Codex model extractor in `otto-transcript`, and delete the other two copies.
2. **error-handling-3-05.**
   - Stop only on a `restore_checkpoint` error.
   - Retry an unavailable journal or a failed commit with a capped backoff.
   - Expose PC-2 on `/usage/status`. The Usage page shows an inline warning for `retrying` and
     `stopped`.
3. **architecture-3-03.**
   - Add `budget.rs` with `BudgetGate::check(ctx, ws, provider) -> Result<(), BudgetBlocked>`
     and one `Display` for the refusal.
   - Add `transcript/resolve.rs` for the transcript helpers.
   - `routes/usage.rs` keeps the GET/PUT handlers and `pub(crate) use` re-exports.
   - Delete the `*_pub` shims and update their callers (`monitor.rs`, `routes/mission.rs`).
4. **backend-performance-3-04.** Migration 0191. Record `transcript_missing_at` when a root scan
   returns `TranscriptMissing`, skip resolution for marked rows within a TTL, and clear the mark on
   the next successful capture.
5. **backend-performance-3-05.** Add `transcript_tail::live_artifacts(...)`. `session_artifacts`
   tries it first and falls back to `cached_fold`.
6. **ux-design-3-05.**
   - Use the shell's wording.
   - Retry re-invokes the import with a fresh `import()` promise held in state.
   - `errorActions` includes "Reload Otto", plus an optional "Back to overview".
7. **maintainability-3-07.** Add `formatUsd(n, { empty })` to `metric-format.ts`, and use it at
   all 8 sites. Use `formatCount` for tokens, and delete the local copies.
8. **backend-correctness-2-01.**
   - Compute the cutoff as `i64::try_from(days).ok().and_then(TimeDelta::try_days).and_then(|d|
     Utc::now().checked_sub_signed(d))`. `None` means off.
   - Tests with `u64::MAX` and `99_999_999_999`.
9. **backend-performance-3-02.**
   - Replace the per-session loop with one query over `json_extract(meta_json,'$.browser')=1`, and
     canonicalize only the matches.
   - Run the reconcile body in one `spawn_blocking`.
10. **integrations-3-03.**
    - Reconcile deletes the launcher env files of servers that are gone or have no env.
    - Sweep them on server delete or disable.
    - Fix the doc comment and `connections-ssh-sftp.md:448`.
    - Test: reconcile with, then without, a server, and assert its env file is gone.
11. **maintainability-3-10.** Rewrite the "Pinned" section of `connections-ssh-sftp.md:216`.
12. **test-quality-3-05.**
    - Add memo namespace tests in `resource_sessions.rs`, in both input orders.
    - Add an HTTP case in `admin_sessions.rs`: a ws Admin without the shell grant doesn't see the
      bound row.
13. **error-handling-3-02.**
    - `finalize` and `block` return `Result`, with a settle-style retry, and emit only the stored
      status plus a Notice on a persistent failure.
    - `stop_loop` propagates the error, and the route returns a Problem.
    - Warn on every discarded per-iteration write.
14. **test-quality-3-06.** Add the two paused-time trigger tests from the finding. Make
    `settle_with_retry` `pub(crate)` if needed.
15. **Cross-package hunks:** delete `transcript_paths`, and fix the `lifecycle.rs` and
    `clickhouse.rs` doc links.
16. **Verify:**
    - `cargo test -p otto-usage`, `-p otto-transcript`, `-p otto-sessions` and `-p otto-automation`
    - `cargo test -p otto-server --lib` with the `usage`, `transcript`, `history` and
      `resource_sessions` filters
    - `cargo test -p otto-server --test it admin_sessions`
    - `cargo check -p ottod`
    - clippy on each crate
    - `npm run check` and the usage unit tests

### P09: Swarm activation lifecycle and identity, `KeyedLocks`, and git and run-service backend

- **Findings:** architecture-3-01, access-control-3-03, error-handling-3-07, test-quality-3-01,
  architecture-3-04, maintainability-3-11, concurrency-async-3-02, maintainability-3-15,
  maintainability-3-05, concurrency-async-3-03, concurrency-async-3-04
- **Owned paths:**
  - `crates/otto-swarm/**`, `crates/otto-product/**`
  - `crates/otto-server/src/swarm_host.rs`, `crates/otto-server/src/product_host.rs`
  - `crates/otto-state/src/swarm.rs`, `crates/otto-state/src/swarm/**`
  - `crates/otto-server/tests/{swarm_router,swarm_scope_api}.rs`, `docs/features/agent-swarm.md`
  - `crates/otto-core/src/lib.rs`, `crates/otto-core/src/keyed_lock.rs` (new)
  - `crates/otto-git/src/{http,local,diff_cache}.rs`, `crates/otto-git/src/providers/github.rs`
  - `crates/otto-server/src/{run_service,run_workspace,repo_directory}.rs`,
    `crates/otto-state/src/activity.rs`
  - `crates/otto-ssh/**`, `crates/otto-context/**`, `scripts/check.sh`

**Instructions:**

1. **architecture-3-01.**
   - Add `otto_swarm::runtime::lifecycle::activate(ctx, ws, sid, ActivatePolicy::{UserStart,
     Resume, Channel}) -> Result<Swarm>`. It owns the operation guard, the gate, the status write,
     `set_paused`, `start_coordinator` and `emit_status`.
   - Move pause and abort beside it.
   - The handlers and `channels.rs` become thin callers.
   - Replace `ProductStudioHost::{start_swarm_coordinator, emit_swarm_status}` and the direct
     `set_swarm_status` call with one `activate_swarm` host method.
   - Test: activating while an abort holds the guard.
2. **access-control-3-03.** Implement PC-4.
   - Propagate the originator through agent-created subtasks (`engine.rs:1546` and `:1783`),
     following `:1720`.
   - Key `find_agent_session` by (agent, acting user).
   - Owner-guard the role-definition fields in `update_agent`.
   - Update `agent-swarm.md`.
   - Tests:
     - Editor B's task runs as B;
     - B can GET its session;
     - it cannot reach A's private connection.
3. **error-handling-3-07.** Propagate the roster read with `?` in `tick_inner`.
4. **test-quality-3-01.** Make `check_utilization` `pub(crate)`, or factor out a pure
   `plan_rebalance()`. Add the 4 cases from the finding in `swarm_router.rs`.
5. **architecture-3-04.**
   - Add `otto_core::keyed_lock::KeyedLocks<K>`: `lock()` and `try_lock()` return
     `OwnedMutexGuard`, weak entries are pruned on insert, and inner locking tolerates poisoning.
   - Replace the registries in otto-git (`http.rs:521`) and otto-swarm (engine, merge), and in
     otto-product and `personal_agents.rs` if they are simple.
   - Add an `rg` guard to `scripts/check.sh` for new
     `HashMap<String, Arc<tokio::sync::Mutex<()>>>` registries, with an allowlist for the ones that
     remain.
6. **maintainability-3-11.** Retarget the doc links in `diff_cache.rs` and `github.rs`. P08 does
   `lifecycle.rs` and `clickhouse.rs`. Add `RUSTDOCFLAGS='-D rustdoc::broken_intra_doc_links' cargo
   doc --no-deps` to `check.sh --all`.
7. **concurrency-async-3-02.**
   - Pass `&fresh` to `remove_worktree`.
   - `worktree_remove` uses `--force --force` and logs real failures.
   - Make provisioning cancel-aware: a kill-on-drop add, or cancel awaits a bounded provisioning
     guard.
   - Add a slow-git-shim test.
8. **maintainability-3-15.** Move the `// Tests` banners in `local.rs` and `activity.rs` directly
   above `#[cfg(test)]`.
9. **maintainability-3-05.** `repo_directory.rs` uses `otto_core::paths::expand_tilde` and treats
   `~user` as no match.
10. **concurrency-async-3-03.**
    - Wait for ssh's "Local forwarding listening on … port N" line (with `LogLevel=VERBOSE`) before
      trusting the probe, or forward to a Unix socket in a 0700 directory.
    - Correct the comment.
11. **concurrency-async-3-04.** Run the library `put_*`, `set_default_soul`, `backup_library_skill`
    and `put_skill` calls through the crate's `spawn_blocking` helper.
12. **Verify:**
    - `cargo test` for otto-swarm, otto-product, otto-core, otto-git, otto-ssh and otto-context
    - `cargo test -p otto-server --test it swarm`
    - `cargo test -p otto-server --lib run_service`
    - clippy on each crate
    - `bash -n scripts/check.sh`

### P10: MCP catalog generation, the api-client REST engine, and the otto-core API split

- **Findings:** architecture-1-05, integrations-3-01, ux-design-3-04, architecture-1-07,
  architecture-3-06, backend-correctness-3-03, maintainability-3-04, maintainability-3-12,
  backend-performance-3-03
- **Owned paths:**
  - `crates/ottod/src/mcp_tools.rs`, `crates/ottod/src/mcp_tools/**`, `crates/otto-mcp/**`
  - `crates/otto-server/src/{mcp_outward,mcp_http}.rs`, `ui/src/modules/mcp/**`
  - `crates/otto-server/src/routes/api_client.rs`, `crates/otto-server/src/routes/api_client/**`,
    `crates/otto-server/src/api_helpers.rs`, `crates/otto-apiclient/**`
  - `crates/otto-core/src/api.rs`, `crates/otto-core/src/api/**` (new),
    `crates/otto-core/src/{text,hooks,domain}.rs`
  - `crates/otto-server/src/modules/pr_draft.rs`, `crates/otto-channels/src/improve_notify.rs`,
    `crates/otto-memory/**`, `crates/otto-k8s/src/http.rs`
  - `crates/otto-server/src/state_archive.rs`, `crates/otto-server/src/state_archive/**`

**Instructions:**

1. **architecture-3-06, the move only.**
   - Split `otto-core/src/api.rs` into `api/<area>.rs` along the `ui/src/lib/api/types/*.ts` seams
     (sessions, git, review, workflows, product, swarm, dbviewer, …).
   - `api.rs` keeps `mod …; pub use …::*;`, so no import path changes.
   - `cargo check -p otto-core` must pass before you move on, because every crate depends on it.
2. **architecture-1-07.**
   - Move `api_helpers.rs` as it is into `otto-apiclient::rest`. The old file becomes a `pub use`
     shim, so `lib.rs` doesn't change.
   - Then move `execute`, `resolve_environment` and `run_step` behind an `ApiClientCtx` trait.
     The routes stay thin.
3. **architecture-1-05 and integrations-3-01.**
   - For each of the 7 deferred drift pairs, choose the winning schema: the one the daemon route
     actually accepts. Generate the native `ottod` listings from `otto_mcp::outward::catalog`,
     keeping the catalog-parity tests.
   - Add `get_json_within` and `post_json_within` with per-route budgets:
     - `wait_session`: `clamp(timeout_secs, 1, 25) + 5 s`, and fix the schema text;
     - `browser_summarize`: `STUCK_IDLE` plus a margin.
   - Raise or remove the client-level `.timeout`.
   - Test: a stub daemon that answers after 21 s, and the default `wait_session` still returns the
     body.
4. **ux-design-3-04.**
   - When any of `fServer`, `fTool` or `fDecision` is set and nothing matches, show "No calls match
     these filters" with a "Clear filters" action.
   - Keep "No calls yet" for the unfiltered case.
5. **backend-correctness-3-03.**
   - When the input ends with `depth > 0`, restart the scan at `start+1`.
   - `parse_pr_draft` uses `otto_core::text::extract_json`.
   - Tests: a lone `{` in prose, and a `{}` preamble.
6. **maintainability-3-04.**
   - Delete `McpServerProvider` (keep `McpServerSpec`) and `NOTIFY_APPROVAL_KEY`.
   - P08 deletes `transcript_paths`, and P04 handles `FINDINGS_POLL`.
7. **maintainability-3-12.**
   - Delete `GlobalRole` and `NsQuery`.
   - Write `is_valid_state` over the `STATE_*` constants.
   - P02 deletes `DbResult`.
8. **backend-performance-3-03.** Use `ORDER BY` the primary key or rowid in SQL and drop the
   comparator re-serialization. Run the walk and the archive assembly in `spawn_blocking`.
9. **Verify:**
   - `cargo check -p otto-core`, then `cargo test -p otto-core`
   - `cargo test -p otto-mcp` and `cargo test -p otto-apiclient`
   - `cargo test -p ottod mcp_tools`
   - `cargo test -p otto-server --lib` with the `api_client`, `state_archive` and `pr_draft` filters
   - `cargo test -p otto-memory`
   - clippy on each crate
   - `npm run check`

## Deferred

| Finding | Reason |
|---|---|
| architecture-1-04 (generic fan-out and summarizer runner) | Genuinely multi-day: it needs a shared agent-row state machine, cancel guard, summarizer stage and persistence trait across at least 3 engines (code review, skill review, skills eval). P04 is changing the review engine's retry and cancel paths this round (backend-correctness-3-01, backend-performance-3-01), so extracting the generic runner belongs after those settle. |
| architecture-2-06 (otto-rooms crate) | Blocked on ownership: rooms and game rooms are mid-change by the games team, and the plan must not touch `rooms/games/**`. Extract after their merge. |
| architecture-2-08 (game catalog JSON) | Paired with 2-06, and touches the games team's map and capacity ids. |
| concurrency-async-1-05 (plugin `free_port` TOCTOU) | Both fixes change the plugin launch protocol (the plugin binds `:0` and reports its port, or `/health` echoes `OTTO_PLUGIN_TOKEN`), which breaks existing third-party plugins. That needs a product decision on plugin-API versioning. |

Notes on scope (these findings are assigned, not deferred):

- architecture-3-04 is done for the primitive, otto-git and otto-swarm, and opportunistically
  otto-product, plus the guard. The other roughly 10 crates' registries are follow-up work, which
  is multi-day if done all at once.
- architecture-1-13 is done for the JsonTree imports. The remaining lib → modules imports
  (uiCommands and stores) stay open.

## Integration step (orchestrator, after all packages)

1. `python3 scripts/loc-ratchet.py --update-baseline`. Use `--raise` only with a stated reason.
2. `cd ui && node scripts/ui-guards.mjs --update-baseline`, then `npm run check` and
   `npm run test:unit`.
3. `scripts/check.sh` over the branch, run once and alone.
