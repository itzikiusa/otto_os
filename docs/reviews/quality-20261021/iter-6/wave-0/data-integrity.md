# Data-integrity review — quality iteration 6, wave 0 (2026-10-23)

Lens: persistence integrity. Covers otto-state SQLite and its migrations (append-only and rollback
compatibility), transactions, partial writes, non-atomic file writes, data loss on crash or restart,
caches that disagree with their source of truth, and id/ownership consistency. It also covers writes
Otto makes into the user's own databases from the Database Explorer.

Branch `fix/quality-iter6-20261023` @ 02a64019b. This was a read-only review: no builds, tests or
e2e runs.

## Score

Rubric: start at 10.0. Each open finding subtracts: blocker 1.0, major 0.3, minor 0.1, nit 0.03.

| Severity | Count | Deduction |
|---|---|---|
| blocker | 0 | 0.00 |
| major | 1 | 0.30 |
| minor | 2 | 0.20 |
| nit | 1 | 0.03 |

**Score = 10.0 − 0.30 − 0.20 − 0.03 = 9.47**

## Previous findings (iteration 5)

| Id | Severity | Status | Evidence |
|---|---|---|---|
| data-integrity-5-01 | major | **Fixed** | New `crates/otto-git/src/collections.rs`. The pull, `read_collection_files` (`:37-62`), checks `symlink_metadata` and skips a symlinked dir or entry. The push, `write_collection_files` (`:71-93`), refuses a symlinked or non-directory `collections/`, resolves every target through `confined_worktree_file` before writing any of them, then writes each file through a sibling temp file and `rename` (`write_via_rename`, `:98-112`). Tests at `:143` and `:164`. |
| data-integrity-5-02 | minor | **Fixed** | `ui/src/lib/events/routes/automation.ts:25` adds `resyncAutomation(workspaceId)`, which `events.svelte.ts:677` calls during resync. |
| data-integrity-5-03 | minor | **Fixed** | `crates/otto-state/src/workflows.rs:453-…` runs one `BEGIN IMMEDIATE` transaction. It deletes the workflow under the live-run guard, pauses the enabled scheduled tasks that target it and records an error run for each. |
| data-integrity-5-04 | nit | **Fixed** | `crates/otto-brokers/src/replay.rs:175-198`: the `Interrupted` drop guard writes the acknowledged evidence with status `interrupted` from a spawned task. Test at `:284`. |
| data-integrity-5-05 | nit | **Fixed** | `crates/otto-state/src/git.rs:343-357`: `delete_repo` deletes `pr_reviews` and the `repos` row in one transaction. Test `delete_repo_removes_its_pr_reviews_and_their_comments`. |

All five findings are fixed and none is invalid: 5 fixed, 0 still open, 0 rejected.

## Areas covered

- **Re-verification of every iteration-5 finding** (table above).
- **Migration 0195** (`swarm_prompt_editor.sql`): two nullable `ADD COLUMN`s with no default. They are rollback-compatible.
- **Uncovered items carried over from iteration 5:**
  - **`otto-git` autostash paths** (`local.rs:3191-3250` `checkout_autostash`, `:4296-4345` `pop_stash_sha` / `restore_autostash`, `ops.rs:219-258` `pull_autostash_mode`, `merge_branch`):
    - Every restore uses `stash apply <sha>`, never `pop`, so the stash entry always survives.
    - A failed switch or merge returns to `origin` before applying the stash.
    - A restore that fails or conflicts says where the changes are.

    No issue.
  - **`otto-skills` `install_into`** (`lib.rs:221-236`): it removes the Library copy, then seeds it. Its only route caller, `install_one_into_dirs` (`http.rs:384-440`), takes a reserved-name backup first when `backup` is set. The source is compiled in, so a crash mid-seed is repaired by reinstalling. No issue.
  - **`otto-insights`:**
    - The config is written with temp file and rename. Finding 6-04 covers its shared temp name.
    - The completion manifests are written with temp file and rename, and the agent is told to rename atomically.
    - `mark_stopped` checks the run id.
  - **`otto-improve` `backup_and_replace`** (`engine.rs:1361-1389`): writes a timestamped backup, then a temp file, then renames. No issue.
  - **`otto-channels` thread→session mapping** (`bridge.rs:445-600`): the map is in memory only. A dead mapping is dropped, and the session is recovered from its `meta` channel/chat/thread stamp. No persisted mapping can be orphaned. No issue.
  - **Personal-agent rooms:**
    - `agent_rooms`, `agent_room_members` and `agent_room_messages` all use `ON DELETE CASCADE` (0112).
    - Retention trims each room to its newest messages and recomputes `message_count` (`retention.rs:340-365`).

    No issue.
  - **`goal_loop.rs` `write_owned_file`:** uses `O_NOFOLLOW` and removes a planted link. Scheduled-task reports go to a unique per-run path, and a crash before settlement leaves the run to the boot reaper. No issue.
  - **State-archive restore and `mcp_profiles`:** finding 6-03.
