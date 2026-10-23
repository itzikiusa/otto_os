# Data-integrity review: quality iteration 6, final (2026-10-23)

Lens: persistence integrity. This covers otto-state SQLite and its migrations (append-only and
rollback-compatible), transactions, partial writes, non-atomic file writes, data loss on crash or
restart, caches that disagree with their source of truth, and id/ownership consistency. It also
covers writes Otto makes into the user's own databases from the Database Explorer and the new
`otto.query_db` tool.

Branch `fix/quality-iter6-20261023` @ 2065f00e9. This was a read-only review: no builds, tests or
e2e runs. Paths under `target/`, `node_modules/` and `docs/reviews/` were out of scope.

## Score

Rubric: start at 10.0. Each confirmed open finding subtracts: blocker 1.0, major 0.3, minor 0.1,
nit 0.03.

| Severity | Count | Deduction |
|---|---|---|
| blocker | 0 | 0.00 |
| major | 0 | 0.00 |
| minor | 2 | 0.20 |
| nit | 1 | 0.03 |

**Score = 10.0 − 0.20 − 0.03 = 9.77**

## Previous findings (wave 0)

| Id | Severity | Status | Evidence |
|---|---|---|---|
| data-integrity-6-01 | major | **Fixed** | See the notes below this table. |
| data-integrity-6-02 | minor | **Fixed** | See the notes below this table. |
| data-integrity-6-03 | minor | **Fixed** | See the notes below this table. |
| data-integrity-6-04 | nit | **Fixed** | See the notes below this table. |

**data-integrity-6-01: an edit review that fails partway.**

- **Failure is detected.** The new `ui/src/modules/database/review-batch.ts` adds `batchFailure`. It reads `errored` on the result and on every `more_results` entry. `EditFlow.runReview` (`EditFlow.svelte.ts:796-800`) now reports "The write stopped partway" instead of "Applied".
- **The modal stays open.** It shows the failing statement and the engine message (`ReviewModal.svelte` `review-error`).
- **MySQL and Postgres apply all or nothing.** `reviewBatchSql` wraps a multi-statement review in `BEGIN; … COMMIT;`. On the server, `BEGIN` matches `sql_leaves_session_state` (`types.rs:1453`), so the session is marked `close_on_drop`:
  - in the Postgres driver, by `run_batch` (`postgres.rs:1775-1780`);
  - in the MySQL driver, by `run_script` (`mysql.rs:2144-2146`).

  The open transaction is therefore rolled back when the session closes.
- **Mongo and Redis clear only what was written.** These batches have no transaction. Only the drafts whose statements all completed are cleared (`completedRows`, using `stmtRows` from all three adapters).
- **Other paths.** Doc-editor, insert, duplicate and delete reviews use the same `runReview`.

**data-integrity-6-02: a swarm prompt edit and the record of its editor.**

- **Same UPDATE for task, project and swarm.** `TaskPatch`, `ProjectPatch` and `SwarmPatch` now carry `prompt_updated_by`. It is written in the same UPDATE as the content (`swarm.rs` `update_swarm`, `update_project`, `update_task`: `COALESCE(?, prompt_updated_by)`). The services pass it: `service.rs:147-163`, `:337`, `:447`.
- **One `BEGIN IMMEDIATE` transaction for goals.** Goal writes stamp the goal and its owner's editor inside the transaction: `create_goal_by`, `update_goal` (`GoalPatch.updated_by`) and `replace_standing_goals_by`, through `stamp_goal_editor` and `stamp_prompt_editor`.
- **No race with verification.** The goal routes now hold `operation_guard` (`engine.rs:2108`, `:2136`, `:2189`).

**data-integrity-6-03: MCP profiles in state archives.**

- **The table is archived again.** `mcp_profiles` is out of the global exclusion list and in the portable list (`state_archive/schema.rs:125-128`).
- **Built-in rows merge instead of conflicting.** On restore, `restore_mcp_profile` (`schema.rs:164-202`) merges an archived built-in into the local seeded copy. It applies the archived copy only while the local one is pristine and unlocked. Custom profiles are inserted as ordinary rows.
- **The default comes back through the shared switch.** `restore_default_profile` (`:207-229`) re-applies the archive's default through `mcp_profiles::switch_default`, which `set_default` also uses, inside the restore transaction (`state_archive.rs:559-561`).
- **Tests:** `state_archive/tests.rs`.

