# Data-integrity review — quality iteration 4 (2026-10-22)

Lens: persistence integrity. Covers otto-state SQLite and its migrations (append-only and rollback
compatibility), transactions, partial writes, non-atomic file writes, data loss on crash or restart,
caches that disagree with their source of truth, and id/ownership consistency.

Branch `fix/quality-loop-20261021` @ 6cff08270. Read-only review: no builds, tests or e2e were run.

## Score

Rubric: start at 10.0. Each open finding subtracts: blocker 1.0, major 0.3, minor 0.1, nit 0.03.

| Severity | Count | Deduction |
|---|---|---|
| blocker | 0 | 0.00 |
| major | 0 | 0.00 |
| minor | 5 | 0.50 |
| nit | 1 | 0.03 |

**Score = 10.0 − 0.50 − 0.03 = 9.47**

## Previous findings (iteration 3)

| Id | Severity | Status | Evidence |
|---|---|---|---|
| data-integrity-3-01 | minor | **Fixed** | Migration `0190_memory_prev_state.sql` adds nullable `memories.prev_state` (additive, rollback-safe). `memory.rs:957-961` stores `state` only on the first forget (`CASE WHEN forgotten_at IS NULL`), and `:1011-1015` restores `COALESCE(prev_state,'accepted')` with `active = (… <> 'contradicted')`. A residual race in the same function is new finding 4-01. |
| data-integrity-3-02 | minor | **Fixed** | `db_explorer.rs:456-476` deletes widgets and the dashboard in one transaction. `otto-dbviewer/src/service.rs:3436-3452` (`ensure_dashboard_in_workspace`) rejects a `dashboard_id` that is missing or in another workspace, on create and on move. |
| data-integrity-3-03 | minor | **Fixed** | `routes/assistant.rs:168-178` and `tasks.rs:1447-1460` remove the paths returned by the thread delete, confined to the owner's inbox (`in_inbox`, `tasks.rs:1478`). `sweep_orphan_inbox` (`:1507-1540`) hourly removes unreferenced inbox files older than the grace period. |
| data-integrity-3-04 | minor | **Fixed** | `mcp_control.rs:604-610`: `risk_label` / `injection_risk` use `CASE WHEN risk_overridden = 1 …` in the UPDATE. `review_findings.rs:367-375`: `state = CASE WHEN state IN ('resolved','declined') THEN 'regressed' ELSE state END`. |
| data-integrity-3-05 | minor | **Fixed** | `otto-rbac/src/tokens.rs:1077-1114`: one transaction, with `DELETE … RETURNING` as the guard first, then insert on the same connection, then commit. A second concurrent rotate finds no row and mints nothing. |
| data-integrity-3-06 | nit | **Fixed** | `otto-review/src/engine.rs:1694-1721`: the file is read as bytes and only the missing lines are appended with `OpenOptions::append`. |
| data-integrity-3-07 | nit | **Fixed** | `settings.rs:169-197` `put_many` writes in one transaction. `routes/settings.rs:56` runs the reloads only after it succeeds. |
| data-integrity-3-08 | nit | **Fixed** | `db_explorer.rs:424-451` and `:555-591` use single `COALESCE` / `CASE` UPDATEs. `k8s_clusters.rs:257-296` uses `json_patch` for `params_json`. |

All eight iteration-3 findings are fixed. Nothing was rejected.

## Areas covered

- **Re-verification of every iteration-3 finding** (table above), plus migrations **0189** and **0190**. Both are nullable `ADD COLUMN`s. The previous build ignores them: it never reads `updated_by` or `prev_state`, and its own forget/undo/trigger edits leave them alone. No compatibility issue.
- **Swarm repo sub-paths** (carried over from iteration 3):
  - `swarm.rs` `update_task`, `update_run`, `update_run_if_status`, `claim_task`, `set_task_status_if`, `fail_running`, `stop_active_runs*`, `pending_task_results`, `finish_task_routing`, `delete_project`, `clear_project_board`, `delete_task`.
  - The full-row read-modify-writes run under the per-swarm `operation_guard`: `otto-swarm/src/service.rs:370-372` for operator edits, `engine.rs:982` for `route_result`, and the scheduler and verify recovery. I could not show a lost update across that gate, so I report none.
  - `finish_task_routing` patches `result_json` with `json_set`, so a concurrent usage backfill is not overwritten.
  - One orphan class: finding 4-06.
