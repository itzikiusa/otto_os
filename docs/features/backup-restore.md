# Backup, restore, and Git snapshots

Open **Settings → Backup & Restore** as a root administrator. The page offers
three different ways to move data: an Otto data archive, portable Git snapshots,
and connection exports for other applications. The older settings transfer
controls remain available below them.

## Otto data archive

**Download data archive** creates a format-2 JSON archive of saved Otto records
and managed files. It includes workflow definitions/versions, scheduled tasks,
connection profiles, API collections/requests/environments/automations, saved
session/history records, and other supported persisted feature data. Managed
Canvas/Product assets and registered Vault documents travel with it.

The download reports record/file counts, exclusions, and reconnect requirements.
Stored credentials, authentication sessions, and active authorization bindings
are excluded. Live processes, PTYs, repository working trees, external database
contents, external provider transcript directories, and disposable indexes are
not copied. Documents and historical text can contain private information; keep
the archive private even though stored credentials are filtered.

The database records share one SQLite snapshot. Files are read and verified
individually; the archive is not a filesystem-wide snapshot of simultaneous
external edits. An unreadable/missing registered root, unsupported special file,
or size overflow fails explicitly instead of silently returning a complete-looking
partial archive. Individual files are limited to 64 MiB and encoded archives to
256 MiB. Import currently requires the same migration level as the source daemon.

### Restore

1. Choose a data archive.
2. Choose **Keep existing; restore only new items**, or stop on any conflict.
3. Select **Preview restore**. Review included records, conflicting items,
   exclusions, and reconnect requirements.
4. Confirm that you reviewed the preview, then select **Restore new items**.

Existing records and files are preserved. This is an additive restore, so it is
suitable for recovering missing data or moving it to a new installation; it does
not overwrite an existing workflow with an older version. Preview tokens bind
apply to the reviewed archive and relevant target conflicts. Changes that affect
the preview require another preview.

Imported automatic activity stays inactive. Reconnect accounts and connection
credentials, review paths, and explicitly enable schedules/services you want to
run. Imported users cannot authenticate until configured. Vault files relocate
under Otto's managed restore area and their derived index is rebuilt. Owned
Canvas/Product files use the normal feature asset layout.

## Git snapshots

Choose an **existing local repository** with Browse, then **Check repository**.
The shared picker offers navigation, Favorites, and Recents. Configure a remote
through Git if you want remote synchronization; Otto does not create or publish
a remote implicitly.

**Preview snapshot** shows the managed files that will change. **Write snapshot**
writes portable configuration and documents under `.otto-sync/`. Each configuration
area has a readable JSON file and managed documents keep separate files. Stored
credentials, active authorization grants, and runtime history are excluded from
the portable profile. Review document content before pushing it to a remote.

Enter a commit message and choose **Commit snapshot** when ready. The commit
contains only the snapshot files and preserves unrelated staged work. Repeated
exports of unchanged data are stable; capture timestamps do not manufacture diffs.
Local edits to managed files are detected before replacement. Files unrelated to
the manifest remain untouched.

Remote actions are separate:

- **Fetch** refreshes remote references.
- **Pull fast-forward** requires a clean repository and refuses diverged history.
- **Push current branch** sends the entire current branch history to the selected configured remote without force, including other commits in the repository.

None of these operations imports changes into Otto automatically. **Preview Git
restore** verifies the snapshot and uses the same additive restore rules as an
archive: missing items can be imported, and existing items remain unchanged.
A repository with updated versions of existing Otto records therefore reports
those records as conflicts instead of overwriting them.

Snapshot publication writes the manifest last but is not one atomic filesystem
operation across all files. If interrupted, inspect the uncommitted Git changes
before retrying; the source Otto data is unaffected by export/sync.

## Connection exports

Choose **All workspaces** or selected workspaces, then an export format. JSON
and CSV cover all connection kinds. Selected workspaces also include global connections. Application-specific formats include only
the connection kinds supported by that application; the result lists skipped
connections and import instructions.

