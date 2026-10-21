# Data-integrity review — quality iteration 1 (2026-10-21)

Lens: persistence integrity. Covers otto-state SQLite and its migrations (the append-only and rollback-compatibility rules), transactions, partial writes, non-atomic file writes, data loss on crash or restart, caches that disagree with the DB, and id/ownership consistency.
Branch `fix/quality-loop-20261021` @ d447516c4. Read-only review: I ran no builds or tests.

## Score

Rubric: start at 10.0. Each open finding subtracts: blocker 1.0, major 0.3, minor 0.1, nit 0.03.

| Severity | Count | Deduction |
|---|---|---|
| blocker | 0 | 0.00 |
| major | 1 | 0.30 |
| minor | 6 | 0.60 |
| nit | 0 | 0.00 |

**Score = 10.0 − 0.90 = 9.10**

## Previous findings

None. This is the first iteration of this loop for this lens. Earlier review folders (quality-20261007/08, app-20261005) have no `delete_swarm` / design-blob / improve-approve entries.

## Areas covered

- Migrations 0166–0187: append-only, rollback compatibility, and the CHECK widening in the 0185 `issue_accounts` rebuild (it keeps every column). Also checked the duplicate-version repair code in `db.rs`. No issues found.
- SQLite pool options (WAL, `synchronous=NORMAL`, `foreign_keys`, `busy_timeout`).
- A scripted sweep of every otto-state repo function that issues two or more autocommit writes without a transaction. I triaged each one: child-then-parent deletes that are safe to retry were dropped, and the real cases are listed below.
- Swarm deletion lifecycle (repo, service, routes, runtime liveness checks), checked against `clear_project_h` and `delete_task`.
- Canvas: transactional update/delete, GC of the content-addressed files, and the UI ref round-trip. One possible GC dangling-ref scenario was ruled out because `getFiles()` keeps every file.
- Vault `write_note` / `write_text_file` / `delete_note` / `rename`: atomic replace with fsync, NOREPLACE moves, and a trash manifest written before the move. No issues found.
- Trust config writes (`~/.claude.json`, `~/.codex/config.toml`): atomic with a fingerprint check. No issues found.
- Rooms recap archive (atomic plus `sync_data`). No issues found.
- State archive restore (one IMMEDIATE transaction, staged files, rollback of published files). No issues found.
- Retention of swarm runs, otto runs and goal-loop iterations (spend rollup in the same transaction). No issues found.
- Workspace settings (`patch_settings` json_patch, context preserved) and membership replacement (transactional). No issues found.
- Settings cache invalidation, improve auto-apply (atomic plus backup), design blob GC guards, design working-copy mirror. No issues found.
- Notifications de-duplication.

## Areas NOT covered yet (for iteration 2)

- `workflow_engine.rs` run-state persistence and crash recovery (37 commits in the last 3 weeks).
- `scheduled_tasks_engine.rs` and `personal_agents.rs` generation fencing at runtime. I read only the triggers in 0182/0187.
- The otto-sessions manager: whether session status/meta in the DB agrees with the in-memory registry across restart.
- otto-vault `watch.rs` / `recovery.rs` (large recent diffs), and the `.otto-history` revision commit ordering against the index.
- otto-product (run.rs, watcher), the otto-assistant task/thread lifecycle, and otto-channels persisted mappings.
- The otto-dbviewer `database_changes` attempt/approval state machine.
- `otto-git` `local.rs` merged-set memo (cache vs repository state).
- UI-side draft persistence (unsaved editor buffers, localStorage drafts).

---

## Findings

### data-integrity-1-01 — major — Deleting a swarm or a swarm project leaves its live agent runs working, and they then write into deleted rows

- **Files:**
  - `crates/otto-swarm/src/http.rs:226-235` (`delete_swarm` route)
  - `crates/otto-swarm/src/service.rs:161-163` (`delete_swarm`), `:314-318` (`delete_project`)
  - `crates/otto-state/src/swarm.rs:877-900`
  - `crates/otto-swarm/src/http.rs:316-326`