**data-integrity-6-04: concurrent Insights config saves.**

- **Unique temp names.** `write_atomic` and `unique_temp` (`otto-insights/src/lib.rs:174-197`) use `<name>.<pid>-<seq>.tmp`.
- **Every writer uses it.** That covers `write_config`, the schedule-state writer (also held under a mutex), `mark_stopped` (`:412`) and the pending-manifest writer (`:1140`).
- **No race with the agent's rename.** `mark_stopped` runs only after `kill_session`, so it cannot overwrite the agent's final rename.

Totals: 4 fixed, 0 still open, 0 partially fixed, 0 invalid.

## Areas covered

### Re-verification

Every wave-0 finding was checked at each location it listed (table above).

### Migrations

- **0196** (`swarm_scope_prompt_editor.sql`) adds five nullable `ADD COLUMN`s with no default. It is rollback-compatible. A 0195 build stamps tasks without `prompt_updated_at`; `prompt_edit` reads those as `updated_at`.
- **No other migration changed.** Nothing was renumbered or edited.
- **No new seeded table**, so the archive-restore seed trap does not apply.

### State archive

- **`apply_rows`:** user remapping, the `mcp_profiles` merge, deferred foreign keys, and the signature parity between preview and apply.
- **Finding 6f-02** covers a gap in the user-id remap.

### Database Explorer writes into the user's database

- **Edit review:** `runReview`, `review-batch.ts`, and the `stmtRows` of the SQL, Mongo and Redis adapters.
- **Doc editor, Redis key edits and the Changes panel:** wave 0 left these uncovered.
  - The doc editor and Redis key edits all go through `openReview` → `runReview`.
  - The Changes panel (`database_changes.rs`) already counts `errored` and `more_results` (`:247-255`) and records `outcome_unknown`.
- **Multi-run:** `service/multirun.rs:853-870` marks a batch with an errored entry as Failed.
- **`otto.query_db` (new):** it self-calls `/db/query`, so partial batches come back with their `errored` flags. MySQL enforces the 60 s deadline with `KILL QUERY` (`mysql.rs:771-785`), and Postgres with `statement_timeout`. A write cannot outlive the 75 s governed call and be retried as a duplicate.
- **Export to path:** the streaming `drain_to_file`. It writes in place by design, because progress is the on-disk size, and an error is reported in the NDJSON stream.

### Swarm

- The repo writers for task, project, swarm and goals; `acting_user`; and the `update_swarm` status-then-patch order (an idempotent retry).

### Product

- **The split runner.** `run/analysis.rs` (finding 6f-01), the summarizer row states, `create_questions`, `run/scratch.rs` (the sweep is limited to the scratch root) and the `reap.rs` split.
- **The `PUBLISH_LOCKS` and `run_detached` publish path.** A dropped request no longer abandons the work between the remote create and the local record.
- **`list_testcases_for_runs`:** read-only.
- **`chat.rs`, `refine.rs` and `watcher.rs` JSON extraction:** no persistence change.

### Usage tailer

- **New transcript roots** (`otto_transcript::roots`; Codex accounts and `$CODEX_HOME`). Wave 0 left the cursor stores uncovered. Checked:
  - sessions never seen before are seeded at their current size;
  - a new rollout of a known session is read from 0 against the persisted cumulative total;
  - canonical-path dedupe stops a file reached through two roots from being counted twice;
  - the seed is committed in the initial journal transaction.
- No double count and no gap was found.

### Files on disk

- **Vault:** `rename` now runs detached, so a dropped request cannot stop it between the move and the backlink rewrite (`otto-vault/src/engine.rs:1617-1641`).
- **Canvas assist:** the database is the source of truth, and commits are guarded by `expect_updated_at`.
- **Rooms:** recap moved to otto-rooms unchanged; the archive still writes temp → rename.
- **Mockup assist:** `write_source` writes temp → rename.
- **Design-assist working copy:** finding 6f-03.

### Assistant threads

- A thread whose `session_id` names a deleted session (left uncovered in wave 0): `session_for_turn` falls back to a fresh session when `resume` fails, and `open_session` repoints the thread (`otto-assistant/src/assistant/threads.rs:525-614`). No issue.