- **Recently changed persistence code (since iteration 5):**
  - `otto-state` `git.rs`, `workflows.rs`, `workflow_progress.rs` (keyed repair gate), `mcp_control.rs` (tests only).
  - `swarm.rs` with the new prompt-editor writers: finding 6-02.
- **Database Explorer edit-review apply path, into the user's own database:**
  - `EditFlow.svelte.ts` `runReview`, the `edit-sql.ts` builders, `database.runManagedStatement`.
  - The batch executors in the Postgres and MySQL drivers, and `types::fold_batch_results` / `errored_batch_entry`.

  Finding 6-01.
- **Design Hall working-copy mirror** (`otto-design/src/service.rs:791-806`): temp file and rename. The blob is the source of truth. No issue.
- **Personal-agent memory documents** (`personal_agent_documents.rs`): staged write plus `renameat`. Symlinks and hard links are refused. No issue.
- **Vault-docs orphan draft trashing** (`vault_docs_agent.rs:1848-1869`): a no-overwrite rename into `.trash`. No issue.
- **Session rooms, room games and classroom:** server state is in memory only (no `sqlx` or file writes under `rooms/` other than the recap archive, which iteration 5 reviewed), and nothing under `game_rooms`. Nothing new is persisted.
- **Session `meta_json` writers:** the live writers use `json_patch`. The whole-object `set_meta` (`sessions.rs:726`) has no production caller.

## Areas NOT covered yet (for iteration 7)

- `otto-assistant` threads: what happens when `assistant_threads.session_id` names a deleted session (`threads.rs:253` only checks `is_none()`). Does the next turn recreate the session, or fail?
- The other DB Explorer write paths that skip the edit review (DocEditor/Mongo raw document saves, Redis key edits, the Changes panel), checked for partial-apply and success-reporting gaps of the same kind as 6-01.
- `otto-usage` ingestion cursor stores (crash between a ClickHouse insert and the cursor advance: double count or gap).
- Workspace delete: the cascade over file-backed stores (vaults, canvas scenes, design projects, session folders).
- Rows written after 0195 that a rolled-back build edits: does an older build's `update_task` keep `prompt_updated_by` (yes, it never names the column), and does a stale editor then outlive a later rewrite by the older build?

---

## Findings

### data-integrity-6-01 — major — A multi-row edit review that fails partway is reported as "Applied": the user's database is left half-written and the drafts are discarded

- **Files:**
  - `ui/src/modules/database/EditFlow.svelte.ts:720-775` (`runReview`)
  - `ui/src/modules/database/edit-sql.ts:156-186` (`buildUpdate`: "ONE statement per pending row … Multiple rows become a batch")
  - `crates/otto-dbviewer/src/drivers/postgres.rs:688-690`, `:1761-1823` (`run_batch`)
  - `crates/otto-dbviewer/src/drivers/mysql.rs:2160-2205`
  - `crates/otto-dbviewer/src/types.rs:954-974` (`fold_batch_results`, `errored_batch_entry`)
