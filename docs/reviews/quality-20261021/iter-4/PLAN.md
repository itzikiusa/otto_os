# Quality iteration 4: fix plan

Ten work packages for ten parallel fixers in `/Users/itziklavon/claude_ade-quality`
(branch `fix/quality-loop-20261021`). Each package names its `owned_paths`, and owned paths are
disjoint across packages. Findings come from `docs/reviews/quality-20261021/iter-4/*.md` (carried
ones also cite `../iter-1/` to `../iter-3/`). Read the full finding before you fix it.

This iteration does the big splits that earlier iterations deferred. Each split has one package that
exclusively owns the god file and its new sibling directory, and every other finding that touches
that file is in the same package:

| Split | Package |
|---|---|
| `run_workflow_with_budget` into `RunState` + `run/{admit,schedule}.rs` (architecture-1-01r), `review_run::execute` and `serve_terminal` (architecture-4-04) | P02 |
| `WorkflowsPage.svelte` run monitor (architecture-1-08r), and the api-client REST engine into `otto-apiclient` (architecture-1-07r) | P03 |
| `GraphView.svelte` (architecture-1-09) and the resize half of the Terminal grid negotiation (architecture-2-01r) | P04 |
| `DatabaseStore` in `database.svelte.ts` (architecture-1-10), and the lib→modules imports of the database and k8s stores (architecture-1-13) | P05 |
| MCP native listings generated from the control-plane catalog (architecture-1-05) | P06 |
| Prompt-submit primitive moved into otto-sessions (architecture-4-01) | P07 |
| Usage tailer into otto-usage (architecture-1-16), and the budget and transcript service layer (architecture-3-03) | P09 |
| Design-format registry and assist helpers (architecture-4-02, phase 1) | P10 |

Coordinator items: CodeQL triage is in P10 (alerts 457–461, 404, 397) and P07 (alert 431).
architecture-1-05 is in P06 with the control-plane catalog as the source of truth.
access-control-3-03 / 4-01 (swarm turn identity) is in P08.

## Rules for every package

1. **Ownership.**
   - Edit only your `owned_paths`. A path not listed for anyone is read-only for everyone.
   - Some findings need a small edit in a file that another package owns. Those edits are listed
     under **Cross-package hunks**. The file's owner makes them as part of its own package.
   - Never stash, reset, checkout or reformat files you don't own: other fixers are editing the same
     worktree. Run `cargo fmt` only on the files you changed (`rustfmt <file>`), never `--all`.
2. **Splits are behaviour-preserving moves.**
   - Put new files in a child directory of the file being split, so no crate root needs a new
     `mod` line. Exceptions: P09 adds `budget` and `transcript` to `crates/otto-server/src/lib.rs`
     (P09 owns that file); P06 adds `pub mod mcp_budgets;` to `crates/otto-core/src/lib.rs` (the
     only edit anyone makes to that file).
   - Re-export everything that other code imports from the old path: `pub use` in Rust, and
     `export … from` or a thin wrapper in the UI. Other packages must keep compiling without
     changing their imports.
   - Never change behaviour in the same hunk as a move. Fix findings after the move, in the new
     location.
3. **Size limits.** A new Rust file stays under 4,000 lines (loc-ratchet `new_file_max`). A new
   `.svelte` or `.ts` file stays under 800 lines.
4. **Baselines belong to the integration step.**
   - No package edits `scripts/loc-baseline.json`, `ui/scripts/ui-guards-baseline.json` or
     `ui/scripts/bundle-budget.json` by hand. P02 changes `scripts/loc-ratchet.py` so that
     `--update-baseline` seeds the new `function_max` section.
   - After all packages land, the orchestrator runs `python3 scripts/loc-ratchet.py
     --update-baseline` and `cd ui && node scripts/ui-guards.mjs --update-baseline` once.
   - If your only failure is a ratchet baseline, or a guard hit in a file you don't own, report it
     and don't fix it.
5. **Migrations** are append-only and must stay compatible with the previous build: nullable
   columns or columns with a DEFAULT only. The numbers are reserved below. Don't take another one.

   | Number | File | Package |
   |---|---|---|
   | 0191 | `0191_session_transcript_missing.sql` (nullable `transcript_missing_at` on `sessions`) | P09 |
   | 0192 | `0192_memory_prev_active.sql` (nullable `prev_active` on the memories table) | P09 |
   | 0193 | `0193_broker_replay_status.sql` (`broker_replays.status TEXT NOT NULL DEFAULT 'done'`) | P10 |

6. **Tests.**
   - Only P01 edits `crates/otto-server/tests/it.rs`. It adds these `#[path]` lines **first**,
     before any other work, each with an empty suite file containing only a module doc comment so
     the binary still compiles: `session_files.rs` (P01), `swarm_identity.rs` (P08),
     `review_forge_post.rs` (P02). The named package then owns and fills its file.
   - Everyone else adds tests in-module (`#[cfg(test)]`) or in a suite file in their `owned_paths`.
7. **Contracts.** P01 is the only writer of `docs/contracts/**`, `ui/src/lib/api/types.ts` and
   `ui/src/lib/api/types/**`. It also owns `docs/features/**` except the five guides named in other
   packages (`agent-swarm.md` P08, `review-findings.md` P07, `database-explorer.md` P08,
   `message-brokers.md` P10, `usage-and-cost.md` P09).
   - When another package changes a documented surface, the wire shape is **pinned in this plan**
     (see **Pinned contracts**), and P01 writes the doc rows and TS types from that text. Implement
     exactly the pinned shape. If you can't, stop and report; don't invent another one.
8. **Machine load.** The user's rule is one heavy command at a time; ten parallel builds once froze
   this Mac.
   - Never run `cargo build --workspace`, `cargo test --workspace` or `scripts/check.sh --all`.
   - Use crate-scoped commands only: `cargo check -p X`, `cargo test -p X --lib <filter>`,
     `cargo test -p otto-server --test it <suite>::<filter>`, and `cargo clippy -p X --all-targets
     -- -D warnings`.
   - Cargo serializes on the shared target lock, so don't add a second target dir.
   - Add no new external crates. Path dependencies between workspace crates are fine
     (`Cargo.lock` is rewritten for them, which is expected).
   - In the UI, run `npm run check` and the relevant `node --test unit/<file>` once, at the end.
     Run at most one Playwright spec, once. Errors in files you don't own come from other fixers'
     work in progress: report them, don't fix them.
   - Kill any vite, ottod or ClickHouse process you start.
9. **Report faithfully.** For each finding, say fixed, partly fixed (and what remains) or not fixed.
   Include the exact commands you ran and their results. Order: majors first.

## Pinned contracts

**PC-1. Session-scoped files (access-control-4-03).** P01 implements and documents it.

- `GET /api/v1/sessions/{id}/files/browse?path=<rel>` → `FsBrowse` (same shape as
  `/fs/browse?files=true`; entry paths relative to the session cwd; empty `path` = root).
- `GET /api/v1/sessions/{id}/files/read?path=<rel>` → `FsRead` (same shape and 400 KiB bound as
  `/fs/read`).
- Root: the canonical session cwd. Paths joined with `otto_core::paths::confine_join`; the canonical
  target must stay under the root. `is_denied_file` and the `otto.db*` / `secrets.json` deny still
  apply.
- Auth: session owner, ws Admin or root, plus ws Viewer. Share, MCP and agent-session tokens get 403.
- Errors: not allowed 403; unknown session or missing file 404; absolute path, `..` or an escaping
  symlink 400.
- `/ws/lsp` accepts a non-root `root` that lies under the cwd of a session the caller may open, when
  that cwd is inside one of `AGENT_DATA_SUBDIRS`; otherwise unchanged.
- Client in `ui/src/lib/api/fs.ts` (or the existing fs client file):
  `browseSessionFiles(sessionId, path, signal?)`, `readSessionFile(sessionId, path, signal?)`.
- `/fs/*` routes are unchanged.

**PC-2. Swarm turn identity (access-control-4-01, carried 3-03).** P08 implements it and documents
`agent-swarm.md`; P01 updates the `api.md` swarm rows from this text.

- Each turn's session is created as the task's human originator: `task.created_by` when it is a
  user; for agent-created subtasks, the parent task's originator (walk up); for seeded tasks, the
  plan caller; the swarm creator only when no human originated the work.
- Agent sessions are reused per (agent, acting user).
- `update_agent` changes to prompt, soul, provider, model or cwd need the swarm creator, a ws Admin
  or root (403 otherwise). Other agent fields keep the ws Editor rule.

**PC-3. Personal-agent owner fields (access-control-4-02).** P01 implements and documents it.

