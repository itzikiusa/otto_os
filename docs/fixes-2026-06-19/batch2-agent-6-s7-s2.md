# Batch-2 Agent 6 — S7 (widget authz) + S2 (fs allow-list)

Date: 2026-06-19
Files owned/edited:
- `crates/otto-dbviewer/src/http.rs`
- `crates/otto-server/src/routes/fs.rs`

---

## S7 — `run_widget` privilege escalation (DB Explorer)

### The hole
`run_widget` (handler for `POST /db/widgets/{id}/run`) required only workspace
`Viewer`, then executed the widget's stored statement via
`DbViewerService::run_widget` → `DbViewerService::run` → `Driver::run` — the
**exact same execution path** that `run_query` (`POST /connections/{id}/db/query`)
gates behind `Editor`. `Driver::run` runs arbitrary SQL/commands including writes
and DDL. So a workspace **Viewer** could trigger arbitrary stored writes/DDL by
running a dashboard tile — a privilege escalation.

`run_query`'s gate (the reference): it resolves the connection by path id and
calls `check_conn_role(&ctx, &user, &conn, WorkspaceRole::Editor)` — Editor in
the connection's workspace, or **root only** for global (workspace-less)
connections (`check_conn_role`, http.rs).

### Fix chosen: require `Editor` (gate on the connection, like `run_query`)
`run_widget` now resolves the widget's **connection** and runs the identical
gate as `run_query`:

```rust
let conn = ctx.db().get_connection(&widget.connection_id).await?;
check_conn_role(&ctx, &user, &conn, WorkspaceRole::Editor).await?;
```

This both closes the escalation (the gate is now byte-for-byte the run_query
gate) and fixes a secondary gap: the old code gated on the *widget's workspace*
only, ignoring that the widget's connection could be a **global** (root-only)
connection. Gating on the connection now correctly handles global connections.

### Why NOT "enforce read-only for Viewers"
The task allowed keeping read-only dashboards working for Viewers "if feasible,
else require Editor." It is **not** cleanly feasible here:

- Read-vs-write classification is **per-engine and private**: e.g.
  `is_read_statement` lives inside `drivers/mysql.rs` and only knows SQL
  keywords; Redis (command-based) and Mongo (operation-based) have entirely
  different semantics. There is no shared, cross-engine read-only classifier.
- A route/service-level reimplementation of read-only detection would duplicate
  per-engine logic and be fragile — exactly the kind of bypassable half-measure
  the audit is trying to eliminate. (A proper version would add an
  `is_read_statement` method to the `Driver` trait, touching all 4 driver files,
  which are owned by other agents this batch.)

So the safe default — **no privilege escalation** — is to require `Editor`,
matching `run_query`. Operational impact: a pure Viewer who only opens a
dashboard will now get 403 on the auto-running widget tiles (the dashboard UI
auto-runs `runWidget` on mount, see `ui/src/modules/database/WidgetCard.svelte`).
Widget **creation** already required `Editor`, so this aligns view-execution with
edit rights. If live Viewer dashboards become a product requirement, the correct
follow-up is a `Driver::is_read_statement` capability + a read-only widget run
path — tracked as a deliberate non-goal here.

### Tests added (otto-dbviewer, `http::tests`)
Exercise the load-bearing gate `check_conn_role` with a recording stub
`RoleChecker` (no live `DbViewerService` needed — `check_conn_role` never touches
`db()`):
- `viewer_is_rejected_from_widget_execution_gate` — a workspace Viewer is denied,
  and the gate is asserted to have demanded `Editor` (not `Viewer`).
- `editor_passes_widget_execution_gate` — a legitimate Editor still runs widgets.
- `global_connection_requires_root` — a global (workspace-less) connection denies
  a non-root user (Admin elsewhere) and the role checker is never consulted;
  root passes. Matches every other execution path.

---

## S2 — `/fs/browse` + `/fs/read` deny-list → allow-list (+ deny-list on top)

### The hole
The host-file sandbox in `routes/fs.rs` was a **deny-list only**: it blocked
known secret stores (`~/.ssh`, `~/.aws`, `/etc`, …) and secret filenames
(`id_rsa`, `.env`, `*.pem`, …). Any **non-denied** sensitive path was still
fully browsable/readable by an authenticated user — e.g. `/var/root` subpaths not
explicitly listed, another user's home, arbitrary `/opt/...`, app data outside
the picker's intended scope. Bypassable by construction.