- **Workflow engine node-level persistence** (carried over):
  - `save_checkpoint` writes the checkpoint and its projection in one transaction. `prepare_retry` requires a terminal status in its own UPDATE guard.
  - Lifecycle writes go through `update_run_if` (a status guard in the WHERE clause). `request_cancel` never touches `nodes_json`.
  - The retry route refuses while `driver_alive`, and `spawn_run` registers its `DriverGuard` before the task starts, so two executors never write one run.
  - Trigger edits (`workflow_triggers.rs:225-262`) run under `BEGIN IMMEDIATE`, and `claim_scheduled_run` claims under a generation and cursor CAS.

  No issue.
- **DB Explorer saved queries and history** (carried over): `create/update/delete_saved` and `add_history`, plus their routes and the connection delete path. Finding 4-02.
- **Brokers ops repo and replay evidence** (carried over): `broker_ops.rs`, `otto-brokers/src/replay.rs`, the replay route. Finding 4-05.
- **Channels bridge state** (carried over):
  - Slack dedup is an in-memory window per bot token. Socket Mode does not redeliver acknowledged envelopes, and edits are keyed on the original ts plus `edited.ts`.
  - The Telegram offset is kept in process memory and confirmed by the next `getUpdates`. At worst, the window between dispatch and the next poll is redelivered once after a crash.

  No data-integrity issue.
- **Desktop shell state files** (carried over): `windows.rs:74-75`, `popout.rs:127-128` and `shortcuts.rs:119-121` all write a temp file and rename it. `supervisor.rs:149-161` copies the sidecar and renames it into place. No issue.
- **Proof-pack archive vs retention** (carried over): 0164 only stamps `archived_at`, and triggers un-archive the pack on any change or new artifact. Retention never deletes `proof_packs`. No issue.
- **Design Hall**: the content-addressed blob store (`otto-design/src/blobs.rs`, fsync, rename, directory fsync, a GC fence with a reference guard) and the scratch writes in design-assist. No issue.
- **Goal loops**:
  - `finalize`, `pause_running` and `mark_running` are conditional single statements.
  - `edit_ledger` runs inside `BEGIN IMMEDIATE`, and the human ledger routes require `require_idle`, so they can't race the controller's `set_ledger`.
  - Boot `fail_running` runs one transaction per loop.

  No issue.
- **Scheduled tasks**: `update_with_next_run` is IMMEDIATE, with COALESCE plus the generation. `settle_generation` is fenced by the generation. No issue.
- **Product stories and analyses**: `update_story` is a full-row RMW between two statements a few microseconds apart. Background runners write only `stage`. The window is very small, so I did not file it.
- **Session lifecycle file artefacts**: `SessionManager::remove`, the transcript image inbox, MCP env and creds files. Finding 4-03.
- **User-level skill install/uninstall into provider skill dirs** (`otto-context/src/user_skills.rs`, `merge.rs` manifest, `otto-skills/src/http.rs`). Finding 4-04.
- **Git stash identity journal and recovery** (`otto-git/src/stash_identity.rs`): the journal is fsynced before the ref rewrite. No issue.
- **Canvas Ask-AI commit** (`otto-canvas/src/assist.rs:294-350`): optimistic `expect_updated_at`, and a conflict keeps the user's manual edits. No issue.
- **Sweep of multi-autocommit `pub async fn`s in otto-state**: 22 hits, each triaged. The remaining ones are idempotent child-first deletes or touches; the one real gap is 4-06.

## Areas NOT covered yet (for iteration 5)

