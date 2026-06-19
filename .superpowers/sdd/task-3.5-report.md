# Task 3.5 — Activity Trail / Tasks / Summary Ownership (#L16–#L18)

## Handlers changed

### `crates/otto-server/src/routes/activity.rs`

**`list_trail` (#L16)**
- Added `require_session_owner_or_admin(&ctx, &user, &session)` call after `session_in_ws(...)`.
- `session_in_ws` now returns the `Session` (was discarded before); the session is forwarded directly to the ownership gate.
- A non-owner workspace Editor now gets 403 before any trail rows are read.

**`list_tasks` (#L17)**
- Identical change: added `require_session_owner_or_admin` after `session_in_ws`.
- A non-owner workspace Editor now gets 403 on another user's task list.

**`workspace_summary` (#L18)**
- After the existing `require_ws_role(Viewer)` check, detects whether the caller is an admin/root:
  ```rust
  let is_admin = user.is_root
      || ctx.roles.check(&user, &wid, WorkspaceRole::Admin).await.is_ok();
  ```
- **Admin / root path**: calls `workspace_summary(&wid)` — the existing full workspace aggregate.
- **Non-admin path**: calls the new `workspace_summary_for_user(&wid, &user.id)` — restricted to sessions owned by the caller.

## The `workspace_summary_for_user` query (`crates/otto-state/src/activity.rs`)

Added `workspace_summary_for_user(workspace_id, user_id)` which delegates to a shared `workspace_summary_inner(workspace_id, Option<user_id>)`. When `user_id` is `Some`, the task and trail aggregation queries join against the `sessions` table on `sessions.created_by = user_id`, filtering out all rows that belong to other users. When `None` (admin/root path), the original queries without any join are used.

## Admin-vs-owner decision

The summary handler checks two conditions in order:
1. `user.is_root` — root always sees everything, no DB round-trip needed.
2. `ctx.roles.check(&user, &wid, WorkspaceRole::Admin).await.is_ok()` — the canonical `RoleChecker`, which also short-circuits root internally. If either passes, the caller gets the full aggregate; otherwise the user-scoped query is used.

This reuses the same `RoleChecker` path that all other admin checks in the codebase use (including `require_session_owner_or_admin`), so no forked logic.

## Security primitives reused (no forked logic)

- `require_session_owner_or_admin` from `crates/otto-server/src/auth.rs` (Task 3.1/3.2 product) — called as-is for trail and tasks.
- The same `RoleChecker::check` interface for the summary admin decision.
- No new authorization logic invented.

## TDD evidence

**RED → GREEN sequence:**

**State layer tests** (`crates/otto-state/src/activity.rs`, inline `#[cfg(test)]`):
- `summary_for_user_excludes_other_users_sessions` — proved the SQL filter works before handler wiring.
- `workspace_summary_returns_all_sessions` — proved the admin path is unaffected.
- `summary_for_user_empty_when_no_own_sessions` — proved no bleed when caller has no sessions.

**Handler integration tests** (`crates/otto-server/tests/activity_isolation.rs`):
- `non_owner_editor_forbidden_on_trail` (#L16) — 403 before implementation; 200-level after.
- `owner_admin_root_can_read_trail` — owner/admin/root must never get 403.
- `non_owner_editor_forbidden_on_tasks` (#L17) — same for tasks.
- `owner_admin_root_can_read_tasks` — same complement.
- `non_admin_summary_excludes_other_users_sessions` (#L18) — bob sees only his session.
- `admin_and_root_see_full_summary` — ws-admin and root see all sessions.

All 6 handler tests + 3 state tests = 9 new tests, all green. Zero regressions across the full workspace (`cargo test --workspace`: 0 failures).

## `created_by` availability in trail/tasks handlers

The `session_in_ws` helper already fetches the full `Session` (via `ctx.manager.get(sid)`). The `Session.created_by` field is already populated. No additional DB query was needed — we simply captured the returned `Session` (previously discarded) and passed it to `require_session_owner_or_admin`.

## Files changed

- `crates/otto-server/src/routes/activity.rs` — 3 handlers updated, 1 import added
- `crates/otto-state/src/activity.rs` — `workspace_summary_for_user` + `workspace_summary_inner` added, 3 inline tests added
- `crates/otto-server/tests/activity_isolation.rs` — new file, 6 handler isolation tests

## Self-review

- No forked owner/admin logic: reuses `require_session_owner_or_admin` and `RoleChecker::check`.
- The workspace-membership requirement stays (non-member still fails the `require_ws_role` check before reaching the ownership gate).
- The `workspace_summary_inner` implementation is additive — existing `workspace_summary` delegates to it unchanged.
- Clippy clean, build clean, all tests green.