- **Evidence:**
  - **The batch reports failure inside a 200.** A review over N edited rows sends N `UPDATE …;` statements in one `POST …/query`. Both drivers run a multi-statement text as a batch, statement by statement, with no transaction. When statement k fails, they push `errored_batch_entry(preview, error)` and `break` (postgres `:1808-1815`, mysql `:2196-2203`). They return `Ok(fold_batch_results(results))`. The doc comment on `errored_batch_entry` confirms that the response is a `200` with the completed results plus the errored one.
  - **The UI never checks the error flag.** `runReview` gets that result from `database.runManagedStatement`, which throws only on a non-2xx response. It checks only `'canceled' in res`. Then it calls `toasts.success('Applied', 'Statement ran successfully')` (`:746`), `clearCoveredDrafts()` (`:752`) and closes the review. Nothing in `ui/src/modules/database` or `ui/src/lib/stores/database*` reads `errored` or `more_results` on this path; only `ResultsGrid.svelte:187-192,1402-1411` renders them, for the query tab.
  - **Failure scenario:**
    - The user edits 5 rows and opens Review & apply.
    - The 3rd `UPDATE` hits a constraint or type error.
    - Rows 1–2 are committed (autocommit, no transaction) and rows 3–5 are not.
    - The UI says "Applied — Statement ran successfully" and throws away all 5 drafts. The refreshed grid quietly shows rows 3–5 back at their old values.
    - The user's edits are lost, and the user is never told the write was partial. The same applies to Redis and Mongo batches, which use the same `errored_batch_entry` contract (`redis.rs:366`, `mongodb.rs:950`).
- **Fix:**
  - **UI:** in `runReview`, treat `res.errored || res.more_results?.some(r => r.errored)` as a failure. Name the failing statement and its `message`, and how many statements ran before it. Clear only the drafts whose statements completed, keep the rest pending, and keep the modal open.
  - **Server:** make the edit-review write atomic on SQL engines. Either wrap the generated batch as `BEGIN; …; COMMIT;`, or add an `atomic: true` query flag that runs the batch in one transaction and rolls back on the first error. `BEGIN` already marks the connection `close_on_drop` (`types.rs:1459`), so an aborted transaction is never returned to the pool.
  - **Test:** a 3-statement review whose 2nd statement fails must not show "Applied", must keep drafts 2–3, and (with the atomic flag) must leave row 1 unchanged.

### data-integrity-6-02 — minor — A swarm prompt edit and the record of who made it are written separately, so a failed second write loses the editor for good

- **Files:**
  - `crates/otto-swarm/src/service.rs:375-422` (`update_task`: `repo.update_task(…)`, then `repo.set_task_prompt_editor(…)`)
  - `crates/otto-swarm/src/runtime/engine.rs:2060-2082` (create task goal), `:2140-2196` (update goal; `note_goal_editor` makes two more separate writes)
  - `crates/otto-state/src/swarm.rs:1378-1411`, `:1523-1555`
- **Evidence:**
  - **The two writes are separate statements.** `SwarmRepo::update_task` commits its `UPDATE swarm_tasks SET title…` on the pool. `set_task_prompt_editor` is a second `UPDATE` on the pool afterwards, with no shared transaction. The goal routes chain three autocommit writes: `update_goal`/`create_goal`, then `set_goal_editor`, then `set_task_prompt_editor`.
  - **A failed second write is never repaired.** If the second write fails (for example `SQLITE_BUSY` past the busy timeout), the route returns an error but the new title, description, labels or goal text is already committed. `prompt_changed` and `rewrites` compare the request with the stored row (`service.rs:387-392`, `engine.rs:2148-2156`). A retry of the same edit therefore sees no change and never records the editor.
  - **Failure scenario:** `acting_user` (`runtime/run.rs:267-289`) falls back to the task's originator. The Editor's rewritten prompt then runs under the originator's identity, which is exactly what migration 0195 was meant to prevent. The goal routes also hold no `operation_guard`, so a verification turn can start between the goal write and the editor write.
