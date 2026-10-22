# Data-integrity review — quality iteration 5 (2026-10-22)

Lens: persistence integrity. Covers otto-state SQLite and its migrations (append-only and rollback
compatibility), transactions, partial writes, non-atomic file writes, data loss on crash or restart,
caches that disagree with their source of truth, and id/ownership consistency.

Branch `fix/quality-iter5-20261022` @ 8177c4d85. This was a read-only review: no builds, tests or
e2e runs.

## Score

Rubric: start at 10.0. Each open finding subtracts: blocker 1.0, major 0.3, minor 0.1, nit 0.03.

| Severity | Count | Deduction |
|---|---|---|
| blocker | 0 | 0.00 |
| major | 1 | 0.30 |
| minor | 2 | 0.20 |
| nit | 2 | 0.06 |

**Score = 10.0 − 0.30 − 0.20 − 0.06 = 9.44**

## Previous findings (iteration 4)

| Id | Severity | Status | Evidence |
|---|---|---|---|
| data-integrity-4-01 | minor | **Fixed** | `otto-state/src/memory.rs:961-978`: undo is now a single `UPDATE … WHERE undo_token=? AND workspace_id=? AND forgotten_at IS NOT NULL AND forgotten_at >= ? AND (? IS NULL OR id=?) RETURNING id`. A second undo matches no row and returns NotFound. It also restores `prev_active` (migration 0192). |
| data-integrity-4-02 | minor | **Fixed** | `otto-state/src/connections.rs:260-290`: one transaction deletes `db_widgets` and `db_query_history`, nulls `db_saved_queries.connection_id`, then deletes the connection. `resource_access` rows go through the 0119 delete triggers. |
| data-integrity-4-03 | minor | **Fixed** | `otto-sessions/src/manager.rs:5493` calls `remove_session_files` after the row delete. `manager/session_files.rs` keeps the delete to one path component, unlinks a symlink without following it, and `sweep_orphan_session_files` (wired in `boot/tasks.rs:249`) removes folders that have no row once the grace period has passed. Nothing else writes under `<data>/sessions/`: the codex, grok and context `sessions` roots are other directories. |
| data-integrity-4-04 | minor | **Fixed** | `otto-context/src/merge.rs`: `write_manifest` writes a temp file, calls `sync_all`, then renames. `read_manifest_checked` refuses a malformed manifest. `user_skills.rs:95-171` holds a process-wide `MANIFEST_EDITS` lock around each read-modify-write. The only other writer (`materialize.rs:722`) targets the per-bundle shadow dir, not the user dirs. |
| data-integrity-4-05 | minor | **Fixed** (residual filed as 5-04) | Migration 0193 adds `status` with a DEFAULT, so it is rollback-safe. `replay.rs:39-63` inserts a `running` row before the first send and refuses to publish if that insert fails. Evidence is checkpointed every 100 acks, and the boot sweep (`service.rs:154-171`) marks leftover `running` rows `interrupted`. A replay dropped while the daemon stays up is not covered: see 5-04. |
| data-integrity-4-06 | nit | **Fixed** | `otto-state/src/swarm.rs:1485-1505` (`delete_task`) and `:1247-1278` (`clear_project_board`) delete the goals, tasks and messages in one transaction. |

All six iteration-4 findings are fixed, and none was rejected. 5-04 is a narrower gap left over from
4-05's fix.

## Areas covered

- **Re-verification of every iteration-4 finding** (table above).
- **Migrations 0191–0194:**
  - 0191 and 0192 are nullable `ADD COLUMN`s.
  - 0193 is `NOT NULL DEFAULT 'done'`, so an older binary that never names the column keeps inserting.
  - 0194 is a new table plus `INSERT OR IGNORE` seeds.

  All four are rollback-compatible.
- **MCP profiles repo** (`otto-state/src/mcp_profiles.rs`, new in this window):
  - The partial unique index allows at most one default. The default switch and the delete-the-default path each run in one transaction.
  - The locked check runs outside the transaction, but `locked` is immutable.
  - A delete racing `set_default` can leave zero defaults. `default_profile()` explicitly falls back to `full`, so the code already handles it.
  - Sessions that name a deleted profile resolve to the default. This is documented and fails open, so it belongs to the access-control lens, not this one.
- **Rooms recap archive restart path** (`rooms/recap/archive.rs`):
  - Metadata, images and drafts are written with temp file, `sync_data` and rename.
  - The journal is append-only. `ensure_index` stops at the first torn or out-of-sequence line and surfaces a recovery gap instead of failing.
  - `Store::open` forces every archive to `Stopped`, so a recovered journal is never appended to.

  No issue.