### Workspace delete

- Left uncovered in wave 0. `DELETE /workspaces/{id}` archives the workspace (`routes/mod.rs:264-266`), so nothing cascades over file-backed stores.

### UI persistence

- **apiClient store split:**
  - `drafts.svelte.ts`: save re-adopts the saved auth only when it is unchanged.
  - `placeDraft` never reuses a dirty tab or one whose saved row has not loaded.
  - `collectionsSync.ts`: moved unchanged; partial imports are reported.
- **Database workbench split** (`workbenchPersist.svelte.ts`): debounced flush, quota fallback, restore tombstones.
- **Git store:** post-mutation status refresh.

### Other changed code checked

- `budget/mod.rs` (pure fold), `admin_sessions` paging, the `otto-mcp` governance stage runner (audit row before execute), `workflows/chat.rs` abort reply, `history_index` multi-root scan, `otto-channels` mirror, and `otto-improve` evidence.
- None of these introduced a persistence issue.

## Areas NOT covered

- **Code that only moved this iteration** was read for changed persistence calls only, not re-audited line by line: `workflow_engine` admission, schedule and review_run; `otto-apiclient` engine and secrets; `agent_session.rs`; `otto-agent-run` `session_turn`.
- **Remote-then-local publishing** (Jira and Confluence create, then a local SQLite record). A failure of the local write after the remote create still leaves a remote object Otto does not track. Nothing short of an outbox can close this, and it was not scored.
- **A rolled-back 0195 build editing a swarm project or swarm** does not stamp the 0196 columns. A stale project or swarm editor can then outlive a later rewrite by the old build. This is an access-control question, not data loss.
- **Vault rename across a daemon crash** (between the move and the backlink rewrite): the index rescan repairs the index, but the links are not rewritten. This is pre-existing and needs a journal.

---

## Findings

### data-integrity-6f-01: minor: an agent-suggested learning is saved active and only then deactivated, so it can feed other agents without review

- **Files:**
  - `crates/otto-product/src/run/analysis.rs:584-627` (`create_suggested_learnings`)
  - `crates/otto-state/src/product.rs:1717-1741` (`create_learning`: `VALUES (…, 1, …)` for `active`)
  - Readers of active learnings: `crates/otto-product/src/service.rs:1089` (`build_inject_bundle`), `:1344`, `crates/otto-product/src/memory_facade.rs:38`
- **Evidence:**
  - **Two separate writes.** `create_suggested_learnings` calls `create_learning`, which INSERTs with `active = 1`. A second, separate autocommit `update_learning(…, active: Some(false))` follows (`:608-626`, "create_learning hardcodes active=1; flip it off").
  - **A failed second write is only logged.** If the second write fails (for example `SQLITE_BUSY` past the busy timeout), the code logs `warn!` and moves on. A crash between the two statements has the same effect. Nothing retries or repairs it.
  - **The row is active even when nothing fails.** It stays active for the gap between the two statements. In that gap, another story's run or `build_inject_bundle` can read it through `list_learnings(true)`.
  - **Learnings are global.** `list_learnings` is not scoped to a workspace (`product.rs:1743-1746`).
- **Failure scenario:**
  - An analysis summarizer suggests a learning, which is agent-written text.
  - The deactivate write fails, or the daemon dies between the two statements.
  - The learning stays `active = 1`. Every later story's agent context bundle in every workspace then includes it as a "Relevant Learning".
  - This is the opposite of the stated rule, "a human reviews them before any agent uses them" (`analysis.rs:581-583`).
- **Fix:**
  - Add `active: bool` to `NewLearning`, or add a `create_learning_inactive` method, and bind it in the single INSERT.
  - Drop the follow-up `update_learning`.
  - **Test:** a suggested learning is never returned by `list_learnings(true)`, including when the post-insert path is made to fail.

### data-integrity-6f-02: minor: state-archive restore does not remap user ids in editor columns without a foreign key, so restored runs act as a user who does not exist