- `PUT …/autonomy`: changing standing-goal texts or the rule set needs the agent owner, a ws Admin
  or root (403 `owner_required` otherwise). Toggling `enabled`, proactive on/budget and `primary`
  stays ws Editor. A full-form save with unchanged goals and rules passes. An ownerless preset is
  adopted by the caller as before.
- Memory-item edit/forget needs the agent owner, a ws Admin or root.
- `run_goal` stays ws Editor (the run acts as the owner).

**PC-4. Suspend reason `respawn_failed` (error-handling-4-05).** P07 implements it and maps it in
`ui/src/lib/status.ts`; P01 adds the value wherever `api.md` / `ws.md` list suspend reasons.

- `meta.suspended = { reason: "respawn_failed", error: "<cause>", at: <ms> }`.
- UI label: "Couldn’t restart: <cause>". The status event is emitted only after the status write
  succeeds.

**PC-5. Broker replay status and offset-reset guard (data-integrity-4-05, integrations-4-04).** P10
implements it and documents `message-brokers.md`; P01 updates `api.md` rows and
`ui/src/lib/api/types/brokers.ts`.

- Replay records gain `status: 'running' | 'done' | 'failed' | 'interrupted'` (migration 0193,
  default `'done'` for old rows).
- An offset reset on a group whose state is not `Empty` or `Dead` returns 409 with
  `"consumer group <g> is <state>; stop its consumers first"`. A dry run returns the plan plus
  `warning?: string` with the same text instead of failing.
- An audit entry is written for every committed reset, success or failure, with the per-partition
  errors.

**PC-6. MCP server secret lookup failure (integrations-4-05).** P06 implements it; P01 updates the
`api.md` MCP server rows.

- `PATCH /mcp/servers/{id}` returns 503 `"Keychain unavailable"` and writes nothing when the secret
  lookup fails or the stored blob doesn't parse. Only a missing entry means "no secrets".

**PC-7. Convert scheduled task to workflow (error-handling-4-03).** P02 implements it (route and
`ui/src/modules/scheduled-tasks/**`); P01 updates the `api.md` row.

- A trigger-creation failure returns that error (4xx/5xx) and the task stays enabled. The task is
  paused only after the trigger exists; a failure to pause is returned too. A success response
  always carries `trigger_id`. On failure the created workflow is deleted.

**PC-8. DB connection delete and MCP query timeout (data-integrity-4-02, integrations-4-02).** P08
implements it and documents `database-explorer.md`; P01 updates the `api.md` connection-delete row
and the `otto_db_query` row.

- Deleting a connection, in the same transaction, deletes its `db_widgets` and
  `db_query_history` rows and sets `db_saved_queries.connection_id` to NULL.
- `/db/mcp-query` always runs with `timeout_ms = min(req.timeout_ms ?? 15000, 15000)`
  (`MCP_QUERY_TIMEOUT_MS = 15_000` in otto-dbviewer). P06 keeps the stdio bridge budget for
  `otto_db_query` at 20 s or more.

**PC-9. Confluence page picker (ux-design-4-07).** P01 creates it; P03 adopts it.

- `ui/src/lib/components/ConfluencePagePicker.svelte`, props
  `{ accountId: string; spaceKey: string; value: string | null; title?: string | null;
  onchange: (id: string | null, title: string | null) => void; label?: string }`. A typeahead over
  `searchConfluence` scoped to the space; it stores the page id and shows its title.
- `ui/src/lib/components/ConfluenceSpaceSelect.svelte`, props
  `{ accountId: string; value: string; onchange: (key: string) => void }`.
- P01 lands both files before its other UI work. P03 adopts them at the end of its run; if they are
  missing, P03 reports it and leaves the raw field.

**PC-10. Unified prompt submit (architecture-4-01).** P07 implements it.

- `otto_sessions::submit::{submit_prompt, SubmitOpts, bracketed_paste, wait_for_tui,
  wait_for_tui_strict, dispatched}`; `SessionManager::submit_prompt(&self, sid, prompt,
  SubmitOpts { echo_wait, extra_settle }) -> bool` holds the per-session submit lock through paste,
  echo wait, Enter and re-Enter.
- `otto_review::session::{submit_prompt, bracketed_paste, wait_for_tui, wait_for_tui_strict,
  dispatched}` stay as `pub use` re-exports with unchanged signatures, so P08's `swarm_host.rs` and
  P02's code keep compiling whether or not P07 has landed.

**PC-11. Fireable event kinds (architecture-4-03).** No wire change. P02 adds
`otto_workflows::triggers::FIREABLE_EVENT_KINDS` with a test against `event_to_kind`, and a test
that parses the kind ids out of `ui/src/modules/workflows/triggerForm.ts` (`EVENT_KINDS`) and
asserts they equal the constant. P03 keeps `EVENT_KINDS` (it carries the UI labels) and adds a
comment pointing at the Rust test. This keeps one source of truth with a guard, without a new
route.

## Packages

### P01: Contracts, personal-agent and session-file access control, product UI

**Findings:** access-control-4-02 (major), access-control-4-03 (major), api-contracts-4-01 (major),
access-control-4-04, api-contracts-4-02, api-contracts-4-03, api-contracts-4-04,
api-contracts-4-05, api-contracts-4-06, ui-correctness-4-01, ui-correctness-4-03,
error-handling-4-04, accessibility-responsive-4-03, accessibility-responsive-4-08,
ux-design-4-07, maintainability-4-09, plus the doc rows for PC-2, PC-4, PC-5, PC-6, PC-7, PC-8.

**Owned paths:** `docs/contracts/**`, `docs/features/**` (except `agent-swarm.md`,
`review-findings.md`, `database-explorer.md`, `message-brokers.md`, `usage-and-cost.md`),
`docs/design/**`, `ui/src/lib/api/types.ts`, `ui/src/lib/api/types/**`, `ui/src/lib/api/issues.ts`,
`ui/src/lib/api/fs.ts`, `ui/src/lib/api/product*.ts`, `ui/src/lib/stores/product.svelte.ts`,
`ui/src/lib/stores/issueAccounts.svelte.ts`, `ui/src/lib/stores/mockup-assist.svelte.ts`,
`ui/src/lib/stores/rightPanel.svelte.ts`, `ui/src/lib/rightTabs.ts`,
`ui/src/lib/components/ConfluencePagePicker.svelte` (new),
`ui/src/lib/components/ConfluenceSpaceSelect.svelte` (new), `ui/src/modules/product/**`,
`ui/src/modules/personal-agents/**`, `ui/src/modules/agents/AttachProductStory.svelte`,
`ui/src/modules/panels/FileTree.svelte`, `ui/src/modules/panels/FilesPanel.svelte`,
`crates/otto-server/src/routes/fs.rs`, `crates/otto-server/src/routes/session_files.rs` (new),
`crates/otto-server/src/routes/personal_agents.rs`, `crates/otto-server/src/routes/personal_agents/**`,
`crates/otto-server/src/routes/mod.rs`, `crates/otto-server/src/lsp/**`,
`crates/otto-server/src/policy.rs`, `crates/otto-server/src/feature_guard.rs`,
`crates/otto-server/tests/it.rs`, `crates/otto-server/tests/session_files.rs`,
`crates/otto-server/tests/personal_agents*.rs`, `crates/otto-server/tests/fs*.rs`,
`ui/unit/product*.test.ts`, `ui/unit/issue*.test.ts`, `ui/unit/fileTree*.test.ts`.

**Order:** it.rs placeholders → PC-9 components → access-control-4-02 → access-control-4-03 →
api-contracts → UI items → pinned doc rows (last, from the PC text).

- **access-control-4-02:** PC-3. In `save_autonomy`, diff the incoming goal texts and rule set
  against the stored ones and call `require_agent_owner(['standing goals','rules'])` only when they
  change. In `edit_memory`, `require_agent_owner(['memory'])`. Update `api.md:4692/:4696` and
  `personal-agents.md:302-310`. Tests (in `tests/personal_agents*.rs`): non-owner goal, rule and
  memory-item changes → 403; toggles → 200; owner, ws Admin and root → 200; ownerless preset adopted.
- **access-control-4-04:** `canEditOwnerFields = isRoot || myRole==='admin' || agent.created_by===me
  || !agent.created_by` gates the memory/context documents, memory-inspector edit/forget and the
  autonomy goals/rules. Keep `canEditDocuments` for proactive toggle/budget, primary and Run now.
  Show "Runs as <owner>" to non-owners.
