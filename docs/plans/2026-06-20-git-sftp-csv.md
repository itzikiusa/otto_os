# Plan: Git redesign + SFTP browser + DB local-CSV export

Branch: `feat/git-sftp-csv` (worktree `/Users/itziklavon/otto_os-gsc`).
Date: 2026-06-20. No new SQLite migrations (decision below). No new heavy Rust deps.

## Source-of-truth facts (from codebase exploration)

- Hash router: `router.module` → `moduleName` (`ui/src/shell/App.svelte`). Right
  panel renders only for `moduleName === 'agents' && ws.activeSession !== null`
  (App.svelte:584) → **no real sidebar on Git**; the "gap" is the repo-list's own
  `.page` + `repeat(auto-fill,minmax(320px,1fr))` grid (GitPage.svelte:360).
- `CommitInfo { sha, short_sha, author, date, subject, parents[], refs[] }` and
  `RefsResp { local[], remote[], tags[] }` with `RefBranch{name,is_current,upstream,remote}`
  already exist. `git log %D` decorations are parsed in `otto-git/src/parse.rs`
  (it currently **strips** the `HEAD -> ` marker — we must preserve it).
- Repos are workspace-scoped (`repos.workspace_id NOT NULL`), but per-repo ops
  (`/repos/{id}/log|refs|status`) authorize via the repo's own workspace, so a
  **global list endpoint is all we need** to decouple — no schema rebuild.
- SSH: Otto has **no embedded SSH lib**; it shells out to system `ssh`
  (`otto-ssh`) and builds flags in `otto-connections/src/builders.rs`. SFTP will
  **drive the system `sftp` binary** (full key/agent/config/jump parity) with
  `ControlMaster`/`ControlPersist` for speed. Daemon is local → `get`/`put`
  read/write the user's real disk.
- DB: `POST /connections/{id}/db/export` already exists (uncapped CSV/JSON as a
  browser attachment); rows return **into the daemon** (SSH tunnel transparent),
  so writing a local file daemon-side is viable. ClickHouse can stream
  `FORMAT CSVWithNames` through the tunnel → local file (sidesteps INTO OUTFILE).
- Policy: unmatched routes **deny by default**. `/connections/{id}/db/` →
  `Database`; `/repos/` & `/git/accounts` → `Git`; connections rules are exact
  matches (no generic `/connections/` fallthrough).
- Persisted UI state convention = localStorage (DB tabs `otto_db_tabs:{ws}:{conn}`,
  agent tabs `otto_tabs_{ws}`, right panel width/tab, etc.).

## Decision: localStorage (not a DB table) for open Git tabs

The user wants Git "not related to any workspace" + "remember what we had open
across restart". A **global** localStorage key (`otto_git_open_tabs`) is the
established pattern for exactly this UI state, is workspace-independent by
construction, persists across Tauri restarts, and avoids migration-number
collisions with the concurrent `feat/must-have-wave` worktree. → No migration.

---

## Feature 1 — Git section redesign

### Requirements (verbatim → mapping)
1. **Tabbed, GitKraken-style repo tabs across the top.** → New `GitTabs.svelte`
   strip above the repo content; each tab = one open repo (name + current branch
   + status dot + close ✕ + middle-click close + reorder). A `+`/"Open repo"
   button opens the repo picker. The existing per-repo sub-tabs
   (Graph/Changes/History/PRs/Review) stay inside the active repo.
2. **Not tied to any workspace + remember open tabs across restart.**
   - New global list endpoint `GET /git/repos` → all repos across workspaces the
     caller can view (root → all). New store method `git.loadAllRepos()`. GitPage
     stops keying off `ws.currentId`; header "Repositories" (drop "in {ws}").
   - Open tabs persisted to global localStorage `otto_git_open_tabs`
     `{ openRepoIds:[], activeRepoId, sub: {repoId: 'graph'|...} }`; restored on
     mount, filtered against the live global repo list (stale ids dropped).