- **Fix:**
  - Record the editor in the same statement as the content change. Add an optional `prompt_updated_by` to `TaskPatch` and `GoalPatch`, and set `prompt_updated_by = COALESCE(?, prompt_updated_by)` inside the existing `UPDATE`. For the goal routes, run the goal write and the parent task's `prompt_updated_by` in one transaction.
  - **Test:** make `set_task_prompt_editor` fail, retry the PATCH, and check that `prompt_updated_by` is set.

### data-integrity-6-03 — minor — State archives leave out every MCP profile, so a restore onto a fresh install loses custom profiles and their sessions widen to the Full profile

- **Files:**
  - `crates/otto-server/src/state_archive/schema.rs:64-90` (`excluded_table`: `"mcp_profiles"`, added in commit e924d0b6c)
  - `crates/otto-state/migrations/0194_mcp_profiles.sql` (`builtin`/`locked` columns; seeds stamped with `now`)
  - `crates/otto-state/src/mcp_profiles.rs:116-152` (`resolve` skips unknown ids and falls back to `default_profile`, which falls back to `full`)
- **Evidence:**
  - **Why the table was excluded.** Commit e924d0b6c excluded the whole table because the seeded built-in rows of a fresh target conflict with the archived copies: their `created_at`/`updated_at` are the migration time, so the rows always differ, and the preview refused the restore.
  - **What the exclusion also drops.** It also drops every user-created profile (`builtin = 0`), user edits to the non-locked presets, and the user's choice of default.
  - **Sessions still name the dropped ids.** Sessions are archived with `meta.mcp_profiles` naming those ids. After restoring onto a new machine, `resolve` finds none of them and returns the default, the locked `full` profile, "every Otto tool".
  - **External connections widen too.** One whose stdio args or URL name a custom profile also resolves to Full.
  - **Result:** a restore silently loses user configuration and turns tool narrowing that the user set up into the widest tool set.
- **Fix:**
  - Archive the table, but treat built-in rows specially. Either export only `builtin = 0` rows plus each builtin's editable fields and `is_default`, or merge built-in rows on restore (skip a builtin whose id exists instead of reporting a conflict).
  - Restore `is_default` inside the repo's transactional default switch.
  - Add a restore test: archive with a custom profile set as default and a session naming it, restore into a fresh DB, and check that the profile, the default and the session's resolution all survive.

### data-integrity-6-04 — nit — Two concurrent Insights config saves share one temp file and can leave a torn `config.json`, which then reads back as all-off defaults

- **Files:**
  - `crates/otto-insights/src/lib.rs:166-175` (`write_config`: fixed `config.json.tmp`)
  - `:1238-1256` (`put_config`: no lock)
  - `:150-162` (`read_config`: parse failure falls back to `InsightsConfig::default()`)
- **Evidence:**
  - **One fixed temp path, no lock.** Every save writes `std::fs::write(<dir>/config.json.tmp)` and then renames it. Two root PUTs run on separate blocking threads with nothing serialising them.
  - **The writes can interleave.** With `O_TRUNC`, the sequence A open → B open/truncate → A write → B write leaves B's bytes followed by A's tail. A renames this torn file into place; B's rename then fails with `ENOENT`, so B gets a 500.
  - **The torn file reads back as defaults.** `read_config` returns the defaults on a parse error. The comment at `:169-170` says the temp-file pattern exists to avoid exactly that.
- **Fix:** Use a unique temp name, `config.json.<pid>-<seq>.tmp` (as `collections.rs:98-112` does), or serialise `write_config` with a process-wide mutex. The same applies to the shared `json.pending.tmp` name used by `mark_stopped` (`:326`) and the pending-manifest writer (`:1056`).