- **Room games and the classroom:** server state is in memory only (`game_rooms/registry.rs`). The classroom and Home layouts are per-device `localStorage` preferences. Nothing is persisted that could be corrupted.
- **Notifications** (iteration-4 carry-over): `mark_read`, `dismiss`, `clear`, `mark_all_read` and `*_many` are each a single statement. No issue.
- **Activity `upsert_task` and workgraph `append_event`** (carry-over):
  - `append_event` only touches `last_event_at` in its second statement, which is harmless if that statement fails.
  - `upsert_task` is check-then-insert with no unique `(session_id, ext_id)`, but the hook ingest is sequential per tool call. I could not show a realistic duplicate, so I filed nothing.
- **Product refinement, discovery chat and mockup repos** (carry-over): `add_message` inserts the message and then bumps `updated_at` on a best-effort basis. Mockup `update` is a read-modify-write a few microseconds wide; I did not file it, consistent with how iteration 4 treated `update_story`.
- **SFTP transfers and S3 downloads** (carry-over):
  - Downloads stage to `.otto-transfer-<id>.part` and publish with `hard_link`, which never overwrites. A download refuses an existing destination.
  - Uploads stage to a remote `.part` and use SFTP `rename`.
  - S3 downloads use an `O_EXCL` part file and publish without overwriting.

  No issue.
- **Backup-to-git sync** (`routes/backup_git.rs`, carry-over):
  - Each published file's expected hash is re-checked.
  - The manifest is written last.
  - `safe_path` refuses symlinks.
  - The commit is `--only` with a NUL pathspec file.
  - Pull and push check HEAD first.

  No issue.
- **`resource_access` orphans** (carry-over): 0119 has `AFTER DELETE` triggers for connections, MCP servers, AWS accounts and k8s clusters. No issue.
- **UI store caches after a WS reconnect** (carry-over): I read `events.svelte.ts` `resyncStores` and every `peek()?.applyEvent` route in `lib/events/routes/*.ts`, and compared them with each store's or page's `liveQuery`/`onResync` registration. Finding 5-02.
- **Files Otto writes into user repositories:**
  - Conflict resolution (`write_resolution` uses `confined_worktree_file`), discard and `commit_only` are fine.
  - The API-collections push is not: finding 5-01.
- **User-config files:** `~/.claude.json` trust grants (`otto-sessions/src/trust.rs`: a process lock plus temp file and rename) and k8s imported kubeconfigs (fresh id, deletion confined to `<data>/kube`). No issue.
- **Dangling references on delete:**
  - Workflow delete versus scheduled tasks: finding 5-03.
  - Repo unregister versus `pr_reviews`: finding 5-05.
  - Projects and users are never hard-deleted.
- **`remove_dir_all` sweep in production code:** skills backups prune, product story delete (confined joins), scheduled-task worktrees, housekeeping. No issue.
- **Workspace settings:** `patch_settings` uses `json_patch`. A full `settings` update carries `$.context` across inside one UPDATE. No issue.

## Areas NOT covered yet (for iteration 6)

- `otto-git` `checkout_autostash`, `pull_autostash_mode` and stash pop conflict paths, end to end: does an autostash that fails to re-apply always leave the stash entry in place?
- `otto-skills` `install_into` (`lib.rs:221-235`): it removes the Library copy and then seeds it. A crash mid-seed leaves a partial skill, and I have not confirmed that a backup is always taken first.
- `otto-insights` report writes and `otto-improve/src/evidence.rs`: derived data, only skimmed.
- `otto-channels` thread/session mapping rows when a session or channel integration is deleted.
- Personal-agent rooms (`agent_room_message`) retention and room deletion cascades.
- `otto-automation/src/goal_loop_workspace.rs` worktree file writes.
- State-archive restore with tables excluded from archives (`mcp_profiles`): the restore-then-reference consistency of `sessions.meta.mcp_profiles`.

---

## Findings

### data-integrity-5-01 — major — Pushing API collections writes through repo-controlled symlinks, so a file outside the repository can be overwritten

