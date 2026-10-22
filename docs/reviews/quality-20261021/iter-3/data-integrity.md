# Data-integrity review — quality iteration 3 (2026-10-22)

Lens: persistence integrity. Covers otto-state SQLite and its migrations (append-only and rollback
compatibility), transactions, partial writes, non-atomic file writes, data loss on crash or restart,
caches that disagree with their source of truth, and id/ownership consistency.

Branch `fix/quality-loop-20261021` @ 3dd55d9f2. Read-only review: no builds, tests or e2e were run.

## Score

Rubric: start at 10.0. Each open finding subtracts: blocker 1.0, major 0.3, minor 0.1, nit 0.03.

| Severity | Count | Deduction |
|---|---|---|
| blocker | 0 | 0.00 |
| major | 0 | 0.00 |
| minor | 5 | 0.50 |
| nit | 3 | 0.09 |

**Score = 10.0 − 0.50 − 0.09 = 9.41**

## Previous findings (iteration 2)

| Id | Severity | Status | Evidence |
|---|---|---|---|
| data-integrity-2-01 | major | **Fixed** | `crates/otto-context/src/library.rs:311-349`: each cache entry stores the `(mtime, len)` stamp taken *before* the read, and `get_skill` re-stats on every call (a missing file evicts the entry). Out-of-band writers (the improve engine, `install_into`, hand edits) are picked up on the next read. A read that races a `put_skill` stores the pre-write stamp, so it can't pin the old body. |
| data-integrity-2-02 | minor | **Fixed** | `library.rs:70-101` `write_atomic`: temp sibling, `sync_all`, rename, directory fsync, keeping permissions and symlinks. It is used by `put_skill` (:358), `write_skill_file` (:438), `set_default_soul` (:698) and `write_md_entry` (:740). Skill-eval promote backs up first: `skill_eval.rs:2676`, called at `:2778`. |
| data-integrity-2-03 | minor | **Fixed** | Each repo's update is now one `UPDATE` with COALESCE, or CASE for tri-state fields: `api_client.rs:550-575`, `broker_clusters.rs:184-203`, `users.rs:149`, `aws_accounts.rs:163`, `workspaces.rs:204`, `mcp_servers.rs:137`. |
| data-integrity-2-04 | minor | **Fixed** | `database_changes.rs:135-160` `finish_in`: inside the same transaction it turns `running` attempts into `outcome_unknown`, and the classifier counts `running` as unknown. `reconcile` (:438-475) runs the attempt update and `finish_in` in one transaction. `recover_interrupted` (:479-497) re-classifies any `outcome_unknown` change that has no unknown or running attempt left. |
| data-integrity-2-05 | minor | **Fixed** | `routes/api_client/collections.rs:44`, `:69-79` and `:108-122`, plus `requests.rs:95` and `:140`: a parent or collection id must belong to `wid`. `ApiClientRepo::collection_parent_would_cycle` (`api_client.rs:237-260`, recursive CTE) rejects a move under the collection itself or one of its descendants. |
| data-integrity-2-06 | nit | **Fixed** | `otto-insights/src/lib.rs:166-174`: the config is written to a temp file, then renamed. |

All six iteration-2 findings are fixed. Nothing was rejected.

## Areas covered

- **Re-verification of every iteration-2 finding** (table above), against the current code.
- **Migrations 0182–0188**, checked for rollback compatibility. 0185 rebuilds `issue_accounts` to widen its CHECK, and I confirmed no table holds a FK to `issue_accounts`, so the implicit `DELETE` in `DROP TABLE` cascades nothing. 0182 and 0187 replace triggers; 0184 adds a column with NOT NULL DEFAULT; 0186 and 0188 add indexes only. No issues.
- **Session restart reconciliation** (`otto-sessions/src/manager.rs` `restore_all` :6411, `mark_dormant_except` in `otto-state/src/sessions.rs:546-576`, and `merge_meta` / `replace_meta_keys`). Meta is merged atomically with `json_patch` in SQL. The read-modify-write `set_meta` has no production callers. No issues.
- **Usage ingestion journal** (`otto-usage/src/ingestion.rs`):
  - The checkpoint and the outbox commit in one FULL-sync transaction.
  - Replay to ClickHouse is idempotent: `usage_events_durable` is a ReplacingMergeTree keyed on `event_id`, read through `FINAL`.

  No issues.
