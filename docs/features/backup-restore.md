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

Four audit/event tables are append-only and used to grow for the life of an
install (one real `otto.db` reached 514 MB). The daemon prunes them **hourly**
(first pass at startup):

| Table | Kept |
|---|---|
| `work_events` (Mission Control item history) | every row younger than **30 days**, AND each item's newest **500** events regardless of age — a row is deleted only when it is both older than the window and outside its item's newest 500 |
| `mcp_tool_calls` (first-party MCP tool ledger) | **90 days** |
| `mcp_call_log` (MCP control-plane call log) | **90 days** |
| `audit_log` (security audit trail) | **90 days** |

Rows younger than their window are never touched. The policy lives in the
`data_retention` setting (a partial object merges over the defaults) and is
re-read every pass; set it with `POST /settings/import`:

```json
{"data_retention": {"enabled": true, "work_events_days": 30, "work_events_keep_per_item": 500,
                    "mcp_audit_days": 90, "audit_log_days": 90}}
```

`enabled: false` turns the job off. Floors a setting can't go below: 7 days
(30 for `audit_log`) and 50 events per item. Deletes run in 5 000-row batches
so the SQLite writer is never held for long; freed pages are reused, so the
file stops growing but only shrinks after a manual `VACUUM`.

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