- **File:** `crates/otto-git/src/http.rs:2128-2160` (`repo_collections_push`); also `:2079-2105` (`repo_collections_pull`)
- **Evidence:**
  - **Writes go to the raw path.** The push handler builds `base.join("collections/<name>.json")` and writes each file with `tokio::fs::write(base.join(&rel), &f.content)` (`:2156`). Before that it calls `tokio::fs::create_dir_all(base.join("collections"))` (`:2146`). Neither checks for symlinks. The only sanitisation is `f.name.replace(['/', '\\'], "_")`.
  - **`tokio::fs::write` follows symlinks.** Two repo layouts redirect the write:
    - If `collections/<Name>.postman_collection.json` is a symlink committed in the repo, the content overwrites the symlink's target.
    - If `collections` itself is a symlink, new files land in that directory.
  - **The file names are predictable.** The UI pushes `<collection name>.postman_collection.json` (`ui/src/lib/stores/apiClient.svelte.ts:1252`). The normal flow is pull first: the pull reads `collections/*.json` with `read_to_string`, which also follows the link (`:2095-2103`), and imports the collection under that name. Push then writes back to the same name, and so to the link's target.
  - **Failure scenario:** a shared API-collections repo contains `collections/Payments.postman_collection.json -> ../../../.zshrc`, or `-> ~/.ssh/config`. The user pulls, edits and pushes. The target file is replaced with Postman JSON, outside the repository. `git add` then stages only the unchanged symlink, so nothing even shows up in the commit.
  - **The safe helper exists but isn't used.** `GitLocal::confined_worktree_file` (`crates/otto-git/src/local.rs:4869-…`) canonicalizes the parent, keeps it inside the work tree and out of `$GIT_DIR`, and refuses a final-component symlink. `write_resolution` uses it; the collections push does not.
  - **Writes are not atomic.** If a later file's write fails, the earlier files are already overwritten in the work tree with nothing committed. When `branch` was given, the repo is also left switched to the new branch (`:2137-2139`).
- **Fix:**
  - Resolve every target through `confined_worktree_file(&rel, true)`. It refuses a symlinked `collections` dir or file, and anything outside the work tree. Do the same for the pull's reads, opening with `O_NOFOLLOW`, or skip symlink entries via `symlink_metadata`.
  - Write each file to a sibling temp file and rename it into place.
  - Validate all names and targets before writing any of them, so a refusal or an I/O error leaves the work tree untouched.
  - Add a test: a repo whose `collections/X.postman_collection.json` is a symlink to a file in a temp dir. The push must fail and the target must stay byte-identical.

### data-integrity-5-02 — minor — The Scheduled Tasks and Run with Otto pages are only updated by events, so after a reconnect or daemon restart they keep showing runs that are no longer running

- **Files:**
  - `ui/src/lib/events.svelte.ts:648-676` (`resyncStores`)
  - `ui/src/lib/stores/scheduledTasks.svelte.ts:108-117` (`applyEvent`)
  - `ui/src/lib/stores/runWithOtto.svelte.ts:164-170` (`applyEvent`)
  - `ui/src/modules/scheduled-tasks/ScheduledTasksPage.svelte:242-248`
  - `ui/src/modules/run-with-otto/RunWithOttoPage.svelte:40`
- **Evidence:**
  - **Updates come only from events.** Both stores refresh their list and runs only when a `scheduled_task_run_updated` or `otto_run_updated` event arrives (routed by `peek()` in `lib/events/routes/automation.ts`), or when the page's workspace `$effect` runs.
  - **The reconnect path skips them.** `resyncStores()` re-reads sessions, workflow runs, notifications, activity, swarm, transcripts, the Mission Control and design buses, assistant and rooms. It calls nothing on `scheduledTasks` or `runWithOtto`.
  - **No resync hook exists.** Neither store nor its page registers a `liveQuery` or `liveEvents.onResync`; a grep of `ui/src/modules/scheduled-tasks` and `ui/src/modules/run-with-otto` finds none. Contrast `loops.svelte.ts:269`, which uses `liveQuery` and is resynced.
  - **Failure scenario:** a scheduled task is running when the daemon restarts. The boot reaper records the run as interrupted (`scheduled_tasks_engine.rs:1373-1392`) while no client is connected, so the event goes nowhere. After reconnect, the open page still shows the row and its runs as `running` until another event for that workspace happens to arrive, or until the user navigates away and back. Run with Otto behaves the same way for a run that finished or failed during the gap.
- **Fix:** In `resyncStores()`, add the two stores:
  - `scheduledTasksStore.peek()?.resync()`, where `resync()` reloads `loadList(wsId)` and the runs of each expanded task;
  - `runWithOttoStore.peek()?.resync()`, where `resync()` reloads the list, the open run and its events.

  Alternatively, register both pages' loaders with `liveEvents.onResync`. Add both stores to the unit test that pins the resync set.

### data-integrity-5-03 — minor — Deleting a workflow leaves the scheduled tasks that target it enabled, and every occurrence then fails

- **Files:**
  - `crates/otto-server/src/routes/workflows.rs:210-219` (`delete_workflow`)
  - `crates/otto-state/src/workflows.rs:443-470` (`delete`)
  - `crates/otto-automation/src/scheduled_tasks_engine.rs:1317-1324` (`execute_workflow`)
  - `crates/otto-server/src/routes/scheduled_tasks.rs:383-393` (`validate_workflow`)
  - `crates/otto-state/migrations/0086_scheduled_tasks_v2.sql:13`