Passwords are excluded by default. **Include saved passwords and credentials**
explicitly requests readable credentials for the prepared export. Download its
files individually, then clear the prepared export if you no longer need it.
Prepared contents are not persisted in browser storage. Keep credential-bearing
files private and out of Git.

| Destination | Export |
|---|---|
| General-purpose transfer | JSON or CSV |
| MySQL Workbench | Connection XML; optional separate credentials file |
| DBeaver, MySQL | Custom connection CSV |
| DBeaver, MongoDB | Custom connection CSV |
| NoSQLBooster 9+ | MongoDB URI list |
| RedisInsight | Native connection JSON |

Native import instructions and unsupported-option warnings appear with the
prepared export. Workbench stores passwords outside its connection XML, so its
password option generates a separate credential file. See the connection guide
for format details and source documentation: [Connection exports](connections-ssh-sftp.md#exporting-connections-to-another-tool).

## Older settings backups

**Export settings** contains daemon settings only. **Download settings backup**
adds workspace names/count, daemon version, migration level, and timestamp to
those settings. Its format-1 manifest does not contain workflows, tasks,
connections, or workspace records. **Restore settings** merges settings and
reloads the relevant provider configuration; it is separate from data restore.

## Data retention

The audit/event tables (plus notifications and room messages) are append-only and used to grow for the life of an
install (one real `otto.db` reached 514 MB). The daemon prunes them **hourly**
(first pass at startup):

| Table | Kept |
|---|---|
| `work_events` (Mission Control item history) | every row younger than **30 days**, AND each *active* item's newest **500** events regardless of age — a row of an active item is deleted only when it is both older than the window and outside its newest 500. An item with no event for **90 days** (`work_events_idle_days`) is idle: its rows older than the window are deleted like any other table's (the item itself stays) |
| `mcp_tool_calls` (first-party MCP tool ledger) | **90 days** |
| `mcp_call_log` (MCP control-plane call log) | **90 days** |
| `audit_log` (security audit trail) | **90 days** |
| `review_agent_prompts` + `review_diffs` (per-agent review retry data) | **14 days** after the artifact was written, and only for a finished review (`done` / `error` / `cancelled`, or the review row is gone). A running review keeps them. Past the window, retrying a review agent falls back to "prompt unavailable" |
| `notifications` (notification center) | read notices **30 days**, unread notices **90 days**, and never more than the newest **5 000** rows overall |
| `agent_room_messages` (Agent rooms) | each room's newest **5 000** messages (the room's message count is recomputed after a trim) |
| Run history (`run_history_days`) — **off by default** (`0` = keep forever). Opt in by setting a window; values are floored at 14 days | When enabled: `otto_runs` + `otto_run_events` (Run with Otto): terminal runs (`completed` / `failed` / `rejected` / `cancelled`) last updated before the window, unless a Proof Pack is attached. `swarm_runs`: terminal runs (`done` / `error` / `stopped`) finished before the window — first added to the swarm's `pruned_runs` / `pruned_cost_usd` rollup (migration `0161`), so `max_total_runs` / `max_cost_usd` budgets still count them. `swarm_messages`: older than the window. `goal_loop_iterations`: every iteration but the last of a loop that finished (terminal) before the window. Live runs and loops are never touched |

Rows younger than their window are never touched. The policy lives in the
`data_retention` setting and is re-read every pass. **Run history** has a
control in the UI: **Settings → Backup & restore → Database storage → Run
history** (root only) offers *Keep forever* (the default) or a window of 14–365
days; choosing a window asks first and lists what will be pruned. Everything
else is set through the API with `PUT /settings` (root), whose body is a map of
setting keys:

```json
{"data_retention": {"enabled": true, "work_events_days": 30, "work_events_keep_per_item": 500,
                    "work_events_idle_days": 90, "mcp_audit_days": 90, "audit_log_days": 90,
                    "review_retry_days": 14, "notifications_read_days": 30,
                    "notifications_unread_days": 90, "notifications_max_rows": 5000,
                    "room_messages_keep_per_room": 5000, "run_history_days": 0}}
```