- **access-control-4-03:** PC-1. New `routes/session_files.rs`, registered in `routes/mod.rs` with
  its `policy.rs` entry. `FileTree` gets a lister/reader prop; `FilesPanel` passes the session
  lister/reader when the right panel is in session mode (`rightTabs.ts`, `rightPanel.svelte.ts`).
  `/ws/lsp` change in `lsp/mod.rs`. Tests in `tests/session_files.rs`: owner 200, another non-admin
  member 403, `../../otto.db` 400, LSP on another user's persona dir 403.
- **accessibility-responsive-4-08:** in the same FileTree pass: `role=tree` with roving tabindex,
  rows `role=treeitem` with `aria-level={depth+1}`, `aria-setsize`/`aria-posinset`,
  `aria-expanded` (model on `vault/FileTree`). Move "Open folder" into the row menu.
- **api-contracts-4-01:** take the UI fix, not a route change: `product.svelte.ts` (`:210-214`) and
  `AttachProductStory.svelte` build the agent-route paths from the selected story's
  `workspace_id`, and the attach picker lists only stories of the session's workspace. Document the
  rule in `api.md:1622-1626`, `:1727-1733`, `:764`, `docs/contracts/product.md` P10/P23/P24/P30 and
  replace "purely … for provenance" in `features/product.md:350-351`.
- **api-contracts-4-02 … 4-06:** doc and type fixes as written in the findings. 4-03: type
  `rewrite()` as `Promise<void>`. 4-06: agy gets `<cwd>/.agents/mcp_config.json`.
- **ui-correctness-4-01:** `untrack(() => void loadThread(currentTid))` and
  `untrack(() => void loadChat(currentCid))` in `RefineChat.svelte`.
- **ui-correctness-4-03:** `issueAccounts.reset()` (list=[], loaded=false, error=null,
  inflight=null), called from `resyncForIdentity` (call site is in the auth/identity code; if that
  file is not yours, key the cache by `auth.me?.id` instead, inside the store).
- **error-handling-4-04:** `editmetaError = loadErrorText(e)`, effect gated on it, compact
  `LoadState` with Retry, no toast.
- **accessibility-responsive-4-03:** shared `beginInlineEdit` helper: on begin `await tick()` and
  focus the first field (`use:focusOnMount`); on Cancel or successful save, refocus the Edit
  trigger. Name buttons `Edit {fieldName}`. Apply to every inline editor in OverviewTab.
- **ux-design-4-07:** PC-9 components; use them in `PublishDialog.svelte` and `TestCasesTab.svelte`.
  Confirm and approval copy say `under “{title}”`. Update `review-checklist.md:148`.
- **maintainability-4-09:** delete the three unused exports (`issues.ts:180`, `types/core.ts:10`,
  `mockup-assist.svelte.ts:24`).
- **Sweep hunks in your files:** ux-design-4-11 (`DiscoveryTab.svelte:169`, `PublishDialog.svelte:281`
  "Reload" → "Refresh"; update `docs/design/guidelines/content.md`), ux-design-4-12
  (`content.md:84-93`), accessibility-responsive-4-09 (`AnalysisTab.svelte:539`,
  `RewriteTab.svelte:312`, `TestCasesTab.svelte:647`: one always-mounted status element whose text
  is set on failure), maintainability-4-05 (`docs/features/proof-packs.md:300` pointer → name
  `modules/review/`), maintainability-4-03 (`docs/design/conversation-view.md:29`,
  `docs/features/conversation-view.md:218`: point at `lib/render/*`).

### P02: Workflow engine split, triggers, function ratchet, review_run and serve_terminal splits, rooms

**Findings:** architecture-1-01r (major split), architecture-4-04, architecture-4-03,
test-quality-4-02 (major), test-quality-4-07, test-quality-4-09, error-handling-4-03,
concurrency-async-3-04, accessibility-responsive-4-05, ux-design-4-10.

**Owned paths:** `crates/otto-server/src/workflow_engine.rs`, `crates/otto-server/src/workflow_engine/**`,
`crates/otto-server/src/workflow_trigger_scheduler.rs`, `crates/otto-server/src/workflow_node_driver.rs`,
`crates/otto-server/src/workflow_api_request.rs`, `crates/otto-server/src/workflow_product_publish.rs`,
`crates/otto-server/src/workflow_budget.rs`, `crates/otto-server/src/routes/workflows.rs`,
`crates/otto-server/src/routes/workflow_run_files.rs`, `crates/otto-server/src/routes/workflow_progress.rs`,
`crates/otto-server/src/routes/scheduled_tasks.rs`, `crates/otto-server/src/skill_eval.rs`,
`crates/otto-server/src/modules/review/**`, `crates/otto-server/src/rooms/**`,
`crates/otto-workflows/**`, `crates/otto-sessions/src/ws.rs`, `crates/otto-sessions/src/ws/**`,
`crates/otto-server/tests/workflow*.rs`, `crates/otto-server/tests/scheduled_tasks*.rs`,
`crates/otto-server/tests/rooms_api.rs`, `crates/otto-server/tests/review_forge_post.rs`,
`scripts/loc-ratchet.py`, `ui/src/modules/scheduled-tasks/**`,
`ui/src/modules/rooms/**` (**excluding** `ui/src/modules/rooms/games/**`, which another team owns),
`ui/unit/rooms*.test.ts` (not tests of `rooms/games/**`), `ui/unit/scheduled*.test.ts`.

- **architecture-1-01r (split):** introduce `RunState` (everything `run_workflow_with_budget`
  threads through its loop) and move admission and scheduling into
  `workflow_engine/run/{admit,schedule}.rs` (the `run/` dir exists: `finalize.rs`, `identity.rs`).
  Pure move first, then shrink the entry function to orchestration. No behaviour change; the
  existing workflow suites must pass unchanged.
- **architecture-4-04:**
  1. `scripts/loc-ratchet.py`: scan every `crates/**/src` production file (skip `tests/`,
     `#[cfg(test)]` modules and `*_tests.rs`). Add `function_max = 400`; every function longer
     than that must be listed in a `functions` map keyed `path::name` and can only shrink.
     `--update-baseline` seeds the map from today's spans. Keep the existing three guarded names.
  2. Split `workflow_engine/nodes/review_run.rs` `execute` into `resolve_targets`,
     `launch_reviews` and `await_and_score`.
  3. Move `serve_terminal`'s `ClientFrame` match into a `TerminalConn` struct in
     `crates/otto-sessions/src/ws/terminal_conn.rs` with one method per frame.
- **architecture-4-03:** move `validate_trigger_spec` (`routes/workflows.rs:1616-1712`) into
  `otto_workflows::triggers::validate_spec`; PC-11 constant and tests; move the schedule tick behind
  `WorkflowCtx`, mirroring `spawn_event_trigger_listener` (`workflow_trigger_scheduler.rs`).
- **test-quality-4-02:** in `tests/review_forge_post.rs`, a GitLab git account whose
  `api_base_url` points at wiremock (pattern: `otto-git/src/http_tests.rs:464`), review with
  `pr_number 7`: 201 → posted=true, one request, second approve sends nothing; 500 → posted=false and
  a later approve posts; 400 anchor rejection then 201 general note → two requests, the second
  citing `a.rs:3`.
- **test-quality-4-07:** seed `id_rsa`, a 500 KiB `big.log` and a NUL-containing `blob.bin` in
  `tests/workflow_run_files.rs` `boot()`; Viewer 403 / root 200 on `id_rsa`; `big.log`
  truncated with content ≤ 400 KiB; `blob.bin` content `""`, truncated.
- **error-handling-4-03:** PC-7, route and `scheduled-tasks` UI (missing `trigger_id` = failure).
- **concurrency-async-3-04:** wrap `backup_library_skill` and `put_skill` in `skill_eval.rs` in
  `tokio::task::spawn_blocking`.
- **test-quality-4-09:** in `tests/rooms_api.rs`, re-post sequence 1 → 409, `member_generation+1` →
  403, `capture_epoch 1` → 403; a pure `sequence()` unit test in `rooms/recap/media.rs` (zero, equal,
  lower, short screen chunk refused; next accepted).
- **accessibility-responsive-4-05:** route the remaining host controls in `RoomParticipants.svelte`
  through `refocusMember`; in `RoomPage.svelte`, one always-mounted Release/Request button.
- **ux-design-4-10:** pending admission asks "Cancel your request to join?" with confirmLabel
  "Cancel request" and body "The host won’t see your request any more."
- **Sweep hunks:** ux-design-4-12 (`rooms/recap-client.ts:21`, `rooms/room-client.ts:20`:
  "Couldn’t"); maintainability-4-01 (`modules/review/summarize.rs:483` and `:17` comment → point
  at `resolve_absent_scoped`).