- **Vault note write and rename** (`otto-vault/src/engine.rs` `write_note_opts` :1339-1404, `rename` :1575-1745):
  - The revision is prepared before the atomic replace.
  - The index row carries the written file's mtime, so a crash before the index update is caught by the next scan.
  - `rename` uses `NOREPLACE` and rolls back a case-only rename.

  No issues.
- **Workgraph** (`otto-state/src/workgraph.rs`, `otto-workgraph/src/service.rs`, `routes/workgraph.rs`):
  - `UNIQUE(workspace_id, kind, source_id)` turns a racing double insert into an error, not a duplicate.
  - The edge and approval routes check that both endpoints belong to the workspace.

  No issues.
- **Product orphan reaper** (`otto-product/src/run.rs:1358-1480`) and discovery-run creation. No data-integrity issue: the discovery run's `status` column has no production writer, but no UI reads it either.
- **Personal agents**:
  - Context CAS (`save_context`), one-run-at-a-time admission, `reset_agent` in one transaction.
  - Report-path uniqueness: the path is per agent plus a per-second stamp, and runs are serialized.
  - `reap_running`.

  No issues.
- **Assistant threads, turns, attachments and incognito expiry.** One finding: 3-03.
- **Memory lifecycle** (soft forget / undo, state machine). One finding: 3-01.
- **DB Explorer saved queries, dashboards and widgets.** Findings 3-02 and 3-08.
- **k8s clusters and monitor rows.** Finding 3-08 (nit).
- **Canvas update, snapshot, prune and GC** (`otto-state/src/canvas.rs:393-735`): files, row and ref set change in one IMMEDIATE transaction. No issue.
- **Workspaces**: context CAS, a settings PATCH that carries `$.context` over, and `patch_settings` via `json_patch`. No issue.
- **Workbench docs**: create, update, trash, purge and restore-revision are all transactional. No issue.
- **Retention pruning** (`otto-state/src/retention/runs.rs`): batch deletes run inside transactions and re-check terminal status. No issue.
- **Room recap archive** (`otto-server/src/rooms/recap/archive.rs`): `atomic` (temp, `sync_data`, rename), an append-only journal with recovery of an interrupted suffix, and quota checks before every write. No issue.
- **UI localStorage drafts**:
  - API client tabs: `ui/src/lib/stores/apiClient.svelte.ts:451-490`. Per-workspace key, `pagehide` flush, quota fallback, secrets stripped.
  - DB Explorer tabs: `ui/src/lib/stores/database.svelte.ts:1613-1680`. Falls back to sessionStorage when over quota and warns once.

  No issues.
- **MCP launcher files** (`otto-sessions/src/mcp.rs`): `.mcp.json` written by temp + rename, and `info/exclude` changed by append-only writes. No issue. The sibling writer in otto-review is finding 3-06.
- **Sweep of multi-autocommit functions in `otto-state`**: every `pub async fn` with two or more `.execute(&self.pool)`. Each one is triaged above, or harmless (child-first deletes, idempotent touches).
- **RMW guard sweep**: Rust-side reads that decide a later SQL write. Finding 3-04.

## Areas NOT covered yet (for iteration 4)

- `crates/otto-state/src/swarm/` sub-modules beyond `swarm.rs` (task/run state transitions under concurrent coordinator + operator moves), and `pending_task_results` vs retention of `done` runs whose routing never completed.
- `crates/otto-server/src/workflow_engine.rs` node-level writes (39 commits in 3 weeks) — only the repo layer was checked in iteration 2.
- `crates/otto-dbviewer/src/service.rs` query-history / saved-query paths beyond widgets (21 commits).
- `crates/otto-design-assist` learned-team-rules persistence and `otto-improve` design-rule write path end to end.
- `crates/otto-channels` bridge state (Slack/Telegram cursors, dedup of inbound messages across restart).
- `crates/otto-brokers` ops repo (consumer offsets / replay evidence) now that it is wired in the daemon.
- Desktop shell (`apps/desktop/src-tauri`) window/pane state files.
- `otto-state/src/proof.rs` / `review_proof_packs.rs` archive (0164 proof_pack_archive) and its interaction with retention.