`POST /settings/import` works too, but its body wraps the map in `settings`:
`{"settings": {"data_retention": {…}}}` (the bare map above is a 422 there).
Either way the stored `data_retention` object is **replaced** by the one you
send, and a field you leave out falls back to its default (not to the value
stored before), so send every field you have customised. With nothing else
customised, `{"data_retention": {"run_history_days": 30}}` is enough to opt in.
The Settings control does this for you: it writes the stored object back with
only `run_history_days` changed.

`enabled: false` turns the job off. Floors a setting can't go below: 7 days
(30 for `audit_log`, 3 for `review_retry_days`), 50 events per item, and `work_events_idle_days` never
below `work_events_days`; `notifications_unread_days` never below
`notifications_read_days`, and at least 500 for `notifications_max_rows` and
`room_messages_keep_per_room`. `run_history_days` is opt-in: `0` (the default)
or a negative value keeps run history forever; any positive value is floored at
14 days. Deletes run in 1 000-row batches, each a bounded
range on an index, with a 25 ms pause between batches so the SQLite writer is
never held for long; the candidate items are found by reads on the read-only
pool (index `idx_work_events_ts`, migration `0144`). Freed pages are reused,
so the file stops growing; it only shrinks after a compaction (below).

Other retention passes:

- **Expired sign-ins** (hourly): login, API and impersonation credentials
  more than 7 days past `expires_at` are deleted (share links keep their own
  window). The auth cache drops each deleted token.
- **Workflow runs** (daily, first pass at startup): per workflow, the newest
  200 finished runs and every run younger than 30 days are kept. A run that is
  pending, running or waiting for approval, or that a Proof Pack or a
  scheduled-task run references, is never deleted. Each pruned run's
  checkpoints and `workflow-context/<run_id>/` folder go with it. Deletes run
  in 100-row transactions.

### Database maintenance

Every hour the daemon runs `PRAGMA optimize` (planner statistics) and a
`wal_checkpoint(TRUNCATE)` (PASSIVE when a reader still needs the log); the
WAL is capped at 64 MiB (`journal_size_limit`). **Settings → Backup & restore
→ Database storage** (root) shows the file size and reclaimable free space,
and **Compact database…** runs the one-time conversion to
`auto_vacuum=INCREMENTAL` plus a `VACUUM`. The rewrite blocks database writes
while it runs, so it asks first; nothing is deleted. After it, the hourly pass
returns up to 4 000 free pages per hour (`incremental_vacuum`).

The per-session activity trail (`agent_trail`, newest 1 000 rows per session)
is pruned by a separate hourly pass. After the first pass at startup it only
looks at sessions that received trail rows since the previous pass
(`idx_agent_trail_ts`), and it deletes each session's excess in 500-row index
ranges instead of re-ranking the whole table per chunk.

Separately, `mcp_tool_calls.args_json` and `mcp_call_log.args_redacted_json`
are shaped at insert: any string value over 1 024 characters is stored as its
first and last 512 characters around `…[truncated: N chars, sha256 H]…`, so a
padded payload's tail (say, a `DROP` after a kilobyte of comments) stays
visible and the full value stays verifiable by hash. The whole document is
capped at 16 KiB; past that the row holds
`{"_otto_truncated":true,"bytes":N,"sha256":H,"head":…,"tail":…}`. The JSON
always stays valid. The ledger records that a tool was called and with what
arguments' shape — not whole note or file bodies.
Migration `0142` also removes duplicate `artifact_added` work events left by an
older reconcile bug (keeping the earliest per item, actor and payload).

Implementation: `crates/otto-state/src/retention.rs` (policy + prune),
`crates/otto-state/src/mcp_audit.rs` (`cap_args_json`), the hourly task in
`crates/ottod/src/main.rs`.

Implementation: `crates/otto-server/src/state_archive.rs`,
`state_archive/{schema,files}.rs`, and the backup/Git/connection export routes.
The authoritative HTTP shapes are in [API contracts](../contracts/api.md).
The [archive inventory](state-archive.md) describes included tables and file roots.
