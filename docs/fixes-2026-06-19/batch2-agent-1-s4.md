# S4 — Cross-user Jira/Confluence credential leak (Batch-2 Agent 1)

**File:** `crates/otto-issues/src/http.rs`
**Severity:** Critical (verified open). Any authenticated user could act with another
user's Atlassian identity because read/use handlers loaded the issue account by id
with no ownership check. Only create/update/delete enforced the guard.

## Root cause

Every read/use handler took `Extension(_user)` (user deliberately discarded) and
called `s.issues().get_account(&account_id)` directly. The account is keyed by
`account_id` from the request path/query — not by the caller — so the server would
happily fetch *any* user's stored credentials, resolve the Keychain token, and
proxy a Jira/Confluence call on their behalf.

## Fix — single load-and-authorize chokepoint

Added two helpers (http.rs):

- `fn authorize_account(account: &IssueAccount, user: &AuthUser) -> Result<(), Error>`
  — pure, testable. Returns `Error::Forbidden("not the account owner")` when
  `account.user_id != user.0.id && !user.0.is_root`. Identical semantics to the
  guard already used by create/update/delete; behavior is unchanged for the
  legitimate owner and for root.
- `async fn load_authorized_account<S: IssuesCtx>(s, id, user) -> Result<IssueAccount, Error>`
  — loads the account by id and authorizes in one step. This is now the **only**
  call site of `get_account` in the crate, so no handler can resolve an account
  without going through the ownership check. Future handlers that call
  `load_authorized_account` are guarded by construction.

`Error::Forbidden` maps to HTTP 403 via the crate's existing `ApiErr` →
`Problem` mapping (http.rs:42-59). No request/response shapes changed; no api.md
change needed.

## Handlers now enforcing ownership

All handlers that resolve an `account_id` were switched from a bare
`get_account` to `load_authorized_account` (and from `Extension(_user)` to
`Extension(user)` where they previously discarded the caller):

| Handler | Route | http.rs line (guard) |
|---|---|---|
| `update_account` | `PATCH /issue/accounts/{id}` | 194 |
| `delete_account` | `DELETE /issue/accounts/{id}` | 226 |
| `list_projects` | `GET /issue/projects` | 241 |
| `search_issues` | `GET /issue/search` | 269 |
| `list_spaces_cf` (Confluence spaces) | `GET /issue/confluence/spaces` | 284 |
| `search_pages_cf` (Confluence CQL search) | `GET /issue/confluence/search` | 303 |
| `get_issue` | `GET /issue/{account_id}/{key}` | 331 |
| `get_issue_full` | `GET /issue/{account_id}/{key}/full` | 354 |
| `list_transitions` | `GET /issue/{account_id}/{key}/transitions` | 372 |
| `do_transition` | `POST /issue/{account_id}/{key}/transitions` | 397 |
| `list_assignable` | `GET /issue/{account_id}/{key}/assignable` | 415 |
| `assign_issue` | `PUT /issue/{account_id}/{key}/assignee` | 441 |
| `get_attachment` (attachment byte proxy) | `GET /issue/{account_id}/{key}/attachment/{attachment_id}` | 461 |
| `list_issue_types_handler` | `GET /issue/{account_id}/{project_key}/issue-types` | 494 |
| `add_comment` | `POST /issue/{account_id}/{key}/comment` | 519 |

`create_account` and `list_accounts` need no change: create binds the account to
the caller (`user.0.id`) and list filters by `user.0.id`.

### Confluence note
The task listed Confluence "get page / create / update page" handlers. Those do
not exist in this crate yet — the only Confluence HTTP surfaces are
`list_spaces_cf` and `search_pages_cf`, both now guarded. http.rs is the sole
router; a repo-wide grep found no `get_account` / `Extension<AuthUser>` handlers
outside this file, so every account access point is covered.

## Tests

Added a `#[cfg(test)] mod tests` in http.rs covering the pure guard:

- `owner_is_authorized` — owner → Ok
- `non_owner_is_forbidden` — other user → `Error::Forbidden`
- `root_can_access_any_account` — root → Ok

Added `chrono` as a **dev-dependency** in `crates/otto-issues/Cargo.toml` (test-only,
needed to build the `IssueAccount`/`User` fixtures; `chrono` is a workspace dep).

## Verification

- `cargo check -p otto-issues` — clean.
- `cargo test -p otto-issues` — 62 passed, 0 failed (includes the 3 new ownership tests).