---

## Findings

### data-integrity-3-01 — minor — Undoing a memory "forget" always restores it as `accepted`, so a pending memory skips review and a superseded one comes back to life

- **Files:**
  - `crates/otto-state/src/memory.rs:948-969` (`soft_forget`), `:978-1008` (`undo_forget_scoped`)
  - `crates/otto-assistant/src/assistant/memory.rs:233-235`, `:281-288`, `:151-152`, `:215-218`
  - `crates/otto-server/src/memory_gov.rs:65`
- **Evidence:**
  - **The forget loses the state.** `soft_forget` sets `active=0, forgotten_at, undo_token, state='stale'` and saves nowhere what `state` was before.
  - **The undo assumes it.** `undo_forget_scoped` always runs `UPDATE memories SET active=1, forgotten_at=NULL, undo_token=NULL, state='accepted' …`.
  - **A pending memory skips review.** With `memory_approval` on, an agent-saved memory lands `suggested`, the pending state (`assistant/memory.rs:151-152`). Accepted and pending rows are split on `state == "suggested"` (`:215-218`). Pressing the Undo chip or toast on a forgotten pending memory (`undo`, `:281-288`) flips it straight to `accepted`. It joins the accepted set with no human approval. Its open `memory_review` needs-you item is never settled: only `accept` calls `settle_review` (:290).
  - **A superseded memory comes back.** A forgotten `contradicted` memory, whose `superseded_by` is still set, comes back as `active=1, state='accepted'`. That reverses the merge/supersede decision.
- **Fix:**
  - Record the previous state on forget, for example `prev_state = state` in a new nullable column via an additive migration, or inside the `provenance_json` / undo payload. Have the undo write `state = COALESCE(prev_state, 'accepted')` and `active = (prev_state NOT IN ('contradicted'))`.
  - Only re-open a review item when the restored state is `suggested`.
  - Add a test: save as pending → forget → undo → `state == 'suggested'`.

### data-integrity-3-02 — minor — Deleting a DB Explorer dashboard leaves its widgets in the database, and `dashboard_id` is never checked

- **Files:**
  - `crates/otto-state/src/db_explorer.rs:454-461` (`delete_dashboard`)
  - `crates/otto-dbviewer/src/http.rs:1407-1419` (route), `:1440-1460` (`create_widget`), `:1474-1495` (`update_widget`)
  - `crates/otto-state/migrations/0021_db_explorer.sql:38-53`
  - `ui/src/modules/database/Dashboards.svelte:44-46`
  - `ui/src/lib/stores/database.svelte.ts:4279-4283`, `:4362-4366`
- **Evidence:**
  - **The UI promises the widgets go too.** It asks "Delete dashboard “…” and its widgets?", and after the call it drops those widgets from local state (`this.widgets.filter((w) => w.dashboard_id !== id)`).
  - **The server deletes only the dashboard.** It runs only `DELETE FROM db_dashboards WHERE id = ?`. `db_widgets.dashboard_id` has no FK (0021), so nothing cascades or nulls.
  - **The widgets come back hidden.** On the next load `list_widgets` returns them again. The only views filter by `w.dashboard_id === did` (`widgetsForSelectedDashboard`, `DbDashboardBox.svelte:79`), so no screen can show or delete them. The rows keep their saved SQL statement and connection id indefinitely, even though the user confirmed they were deleted.
  - **The id is never validated.** `create_widget` / `update_widget` store `req.dashboard_id` without checking that the dashboard exists or belongs to `wid`. A widget filed under another workspace's dashboard, or a mistyped id, is invisible from creation.