3. **Graph clarity (which branch we're on + GitKraken-level detail).**
   - Backend: `parse.rs` preserves `HEAD` / `HEAD -> <branch>` in `refs[]` so the
     checked-out branch is identifiable on the graphed commit.
   - Frontend `GraphView.svelte`: classify each ref into local / remote / tag /
     HEAD and render distinct color-coded **ref chips** on the commit row; mark
     the **current branch** prominently (filled chip + "checked-out" icon) and
     highlight the HEAD commit row ("you are here"). Show ahead/behind on the
     current branch (from `RepoStatusResp.ahead/behind`). Strengthen the left
     refs panel: clearer current-branch indicator, local/remote/tags grouping,
     upstream shown. Keep lanes but improve node/branch legibility.
4. **Remove right gap on the Git page.** → Make the Git page full-bleed: the
   tabbed workspace fills the center column; the repo-list / empty landing uses
   full width (no `.page` max-width / no reserved right column). Confirm
   `RightPanel` never renders for `git` (already true) — no shell regression.

### Backend changes
- `crates/otto-state/src/git.rs`: add `list_all_repos(&self) -> Vec<Repo>`
  (`SELECT * FROM repos ORDER BY name`) and/or
  `list_repos_for_workspaces(&[Id])`.
- `crates/otto-git/src/http.rs`: add `GET /git/repos` handler → returns repos
  across workspaces where the caller has ≥Viewer (root → all). Reuse role
  helpers already used by the list handler.
- `crates/otto-git/src/parse.rs`: stop discarding the `HEAD` decoration; keep an
  explicit `HEAD` entry (and/or `HEAD -> branch`) in `CommitInfo.refs`. Keep
  ordering stable. Add/adjust unit tests in-file.
- `crates/otto-server/src/policy.rs`: add `if p == "/git/repos" { Require(Git, View) }`
  in the Git block (before `/workspaces/{id}/repos`).
- `docs/contracts/api.md`: document `GET /git/repos`.

### Frontend changes
- `ui/src/lib/stores/git.svelte.ts`: `loadAllRepos()`, open-tabs state
  (`openRepoIds`, `activeRepoId`, per-repo sub-tab), persist/restore to
  `otto_git_open_tabs`, `openRepoTab/closeRepoTab/reorderRepoTab/setSubTab`.
- `ui/src/modules/git/GitTabs.svelte` (new): the top repo-tab strip (mirrors
  `shell/TabBar.svelte` interactions; styled like ApiPage `.req-tabs`).
- `ui/src/modules/git/GitPage.svelte`: render `GitTabs` + active repo's
  `RepoView`; landing (no tabs open) = full-width repo browser/empty state; load
  via `loadAllRepos()`; remove workspace coupling + `.page` width constraint.
- `ui/src/modules/git/RepoView.svelte`: drive sub-tab via store (keep URL sync
  optional); accept active-repo from tabs.
- `ui/src/modules/git/GraphView.svelte`: ref-chip classification + current-branch
  / HEAD emphasis + ahead/behind; left-panel polish.
- `ui/src/lib/api/types.ts`: only if a field is added (prefer none; classify on FE).

### Out of scope / guardrails
- Don't drop/alter the `repos` table. Don't change per-repo endpoints' auth.
- Keep the embedded (agent right-panel) Git usage working (`embedded` prop).

---

## Feature 2 — SFTP file browser / transfer over SSH connections

### Requirement
For a regular SSH connection, browse/read/transfer files (MobaXterm-lite): list &
navigate dirs, download to local, upload from local, mkdir, delete, rename, view
a text file. Reuse the SSH connection's existing auth.

### Approach
Drive system `sftp -b` (batch) per op, sharing one `ControlMaster` socket per
open SFTP session (fast through bastions). Build base args from the connection
params exactly like `builders.rs` does for SSH (`-p port`, `-i identity_file`,
`-J jump`, `user@host`). Downloads/uploads use sftp `get`/`put` → local disk.

### Backend changes
- `crates/otto-ssh/src/sftp.rs` (new): `SftpSession` holding base ssh/sftp args +
  a per-session control-path socket dir; methods `list(path)`, `realpath(path)`,
  `download(remote, local)`, `upload(local, remote)`, `mkdir(path)`,
  `remove(path)`, `rmdir(path)`, `rename(from,to)`. `SftpEntry { name, kind:
  dir|file|symlink|other, size, mtime, perms, symlink_target? }`. Tolerant
  parser for sftp `ls -l` longname (perms[0]=type, size@idx4, name=join(idx≥8),
  split " -> " for symlinks). Cleanup control socket on drop.
- `crates/otto-connections/src/http.rs`: routes (all require kind==ssh):
  - `GET  /connections/{id}/sftp/list?path=` → `{ path, entries[] }`
  - `POST /connections/{id}/sftp/download` `{ remote_path, local_path }` → `{ bytes, local_path }`
  - `POST /connections/{id}/sftp/upload`   `{ local_path, remote_path }` → 200
  - `POST /connections/{id}/sftp/mkdir`    `{ path }` → 200
  - `POST /connections/{id}/sftp/remove`   `{ path, dir? }` → 200
  - `POST /connections/{id}/sftp/rename`   `{ from, to }` → 200
  - `GET  /connections/{id}/sftp/read?path=` (text view; size-capped) → `{ text, truncated }`
  Resolve the connection, ensure `kind == ssh`, build args from params + Keychain
  identity, run via `otto-ssh::sftp`.
- `crates/otto-server/src/policy.rs`: add
  `if p.starts_with("/connections/{id}/sftp/") { Require(Connections, if get { View } else { Edit }) }`
  in the Connections block.
- Types: define `SftpEntry`/req-resp in `otto-core` api (or otto-connections) +
  mirror in `types.ts`.
- `docs/contracts/api.md`: document the SFTP routes.

### Frontend changes
- `ui/src/lib/stores/sftp.svelte.ts` (new): per-connection cwd, entries, ops,
  loading, last-used local dir (localStorage `otto_sftp_local_dir`, default
  `~/Downloads`).
- `ui/src/modules/connections/SftpBrowser.svelte` (new): path/breadcrumb bar, up,
  refresh, file list (name/size/modified/perms, dir vs file icons), navigate into
  dirs, download (to chosen local dir via existing `/fs/browse` picker), upload
  (pick local file), mkdir, delete (confirm), rename, view-text modal.
- `ui/src/modules/connections/ConnectionsPage.svelte`: add a "Files" action on
  SSH connections that opens `SftpBrowser` (overlay/pane). No terminal session
  required.

### Guardrails
- SSH connections use key/agent auth (no password in current model) → don't try
  password injection. Gate transfers behind `Connections:Edit`. Validate/expand
  `~` in local paths; default downloads to `~/Downloads`.

---

## Feature 3 — DB export to a local file (large batch, selectable format)

### Requirement
Download query results as CSV (and other formats) for **large** result sets to a
**configurable local path**, format selectable in the UI. ClickHouse is reached
over SSH so server-side `INTO OUTFILE` would land on the tunnel/remote — must
instead stream results back through the daemon and write the user's local path.

### Approach — STREAMING IS MANDATORY (do not buffer the whole result in RAM)
New endpoint that runs the query **uncapped** and writes the formatted output to
a **local path** (daemon is local), with a selectable format. The export MUST
stream row/chunk-by-chunk from the driver straight to a buffered file writer so
daemon memory stays **bounded** regardless of result size. Buffering the full
result set is a **last-resort fallback ONLY** when a given driver path genuinely
cannot stream — and that fallback must be `log!`-noted, not silent.

Per-engine streaming (all three drivers CAN stream — use it):
- **ClickHouse**: issue the query with an explicit `FORMAT <fmt>` and stream the
  HTTP response body (`reqwest` `bytes_stream()`) through the SSH tunnel straight
  to the file. Constant memory; native formatting. (NOT `INTO OUTFILE`.)
- **MySQL (sqlx)**: use the cursor stream `sqlx::query(...).fetch(&pool)` (a
  `Stream` of rows), format each row, write incrementally. Do NOT use
  `fetch_all`.
- **MongoDB**: iterate the `Cursor` (it is a `Stream`) document-by-document,
  format, write incrementally. Do NOT collect the cursor into a Vec.

Use a `BufWriter` to the local file and flush periodically; track rows/bytes for
the response. Honour an optional `max_rows` cap by stopping the stream early.

### Backend changes
- `crates/otto-dbviewer/src/http.rs`: add
  `POST /connections/{id}/db/export-to-path`
  `{ statement, node?, format, local_path, max_rows? }` →
  `{ local_path, rows, bytes, duration_ms }`. Formats: `csv`, `csv_with_names`,
  `tsv`, `tsv_with_names`, `json` (array), `ndjson`. Write to the resolved local
  path (expand `~`, ensure parent dir). Already gated `Database:Edit` by the
  `/connections/{id}/db/` policy prefix — no policy change.
- `crates/otto-dbviewer/src/drivers/clickhouse.rs`: helper to run a query with an
  explicit `FORMAT` and stream the HTTP response body to a writer (large-batch
  path). MySQL/Mongo reuse the existing run + daemon-side formatter, writing
  incrementally.
- Reuse/extend the existing CSV escaping helpers in `http.rs`.
- Types: `ExportToPathReq`/`ExportToPathResp` (+ format enum) in otto-dbviewer
  types or otto-core; mirror in `types.ts`.
- `docs/contracts/api.md`: document the route + formats.

### Frontend changes
- `ui/src/modules/database/ResultsGrid.svelte`: add a "Download / Export…" action
  opening a small dialog: **format** select (CSV / CSV w/ header / TSV / TSV w/
  header / JSON / NDJSON), **destination** (dir via `/fs/browse` picker + file
  name; default `~/Downloads/<query>.<ext>`), optional **row limit** (blank =
  all). Calls `export-to-path`; shows rows/bytes/path + a toast on completion.
  Keep the existing client-side CSV/JSON and "Full Export" browser-download
  buttons; this adds the local-path/large-batch path. Persist last format + dir.
- `ui/src/lib/api/types.ts`: the new req/resp.

---

## Coverage matrix (every user request → where handled)

| # | Request | Handled by |
|---|---------|-----------|
| G1 | Git tabbed like GitKraken | `GitTabs.svelte` + GitPage compose; per-repo sub-tabs retained |
| G2a | Not tied to any workspace | `GET /git/repos` global list + `loadAllRepos()`; drop ws coupling |
| G2b | Remember open tabs across restart | global localStorage `otto_git_open_tabs` restore-on-mount |
| G3 | Graph: clear current branch + GitKraken detail | `parse.rs` keep HEAD; GraphView ref-chip classes, current-branch/HEAD emphasis, ahead/behind, left-panel polish |
| G4 | Right gap on Git page | full-bleed Git page; no `.page` width / reserved right column; RightPanel stays agents-only |
| S1 | SFTP transfer/read on SSH connections | `otto-ssh::sftp` + `/connections/{id}/sftp/*` + `SftpBrowser.svelte` |
| C1 | DB CSV download for large batch | `export-to-path` uncapped + incremental write; CH `FORMAT` streaming |
| C2 | Save to configurable local path (not INTO OUTFILE on tunnel) | daemon writes user-chosen `local_path`; CH `FORMAT CSVWithNames` streamed to local file |
| C3 | Selectable result format | format enum + UI select |

## Verification gates (run after each feature, and at the end)
- `cargo build --workspace` ; `cargo test --workspace` ;
  `cargo clippy --workspace --all-targets -- -D warnings`
- `cd ui && npm run check && npm run build`
- Manual: open Git (tabs/graph/no-gap), SFTP browse+download+upload, DB export to
  a local CSV from a ClickHouse-over-SSH connection.
- Then rebuild/resign/reinstall the Tauri app + restart ottod.