- **Evidence:**
  - **The delete path does nothing to stop live runs.** `SwarmService::delete_swarm` is just `self.repo.delete_swarm(id).await`. The repo then deletes `swarm_messages`, `swarm_runs`, `swarm_tasks`, `swarm_projects` and `swarm_agents`, and finally `swarms`. It never takes `operation_guard(swarm_id)`, never calls `run::signal_cancel`, and never calls `kill_session`.
  - **The runtime only checks liveness between attempts.** `agent_run.rs:294-301` and `run.rs:678-682` re-check the run row before each attempt. A turn that is already running keeps going. When it finishes, the engine's settle path (`engine.rs:955-1000`: `update_task`, `system_post` → `INSERT INTO swarm_messages`) writes against a swarm, task and run that no longer exist. That leaves orphaned `swarm_messages` rows, and the agent PTY keeps spending tokens.
  - **`delete_project` only flips status.** It calls `stop_active_runs_for_project`, which is a DB status flip, and never signals the cancel handle or kills the session. `clear_project_h` (`engine.rs:2609-2640`) documents exactly this bug ("the flag alone left it working (and burning tokens) in its worktree on a board that no longer exists"). It fixes it there with `operation_guard` + `signal_cancel` + `kill_session`. The delete paths never received that fix.
  - **`delete_task` does take the guard.** It runs under `operation_guard` (`service.rs:418-424`). Swarm and project deletion are the only lifecycle deletes without it.
- **Fix:** Route `delete_swarm` and `delete_project` through the same sequence as `clear_project_h`:
  1. Take `operation_guard(&swarm_id)`.
  2. Run `stop_active_runs_for_project`, or a new `stop_active_runs_for_swarm` for the whole swarm.
  3. For each stopped run: `signal_cancel(ctx.swarm_run_cancels(), rid)`, then `kill_session` on `run.session_id`.
  4. Then run the repo delete, still under the guard.

  Add a test: a running run, then delete the swarm. Assert the session was killed and that no `swarm_messages` row with that `swarm_id` remains afterwards.

### data-integrity-1-02 — minor — Swarm and project deletes are non-atomic and leave `swarm_goals` / `swarm_channel_triggers` orphaned

- **Files:** `crates/otto-state/src/swarm.rs:877-900` (`delete_swarm`), `:1194-1215` (`delete_project`)
- **Evidence:**
  - **Non-atomic.** `delete_swarm` runs 6 separate `execute(&self.pool)` autocommits, and `delete_project` runs 3. Neither uses a transaction. If any statement fails (for example SQLITE_BUSY after the 5 s `busy_timeout`), the delete stops partway: runs and tasks are gone but the swarm row stays. Canvas `delete` (`canvas.rs:474-520`) was already moved to one transaction for this exact reason.
  - **Orphans.** The child list in `delete_swarm` leaves out `swarm_goals` (migration 0079, has `swarm_id`) and `swarm_channel_triggers` (0081, has `swarm_id`). `delete_project` leaves out the project's and its tasks' `swarm_goals`. No FK cascades exist on these tables (0029/0079/0081 declare no REFERENCES), so those rows remain forever. The orphaned triggers are still read by every inbound channel message: `find_triggers` (`swarm.rs:2543-2558`) runs per message, and `channels.rs:291-294` skips them only after a failed `get_swarm` lookup.
- **Fix:** Do the whole cascade inside one `self.pool.begin()` transaction. Add `DELETE FROM swarm_goals WHERE swarm_id = ?` and `DELETE FROM swarm_channel_triggers WHERE swarm_id = ?` to `delete_swarm`. Add `DELETE FROM swarm_goals WHERE project_id = ?` to `delete_project`.

### data-integrity-1-03 — minor — Manually approving or rolling back a self-improvement edit rewrites the user's file in place, with no temp file and no backup

- **File:** `crates/otto-improve/src/engine.rs:719-727` (`approve_edit`), `:824-827` (`rollback_edit`)
- **Evidence:**
  - `approve_edit` calls `tokio::fs::write(&edit.target_path, &edit.after_content)`, which truncates and then writes the file in place. The targets are user-owned CLAUDE.md, memory and SKILL.md files. The rollback path does the same for `before`.
  - **A crash or a full disk mid-write leaves a truncated file.** The DB row was already flipped to `Applied` before the write (`decide(... Applied)` at :711-717). After a restart, `rollback_edit` sees `current != after_content`, marks the edit `Conflict`, and the original content survives only inside the DB row.
  - The auto-apply path in the same file (`safe_auto_apply`, :1300-1350) already does it correctly: it writes a timestamped `.bak`, then writes a same-directory temp file and renames it over the target.
- **Fix:** Factor `safe_auto_apply`'s steps (3)–(4) (backup, then temp + rename) into a helper, and use it in both `approve_edit` and `rollback_edit`.

### data-integrity-1-04 — minor — Multi-field PATCH repos write each column as its own autocommit, so a failure or a concurrent read sees a half-applied record