- **Fix:**
  - In `delete_dashboard`, delete the dashboard's widgets in the same transaction (`DELETE FROM db_widgets WHERE dashboard_id = ?`, then the dashboard). This matches the UI's promise.
  - In the create and update routes, reject a `dashboard_id` whose row isn't in the widget's workspace (`SELECT workspace_id FROM db_dashboards WHERE id = ?`).
  - Optionally run a one-off cleanup, `DELETE FROM db_widgets WHERE dashboard_id IS NOT NULL AND dashboard_id NOT IN (SELECT id FROM db_dashboards)`. As a data migration this is rollback-safe, since it drops no column.

### data-integrity-3-03 — minor — Assistant attachment files are never deleted with their thread, including expired incognito threads

- **Files:**
  - `crates/otto-server/src/routes/assistant.rs:227-246` (upload: bytes written to `<assistant_dir>/<user>/inbox/<id>-<name>`), `:168-178` (`delete_thread`)
  - `crates/otto-assistant/src/assistant/tasks.rs:1447-1458` (`expire_incognito`)
  - `crates/otto-state/migrations/0140_assistant.sql:50-55`
- **Evidence:**
  - **Uploads land on disk.** An upload writes up to 20 MiB into the user's `inbox/` directory and records the path in `assistant_attachments`. That row's `thread_id` is `ON DELETE CASCADE`.
  - **Deletes stop at the database.** `delete_thread`, from the route and from the incognito expiry tick, deletes the thread row. The attachment rows cascade away, but nothing reads their `path` first or removes the file.
  - **Nothing else cleans the inbox.** A grep for an `inbox` remove/prune/clean finds no cleanup anywhere.
  - **Result:** every deleted thread leaks its uploaded files, with no DB row left that references them. Incognito threads are documented as deleted 24 h after their last turn (`tasks.rs:15`, `:55`), but their pasted files stay on disk for good.
- **Fix:**
  - In `AssistantRepo::delete_thread`, use `DELETE … RETURNING`, or select `path` from `assistant_attachments WHERE thread_id = ?` inside the same transaction, and return the paths.
  - After the commit, the route and `expire_incognito` remove each path. Confine it to the user's `inbox/` dir and ignore `NotFound`.
  - Add a boot or tick sweep that removes `inbox/*` files with no `assistant_attachments.path` row, older than a grace period.

### data-integrity-3-04 — minor — Rediscovery and re-detection make their decision from a separate read, so they can overwrite a pinned MCP risk label or a user's finding decision

- **Files:**
  - `crates/otto-state/src/mcp_control.rs:588-621` (`upsert_discovered`), compared with `:668-703` (`patch`)
  - `crates/otto-state/src/review_findings.rs:348-392` (`upsert_tracked`)
- **Evidence:**
  - **MCP risk pin.** `upsert_discovered` reads `risk_overridden` on the reader pool (:591-598), then picks an UPDATE in Rust. With `overridden == false` it writes `risk_label = ?, injection_risk = ?` unconditionally (:613-619).
    - If an admin's `patch` lands between that read and the UPDATE, the patch sets `risk_label`, `risk_overridden = 1` (:680-688), and rediscovery then overwrites the admin's label with the discovered one.
    - The row is left `risk_overridden = 1` with the auto label. That breaks the documented invariant "a human setting risk/injection pins it so rediscovery never lowers it" (:677-679), on a column the governance pipeline uses to gate tool calls.
    - The iteration-2 test (`test-quality-2-03`) covers only the sequential case.
  - **Review finding decisions.** `upsert_tracked` reads `state` (:348-367), computes `next_state` in Rust (`resolved|declined → regressed`, else keep), then writes `SET state = ?` (:373-392).
    - If a user declines or resolves the finding between those two statements, the stale `open` is written back.
    - The user's decision is silently reverted, where the intended outcome is `regressed`.
- **Fix:** Move each guard into the write statement:
  - **mcp_tools:** `risk_label = CASE WHEN risk_overridden = 1 THEN risk_label ELSE ? END`, and the same for `injection_risk`, in one UPDATE.
  - **review_findings:** `state = CASE WHEN state IN ('resolved','declined') THEN 'regressed' ELSE state END`.

  Or wrap each read and write in `self.pool.begin()` (IMMEDIATE).