### P03: WorkflowsPage run monitor, workflows UI, workbench editor, api-client REST engine

**Findings:** architecture-1-08r (split), architecture-1-07r (split), ui-performance-4-01 (major),
ui-performance-4-02, ui-correctness-4-04, ui-correctness-4-05, ux-design-4-03, ux-design-4-08,
ux-design-4-09, ux-design-4-11 (lead), error-handling-4-08, maintainability-4-11.

**Owned paths:** `ui/src/modules/workflows/**`, `ui/src/modules/workbench/**`,
`ui/src/modules/api/**`, `ui/src/modules/browser/**`, `ui/src/modules/proof/**`,
`ui/src/modules/panels/**` (except `FileTree.svelte` and `FilesPanel.svelte`),
`ui/src/lib/components/CodeEditor.svelte`, `ui/src/lib/stores/apiClient.svelte.ts`,
`ui/src/lib/stores/proof.svelte.ts`, `ui/src/lib/stores/workspace.svelte.ts`,
`ui/src/lib/api/workflows.ts`, `ui/src/lib/api/api-client.ts`, `ui/e2e/workbench*.spec.ts`,
`ui/unit/workflow*.test.ts`, `ui/unit/workbench*.test.ts`, `ui/unit/detect*.test.ts`,
`ui/unit/apiClient*.test.ts`, `ui/unit/proof*.test.ts`, `ui/unit/workspace*.test.ts`,
`crates/otto-server/src/routes/api_client.rs`, `crates/otto-server/src/routes/api_client/**`,
`crates/otto-apiclient/**`.

- **architecture-1-08r (split):** move run sync, final output and approvals out of
  `WorkflowsPage.svelte` into `workflows/RunMonitor.svelte` beside `RunSteps.svelte`. Pure move.
- **maintainability-4-11:** rename the canvas pair in `canvasGeometry.ts` to `LoopStepLabel` /
  `loopStepLabels` (or derive from `util.ts` `loopSteps`).
- **ux-design-4-08:** `IdPicker` shows a disabled select "Loading {what}…" while loading (like
  `RepoPicker.svelte:26-27`), free text only on error, an `emptyHint` prop, display labels for kind
  and environment.
- **ux-design-4-11 (lead):** "Refresh" for data refetches; preview messages name the real control
  ("Select Retry preview and review it again…"). Your sites: `WorkflowsPage.svelte:1463`. Other
  sites are cross-package hunks (P01, P04, P08, P10).
- **ux-design-4-07 (adopt):** use PC-9 in `workflows/inspector/ProductPublish.svelte:55-70` and
  `WorkflowsPage.svelte:1998-1999`.
- **PC-11:** comment on `triggerForm.ts` `EVENT_KINDS` pointing at the Rust guard test.
- **ui-performance-4-01 (major):** editor path stable per document (`workbench/${doc.id}`), pass
  `language={cmExtFor(lang)}`. In `CodeEditor.svelte`, a live language reconfigure effect modelled on
  the `sqlDialect` effect (`:1116-1122`: `langCompartment.reconfigure` + `ensureLang`), so no
  `setState` runs and selection and history survive. Sticky detection: re-sniff on open, on paste or
  after ~400 ms idle. E2E (`ui/e2e/workbench-typing.spec.ts`): type `{"a": 1}` and `# Heading`
  character by character into a new Untitled file; assert text and caret.
- **ui-performance-4-02:** compute the sniff once from the debounced snapshot; in `detect.ts`, bound
  the sniff to a prefix (16 KB / ~200 lines), `firstLine` via `indexOf`, JSON.parse confirmation only
  on the bounded/idle path; debounce `PlaceholdersPanel` (~200 ms).
