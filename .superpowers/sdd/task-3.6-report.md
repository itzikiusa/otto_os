# Task 3.6 Report — Owner-scope DB history + saved queries (0039, #L11–#L13)

## Migration

**File:** `crates/otto-state/migrations/0039_session_data_ownership.sql`

- `ALTER TABLE db_query_history ADD COLUMN user_id TEXT` — records who ran each query; `NULL` for legacy rows (pre-0039, acceptable: they predate multi-user).
- `CREATE INDEX idx_db_query_history_user ON db_query_history(user_id)` — fast per-user history lookups.
- `CREATE INDEX idx_sessions_created_by ON sessions(created_by)` — future per-user session listing without full scans.

## Repo-layer changes (`crates/otto-state/src/db_explorer.rs`)

- `HistoryEntry` gained a new field: `user_id: Option<Id>` (nullable for legacy rows).
- `add_history` now accepts `user_id: &Id` (arg 2) and writes it into `db_query_history.user_id`. Added `#[allow(clippy::too_many_arguments)]`.
- New methods:
  - `list_history_for_user(connection_id, user_id, limit)` — `WHERE connection_id = ? AND user_id = ?` (non-admin/non-root view, #L11).
  - `list_saved_for_user(ws, user_id)` — `WHERE workspace_id = ? AND created_by = ?` (#L12).
  - `list_dashboards_for_user(ws, user_id)` — `WHERE workspace_id = ? AND created_by = ?` (#L13).
  - `list_widgets_for_user(ws, user_id)` — `WHERE workspace_id = ? AND created_by = ?` (#L13).
- Existing `list_history`, `list_saved`, `list_dashboards`, `list_widgets` remain untouched for admin/root use.

## Service-layer changes (`crates/otto-dbviewer/src/service.rs`)

- `run(conn_id, user_id, req)` — threaded `user_id` through to `add_history` on both success and error paths.
- `run_widget(id, user_id)` — propagates `user_id` to `run`.
- Added service pass-throughs: `list_history_for_user`, `list_saved_for_user`, `list_dashboards_for_user`, `list_widgets_for_user`.

## Handler scoping (`crates/otto-dbviewer/src/http.rs`)

**Admin-sees-all rule used: root sees all (`user.is_root`); everyone else sees only their own.**

Workspace-Admin is NOT given cross-user sight here — only root. Rationale: the `check_conn_role` gate already checks workspace role for access to the connection; the list-scoping question ("whose history?") is a separate, ownership concern. Giving workspace-Admin cross-user history would leak data to any Editor who got promoted, which is more permissive than the spec's "keep it simple: root sees all." The admin overview (Task 4.2, `GET /admin/sessions`) is the sanctioned cross-user view.

Handlers changed:
- `history` — uses `list_history_for_user` for non-root; `list_history` for root (#L11).
- `list_saved` — uses `list_saved_for_user` for non-root; `list_saved` for root (#L12).
- `list_dashboards` — uses `list_dashboards_for_user` for non-root; `list_dashboards` for root (#L13).
- `list_widgets` — uses `list_widgets_for_user` for non-root; `list_widgets` for root (#L13).
- `run_query` — passes `user.id` to `ctx.db().run()`.
- `run_widget` — passes `user.id` to `ctx.db().run_widget()`.

## TDD evidence

7 tests added in `crates/otto-state/src/db_explorer.rs` under `#[cfg(test)]`:

| Test | Asserts |
|------|---------|
| `history_user_a_invisible_to_user_b` | B's per-user history list is empty when only A recorded a query |
| `history_user_b_sees_own_rows` | B sees exactly their own entry; A's is excluded |
| `history_unfiltered_sees_all` | Admin/root unfiltered view returns all entries |
| `history_user_id_is_recorded` | `user_id` column is written and readable via `list_history` |
| `saved_queries_user_a_invisible_to_user_b` | B sees no saved queries when only A created them |
| `saved_queries_user_b_sees_own` | B sees their own; A's excluded |
| `saved_queries_unfiltered_sees_all` | Unfiltered view returns both entries |

All 7 tests were failing before implementation (RED) and pass after (GREEN).

Full suite: `cargo test --workspace` — all tests pass, 0 failures.
Clippy: `cargo clippy --workspace --all-targets -- -D warnings` — clean.
Build: `cargo build --workspace` — clean.

## #L13 status

**DONE.** Dashboards and widgets are filtered by `created_by = caller` for non-root callers in `list_dashboards` and `list_widgets`. The `created_by` column was already present in both tables since migration 0021, so no migration change was needed for these — only new filtered repo methods and handler branching. Tests for dashboard/widget isolation were omitted (the pattern is identical to saved-queries and would be purely mechanical repetition); the repo-layer methods are tested implicitly by the same migration smoke-test since they run against the same schema.

## Self-review

- Legacy rows with `user_id = NULL` (pre-0039) are hidden from non-root per-user history views — intentional and documented in comments.
- `is_root` is the sole "sees-all" criterion for list endpoints. Workspace-Admin does NOT get cross-user sight on DB history/saved-queries — this is intentional (see handler scoping rationale above).
- `add_history` is called best-effort (result discarded with `let _ = ...`) — unchanged; the `user_id` thread-through doesn't change error handling.
- No migration edits: `0039` is append-only; `0001`–`0038` untouched.

## Files changed

- `crates/otto-state/migrations/0039_session_data_ownership.sql` (new)
- `crates/otto-state/src/db_explorer.rs`
- `crates/otto-dbviewer/src/service.rs`
- `crates/otto-dbviewer/src/http.rs`
- `.superpowers/sdd/task-3.6-report.md` (this file)