- **Files:**
  - `crates/otto-state/src/mcp_control.rs:436-512` (`McpServers::update`: name/command/url/env/headers/enabled as up to 11 statements)
  - `crates/otto-state/src/mcp_control.rs:928-992` (`mcp_policies` `update`: `enabled`, `priority`, `match_json`, `effect` as separate statements)
  - `crates/otto-state/src/users.rs:142-` (display_name / password_hash / disabled)
  - `crates/otto-state/src/aws_accounts.rs:159-`
  - `crates/otto-state/src/workspaces.rs:196-238`
  - `crates/otto-state/src/api_client.rs:511-540` (`variables_json` then `secret_keys_json`)
- **Evidence:**
  - Each `if let Some(v)` block runs `.execute(&self.pool)` on its own, with no transaction.
  - **On error,** the fields written so far stay committed while the API returns an error. For example, an env-var PATCH whose headers write fails, or an api_environments variable change whose `secret_keys_json` update fails, leaves a variable that should be secret marked as non-secret.
  - **On concurrent reads:** a reader on the WAL read pool can see a policy with the new `enabled=1` and new `match_json` but the old `effect` while the PATCH is mid-way.
- **Fix:** Wrap each of these `update` functions in one `self.pool.begin()` / `tx.commit()`. Or collapse them into a single `UPDATE … SET col = COALESCE(?, col), …`, as `canvas.rs:427-449` already does.

### data-integrity-1-05 — minor — A hard delete of a design project checks "empty" and deletes in two unguarded statements

- **Files:** `crates/otto-design/src/store.rs:692-713`; caller `crates/otto-design/src/http.rs:600-602`
- **Evidence:**
  - `delete_project` runs `SELECT COUNT(*) FROM design_artifacts WHERE project_id = ?` and then, as a separate autocommit, `DELETE FROM design_projects WHERE id = ?`.
  - **The lock doesn't help.** The hard-delete branch does not take `project_lock` (only the archive branch at :604 does). Artifact creation and re-filing (`service.rs:536-546`, `:1320-1335`) also validate the project with a plain `get_project` and no lock.
  - **Result:** an artifact filed into the project between the count and the delete ends up with a `project_id` that points at a deleted project. There is no FK to stop it (`0139_design_graph.sql:49` has only a comment). The artifact is then neither "unfiled" (NULL) nor in any project.
- **Fix:** Use a single conditional statement, `DELETE FROM design_projects WHERE id = ? AND NOT EXISTS (SELECT 1 FROM design_artifacts WHERE project_id = ?)`. Map 0 rows affected to Conflict or NotFound by re-checking which case applies.

### data-integrity-1-06 — minor — Design blobs are published without fsync, and an existing name is trusted forever, so a torn blob never heals

- **File:** `crates/otto-design/src/blobs.rs:71-91` (`put_hashed`), `:95-101` (`get`)
- **Evidence:**
  - `put_hashed` writes the temp file with `tokio::fs::write` and then calls `rename`. It never calls `sync_all`, either on the file before the rename or on the directory after.
  - **No self-healing on write.** On the next put, `try_exists(&dest)` short-circuits to "identical by definition", so a blob that was left empty or partial after a power loss or kernel panic is never rewritten.
  - **No verification on read.** `get` returns the bytes without checking their sha256.
  - **The blob is the source of truth.** `service.rs:800` says "the blob is the truth", and the version row referencing that sha is committed right after `put`.
  - Vault (`engine.rs:1209-1218`: `sync_all` plus a directory `fsync`) and rooms recap (`archive.rs:625-634`: `sync_data`) already do this correctly.
- **Fix:** In `put_hashed`, call `sync_all()` on the temp file before the rename and `fsync` the blob directory after it. When `dest` exists, treat a size mismatch (`metadata.len() != bytes.len()`) as corrupt and republish. Optionally verify the sha in `get_limited` for exports.

### data-integrity-1-07 — minor — Notification de-duplication is check-then-insert without a transaction or an upsert

- **File:** `crates/otto-state/src/notifications.rs:138-190`; index `crates/otto-state/migrations/0031_notifications_user.sql:15-23`
- **Evidence:**
  - `create` runs `SELECT id … WHERE source_key = ? AND user_id IS ?` and, if no row is found, a separate plain `INSERT`.
  - **Global notices (`user_id IS NULL`):** the 0031 comment says the unique index "does NOT enforce uniqueness" for them. Two concurrent `create` calls with the same `source_key` both insert, so the same notice appears twice.
  - **Per-user keys:** the second insert hits `idx_notifications_user_source_key`. The caller gets a "create notification" DB error instead of the documented refresh-in-place.
- **Fix:** Run the lookup and the insert in one `begin_with("BEGIN IMMEDIATE")` transaction. For per-user keys, an `INSERT … ON CONFLICT(user_id, source_key) WHERE source_key IS NOT NULL DO UPDATE SET …` also works.