- `otto-state/src/notifications.rs` `mark_read` / `dismiss` / `clear`, which run two autocommits each. They're likely idempotent, but I did not read them line by line.
- `otto-state/src/activity.rs` `upsert_task` and `workgraph.rs` `append_event`: two-statement writes, only skimmed.
- `otto-product` refinement, discovery chat and mockup repos (`product_refinement.rs`, `product_chat.rs` `add_message`, `product_mockup.rs`): full-row RMWs and their background writers.
- `otto-connections/src/transfers.rs` and `otto-ssh/src/sftp.rs`: whether a download or upload overwrites a user's local or remote file in place (temp + rename vs truncate).
- `otto-server/src/routes/backup_git.rs` sync into the user's backup repo: partial-commit and crash behaviour.
- `resource_access` policies and grants left behind when a connection, cluster or AWS account is deleted. This is the same class as 4-02 and should be checked across resource kinds.
- The UI `ui/src/lib/stores/*` caches against server truth after a WS reconnect, for modules other than the API client and DB Explorer.

---

## Findings

### data-integrity-4-01 — minor — Memory undo writes without re-checking the token, so a double Undo restores a pending memory as `accepted`

- **File:** `crates/otto-state/src/memory.rs:987-1023` (`undo_forget_scoped`)
- **Evidence:**
  - **Lookup and write are separate.** The function finds the row with `SELECT id FROM memories WHERE undo_token=? AND … forgotten_at IS NOT NULL …` (:996-1008). It then runs a separate UPDATE whose WHERE clause is only `id=? AND workspace_id=?` (:1011-1022). The UPDATE does not re-check `undo_token` or `forgotten_at`.
  - **The UPDATE consumes `prev_state`.** It sets `state = COALESCE(prev_state,'accepted'), prev_state = NULL`.
  - **Failure scenario:** two concurrent undo requests with the same token, for example a double click on the Undo chip, or the chip and the toast.
    - Both SELECTs find the row.
    - The first UPDATE restores `suggested` and nulls `prev_state`.
    - The second UPDATE then computes `COALESCE(NULL,'accepted')` and writes `state='accepted', active=1`.
  - **Result:** the pending memory joins the accepted set with no human approval. This is the exact outcome 3-01 fixed, reached through a race instead.
  - **A second case:** an undo whose SELECT ran before a re-forget would clear the new forget's token and timestamp.
- **Fix:** Put the guard in the write. Add `AND undo_token = ? AND forgotten_at IS NOT NULL` to the UPDATE, use `RETURNING id`, and return NotFound ("already used") when no row comes back. Alternatively, fold the SELECT into the UPDATE: `UPDATE … WHERE undo_token=? AND workspace_id=? AND forgotten_at >= ? AND (? IS NULL OR id=?) RETURNING id`. Add a test: forget a `suggested` memory, call undo twice, and check the second call fails and `state` stays `suggested`.

### data-integrity-4-02 — minor — Deleting a DB connection strands its saved queries and widgets as undeletable rows, and leaves its query history behind for good

- **Files:**
  - `crates/otto-connections/src/service.rs:646-657` (`delete`)
  - `crates/otto-state/src/connections.rs:254-261`
  - `crates/otto-dbviewer/src/http.rs:1321-1338` (`delete_saved`), `:1297-1319` (`update_saved`), `:1466-1495` (`update_widget`), `:1497-1514` (`delete_widget`)
  - `crates/otto-dbviewer/src/service.rs:769-786` (`authorize`)
  - `crates/otto-state/migrations/0021_db_explorer.sql:3-53`
- **Evidence:**
  - **Only the profile row goes.** Connection delete runs `DELETE FROM connections WHERE id = ?` and removes the secret. `db_saved_queries.connection_id`, `db_widgets.connection_id` and `db_query_history.connection_id` have no FK (0021), and no code anywhere deletes or nulls them. A grep for `db_query_history|db_saved_queries|db_widgets` outside `db_explorer.rs` finds only migrations and the state archive.
  - **The rows stay listed.** `list_saved` and `list_widgets` filter by workspace, so the rows are still returned.
  - **They can't be removed.** Deleting or editing one first calls `ctx.db().authorize(&connection_id, …)`. That runs `self.connections.get(conn_id).await?`, which is `fetch_one`, so `RowNotFound` becomes `Error::NotFound("connection")` (`convert.rs:26`). Every saved query and widget of the deleted connection therefore answers 404 to Delete and Edit, for every user including root. The only way to remove such a widget is to delete its whole dashboard.
  - **History is never pruned.** `db_query_history` keeps every executed statement for the deleted connection. Nothing prunes it, and the retention module only touches `notifications`, `otto_runs`, `swarm_runs` and similar.