- **architecture-1-07r (split):** move `execute`, `resolve_environment` and `run_step` from
  `routes/api_client.rs` into `otto-apiclient::rest` behind an `ApiClientCtx` trait, calling
  `otto_netguard` directly. Thin axum handlers stay; re-export from the old path. Do not change the
  `/api-client/*` wire shapes (P06's MCP dispatch calls them).
- **ui-correctness-4-05:** catch errors per delta poll, retry with backoff, give up after N
  consecutive failures with a "Lost track of this run" message, then `loadAutomationRuns(id)`.
- **ux-design-4-03:** per-list `loading`/`error` in the apiClient store, no toasts; `LoadState`
  (what = automations / run history / cookies, onretry = loader) with the existing `EmptyState`.
- **ui-correctness-4-04:** track `wantedId` in `open()`; `refreshDetail` and the timer refresh use
  `wantedId ?? detail.pack.id`.
- **error-handling-4-08:** bulk archive/delete collect `{title, loadErrorText(e)}` and name them in
  the toast (first cause + "and N more", or one shared cause once).
- **ux-design-4-09:** `BrowserPanel.svelte:664` labels the button with the session title and
  disables it with a reason when no agent is active (or reuse `SendToSession`).
- **Sweep hunks:** ux-design-4-12 (`browser/CredentialsPanel.svelte:178`,
  `workbench/preview/CsvPreview.svelte:25`); maintainability-4-03 (`browser/ReaderView.svelte:24`,
  `panels/CanvasPanel.svelte:98`, `panels/OutputsPanel.svelte:7,22`,
  `workbench/preview/MarkdownPreview.svelte:7`: import from `lib/render/*` / `lib/excalidraw/*`).

### P04: GraphView split, git UI, Terminal resize, shell, agents UI and live-region sweep

**Findings:** architecture-1-09 (split), architecture-2-01r, accessibility-responsive-4-01 (major),
accessibility-responsive-4-02 (major; all sites except ResultsGrid and Athena),
ui-correctness-4-02, ui-correctness-4-07, error-handling-4-01, accessibility-responsive-4-04,
accessibility-responsive-4-06, accessibility-responsive-4-09 (PrList), ux-design-4-01,
ux-design-4-02, maintainability-4-08.

**Owned paths:** `ui/src/modules/git/**`, `ui/src/modules/agents/**` (except
`AttachProductStory.svelte`), `ui/src/modules/insights/**`, `ui/src/modules/mission-control/**`,
`ui/src/modules/skills-lab/**`, `ui/src/modules/settings/**`, `ui/src/shell/**`,
`ui/src/lib/components/Terminal.svelte`, `ui/src/lib/components/termGridSync.ts`,
`ui/src/lib/components/FloatingBar.svelte`, `ui/src/lib/components/AgentDrivingBar.svelte`,
`ui/src/lib/components/FolderPicker.svelte`, `ui/src/lib/components/Skeleton.svelte`,
`ui/src/lib/stores/git*.svelte.ts`, `ui/src/lib/stores/notifications.svelte.ts`,
`ui/src/lib/stores/now.svelte.ts`, `ui/unit/git*.test.ts`, `ui/unit/graph*.test.ts`,
`ui/unit/term*.test.ts`, `ui/unit/notifications*.test.ts`, `ui/unit/archiveGuard.test.ts`.

- **architecture-1-09 (split):** extract `git/RefSidebar.svelte` and `git/graphActions.ts` (action
  handlers) from `GraphView.svelte`. The sidebar uses ~150 GraphView identifiers: pass a typed
  context object (or a small `graphState.svelte.ts` class) rather than 150 props. Run `npm run check`
  before and after the move. Pure move first.
- **ui-correctness-4-02:** in `lostRef`, collect tips of refs in `next` but not `prev`; in
  `applyGraph`, `reloadGraph(true)` when any of them is missing from `freshPrefix`.
- **ui-correctness-4-07:** key Bitbucket participants and GitLab approvers by index in
  `PrDetail.svelte` (`as reviewer, i (i)` / `as name, i (i)`).
- **error-handling-4-01:** `loadErrorText(e)` at `graph-commit-diff.svelte:360`, the stage toast,
  `notifications.svelte.ts:329-332`, `FloatingBar.svelte:315`; in `DiffViewer.svelte` keep `msg` for
  the 400/409 classification and pass `loadErrorText(e)` to the fallback toast.
- **accessibility-responsive-4-09 (PrList):** one always-mounted status element; button
  `aria-describedby` points at the error.
- **architecture-2-01r:** move the resize half of the grid negotiation and the resize-compact queue
  from `Terminal.svelte` into `termGridSync.ts`. Pure move.
- **accessibility-responsive-4-01:** drop `role=alert` from the impersonation banner in
  `App.svelte`; the countdown stays visible but non-live; one always-mounted polite sr-only region
  announces thresholds (5 min, 1 min, expired).
- **accessibility-responsive-4-02:** take ticking text out of live regions (or `aria-hidden`), and
  announce only edges in an always-mounted sr-only status, at `AgentDrivingBar.svelte:73-80`,
  `ConversationView.svelte:361-368`, `now.svelte.ts:80-93` consumers, `InsightsPage.svelte:756-759`,
  `settings/Channels.svelte:565-566`. P05 does ResultsGrid and Athena.
- **accessibility-responsive-4-06:** remove the wrapper role/aria-busy/aria-label and pass
  `label="…"` to `Skeleton` at `SidePane.svelte:120-121`, `App.svelte:1136`, `SplitNode.svelte:213`,
  `InsightsPage.svelte:772`, `WorkItemDetail.svelte:361`, `SkillsBrowser.svelte:350`; second
  simultaneous skeleton gets `announce={false}`.
- **accessibility-responsive-4-04:** `onProviderKeydown` returns unless `e.target` is a
  `[role=radio]` card and resolves the card from `e.target` for +/-; move the ± controls outside the
  radiogroup; one sr-only status replaces per-card live counts.
- **ux-design-4-01:** SplitNode Database pane uses `LazyMount` (what="the Database pane",
  variant="panel") or LoadState with Retry (fresh `loadPage('database')`) and "Reload Otto".
- **ux-design-4-02:** pass `(ms) => rel(ms)` to `suspendedNote` in `SessionView.svelte:555`.
- **maintainability-4-08:** `codeBlocks.ts` imports `unescapeHtml` and uses `escapeHtml` /
  `unescapeHtml` directly.
- **Sweep hunks:** ux-design-4-11 (`settings/Channels.svelte:801`); ux-design-4-12
  (`FolderPicker.svelte:95`, `agents/conversation/SubagentCard.svelte:74`,
  `agents/FirstRunCoach.svelte:264`); maintainability-4-03 (`agents/conversation/chatMarkdown.ts:13`,
  `codeBlocks.ts:40`: import from `lib/render/*`).

### P05: DatabaseStore split, lib→modules imports, database, connections, k8s and AWS UI

**Findings:** architecture-1-10 (split), architecture-1-13 (database + k8s stores),
accessibility-responsive-4-02 (ResultsGrid and Athena part), accessibility-responsive-4-07,
ux-design-4-04, ux-design-4-05, ux-design-4-06, error-handling-4-02.

**Owned paths:** `ui/src/lib/stores/database.svelte.ts`, `ui/src/lib/stores/database/**` (new),
`ui/src/lib/stores/k8s.svelte.ts`, `ui/src/lib/stores/k8s/**` (new), `ui/src/lib/database/**`
(new), `ui/src/lib/k8s/**` (new), `ui/src/lib/stores/aws*.svelte.ts`, `ui/src/modules/database/**`,
`ui/src/modules/connections/**`, `ui/src/modules/kubernetes/**`, `ui/src/modules/aws/**`,
`ui/unit/db*.test.ts`, `ui/unit/database*.test.ts`, `ui/unit/query*.test.ts`,
`ui/unit/k8s*.test.ts`, `ui/unit/aws*.test.ts`, `ui/unit/connection*.test.ts`.

- **architecture-1-10 (split):** split `DatabaseStore` into per-concern stores under
  `lib/stores/database/` (tabs, schema cache, results, history), with `database.svelte.ts` keeping
  the facade class that delegates and re-exports, so every importer keeps working. Port the seven
  store tests in the same package (the harness supports `loadSource(…, {relative: true})`). Pure
  move first.
- **architecture-1-13:** move the helpers that `lib/stores/database*` and `lib/stores/k8s.svelte.ts`
  import from `modules/` into `lib/database/` and `lib/k8s/`, with re-exports at the old module
  paths. Report the remaining lib→modules imports (other stores are out of scope this round).
- **accessibility-responsive-4-02 (part):** `ResultsGrid.svelte:1752-1755` and
  `AthenaView.svelte:569-576`: ticking clock out of the live region; always-mounted sr-only status
  announcing "Running query" / "Query finished in N s" / "Cancelled"; Athena announces only state
  changes and its tabs move out of the status element.
- **accessibility-responsive-4-07:** `onTreeKey` returns when the target is inside
  input/textarea/select or a button other than `.node-label`; key filter, Retry and notes become
  their own treeitems or a `role=group`.
- **ux-design-4-04:** `listBuilderTables` throws or returns `{error}`; `paletteError` +
  `LoadState what="tables" variant="compact"` with retry; import path says "Couldn’t load the tables
  of {db}".
- **ux-design-4-06:** `parseDsn` returns an error string; keep the field open with an inline
  `p.field-error` (`aria-invalid`, `aria-describedby`); clear and close only on success.
- **ux-design-4-05:** check `aws.accountsError && !aws.accounts.length` before `!account`, render
  `LoadState variant=page what="AWS accounts"`; AccountRail shows a Skeleton while loading and a
  compact LoadState on error.
- **error-handling-4-02:** keep the raw message for `isLoginRequired`; display
  `awsErrorText(providerErrorText(e))` in S3, Logs and Athena.

### P06: MCP catalog generation, MCP timeouts, MCP tests and audit tab

**Findings:** architecture-1-05 (major, coordinator item), integrations-4-01 (major),
integrations-4-03 (major), test-quality-4-01 (major), integrations-4-05, architecture-4-05,
error-handling-4-07, ui-correctness-4-06.

**Owned paths:** `crates/otto-mcp/**`, `crates/ottod/src/mcp_tools.rs`, `crates/ottod/src/mcp_tools/**`,
`crates/ottod/Cargo.toml`, `crates/otto-sessions/src/mcp.rs`, `crates/otto-core/src/mcp_budgets.rs`
(new) plus the single `pub mod mcp_budgets;` line in `crates/otto-core/src/lib.rs`,
`crates/otto-server/src/routes/mcp_servers.rs`, `crates/otto-server/src/routes/mcp_cp.rs`,
`crates/otto-server/src/mcp_*.rs`, `crates/otto-server/tests/mcp_*.rs`, `ui/src/modules/mcp/**`,
`ui/unit/mcp*.test.ts`.

- **architecture-1-05:** the control-plane catalog (`otto_mcp::outward::catalog`) is the source of
  truth (user rule: whatever is enabled in the control plane must be visible to sessions). Add
  `otto-mcp` as a path dependency of ottod. Generate the native tool listing (name mapping
  `otto.x` → `otto_x`, description rewrite, session-defaulted `workspace_id`/`session_id`) and the
  routing table from the catalog; keep hand-written native-only tools in an explicit list with a
  test that every catalog entry is either generated or listed as intentionally absent. Resolve the 7
  `DEFERRED_SCHEMA_DRIFT` pairs (`otto_list_connections`, `otto_api_list`, `otto_api_get_request`,
  `otto_api_history`, `otto_api_execute`, `otto_api_upsert_request`, `otto_api_run_automation`)
  toward the control-plane schema, and adapt the native dispatch so saved-request resolution (by
  name, collection or id) keeps working, mapping arguments to the same daemon routes. Do not change
  `routes/api_client.rs` (P03 owns it). Empty `DEFERRED_SCHEMA_DRIFT` and keep the drift test.
- **integrations-4-01:** `GATEWAY_CALL_TIMEOUT` (≥ 3 × `OP_TIMEOUT` + 5 s) next to `OP_TIMEOUT` in
  otto-mcp; the stdio bridge uses `post_json_within("/mcp/gateway/invoke", &body,
  GATEWAY_CALL_TIMEOUT)`. Stub-daemon test that answers after 21 s.
- **integrations-4-03:** shared bridge budgets in `otto_core::mcp_budgets` (ottod `budgets.rs`
  reads them). `codex_mcp_inject_args` appends `-c mcp_servers.otto.tool_timeout_sec=<N>` (N ≥ the
  largest budget, e.g. 960) and the same for otto-browser; user servers pass through a configured
  value or a generous default. Assert in the argv test. Keep the `otto_db_query` budget ≥ 20 s
  (PC-8).
- **test-quality-4-01:** in `tests/mcp_auto_approve.rs`: (1) Mcp Admin grant holder from another
  workspace gets 403/404 on decide and the card stays pending; (2) server-bound card, member with no
  `approve` rule under an enforced policy → 403; (3) request raised by U and decided via a token
  where U impersonates V → 403 "requester cannot approve through impersonation".
- **integrations-4-05:** PC-6. `load_secret_blob` returns `Result`; `enabled_servers` returns `Err`
  on lookup failure (reconcile keeps entries); only `Ok(None)` maps to `{}`.
- **architecture-4-05:** drop otto-mcp's own read-only prefix check and let `/db/mcp-query`'s
  `ReadClass` verdict be the only gate (keep a test that a write is refused through the route).
- **error-handling-4-07:** on a 2xx, a `text()` error maps to `Error::Upstream(format!("{status}:
  reading the response failed: {e}"))`.
- **ui-correctness-4-06:** `loadingMore = false;` in `load()` next to `moreError = null;`.
- **Sweep hunk:** ux-design-4-12 (`mcp/OttoServerHome.svelte:352`).

### P07: Sessions, unified prompt submit, automation, assistant, insights and channels

**Findings:** architecture-4-01 (major), concurrency-async-4-01, maintainability-4-01,
maintainability-4-04, maintainability-4-05, backend-performance-4-03, data-integrity-4-03,
error-handling-4-05, test-quality-4-06, test-quality-4-08, CodeQL alert 431.

**Owned paths:** `crates/otto-sessions/**` (except `src/ws.rs`, `src/ws/**` and `src/mcp.rs`),
`crates/otto-review/src/session.rs`, `crates/otto-review/src/engine.rs`,
`crates/otto-review/src/lib.rs`, `crates/otto-automation/**`, `crates/otto-insights/**`,
`crates/otto-channels/**`, `crates/otto-assistant/**`, `crates/otto-improve/**`,
`crates/otto-orchestrator/src/claude_pty.rs`, `crates/otto-server/src/automation_ctx.rs`,
`crates/otto-server/src/assistant_host.rs`, `crates/otto-server/src/product_host.rs`,
`crates/otto-server/src/agent_session.rs`, `crates/otto-server/src/finding_agent.rs`,
`crates/otto-server/src/cli_update.rs`, `crates/otto-server/src/modules.rs`,
`crates/otto-server/src/modules/ctx_impls.rs`, `crates/otto-server/src/modules/session_input.rs`,
`crates/otto-server/src/routes/resource_access.rs`, `crates/otto-server/src/boot/**`,
`crates/otto-state/src/sessions.rs`, `crates/otto-state/src/improvements.rs`,
`crates/otto-state/src/review_findings.rs`, `scripts/check.sh`, `ui/src/lib/status.ts`,
`ui/unit/status.test.ts`, `docs/features/review-findings.md`.

- **architecture-4-01 + concurrency-async-4-01:** PC-10. Move `wait_for_tui*`,
  `bracketed_paste`, `dispatched`, the echo probe and `submit_prompt` into
  `crates/otto-sessions/src/submit.rs` (plus `SessionManager::submit_prompt` with `SubmitOpts
  { echo_wait, extra_settle }`), re-exported from `otto_review::session`. Replace the
  `AutomationCtx` paste/wait/dispatched primitives with one `submit_prompt` host method; rewrite
  scheduled tasks (`scheduled_tasks_engine.rs:1057-1067`), goal loops (`goal_loop.rs:2079-2093`),
  product (`product_host.rs:181-250`, `extra_settle` for Codex, lock per re-paste group, not across
  the 25 s poll), personal agents (`assistant_host.rs:369-376`), insights
  (`ctx_impls.rs:593-610`) and the channel bridge as calls to it. `scripts/check.sh` guard: fail on
  `bracketed_paste(` or a raw `b"\r"` PTY write outside otto-sessions, with an explicit allowlist
  for `prompt_guard.rs`, `claude_pty.rs` and the swarm runtime until P08 converts it (P08 keeps its
  locked `submit_run_prompt`). Extend `submit_prompt_and_submit_text_do_not_interleave` with a
  scheduled-task-style caller.
- **maintainability-4-04:** the channel bridge uses `otto_core::keyed_lock::KeyedLocks<ConvKey>`;
  `enqueue` is a free function over `locks.handle(key).lock_owned()`; drop `bridge.rs` from
  `KEYED_LOCK_ALLOW` in `check.sh`.
- **maintainability-4-01:** delete `SessionManager::relay` and `RelayOutcome` (move the stray doc
  line onto `SessionManager`), `ImprovementsRepo::set_edit_status` (consider giving the CAS function
  the plain name), and `ReviewFindingsRepo::resolve_absent` (or make it private); point comments
  (`modules.rs:403-459`, `review-findings.md:152`) at `resolve_absent_scoped`.
- **maintainability-4-05:** retarget the `modules.rs` pointers in `otto-review/src/session.rs:704`,
  `otto-review/src/engine.rs`, `otto-improve/src/engine.rs:78`, `modules/ctx_impls.rs:298` to
  `modules/review/`.
- **backend-performance-4-03:** `list_filtered` with `archived=Some(false)`, `kind=Some("agent")`, or a
  narrow `open_agent_titles` projection in `otto-state/src/sessions.rs` for name allocation; live-only
  callers start from the in-memory live map and fetch by `filter.ids` (`manager.rs:2244-2268`,
  `:3346-3370`, `goal_loop.rs:1291-1303`, `cli_update.rs:513-521`, `resource_access.rs:145-158`,
  `bridge.rs:771-786`).
- **data-integrity-4-03:** `remove()` deletes `<data>/sessions/<id>` (confined to the sessions
  root, NotFound ignored) after the row delete; a boot/periodic sweep removes `<data>/sessions/<dir>`
  entries with no sessions row older than a grace period (like `sweep_orphan_inbox`).
- **error-handling-4-05:** PC-4 (`SUSPEND_REASON_RESPAWN_FAILED`, `suspend_lifecycle_line`,
  `status.ts`); emit status only after `update_status` succeeds.
- **test-quality-4-06:** goal-loop `settle_status`: a `BEFORE UPDATE OF status ON goal_loops` trigger
  raising ABORT, dropped after 300 ms → stored status asserted; a persistent case asserts `Err` and one
  error Notice. `advance_cursor` (`personal_agents_engine.rs:1114-1155`): `cfg(test)` hold constant;
  cursor settles after the trigger is dropped.
- **test-quality-4-08:** manager test that runs `sync_workspace_mcp` for an agy session in a temp
  cwd: `.agents/mcp_config.json` lists otto and the delivered set contains otto; for claude, no
  `.agents/mcp_config.json`.
- **CodeQL alert 431 (`rust/command-line-injection`, `otto-automation/src/goal_loop_commands.rs:29`):**
  the verification command is user-authored goal-loop config run in the loop's own cwd by design.
  If it is reachable only from an authenticated editor of that loop, make that visible: route the
  command through a small `VerifyCommand::from_config(&loop)` constructor documented as the trust
  boundary, with a test that a command from another workspace's loop can't be run. Don't add a
  command allowlist (it would break legitimate loops). Report the reasoning for FIXES.md.
- **Sweep hunk:** maintainability-4-02 (`otto-improve/src/engine.rs:93`: delete
  `execute_run_with_autonomy`; fix the doc of `execute_run_with_autonomy_providers`).

### P08: Swarm turn identity and swarm runtime, DB Explorer backend

**Findings:** access-control-4-01 (major, includes carried access-control-3-03), test-quality-4-03
(major), backend-correctness-4-04, data-integrity-4-06, error-handling-4-06, architecture-4-01
(swarm half), backend-correctness-4-03, data-integrity-4-02, integrations-4-02 (major),
test-quality-4-04, test-quality-4-05, maintainability-4-10.

**Owned paths:** `crates/otto-swarm/**`, `crates/otto-server/src/swarm_host.rs`,
`crates/otto-server/src/routes/swarm_ingest.rs`, `crates/otto-server/src/routes/swarm_webhook.rs`,
`crates/otto-state/src/swarm.rs`, `crates/otto-product/src/swarm.rs`,
`crates/otto-server/tests/swarm*.rs`, `ui/src/modules/swarm/**`, `ui/src/lib/stores/swarm.svelte.ts`,
`docs/features/agent-swarm.md`, `crates/otto-dbviewer/**`, `crates/otto-connections/**`,
`crates/otto-state/src/connections.rs`, `crates/otto-state/src/db_explorer.rs`, `dev/dbviewer/**`,
`.config/nextest.toml`, `.github/workflows/ci.yml`, `docs/features/database-explorer.md`,
`crates/otto-server/tests/db*.rs`.

- **access-control-4-01 (+3-03):** PC-2. Resolve the acting user per task (`task.created_by`; agent
  subtasks walk to the parent's originator; seeded tasks use the plan caller; swarm creator only as
  fallback) and create each turn's session as that user (`run.rs:722-734`, `engine.rs:2789`). Key
  `find_agent_session` (`run.rs:218`) by (agent, acting user). `update_agent` prompt/soul/provider/
  model/cwd changes need creator, ws Admin or root (`http.rs:250`). Keep user start, resume, channel
  start, product↔swarm and scheduled flows working. Tests in `tests/swarm_identity.rs`: Editor B's
  task on A's swarm runs as B; B can GET the session; B cannot reach A's private connection; an agent
  subtask of B's task runs as B; a seeded task runs as the plan caller.
- **test-quality-4-03 + backend-correctness-4-04:** switch `Verdict::parse` to
  `extract_json_where(reply, |v| v.get("target_met").is_some())`; the run status parser and the
  product swarm caller (`otto-product/src/swarm.rs:244`, `engine.rs:3188-3229`, `run.rs:120-130`)
  use their own schema predicate (status in {done, blocked, needs_review, in_progress}; tasks is an
  array) in both the parse and the `transcript_ok` closures. Tests: fenced full verdict; `{"target_met":
  false}` with no blocker → blocker true, Unmet at the ceiling; string `"true"` → false; prose → None;
  `{}`-preamble per caller.
- **data-integrity-4-06:** delete `swarm_goals WHERE task_id=?` in `delete_task`'s transaction;
  `clear_project_board` deletes task goals, tasks, then messages in one `begin()` transaction.
- **error-handling-4-06:** `busy_agents(...).await?`; count and announce only moves whose
  `update_task` returned Ok; `warn!` on the rest.
- **architecture-4-01 (swarm half):** collapse the `SwarmHost` `bracketed_paste`/`wait_for_tui`/
  `dispatched` primitives into one locked `submit_prompt` host method; `swarm_host.rs` implements it
  with `otto_review::session::submit_prompt` (PC-10 re-export). Keep `submit_run_prompt`'s lock
  semantics. When done, tell the orchestrator so P07's check.sh allowlist entry for the swarm runtime
  can be dropped at integration.
- **backend-correctness-4-03:** `config::parse` replaces non-object params with `{}`; optionally
  reject non-object params in `validate_params` for DB kinds.
- **data-integrity-4-02:** PC-8 (cascade in one transaction); delete routes treat a missing
  connection as no connection gate, keeping role and ownership checks; optional one-off orphan
  cleanup.
- **integrations-4-02:** PC-8 timeout in `run_read_only`; test that it always sends a bounded
  timeout. Optional cancel-on-drop guard on the inline path.
- **test-quality-4-04:** reuse the shop-only reader stub for `schema_root`, `search_objects`
  (two shop hits in order, `scanned == 1`) and `schema_graph` (only the shop edge).
- **test-quality-4-05:** edit params / credential profile after the claim → Conflict; seed
  `otto_reader`, `shop`, `hidden` in `dev/dbviewer` and drop the `not binary(resource_access)`
  nextest filter; in-flight recheck with a slow statement plus a mid-run `cancellation_requested=1`
  write → Forbidden. If CI can't host the fixture DB, keep the filter and report.
- **maintainability-4-10:** delete `ObjectSnap::fields_ready` and its 13 writes; point the doc at
  the sampled-field cache.
- **Sweep hunk:** ux-design-4-11 (`swarm/SwarmSettings.svelte:314`).

### P09: Usage tailer move, budget and transcript service layer, history, state cleanups, Atlassian transport

**Findings:** architecture-1-16 (major split), architecture-3-03, backend-performance-4-06,
backend-performance-4-01, backend-performance-4-04, backend-correctness-4-01, data-integrity-4-01,
maintainability-4-02, maintainability-4-07, integrations-4-06, architecture-3-02r.

**Owned paths:** `crates/ottod/src/usage_tailer.rs`, `crates/ottod/src/main.rs`,
`crates/otto-usage/**`, `crates/otto-transcript/**`, `crates/otto-server/src/lib.rs`,
`crates/otto-server/src/budget/**` (new), `crates/otto-server/src/transcript/**` (new),
`crates/otto-server/src/routes/usage.rs`, `crates/otto-server/src/routes/history_page.rs`,
`crates/otto-server/src/routes/transcript.rs`, `crates/otto-server/src/routes/mission.rs`,
`crates/otto-server/src/monitor.rs`, `crates/otto-server/src/state_archive.rs`,
`crates/otto-server/src/workgraph_projector.rs`, `crates/otto-state/src/memory.rs`,
`crates/otto-state/src/goal_loops.rs`, `crates/otto-state/src/broker_audit.rs`,
`crates/otto-state/src/reviews.rs`, `crates/otto-state/migrations/0191_session_transcript_missing.sql`,
`crates/otto-state/migrations/0192_memory_prev_active.sql`, `crates/otto-browser/src/cdp.rs`,
`crates/otto-browser/src/live/session.rs`, `crates/otto-git/src/local.rs`, `crates/otto-issues/**`,
`ui/src/lib/sessionUsage.ts`, `ui/unit/sessionUsage.test.ts`, `ui/src/modules/usage/**`,
`docs/features/usage-and-cost.md`.

- **architecture-1-16 (split):** move the tailer driver from `ottod/src/usage_tailer.rs` into
  `otto-usage` (new `otto-usage/src/tailer/` dir); `ottod` keeps a thin wiring call. Keep one Codex
  model extractor in `otto-transcript` and delete the other two copies. Pure move first; the tailer
  tests move with it.
- **architecture-3-03:** add `crates/otto-server/src/budget/` (`BudgetGate`) and
  `crates/otto-server/src/transcript/resolve.rs`; repoint `monitor.rs`, `routes/mission.rs` and
  `routes/usage.rs`, then delete the `*_pub` shims in `routes/usage.rs`. `workflow_budget.rs` (P02)
  keeps compiling through re-exports.
- **backend-performance-4-06:** migration 0191; set `transcript_missing_at` when
  `resolve_transcript_sync` returns `TranscriptMissing` after a root scan; skip the scan within a TTL;
  clear it on the next successful capture.
- **backend-correctness-4-01 + data-integrity-4-01:** migration 0192 (`prev_active`). `soft_forget`
  sets `prev_active = CASE WHEN forgotten_at IS NULL THEN active ELSE prev_active END`; undo restores
  `active = COALESCE(prev_active, COALESCE(prev_state,'accepted') <> 'contradicted')` and the UPDATE
  carries `AND undo_token = ? AND forgotten_at IS NOT NULL` (NotFound when nothing updates). Tests:
  (accepted,false) case; double undo of a suggested memory fails and stays suggested.
- **maintainability-4-02:** delete the unused functions in `memory.rs`, `goal_loops.rs` (and the
  stale first paragraph of `finalize`'s doc), `local.rs`, `otto-usage/src/clickhouse.rs:123`,
  `cdp.rs:237`, `live/session.rs:1236`, `broker_audit.rs:69`. P07 does `otto-improve`; P10 does
  `otto-vault/src/store.rs`.
- **backend-performance-4-01:** decode, write and fsync all candidate files into the stage dir (and
  run the existence probes) in one `spawn_blocking` before `BEGIN IMMEDIATE`; inside the transaction
  publish with a no-replace rename on the blocking pool (`renameatx_np RENAME_EXCL` or
  `linkat`+`unlink`); keep the rollback on commit failure.
- **backend-performance-4-04:** `ReviewsRepo::review_head(id)` (no comments) in `upsert_review`;
  the backfill selects the needed columns for its 50 reviews in one query.
- **maintainability-4-07:** `formatTokens` calls `formatCount` (or is deleted and
  `sessionUsageLabel` calls `formatCount`); update the test.
- **integrations-4-06:** share `rate_limit_wait` with the Atlassian transport; retry short 429 waits
  once on reads, otherwise "jira rate limited — retry in Ns"; map 401/403 to Forbidden with a
  token hint; cap the search error body at `BODY_SNIPPET_MAX`. If `rate_limit_wait` lives in a
  crate otto-issues can't depend on, write a small equivalent in `otto-issues/src/atlassian.rs`
  (you don't own otto-core or otto-git) and report the duplicate for a later move.
- **architecture-3-02r:** route the remaining Jira/Confluence calls through `t.send(req, what)`.
- **Sweep hunk:** maintainability-4-06 (`otto-issues/src/linear.rs:664-669` →
  `otto_core::text::clip_chars`).

### P10: Design-assist phase 1, canvas and render shims, vault, brokers, context, SSH, CodeQL

**Findings:** architecture-4-02 (major, phase 1), backend-performance-4-05, maintainability-4-06,
maintainability-4-03 (lead), ux-design-4-12 (lead), backend-performance-4-02, concurrency-async-4-02,
data-integrity-4-05, integrations-4-04, data-integrity-4-04, backend-correctness-4-02, CodeQL alerts
457–461, 404, 397.

**Owned paths:** `crates/otto-design-assist/**`, `crates/otto-design/**`, `crates/otto-canvas/**`,
`crates/otto-agent-run/**`, `crates/otto-product/src/design_format.rs`,
`crates/otto-server/src/mockup_assist.rs`, `crates/otto-server/src/design_scene3d.rs`,
`crates/otto-vault/**`, `crates/otto-brokers/**`, `crates/otto-state/src/broker_ops.rs`,
`crates/otto-state/migrations/0193_broker_replay_status.sql`, `crates/otto-context/**`,
`crates/otto-ssh/**`, `crates/otto-k8s/**`, `crates/otto-state/src/workbench.rs`,
`crates/otto-core/src/secret_paths.rs`, `ui/src/modules/canvas/**`, `ui/src/modules/design-hall/**`,
`ui/src/modules/vault/**`, `ui/src/modules/snip/**`, `ui/src/lib/render/**`,
`ui/src/lib/excalidraw/**`, `ui/unit/renderQueue.test.ts`, `ui/unit/canvas*.test.ts`,
`docs/features/message-brokers.md`.

- **architecture-4-02 (phase 1, behaviour-preserving):** make `otto-design::format` the only format
  registry: move the scene3d model and validator there (from `design_scene3d.rs` / the product
  copy) and make `otto_product::DesignFormat` (`design_format.rs`) a view over it; move
  `split_excalidraw` / `merge_excalidraw` into an Excalidraw adapter module in otto-design-assist
  (`canvas/assist.rs` uses it); move `extract_fenced` into otto-agent-run; one `AgentTurn` type with
  `done_file`. Re-export from old paths. **Phase 2 (deferred):** one engine behind an `AssistTarget`
  trait for Design Hall, canvas and product mockups. The three engines differ in completion,
  validation and Excalidraw rules, so merging them means choosing which rules win (a product
  decision) and is multi-day; report it as not done with that reason.
- **backend-performance-4-05:** one shared live-preview poll helper that remembers `(len, modified)`
  and skips the read when unchanged; used by the design and canvas loops.
- **maintainability-4-06:** `otto-design-assist/src/lib.rs:600-608` → `otto_core::text::clip_chars`.
- **maintainability-4-03 (lead):** importers move to `lib/render/*` and `lib/excalidraw/*`; delete
  `canvas/mermaid.ts`, `canvas/renderQueue.ts`, `canvas/excalidraw-build.ts` and `vault/mdRender.ts`
  only after `grep` shows no importer left (other packages update theirs as sweep hunks; if any
  remain at your end, keep that shim and report). Your sites: `canvas/d2.ts:13`,
  `canvas/ExcalidrawCanvas.svelte:30`, `design-hall/ArtifactThumb.svelte:121`,
  `design-hall/ArtifactStage.svelte:26`, `vault/NoteView.svelte:46`, `unit/renderQueue.test.ts:6`.
- **ux-design-4-12 (lead):** "Couldn’t …" and clipboard toasts that say "Select the text and copy it
  manually." Your sites: `CanvasPage.svelte:206`, `D2Canvas.svelte:146`, `MermaidCanvas.svelte:136`,
  `snip/SnipEditor.svelte:126`. Others are sweep hunks.
- **backend-performance-4-02:** batch verified `(rel,size,mtime)` tuples (500 per batch) under one
  publication lock and one `changed_since`/stat re-check per batch, written with one transaction
  (`store.upsert_files_batch`); delete removals in batches with `path IN (SELECT value FROM
  json_each(?))`.
- **concurrency-async-4-02:** after `start_watch` succeeds, re-check `store().get_vault(id)` and
  `stop_watch` if gone (or a retired set under the `watch_starting` lock).
- **data-integrity-4-05 + integrations-4-04:** PC-5. Insert the `broker_replays` row up front with
  `status='running'` and an audit `replay.start` entry before the first send; update evidence
  incrementally; final status at the end; mark running rows `interrupted` inside
  `BrokersService::new` (it runs at boot; no boot edit needed). Group-state check and the audit
  entry on every reset.
- **data-integrity-4-04:** write the skills manifest with temp file + `sync_all` + rename;
  serialize edits per skills dir with a process-wide `KeyedLocks<PathBuf>`; a malformed (not missing)
  manifest doesn't degrade to "all user-owned".
- **backend-correctness-4-02:** add `-o ControlMaster=no -o ControlPath=none` to `base_args`; assert
  in the local and socks arg-shape tests.
- **CodeQL (coordinator item 1):** alerts 457–460 `rust/path-injection` in
  `otto-k8s/src/clusters.rs:205/214/368/584`, 461 `rust/weak-sensitive-data-hashing` in
  `otto-state/src/workbench.rs:192`, plus 404/397 `rust/path-injection` in
  `otto-core/src/secret_paths.rs:177/491`.
  - k8s: the kubeconfig path is chosen by the (authenticated) user on purpose. Check who can reach
    these call sites. If any non-root, non-owner caller can point it at an arbitrary file, that is real:
    fix it with a regression test. Otherwise make the safety visible: one
    `validated_kubeconfig_path(raw) -> Result<PathBuf>` helper (expand `~`, canonicalize, must be a
    regular file, size cap, not inside Otto's data dir) used at all four sites, with unit tests. Don't
    restrict kubeconfigs to `~/.kube` (legitimate setups keep them elsewhere).
  - workbench `sha256_hex`: if it hashes content for change detection / dedup (not passwords), it is a
    false positive. Rename to make the purpose explicit (e.g. `content_digest_hex`) with a doc comment
    and keep SHA-256. If it ever hashes a secret, switch that use to a keyed or password hash.
  - secret_paths: same approach as k8s; check the existing confinement and route the two sites
    through a named validation helper if it isn't already visible.
  - Return a FIXES.md paragraph per alert: real or false positive, why, and what changed.
- **Sweep hunks:** maintainability-4-02 (`otto-vault/src/store.rs`: delete `upsert_note`,
  `replace_links`, `replace_tags`, `unresolved_sources`, `update_link_dst`, `like_search`; move
  `fts_search` into the test module); ux-design-4-11 (`design-hall/ArtifactView.svelte:482`).

## Cross-package hunks (who edits what in a file they own)

| Finding | Site | Edited by |
|---|---|---|
| ux-design-4-11 | DiscoveryTab, PublishDialog, content.md / WorkflowsPage / Channels / SwarmSettings / ArtifactView | P01 / P03 / P04 / P08 / P10 |
| ux-design-4-12 | content.md / room-client, recap-client / CredentialsPanel, CsvPreview / FolderPicker, SubagentCard, FirstRunCoach / OttoServerHome / canvas, snip | P01 / P02 / P03 / P04 / P06 / P10 |
| maintainability-4-03 | conversation-view docs / ReaderView, CanvasPanel, OutputsPanel, MarkdownPreview / chatMarkdown, codeBlocks / canvas, design-hall, vault, unit test + shim deletion | P01 / P03 / P04 / P10 |
| maintainability-4-02 | otto-improve / otto-vault store / everything else | P07 / P10 / P09 |
| maintainability-4-01 | `modules/review/summarize.rs` comments | P02 |
| maintainability-4-05 | `proof-packs.md:300` | P01 |
| maintainability-4-06 | `otto-issues/src/linear.rs` / `otto-design-assist/src/lib.rs` | P09 / P10 |
| accessibility-responsive-4-02 | ResultsGrid, Athena / all other sites | P05 / P04 |
| accessibility-responsive-4-09 | AnalysisTab, RewriteTab, TestCasesTab / PrList | P01 / P04 |
| ux-design-4-07 | ProductPublish, WorkflowsPage:1998 (adopt PC-9) | P03 |
| architecture-4-01 | swarm runtime + `swarm_host.rs` | P08 |

## Deferred

- **architecture-1-04** (shared fan-out + summarizer runner for review, skill review and vault
  docs): multi-day. The three engines have different summarizer contracts, budgets and fallbacks;
  a shared runner needs a design pass and a port of all three with their tests.
- **architecture-2-06** (extract an `otto-rooms` crate): blocked on the games team's merge; their
  `game_rooms` and `ui/src/modules/rooms/games/**` are owned by another team.
- **architecture-2-08** (one game catalog JSON): depends on 2-06 and on the games team's files.
- **concurrency-async-1-05** (plugin `free_port` TOCTOU): both fixes change the plugin protocol
  (plugin binds `:0` and reports its port, or `/health` echoes `OTTO_PLUGIN_TOKEN`), which breaks
  existing third-party plugins; needs a product decision and a protocol version bump.
- **architecture-4-02 phase 2** (one `AssistTarget` engine): see P10; product decision on which
  completion and validation rules win, and multi-day. Phase 1 is in P10.