### data-integrity-3-05 — minor — `rotate_mcp_token` creates the new token and deletes the old one in two separate commits

- **File:** `crates/otto-rbac/src/tokens.rs:1035-1067`
- **Evidence:**
  - **Non-atomic sequence.** Rotation reads the row (:1036-1044), then `issue_mcp_token_with_scope` commits a new token (:1056-1058), then a separate `DELETE FROM auth_sessions WHERE id = ? AND kind = 'mcp'` runs (:1060-1064). No transaction or guard ties the steps together.
  - **Orphaned live credential.** If the delete fails (SQLITE_BUSY after the busy timeout, a full disk), the call returns an error. The new full-scope MCP token is already committed and its plaintext was never shown, so a live credential exists that nobody holds. Meanwhile the old token is still valid.
  - **Concurrent rotations multiply tokens.** Two rotations of the same id (a double click, or two tabs) both pass the lookup, and each mints a token. Both new tokens stay live and the old one is deleted once, so one more credential exists than the UI shows was rotated.
- **Fix:** Do the lookup, insert and delete in one `begin()` transaction. Make the delete the guard and run it first: `DELETE … WHERE id = ? AND kind = 'mcp' RETURNING user_id, label, mcp_scope`. Insert the new token only if that delete returned a row, and commit both together.

### data-integrity-3-06 — nit — The review engine rewrites the user's `.git/info/exclude` in place

- **File:** `crates/otto-review/src/engine.rs:1693-1706` (`exclude_otto_dir`)
- **Evidence:**
  - **In-place truncate.** The function reads the whole `info/exclude`, appends `/.otto/`, and writes the file back with `std::fs::write` (`O_TRUNC`, then write). That file holds the user's own ignore rules.
  - **Crash risk.** A crash or full disk mid-write truncates it.
  - **Concurrent-writer risk.** Because it is read-then-rewrite, it can also drop lines another writer appended in between: `otto-sessions/src/mcp.rs:522-554` `append_exclude_lines` adds the launcher entries in append mode.
- **Fix:** Use the same append-only write as `append_exclude_lines`: `OpenOptions::new().create(true).append(true)`, writing only the missing lines, with a leading newline when the file doesn't end in one.

### data-integrity-3-07 — nit — `PUT /settings` writes each key in its own commit and skips the in-memory reloads after a partial failure

- **File:** `crates/otto-server/src/routes/settings.rs:54-71`
- **Evidence:** `for (key, value) in &body { repo.put(key, value).await?; }` commits one key at a time. If a later key fails, `?` returns before the reload block runs. That block applies `providers`, `disabled_providers` and `agent_skip_permissions` to the running registry. Keys that were already committed (for example `providers`) are now in the DB but not in the live registry until the next restart, and the client receives an error for a request that partly applied. The UI sends one key per call today, so this only affects API and multi-key callers.
- **Fix:** Add `SettingsRepo::put_many(&Map)`, which writes every key in one `begin()` transaction and invalidates the cache after the commit. Run the reloads only after it succeeds.

### data-integrity-3-08 — nit — Read-then-write PATCHes in DB Explorer and k8s clusters lose concurrent edits to other fields

- **Files:** `crates/otto-state/src/db_explorer.rs:421-452` (`update_dashboard`), `:526-568` (`update_widget`); `crates/otto-state/src/k8s_clusters.rs:252-288` (`update`)
- **Evidence:** Each function loads the whole row on the reader pool, fills the omitted fields from that snapshot, and writes every column back in a separate statement. Two concurrent PATCHes that touch different fields can therefore lose one edit, because the second write carries the first's stale value for the other field. Examples:
  - a dashboard layout save racing a rename or a refresh-interval change;
  - a widget title edit racing a move to another dashboard.

  This is the same class iteration 2 fixed in eight other repos (2-03): there, omitted fields use `COALESCE(?, col)` against the row at write time.
- **Fix:** Use single `UPDATE … SET col = COALESCE(?, col)` statements. Use `CASE WHEN ? THEN ? ELSE col END` for the tri-state `dashboard_id` and `refresh_secs`. For k8s `params_json`, use `json_set` / `json_patch`.