- **Files:**
  - `crates/otto-server/src/state_archive.rs:470-495`: the remap applies only to `spec.user_columns`, which are foreign keys to `users` (`state_archive/schema.rs:42-46`), plus a fixed list of names (`user_id`, `created_by`, `approved_by`, …).
  - Identity columns with no foreign key and not in that list:
    - `workflow_triggers.updated_by` (`migrations/0189_workflow_trigger_updated_by.sql:8`)
    - `swarm_tasks.prompt_updated_by` and `swarm_goals.updated_by` (`0195_swarm_prompt_editor.sql:14-15`)
    - `swarm_projects.prompt_updated_by` and `swarms.prompt_updated_by` (`0196_swarm_scope_prompt_editor.sql:15-18`, added this iteration)
    - `database_changes.approved_real_by` (`0120_database_changes.sql:9`)
  - Consumers:
    - `crates/otto-state/src/workflows.rs:1261-1283` (`run_acting_user`, no existence check)
    - `crates/otto-server/src/workflow_engine/run/identity.rs:27-44`
    - `crates/otto-swarm/src/runtime/run.rs:309-389` (`acting_user`)
- **Evidence:**
  - **When users are remapped.** `apply_rows` builds `mapped_users` for every archived user whose `username` already exists locally under another id. A typical case is restoring onto a fresh install whose own `root` was created first (`state_archive.rs:398-416`).
  - **What gets remapped.** `created_by`, `owner_id` and similar names are rewritten, and the archived user row is skipped (`:438-445`).
  - **What does not.** The editor columns above keep the archive's user ids, which do not exist on the target.
  - **Workflows act as the dangling id.** `run_acting_user` returns the newest of `workflow_versions.created_by` (remapped) and `workflow_triggers.updated_by` (not remapped) without checking that the user exists. `run_identity` then uses it as the identity of every unattended run.
  - **Swarms fall back to the originator.** `is_person` fails the existence check, so `acting_user` falls back to the originator, and the recorded editor is silently dropped. That editor identity is what 0195 and 0196 were added to preserve.
- **Failure scenario:**
  - On the old machine, someone edits a workflow's trigger spec after its last graph version.
  - The user restores a full or portable archive onto a new install.
  - Every trigger, schedule, webhook or chat run of that workflow now acts as an id with no `users` row. Its agent sessions, DB grants and MCP token resolve against a ghost.
  - The same restore makes swarm turns that an Editor rewrote run as the original author.
  - The database-change approval ledger names a non-existent real approver.
- **Fix:**
  - Add `updated_by`, `prompt_updated_by` and `approved_real_by` to the remap list in `apply_rows`.
  - Better, derive the list from one shared `USER_ID_COLUMNS` constant guarded by a test that scans every migration for `*_by` / `*_user_id` TEXT columns, so the next identity column cannot be missed.
  - Make `run_acting_user` ignore an id with no `users` row, as swarm's `is_person` does.
  - **Test:** archive a workflow whose trigger was edited by user A and a swarm task whose prompt A rewrote. Restore into a DB where A's username maps to another id, and check that both resolve to the mapped id.

### data-integrity-6f-03: nit: the design-assist working copy is rewritten in place, and a named version with no content commits whatever bytes the working copy holds

- **Files:**
  - `crates/otto-design-assist/src/job.rs:598-618` (`restore_document`: `tokio::fs::write(wf, &head)`)
  - `crates/otto-design-assist/src/lib.rs:1619-1648` (`materialize`: `tokio::fs::write(wf, base_doc)` after moving the old copy aside)
  - Reader: `crates/otto-design/src/http.rs:922-934` (`commit_version`: no `content` → `svc.working_bytes(&a)`, `service.rs:423-430`)
- **Evidence:**
  - **The other writer is atomic.** The save mirror in `otto-design/src/service.rs:791-806` writes temp → rename, with the comment "Same-directory replacement never exposes a partially written file".
  - **These two writers are not.** The assist paths truncate the user-visible working copy and write it directly.
  - **A torn copy survives.** A crash, `ENOSPC` or an I/O error mid-write leaves a truncated file. A failed restore is only logged (`job.rs:593-595`).
  - **It can become a version.** A later Commit version with no `content` reads the working copy as-is. Format validation accepts a truncated HTML document, so the torn file becomes a named version.
- **Fix:**
  - Write both assist paths through the same temp → rename used in `service.rs:797-806`, for example by moving it to a shared `write_working_copy` helper in otto-design.