- **Evidence:**
  - **Only real FKs cascade.** `scheduled_tasks.workflow_id` is a plain `TEXT` column (0086), unlike `workflow_triggers.workflow_id`, which has `REFERENCES workflows(id) ON DELETE CASCADE`.
  - **Delete checks only for active runs.** The workflow delete refuses only while a run is pending or running. It does not look at scheduled tasks, and the route does nothing else.
  - **Every later occurrence fails.** `execute_workflow` does `repo.get(wf_id).await?`. The NotFound becomes a failed run. Nothing auto-disables a task after repeated failures (a grep for `consecutive`, `auto-disable` and `failure_count` in otto-automation finds none).
  - **The reference is validated only on write.** Create and update reject an unknown `workflow_id` (`validate_workflow`, "workflow_id not found"), so the task can't be pointed at a missing workflow by an edit, but it is left dangling by a delete.
  - **Result:** each occurrence appends a failed run row, and runs its failure delivery to the destination, for as long as the task stays enabled.
- **Fix:** In `WorkflowsRepo::delete`, inside the same statement guard (or one transaction), set `enabled = 0` on `scheduled_tasks WHERE workflow_id = ?` and record why, for example in a `last_error` or report note. Alternatively, refuse the delete with 409 and name the tasks that still target it. Have `execute_workflow` map a NotFound workflow to a single "workflow deleted — task paused" outcome that disables the task, instead of failing again on every tick.

### data-integrity-5-04 — nit — A Kafka replay cut short while the daemon stays up stays `running` until the next restart, and loses up to 99 acknowledgements of evidence

- **Files:**
  - `crates/otto-brokers/src/http.rs:558-583` (route)
  - `crates/otto-brokers/src/replay.rs:107-124` (checkpoint every `CHECKPOINT_EVERY` = 100)
  - `crates/otto-brokers/src/service.rs:154-171` (`with_ops`: the `interrupted` sweep runs only at boot)
- **Evidence:**
  - **The publish loop runs inline in the request.** It sits inside the axum handler future (`ctx.brokers().replay(...)`), up to 5,000 messages, and is not spawned. When the client disconnects (window closed, page reloaded, webview navigation), hyper drops the handler future mid-loop.
  - **Nothing records the drop.** The final `update_replay(..., "done"|"failed")` (`replay.rs:126-141`) and the route's closing `audit_write("replay", …)` never run.
  - **The row stays stale.** It keeps `status='running'`, and its evidence reflects only the last multiple-of-100 checkpoint. Only the next daemon start rewrites the status (`mark_interrupted_replays`), and acknowledged messages after the last checkpoint are never recorded.
  - Iteration 4's fix closed the daemon-stop case; this is the in-process cancellation case of the same gap.
- **Fix:** Run `publish` in a `tokio::spawn` inside `BrokersService::replay` and await its `JoinHandle`. A dropped request then no longer cancels the loop, and the final status and audit entry always land. Alternatively, wrap the loop in a drop guard that writes the current evidence with status `interrupted`.

### data-integrity-5-05 — nit — Unregistering a repo leaves its PR reviews and their comments behind, invisible and never pruned

- **Files:**
  - `crates/otto-state/src/git.rs:338-345` (`delete_repo`)
  - `crates/otto-git/src/http.rs:1525-1538` (route)
  - `crates/otto-state/migrations/0006_pr_reviews.sql:3`
  - `crates/otto-state/src/reviews.rs:365-368`
  - `crates/otto-state/src/retention.rs:375-381`
- **Evidence:**
  - **`pr_reviews.repo_id` has no FK** (0006). `delete_repo` runs only `DELETE FROM repos WHERE id = ?`.
  - **Some related rows go, the reviews stay.** `review_findings` rows cascade away through their FK (0082), but the `pr_reviews` rows, their `pr_review_comments` and the `agents_json` with the reviewers' full output remain.
  - **They are invisible.** Workspace review lists join on `repos` (`recent_heads_in_workspace`: `JOIN repos gr ON gr.id = r.repo_id`), so the rows can no longer be seen or deleted from the UI.
  - **They are never pruned.** Retention prunes only the review retry-artifact tables, never `pr_reviews` itself.
  - **Re-registering doesn't bring them back.** The same path gets a new repo id, so its review history does not reappear.
  - This is the same orphan class as 4-02 and 4-06.
- **Fix:** In `delete_repo`, inside one transaction, delete `pr_reviews WHERE repo_id = ?`. Comments cascade through their FK. Alternatively, keep the reviews but expose them by `repo_id` so they can be cleaned up. Optionally add a one-off, rollback-safe cleanup of `pr_reviews` whose `repo_id` has no `repos` row.