- **Fix:**
  - In `ConnectionsService::delete`, inside one transaction with the connection delete:
    - delete the connection's `db_widgets` and `db_query_history`;
    - delete its `db_saved_queries`, or set their `connection_id = NULL`, since that column is nullable and the routes already skip `authorize` when it is `None`.
  - Defensively, have the saved-query and widget delete routes treat a NotFound connection as "no connection gate" for **delete only**, keeping the workspace role and ownership checks.
  - Optionally add a one-off data cleanup of rows whose `connection_id` no longer exists. It is rollback-safe, since it drops no column.

### data-integrity-4-03 — minor — Images pasted into a session are never deleted, not when the session is removed and not when channel sessions are purged

- **Files:**
  - `crates/otto-server/src/routes/transcript.rs:1000-1070` (`inbox_upload`: writes `<data>/sessions/<id>/inbox/<ulid>-<stem>.<ext>`, up to 10 MB each)
  - `crates/otto-sessions/src/manager.rs:5602-5640` (`remove`), `:5560-5580` (archived channel-session purge)
- **Evidence:**
  - **Uploads land on disk.** The composer's image paste stores the decoded bytes under the data dir and returns the path.
  - **Removal stops at memory and the DB.** `SessionManager::remove` kills the PTY, deletes the DB row, clears in-memory maps, revokes the MCP token, and removes the codex env and creds files. It never touches `<data>/sessions/<id>/`.
  - **Nothing else cleans the folder.** A grep for `"sessions"` joined onto `data_dir` finds only the upload site. No housekeeping (`ottod/src/housekeeping.rs` sweeps only `bin/ottod.resigned.*`, orphan `-shm`/`-wal` and a 0-byte `state.db`) or retention job ever removes it.
  - **Result:** every deleted or auto-purged session leaks its pasted screenshots, which may hold sensitive content, with no row referencing them. This is the same class as 3-03, which was fixed for assistant attachments only.
- **Fix:**
  - In `remove`, after the row delete, `remove_dir_all(<data>/sessions/<id>)`. Confine it lexically to the `sessions/` root and ignore NotFound.
  - Add a boot or periodic sweep that removes `<data>/sessions/<dir>` entries older than a grace period whose id has no `sessions` row. This is the same approach as `sweep_orphan_inbox`.

### data-integrity-4-04 — minor — The ownership manifest in the user's skill dirs is updated read-modify-write with no lock and a non-atomic write, so Otto-installed skills can become "user-owned" and stuck

- **Files:**
  - `crates/otto-context/src/merge.rs:52-72` (`read_manifest` / `write_manifest`)
  - `crates/otto-context/src/user_skills.rs:97-160` (`install_into_dirs` / `uninstall_from_dirs`)
  - Callers: `crates/otto-skills/src/http.rs:328`, `:429`; `crates/otto-context/src/http.rs:212`
- **Evidence:**
  - **The manifest decides ownership.** `.otto-managed.json` in `~/.claude/skills`, `$CODEX_HOME/skills` and `~/.gemini/skills` is the only record of which skill dirs Otto owns.
    - Install refuses to overwrite a dir that is not in the manifest ("the user's own skill — never clobber it", `user_skills.rs:114-117`).
    - Uninstall skips a dir that is not in the manifest (`:148-150`).
  - **Non-atomic write.** `write_manifest` uses `fs::write`, which truncates and then writes. `read_manifest` turns a missing or malformed file into an empty list (`merge.rs:52-60`). A crash or full disk mid-write therefore empties the manifest, and every Otto-managed skill in that dir becomes "user-owned" for good.
  - **Unsynchronized RMW.** Each install or uninstall does read, push or retain, then write, with no lock. Two concurrent install or uninstall requests on the same dir, or a bulk install racing a single one, lose an entry. A grep finds no Mutex in `otto-skills/src/http.rs`.
  - **Result:** updates report the skill as `user_owned` and skip it, so a stale global skill is served to every CLI session. Uninstall leaves the dir in place.