### Fix: allow-list (primary) + existing deny-list (defense in depth)
A canonical (symlink + `..` resolved) path is now served **only if it is within
one of the permitted roots**, AND still not caught by the secret-store deny-list.
Both `guard_dir` and `guard_file` apply the allow-list first, then the deny-list.

### Allowed roots (`sandbox::allowed_roots(data_dir)`)
Each is **canonicalized**; an unresolvable root is dropped (a non-existent root
can't contain a canonical target anyway). The set:

1. **`$HOME` subtree** — the folder picker's primary playground (it defaults to
   `~` and navigates from there; FolderPicker/FileTree/NewWorkspace/GitPage/etc.
   all start within HOME).
2. **`ctx.data_dir`** — the daemon data dir (library, swarm worktrees/scratch).
   Under `$HOME` by default (`~/Library/Application Support/Otto`) but can be
   relocated via `$OTTO_DATA_DIR`, so it's added explicitly. The FileTree / swarm
   features legitimately browse worktrees under here.
3. **Env single-dir roots** the daemon manages: `$OTTO_LOG_DIR`,
   `$OTTO_SKILLEVAL_DIR` (added when set and resolvable).
4. **`$OTTO_FS_EXTRA_ROOTS`** — new colon-separated operator escape hatch for
   extra permitted roots, e.g. a workspaces tree created **outside** `$HOME`
   (workspaces can be created at any path the user types; this lets an operator
   opt those trees in without weakening the default).

**Fail closed:** if nothing resolves (no `$HOME`, no `data_dir`), the root set is
empty and `is_within_allowed` is always false → every path is denied.

Path containment uses canonical `==` / `starts_with` on **path components**, so a
prefix-sibling like `/Users/me-evil` does NOT match the `/Users/me` root.

### What stays (deny-list, now defense in depth)
`HOME_DENY_DIRS` (`~/.ssh`, `~/.aws`, `~/.gnupg`, `~/.kube`, `~/.docker`,
`~/.config/gcloud`, `~/.config/gh`, `~/.azure`, `~/.password-store`),
`ABS_DENY_PREFIXES` (`/etc`, `/private/etc`, `/root`, `/var/root`, `/proc`,
`/sys`), `DENY_FILE_NAMES` (`id_rsa`, `credentials`, `.env`, `.netrc`, …),
`DENY_FILE_SUFFIXES` (`.pem`, `.key`, `.pfx`, `.p12`, `.keystore`). So a secret
store **nested inside an allowed root** (e.g. `~/.ssh`) is still blocked even
though `~` is permitted.

`CurrentUser` enforcement is preserved (both handlers still take
`CurrentUser(user)`); `data_dir` is read from the authenticated `ServerCtx`. The
`_user` parameter remains threaded through `guard_dir`/`guard_file` for future
per-user root scoping; root vs non-root relaxes neither list.

### Tests added (otto-server, `routes::fs::tests`)
A module-local `ENV_LOCK` mutex serializes env-mutating tests (`HOME`,
`OTTO_FS_EXTRA_ROOTS`) so the parallel runner can't interleave one test's env
with another's reads:
- `allow_list_admits_in_root_denies_out_of_root` — HOME, a nested project under
  HOME, and a separate `data_dir` are **allowed**; `/var/root`,
  `/Users/someone-else/Documents`, `/opt/secret`, and a prefix-sibling of HOME
  are **denied** purely for being outside the roots (not via the deny-list).
- `allow_list_honors_extra_roots_env` — a workspaces tree outside HOME is admitted
  only while `$OTTO_FS_EXTRA_ROOTS` names it; revoked once the env is cleared.
- `allow_list_fails_closed_with_no_roots` — no resolvable roots → empty list →
  everything denied.
Existing deny-list tests (`denies_system_secret_dirs`, `denies_home_secret_dirs`,
`denies_secret_files_by_name_and_ext`) retained and still pass.

---

## Verification

```
cargo check -p otto-dbviewer -p otto-server    # clean (only pre-existing
                                               # otto-usage dead-code warnings,
                                               # not my files)

cargo test -p otto-dbviewer --lib http::       # 3 passed (S7 gate tests)
cargo test -p otto-server  --lib routes::fs::  # 6 passed (3 new allow-list + 3 deny-list)

cargo test -p otto-dbviewer -p otto-server --lib
  otto-dbviewer: 75 passed; 0 failed; 1 ignored (pre-existing)
  otto-server:  122 passed; 0 failed; 0 ignored
```

No errors originated from files I don't own. Did not run `cargo fmt`, did not
commit, did not touch other agents' files.