- **Fix:**
  - Write the manifest with the temp-file, `sync_all`, rename pattern already used for `library.rs` `write_atomic`.
  - Serialize manifest edits per skills dir, either with a process-wide `KeyedLocks<PathBuf>` around the read-modify-write in `install_into_dirs` / `uninstall_from_dirs`, or an `flock` on the manifest.
  - Optionally, when the manifest is malformed (as opposed to missing), refuse to treat entries as user-owned and log it, instead of silently degrading to "nothing is ours".

### data-integrity-4-05 — minor — A Kafka replay records its evidence and audit only after the whole publish loop, so an interrupted replay leaves no trace of the messages it already sent

- **Files:**
  - `crates/otto-brokers/src/replay.rs:11-98` (`publish`)
  - `crates/otto-brokers/src/service.rs:1051-1087` (`replay`)
  - `crates/otto-brokers/src/http.rs:561-586` (route)
- **Evidence:**
  - **Nothing is persisted during the loop.** `publish` produces each message in turn (up to `MAX_CONSUME` = 5000, `service.rs:27`) and only collects evidence in memory (:24-64). `record_replay` (the `broker_replays` row) runs only after the loop (:66-91). The route writes `audit_write("replay", …)` only after `replay` returns (:570-584).
  - **An interrupted request records nothing.** If the request stops mid-loop, nothing is persisted: the daemon is restarted (deploy, crash, quit) or the handler future is dropped. Neither an evidence row nor an audit entry exists, yet the acknowledged messages are already on the target topic. This can be a prod cluster, since the route only requires `confirm`.
  - **The tool's own warning can't be acted on.** The code tells the user that "retrying can duplicate messages" (:58, :84). For an interrupted replay, the user has no record of how far it got.
- **Fix:**
  - Persist intent before the first send:
    - Insert the `broker_replays` row up front with `status='running'` and an empty evidence list. This needs an additive nullable `status` column with a DEFAULT, which is rollback-safe.
    - Write an `audit_write("replay.start", …)` entry.
  - Update the evidence incrementally, for example every N acknowledgements and at the end, and set the final status.
  - On boot, mark any `running` replay rows as `interrupted`.
  - Optionally run the publish loop in a spawned task, so a client disconnect does not cut it short.

### data-integrity-4-06 — nit — Deleting a swarm task or clearing a project board leaves the task-scoped goals behind

- **File:** `crates/otto-state/src/swarm.rs:1468-1475` (`delete_task`), `:1247-1262` (`clear_project_board`), compared with `:1206-1242` (`delete_project`)
- **Evidence:**
  - **`delete_project` cleans up goals.** It runs `DELETE FROM swarm_goals WHERE project_id = ? OR task_id IN (SELECT id FROM swarm_tasks WHERE project_id = ?)` in its transaction.
  - **The other two don't.** `delete_task` runs only `DELETE FROM swarm_tasks WHERE id = ?`. `clear_project_board` deletes tasks and messages, in two autocommits, but no goals.
  - **No FK cascades.** `swarm_goals.task_id` has no FK (0079), so the task's goals and their `verdict_json` are orphaned for good. Nothing lists them afterwards, since `list_goals_for_task` is keyed by the deleted id and `list_goals_for_project` filters `task_id IS NULL`, and nothing deletes them until the whole project or swarm goes.
- **Fix:** Delete `swarm_goals WHERE task_id = ?` in the same transaction as `delete_task`. In `clear_project_board`, delete `swarm_goals WHERE task_id IN (SELECT id FROM swarm_tasks WHERE project_id = ?)`, then the tasks, then the messages, all in one `begin()` transaction, mirroring `delete_project`.
